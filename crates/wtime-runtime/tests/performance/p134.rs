#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use std::sync::{Arc, Barrier, Mutex};
use wtime_runtime::*;

fn engine(kind: usize, fuel: Option<u64>) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    match kind {
        0 => Engine::new(config).unwrap(),
        1 => Engine::with_execution_policy(config, ExecutionPolicy::Register).unwrap(),
        2 => Engine::with_register_entry_threshold(config, 1).unwrap(),
        _ => Engine::with_register_entry_threshold(config, 3).unwrap(),
    }
}
fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!()
    };
    function
}
fn mem(store: &Store, instance: Instance) -> Memory {
    let External::Memory(memory) = instance.export(store, "memory").unwrap() else {
        panic!()
    };
    memory
}
fn view(result: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    result.map_err(|error| (error.kind, error.message))
}

#[test]
fn p134_cold_hot_entry_threshold_and_diagnostics() {
    assert!(Engine::with_register_entry_threshold(Config::default(), 0).is_err());
    let engine = Engine::with_register_entry_threshold(Config::default(), 3).unwrap();
    assert_eq!(engine.execution_policy(), ExecutionPolicy::Register);
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let function = func(&store, instance, "i32_eqz");
    for entry in 1..=4 {
        let before = store.register_dispatches();
        assert_eq!(
            store.invoke(function, &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(1)])
        );
        assert_eq!(
            store.register_dispatches() - before,
            if entry < 3 { 0 } else { 3 }
        );
    }
    assert_eq!(store.register_fallback_frames(), 2);
    assert_eq!(store.register_exits(), 0);
    for fuel in 0..=3 {
        let mut results = Vec::new();
        for kind in 0..4 {
            let engine = self::engine(kind, Some(fuel));
            let module = engine
                .compile(include_bytes!(
                    "../fixtures/performance/p061/integer_ops.wasm"
                ))
                .unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            results.push(view(
                store.invoke(func(&store, instance, "i32_eqz"), &[Value::I32(0)]),
            ));
        }
        assert!(results.iter().all(|result| result == &results[0]));
    }
    eprintln!(
        "P134_CASE {{\"name\":\"p134_cold_hot_entry_threshold_and_diagnostics\",\"passed\":true,\"threshold\":3,\"cold_fallback_frames\":2,\"hot_dispatches\":6,\"fuel_prefixes\":4,\"zero_threshold_refused\":true}}"
    );
}

#[test]
fn p134_shared_module_concurrency_and_store_ownership() {
    let engine = engine(3, Some(100));
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p064/direct_call.wasm"
        ))
        .unwrap();
    let barrier = Barrier::new(2);
    let observations = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for input in [21, 37] {
            let engine = engine.clone();
            let module = module.clone();
            let barrier = &barrier;
            workers.push(scope.spawn(move || {
                let mut store = Store::new(&engine);
                let instance = Linker::new(&engine)
                    .instantiate(&mut store, &module)
                    .unwrap();
                let function = func(&store, instance, "run");
                let memory = mem(&store, instance);
                barrier.wait();
                for _ in 0..20 {
                    assert_eq!(
                        store.invoke(function, &[Value::I32(input)]).unwrap(),
                        RunOutcome::Returned(vec![Value::I32(3 * (input + 1))])
                    );
                    assert_eq!(
                        store.read_memory(memory, 8, 4).unwrap(),
                        input.to_le_bytes()
                    );
                }
                (
                    store.register_dispatches(),
                    store.read_memory(memory, 8, 4).unwrap(),
                )
            }));
        }
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        observations
            .iter()
            .map(|observation| observation.0)
            .sum::<u64>()
            > 0
    );
    assert_eq!(observations[0].1, 21i32.to_le_bytes());
    assert_eq!(observations[1].1, 37i32.to_le_bytes());
    let foreign = self::engine(2, None);
    let mut foreign_store = Store::new(&foreign);
    assert!(
        Linker::new(&foreign)
            .instantiate(&mut foreign_store, &module)
            .is_err()
    );
    let mut first = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut first, &module)
        .unwrap();
    assert!(
        foreign_store
            .invoke(func(&first, instance, "run"), &[Value::I32(21)])
            .is_err()
    );
    eprintln!(
        "P134_CASE {{\"name\":\"p134_shared_module_concurrency_and_store_ownership\",\"passed\":true,\"stores\":2,\"calls\":40,\"optimized_path_ran\":true,\"isolated_memory\":true,\"foreign_store_refused\":true}}"
    );
}

fn host_observation(
    kind: usize,
    fuel: u64,
    cancel: bool,
) -> (
    std::result::Result<RunOutcome, (ErrorKind, String)>,
    Vec<u8>,
    usize,
) {
    let engine = engine(kind, Some(fuel));
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p064/host_call.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let handles: Arc<Mutex<Option<(Func, Memory)>>> = Arc::new(Mutex::new(None));
    let count = Arc::new(Mutex::new(0));
    let host_handles = handles.clone();
    let host_count = count.clone();
    let callback = store
        .create_host_func_with_caller(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            move |caller, args| {
                *host_count.lock().unwrap() += 1;
                assert_eq!(args, &[Value::I32(22)]);
                let (child, memory) = host_handles.lock().unwrap().unwrap();
                assert_eq!(
                    caller.store().read_memory(memory, 8, 4)?,
                    21i32.to_le_bytes()
                );
                let snapshot = caller.store().execution_snapshot().unwrap();
                assert_eq!(snapshot.cursor.source_pc, 7);
                assert_eq!(snapshot.remaining_fuel, Some(fuel - 8));
                assert!(snapshot.pending_boundary_charged);
                assert_eq!(
                    snapshot.frames[0].locals,
                    [DiagnosticValue::I32(21), DiagnosticValue::I32(22)]
                );
                caller.collect()?;
                assert_eq!(
                    caller.invoke(child, args)?,
                    RunOutcome::Returned(vec![Value::I32(44)])
                );
                caller
                    .store()
                    .write_memory(memory, 8, &99i32.to_le_bytes())?;
                if cancel {
                    caller.store().cancellation_token().cancel();
                }
                Ok(vec![Value::I32(44)])
            },
        )
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker
        .define("env", "callback", External::Func(callback))
        .unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let memory = mem(&store, instance);
    *handles.lock().unwrap() = Some((func(&store, instance, "child"), memory));
    let run = func(&store, instance, "run");
    store.write_memory(memory, 0, &vec![0x5a; 65536]).unwrap();
    // Cold mode must pass its threshold before the measured semantic invocation.
    if kind == 3 {
        for _ in 0..2 {
            store.request_materialization_roundtrip(run, 7).unwrap();
            let _ = store.invoke(run, &[Value::I32(21)]);
        }
        *count.lock().unwrap() = 0;
        store.write_memory(memory, 0, &vec![0x5a; 65536]).unwrap();
    }
    store.request_materialization_roundtrip(run, 7).unwrap();
    let dispatches = store.register_dispatches();
    let result = view(store.invoke(run, &[Value::I32(21)]));
    if kind >= 2 && fuel >= 7 {
        assert!(store.register_dispatches() - dispatches >= 7);
    }
    let calls = *count.lock().unwrap();
    let image = store.read_memory(memory, 0, 65536).unwrap();
    (result, image, calls)
}

#[test]
fn p134_host_effect_fuel_and_foreign_refusal() {
    for fuel in 0..=11 {
        let reference = host_observation(0, fuel, false);
        for kind in 1..=3 {
            assert_eq!(host_observation(kind, fuel, false), reference);
        }
    }
    let reference = host_observation(0, 100, true);
    assert_eq!(
        reference.0,
        Err((ErrorKind::ResourceLimit, "execution cancelled".into()))
    );
    assert_eq!(reference.2, 1);
    // Threshold-one avoids cancellation of preliminary calls.
    assert_eq!(host_observation(2, 100, true), reference);
    let success = host_observation(2, 100, false);
    assert_eq!(success.0, Ok(RunOutcome::Returned(vec![Value::I32(66)])));
    assert_eq!(success.2, 1);
    let mut expected = vec![0x5a; 65536];
    expected[8..12].copy_from_slice(&99i32.to_le_bytes());
    assert_eq!(success.1, expected);
    eprintln!(
        "P134_CASE {{\"name\":\"p134_host_effect_fuel_and_foreign_refusal\",\"passed\":true,\"fuel_prefixes\":12,\"policies\":4,\"callback_once\":true,\"reentry_gc_and_memory_preserved\":true,\"cancellation_preserved\":true}}"
    );
}

#[test]
fn p134_eviction_pin_and_continuation_resume_without_reentry() {
    for exact in [false, true] {
        for kind in 0..4 {
            let engine = engine(kind, Some(100));
            let provider = engine
                .compile(if exact {
                    include_bytes!("../fixtures/performance/p065/exact_provider.wasm")
                } else {
                    include_bytes!("../fixtures/performance/p065/provider.wasm")
                })
                .unwrap();
            let consumer = engine
                .compile(if exact {
                    include_bytes!("../fixtures/performance/p065/exact_capture.wasm")
                } else {
                    include_bytes!("../fixtures/performance/p065/numeric_capture.wasm")
                })
                .unwrap();
            let mut store = Store::new(&engine);
            let provider_instance = Linker::new(&engine)
                .instantiate(&mut store, &provider)
                .unwrap();
            let External::Tag(tag) = provider_instance.export(&store, "yield").unwrap() else {
                panic!()
            };
            let mut linker = Linker::new(&engine);
            linker
                .define("provider", "yield", External::Tag(tag))
                .unwrap();
            let instance = linker.instantiate(&mut store, &consumer).unwrap();
            let run = func(&store, instance, "run");
            for entry in 1..=3 {
                store.request_materialization_roundtrip(run, 5).unwrap();
                let before = store.register_dispatches();
                let RunOutcome::Suspended {
                    tag: actual,
                    continuation,
                    payload,
                } = store.invoke(run, &[]).unwrap()
                else {
                    panic!()
                };
                assert_eq!(actual, tag);
                assert!(payload.is_empty());
                assert_eq!(
                    store.register_dispatches() - before,
                    if kind == 0 || (kind == 3 && entry < 3) {
                        0
                    } else {
                        5
                    }
                );
                let snapshot = store.execution_snapshot().unwrap();
                assert_eq!(snapshot.cursor.source_pc, 5);
                assert_eq!(snapshot.remaining_fuel, Some(94));
                assert_eq!(snapshot.frames[0].locals, [DiagnosticValue::I32(13)]);
                assert_eq!(snapshot.operands, [DiagnosticValue::I32(99)]);
                let root = store.root(continuation).unwrap();
                let continuation = Value::Ref(RefValue::Gc(root.reference()));
                store.release_published(&[continuation]);
                store.collect().unwrap();
                let RunOutcome::Returned(objects) =
                    store.invoke(func(&store, instance, "make"), &[]).unwrap()
                else {
                    panic!()
                };
                let object = objects[0];
                store.request_materialization_roundtrip(run, 6).unwrap();
                let before = (store.register_dispatches(), store.register_exits());
                assert_eq!(
                    store
                        .invoke(func(&store, instance, "resume"), &[continuation, object])
                        .unwrap(),
                    RunOutcome::Returned(vec![Value::I32(112)])
                );
                assert_eq!(
                    (store.register_dispatches(), store.register_exits()),
                    before
                );
                let snapshot = store.execution_snapshot().unwrap();
                assert_eq!(snapshot.cursor.source_pc, 6);
                assert_eq!(snapshot.remaining_fuel, Some(93));
                assert_eq!(
                    snapshot.frames.last().unwrap().locals,
                    [DiagnosticValue::I32(13)]
                );
                assert_eq!(
                    store
                        .invoke(func(&store, instance, "resume"), &[continuation, object])
                        .unwrap_err()
                        .kind,
                    ErrorKind::Trap
                );
                store.release_published(&[object]);
                drop(root);
                store.collect().unwrap();
                assert_eq!(store.continuation_bytes(), 0);
            }
        }
    }
    eprintln!(
        "P134_CASE {{\"name\":\"p134_eviction_pin_and_continuation_resume_without_reentry\",\"passed\":true,\"capture_resumes\":24,\"exact_and_nominal\":true,\"source_state_preserved\":true,\"resume_dispatches\":0,\"one_shot_preserved\":true}}"
    );
}
