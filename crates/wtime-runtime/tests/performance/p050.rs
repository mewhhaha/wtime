#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const BYTES: &[u8] = include_bytes!("../fixtures/performance/p050/stack_calls.wasm");
const POLICIES: [ExecutionPolicy; 3] = [
    ExecutionPolicy::Reference,
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Compact,
];
fn load(policy: ExecutionPolicy, mut config: Config, bytes: &[u8]) -> (Store, Instance) {
    config.features = FeatureSet::all_phase3_core();
    let e = Engine::with_execution_policy(config, policy).unwrap();
    let m = e.compile(bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    (s, i)
}
fn function(s: &Store, i: Instance, name: &str) -> Func {
    let External::Func(f) = i.export(s, name).unwrap() else {
        panic!()
    };
    f
}
fn run(s: &mut Store, i: Instance, name: &str, x: i32) -> Result<RunOutcome> {
    s.invoke(function(s, i, name), &[Value::I32(x)])
}
fn error(result: Result<RunOutcome>, message: &str) {
    let e = result.unwrap_err();
    assert_eq!(
        (e.kind, e.message.as_str()),
        (ErrorKind::ResourceLimit, message)
    );
}

#[test]
fn p050_low_logical_call_depth_fails_without_physical_frame() {
    for policy in POLICIES {
        for fuel in [2u64, 3, 12] {
            let mut c = Config::default();
            c.limits.max_call_depth = 1;
            c.limits.fuel = Some(fuel);
            let (mut s, i) = load(policy, c, BYTES);
            error(
                run(&mut s, i, "run", 5),
                if fuel < 3 {
                    "fuel exhausted"
                } else {
                    "call depth exhausted"
                },
            );
            assert_eq!(s.compact_inline_leaf_calls(), 0);
            assert_eq!(
                s.compact_dispatches(),
                if policy == ExecutionPolicy::Compact {
                    fuel.min(3)
                } else {
                    0
                }
            );
        }
    }
}

#[test]
fn p050_fuel_stop_in_inline_leaf_reconstructs_arguments_and_return_point() {
    for fuel in 0u64..=12 {
        for policy in POLICIES {
            let mut c = Config::default();
            c.limits.fuel = Some(fuel);
            let (mut s, i) = load(policy, c, BYTES);
            let result = run(&mut s, i, "run", 5);
            if fuel < 11 {
                error(result, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(107)]));
            }
            assert_eq!(
                s.compact_dispatches(),
                if policy == ExecutionPolicy::Compact {
                    fuel.min(11)
                } else {
                    0
                }
            );
            assert_eq!(
                s.compact_inline_leaf_calls(),
                u64::from(policy == ExecutionPolicy::Compact && fuel >= 9)
            );
        }
    }
    for pc in 0..6 {
        let mut snapshots = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let mut c = Config::default();
            c.limits.fuel = Some(11);
            let (mut s, i) = load(policy, c, BYTES);
            let step = function(&s, i, "step");
            s.request_materialization_roundtrip(step, pc).unwrap();
            assert_eq!(
                run(&mut s, i, "run", 5).unwrap(),
                RunOutcome::Returned(vec![Value::I32(107)])
            );
            let snap = s.execution_snapshot().unwrap().clone();
            assert_eq!(snap.cursor.source_pc, pc);
            assert_eq!(snap.frame_count, 2);
            assert_eq!(snap.remaining_fuel, Some(7 - pc as u64));
            assert!(snap.pending_boundary_charged && !snap.truncated);
            assert_eq!(s.materialization_roundtrips(), 1);
            assert_eq!(s.compact_inline_leaf_calls(), 0);
            snapshots.push(snap);
        }
        assert_eq!(snapshots[0], snapshots[1], "callee source{pc}");
    }
}

#[test]
fn p050_recursive_imported_effectful_leaves_decline_and_expansion_bounded() {
    for policy in POLICIES {
        let (mut s, i) = load(policy, Config::default(), BYTES);
        let e = run(&mut s, i, "division_by_zero", 5).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Trap);
        assert_eq!(e.trap_code, Some(TrapCode::IntegerDivisionByZero));
        assert_eq!(s.compact_inline_leaf_calls(), 0);
        if policy == ExecutionPolicy::Compact {
            assert!(s.compact_inline_declines() > 0);
        }
        let before = s.compact_inline_declines();
        assert_eq!(
            run(&mut s, i, "nested_call", 5).unwrap(),
            RunOutcome::Returned(vec![Value::I32(16)])
        );
        assert_eq!(
            s.compact_inline_leaf_calls(),
            u64::from(policy == ExecutionPolicy::Compact)
        );
        if policy == ExecutionPolicy::Compact {
            assert!(s.compact_inline_declines() > before);
        }
        for cross_instance in [false, true] {
            let c = Config {
                features: FeatureSet::all_phase3_core(),
                ..Default::default()
            };
            let e = Engine::with_execution_policy(c, policy).unwrap();
            let mut s = Store::new(&e);
            let target = if cross_instance {
                let module = e.compile(BYTES).unwrap();
                let instance = Linker::new(&e).instantiate(&mut s, &module).unwrap();
                function(&s, instance, "step")
            } else {
                s.create_host_func(
                    FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                    |args| {
                        let Value::I32(x) = args[0] else { panic!() };
                        Ok(vec![Value::I32(x.wrapping_add(7))])
                    },
                )
                .unwrap()
            };
            let module = e
                .compile(include_bytes!(
                    "../fixtures/performance/p050/imported_leaf.wasm"
                ))
                .unwrap();
            let mut linker = Linker::new(&e);
            linker
                .define("env", "leaf", External::Func(target))
                .unwrap();
            let i = linker.instantiate(&mut s, &module).unwrap();
            assert_eq!(
                run(&mut s, i, "run", 5).unwrap(),
                RunOutcome::Returned(vec![Value::I32(if cross_instance { 16 } else { 12 })])
            );
            assert_eq!(s.compact_inline_leaf_calls(), 0);
        }
        let (mut s, i) = load(
            policy,
            Config::default(),
            include_bytes!("../fixtures/performance/p050/ordinary_recursion.wasm"),
        );
        let mut expected = 1u64;
        for _ in 0..17 {
            expected = (expected * 3 + 1) & 0xffff_ffff;
        }
        assert_eq!(
            run(&mut s, i, "run", 17).unwrap(),
            RunOutcome::Returned(vec![Value::I32(expected as u32 as i32)])
        );
        assert_eq!(s.compact_inline_leaf_calls(), 0);
    }
}

#[test]
fn p050_direct_leaf_results_and_wrapping_match() {
    for policy in POLICIES {
        let (mut s, i) = load(policy, Config::default(), BYTES);
        let mut values = vec![i32::MIN, i32::MAX, -1, 0, 1, 5, 0x40000000];
        let mut seed = 0x050abcdefu32;
        for _ in 0..256 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            values.push(seed as i32);
        }
        for x in values {
            let expected = ((u64::from(x as u32) * 3 + 92) & 0xffff_ffff) as u32 as i32;
            let before = s.compact_inline_leaf_calls();
            assert_eq!(
                run(&mut s, i, "run", x).unwrap(),
                RunOutcome::Returned(vec![Value::I32(expected)])
            );
            assert_eq!(
                s.compact_inline_leaf_calls() - before,
                u64::from(policy == ExecutionPolicy::Compact)
            );
        }
        for n in [0i32, 1, 17, 32, 1024, 10000] {
            let (mut s, i) = load(
                policy,
                Config::default(),
                include_bytes!("../fixtures/performance/p050/direct_calls.wasm"),
            );
            let mut expected = 1u64;
            for _ in 0..n {
                expected = (expected * 3 + 1) & 0xffff_ffff;
            }
            assert_eq!(
                run(&mut s, i, "run", n).unwrap(),
                RunOutcome::Returned(vec![Value::I32(expected as u32 as i32)])
            );
            assert_eq!(
                s.compact_inline_leaf_calls(),
                if policy == ExecutionPolicy::Compact {
                    n as u64
                } else {
                    0
                }
            );
            assert_eq!(
                s.compact_bound_calls(),
                if policy == ExecutionPolicy::Compact {
                    n as u64
                } else {
                    0
                }
            );
        }
    }
}

#[test]
fn p050_retained_operands_keep_global_peak_and_source_limits() {
    for policy in POLICIES {
        for (limit, fuel, message) in [(2, 5, "fuel exhausted"), (2, 6, "operand stack limit")] {
            let mut c = Config::default();
            c.limits.max_stack_height = limit;
            c.limits.fuel = Some(fuel);
            let (mut s, i) = load(policy, c, BYTES);
            error(run(&mut s, i, "run", 5), message);
            assert_eq!(s.compact_inline_leaf_calls(), 0);
        }
        let mut c = Config::default();
        c.limits.max_stack_height = 3;
        c.limits.fuel = Some(11);
        let (mut s, i) = load(policy, c, BYTES);
        assert_eq!(
            run(&mut s, i, "run", 5).unwrap(),
            RunOutcome::Returned(vec![Value::I32(107)])
        );
        assert_eq!(
            s.compact_inline_leaf_calls(),
            u64::from(policy == ExecutionPolicy::Compact)
        );
        let (mut s, i) = load(policy, Config::default(), BYTES);
        s.cancellation_token().cancel();
        error(run(&mut s, i, "run", 5), "execution cancelled");
        assert_eq!(s.compact_inline_leaf_calls(), 0);
    }
}
