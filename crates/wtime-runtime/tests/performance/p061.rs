#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const LOOP: &[u8] = include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm");
const MULTI: &[u8] = include_bytes!("../fixtures/performance/p040/multivalue_loop.wasm");
const EDGE: &[u8] = include_bytes!("../fixtures/performance/p058/edge_values.wasm");
const NESTED: &[u8] = include_bytes!("../fixtures/performance/p040/nested_if_results.wasm");
fn loaded(
    policy: ExecutionPolicy,
    config: Config,
    bytes: &[u8],
    export: &str,
) -> (Store, Instance, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, export).unwrap() else {
        panic!()
    };
    (store, instance, function)
}
fn outcome(result: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    result.map_err(|error| (error.kind, error.message))
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut view = store.execution_snapshot().unwrap().clone();
    // Compilations/instances have separate canonical context identities.
    for frame in &mut view.frames {
        frame.module_fingerprint = 0;
    }
    view
}
fn compare_trace(bytes: &[u8], args: &[Value], expected: &[Value], pcs: &[usize]) -> u64 {
    let mut visits = vec![0; 128];
    let mut dispatches = 0;
    for (index, pc) in pcs.iter().copied().enumerate() {
        let mut observations = vec![];
        for policy in [
            ExecutionPolicy::Decoded,
            ExecutionPolicy::Register,
            #[cfg(feature = "compact-executor")]
            ExecutionPolicy::Compact,
        ] {
            let mut config = Config::default();
            config.limits.fuel = Some(index as u64 + 1);
            let (mut store, _, function) = loaded(policy, config, bytes, "run");
            store
                .request_materialization_roundtrip_after_visits(function, pc, visits[pc])
                .unwrap();
            let result = outcome(store.invoke(function, args));
            let view = snapshot(&store);
            assert!(!view.truncated && view.pending_boundary_charged);
            assert_eq!(view.remaining_fuel, Some(0));
            assert_eq!(view.cursor.source_pc, pc);
            assert_eq!(store.materialization_roundtrips(), 1);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), index as u64 + 1);
                assert_eq!(store.register_fallback_frames(), 0);
                dispatches += store.register_dispatches();
            }
            observations.push((result, view));
        }
        for observation in &observations[1..] {
            assert_eq!(
                &observations[0], observation,
                "dynamic visit {index}, source PC {pc}"
            );
        }
        visits[pc] += 1;
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let mut config = Config::default();
        config.limits.fuel = Some(pcs.len() as u64);
        let (mut store, _, function) = loaded(policy, config, bytes, "run");
        assert_eq!(
            store.invoke(function, args).unwrap(),
            RunOutcome::Returned(expected.to_vec())
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), pcs.len() as u64);
        }
    }
    dispatches
}
#[test]
fn p061_control_edges_and_repeated_visits() {
    let mut path: Vec<_> = (0..8).collect();
    for _ in 0..4 {
        path.extend(8..21);
    }
    path.extend(8..12);
    path.extend([23, 24]);
    assert_eq!(path.len(), 66);
    let mut count = compare_trace(LOOP, &[], &[Value::I32(32)], &path);
    let mut path: Vec<_> = (0..4).collect();
    for _ in 0..2 {
        path.extend(4..13);
    }
    path.extend(4..8);
    path.push(15);
    count += compare_trace(MULTI, &[], &[Value::I32(2), Value::I64(40)], &path);
    count += compare_trace(
        EDGE,
        &[],
        &[Value::I32(7), Value::I64(9)],
        &[0, 1, 2, 3, 4, 6],
    );
    for (args, result, path) in [
        (
            [Value::I32(1), Value::I32(1)],
            [Value::I32(41), Value::I64(420)],
            vec![0, 1, 2, 3, 4, 5, 6, 9, 10, 13, 14],
        ),
        (
            [Value::I32(1), Value::I32(0)],
            [Value::I32(42), Value::I64(421)],
            vec![0, 1, 2, 3, 7, 8, 9, 10, 13, 14],
        ),
        (
            [Value::I32(0), Value::I32(0)],
            [Value::I32(-1), Value::I64(-2)],
            vec![0, 1, 11, 12, 13, 14],
        ),
    ] {
        count += compare_trace(NESTED, &args, &result, &path);
    }
    assert!(count > 0);
    eprintln!("P061: 126 source prefixes; {count} register source events");
}
#[test]
fn p061_every_source_prefix_and_logical_limits() {
    let bytes = include_bytes!("../fixtures/performance/p047/p047_local_add.wasm");
    compare_trace(
        bytes,
        &[Value::I32(i32::MAX), Value::I32(1)],
        &[Value::I32(i32::MIN)],
        &[0, 1, 2, 3],
    );
    for fuel in 0..=5 {
        let mut results = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut config = Config::default();
            config.limits.fuel = Some(fuel);
            let (mut store, _, function) = loaded(policy, config, bytes, "run");
            results.push(outcome(
                store.invoke(function, &[Value::I32(21), Value::I32(4)]),
            ));
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), fuel.min(4));
            }
        }
        assert_eq!(results[0], results[1]);
    }
    // Each function validates at height two; caller live operands make the
    // eligible numeric callee reach global logical height three.
    for (limit, fuel, message) in [
        (2, 4, Some("fuel exhausted")),
        (2, 5, Some("operand stack limit")),
        (3, 8, None),
    ] {
        let mut results = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut config = Config::default();
            config.limits.max_stack_height = limit;
            config.limits.fuel = Some(fuel);
            let (mut store, _, function) = loaded(
                policy,
                config,
                include_bytes!("../fixtures/performance/accounting/transient_stack.wasm"),
                "run",
            );
            let result = outcome(store.invoke(function, &[]));
            match message {
                Some(message) => {
                    assert_eq!(result, Err((ErrorKind::ResourceLimit, message.to_owned())))
                }
                None => assert_eq!(result, Ok(RunOutcome::Returned(vec![Value::I32(10)]))),
            }
            results.push(result);
            if policy == ExecutionPolicy::Register {
                assert_eq!(
                    store.register_dispatches(),
                    if message.is_some() { 2 } else { 4 }
                );
                assert_eq!(store.register_fallback_frames(), 1);
            }
        }
        assert_eq!(results[0], results[1]);
    }
}
#[test]
fn p061_whole_function_fallback_and_owner_guards() {
    for bytes in [
        include_bytes!("../fixtures/performance/p058/numeric_slots.wasm").as_slice(),
        include_bytes!("../fixtures/performance/p047/p047_mixed_externref_local.wasm").as_slice(),
    ] {
        let mut observations = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let (mut store, _, function) = loaded(policy, Config::default(), bytes, "run");
            let args = if bytes
                == include_bytes!("../fixtures/performance/p058/numeric_slots.wasm").as_slice()
            {
                vec![Value::I32(21), Value::I32(4)]
            } else {
                vec![Value::I32(21)]
            };
            observations.push(outcome(store.invoke(function, &args)));
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), 0);
                assert_eq!(store.register_fallback_frames(), 1);
            }
        }
        assert_eq!(observations[0], observations[1]);
    }
    let (mut first, _, function) =
        loaded(ExecutionPolicy::Register, Config::default(), LOOP, "run");
    let (mut second, _, _) = loaded(ExecutionPolicy::Register, Config::default(), LOOP, "run");
    assert_eq!(
        second.invoke(function, &[]).unwrap_err().kind,
        ErrorKind::Host
    );
    assert_eq!(second.register_dispatches(), 0);
    assert_eq!(
        first.invoke(function, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(32)])
    );
    assert_eq!(first.register_dispatches(), 66);
}
#[test]
fn p061_cancel_before_entry() {
    let (mut store, _, function) =
        loaded(ExecutionPolicy::Register, Config::default(), LOOP, "run");
    store.cancellation_token().cancel();
    let error = store.invoke(function, &[]).unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (ErrorKind::ResourceLimit, "execution cancelled")
    );
    assert_eq!(store.register_dispatches(), 0);
}

#[test]
fn p061_integer_boundaries_and_source_traps() {
    let bytes = include_bytes!("../fixtures/performance/p061/integer_ops.wasm");
    let unary = [
        ("i32_eqz", false),
        ("i64_eqz", true),
        ("i32_clz", false),
        ("i32_ctz", false),
        ("i32_popcnt", false),
        ("i64_clz", true),
        ("i64_ctz", true),
        ("i64_popcnt", true),
        ("i32_wrap_i64", true),
        ("i64_extend_i32_s", false),
        ("i64_extend_i32_u", false),
        ("i32_extend8_s", false),
        ("i32_extend16_s", false),
        ("i64_extend8_s", true),
        ("i64_extend16_s", true),
        ("i64_extend32_s", true),
    ];
    let binary = [
        "eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u", "add", "sub",
        "mul", "div_s", "div_u", "rem_s", "rem_u", "and", "or", "xor", "shl", "shr_s", "shr_u",
        "rotl", "rotr",
    ];
    let inputs = [
        0,
        1,
        31,
        32,
        63,
        64,
        0x7fffffff,
        0x80000000,
        0xffffffff,
        0x7fffffffffffffff,
        0x8000000000000000,
        u64::MAX,
    ];
    let mut cases = vec![];
    for (name, wide) in unary {
        cases.push((name.to_owned(), wide, false));
    }
    for (prefix, wide) in [("i32", false), ("i64", true)] {
        for name in binary {
            cases.push((format!("{prefix}_{name}"), wide, true));
        }
    }
    assert_eq!(cases.len(), 66);
    let mut executions = 0;
    let mut events = 0;
    for (name, wide, two) in cases {
        let (mut decoded, _, normal) =
            loaded(ExecutionPolicy::Decoded, Config::default(), bytes, &name);
        let (mut register, _, fast) =
            loaded(ExecutionPolicy::Register, Config::default(), bytes, &name);
        let values = |a, b| {
            let v = |n: u64| {
                if wide {
                    Value::I64(n as i64)
                } else {
                    Value::I32(n as u32 as i32)
                }
            };
            if two { vec![v(a), v(b)] } else { vec![v(a)] }
        };
        let mut seed = 0x123456789abcdef0u64;
        let mut pairs = vec![];
        for a in inputs {
            if two {
                for b in inputs {
                    pairs.push((a, b));
                }
            } else {
                pairs.push((a, 0));
            }
        }
        for _ in 0..16 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            pairs.push((seed, seed.rotate_left(21).wrapping_add(0x12345)));
        }
        for (a, b) in pairs {
            let args = values(a, b);
            let before = register.register_dispatches();
            let reference = outcome(decoded.invoke(normal, &args));
            let candidate = outcome(register.invoke(fast, &args));
            assert_eq!(reference, candidate, "{name} operands {a:x}, {b:x}");
            let expected_events = if two {
                if candidate.is_err() { 3 } else { 4 }
            } else {
                3
            };
            assert_eq!(
                register.register_dispatches() - before,
                expected_events,
                "{name} must execute register tier"
            );
            assert_eq!(register.register_fallback_frames(), 0);
            executions += 1;
            events += expected_events;
        }
    }
    // Observe the original divide boundary, including operands and remaining
    // fuel, for both division traps; implicit end must never run after a trap.
    for (name, args, message) in [
        (
            "i32_div_s",
            vec![Value::I32(i32::MIN), Value::I32(-1)],
            "integer overflow",
        ),
        (
            "i64_div_s",
            vec![Value::I64(i64::MIN), Value::I64(-1)],
            "integer overflow",
        ),
        (
            "i32_div_u",
            vec![Value::I32(7), Value::I32(0)],
            "integer divide by zero",
        ),
        (
            "i64_rem_s",
            vec![Value::I64(7), Value::I64(0)],
            "integer divide by zero",
        ),
    ] {
        let mut observations = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut config = Config::default();
            config.limits.fuel = Some(3);
            let (mut store, _, function) = loaded(policy, config, bytes, name);
            store
                .request_materialization_roundtrip(function, 2)
                .unwrap();
            assert_eq!(
                outcome(store.invoke(function, &args)),
                Err((ErrorKind::Trap, message.to_owned()))
            );
            let view = snapshot(&store);
            assert_eq!(view.operands.len(), 2);
            assert_eq!(view.remaining_fuel, Some(0));
            observations.push(view);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), 3);
            }
        }
        assert_eq!(observations[0], observations[1]);
    }
    eprintln!("P061 INTEGER: {executions} decoded oracle cases; {events} register source events");
}
