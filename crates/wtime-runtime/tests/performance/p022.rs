use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use wtime_runtime::*;

fn policies() -> Vec<ExecutionPolicy> {
    #[allow(unused_mut)]
    let mut p = vec![ExecutionPolicy::Decoded, ExecutionPolicy::Portable];
    #[cfg(feature = "execution-reference")]
    p.push(ExecutionPolicy::Reference);
    #[cfg(feature = "compact-executor")]
    p.push(ExecutionPolicy::Compact);
    p
}
fn engine(policy: ExecutionPolicy, fuel: Option<u64>) -> Engine {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.fuel = fuel;
    Engine::with_execution_policy(c, policy).unwrap()
}
fn function(s: &Store, i: Instance, name: &str) -> Func {
    let External::Func(f) = i.export(s, name).unwrap() else {
        panic!()
    };
    f
}
fn call(s: &mut Store, i: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    s.invoke(function(s, i, name), args)
}
fn path_proof(s: &Store, policy: ExecutionPolicy) {
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    if policy == ExecutionPolicy::Compact {
        assert!(s.compact_dispatches() > 0);
    }
    #[cfg(feature = "performance-counters")]
    if policy != ExecutionPolicy::Reference {
        assert!(s.performance_counters().source_dispatches > 0);
    }
    #[cfg(feature = "execution-reference")]
    if policy == ExecutionPolicy::Reference {
        assert!(s.reference_dispatches() > 0);
    }
    #[cfg(not(feature = "execution-reference"))]
    let _ = (s, policy);
}
#[test]
fn p022_prefix_effects_and_fuel_match_all_policies() {
    let bytes = include_bytes!("../fixtures/performance/cursor/store_successor.wasm");
    for policy in policies() {
        for fuel in 0..=7 {
            let e = engine(policy, Some(fuel));
            let m = e.compile(bytes).unwrap();
            let mut s = Store::new(&e);
            let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
            let result = call(&mut s, i, "run", &[]);
            if fuel < 6 {
                let error = result.unwrap_err();
                assert_eq!(error.kind, ErrorKind::ResourceLimit);
                assert_eq!(error.message, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
            }
            let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
                panic!()
            };
            let all = s.read_memory(mem, 0, 65536).unwrap();
            assert_eq!(
                &all[..4],
                if fuel >= 3 {
                    &[7, 0, 0, 0]
                } else {
                    &[0, 0, 0, 0]
                }
            );
            assert!(all[4..].iter().all(|&x| x == 0));
            if fuel > 0 {
                path_proof(&s, policy);
            }
        }
    }
}
const HOST: &[u8] =
    include_bytes!("../fixtures/performance/bench_wasm/integration/host_reentry.wasm");
#[test]
fn p022_host_reentry_collection_preserves_outer_root_and_state() {
    for policy in policies() {
        let e = engine(policy, None);
        let m = e.compile(HOST).unwrap();
        let mut s = Store::new(&e);
        let nested = Arc::new(Mutex::new(None::<Func>));
        let slot = nested.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let host = s
            .create_host_func_with_caller(FuncType::new(vec![], vec![]), move |caller, _| {
                observed.fetch_add(1, Ordering::SeqCst);
                assert_eq!(caller.collect()?.live, 1);
                let f = slot.lock().unwrap().unwrap();
                assert_eq!(
                    caller.invoke(f, &[])?,
                    RunOutcome::Returned(vec![Value::I32(19)])
                );
                assert_eq!(caller.collect()?.live, 1);
                Ok(vec![])
            })
            .unwrap();
        let mut l = Linker::new(&e);
        l.define("host", "collect", External::Func(host)).unwrap();
        let i = l.instantiate(&mut s, &m).unwrap();
        *nested.lock().unwrap() = Some(function(&s, i, "nested"));
        assert_eq!(
            call(&mut s, i, "run", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let External::Global(g) = i.export(&s, "phase").unwrap() else {
            panic!()
        };
        assert_eq!(s.get_global(g).unwrap(), Value::I32(2));
        let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
            panic!()
        };
        let all = s.read_memory(mem, 0, 65536).unwrap();
        assert_eq!(&all[..8], &[11, 0, 0, 0, 19, 0, 0, 0]);
        assert!(all[8..].iter().all(|&x| x == 0));
        assert_eq!(s.collect().unwrap().live, 0);
        path_proof(&s, policy);
    }
}
#[test]
fn p022_cross_module_continuation_preserves_context_roots_and_one_shot() {
    for policy in policies() {
        let e = engine(policy, None);
        let pm = e
            .compile(include_bytes!(
                "../fixtures/performance/bench_wasm/integration/cross_module_provider.wasm"
            ))
            .unwrap();
        let cm = e
            .compile(include_bytes!(
                "../fixtures/performance/bench_wasm/integration/cross_module_consumer.wasm"
            ))
            .unwrap();
        let mut s = Store::new(&e);
        let p = Linker::new(&e).instantiate(&mut s, &pm).unwrap();
        let mut l = Linker::new(&e);
        l.define("provider", "yield", p.export(&s, "yield").unwrap())
            .unwrap();
        let i = l.instantiate(&mut s, &cm).unwrap();
        drop(pm);
        drop(cm);
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = call(&mut s, i, "run", &[]).unwrap()
        else {
            panic!()
        };
        assert!(payload.is_empty());
        assert!(s.continuation_bytes() > 0);
        let captured_bytes = s.continuation_bytes();
        let root = s.root(continuation).unwrap();
        let arg = Value::Ref(RefValue::Gc(root.reference()));
        s.release_published(&[arg]);
        assert!(s.collect().unwrap().live > 0);
        assert_eq!(
            call(&mut s, i, "resume", &[arg]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert!(s.continuation_bytes() <= captured_bytes);
        assert_eq!(
            call(&mut s, i, "resume", &[arg]).unwrap_err().kind,
            ErrorKind::Trap
        );
        drop(root);
        s.collect().unwrap();
        assert_eq!(s.gc_payload_bytes(), 0);
        path_proof(&s, policy);
    }
}
#[test]
fn p022_budget_priority_and_continuation_cleanup_match_policies() {
    for policy in policies() {
        for (fuel, message) in [(4, "fuel exhausted"), (5, "operand stack limit")] {
            let mut c = Config {
                features: FeatureSet::all_phase3_core(),
                ..Config::default()
            };
            c.limits.fuel = Some(fuel);
            c.limits.max_stack_height = 2;
            let e = Engine::with_execution_policy(c, policy).unwrap();
            let m = e
                .compile(include_bytes!(
                    "../fixtures/performance/accounting/transient_stack.wasm"
                ))
                .unwrap();
            let mut s = Store::new(&e);
            let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
            let error = call(&mut s, i, "run", &[]).unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert_eq!(error.message, message);
            path_proof(&s, policy);
        }
        let mut c = Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        };
        c.limits.max_continuation_bytes = 0;
        let e = Engine::with_execution_policy(c, policy).unwrap();
        let m = e
            .compile(include_bytes!("../../examples/suspension.wasm"))
            .unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let error = call(&mut s, i, "run", &[]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert!(error.message.contains("continuation byte budget"));
        assert_eq!(s.continuation_bytes(), 0);
        s.collect().unwrap();
        assert_eq!(s.gc_payload_bytes(), 0);
        path_proof(&s, policy);
    }
}
#[test]
fn p022_table_mutations_and_foreign_handles_are_checked() {
    for policy in policies() {
        let e = engine(policy, None);
        let m = e
            .compile(include_bytes!(
                "../fixtures/performance/bench_wasm/integration/table_targets.wasm"
            ))
            .unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let External::Table(table) = i.export(&s, "table").unwrap() else {
            panic!()
        };
        for (name, expected) in [("a", 17), ("b", 29), ("a", 17)] {
            let f = function(&s, i, name);
            s.write_table(table, 0, Value::Ref(RefValue::Func(f)))
                .unwrap();
            assert_eq!(
                call(&mut s, i, "run", &[]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(expected)])
            );
            assert!(matches!(
                s.read_table(table, 0).unwrap(),
                Value::Ref(RefValue::Func(_))
            ));
            assert!(matches!(
                s.read_table(table, 1).unwrap(),
                Value::Ref(RefValue::Null(_))
            ));
        }
        let mut foreign = Store::new(&e);
        let j = Linker::new(&e).instantiate(&mut foreign, &m).unwrap();
        let f = function(&foreign, j, "b");
        assert_eq!(
            s.write_table(table, 0, Value::Ref(RefValue::Func(f)))
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
        assert_eq!(s.invoke(f, &[]).unwrap_err().kind, ErrorKind::Host);
        path_proof(&s, policy);
    }
}
#[test]
fn p022_exception_unwind_and_stale_foreign_gc_values_are_checked() {
    for policy in policies() {
        let e = engine(policy, None);
        let m = e
            .compile(include_bytes!("../fixtures/exceptions/cross_call.wasm"))
            .unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        drop(m);
        assert_eq!(
            call(&mut s, i, "f", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        path_proof(&s, policy);
        let m = e.compile(HOST).unwrap();
        let noop = s
            .create_host_func(FuncType::new(vec![], vec![]), |_| Ok(vec![]))
            .unwrap();
        let mut l = Linker::new(&e);
        l.define("host", "collect", External::Func(noop)).unwrap();
        let i = l.instantiate(&mut s, &m).unwrap();
        let RunOutcome::Returned(values) = call(&mut s, i, "make", &[]).unwrap() else {
            panic!()
        };
        assert_eq!(
            call(&mut s, i, "get", &values).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        let mut foreign = Store::new(&e);
        let noop = foreign
            .create_host_func(FuncType::new(vec![], vec![]), |_| Ok(vec![]))
            .unwrap();
        let mut l = Linker::new(&e);
        l.define("host", "collect", External::Func(noop)).unwrap();
        let j = l.instantiate(&mut foreign, &m).unwrap();
        assert_eq!(
            call(&mut foreign, j, "get", &values).unwrap_err().kind,
            ErrorKind::Host
        );
        s.release_published(&values);
        assert_eq!(s.collect().unwrap().live, 0);
        assert_eq!(
            call(&mut s, i, "get", &values).unwrap_err().kind,
            ErrorKind::Host
        );
        path_proof(&s, policy);
    }
}

#[test]
fn p022_call_depth_and_tail_call_controls_match_policies() {
    for policy in policies() {
        let mut c = Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        };
        c.limits.max_call_depth = 16;
        let e = Engine::with_execution_policy(c, policy).unwrap();
        for (bytes, tail) in [
            (
                include_bytes!("../fixtures/performance/accounting/tail_calls.wasm").as_slice(),
                true,
            ),
            (
                include_bytes!("../fixtures/performance/accounting/ordinary_recursion.wasm")
                    .as_slice(),
                false,
            ),
        ] {
            let m = e.compile(bytes).unwrap();
            let mut s = Store::new(&e);
            let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
            let outcome = call(&mut s, i, "run", &[Value::I32(1024)]);
            if tail {
                let mut expected = 1i32;
                for _ in 0..1024 {
                    expected = expected.wrapping_mul(3).wrapping_add(1);
                }
                assert_eq!(
                    outcome.unwrap(),
                    RunOutcome::Returned(vec![Value::I32(expected)])
                );
            } else {
                let error = outcome.unwrap_err();
                assert_eq!(error.kind, ErrorKind::ResourceLimit);
                assert!(error.message.contains("call depth"));
            }
            path_proof(&s, policy);
        }
    }
}

#[test]
fn p022_gc_budget_failure_preserves_prefix_and_heap() {
    for policy in policies() {
        let mut c = Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        };
        c.limits.max_gc_bytes = 0;
        let e = Engine::with_execution_policy(c, policy).unwrap();
        let m = e.compile(HOST).unwrap();
        let mut s = Store::new(&e);
        let noop = s
            .create_host_func(FuncType::new(vec![], vec![]), |_| Ok(vec![]))
            .unwrap();
        let mut l = Linker::new(&e);
        l.define("host", "collect", External::Func(noop)).unwrap();
        let i = l.instantiate(&mut s, &m).unwrap();
        let error = call(&mut s, i, "run", &[]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert_eq!(error.message, "guest heap byte budget exceeded");
        assert_eq!(s.gc_payload_bytes(), 0);
        let External::Global(g) = i.export(&s, "phase").unwrap() else {
            panic!()
        };
        assert_eq!(s.get_global(g).unwrap(), Value::I32(0));
        let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
            panic!()
        };
        assert!(
            s.read_memory(mem, 0, 65536)
                .unwrap()
                .iter()
                .all(|&b| b == 0)
        );
        assert_eq!(s.collect().unwrap().live, 0);
        assert_eq!(
            call(&mut s, i, "nested", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(19)])
        );
        assert_eq!(s.get_global(g).unwrap(), Value::I32(2));
        path_proof(&s, policy);
    }
}
