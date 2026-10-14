//! Draft P013 host-thread driver. Integrate as a private benchmark module.
//! Setup/instantiation happens before each measured start barrier.
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use wtime_runtime::{
    Config, Engine, External, Linker, Module, RunOutcome, ShareableMemory, Store, Value,
};
use wtime_types::{Limits, MemoryType};

const JOIN_TIMEOUT: Duration = Duration::from_secs(30);

fn error(message: impl Into<String>) -> String {
    message.into()
}

fn enabled_engine() -> Result<Engine, String> {
    let mut config = Config::default();
    config.features.threads = true;
    config.limits.fuel = Some(50_000_000);
    config.limits.max_module_bytes = 1 << 20;
    Engine::new(config).map_err(|e| error(format!("threads engine setup failed: {e}")))
}

fn compile(engine: &Engine, file: &str) -> Result<Module, String> {
    if std::fs::metadata(file).map_err(|e| e.to_string())?.len() > 1 << 20 {
        return Err("thread fixture exceeds bound".into());
    }
    let bytes = std::fs::read(file).map_err(|e| error(format!("read {file}: {e}")))?;
    engine
        .compile(&bytes)
        .map_err(|e| error(format!("compile {file}: {e}")))
}

fn make_shared_memory(
    store: &mut Store,
) -> Result<(wtime_runtime::Memory, ShareableMemory), String> {
    let memory = store
        .create_memory(MemoryType {
            limits: Limits::new(1, Some(1)),
            shared: true,
            ..MemoryType::default()
        })
        .map_err(|e| error(format!("create shared memory: {e}")))?;
    let token = store
        .share_memory(memory)
        .map_err(|e| error(format!("share memory: {e}")))?;
    Ok((memory, token))
}

fn bind(
    engine: &Engine,
    module: &Module,
    imports: &[(&str, ShareableMemory)],
    export: &str,
) -> Result<(Store, wtime_runtime::Func), String> {
    let mut store = Store::new(engine);
    let mut linker = Linker::new(engine);
    for (name, token) in imports {
        let memory = store
            .import_shared_memory(token)
            .map_err(|e| error(format!("import shared memory {name}: {e}")))?;
        linker
            .define("env", name, External::Memory(memory))
            .map_err(|e| error(format!("define shared memory {name}: {e}")))?;
    }
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| error(format!("instantiate thread worker: {e}")))?;
    let External::Func(func) = instance
        .export(&store, export)
        .map_err(|e| error(format!("missing thread export {export}: {e}")))?
    else {
        return Err(format!("thread export {export} is not a function"));
    };
    Ok((store, func))
}

fn returned_i32(
    store: &mut Store,
    func: wtime_runtime::Func,
    args: &[Value],
) -> Result<i32, String> {
    match store
        .invoke(func, args)
        .map_err(|e| error(format!("guest call failed: {e}")))?
    {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => Ok(*value),
            other => Err(format!("guest returned unexpected values: {other:?}")),
        },
        other => Err(format!("guest returned unexpected outcome: {other:?}")),
    }
}

fn reset_words(store: &mut Store, memories: &[wtime_runtime::Memory]) -> Result<(), String> {
    for memory in memories {
        store
            .write_memory(*memory, 0, &[0; 8])
            .map_err(|e| error(format!("reset shared memory: {e}")))?;
    }
    Ok(())
}

/// One host-thread group. Setup/readback/join are outside the measured interval.
/// Timing starts at coordinator barrier release and stops on the last worker result.
fn invoke_group(
    contexts: Vec<(Store, wtime_runtime::Func)>,
    args: Vec<Value>,
    skew_seed: u64,
) -> Result<(u128, Vec<i32>), String> {
    let cancellation: Vec<_> = contexts
        .iter()
        .map(|(store, _)| store.cancellation_token())
        .collect();
    let workers = contexts.len();
    if workers == 0 {
        return Err("thread group cannot be empty".into());
    }
    let gate = Arc::new(Barrier::new(workers + 1));
    let enabled = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel::<(usize, Result<i32, String>)>();
    let outcome = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for (index, (mut store, func)) in contexts.into_iter().enumerate() {
            let gate = gate.clone();
            let enabled = enabled.clone();
            let tx = tx.clone();
            let args = args.clone();
            let mut skew = skew_seed ^ ((index as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            handles.push(scope.spawn(move || {
                gate.wait();
                if !enabled.load(Ordering::Acquire) {
                    return;
                }
                // Deterministic, bounded scheduling perturbation; guest inputs do not change.
                skew ^= skew << 13;
                skew ^= skew >> 7;
                skew ^= skew << 17;
                for _ in 0..(skew % 3) {
                    thread::yield_now();
                }
                let result = returned_i32(&mut store, func, &args);
                let _ = tx.send((index, result));
            }));
        }
        drop(tx);
        let started = Instant::now();
        gate.wait();
        let mut results = vec![None; workers];
        let mut error = None;
        for _ in 0..workers {
            match rx.recv_timeout(JOIN_TIMEOUT) {
                Ok((index, result)) if index < workers => results[index] = Some(result),
                Ok((index, _)) => error = Some(format!("invalid worker index {index}")),
                Err(e) => {
                    error = Some(format!("worker result timeout/disconnect: {e}"));
                    break;
                }
            }
        }
        let elapsed = started.elapsed().as_nanos();
        if error.is_some() {
            for token in &cancellation {
                token.cancel();
            }
        }
        for (index, handle) in handles.into_iter().enumerate() {
            if handle.join().is_err() {
                error.get_or_insert_with(|| format!("worker {index} panicked"));
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        let values = results
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                item.ok_or_else(|| format!("worker {index} produced no result"))?
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((elapsed, values))
    });
    outcome
}

fn read_words(store: &Store, memories: &[wtime_runtime::Memory]) -> Result<Vec<i32>, String> {
    memories
        .iter()
        .map(|memory| {
            let bytes = store
                .read_memory(*memory, 0, 4)
                .map_err(|e| error(format!("read final shared memory: {e}")))?;
            Ok(i32::from_le_bytes(
                bytes.try_into().map_err(|_| "memory word length")?,
            ))
        })
        .collect()
}

fn atomic_trial(
    file: &str,
    workload: &str,
    workers: usize,
    iterations: i32,
    seed: u64,
) -> Result<(u128, Vec<i32>, Vec<i32>), String> {
    let engine = enabled_engine()?;
    let module = compile(&engine, file)?;
    let mut coordinator = Store::new(&engine);
    let distinct = workload.contains("_distinct_");
    let memories_count = if distinct { workers } else { 1 };
    let mut memories = Vec::with_capacity(memories_count);
    let mut tokens = Vec::with_capacity(memories_count);
    for _ in 0..memories_count {
        let (memory, token) = make_shared_memory(&mut coordinator)?;
        memories.push(memory);
        tokens.push(token);
    }
    reset_words(&mut coordinator, &memories)?;

    let mut contexts = Vec::with_capacity(workers);
    for worker in 0..workers {
        let token = if distinct {
            tokens[worker].clone()
        } else {
            tokens[0].clone()
        };
        contexts.push(bind(&engine, &module, &[("memory", token)], "run")?);
    }
    let (wall_ns, returns) = invoke_group(contexts, vec![Value::I32(iterations)], seed)?;
    let final_memory = read_words(&coordinator, &memories)?;
    let expected_worker = vec![iterations; workers];
    if returns != expected_worker {
        return Err(format!(
            "worker returns {returns:?}, expected {expected_worker:?}"
        ));
    }
    let expected_memory = if distinct {
        vec![iterations; workers]
    } else {
        vec![(iterations as i64 * workers as i64) as i32]
    };
    if final_memory != expected_memory {
        return Err(format!(
            "final memory {final_memory:?}, expected {expected_memory:?}"
        ));
    }
    Ok((wall_ns, returns, final_memory))
}

fn sc_trial(file: &str, seed: u64) -> Result<(u128, [i32; 2]), String> {
    let engine = enabled_engine()?;
    let module = compile(&engine, file)?;
    let mut coordinator = Store::new(&engine);
    let (x_mem, x_token) = make_shared_memory(&mut coordinator)?;
    let (y_mem, y_token) = make_shared_memory(&mut coordinator)?;
    reset_words(&mut coordinator, &[x_mem, y_mem])?;
    let left = bind(
        &engine,
        &module,
        &[("x", x_token.clone()), ("y", y_token.clone())],
        "store_x_load_y",
    )?;
    let right = bind(
        &engine,
        &module,
        &[("x", x_token), ("y", y_token)],
        "store_y_load_x",
    )?;
    let (wall_ns, values) = invoke_group(vec![left, right], vec![], seed)?;
    let pair: [i32; 2] = values
        .try_into()
        .map_err(|_| "SC worker return count mismatch")?;
    if pair == [0, 0] {
        return Err("forbidden SC store-buffering result [0,0]".into());
    }
    if pair.iter().any(|value| !matches!(value, 0 | 1)) {
        return Err(format!("invalid SC result {pair:?}"));
    }
    Ok((wall_ns, pair))
}

fn run_thread_case(args: &[String]) -> Result<String, String> {
    if args.len() != 7 {
        return Err("--thread-case FILE CASE WORKERS ITERATIONS TRIALS SEED".into());
    }
    let (file, workload) = (&args[1], &args[2]);
    let requested = args[3]
        .parse::<usize>()
        .map_err(|_| "workers must be integer")?;
    let quota = thread::available_parallelism().map_or(1, |count| count.get());
    let workers = requested.min(quota).min(4);
    let iterations = args[4]
        .parse::<i32>()
        .map_err(|_| "iterations must be i32")?;
    let trials = args[5]
        .parse::<usize>()
        .map_err(|_| "trials must be integer")?;
    let mut seed = args[6].parse::<u64>().map_err(|_| "seed must be integer")?;
    if ![1, 2, 4].contains(&requested)
        || ![32, 1024].contains(&iterations)
        || trials == 0
        || trials > 4096
        || workers == 0
        || seed == 0
    {
        return Err("invalid worker/iteration/trial configuration".into());
    }
    if (workload.contains("ordinary_rw") || workload.contains("atomic_load_store")) && workers != 1
    {
        return Err(
            "single-worker control was quota-expanded or requested with multiple workers".into(),
        );
    }
    let mut walls = Vec::with_capacity(trials);
    let mut returns = Vec::with_capacity(trials);
    let mut memory = Vec::with_capacity(trials);
    let mut trial_seeds = Vec::with_capacity(trials);
    for _ in 0..trials {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        trial_seeds.push(seed);
        let (wall, worker_values, memory_values) =
            atomic_trial(file, workload, workers, iterations, seed)?;
        walls.push(wall);
        returns.push(worker_values);
        memory.push(memory_values);
    }
    let total_ops = (workers as u64)
        .checked_mul(iterations as u64)
        .ok_or("operation count overflow")?;
    Ok(format!(
        "{{\"schema_version\":1,\"status\":\"ok\",\"case\":\"{}\",\"requested_workers\":{},\"effective_workers\":{},\"worker_quota\":{},\"iterations_per_worker\":{},\"total_operations_per_trial\":{},\"trials\":{},\"seed\":{},\"trial_seeds\":{:?},\"trial_wall_ns\":{:?},\"trial_worker_returns\":{:?},\"trial_final_memory_values\":{:?},\"timing_boundary\":\"start_barrier_to_last_worker_result_receive; setup/readback/join excluded\"}}",
        workload,
        requested,
        workers,
        quota,
        iterations,
        total_ops,
        trials,
        args[6],
        trial_seeds,
        walls,
        returns,
        memory
    ))
}

fn run_sc_litmus(args: &[String]) -> Result<String, String> {
    if args.len() != 4 {
        return Err("--sc-litmus FILE TRIALS SEED".into());
    }
    let file = &args[1];
    let trials = args[2]
        .parse::<usize>()
        .map_err(|_| "trials must be integer")?;
    let mut seed = args[3].parse::<u64>().map_err(|_| "seed must be integer")?;
    if trials == 0 || trials > 4096 || seed == 0 {
        return Err("SC trials require bounded nonzero trials/seed".into());
    }
    let mut walls = Vec::with_capacity(trials);
    let mut results = Vec::with_capacity(trials);
    let mut trial_seeds = Vec::with_capacity(trials);
    for _ in 0..trials {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        trial_seeds.push(seed);
        let (wall, result) = sc_trial(file, seed)?;
        walls.push(wall);
        results.push(result);
    }
    Ok(format!(
        "{{\"schema_version\":1,\"status\":\"ok\",\"case\":\"threads_sc_store_buffering\",\"trials\":{},\"trial_seeds\":{:?},\"trial_wall_ns\":{:?},\"trial_results\":{:?},\"forbidden_result\":[0,0],\"memories_per_trial\":2,\"reset_per_trial\":true}}",
        trials, trial_seeds, walls, results
    ))
}

fn run_wait_latency(args: &[String]) -> Result<String, String> {
    if args.len() != 4 {
        return Err("--wait-latency FILE SAMPLES TIMEOUT_NS".into());
    }
    let file = &args[1];
    let samples = args[2]
        .parse::<usize>()
        .map_err(|_| "samples must be integer")?;
    let timeout_ns = args[3]
        .parse::<i64>()
        .map_err(|_| "timeout must be i64 nanoseconds")?;
    if samples < 100 || samples > 4096 || timeout_ns <= 0 || timeout_ns > 5_000_000_000 {
        return Err("wait latency requires at least 100 samples and a positive timeout".into());
    }
    let engine = enabled_engine()?;
    let module = compile(&engine, file)?;
    let timeout = Duration::from_nanos(timeout_ns as u64);
    let mut latency_ns = Vec::with_capacity(samples);
    for sample in 0..samples {
        let mut coordinator = Store::new(&engine);
        let (_memory, token) = make_shared_memory(&mut coordinator)?;
        let notifier_import = token.clone();
        let waiter_import = token;
        let (mut notify_store, notify_func) =
            bind(&engine, &module, &[("memory", notifier_import)], "notify")?;
        let (mut wait_store, wait_func) =
            bind(&engine, &module, &[("memory", waiter_import)], "wait")?;
        let cancellation = wait_store.cancellation_token();
        let (send, receive) = mpsc::channel::<(Result<i32, String>, Instant)>();
        let waiter = thread::spawn(move || {
            let result = returned_i32(
                &mut wait_store,
                wait_func,
                &[Value::I32(0), Value::I64(timeout_ns)],
            );
            let returned_at = Instant::now();
            let _ = send.send((result, returned_at));
        });
        let deadline = Instant::now() + timeout;
        // Always join the waiter, including on protocol errors. This keeps one
        // malformed sample from silently detaching a worker into later samples.
        let attempt = (|| -> Result<u128, String> {
            loop {
                match receive.try_recv() {
                    Ok((result, _)) => {
                        return Err(format!(
                            "waiter ended before a successful notify in sample {sample}: {result:?}"
                        ));
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err(format!(
                            "waiter disconnected before notify in sample {sample}"
                        ));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                if Instant::now() >= deadline {
                    return Err(format!("waiter registration timed out in sample {sample}"));
                }
                // Failed notify(1)==0 probes prove nothing and are outside the accepted
                // interval. A return count of one proves a waiter was registered at the
                // successful call's linearization point.
                let notify_call_start = Instant::now();
                let notified = returned_i32(&mut notify_store, notify_func, &[Value::I32(1)])?;
                if notified == 1 {
                    let (wait_result, waiter_return) = receive
                        .recv_timeout(timeout)
                        .map_err(|e| format!("waiter did not return after notify: {e}"))?;
                    let wait_result = wait_result?;
                    if wait_result != 0 {
                        return Err(format!("notified wait returned {wait_result}, expected 0"));
                    }
                    let elapsed = waiter_return
                        .checked_duration_since(notify_call_start)
                        .ok_or("waiter return timestamp preceded notify invocation")?;
                    return Ok(elapsed.as_nanos());
                }
                if notified != 0 {
                    return Err(format!("notify returned {notified}, expected 0 or 1"));
                }
                thread::yield_now();
            }
        })();
        if attempt.is_err() {
            cancellation.cancel();
        }
        let joined = waiter
            .join()
            .map_err(|_| format!("waiter panicked in sample {sample}"));
        let sample_ns = attempt?;
        joined?;
        latency_ns.push(sample_ns);
    }
    Ok(format!(
        "{{\"schema_version\":1,\"status\":\"ok\",\"case\":\"threads_wait_notify_latency\",\"sample_kind\":\"one_wait_notify_round_trip\",\"samples\":{},\"sample_count\":{},\"registration_observed\":true,\"registration_evidence\":\"successful notify returned one for the address before waiter return\",\"outcome\":\"notified\",\"timeout_ns\":{},\"notify_to_return_ns\":{:?},\"p99_requires_individual_samples\":100,\"cancellation_cleanup\":\"unverified_public_registration_observer_unavailable\"}}",
        samples,
        latency_ns.len(),
        timeout_ns,
        latency_ns
    ))
}

fn run_wait_cleanup(args: &[String]) -> Result<String, String> {
    if args.len() != 3 {
        return Err("--wait-cleanup FILE TIMEOUT_NS".into());
    }
    let timeout_ns = args[2].parse::<i64>().map_err(|_| "invalid timeout")?;
    if !(1_000_000..=100_000_000).contains(&timeout_ns) {
        return Err("cleanup timeout must be 1..100 ms".into());
    }
    let engine = enabled_engine()?;
    let module = compile(&engine, &args[1])?;
    let mut owner = Store::new(&engine);
    let (_, token) = make_shared_memory(&mut owner)?;
    let (mut notifier, notify) = bind(&engine, &module, &[("memory", token.clone())], "notify")?;
    let (mut waiter, wait) = bind(&engine, &module, &[("memory", token.clone())], "wait")?;
    let mismatch = returned_i32(&mut waiter, wait, &[Value::I32(1), Value::I64(timeout_ns)])?;
    let zero = returned_i32(&mut waiter, wait, &[Value::I32(0), Value::I64(0)])?;
    if mismatch != 1 || zero != 2 {
        return Err("wrong immediate wait outcomes".into());
    }
    let started = Instant::now();
    let timed = returned_i32(&mut waiter, wait, &[Value::I32(0), Value::I64(timeout_ns)])?;
    let elapsed = started.elapsed().as_nanos();
    if timed != 2 || elapsed < timeout_ns as u128 {
        return Err("finite wait did not actually time out".into());
    }
    let timeout_after = returned_i32(&mut notifier, notify, &[Value::I32(1)])?;
    if timeout_after != 0 {
        return Err("timeout retained wait registration".into());
    }
    let (mut waiter, wait) = bind(&engine, &module, &[("memory", token)], "wait")?;
    let cancel = waiter.cancellation_token();
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = send.send(());
        waiter.invoke(wait, &[Value::I32(0), Value::I64(-1)])
    });
    if receive.recv_timeout(JOIN_TIMEOUT).is_err() {
        cancel.cancel();
        let _ = worker.join();
        return Err("cancel waiter did not start".into());
    }
    // This bounded delay exercises cancellation of an indefinite wait. No public
    // registration observer exists; the report does not claim exact entry ordering.
    thread::sleep(Duration::from_millis(2));
    cancel.cancel();
    let outcome = worker.join().map_err(|_| "cancel waiter panic")?;
    let error = outcome
        .map_err(|e| e)
        .expect_err("cancelled wait must fail");
    if error.kind != wtime_runtime::ErrorKind::ResourceLimit || !error.message.contains("cancel") {
        return Err(format!("wrong cancellation outcome: {error:?}"));
    }
    let cancel_after = returned_i32(&mut notifier, notify, &[Value::I32(1)])?;
    if cancel_after != 0 {
        return Err("cancelled wait retained registration".into());
    }
    Ok(format!(
        "{{\"schema_version\":1,\"status\":\"ok\",\"case\":\"threads_wait_cleanup\",\"timeout_ns\":{timeout_ns},\"timeout_elapsed_ns\":{elapsed},\"timeout_result\":{timed},\"value_mismatch_result\":{mismatch},\"zero_timeout_result\":{zero},\"timeout_notify_after\":{timeout_after},\"cancellation_error_kind\":\"ResourceLimit\",\"cancellation_message\":{},\"cancellation_notify_after\":{cancel_after},\"registration_observer_available\":false}}",
        super::quote(&error.message)
    ))
}

pub fn try_main(args: &[String]) -> bool {
    if args.first().is_some_and(|s| {
        matches!(
            s.as_str(),
            "--thread-case" | "--sc-litmus" | "--wait-latency" | "--wait-cleanup"
        )
    }) {
        assert!(
            !cfg!(any(
                feature = "performance-counters",
                feature = "execution-reference"
            )),
            "thread timing requires counters disabled"
        );
    }
    let result = match args.first().map(String::as_str) {
        Some("--thread-case") => Some(run_thread_case(args)),
        Some("--sc-litmus") => Some(run_sc_litmus(args)),
        Some("--wait-latency") => Some(run_wait_latency(args)),
        Some("--wait-cleanup") => Some(run_wait_cleanup(args)),
        _ => None,
    };
    if let Some(result) = result {
        match result {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("thread driver: {error}");
                std::process::exit(2);
            }
        }
        true
    } else {
        false
    }
}
