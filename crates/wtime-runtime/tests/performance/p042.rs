//! Source-only P042 independent current-target and shared-boundary regressions.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const TABLES: &[u8] = include_bytes!("../fixtures/performance/p042/table-mutations.wasm");
const PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p042/table-alias-provider.wasm");
const CONSUMER: &[u8] = include_bytes!("../fixtures/performance/p042/table-alias-consumer.wasm");
const INVALID: &[u8] = include_bytes!("../fixtures/performance/p042/invalid-indirect.wasm");
const REFS: &[u8] = include_bytes!("../fixtures/performance/p042/call-ref.wasm");
const HOST: &[u8] = include_bytes!("../fixtures/performance/p042/host-indirect.wasm");
const WIDE: &[u8] = include_bytes!("../fixtures/performance/p042/table64-indirect.wasm");
const POLICIES: [ExecutionPolicy; 3] = [
    ExecutionPolicy::Reference,
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Compact,
];

fn make_engine(policy: ExecutionPolicy, fuel: Option<u64>, depth: usize) -> Engine {
    let mut config = Config::default();
    config.limits.fuel = fuel;
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn load(engine: &Engine, store: &mut Store, bytes: &[u8]) -> Instance {
    let module = engine.compile(bytes).unwrap();
    Linker::new(engine).instantiate(store, &module).unwrap()
}
fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(value) = instance.export(store, name).unwrap() else {
        panic!()
    };
    value
}
fn call(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let target = func(store, instance, name);
    store.invoke(target, args)
}
fn returned(store: &mut Store, instance: Instance, name: &str, args: &[Value], expected: i32) {
    assert_eq!(
        call(store, instance, name, args).unwrap(),
        RunOutcome::Returned(vec![Value::I32(expected)])
    );
}
fn view(value: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    value.map_err(|error| (error.kind, error.message))
}
fn no_cache_for_other_policies(store: &Store, policy: ExecutionPolicy) {
    if policy != ExecutionPolicy::Compact {
        assert_eq!(store.compact_indirect_cache_counts(), (0, 0, 0));
    }
}

#[test]
fn p042_table_set_grow_fill_copy_and_polymorphic_churn_read_current_targets() {
    for policy in POLICIES {
        let engine = make_engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let instance = load(&engine, &mut store, TABLES);
        for index in [0, 0, 1, 1] {
            returned(
                &mut store,
                instance,
                "run_t",
                &[Value::I32(index)],
                if index == 0 { 11 } else { 22 },
            );
        }
        assert_eq!(
            call(
                &mut store,
                instance,
                "fill_c",
                &[Value::I32(0), Value::I32(1)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![])
        );
        returned(&mut store, instance, "run_t", &[Value::I32(0)], 33);
        returned(&mut store, instance, "run_t", &[Value::I32(0)], 33);
        call(&mut store, instance, "set_a", &[Value::I32(0)]).unwrap();
        returned(&mut store, instance, "run_t", &[Value::I32(0)], 11);
        call(
            &mut store,
            instance,
            "copy_from_other",
            &[Value::I32(0), Value::I32(0), Value::I32(2)],
        )
        .unwrap();
        returned(&mut store, instance, "run_t", &[Value::I32(0)], 22);
        returned(&mut store, instance, "run_t", &[Value::I32(1)], 33);
        returned(&mut store, instance, "grow_c", &[Value::I32(1)], 2);
        returned(&mut store, instance, "run_t", &[Value::I32(2)], 33);
        call(&mut store, instance, "copy_overlap_right", &[]).unwrap();
        for (index, expected) in [22, 22, 33].into_iter().enumerate() {
            returned(
                &mut store,
                instance,
                "run_t",
                &[Value::I32(index as i32)],
                expected,
            );
        }
        call(
            &mut store,
            instance,
            "fill_c",
            &[Value::I32(0), Value::I32(2)],
        )
        .unwrap();
        for index in 0..3 {
            returned(&mut store, instance, "run_t", &[Value::I32(index)], 33);
        }
        returned(&mut store, instance, "run_other", &[Value::I32(0)], 22);
        returned(&mut store, instance, "run_other", &[Value::I32(1)], 33);
        if policy == ExecutionPolicy::Compact {
            let (hits, misses, evictions) = store.compact_indirect_cache_counts();
            assert!(hits >= 3 && misses >= 3 && evictions > 0);
        }
        no_cache_for_other_policies(&store, policy);
    }
}

#[test]
fn p042_imported_table_aliases_and_instances_do_not_reuse_other_targets() {
    for policy in POLICIES {
        let engine = make_engine(policy, None, 16);
        let consumer_module = engine.compile(CONSUMER).unwrap();
        for _ in 0..2 {
            let mut store = Store::new(&engine);
            let provider = load(&engine, &mut store, PROVIDER);
            let External::Table(table) = provider.export(&store, "shared").unwrap() else {
                panic!()
            };
            let mut linker = Linker::new(&engine);
            linker
                .define("provider", "shared", External::Table(table))
                .unwrap();
            let consumer = linker.instantiate(&mut store, &consumer_module).unwrap();
            for _ in 0..2 {
                returned(&mut store, consumer, "run", &[Value::I32(0)], 10);
            }
            call(&mut store, consumer, "set_c30", &[Value::I32(0)]).unwrap();
            returned(&mut store, provider, "run", &[Value::I32(0)], 30);
            returned(&mut store, consumer, "run", &[Value::I32(0)], 30);
            let replacement = store.read_table(table, 1).unwrap();
            store.write_table(table, 0, replacement).unwrap();
            returned(&mut store, consumer, "run", &[Value::I32(0)], 20);
            returned(&mut store, consumer, "grow_c30", &[Value::I32(1)], 2);
            returned(&mut store, provider, "run", &[Value::I32(2)], 30);
            let isolated = load(&engine, &mut store, PROVIDER);
            returned(&mut store, isolated, "run", &[Value::I32(0)], 10);
            no_cache_for_other_policies(&store, policy);
            if policy == ExecutionPolicy::Compact {
                assert!(store.compact_indirect_cache_counts().0 > 0);
            }
        }
    }
}

#[test]
fn p042_indirect_null_mismatch_bounds_and_every_trap_fuel_prefix_are_original() {
    for (index, message) in [
        (1, "null indirect call"),
        (2, "indirect call type mismatch"),
        (3, "table out of bounds"),
    ] {
        for fuel in 0..=3 {
            for policy in POLICIES {
                let engine = make_engine(policy, Some(fuel), 16);
                let mut store = Store::new(&engine);
                let instance = load(&engine, &mut store, INVALID);
                for _ in 0..2 {
                    assert_eq!(
                        view(call(&mut store, instance, "run", &[Value::I32(index)])),
                        if fuel < 2 {
                            Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                        } else {
                            Err((ErrorKind::Trap, message.into()))
                        }
                    );
                }
                assert_eq!(store.compact_indirect_cache_counts().0, 0);
            }
        }
    }
    for fuel in 0..=6 {
        for policy in POLICIES {
            let engine = make_engine(policy, Some(fuel), 16);
            let mut store = Store::new(&engine);
            let instance = load(&engine, &mut store, INVALID);
            for _ in 0..2 {
                assert_eq!(
                    view(call(&mut store, instance, "run", &[Value::I32(0)])),
                    if fuel < 5 {
                        Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                    } else {
                        Ok(RunOutcome::Returned(vec![Value::I32(42)]))
                    }
                );
            }
            if policy == ExecutionPolicy::Compact && fuel >= 2 {
                assert!(store.compact_indirect_cache_counts().0 > 0);
            }
        }
    }
}

#[test]
fn p042_typed_call_ref_null_fuel_prefix_foreign_and_argument_validation_remain_checked() {
    for policy in POLICIES {
        let engine = make_engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let instance = load(&engine, &mut store, REFS);
        let inc = func(&store, instance, "inc");
        let wrong = func(&store, instance, "wrong");
        for name in ["call", "tail_call"] {
            for _ in 0..2 {
                returned(
                    &mut store,
                    instance,
                    name,
                    &[Value::I32(7), Value::Ref(RefValue::Func(inc))],
                    8,
                );
            }
            assert_eq!(
                call(
                    &mut store,
                    instance,
                    name,
                    &[Value::I32(7), Value::Ref(RefValue::Func(wrong))]
                )
                .unwrap_err()
                .kind,
                ErrorKind::Host
            );
        }
        for name in ["call_inc", "tail_inc"] {
            for _ in 0..2 {
                returned(&mut store, instance, name, &[Value::I32(7)], 8);
            }
        }
        for name in ["call_null", "tail_null"] {
            let error = call(&mut store, instance, name, &[Value::I32(7)]).unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (ErrorKind::Trap, "null call_ref")
            );
        }
        let mut other = Store::new(&engine);
        let other_instance = load(&engine, &mut other, REFS);
        let foreign = func(&other, other_instance, "inc");
        assert_eq!(
            call(
                &mut store,
                instance,
                "call",
                &[Value::I32(7), Value::Ref(RefValue::Func(foreign))]
            )
            .unwrap_err()
            .kind,
            ErrorKind::Host
        );
        no_cache_for_other_policies(&store, policy);
        if policy == ExecutionPolicy::Compact {
            assert!(store.compact_indirect_cache_counts().0 >= 4);
        }
        for fuel in 0..=4 {
            let limited = make_engine(policy, Some(fuel), 16);
            let mut store = Store::new(&limited);
            let instance = load(&limited, &mut store, REFS);
            for name in ["call_null", "tail_null"] {
                assert_eq!(view(call(&mut store, instance, name, &[Value::I32(7)])),
                    if fuel < 3 { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                    else { Err((ErrorKind::Trap, "null call_ref".into())) });
            }
            assert_eq!(store.compact_indirect_cache_counts().0, 0);
        }
    }
}

#[test]
fn p042_cached_host_targets_preserve_gc_roots_reentry_results_and_suspension() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = make_engine(policy, None, 32);
        let mut store = Store::new(&engine);
        let nested_instance = load(
            &engine,
            &mut store,
            include_bytes!("../fixtures/performance/p041/direct-numeric.wasm"),
        );
        let nested = func(&store, nested_instance, "zero_run");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        let growth_engine = engine.clone();
        let growth_module = engine
            .compile(include_bytes!("../fixtures/performance/p041/direct-numeric.wasm"))
            .unwrap();
        let host = store
            .create_host_func_with_caller(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, args| {
                    let Value::I32(value) = args[0] else { panic!() };
                    if value == 5 {
                        let stats = caller.collect()?;
                        assert_eq!((stats.live, stats.reclaimed), (1, 0));
                        for _ in 0..17 {
                            Linker::new(&growth_engine)
                                .instantiate(caller.store(), &growth_module)?;
                        }
                        assert_eq!(
                            caller.invoke(nested, &[])?,
                            RunOutcome::Returned(vec![Value::I32(29)])
                        );
                    }
                    observed.lock().unwrap().push(value);
                    Ok(vec![Value::I32(value + 3)])
                },
            )
            .unwrap();
        let module = engine.compile(HOST).unwrap();
        let mut linker = Linker::new(&engine);
        linker.define("env", "host", External::Func(host)).unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        for name in ["indirect", "tail_indirect", "by_ref", "tail_by_ref"] {
            for _ in 0..2 {
                returned(&mut store, instance, name, &[Value::I32(7)], 10);
            }
        }
        returned(&mut store, instance, "root", &[], 91);
        assert_eq!(*seen.lock().unwrap(), vec![7, 7, 7, 7, 7, 7, 7, 7, 5]);
        assert_eq!(store.collect().unwrap().reclaimed, 1);
        if policy == ExecutionPolicy::Compact {
            assert!(store.compact_indirect_cache_counts().0 >= 5);
        }
        let suspended = load(
            &engine,
            &mut store,
            include_bytes!("../../examples/suspension.wasm"),
        );
        let suspend = func(&store, suspended, "run");
        let suspending = store
            .create_host_func_with_outcome(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, _| caller.invoke(suspend, &[]),
            )
            .unwrap();
        let mut another = Linker::new(&engine);
        another
            .define("env", "host", External::Func(suspending))
            .unwrap();
        let boundary = another.instantiate(&mut store, &module).unwrap();
        for name in ["indirect", "tail_indirect", "by_ref", "tail_by_ref"] {
            for _ in 0..2 {
                let error = call(&mut store, boundary, name, &[Value::I32(7)]).unwrap_err();
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
}

#[test]
fn p042_table64_large_indices_and_tail_logical_depth_do_not_alias_or_grow() {
    for policy in POLICIES {
        let engine = make_engine(policy, None, 1);
        let mut store = Store::new(&engine);
        let instance = load(&engine, &mut store, REFS);
        for _ in 0..2 {
            returned(&mut store, instance, "tail_inc", &[Value::I32(7)], 8);
        }
        let error = call(&mut store, instance, "call_inc", &[Value::I32(7)]).unwrap_err();
        assert_eq!(
            (error.kind, error.message.as_str()),
            (ErrorKind::ResourceLimit, "call depth exhausted")
        );
        let engine = make_engine(policy, None, 16);
        let mut store = Store::new(&engine);
        let wide = load(&engine, &mut store, WIDE);
        for _ in 0..2 {
            returned(&mut store, wide, "run", &[Value::I64(0)], 64);
        }
        for index in [1, 1i64 << 32, i64::MAX, -1] {
            let error = call(&mut store, wide, "run", &[Value::I64(index)]).unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (ErrorKind::Trap, "table out of bounds")
            );
        }
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_indirect_cache_counts().0, 1);
        }
    }
}

#[test]
fn p042_cache_hits_keep_materialized_source_cursors_and_charged_fuel() {
    for pc in 0..3 {
        let mut snapshots = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let engine = make_engine(policy, Some(5), 16);
            let mut store = Store::new(&engine);
            let instance = load(&engine, &mut store, INVALID);
            let run = func(&store, instance, "run");
            assert_eq!(
                store.invoke(run, &[Value::I32(0)]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(42)])
            );
            store.request_materialization_roundtrip(run, pc).unwrap();
            assert_eq!(
                store.invoke(run, &[Value::I32(0)]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(42)])
            );
            let snapshot = store.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc, pc);
            assert_eq!(snapshot.remaining_fuel, Some([4, 3, 0][pc]));
            assert!(snapshot.pending_boundary_charged);
            assert_eq!(store.materialization_roundtrips(), 1);
            snapshots.push(snapshot);
            if policy == ExecutionPolicy::Compact {
                assert_eq!(store.compact_indirect_cache_counts().0, 1);
            }
        }
        assert_eq!(snapshots[0], snapshots[1]);
    }
}
