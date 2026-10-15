#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;
include!("p138_trace.rs");

const UNIFORM: &[u8] = include_bytes!("../fixtures/performance/p138/uniform66.wasm");

fn p138_engine(threshold: Option<u64>, eager: bool, fuel: Option<u64>) -> Engine {
    let mut config = Config {
        features: FeatureSet::mvp(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    if let Some(threshold) = threshold {
        Engine::with_register_entry_threshold(config, threshold).unwrap()
    } else if eager {
        Engine::with_execution_policy(config, ExecutionPolicy::Register).unwrap()
    } else {
        Engine::new(config).unwrap()
    }
}

fn p138_call(store: &mut Store, instance: Instance, body: usize, input: i32) {
    let External::Func(function) = instance.export(store, &format!("f{body:02}")).unwrap() else {
        panic!()
    };
    assert_eq!(
        store.invoke(function, &[Value::I32(input)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(input.wrapping_add(body as i32))])
    );
}

#[test]
fn p138_cold_threshold_and_single_builder_are_observed() {
    let case = "p138_cold_threshold_and_single_builder_are_observed";
    for eager in [false, true] {
        let engine = p138_engine(None, eager, None);
        assert!(
            engine
                .compile(UNIFORM)
                .unwrap()
                .register_cache_snapshot()
                .is_none()
        );
    }
    let engine = p138_engine(Some(3), false, None);
    let module = engine.compile(UNIFORM).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let initial = module.register_cache_snapshot().unwrap();
    assert!(initial.entries.is_empty());
    assert_eq!(initial.lifecycle, RegisterCacheLifecycle::default());
    p138_emit(case, "initial", &initial);
    for call in 1..=5 {
        let before = module.register_cache_snapshot().unwrap();
        assert_eq!(before, module.register_cache_snapshot().unwrap());
        let steps = store.register_dispatches();
        p138_call(&mut store, instance, 0, i32::MAX);
        let after = module.register_cache_snapshot().unwrap();
        assert_eq!(after.entries.len(), 1);
        let row = after.entries[0];
        assert_eq!((row.body, row.entries), (0, call));
        assert_eq!(row.attempts, u8::from(call >= 3));
        assert_eq!(after.lifecycle.builder_starts, u64::from(call >= 3));
        assert_eq!(after.lifecycle.published, u64::from(call >= 3));
        assert_eq!(
            store.register_dispatches() - steps,
            if call >= 3 { 4 } else { 0 }
        );
        if call < 3 {
            assert_eq!(row.state, RegisterCacheEntryState::Cold);
        } else {
            assert!(
                matches!(row.state, RegisterCacheEntryState::Ready { retained_bytes } if retained_bytes > 0)
            );
        }
        p138_emit(case, &format!("call-{call}"), &after);
    }
    assert_eq!(store.register_fallback_frames(), 2);
    eprintln!(
        "P138_CASE {{\"name\":\"{case}\",\"passed\":true,\"calls\":5,\"cold_calls\":2,\"builders\":1,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}

#[test]
fn p138_guest_fifo_eviction_and_recompilation_stay_bounded() {
    let case = "p138_guest_fifo_eviction_and_recompilation_stay_bounded";
    let engine = p138_engine(Some(1), false, None);
    let module = engine.compile(UNIFORM).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let mut previous = module.register_cache_snapshot().unwrap();
    p138_emit(case, "initial", &previous);
    for (index, body) in (0..66).chain(std::iter::once(0)).enumerate() {
        p138_call(&mut store, instance, body, 7);
        let after = module.register_cache_snapshot().unwrap();
        assert_eq!(after.active_body, None);
        assert_eq!(after.lifecycle.preparation_starts, (index + 1) as u64);
        assert_eq!(after.lifecycle.builder_starts, (index + 1) as u64);
        assert_eq!(after.lifecycle.published, (index + 1) as u64);
        assert_eq!(
            after.lifecycle.evictions,
            (index + 1).saturating_sub(64) as u64
        );
        assert_eq!(after.entries.len(), (index + 1).min(64));
        assert!(
            after
                .entries
                .iter()
                .all(|entry| matches!(entry.state, RegisterCacheEntryState::Ready { .. }))
        );
        assert_eq!(after.entries.last().unwrap().body, body);
        assert!(after.remaining_attempted_work <= previous.remaining_attempted_work);
        assert!(after.remaining_retained_work <= previous.remaining_retained_work);
        assert!(after.retained_bytes <= after.byte_limit);
        p138_emit(case, &format!("call-{}-body-{body}", index + 1), &after);
        previous = after;
    }
    assert_eq!(
        previous
            .entries
            .iter()
            .map(|entry| entry.body)
            .collect::<Vec<_>>(),
        (3..66).chain(std::iter::once(0)).collect::<Vec<_>>()
    );
    assert_eq!(store.register_dispatches(), 67 * 4);
    assert_eq!(store.register_fallback_frames(), 0);
    eprintln!(
        "P138_CASE {{\"name\":\"{case}\",\"passed\":true,\"calls\":67,\"distinct_bodies\":66,\"builders\":67,\"evictions\":3,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}

#[test]
fn p138_snapshot_preserves_all_fuel_prefixes_and_wraparound_results() {
    for fuel in 0..=4 {
        let mut results = Vec::new();
        for (threshold, eager) in [
            (None, false),
            (None, true),
            (Some(1), false),
            (Some(3), false),
        ] {
            let engine = p138_engine(threshold, eager, Some(fuel));
            let module = engine.compile(UNIFORM).unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let External::Func(function) = instance.export(&store, "f65").unwrap() else {
                panic!()
            };
            store
                .request_materialization_roundtrip(function, 3)
                .unwrap();
            assert!(store.execution_snapshot().is_none());
            let _snapshot = module.register_cache_snapshot();
            assert!(store.execution_snapshot().is_none());
            let result = store
                .invoke(function, &[Value::I32(i32::MAX)])
                .map_err(|error| (error.kind, error.message));
            let observed = store
                .execution_snapshot()
                .map(|snapshot| (snapshot.cursor.source_pc, snapshot.remaining_fuel));
            let _snapshot = module.register_cache_snapshot();
            assert_eq!(
                store
                    .execution_snapshot()
                    .map(|snapshot| (snapshot.cursor.source_pc, snapshot.remaining_fuel)),
                observed
            );
            results.push((result, observed));
        }
        assert!(results.iter().all(|result| result == &results[0]));
        if fuel == 4 {
            assert_eq!(
                results[0].0,
                Ok(RunOutcome::Returned(vec![Value::I32(
                    i32::MAX.wrapping_add(65)
                )]))
            );
            assert_eq!(results[0].1, Some((3, Some(0))));
        }
    }
    eprintln!(
        "P138_CASE {{\"name\":\"p138_snapshot_preserves_all_fuel_prefixes_and_wraparound_results\",\"passed\":true,\"fuel_prefixes\":5,\"policies\":4,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}
