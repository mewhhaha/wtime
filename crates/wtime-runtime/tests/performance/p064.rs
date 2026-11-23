#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex};
use wtime_runtime::*;
const DIRECT: &[u8] = include_bytes!("../fixtures/performance/p064/direct_call.wasm");
const HOST: &[u8] = include_bytes!("../fixtures/performance/p064/host_call.wasm");
const CALLS: &[u8] = include_bytes!("../fixtures/performance/p064/calls.wasm");
const POLICIES: &[ExecutionPolicy] = &[
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Register,
    #[cfg(feature = "compact-executor")]
    ExecutionPolicy::Compact,
];
fn engine(policy: ExecutionPolicy, fuel: Option<u64>, depth: usize) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Default::default()
    };
    config.limits.fuel = fuel;
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn loaded(engine: &Engine, bytes: &[u8]) -> (Store, Instance) {
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(engine);
    let instance = Linker::new(engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
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
fn outcome(
    result: Result<RunOutcome>,
) -> std::result::Result<RunOutcome, (ErrorKind, String, Option<TrapCode>)> {
    result.map_err(|error| (error.kind, error.message, error.trap_code))
}
fn fuel_error() -> (ErrorKind, String, Option<TrapCode>) {
    (ErrorKind::ResourceLimit, "fuel exhausted".into(), None)
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut snapshot = store.execution_snapshot().unwrap().clone();
    for frame in &mut snapshot.frames {
        frame.module_fingerprint = 0;
        assert_eq!(frame.locals.len(), frame.local_count);
        assert_eq!(frame.labels.len(), frame.label_count);
    }
    assert_eq!(snapshot.operands.len(), snapshot.operand_count);
    assert_eq!(snapshot.frames.len(), snapshot.frame_count);
    snapshot
}

#[test]
fn p064_direct_call_and_store_prefix_execute_once() {
    for arg in [21, i32::MAX] {
        for fuel in 0u64..=16 {
            let mut observations = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(fuel), 10), DIRECT);
                let memory = mem(&store, instance);
                store.write_memory(memory, 0, &vec![0x5a; 65536]).unwrap();
                let result =
                    outcome(store.invoke(func(&store, instance, "run"), &[Value::I32(arg)]));
                assert_eq!(
                    result,
                    if fuel < 15 {
                        Err(fuel_error())
                    } else {
                        Ok(RunOutcome::Returned(vec![Value::I32(
                            arg.wrapping_add(1).wrapping_mul(3),
                        )]))
                    }
                );
                let mut expected = vec![0x5a; 65536];
                if fuel >= 3 {
                    expected[8..12].copy_from_slice(&arg.to_le_bytes());
                }
                let image = store.read_memory(memory, 0, 65536).unwrap();
                assert_eq!(image, expected);
                if policy == ExecutionPolicy::Register {
                    assert_eq!(
                        store.register_dispatches(),
                        fuel.min(7) + fuel.saturating_sub(8).min(4)
                    );
                    assert_eq!(store.register_exits(), u64::from(fuel >= 8));
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                observations.push((result, image));
            }
            for observation in &observations[1..] {
                assert_eq!(&observations[0], observation);
            }
        }
    }
}

#[test]
fn p064_host_reentry_keeps_state_and_fuel() {
    let mut observations = vec![];
    for &policy in POLICIES {
        let engine = engine(policy, Some(100), 10);
        let module = engine.compile(HOST).unwrap();
        let mut store = Store::new(&engine);
        let handles: Arc<Mutex<Option<(Func, Memory)>>> = Arc::new(Mutex::new(None));
        let calls = Arc::new(Mutex::new(vec![]));
        let host_handles = handles.clone();
        let host_calls = calls.clone();
        let callback = store
            .create_host_func_with_caller(
                FuncType {
                    params: vec![ValType::I32],
                    results: vec![ValType::I32],
                },
                move |caller, args| {
                    assert_eq!(args, &[Value::I32(22)]);
                    let (child, memory) = host_handles.lock().unwrap().unwrap();
                    assert_eq!(
                        caller.store().read_memory(memory, 8, 4)?,
                        21i32.to_le_bytes()
                    );
                    let view = snapshot(caller.store());
                    assert_eq!(view.cursor.source_pc, 7);
                    assert_eq!(view.remaining_fuel, Some(92));
                    assert!(view.pending_boundary_charged);
                    assert_eq!(
                        view.frames[0].locals,
                        [DiagnosticValue::I32(21), DiagnosticValue::I32(22)]
                    );
                    assert_eq!(view.operands, [DiagnosticValue::I32(22)]);
                    host_calls.lock().unwrap().push(view);
                    caller.collect()?;
                    let result = caller.invoke(child, args)?;
                    assert_eq!(result, RunOutcome::Returned(vec![Value::I32(44)]));
                    // A replay of the caller's earlier store would replace this value.
                    caller
                        .store()
                        .write_memory(memory, 8, &99i32.to_le_bytes())?;
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
        store.write_memory(memory, 0, &vec![0x5a; 65536]).unwrap();
        let function = func(&store, instance, "run");
        store
            .request_materialization_roundtrip(function, 7)
            .unwrap();
        let result = outcome(store.invoke(function, &[Value::I32(21)]));
        assert_eq!(result, Ok(RunOutcome::Returned(vec![Value::I32(66)])));
        let mut expected = vec![0x5a; 65536];
        expected[8..12].copy_from_slice(&99i32.to_le_bytes());
        let image = store.read_memory(memory, 0, 65536).unwrap();
        assert_eq!(image, expected);
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 11);
            assert_eq!(store.register_exits(), 1);
            assert_eq!(store.register_fallback_frames(), 0);
        }
        observations.push((result, image, calls));
    }
    for observation in &observations[1..] {
        assert_eq!(&observations[0], observation);
    }
}

#[test]
fn p064_tail_indirect_and_depth_errors_match() {
    for slot in [0, 1, 2, 3, -1] {
        for fuel in 0u64..=9 {
            let mut observations = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(fuel), 10), CALLS);
                let result = outcome(
                    store.invoke(func(&store, instance, "run_indirect"), &[Value::I32(slot)]),
                );
                if fuel < 3 {
                    assert_eq!(result, Err(fuel_error()));
                } else if slot == 0 {
                    assert_eq!(
                        result,
                        if fuel < 8 {
                            Err(fuel_error())
                        } else {
                            Ok(RunOutcome::Returned(vec![Value::I32(42)]))
                        }
                    );
                } else {
                    assert_eq!(result.as_ref().unwrap_err().0, ErrorKind::Trap);
                }
                if policy == ExecutionPolicy::Register {
                    assert_eq!(store.register_exits(), u64::from(fuel >= 3));
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                observations.push(result);
            }
            for observation in &observations[1..] {
                assert_eq!(&observations[0], observation);
            }
        }
    }
    for (name, args, depth, expected) in [
        ("tail", vec![Value::I32(20)], 1, 42),
        ("run_ref", vec![], 10, 42),
    ] {
        for fuel in 0..=9 {
            let mut observations = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(fuel), depth), CALLS);
                let result = outcome(store.invoke(func(&store, instance, name), &args));
                assert_eq!(
                    result,
                    if fuel < 8 {
                        Err(fuel_error())
                    } else {
                        Ok(RunOutcome::Returned(vec![Value::I32(expected)]))
                    }
                );
                if policy == ExecutionPolicy::Register && fuel >= 8 {
                    assert_eq!(store.register_exits(), 1);
                    assert!(store.register_dispatches() > 0);
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                observations.push(result);
            }
            for observation in &observations[1..] {
                assert_eq!(&observations[0], observation);
            }
        }
    }
    for depth in [1, 2, 4] {
        for n in [0, 1, 3] {
            let mut observations = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(100), depth), CALLS);
                let result =
                    outcome(store.invoke(func(&store, instance, "recursive"), &[Value::I32(n)]));
                if n as usize >= depth {
                    assert_eq!(result.as_ref().unwrap_err().0, ErrorKind::ResourceLimit);
                } else {
                    assert_eq!(result, Ok(RunOutcome::Returned(vec![Value::I32(42)])));
                }
                if policy == ExecutionPolicy::Register {
                    assert!(store.register_exits() > 0);
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                observations.push(result);
            }
            for observation in &observations[1..] {
                assert_eq!(&observations[0], observation);
            }
        }
    }
}

#[test]
fn p064_every_prefix_and_fallback_preserve_accounting() {
    for (pc, visit_index) in [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 5),
        (6, 6),
        (7, 7),
        (8, 12),
        (9, 13),
        (10, 14),
    ] {
        let mut observations = vec![];
        for &policy in POLICIES {
            let (mut store, instance) = loaded(&engine(policy, Some(visit_index + 1), 10), DIRECT);
            let function = func(&store, instance, "run");
            store
                .request_materialization_roundtrip(function, pc)
                .unwrap();
            let result = outcome(store.invoke(function, &[Value::I32(21)]));
            let view = snapshot(&store);
            assert_eq!(view.cursor.source_pc, pc);
            assert_eq!(view.remaining_fuel, Some(0));
            assert!(view.pending_boundary_charged);
            let image = store.read_memory(mem(&store, instance), 0, 65536).unwrap();
            observations.push((result, view, image));
        }
        for observation in &observations[1..] {
            assert_eq!(&observations[0], observation, "caller PC{pc}");
        }
    }
    // Reference-bearing locals remain canonical for the whole activation.
    let bytes = include_bytes!("../fixtures/performance/p047/p047_mixed_externref_local.wasm");
    for &policy in POLICIES {
        let (mut store, instance) = loaded(&engine(policy, None, 10), bytes);
        assert!(
            store
                .invoke(func(&store, instance, "run"), &[Value::I32(21)])
                .is_ok()
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 0);
            assert_eq!(store.register_exits(), 0);
            assert_eq!(store.register_fallback_frames(), 1);
        }
    }
    let (mut store, instance) = loaded(&engine(ExecutionPolicy::Register, None, 10), DIRECT);
    store.cancellation_token().cancel();
    assert_eq!(
        store
            .invoke(func(&store, instance, "run"), &[Value::I32(21)])
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(store.register_dispatches(), 0);
    assert_eq!(store.register_exits(), 0);
    assert_eq!(
        store.read_memory(mem(&store, instance), 0, 65536).unwrap(),
        vec![0; 65536]
    );
    eprintln!(
        "P064: 11 caller source snapshots across pending call and decoded continuation; exact source fuel, callback and memory checks"
    );
}
