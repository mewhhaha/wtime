//! Source-bound shared-memory copy groups; setup and image checks are untimed.
use std::{
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use wtime_runtime::{Config, External, Linker, RunOutcome, ShareableMemory, Store, Value};
use wtime_types::{FeatureSet, Limits, MemoryType};

const MEMORY_BYTES: usize = 4 * 65536;
const REPETITIONS: i32 = 32;
const MAX_BATCH: usize = 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
struct Request {
    case: String,
    alias: bool,
    workers: usize,
    bytes: usize,
    batch: usize,
}
fn parse(args: &[String]) -> Result<Request, String> {
    if args.len() != 10 || args[0] != "--shared-bulk-copy" {
        return Err("expected FILE CASE BACKING WORKERS BYTES STAGE BATCH MODE PROFILE".into());
    }
    let alias = match args[3].as_str() {
        "alias" => true,
        "distinct" => false,
        _ => return Err("unknown shared backing policy".into()),
    };
    let number = |index: usize| {
        args[index]
            .parse::<usize>()
            .map_err(|_| "invalid shared bulk count".to_owned())
    };
    let workers = number(4)?;
    let bytes = number(5)?;
    let batch = number(7)?;
    if !matches!(workers, 1 | 4)
        || !matches!(bytes, 512 | 32768 | 4097)
        || !(1..=MAX_BATCH).contains(&batch)
        || args[6] != "steady_state_group"
        || args[8] != "decoded"
        || args[9] != "all-phase3-core"
    {
        return Err("unsupported shared bulk request".into());
    }
    let backing = if alias { "alias" } else { "distinct" };
    if args[2] != format!("shared_bulk_copy_{backing}_w{workers}_b{bytes}") {
        return Err("shared bulk case/count mismatch".into());
    }
    Ok(Request {
        case: args[2].clone(),
        alias,
        workers,
        bytes,
        batch,
    })
}

pub(super) fn affinity() -> Result<Vec<usize>, String> {
    let status = std::fs::read_to_string("/proc/thread-self/status").map_err(|e| e.to_string())?;
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
        .ok_or("missing worker CPU affinity")?;
    let mut result = Vec::new();
    for part in value.trim().split(',') {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first = first.parse::<usize>().map_err(|e| e.to_string())?;
        let last = last.parse::<usize>().map_err(|e| e.to_string())?;
        if first > last || last - first > 65536 {
            return Err("invalid CPU mask".into());
        }
        result.extend(first..=last);
    }
    if result.is_empty() {
        return Err("empty worker CPU affinity".into());
    }
    Ok(result)
}

#[derive(Default)]
pub(super) struct Gate {
    state: Mutex<Option<bool>>,
    wake: Condvar,
}
impl Gate {
    pub(super) fn release(&self, execute: bool) {
        *self.state.lock().unwrap() = Some(execute);
        self.wake.notify_all();
    }
    pub(super) fn wait(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.is_none() {
            state = self.wake.wait(state).unwrap();
        }
        state.unwrap()
    }
}

fn run_group(
    contexts: Vec<(Store, wtime_runtime::Func, Vec<Value>)>,
) -> Result<(u128, Vec<i32>, Vec<Vec<usize>>), String> {
    let count = contexts.len();
    let cancellations: Vec<_> = contexts
        .iter()
        .map(|(s, _, _)| s.cancellation_token())
        .collect();
    let gate = Arc::new(Gate::default());
    let (ready_tx, ready_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for (index, (mut store, function, arguments)) in contexts.into_iter().enumerate() {
            let worker_gate = gate.clone();
            let ready = ready_tx.clone();
            let result = result_tx.clone();
            let worker = thread::Builder::new().spawn_scoped(scope, move || {
                let mask = affinity();
                if ready.send((index, mask)).is_err() {
                    return;
                }
                if !worker_gate.wait() {
                    return;
                }
                let value = match store.invoke(function, &arguments) {
                    Ok(RunOutcome::Returned(values)) => match values.as_slice() {
                        [Value::I32(n)] => Ok(*n),
                        _ => Err("shared copy worker returned wrong values".into()),
                    },
                    Ok(_) => Err("shared copy worker did not return".into()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = result.send((index, value));
            });
            match worker {
                Ok(worker) => handles.push(worker),
                Err(e) => {
                    gate.release(false);
                    return Err(e.to_string());
                }
            }
        }
        drop(ready_tx);
        drop(result_tx);
        let mut masks = vec![Vec::new(); count];
        for _ in 0..count {
            match ready_rx.recv_timeout(TIMEOUT) {
                Ok((index, Ok(mask))) if index < count => masks[index] = mask,
                other => {
                    gate.release(false);
                    return Err(format!("shared worker readiness: {other:?}"));
                }
            }
        }
        // Every worker finished setup and read its actual inherited affinity.
        let start = Instant::now();
        gate.release(true);
        let mut values = vec![0; count];
        let mut failure = None;
        for _ in 0..count {
            match result_rx.recv_timeout(TIMEOUT) {
                Ok((index, Ok(value))) if index < count => values[index] = value,
                other => {
                    failure = Some(format!("shared worker result: {other:?}"));
                    break;
                }
            }
        }
        let elapsed = start.elapsed().as_nanos().max(1);
        if failure.is_some() {
            for token in &cancellations {
                token.cancel();
            }
        }
        for worker in handles {
            if worker.join().is_err() {
                failure.get_or_insert("shared worker panicked".into());
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok((elapsed, values, masks))
    })
}

fn memory(store: &mut Store) -> Result<(wtime_runtime::Memory, ShareableMemory), String> {
    let memory = store
        .create_memory(MemoryType {
            limits: Limits::new(4, Some(4)),
            shared: true,
            ..MemoryType::default()
        })
        .map_err(|e| e.to_string())?;
    let token = store.share_memory(memory).map_err(|e| e.to_string())?;
    Ok((memory, token))
}
fn pattern(destination: bool) -> Vec<u8> {
    (0..MEMORY_BYTES)
        .map(|i| {
            if destination {
                ((i * 29 + 7) % 253) as u8
            } else {
                ((i * 17 + 31) % 251) as u8
            }
        })
        .collect()
}
struct Observation {
    elapsed: u128,
    returns: Vec<i32>,
    masks: Vec<Vec<usize>>,
    images: Vec<(&'static str, Vec<u8>)>,
}
fn execute(bytes: &[u8], request: &Request) -> Result<Observation, String> {
    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        "decoded",
    );
    let module = engine.compile(bytes).map_err(|e| e.to_string())?;
    let mut observation = Observation {
        elapsed: 0,
        returns: vec![],
        masks: vec![],
        images: vec![],
    };
    for _ in 0..request.batch {
        let mut owner = Store::new(&engine);
        let (source, source_token) = memory(&mut owner)?;
        let (destination, destination_token) = if request.alias {
            (source, source_token.clone())
        } else {
            memory(&mut owner)?
        };
        let source_image = pattern(false);
        let mut expected = if request.alias {
            source_image.clone()
        } else {
            pattern(true)
        };
        owner
            .write_memory(source, 0, &source_image)
            .map_err(|e| e.to_string())?;
        if !request.alias {
            owner
                .write_memory(destination, 0, &expected)
                .map_err(|e| e.to_string())?;
        }
        let mut contexts = Vec::new();
        for worker in 0..request.workers {
            let mut store = Store::new(&engine);
            let mut linker = Linker::new(&engine);
            for (name, token) in [("src", &source_token), ("dst", &destination_token)] {
                let imported = store
                    .import_shared_memory(token)
                    .map_err(|e| e.to_string())?;
                linker
                    .define("env", name, External::Memory(imported))
                    .map_err(|e| e.to_string())?;
            }
            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(|e| e.to_string())?;
            let External::Func(function) =
                instance.export(&store, "run").map_err(|e| e.to_string())?
            else {
                return Err("shared copy export is not a function".into());
            };
            let destination_offset = 65536 + worker * 32768;
            expected[destination_offset..destination_offset + request.bytes]
                .copy_from_slice(&source_image[..request.bytes]);
            contexts.push((
                store,
                function,
                vec![
                    Value::I32(destination_offset as i32),
                    Value::I32(0),
                    Value::I32(request.bytes as i32),
                    Value::I32(REPETITIONS),
                ],
            ));
        }
        let (elapsed, returns, masks) = run_group(contexts)?;
        if returns != vec![REPETITIONS; request.workers] {
            return Err("shared copy return oracle mismatch".into());
        }
        if masks.iter().any(|mask| mask != &masks[0]) {
            return Err("worker CPU masks differ".into());
        }
        if !observation.masks.is_empty() && observation.masks != masks {
            return Err("group CPU masks changed".into());
        }
        observation.masks = masks;
        let actual = owner
            .read_memory(destination, 0, MEMORY_BYTES)
            .map_err(|e| e.to_string())?;
        if actual != expected {
            return Err("shared copy full destination image differs".into());
        }
        observation.images = if request.alias {
            vec![("shared", actual)]
        } else {
            let unchanged = owner
                .read_memory(source, 0, MEMORY_BYTES)
                .map_err(|e| e.to_string())?;
            if unchanged != source_image {
                return Err("shared copy changed source backing".into());
            }
            vec![("source", unchanged), ("destination", actual)]
        };
        observation.elapsed = observation
            .elapsed
            .checked_add(elapsed)
            .ok_or("shared copy timer overflow")?;
        observation.returns.extend(returns);
    }
    Ok(observation)
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--shared-bulk-copy") {
        return false;
    }
    let request = parse(args).expect("invalid shared bulk request");
    assert!(!cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference"));
    assert!(std::fs::metadata(&args[1]).unwrap().len() <= 4 * 1024 * 1024);
    let bytes = std::fs::read(&args[1]).unwrap();
    let observed = execute(&bytes, &request).expect("shared bulk lifecycle failed");
    let images = observed
        .images
        .iter()
        .map(|(name, bytes)| format!("{}:{}", super::quote(name), super::quote(&hex(bytes))))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        concat!(
            "{{\"schema\":\"wtime-shared-bulk-copy-v1\",\"schema_version\":1,",
            "\"case\":{},\"mode\":\"decoded\",\"profile\":\"all-phase3-core\",",
            "\"stage\":\"steady_state_group\",\"backing\":{},\"workers\":{},\"bytes\":{},",
            "\"repetitions\":32,\"batch\":{},\"instrumented\":false,\"report_complete\":true,",
            "\"elapsed_ns\":{},\"calls_checked\":{},\"every_group_full_memory_checked\":true,",
            "\"worker_returns\":{:?},\"worker_affinity\":{:?},\"backings_hex\":{{{}}}}}"
        ),
        super::quote(&request.case),
        super::quote(if request.alias { "alias" } else { "distinct" }),
        request.workers,
        request.bytes,
        request.batch,
        observed.elapsed,
        observed.returns.len(),
        observed.returns,
        observed.masks,
        images
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../fixtures/threads/shared_bulk_copy.wasm");

    #[test]
    fn p078_driver_rejects_invalid_shape_before_io() {
        let good = [
            "--shared-bulk-copy",
            "nonexistent.wasm",
            "shared_bulk_copy_alias_w4_b512",
            "alias",
            "4",
            "512",
            "steady_state_group",
            "2",
            "decoded",
            "all-phase3-core",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(parse(&good).unwrap().batch, 2);
        let mut maximum = good.clone();
        maximum[7] = MAX_BATCH.to_string();
        assert_eq!(parse(&maximum).unwrap().batch, MAX_BATCH);
        for (index, value) in [
            (2, "shared_bulk_copy_alias_w1_b512"),
            (3, "mixed"),
            (4, "0"),
            (4, "5"),
            (5, "0"),
            (5, "32769"),
            (6, "warm_call"),
            (7, "0"),
            (7, "1025"),
            (7, "18446744073709551616"),
            (8, "register"),
            (9, "mvp"),
        ] {
            let mut invalid = good.clone();
            invalid[index] = value.into();
            assert!(parse(&invalid).is_err(), "{index} {value}");
        }
        let mut extra = good.clone();
        extra.push("extra".into());
        assert!(parse(&extra).is_err());
        assert!(parse(&good[..9]).is_err());
    }

    #[test]
    fn p078_driver_checks_full_images_and_worker_returns() {
        for alias in [false, true] {
            for workers in [1, 4] {
                for bytes in [512, 32768, 4097] {
                    let request = Request {
                        case: "unit-test".into(),
                        alias,
                        workers,
                        bytes,
                        batch: 2,
                    };
                    let observed = execute(FIXTURE, &request).unwrap();
                    assert_eq!(observed.returns, vec![32; workers * 2]);
                    assert_eq!(observed.images.len(), if alias { 1 } else { 2 });
                    assert!(
                        observed
                            .images
                            .iter()
                            .all(|(_, bytes)| bytes.len() == MEMORY_BYTES)
                    );
                    assert_eq!(observed.masks.len(), workers);
                    assert!(
                        observed
                            .masks
                            .iter()
                            .all(|mask| !mask.is_empty() && mask == &observed.masks[0])
                    );
                    assert!(observed.elapsed > 0);
                }
            }
        }
    }

    #[test]
    fn p078_driver_detects_corrupted_copy_fixture() {
        for (needle, argument, message) in [
            (&[0x20, 2][..], 1, "destination image"),
            (&[0x20, 4, 0x0b][..], 2, "return oracle"),
        ] {
            let positions: Vec<_> = FIXTURE
                .windows(needle.len())
                .enumerate()
                .filter_map(|(index, bytes)| (bytes == needle).then_some(index))
                .collect();
            assert_eq!(positions.len(), 1);
            let mut corrupted = FIXTURE.to_vec();
            corrupted[positions[0] + 1] = argument;
            let request = Request {
                case: "unit-test".into(),
                alias: false,
                workers: 4,
                bytes: 512,
                batch: 2,
            };
            let error = execute(&corrupted, &request)
                .err()
                .expect("corrupted guest passed its oracle");
            assert!(error.contains(message), "{error}");
        }
    }
}
