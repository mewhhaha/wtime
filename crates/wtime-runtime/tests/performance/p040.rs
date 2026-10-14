//! Independent source paths and results for checked integer control execution.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;

fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8], export: &str) -> (Store, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, export).unwrap() else { panic!() };
    (store, function)
}
fn view(outcome: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    outcome.map_err(|error| (error.kind, error.message))
}
fn config(fuel: Option<u64>) -> Config {
    let mut result = Config::default(); result.limits.fuel = fuel; result
}
fn section(bytes: &mut Vec<u8>, id: u8, contents: &[u8]) {
    assert!(contents.len() < 128); bytes.push(id); bytes.push(contents.len() as u8); bytes.extend(contents);
}
fn original(types: &[u8], funcs: &[u8], export_index: u8, bodies: &[&[u8]]) -> Vec<u8> {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 1, types); section(&mut bytes, 3, funcs);
    section(&mut bytes, 7, &[1, 3, b'r', b'u', b'n', 0, export_index]);
    let mut code = vec![bodies.len() as u8];
    for body in bodies { assert!(body.len() < 128); code.push(body.len() as u8); code.extend(*body); }
    section(&mut bytes, 10, &code); bytes
}

#[test]
fn p040_multivalue_loop_and_nested_if_match_each_source_fuel_prefix() {
    let loop_bytes = include_bytes!("../fixtures/performance/p040/multivalue_loop.wasm").as_slice();
    let nested = include_bytes!("../fixtures/performance/p040/nested_if_results.wasm").as_slice();
    for (bytes, args, expected, events) in [
        (loop_bytes, vec![], vec![Value::I32(2), Value::I64(40)], 27),
        (nested, vec![Value::I32(0), Value::I32(0)], vec![Value::I32(-1), Value::I64(-2)], 6),
        (nested, vec![Value::I32(1), Value::I32(0)], vec![Value::I32(42), Value::I64(421)], 10),
        (nested, vec![Value::I32(1), Value::I32(1)], vec![Value::I32(41), Value::I64(420)], 11),
    ] {
        for fuel in 0..=events + 1 {
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
                let (mut store, function) = loaded(policy, config(Some(fuel)), bytes, "run");
                let actual = view(store.invoke(function, &args));
                let oracle = if fuel < events { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                    else { Ok(RunOutcome::Returned(expected.clone())) };
                assert_eq!(actual, oracle, "{policy:?} fuel {fuel}");
                assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { fuel.min(events) } else { 0 });
            }
        }
    }
}

#[test]
fn p040_br_table_targets_and_early_returns_have_independent_prefixes() {
    let table = include_bytes!("../fixtures/performance/p040/br_table_three_paths.wasm").as_slice();
    let branch = include_bytes!("../fixtures/performance/p040/br_if_taken_untaken.wasm").as_slice();
    let returns = include_bytes!("../fixtures/performance/p040/function_branch_vs_return.wasm").as_slice();
    for (bytes, export, input, expected, events) in [
        (table, "run", 0, 1107, 11), (table, "run", 1, 107, 8),
        (table, "run", 2, 7, 5), (table, "run", -1, 7, 5),
        (branch, "run", 0, 7, 8), (branch, "run", 1, 42, 4),
        (returns, "branch", 0, 7, 8), (returns, "branch", 1, 42, 4),
        (returns, "return", 0, 7, 5), (returns, "return", 1, 42, 4),
    ] {
        for fuel in 0..=events + 1 {
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
                let (mut store, function) = loaded(policy, config(Some(fuel)), bytes, export);
                assert_eq!(view(store.invoke(function, &[Value::I32(input)])),
                    if fuel < events { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                    else { Ok(RunOutcome::Returned(vec![Value::I32(expected)])) },
                    "{export} input {input} {policy:?} fuel {fuel}");
                assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { fuel.min(events) } else { 0 });
            }
        }
    }
}

#[test]
fn p040_taken_fallthrough_and_structural_cursors_keep_exact_labels_and_fuel() {
    let table = include_bytes!("../fixtures/performance/p040/br_table_three_paths.wasm").as_slice();
    let nested = include_bytes!("../fixtures/performance/p040/nested_if_results.wasm").as_slice();
    for (bytes, args, expected, path) in [
        (table, vec![Value::I32(0)], vec![Value::I32(1107)], vec![0,1,2,3,4,6,7,8,9,10,11]),
        (table, vec![Value::I32(1)], vec![Value::I32(107)], vec![0,1,2,3,4,9,10,11]),
        (table, vec![Value::I32(2)], vec![Value::I32(7)], vec![0,1,2,3,4]),
        (nested, vec![Value::I32(0),Value::I32(0)], vec![Value::I32(-1),Value::I64(-2)], vec![0,1,11,12,13,14]),
        (nested, vec![Value::I32(1),Value::I32(0)], vec![Value::I32(42),Value::I64(421)], vec![0,1,2,3,7,8,9,10,13,14]),
        (nested, vec![Value::I32(1),Value::I32(1)], vec![Value::I32(41),Value::I64(420)], vec![0,1,2,3,4,5,6,9,10,13,14]),
    ] {
        for (event, pc) in path.iter().copied().enumerate() {
            let mut observations = Vec::new();
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
                let (mut store, function) = loaded(policy, config(Some(path.len() as u64)), bytes, "run");
                store.request_materialization_roundtrip(function, pc).unwrap();
                assert_eq!(store.invoke(function, &args).unwrap(), RunOutcome::Returned(expected.clone()));
                let snapshot = store.execution_snapshot().unwrap().clone();
                assert_eq!(snapshot.cursor.source_pc, pc);
                assert_eq!(snapshot.cursor.prepared_pc, pc);
                assert_eq!(snapshot.remaining_fuel, Some((path.len() - event - 1) as u64));
                assert!(snapshot.pending_boundary_charged);
                assert_eq!(store.materialization_roundtrips(), 1);
                assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { path.len() as u64 } else { 0 });
                observations.push(snapshot);
            }
            assert_eq!(observations[0], observations[1], "pc {pc}");
        }
    }
}

#[test]
fn p040_unreachable_keeps_completed_local_prefix_and_charged_trap_cursor() {
    let bytes = include_bytes!("../fixtures/performance/p040/unreachable_local_prefix.wasm");
    for fuel in 0..=4 {
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
            let (mut store, function) = loaded(policy, config(Some(fuel)), bytes, "run");
            assert_eq!(view(store.invoke(function, &[])),
                if fuel < 3 { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                else { Err((ErrorKind::Trap, "unreachable".into())) });
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { fuel.min(3) } else { 0 });
        }
    }
    let mut observations = Vec::new();
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let (mut store, function) = loaded(policy, config(Some(3)), bytes, "run");
        store.request_materialization_roundtrip(function, 2).unwrap();
        assert_eq!(view(store.invoke(function, &[])), Err((ErrorKind::Trap, "unreachable".into())));
        let snapshot = store.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc, 2); assert_eq!(snapshot.remaining_fuel, Some(0));
        assert_eq!(snapshot.frames[0].locals, vec![DiagnosticValue::I32(9)]);
        assert!(snapshot.pending_boundary_charged); observations.push(snapshot);
    }
    assert_eq!(observations[0], observations[1]);
}

#[test]
fn p040_zero_result_return_and_global_stack_peak_preserve_trap_priority() {
    // Independently authored binaries are validated by Engine::compile before use.
    let zero = original(&[1,0x60,0,0], &[1,0], 0, &[&[0,2,0x40,0x0f,0x0b,0x0b]]);
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
        let (mut store, function) = loaded(policy, config(Some(2)), &zero, "run");
        assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![]));
        assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { 2 } else { 0 });
    }
    let peak = original(&[2,0x60,0,1,0x7f,0x60,1,0x7f,1,0x7f], &[2,0,0], 1,
        &[&[0,0x41,1,2,1,0x41,2,0x6a,0x0b,0x0b], &[0,0x41,7,0x41,8,0x10,0,0x6a,0x6a,0x0b]]);
    for (fuel, message) in [(6, "fuel exhausted"), (7, "operand stack limit")] {
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
            let mut config = config(Some(fuel)); config.limits.max_stack_height = 3;
            let (mut store, function) = loaded(policy, config, &peak, "run");
            assert_eq!(view(store.invoke(function, &[])), Err((ErrorKind::ResourceLimit, message.into())));
            assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { 6 } else { 0 });
        }
    }
}

#[test]
fn p040_reference_valued_control_declines_whole_function_without_changing_identity() {
    let bytes = original(&[1,0x60,1,0x6f,1,0x6f], &[1,0], 0,
        &[&[0,0x41,1,4,0x6f,0x20,0,5,0x20,0,0x0b,0x0b]]);
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact] {
        let (mut store, function) = loaded(policy, Config::default(), &bytes, "run");
        let reference = store.create_extern_ref(709).unwrap();
        assert_eq!(store.invoke(function, &[reference]).unwrap(), RunOutcome::Returned(vec![reference]));
        assert_eq!(store.compact_dispatches(), u64::from(policy == ExecutionPolicy::Compact));
        if policy == ExecutionPolicy::Compact {
            assert_eq!(store.compact_prefix_counts(), (1, [0,0,0,1], Some((1,4))));
        }
    }
}
