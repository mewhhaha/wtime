use wtime_runtime::*;

#[test]
fn p037_public_metadata_keeps_default_and_portable_decoded() {
    assert_eq!(Engine::new(Config::default()).unwrap().execution_policy(), ExecutionPolicy::Decoded);
    assert_eq!(Engine::with_execution_policy(Config::default(), ExecutionPolicy::Portable).unwrap().execution_policy(), ExecutionPolicy::Decoded);
}

#[cfg(feature = "execution-reference")]
fn roundtrip(bytes: &[u8], pc: usize) -> (Store, RunOutcome) {
    let mut config = Config::default();
    config.limits.fuel = Some(20);
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else { panic!("missing run") };
    store.request_materialization_roundtrip(function, pc).unwrap();
    let outcome = store.invoke(function, &[]).unwrap();
    (store, outcome)
}

#[cfg(feature = "execution-reference")]
#[test]
fn p037_public_decoded_store_successor_has_exact_charged_cursor() {
    let (store, outcome) = roundtrip(include_bytes!("../fixtures/performance/cursor/store_successor.wasm"), 3);
    assert_eq!(outcome, RunOutcome::Returned(vec![Value::I32(7)]));
    let snapshot = store.execution_snapshot().unwrap();
    assert_eq!(snapshot.cursor.source_pc, 3);
    assert_eq!(snapshot.cursor.prepared_pc, 3);
    assert_eq!(snapshot.memory_prefixes[0][..4], [7, 0, 0, 0]);
    assert_eq!(snapshot.remaining_fuel, Some(16));
    assert!(snapshot.pending_boundary_charged);
    assert_eq!(store.materialization_roundtrips(), 1);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p037_public_decoded_branch_handoff_keeps_source_order_and_labels() {
    let (store, outcome) = roundtrip(include_bytes!("../fixtures/performance/cursor/multivalue_branch.wasm"), 3);
    assert_eq!(outcome, RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)]));
    let snapshot = store.execution_snapshot().unwrap();
    assert_eq!(snapshot.cursor.source_pc, 3);
    assert_eq!(snapshot.cursor.prepared_pc, 3);
    assert_eq!(snapshot.operands, [DiagnosticValue::I32(7), DiagnosticValue::I64(9)]);
    assert_eq!(snapshot.frames[0].label_count, 1);
    assert_eq!(snapshot.frames[0].labels[0].4, 2);
    assert!(snapshot.pending_boundary_charged);
    assert_eq!(store.materialization_roundtrips(), 1);
}
