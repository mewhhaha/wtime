#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const POLICIES: &[ExecutionPolicy] = &[
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Register,
    #[cfg(feature = "compact-executor")]
    ExecutionPolicy::Compact,
];
const CAPTURE: &[u8] = include_bytes!("../fixtures/performance/p065/numeric_capture.wasm");
const PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p065/provider.wasm");
const EXACT: &[u8] = include_bytes!("../fixtures/performance/p065/exact_capture.wasm");
const EXACT_PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p065/exact_provider.wasm");

fn engine(policy: ExecutionPolicy, fuel: Option<u64>, bytes: usize, depth: usize) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    config.limits.max_continuation_bytes = bytes;
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!()
    };
    function
}
fn call(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    store.invoke(func(store, instance, name), args)
}
fn view(
    result: Result<RunOutcome>,
) -> std::result::Result<RunOutcome, (ErrorKind, String, Option<TrapCode>)> {
    result.map_err(|error| (error.kind, error.message, error.trap_code))
}
fn scalar(store: &mut Store, instance: Instance, name: &str) -> Value {
    let RunOutcome::Returned(values) = call(store, instance, name, &[]).unwrap() else {
        panic!()
    };
    assert_eq!(values.len(), 1);
    values[0]
}
fn loaded_capture(
    policy: ExecutionPolicy,
    fuel: Option<u64>,
    bytes: usize,
    depth: usize,
    exact: bool,
) -> (Store, Instance, Tag) {
    let engine = engine(policy, fuel, bytes, depth);
    let provider = engine
        .compile(if exact { EXACT_PROVIDER } else { PROVIDER })
        .unwrap();
    let consumer = engine.compile(if exact { EXACT } else { CAPTURE }).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &provider)
        .unwrap();
    let External::Tag(tag) = instance.export(&store, "yield").unwrap() else {
        panic!()
    };
    let mut linker = Linker::new(&engine);
    linker
        .define("provider", "yield", External::Tag(tag))
        .unwrap();
    let consumer = linker.instantiate(&mut store, &consumer).unwrap();
    (store, consumer, tag)
}
fn captured(store: &mut Store, instance: Instance, tag: Tag) -> (RootToken, Value, usize) {
    let RunOutcome::Suspended {
        tag: actual,
        payload,
        continuation,
    } = call(store, instance, "run", &[]).unwrap()
    else {
        panic!()
    };
    assert_eq!(actual, tag);
    assert!(payload.is_empty());
    let bytes = store.continuation_bytes();
    assert!(bytes > 0);
    let root = store.root(continuation).unwrap();
    let value = Value::Ref(RefValue::Gc(root.reference()));
    store.release_published(&[value]);
    assert_eq!(store.collect().unwrap().live, 1);
    (root, value, bytes)
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut snapshot = store.execution_snapshot().unwrap().clone();
    assert!(!snapshot.truncated);
    for frame in &mut snapshot.frames {
        frame.module_fingerprint = 0;
    }
    snapshot
}
fn clean(store: &mut Store, root: RootToken, values: &[Value]) {
    store.release_published(values);
    drop(root);
    assert_eq!(store.collect().unwrap().live, 0);
    assert_eq!(store.continuation_bytes(), 0);
    assert_eq!(store.gc_payload_bytes(), 0);
}

#[test]
fn p065_outer_reference_survives_reentrant_collection() {
    for &policy in POLICIES {
        let engine = engine(policy, None, usize::MAX, 32);
        let numeric = engine
            .compile(include_bytes!(
                "../fixtures/performance/p041/direct-numeric.wasm"
            ))
            .unwrap();
        let roots = engine
            .compile(include_bytes!(
                "../fixtures/performance/p041/host-gc-reentry.wasm"
            ))
            .unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &numeric)
            .unwrap();
        let nested = func(&store, instance, "zero_run");
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observed = observations.clone();
        let growth_engine = engine.clone();
        let growth_module = numeric.clone();
        let host = store
            .create_host_func_with_caller(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, args| {
                    assert_eq!(args, &[Value::I32(5)]);
                    let stats = caller.collect()?;
                    assert_eq!((stats.live, stats.reclaimed), (1, 0));
                    for _ in 0..17 {
                        Linker::new(&growth_engine).instantiate(caller.store(), &growth_module)?;
                    }
                    assert_eq!(
                        caller.invoke(nested, &[])?,
                        RunOutcome::Returned(vec![Value::I32(29)])
                    );
                    observed.lock().unwrap().push(5);
                    Ok(vec![Value::I32(6)])
                },
            )
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "host", External::Func(host)).unwrap();
        let instance = linker.instantiate(&mut store, &roots).unwrap();
        assert_eq!(
            call(&mut store, instance, "run", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(91)])
        );
        assert_eq!(*observations.lock().unwrap(), vec![5]);
        if policy == ExecutionPolicy::Register {
            assert!(store.register_dispatches() >= 3);
            assert_eq!(store.register_exits(), 1);
            assert!(store.register_fallback_frames() >= 1);
        }
        assert_eq!(store.collect().unwrap().reclaimed, 1);
    }
    eprintln!("P065: sole outer root survives collection across a Register call exit");
}

#[test]
fn p065_capture_retains_numeric_state_and_source_cursor() {
    let mut captured_views = vec![];
    let mut restored_views = vec![];
    for &policy in POLICIES {
        let (mut store, instance, tag) = loaded_capture(policy, Some(100), usize::MAX, 16, false);
        let run = func(&store, instance, "run");
        store.request_materialization_roundtrip(run, 5).unwrap();
        let (root, continuation, bytes) = captured(&mut store, instance, tag);
        let state = snapshot(&store);
        assert_eq!(state.cursor.source_pc, 5);
        assert_eq!(state.frames[0].locals, [DiagnosticValue::I32(13)]);
        assert_eq!(state.operands, [DiagnosticValue::I32(99)]);
        assert_eq!(state.remaining_fuel, Some(94));
        assert!(state.pending_boundary_charged);
        captured_views.push(state);
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 5);
            assert_eq!(store.register_exits(), 1);
            assert_eq!(store.register_fallback_frames(), 0);
        }
        let object = scalar(&mut store, instance, "make");
        store.request_materialization_roundtrip(run, 6).unwrap();
        let before = (store.register_dispatches(), store.register_exits());
        assert_eq!(
            call(&mut store, instance, "resume", &[continuation, object]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(112)])
        );
        let state = snapshot(&store);
        assert_eq!(state.cursor.source_pc, 6);
        assert_eq!(
            state.frames.last().unwrap().locals,
            [DiagnosticValue::I32(13)]
        );
        assert_eq!(state.operands[0], DiagnosticValue::I32(99));
        assert_eq!(state.remaining_fuel, Some(93));
        restored_views.push(state);
        assert_eq!(
            (store.register_dispatches(), store.register_exits()),
            before
        );
        assert!(store.continuation_bytes() < bytes);
        assert_eq!(
            call(&mut store, instance, "get", &[object]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(
            call(&mut store, instance, "resume", &[continuation, object])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
        clean(&mut store, root, &[object]);
    }
    for state in &captured_views[1..] {
        assert_eq!(*state, captured_views[0]);
    }
    for state in &restored_views[1..] {
        assert_eq!(*state, restored_views[0]);
    }
    eprintln!("P065: numeric local=13 and operand=99 captured at suspend source_pc=5");
}

#[test]
fn p065_cross_module_exact_suspend_and_resource_controls() {
    for exact in [false, true] {
        for fuel in 0u64..=12 {
            let mut outcomes = vec![];
            for &policy in POLICIES {
                let (mut store, instance, tag) =
                    loaded_capture(policy, Some(fuel), usize::MAX, 16, exact);
                if fuel < 6 {
                    let error = call(&mut store, instance, "run", &[]).unwrap_err();
                    assert_eq!(
                        (error.kind, error.message.as_str()),
                        (ErrorKind::ResourceLimit, "fuel exhausted")
                    );
                    assert_eq!(store.continuation_bytes(), 0);
                    if policy == ExecutionPolicy::Register {
                        assert_eq!(store.register_dispatches(), fuel.min(5));
                        assert_eq!(store.register_exits(), 0);
                    }
                    continue;
                }
                let (root, continuation, _) = captured(&mut store, instance, tag);
                let object = scalar(&mut store, instance, "make");
                let result = view(call(
                    &mut store,
                    instance,
                    "resume",
                    &[continuation, object],
                ));
                assert_eq!(
                    result,
                    if fuel < 11 {
                        Err((ErrorKind::ResourceLimit, "fuel exhausted".into(), None))
                    } else {
                        Ok(RunOutcome::Returned(vec![Value::I32(112)]))
                    }
                );
                outcomes.push(result);
                assert_eq!(
                    call(&mut store, instance, "resume", &[continuation, object])
                        .unwrap_err()
                        .kind,
                    ErrorKind::Trap
                );
                if policy == ExecutionPolicy::Register {
                    assert_eq!(store.register_dispatches(), 5);
                    assert_eq!(store.register_exits(), 1);
                }
                clean(&mut store, root, &[object]);
            }
            for result in outcomes.iter().skip(1) {
                assert_eq!(*result, outcomes[0]);
            }
        }
        for &policy in POLICIES {
            let (mut store, instance, tag) = loaded_capture(policy, None, usize::MAX, 16, exact);
            let (root, continuation, bytes) = captured(&mut store, instance, tag);
            let object = scalar(&mut store, instance, "make");
            let wrong = scalar(&mut store, instance, "make_wrong");
            assert_eq!(
                call(&mut store, instance, "resume", &[continuation, wrong])
                    .unwrap_err()
                    .kind,
                ErrorKind::Host
            );
            assert_eq!(store.continuation_bytes(), bytes);
            let (mut foreign, other, _) = loaded_capture(policy, None, usize::MAX, 16, exact);
            assert_eq!(
                call(&mut foreign, other, "resume", &[continuation, object])
                    .unwrap_err()
                    .kind,
                ErrorKind::Host
            );
            assert_eq!(foreign.register_dispatches(), 0);
            assert_eq!(store.continuation_bytes(), bytes);
            let before = (store.register_dispatches(), store.register_exits());
            store.cancellation_token().cancel();
            assert_eq!(
                call(&mut store, instance, "resume", &[continuation, object])
                    .unwrap_err()
                    .message,
                "execution cancelled"
            );
            assert_eq!(
                (store.register_dispatches(), store.register_exits()),
                before
            );
            assert_eq!(store.continuation_bytes(), bytes);
            store.cancellation_token().reset();
            assert_eq!(
                call(&mut store, instance, "resume", &[continuation, object]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(112)])
            );
            assert_eq!(
                call(&mut store, instance, "resume", &[continuation, object])
                    .unwrap_err()
                    .kind,
                ErrorKind::Trap
            );
            clean(&mut store, root, &[object, wrong]);
            let (mut limited, other, _) = loaded_capture(policy, None, bytes - 1, 16, exact);
            let error = call(&mut limited, other, "run", &[]).unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (ErrorKind::ResourceLimit, "continuation byte budget")
            );
            if policy == ExecutionPolicy::Register {
                assert_eq!(limited.register_dispatches(), 5);
                assert_eq!(limited.register_exits(), 1);
            }
            assert_eq!(limited.collect().unwrap().live, 0);
            assert_eq!(limited.continuation_bytes(), 0);
            let (mut limited, other, tag) = loaded_capture(policy, None, usize::MAX, 1, exact);
            let (root, continuation, bytes) = captured(&mut limited, other, tag);
            let object = scalar(&mut limited, other, "make");
            for _ in 0..2 {
                let error =
                    call(&mut limited, other, "resume", &[continuation, object]).unwrap_err();
                assert_eq!(error.message, "continuation frame budget");
                assert_eq!(limited.continuation_bytes(), bytes);
            }
            clean(&mut limited, root, &[object]);
        }
    }
    eprintln!(
        "P065: cross-module exact suspension preserves one-shot fuel cancellation and byte budgets"
    );
}

#[test]
fn p065_reference_and_vector_frames_decline() {
    let bytes = include_bytes!("../fixtures/performance/p065/barrier_guards.wasm");
    for name in ["ref_local", "vector_local"] {
        for &policy in POLICIES {
            let engine = engine(policy, None, usize::MAX, 16);
            let module = engine.compile(bytes).unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            assert_eq!(
                call(&mut store, instance, name, &[]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(42)])
            );
            assert_eq!(store.register_dispatches(), 0);
            assert_eq!(store.register_exits(), 0);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_fallback_frames(), 1);
            }
        }
    }
    for (name, pc, steps, operands) in [
        ("numeric_gc", 1, 1, vec![DiagnosticValue::I32(13)]),
        ("numeric_ref", 2, 2, vec![]),
    ] {
        let mut snapshots = vec![];
        for &policy in POLICIES {
            let engine = engine(policy, None, usize::MAX, 16);
            let module = engine.compile(bytes).unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            store
                .request_materialization_roundtrip(func(&store, instance, name), pc)
                .unwrap();
            assert_eq!(
                call(&mut store, instance, name, &[]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(42)])
            );
            let state = snapshot(&store);
            assert_eq!(state.operands, operands);
            snapshots.push(state);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), steps);
                assert_eq!(store.register_exits(), 1);
                assert_eq!(store.register_fallback_frames(), 0);
            }
            assert_eq!(store.collect().unwrap().live, 0);
        }
        for state in &snapshots[1..] {
            assert_eq!(*state, snapshots[0]);
        }
    }
    eprintln!("P065: reference and vector layouts take whole-function fallback");
}

#[test]
fn p065_exception_unwind_preserves_numeric_payload() {
    let bytes = include_bytes!("../fixtures/performance/p065/numeric_exception.wasm");
    for fuel in 0u64..=12 {
        let mut observations = vec![];
        for &policy in POLICIES {
            let engine = engine(policy, Some(fuel), usize::MAX, 16);
            let module = engine.compile(bytes).unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let result = view(call(&mut store, instance, "run", &[]));
            assert_eq!(
                result,
                if fuel < 8 {
                    Err((ErrorKind::ResourceLimit, "fuel exhausted".into(), None))
                } else {
                    Ok(RunOutcome::Returned(vec![Value::I32(20)]))
                }
            );
            observations.push(result);
            if policy == ExecutionPolicy::Register && fuel >= 8 {
                assert_eq!(store.register_dispatches(), 4);
                assert_eq!(store.register_exits(), 2);
            }
            assert_eq!(store.collect().unwrap().live, 0);
        }
        for result in &observations[1..] {
            assert_eq!(*result, observations[0]);
        }
    }
    for &policy in POLICIES {
        let engine = engine(policy, None, usize::MAX, 16);
        let module = engine.compile(bytes).unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        store
            .request_materialization_roundtrip(func(&store, instance, "thrower"), 3)
            .unwrap();
        let RunOutcome::Exception(exception) = call(&mut store, instance, "thrower", &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            store.exception_payload(exception).unwrap(),
            [Value::I32(20)]
        );
        let External::Tag(tag) = instance.export(&store, "e").unwrap() else {
            panic!()
        };
        assert_eq!(store.exception_tag(exception).unwrap(), tag);
        let state = snapshot(&store);
        assert_eq!(state.cursor.source_pc, 3);
        assert_eq!(state.operands, [DiagnosticValue::I32(20)]);
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 3);
            assert_eq!(store.register_exits(), 1);
        }
        store.release_published(&[Value::Ref(RefValue::Exn(exception))]);
        assert_eq!(store.collect().unwrap().live, 0);
    }
    eprintln!("P065: numeric exception payload=20 unwinds after a Register exit");
}
