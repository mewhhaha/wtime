#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const TABLE: &[u8] = include_bytes!("../fixtures/performance/p040/br_table_three_paths.wasm");
const PAIR: &[u8] = include_bytes!("../fixtures/performance/p060/multivalue_table.wasm");
fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8]) -> (Store, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    (store, function)
}
fn view(outcome: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    outcome.map_err(|error| (error.kind, error.message))
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut snapshot = store.execution_snapshot().unwrap().clone();
    for frame in &mut snapshot.frames {
        frame.module_fingerprint = 0;
    }
    snapshot
}
fn prefixes(bytes: &[u8], which: i32, expected: &[Value], path: &[usize]) -> usize {
    for (index, pc) in path.iter().copied().enumerate() {
        let mut observations = vec![];
        for policy in [
            ExecutionPolicy::Decoded,
            ExecutionPolicy::Register,
            #[cfg(feature = "compact-executor")]
            ExecutionPolicy::Compact,
        ] {
            let mut config = Config::default();
            config.limits.fuel = Some(index as u64 + 1);
            let (mut store, function) = loaded(policy, config, bytes);
            store
                .request_materialization_roundtrip(function, pc)
                .unwrap();
            let outcome = view(store.invoke(function, &[Value::I32(which)]));
            let snapshot = snapshot(&store);
            assert!(!snapshot.truncated && snapshot.pending_boundary_charged);
            assert_eq!(snapshot.remaining_fuel, Some(0));
            assert_eq!(snapshot.cursor.source_pc, pc);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), index as u64 + 1);
                assert_eq!(store.register_fallback_frames(), 0);
            }
            observations.push((outcome, snapshot));
        }
        for observation in &observations[1..] {
            assert_eq!(
                &observations[0], observation,
                "selector {which}, visit {index}, PC {pc}"
            );
        }
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let mut config = Config::default();
        config.limits.fuel = Some(path.len() as u64);
        let (mut store, function) = loaded(policy, config, bytes);
        assert_eq!(
            store.invoke(function, &[Value::I32(which)]).unwrap(),
            RunOutcome::Returned(expected.to_vec())
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), path.len() as u64);
        }
    }
    path.len()
}
#[test]
fn p060_each_br_table_target_and_function_return() {
    let mut checks = 0;
    for (which, expected, path) in [
        (0, 1107, vec![0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11]),
        (1, 107, vec![0, 1, 2, 3, 4, 9, 10, 11]),
        (2, 7, vec![0, 1, 2, 3, 4]),
        (-1, 7, vec![0, 1, 2, 3, 4]),
        (i32::MAX, 7, vec![0, 1, 2, 3, 4]),
    ] {
        checks += prefixes(TABLE, which, &[Value::I32(expected)], &path);
    }
    for (which, path) in [
        (0, vec![0, 1, 2, 3, 4, 5, 6, 8, 9]),
        (1, vec![0, 1, 2, 3, 4, 5, 6, 9]),
        (2, vec![0, 1, 2, 3, 4, 5, 6]),
        (-1, vec![0, 1, 2, 3, 4, 5, 6]),
    ] {
        checks += prefixes(PAIR, which, &[Value::I32(7), Value::I64(9)], &path);
    }
    assert_eq!(checks, 65);
    eprintln!(
        "P060: {checks} table source prefixes; every explicit/default target; shifted multivalue results"
    );
}
#[test]
fn p060_structural_fuel_cancel_and_operand_limits() {
    for which in [0, 1, 2, -1] {
        for fuel in 0..=12 {
            // Smaller limits reject this body during validation; dynamic
            // shared-stack limits are covered with a live caller below.
            for limit in 3..=4 {
                let mut observations = vec![];
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut config = Config::default();
                    config.limits.fuel = Some(fuel);
                    config.limits.max_stack_height = limit;
                    let (mut store, function) = loaded(policy, config, TABLE);
                    observations.push(view(store.invoke(function, &[Value::I32(which)])));
                    if policy == ExecutionPolicy::Register {
                        assert_eq!(store.register_fallback_frames(), 0);
                    }
                }
                assert_eq!(
                    observations[0], observations[1],
                    "selector {which}, fuel {fuel}, logical limit {limit}"
                );
            }
        }
    }
    // Validation permits nesting three. Two caller operands make the callee
    // reach logical height four immediately before its br_table at PC 4.
    for (limit, fuel, expected) in [
        (
            3,
            8,
            Err((ErrorKind::ResourceLimit, "fuel exhausted".into())),
        ),
        (
            3,
            9,
            Err((ErrorKind::ResourceLimit, "operand stack limit".into())),
        ),
        (4, 18, Ok(RunOutcome::Returned(vec![Value::I32(1304)]))),
    ] {
        let mut observations = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut config = Config::default();
            config.limits.max_stack_height = limit;
            config.limits.fuel = Some(fuel);
            let (mut store, function) = loaded(
                policy,
                config,
                include_bytes!("../fixtures/performance/p060/table_caller.wasm"),
            );
            let actual = view(store.invoke(function, &[Value::I32(0)]));
            assert_eq!(actual, expected);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_fallback_frames(), 0);
                assert_eq!(store.register_exits(), 1);
                // Three caller source events now execute before the call exit.
                assert_eq!(store.register_dispatches(), if limit == 3 { 7 } else { 14 });
            }
            observations.push(actual);
        }
        assert_eq!(observations[0], observations[1]);
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let (mut store, function) = loaded(policy, Config::default(), PAIR);
        store.cancellation_token().cancel();
        assert_eq!(
            view(store.invoke(function, &[Value::I32(0)])),
            Err((ErrorKind::ResourceLimit, "execution cancelled".into()))
        );
        assert_eq!(store.register_dispatches(), 0);
    }
    // Protected caller still uses decoded calls; the numeric helper executes
    // the existing register path without losing the caller's logical stack.
    let bytes = include_bytes!("../fixtures/performance/p060/multivalue_caller.wasm");
    let mut observations = vec![];
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let (mut store, function) = loaded(policy, Config::default(), bytes);
        observations.push(view(store.invoke(function, &[Value::I32(17)])));
        if policy == ExecutionPolicy::Register {
            assert!(store.register_dispatches() > 0);
            assert_eq!(store.register_fallback_frames(), 0);
            assert_eq!(store.register_exits(), 1);
        }
    }
    assert_eq!(observations[0], observations[1]);
}
