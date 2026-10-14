#![cfg(feature = "execution-reference")]
use wtime_runtime::*;
const LOOP: &[u8] = include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm");
fn loaded(fuel: Option<u64>) -> (Engine, Module, Store, Func) {
    let mut config = Config::default();
    config.limits.fuel = fuel;
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(LOOP).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    (engine, module, store, function)
}
#[test]
fn p129_zero_skip_preserves_first_visit() {
    let mut snapshots = vec![];
    for old_api in [true, false] {
        let (_, _, mut store, function) = loaded(Some(9));
        if old_api {
            store
                .request_materialization_roundtrip(function, 8)
                .unwrap();
        } else {
            store
                .request_materialization_roundtrip_after_visits(function, 8, 0)
                .unwrap();
        }
        assert_eq!(
            store.invoke(function, &[]).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        let snapshot = store.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc, 8);
        assert_eq!(
            snapshot.frames[0].locals,
            vec![
                DiagnosticValue::I32(0),
                DiagnosticValue::I32(0),
                DiagnosticValue::I32(8)
            ]
        );
        assert_eq!(snapshot.remaining_fuel, Some(0));
        assert!(snapshot.pending_boundary_charged && !snapshot.truncated);
        assert_eq!(store.materialization_roundtrips(), 1);
        snapshots.push(snapshot);
    }
    assert_eq!(snapshots[0], snapshots[1]);
}
#[test]
fn p129_later_loop_visit_keeps_exact_state_and_fuel() {
    let (_, _, mut store, function) = loaded(Some(35));
    store
        .request_materialization_roundtrip_after_visits(function, 8, 2)
        .unwrap();
    assert_eq!(
        store.invoke(function, &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    let snapshot = store.execution_snapshot().unwrap();
    assert_eq!(
        (snapshot.cursor.source_pc, snapshot.cursor.prepared_pc),
        (8, 8)
    );
    assert_eq!(
        snapshot.frames[0].locals,
        vec![
            DiagnosticValue::I32(2),
            DiagnosticValue::I32(16),
            DiagnosticValue::I32(8)
        ]
    );
    assert_eq!(snapshot.operand_count, 0);
    assert!(snapshot.operands.is_empty());
    assert_eq!(snapshot.remaining_fuel, Some(0));
    assert!(snapshot.pending_boundary_charged && !snapshot.truncated);
    assert_eq!(store.materialization_roundtrips(), 1);
    eprintln!("P129: third source visit; 35 charged events; 1 roundtrip; remaining fuel 0");
}
#[test]
fn p129_invalid_visit_bound_and_unreached_request_are_bounded() {
    let (engine, module, mut store, function) = loaded(None);
    assert_eq!(
        store
            .request_materialization_roundtrip_after_visits(function, 8, 100_001)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        store
            .request_materialization_roundtrip_after_visits(function, usize::MAX, 0)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    assert_eq!(
        store.invoke(function, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(32)])
    );
    assert!(store.execution_snapshot().is_none());
    assert_eq!(store.materialization_roundtrips(), 0);
    store
        .request_materialization_roundtrip_after_visits(function, 8, 5)
        .unwrap();
    assert_eq!(
        store.invoke(function, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(32)])
    );
    assert!(store.execution_snapshot().is_none());
    assert_eq!(store.materialization_roundtrips(), 0);
    // Identical PCs in another instance cannot consume the selector.
    store
        .request_materialization_roundtrip_after_visits(function, 8, 1)
        .unwrap();
    let other = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(other_function) = other.export(&store, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        store.invoke(other_function, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(32)])
    );
    assert!(store.execution_snapshot().is_none());
    assert_eq!(
        store.invoke(function, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(32)])
    );
    assert_eq!(
        store.execution_snapshot().unwrap().frames[0].locals,
        vec![
            DiagnosticValue::I32(1),
            DiagnosticValue::I32(8),
            DiagnosticValue::I32(8)
        ]
    );
    assert_eq!(store.materialization_roundtrips(), 1);
}
