//! Sealed, development-only publication telemetry; no production dependency.
#![cfg(all(feature = "compact-executor", feature = "performance-counters"))]
use wtime_runtime::*;
fn engine(policy: ExecutionPolicy) -> Engine {
    Engine::with_execution_policy(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        policy,
    )
    .unwrap()
}
#[test]
fn p121_publication_counts_are_immutable_and_capacity_bounded() {
    let engine = engine(ExecutionPolicy::Compact);
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/bench_wasm/integration_p113/integration_numeric_control.wasm"
        ))
        .unwrap();
    let counts = module.compact_preparation_counts().unwrap();
    assert_eq!((counts.0, counts.1), (1, 0));
    assert!(counts.2 > 0 && counts.2 <= 100_000);
    assert!(counts.3 > 0 && counts.3 <= 1024 * 1024);
    let copy = module.clone();
    assert_eq!(copy.compact_preparation_counts(), Some(counts));
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(func) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        store.invoke(func, &[Value::I32(32)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(64)])
    );
    assert_eq!(module.compact_preparation_counts(), Some(counts));
    let counters = store.performance_counters();
    assert!(counters.compact_steps > 0);
    assert_eq!(counters.source_dispatches, counters.compact_steps);
}
#[test]
fn p121_prefix_execution_reports_real_optimized_and_fallback_events() {
    let engine = engine(ExecutionPolicy::Compact);
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/bench_wasm/integration_p113/integration_gc_prefix.wasm"
        ))
        .unwrap();
    let counts = module.compact_preparation_counts().unwrap();
    assert_eq!((counts.0, counts.1), (1, 0));
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(func) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        store.invoke(func, &[Value::I32(32)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(64)])
    );
    let counters = store.performance_counters();
    assert_eq!(counters.compact_steps, 67);
    assert!(
        counters
            .source_dispatches
            .checked_sub(counters.compact_steps)
            .unwrap()
            > 0
    );
    assert_eq!(module.compact_preparation_counts(), Some(counts));
}
#[test]
fn p121_absent_or_budget_declined_artifact_has_unknown_discarded_work() {
    let bytes = include_bytes!(
        "../fixtures/performance/bench_wasm/integration_p113/integration_numeric_control.wasm"
    );
    assert_eq!(
        engine(ExecutionPolicy::Decoded)
            .compile(bytes)
            .unwrap()
            .compact_preparation_counts(),
        None
    );
    let engine = engine(ExecutionPolicy::Compact);
    let entry = include_bytes!(
        "../fixtures/performance/bench_wasm/integration_p113/integration_ref_entry_control.wasm"
    );
    assert_eq!(
        engine.compile(entry).unwrap().compact_preparation_counts(),
        None
    );
    let long = include_bytes!("../fixtures/performance/compact/long_metadata.wasm");
    let module = engine.compile(long).unwrap();
    assert_eq!(module.compact_preparation_counts(), None);
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(func) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        store.invoke(func, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7)])
    );
    let counters = store.performance_counters();
    assert_eq!(counters.compact_steps, 0);
    assert!(counters.source_dispatches > 0);
}
