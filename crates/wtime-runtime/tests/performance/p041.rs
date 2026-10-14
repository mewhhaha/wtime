//! Unexecuted P041 independent binding, range-entry, and boundary regressions.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const DIRECT: &[u8] = include_bytes!("../fixtures/performance/p041/direct-numeric.wasm");
const CONSUMER: &[u8] = include_bytes!("../fixtures/performance/p041/import-consumer.wasm");
const PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p041/provider-distinct.wasm");
const HOST: &[u8] = include_bytes!("../fixtures/performance/p041/host-calls.wasm");
const START: &[u8] = include_bytes!("../fixtures/performance/p041/start-failure.wasm");
const NONNUMERIC: &[u8] = include_bytes!("../fixtures/performance/p041/nonnumeric-target.wasm");
const ROOTS: &[u8] = include_bytes!("../fixtures/performance/p041/host-gc-reentry.wasm");

fn engine(policy: ExecutionPolicy, fuel: Option<u64>, depth: usize) -> Engine {
    let mut config = Config::default();
    config.features = FeatureSet::all_phase3_core();
    config.limits.fuel = fuel;
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!()
    };
    function
}
fn load(engine: &Engine, store: &mut Store, bytes: &[u8]) -> Instance {
    let module = engine.compile(bytes).unwrap();
    Linker::new(engine).instantiate(store, &module).unwrap()
}
fn view(value: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    value.map_err(|error| (error.kind, error.message))
}

#[test]
fn p041_argument_ranges_and_every_call_fuel_prefix_match_independent_counts() {
    for (name, args, result, events) in [
        (
            "sum2_run",
            vec![Value::I32(-17), Value::I32(42)],
            vec![Value::I32(25)],
            8,
        ),
        (
            "pair_run",
            vec![Value::I64(i64::MIN + 1), Value::I32(91)],
            vec![Value::I64(i64::MIN + 1), Value::I32(91)],
            7,
        ),
        ("zero_run", vec![], vec![Value::I32(29)], 4),
    ] {
        for fuel in 0..=events + 1 {
            for policy in [
                ExecutionPolicy::Reference,
                ExecutionPolicy::Decoded,
                ExecutionPolicy::Compact,
            ] {
                let engine = engine(policy, Some(fuel), 16);
                let mut store = Store::new(&engine);
                let instance = load(&engine, &mut store, DIRECT);
                let target = func(&store, instance, name);
                assert_eq!(
                    view(store.invoke(target, &args)),
                    if fuel < events {
                        Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                    } else {
                        Ok(RunOutcome::Returned(result.clone()))
                    },
                    "{name} {policy:?} fuel={fuel}"
                );
                assert_eq!(
                    store.compact_dispatches(),
                    if policy == ExecutionPolicy::Compact {
                        fuel.min(events)
                    } else {
                        0
                    }
                );
                if policy == ExecutionPolicy::Compact && fuel >= events {
                    assert_eq!(store.compact_bound_calls(), 1);
                }
            }
        }
    }
    let wide = include_bytes!("../fixtures/performance/frames/guest_frame.wasm");
    let mut lanes = [0u8; 16];
    for (index, lane) in [-1i32, 0, 17, 99].into_iter().enumerate() {
        lanes[index * 4..index * 4 + 4].copy_from_slice(&lane.to_le_bytes());
    }
    let expected = vec![
        Value::I32(-17),
        Value::I64(i64::MAX),
        Value::F32((-0.0f32).to_bits()),
        Value::F64((-2.25f64).to_bits()),
        Value::V128(V128(lanes)),
    ];
    for policy in [
        ExecutionPolicy::Reference,
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Compact,
    ] {
        let engine = engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let instance = load(&engine, &mut store, wide);
        let echo = func(&store, instance, "echo_run");
        assert_eq!(
            store.invoke(echo, &[]).unwrap(),
            RunOutcome::Returned(expected.clone())
        );
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_bound_calls(), 1);
        }
        // Every target local is made nonzero. Its next activation must begin
        // with fresh zeros across all five numeric types.
        let reset = func(&store, instance, "run");
        for _ in 0..2 {
            assert_eq!(
                store.invoke(reset, &[]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(84)])
            );
        }
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_bound_calls(), 5);
        }
    }
}

#[test]
fn p041_call_and_callee_cursors_materialize_the_same_canonical_state() {
    for (hook_name, pc) in [("sum2_run", 2), ("sum2", 0), ("sum2", 3), ("sum2_run", 3)] {
        let mut observations = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let engine = engine(policy, Some(8), 16);
            let mut store = Store::new(&engine);
            let instance = load(&engine, &mut store, DIRECT);
            let hook = func(&store, instance, hook_name);
            let caller = func(&store, instance, "sum2_run");
            store.request_materialization_roundtrip(hook, pc).unwrap();
            assert_eq!(
                store
                    .invoke(caller, &[Value::I32(-17), Value::I32(42)])
                    .unwrap(),
                RunOutcome::Returned(vec![Value::I32(25)])
            );
            assert_eq!(store.materialization_roundtrips(), 1);
            observations.push(store.execution_snapshot().unwrap().clone());
            if policy == ExecutionPolicy::Compact {
                assert_eq!(store.compact_bound_calls(), 1);
            }
        }
        assert_eq!(observations[0], observations[1], "{hook_name} pc={pc}");
    }
}

#[test]
fn p041_shared_module_instances_and_stores_resolve_their_own_imported_guest() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = engine(policy, None, 16);
        let consumer = engine.compile(CONSUMER).unwrap();
        let provider = engine.compile(PROVIDER).unwrap();
        let mut first = Store::new(&engine);
        let mut second = Store::new(&engine);
        let mut exported = Vec::new();
        for (store, offsets) in [(&mut first, [10, 20]), (&mut second, [20, 10])] {
            let p = Linker::new(&engine).instantiate(store, &provider).unwrap();
            for offset in offsets {
                let target = func(store, p, if offset == 10 { "f10" } else { "f20" });
                let mut linker = Linker::new(&engine);
                linker.define("env", "f", External::Func(target)).unwrap();
                let instance = linker.instantiate(store, &consumer).unwrap();
                let caller = func(store, instance, "main");
                for input in [-17, 0, 91] {
                    assert_eq!(
                        store.invoke(caller, &[Value::I32(input)]).unwrap(),
                        RunOutcome::Returned(vec![Value::I32(input + offset)])
                    );
                }
                if policy == ExecutionPolicy::Compact {
                    assert!(store.compact_bound_calls() >= 3);
                }
                exported.push(caller);
            }
        }
        drop(consumer);
        drop(provider);
        assert_eq!(
            first.invoke(exported[0], &[Value::I32(7)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(17)])
        );
        assert_eq!(
            second.invoke(exported[2], &[Value::I32(7)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(27)])
        );
        assert_eq!(
            second
                .invoke(exported[0], &[Value::I32(7)])
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
    }
}

#[test]
fn p041_numeric_host_reentry_preserves_decoded_caller_gc_roots_and_growth() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = engine(policy, None, 32);
        let mut store = Store::new(&engine);
        let nested_instance = load(&engine, &mut store, DIRECT);
        let nested = func(&store, nested_instance, "zero_run");
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observed = observations.clone();
        let growth_engine = engine.clone();
        let growth_module = engine.compile(DIRECT).unwrap();
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
        let module = engine.compile(ROOTS).unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "host", External::Func(host)).unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let run = func(&store, instance, "run");
        assert_eq!(
            store.invoke(run, &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(91)])
        );
        assert_eq!(*observations.lock().unwrap(), vec![5]);
        assert_eq!(store.collect().unwrap().reclaimed, 1);
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_bound_calls(), 2);
        }
    }
}

#[test]
fn p041_tail_calls_preserve_logical_depth_and_host_suspension_barriers() {
    for policy in [
        ExecutionPolicy::Reference,
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Compact,
    ] {
        let engine = engine(policy, None, 2);
        let mut store = Store::new(&engine);
        let host = store
            .create_host_func(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                |args| {
                    let Value::I32(value) = args[0] else { panic!() };
                    Ok(vec![Value::I32(value + 3)])
                },
            )
            .unwrap();
        let module = engine.compile(HOST).unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "host", External::Func(host)).unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        for name in ["ordinary", "tail"] {
            let target = func(&store, instance, name);
            assert_eq!(
                store.invoke(target, &[Value::I32(5)]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(8)])
            );
        }
        let tail = func(&store, instance, "tail_recurse");
        assert_eq!(
            store.invoke(tail, &[Value::I32(32)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(0)])
        );
        let ordinary = func(&store, instance, "recurse");
        let error = store.invoke(ordinary, &[Value::I32(7)]).unwrap_err();
        assert_eq!(
            (error.kind, error.message.as_str()),
            (ErrorKind::ResourceLimit, "call depth exhausted")
        );
        if policy == ExecutionPolicy::Compact {
            assert!(store.compact_bound_calls() > 32);
        }
        let suspension = load(
            &engine,
            &mut store,
            include_bytes!("../../examples/suspension.wasm"),
        );
        let suspend = func(&store, suspension, "run");
        let suspending_host = store
            .create_host_func_with_outcome(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, _args| caller.invoke(suspend, &[]),
            )
            .unwrap();
        let mut boundary_linker = Linker::new(&engine);
        boundary_linker
            .define("env", "host", External::Func(suspending_host))
            .unwrap();
        let boundary = boundary_linker.instantiate(&mut store, &module).unwrap();
        for name in ["ordinary", "tail"] {
            let target = func(&store, boundary, name);
            let error = store.invoke(target, &[Value::I32(5)]).unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (
                    ErrorKind::Host,
                    "host suspension cannot cross an active guest call"
                )
            );
        }
    }
}

#[test]
fn p041_failed_start_preserves_host_side_effect_and_cannot_reuse_old_bindings() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let marked = Arc::new(Mutex::new(0));
        let count = marked.clone();
        let host = store
            .create_host_func(FuncType::new(vec![], vec![]), move |_| {
                *count.lock().unwrap() += 1;
                Ok(vec![])
            })
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "mark", External::Func(host)).unwrap();
        let module = engine.compile(START).unwrap();
        assert_eq!(
            linker.instantiate(&mut store, &module).unwrap_err().kind,
            ErrorKind::Trap
        );
        assert_eq!(*marked.lock().unwrap(), 1);
        let next = load(&engine, &mut store, DIRECT);
        let target = func(&store, next, "sum2_run");
        assert_eq!(
            store
                .invoke(target, &[Value::I32(7), Value::I32(29)])
                .unwrap(),
            RunOutcome::Returned(vec![Value::I32(36)])
        );
        assert_eq!(*marked.lock().unwrap(), 1);
        if policy == ExecutionPolicy::Compact {
            assert!(store.compact_bound_calls() >= 2);
        }
    }
}

#[test]
fn p041_reference_target_and_wrong_import_signature_decline_before_publication() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let module = engine.compile(NONNUMERIC).unwrap();
        let host = store
            .create_host_func(
                FuncType::new(vec![], vec![ValType::Ref(RefType::EXTERNREF)]),
                |_| Ok(vec![Value::Ref(RefValue::Null(HeapType::Extern))]),
            )
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "ref", External::Func(host)).unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let run = func(&store, instance, "run");
        assert_eq!(
            store.invoke(run, &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(7)])
        );
        assert_eq!(store.compact_dispatches(), 0);
        assert_eq!(store.compact_bound_calls(), 0);
        let consumer = engine.compile(CONSUMER).unwrap();
        let wrong = store
            .create_host_func(
                FuncType::new(vec![ValType::I64], vec![ValType::I64]),
                |args| Ok(args.to_vec()),
            )
            .unwrap();
        let mut wrong_linker = Linker::new(&engine);
        wrong_linker
            .define("env", "f", External::Func(wrong))
            .unwrap();
        let error = wrong_linker.instantiate(&mut store, &consumer).unwrap_err();
        assert_eq!(
            (error.kind, error.message.as_str()),
            (ErrorKind::Link, "function import type mismatch")
        );
    }
}
