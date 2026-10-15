//! Wasm waiter cohorts. Guest acknowledgement does not prove queue insertion.
use super::shared_bulk_driver::{Gate, affinity};
use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
use wtime_runtime::{
    CancellationToken, Config, Engine, External, Func, Linker, Module, RunOutcome, ShareableMemory,
    Store, Value,
};
use wtime_types::{Limits, MemoryType};

const MEMORY_BYTES: usize = 65536;
const READY_OFFSET: u64 = 4096;
const MAX_BATCH: usize = 4096;
const WAIT_TIMEOUT_NS: i64 = 5_000_000_000;
const MAX_NOTIFY_ATTEMPTS: i32 = 1_000_000;
const DEADLINE: Duration = Duration::from_secs(30);
const READINESS: &str =
    "guest acknowledgement before wait; queue registration is not observed before timing";
const BOUNDARY: &str = "waiter gate release through all waiter and drain results; guest readiness and failed notify probes included; host setup/join/image/cleanup checks excluded";

#[derive(Debug)]
struct Request {
    case: String,
    waiters: usize,
    addresses: usize,
    batch: usize,
}
fn parse(args: &[String]) -> Result<Request, String> {
    if args.len() != 9 || args[0] != "--wait-queue-cohort" {
        return Err("expected FILE CASE WAITERS ADDRESSES STAGE BATCH MODE PROFILE".into());
    }
    let number = |i: usize| {
        args[i]
            .parse::<usize>()
            .map_err(|_| "invalid cohort count".to_owned())
    };
    let (waiters, addresses, batch) = (number(3)?, number(4)?, number(6)?);
    if !matches!(
        (waiters, addresses),
        (2, 1) | (32, 1) | (8, 1) | (2, 2) | (32, 8) | (8, 4)
    ) || !(1..=MAX_BATCH).contains(&batch)
        || args[5] != "steady_state_group"
        || args[7] != "decoded"
        || args[8] != "threads"
        || args[2] != format!("wait_queue_cohort_w{waiters}_a{addresses}")
    {
        return Err("unsupported waiter cohort request".into());
    }
    Ok(Request {
        case: args[2].clone(),
        waiters,
        addresses,
        batch,
    })
}
fn bind(
    engine: &Engine,
    module: &Module,
    token: &ShareableMemory,
    export: &str,
) -> Result<(Store, Func), String> {
    let mut store = Store::new(engine);
    let memory = store
        .import_shared_memory(token)
        .map_err(|e| e.to_string())?;
    let mut linker = Linker::new(engine);
    linker
        .define("env", "memory", External::Memory(memory))
        .map_err(|e| e.to_string())?;
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| e.to_string())?;
    let External::Func(function) = instance.export(&store, export).map_err(|e| e.to_string())?
    else {
        return Err("cohort export is not a function".into());
    };
    Ok((store, function))
}
fn invoke(store: &mut Store, function: Func, arguments: &[Value]) -> Result<Vec<Value>, String> {
    match store
        .invoke(function, arguments)
        .map_err(|e| e.to_string())?
    {
        RunOutcome::Returned(values) => Ok(values),
        _ => Err("cohort guest did not return".into()),
    }
}
fn one(store: &mut Store, function: Func, arguments: &[Value]) -> Result<i32, String> {
    match invoke(store, function, arguments)?.as_slice() {
        [Value::I32(n)] => Ok(*n),
        _ => Err("cohort guest did not return one i32".into()),
    }
}

struct Abort {
    wait_gate: Arc<Gate>,
    drain_gate: Arc<Gate>,
    tokens: Vec<CancellationToken>,
}
impl Drop for Abort {
    fn drop(&mut self) {
        // Scope cleanup runs this guard before implicit joins on every error.
        for token in &self.tokens {
            token.cancel();
        }
        self.wait_gate.release(false);
        self.drain_gate.release(false);
    }
}
enum Completion {
    Waiter(usize, Result<i32, String>),
    Drain(Instant, Result<(i32, i32), String>),
}
struct Group {
    elapsed: u128,
    masks: Vec<Vec<usize>>,
    attempts: i32,
    returns: Vec<i32>,
    total: i32,
    ready: i32,
}
fn run_group(
    owner: &Store,
    memory: wtime_runtime::Memory,
    waiters: Vec<(Store, Func, usize)>,
    controller: (Store, Func),
    addresses: usize,
) -> Result<Group, String> {
    let count = waiters.len();
    let wait_gate = Arc::new(Gate::default());
    let drain_gate = Arc::new(Gate::default());
    let tokens = waiters
        .iter()
        .map(|(s, _, _)| s.cancellation_token())
        .chain(std::iter::once(controller.0.cancellation_token()))
        .collect();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    thread::scope(|scope| {
        let _abort = Abort {
            wait_gate: wait_gate.clone(),
            drain_gate: drain_gate.clone(),
            tokens,
        };
        let mut handles = Vec::new();
        for (index, (mut store, function, address)) in waiters.into_iter().enumerate() {
            let ready = ready_tx.clone();
            let done = done_tx.clone();
            let gate = wait_gate.clone();
            handles.push(
                thread::Builder::new()
                    .spawn_scoped(scope, move || {
                        if ready.send((index, affinity())).is_err() || !gate.wait() {
                            return;
                        }
                        let result = one(
                            &mut store,
                            function,
                            &[Value::I32(address as i32), Value::I64(WAIT_TIMEOUT_NS)],
                        );
                        let _ = done.send(Completion::Waiter(index, result));
                    })
                    .map_err(|e| e.to_string())?,
            );
        }
        let ready = ready_tx.clone();
        let done = done_tx.clone();
        let gate = drain_gate.clone();
        let (mut controller, drain) = controller;
        handles.push(
            thread::Builder::new()
                .spawn_scoped(scope, move || {
                    if ready.send((count, affinity())).is_err() || !gate.wait() {
                        return;
                    }
                    let start = Instant::now();
                    let result = invoke(
                        &mut controller,
                        drain,
                        &[Value::I32(count as i32), Value::I32(addresses as i32)],
                    )
                    .and_then(|values| match values.as_slice() {
                        [Value::I32(total), Value::I32(attempts)] => Ok((*total, *attempts)),
                        _ => Err("cohort drain did not return two i32 values".into()),
                    });
                    let _ = done.send(Completion::Drain(start, result));
                })
                .map_err(|e| e.to_string())?,
        );
        drop(ready_tx);
        drop(done_tx);
        let deadline = Instant::now() + DEADLINE;
        let mut masks = vec![Vec::new(); count + 1];
        for _ in 0..=count {
            let (index, mask) = ready_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| format!("cohort host readiness failed: {e}"))?;
            if index > count || !masks[index].is_empty() {
                return Err("duplicate worker readiness".into());
            }
            masks[index] = mask?;
        }
        if masks.iter().any(|mask| mask != &masks[0]) {
            return Err("cohort worker masks differ".into());
        }
        masks.pop(); // The drainer was checked against every waiter as well.
        let group_start = Instant::now();
        wait_gate.release(true);
        let ready_count = loop {
            // A guest flag immediately precedes wait; no enqueue claim follows.
            match done_rx.try_recv() {
                Ok(_) => return Err("cohort waiter completed before drain started".into()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("cohort workers disconnected".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let bytes = owner
                .read_memory(memory, READY_OFFSET, 4)
                .map_err(|e| e.to_string())?;
            let ready =
                u32::from_le_bytes(bytes.try_into().map_err(|_| "ready flag width")?) as usize;
            if ready == count {
                break ready as i32;
            }
            if ready > count {
                return Err("cohort guest acknowledgement count differs".into());
            }
            if Instant::now() >= deadline {
                return Err("cohort guest readiness timed out".into());
            }
            thread::yield_now();
        };
        drain_gate.release(true);
        let mut seen = vec![false; count];
        let mut returns = vec![0; count];
        let mut drain_result = None;
        for _ in 0..=count {
            let completion = done_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| format!("cohort completion timed out: {e}"))?;
            match completion {
                Completion::Waiter(index, value) => {
                    let value = value?;
                    if index >= count || seen[index] || value != 0 {
                        return Err("cohort waiter return oracle differs".into());
                    }
                    seen[index] = true;
                    returns[index] = value;
                }
                Completion::Drain(start, result) => {
                    if drain_result.is_some() {
                        return Err("duplicate drain result".into());
                    }
                    let (total, attempts) = result?;
                    if total != count as i32
                        || !(count as i32..=MAX_NOTIFY_ATTEMPTS).contains(&attempts)
                    {
                        return Err("cohort notification count oracle differs".into());
                    }
                    drain_result = Some((start, total, attempts));
                }
            }
        }
        let finish = Instant::now();
        let (start, total, attempts) = drain_result.ok_or("missing cohort drain result")?;
        if start < group_start || finish < start {
            return Err("cohort drain timestamps reversed".into());
        }
        if seen.iter().any(|done| !done) {
            return Err("missing cohort waiter result".into());
        }
        let elapsed = finish
            .checked_duration_since(group_start)
            .ok_or("cohort timestamps reversed")?
            .as_nanos()
            .max(1);
        for handle in handles {
            handle.join().map_err(|_| "cohort worker panicked")?;
        }
        Ok(Group {
            elapsed,
            masks,
            attempts,
            returns,
            total,
            ready: ready_count,
        })
    })
}

struct Observation {
    elapsed: u128,
    masks: Vec<Vec<usize>>,
    attempts: Vec<i32>,
    image: Vec<u8>,
    returns: Vec<i32>,
    totals: Vec<i32>,
    cleanup: Vec<i32>,
    ready: Vec<i32>,
}
fn execute(bytes: &[u8], request: &Request) -> Result<Observation, String> {
    let mut config = Config::default();
    config.features.threads = true;
    config.limits.fuel = None;
    let engine = super::configured_engine(config, "decoded");
    let module = engine.compile(bytes).map_err(|e| e.to_string())?;
    let mut observation = Observation {
        elapsed: 0,
        masks: vec![],
        attempts: vec![],
        image: vec![],
        returns: vec![],
        totals: vec![],
        cleanup: vec![],
        ready: vec![],
    };
    for _ in 0..request.batch {
        let mut owner = Store::new(&engine);
        let memory = owner
            .create_memory(MemoryType {
                limits: Limits::new(1, Some(1)),
                shared: true,
                ..MemoryType::default()
            })
            .map_err(|e| e.to_string())?;
        let token = owner.share_memory(memory).map_err(|e| e.to_string())?;
        let waiters = (0..request.waiters)
            .map(|index| {
                bind(&engine, &module, &token, "wait")
                    .map(|(store, function)| (store, function, (index % request.addresses) * 4))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let controller = bind(&engine, &module, &token, "drain")?;
        let group = run_group(&owner, memory, waiters, controller, request.addresses)?;
        if !observation.masks.is_empty() && observation.masks != group.masks {
            return Err("cohort affinity changed across groups".into());
        }
        observation.masks = group.masks;
        let (mut cleanup, notify) = bind(&engine, &module, &token, "notify")?;
        for address in 0..request.addresses {
            let remaining = one(
                &mut cleanup,
                notify,
                &[Value::I32((address * 4) as i32), Value::I32(-1)],
            )?;
            if remaining != 0 {
                return Err("cohort left a stale wait registration".into());
            }
            observation.cleanup.push(remaining);
        }
        let image = owner
            .read_memory(memory, 0, MEMORY_BYTES)
            .map_err(|e| e.to_string())?;
        let mut expected = vec![0; MEMORY_BYTES];
        expected[READY_OFFSET as usize..READY_OFFSET as usize + 4]
            .copy_from_slice(&(request.waiters as u32).to_le_bytes());
        if image != expected {
            return Err("cohort full memory image oracle differs".into());
        }
        observation.image = image;
        observation.elapsed = observation
            .elapsed
            .checked_add(group.elapsed)
            .ok_or("cohort time overflow")?;
        observation.attempts.push(group.attempts);
        observation.returns.extend(group.returns);
        observation.totals.push(group.total);
        observation.ready.push(group.ready);
    }
    Ok(observation)
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--wait-queue-cohort") {
        return false;
    }
    let request = parse(args).expect("invalid wait queue request");
    assert!(!cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference"));
    assert!(std::fs::metadata(&args[1]).unwrap().len() <= 1024 * 1024);
    let bytes = std::fs::read(&args[1]).unwrap();
    let result = execute(&bytes, &request).expect("wait queue cohort failed");
    println!(
        concat!(
            "{{\"schema\":\"wtime-wait-queue-cohort-v1\",\"schema_version\":1,\"case\":{},",
            "\"mode\":\"decoded\",\"profile\":\"threads\",\"stage\":\"steady_state_group\",",
            "\"waiters\":{},\"addresses\":{},\"batch\":{},\"timeout_ns\":{},",
            "\"instrumented\":false,\"report_complete\":true,\"every_group_full_memory_checked\":true,",
            "\"elapsed_ns\":{},\"calls_checked\":{},\"worker_returns\":{:?},\"notify_totals\":{:?},",
            "\"notify_attempts\":{:?},\"cleanup_counts\":{:?},\"pre_wait_ready_counts\":{:?},",
            "\"worker_affinity\":{:?},\"readiness_evidence\":{},\"timing_boundary\":{},",
            "\"backings_hex\":{{\"shared\":{}}}}}"
        ),
        super::quote(&request.case),
        request.waiters,
        request.addresses,
        request.batch,
        WAIT_TIMEOUT_NS,
        result.elapsed,
        request.batch * (request.waiters + 1 + request.addresses),
        result.returns,
        result.totals,
        result.attempts,
        result.cleanup,
        result.ready,
        result.masks,
        super::quote(READINESS),
        super::quote(BOUNDARY),
        super::quote(&hex(&result.image))
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../fixtures/threads/wait_queue_cohort.wasm");
    #[test]
    fn p077_driver_rejects_invalid_shape_before_io() {
        let valid: Vec<String> = [
            "--wait-queue-cohort",
            "nonexistent.wasm",
            "wait_queue_cohort_w2_a1",
            "2",
            "1",
            "steady_state_group",
            "1",
            "decoded",
            "threads",
        ]
        .map(str::to_owned)
        .to_vec();
        assert!(parse(&valid).is_ok());
        let mut maximum = valid.clone();
        maximum[6] = MAX_BATCH.to_string();
        assert!(parse(&maximum).is_ok());
        for (index, value) in [
            (2, "wait_queue_cohort_w2_a2"),
            (3, "0"),
            (3, "64"),
            (4, "3"),
            (5, "warm_call"),
            (6, "0"),
            (6, "4097"),
            (7, "register"),
            (8, "mvp"),
        ] {
            let mut bad = valid.clone();
            bad[index] = value.into();
            assert!(parse(&bad).is_err());
        }
        assert!(parse(&valid[..8]).is_err());
        let mut extra = valid;
        extra.push("extra".into());
        assert!(parse(&extra).is_err());
    }
    #[test]
    fn p077_driver_checks_cohort_counts_and_full_memory() {
        for (waiters, addresses) in [(2, 1), (32, 1), (8, 1), (2, 2), (32, 8), (8, 4)] {
            let request = Request {
                case: format!("wait_queue_cohort_w{waiters}_a{addresses}"),
                waiters,
                addresses,
                batch: 2,
            };
            let observed = execute(FIXTURE, &request).unwrap();
            assert!(observed.elapsed > 0);
            assert_eq!(observed.attempts.len(), 2);
            assert!(
                observed
                    .attempts
                    .iter()
                    .all(|n| *n >= waiters as i32 && *n <= MAX_NOTIFY_ATTEMPTS)
            );
            let mut expected = vec![0; MEMORY_BYTES];
            expected[4096..4100].copy_from_slice(&(waiters as u32).to_le_bytes());
            assert_eq!(observed.image, expected);
            assert_eq!(observed.masks.len(), waiters);
        }
    }
    #[test]
    fn p077_driver_rejects_corrupt_wait_fixture() {
        // Change the waited-for value to one. Every waiter now returns mismatch
        // without registration; a fake early completion cannot pass this driver.
        let needle = [0x20, 0, 0x41, 0, 0x20, 1, 0xfe, 1];
        let sites: Vec<_> = FIXTURE
            .windows(needle.len())
            .enumerate()
            .filter_map(|(i, window)| (window == needle).then_some(i))
            .collect();
        assert_eq!(sites.len(), 1);
        let mut corrupt = FIXTURE.to_vec();
        corrupt[sites[0] + 3] = 1;
        let request = Request {
            case: "wait_queue_cohort_w2_a1".into(),
            waiters: 2,
            addresses: 1,
            batch: 1,
        };
        assert!(execute(&corrupt, &request).is_err());
    }
}
