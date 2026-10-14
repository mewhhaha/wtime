#![cfg(feature = "execution-reference")]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wtime_runtime::*;
fn engine() -> Engine {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.fuel = Some(20);
    Engine::new(c).unwrap()
}
fn run(s: &mut Store, i: Instance) -> RunOutcome {
    let External::Func(f) = i.export(s, "run").unwrap() else {
        panic!()
    };
    s.invoke(f, &[]).unwrap()
}
fn request(s: &mut Store, i: Instance, pc: usize) {
    let External::Func(f) = i.export(s, "run").unwrap() else {
        panic!()
    };
    s.request_materialization_roundtrip(f, pc).unwrap();
}
#[test]
fn p018_store_successor_preserves_memory_and_fuel() {
    let e = engine();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/store_successor.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    request(&mut s, i, 3);
    assert_eq!(run(&mut s, i), RunOutcome::Returned(vec![Value::I32(7)]));
    let snapshot = s.execution_snapshot().unwrap();
    assert_eq!(snapshot.cursor.source_pc, 3);
    assert_eq!(snapshot.cursor.prepared_pc, 3);
    assert_eq!(snapshot.memory_prefixes[0][..4], [7, 0, 0, 0]);
    assert_eq!(snapshot.remaining_fuel, Some(16));
    assert!(snapshot.pending_boundary_charged);
    assert_eq!(s.materialization_roundtrips(), 1);
}
#[test]
fn p018_multivalue_branch_restores_order_and_label() {
    let e = engine();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/multivalue_branch.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    request(&mut s, i, 3);
    assert_eq!(
        run(&mut s, i),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );
    let snapshot = s.execution_snapshot().unwrap();
    assert_eq!(
        snapshot.operands,
        [DiagnosticValue::I32(7), DiagnosticValue::I64(9)]
    );
    assert_eq!(snapshot.frames[0].label_count, 1);
    assert_eq!(snapshot.frames[0].labels[0].4, 2);
    assert_eq!(s.materialization_roundtrips(), 1);
}
#[test]
fn p018_restore_never_replays_completed_host_call() {
    let e = engine();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/host_prefix.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = ticks.clone();
    let f = s
        .create_host_func(FuncType::new(vec![], vec![]), move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        })
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("host", "tick", External::Func(f)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    request(&mut s, i, 1);
    assert_eq!(run(&mut s, i), RunOutcome::Returned(vec![Value::I32(9)]));
    assert_eq!(ticks.load(Ordering::SeqCst), 1);
    assert_eq!(s.execution_snapshot().unwrap().remaining_fuel, Some(18));
    assert_eq!(s.materialization_roundtrips(), 1);
}
#[test]
fn p018_suspend_resume_preserves_saved_fuel_and_context() {
    let e = engine();
    let m = e
        .compile(include_bytes!("../../examples/suspension.wasm"))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    drop(m);
    let RunOutcome::Suspended { continuation, .. } = run(&mut s, i) else {
        panic!()
    };
    s.collect().unwrap();
    request(&mut s, i, 2);
    let External::Func(f) = i.export(&s, "resume").unwrap() else {
        panic!()
    };
    assert_eq!(
        s.invoke(f, &[Value::Ref(RefValue::Gc(continuation))])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(9)])
    );
    let snapshot = s.execution_snapshot().unwrap();
    assert_eq!(snapshot.cursor.function_body, 0);
    assert_eq!(snapshot.cursor.source_pc, 2);
    assert_eq!(snapshot.remaining_fuel, Some(16));
    assert_eq!(s.materialization_roundtrips(), 1);
}
#[test]
fn p018_request_distinguishes_cross_module_body_indices() {
    let e = engine();
    let a = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/cross_callee.wasm"
        ))
        .unwrap();
    let b = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/cross_caller.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let callee = Linker::new(&e).instantiate(&mut s, &a).unwrap();
    let External::Func(step) = callee.export(&s, "step").unwrap() else {
        panic!()
    };
    let mut l = Linker::new(&e);
    l.define("other", "step", External::Func(step)).unwrap();
    let caller = l.instantiate(&mut s, &b).unwrap();
    request(&mut s, caller, 1);
    assert_eq!(
        run(&mut s, caller),
        RunOutcome::Returned(vec![Value::I32(12)])
    );
    let snapshot = s.execution_snapshot().unwrap();
    assert_eq!(snapshot.frame_count, 1);
    assert_eq!(snapshot.operands, [DiagnosticValue::I32(9)]);
    assert_eq!(snapshot.cursor.instance, 1);
}
#[test]
fn p018_invalid_or_foreign_cursor_request_is_rejected() {
    let e = engine();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/store_successor.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    let External::Func(f) = i.export(&s, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        s.request_materialization_roundtrip(f, usize::MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    let mut foreign = Store::new(&e);
    assert_eq!(
        foreign
            .request_materialization_roundtrip(f, 0)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    assert_eq!(s.materialization_roundtrips(), 0);
    assert!(s.execution_snapshot().is_none());
}
#[test]
fn p018_restored_nominal_local_remains_rooted_during_host_collection() {
    let e = engine();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/cursor/root_after_restore.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let f = s
        .create_host_func_with_caller(FuncType::new(vec![], vec![]), |caller, _| {
            caller.collect()?;
            Ok(vec![])
        })
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("host", "collect", External::Func(f)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    drop(m);
    request(&mut s, i, 3);
    assert_eq!(run(&mut s, i), RunOutcome::Returned(vec![Value::I32(42)]));
    assert_eq!(
        s.execution_snapshot().unwrap().frames[0].locals,
        [DiagnosticValue::Reference {
            category: "struct",
            identity: Some(0)
        }]
    );
    assert_eq!(s.materialization_roundtrips(), 1);
}
