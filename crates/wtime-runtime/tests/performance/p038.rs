//! Unexecuted root test draft; requires freshly encoded, validated fixtures.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;

const SCALAR: &[u8] = include_bytes!("../fixtures/performance/p038/scalar32.wasm");
fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8]) -> (Store, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else { panic!("missing run") };
    (store, function)
}
fn view(outcome: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    outcome.map_err(|error| (error.kind, error.message))
}

#[test]
fn p038_integer_boundaries_locals_select_drop_execute_direct_records() {
    let (mut store, function) = loaded(ExecutionPolicy::Compact, Config::default(), SCALAR);
    let mut inputs = vec![i32::MIN, i32::MAX, -1, 0, 1, 0x4000_0000];
    let mut seed = 0x1234_5678u32;
    for _ in 0..256 {
        seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
        inputs.push(seed as i32);
    }
    for input in inputs {
        for condition in [0, 1, -1] {
            let expected = if condition == 0 { 7 } else { input.wrapping_add(i32::MAX) ^ 1 };
            let before = store.compact_dispatches();
            assert_eq!(store.invoke(function, &[Value::I32(input), Value::I32(condition)]).unwrap(),
                       RunOutcome::Returned(vec![Value::I32(expected)]));
            assert_eq!(store.compact_dispatches() - before, 14);
        }
    }
}

#[test]
fn p038_i64_wrapping_and_early_return_have_exact_executed_source_counts() {
    for (bytes, expected, events) in [
        (include_bytes!("../fixtures/performance/p038/scalar64.wasm").as_slice(), Value::I64(i64::MIN), 6),
        (include_bytes!("../fixtures/performance/p038/explicit-return.wasm").as_slice(), Value::I32(42), 2),
    ] {
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
            let (mut store, function) = loaded(policy, Config::default(), bytes);
            assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![expected]));
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { events } else { 0 });
            if policy == ExecutionPolicy::Reference { assert_eq!(store.reference_dispatches(), events); }
        }
    }
}

#[test]
fn p038_every_fuel_prefix_and_transient_stack_failure_match_decoded_and_reference() {
    for fuel in 0..=15 {
        let mut observed = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
            let mut config = Config::default(); config.limits.fuel = Some(fuel);
            let (mut store, function) = loaded(policy, config, SCALAR);
            observed.push(view(store.invoke(function, &[Value::I32(i32::MIN), Value::I32(1)])));
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { fuel.min(14) } else { 0 });
        }
        assert_eq!(observed[0], observed[1], "reference fuel {fuel}");
        assert_eq!(observed[0], observed[2], "compact fuel {fuel}");
        if fuel < 14 { assert_eq!(observed[0], Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))); }
        else { assert_eq!(observed[0], Ok(RunOutcome::Returned(vec![Value::I32(-2)]))); }
    }
    // Each function validates at height two. The caller retains one
    // value while its leaf creates shared operand height three; both compact
    // bodies execute two instructions before the shared boundary refuses work.
    let bytes = include_bytes!("../fixtures/performance/accounting/transient_stack.wasm");
    for (fuel, message) in [(4, "fuel exhausted"), (5, "operand stack limit")] {
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
            let mut config = Config::default(); config.limits.fuel = Some(fuel); config.limits.max_stack_height = 2;
            let (mut store, function) = loaded(policy, config, bytes);
            assert_eq!(view(store.invoke(function, &[])), Err((ErrorKind::ResourceLimit, message.into())));
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { 4 } else { 0 });
        }
    }
    let mut config = Config::default();
    config.limits.fuel = Some(6);
    config.limits.max_stack_height = 3;
    let (mut store, function) = loaded(ExecutionPolicy::Compact, config,
        include_bytes!("../fixtures/performance/p038/transient-stack.wasm"));
    assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(1)]));
    assert_eq!(store.compact_dispatches(), 6);
}

#[test]
fn p038_materialization_preserves_every_reachable_next_source_cursor_and_fuel() {
    for pc in 0..14 {
        let mut observations = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let mut config = Config::default(); config.limits.fuel = Some(14);
            let (mut store, function) = loaded(policy, config, SCALAR);
            store.request_materialization_roundtrip(function, pc).unwrap();
            assert_eq!(store.invoke(function, &[Value::I32(i32::MIN), Value::I32(1)]).unwrap(),
                       RunOutcome::Returned(vec![Value::I32(-2)]));
            let snapshot = store.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc, pc);
            assert_eq!(snapshot.cursor.prepared_pc, pc);
            assert_eq!(snapshot.remaining_fuel, Some(13 - pc as u64));
            assert!(snapshot.pending_boundary_charged);
            assert_eq!(store.materialization_roundtrips(), 1);
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { 14 } else { 0 });
            observations.push(snapshot);
        }
        assert_eq!(observations[0], observations[1], "source pc {pc}");
    }
}

#[test]
fn p038_reference_select_and_wide_locals_visibly_keep_decoded_fallback() {
    let bytes = include_bytes!("../fixtures/performance/p038/reference-select.wasm");
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
        let (mut store, function) = loaded(policy, Config::default(), bytes);
        let first = store.create_extern_ref(101).unwrap();
        let second = store.create_extern_ref(202).unwrap();
        assert_ne!(first, second);
        for condition in [0, 1, -1] {
            assert_eq!(store.invoke(function, &[first, second, Value::I32(condition)]).unwrap(),
                       RunOutcome::Returned(vec![if condition == 0 { second } else { first }]));
        }
        assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { 9 } else { 0 });
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_prefix_counts(), (9, [0,0,0,3], Some((3, 0x1c))));
        }
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let mut config = Config::default(); config.limits.max_locals = 70_001;
        let (mut store, function) = loaded(policy, config, include_bytes!("../fixtures/performance/plans/wide_local.wasm"));
        assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(42)]));
        assert_eq!(store.compact_dispatches(), 0);
    }
}

#[test]
fn p038_module_lifetime_and_pre_cancel_do_not_weaken_entry_contracts() {
    let engine = Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap();
    let module = engine.compile(include_bytes!("../fixtures/performance/p038/explicit-return.wasm")).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else { panic!() };
    drop(module); drop(engine);
    assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(42)]));
    assert_eq!(store.compact_dispatches(), 2);
    store.cancellation_token().cancel();
    assert_eq!(view(store.invoke(function, &[])), Err((ErrorKind::ResourceLimit, "execution cancelled".into())));
    assert_eq!(store.compact_dispatches(), 2);
}
