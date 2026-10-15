#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(func) = instance.export(store, name).unwrap() else {
        panic!()
    };
    func
}

fn inactive(snapshot: RegisterActivationSnapshot) {
    assert_eq!(
        (
            snapshot.current_bytes,
            snapshot.current_frames,
            snapshot.active_scopes
        ),
        (0, 0, 0)
    );
    assert!(!snapshot.overflowed);
}

#[test]
fn p144_eager_and_threshold_three_report_capacity_high_water_per_invoke() {
    let mut all_peaks = Vec::new();
    for adaptive in [false, true] {
        let engine = if adaptive {
            Engine::with_register_entry_threshold(Config::default(), 3).unwrap()
        } else {
            Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap()
        };
        let module = engine
            .compile(include_bytes!(
                "../fixtures/performance/p138/uniform66.wasm"
            ))
            .unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        let run = function(&store, instance, "f00");
        let mut peaks = Vec::new();
        for entry in 1..=4 {
            store.reset_register_activation_snapshot().unwrap();
            assert_eq!(
                store.register_activation_snapshot(),
                RegisterActivationSnapshot::default()
            );
            let before = store.register_dispatches();
            assert_eq!(
                store.invoke(run, &[Value::I32(7)]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(7)])
            );
            let snapshot = store.register_activation_snapshot();
            inactive(snapshot);
            assert!(snapshot.capacity_samples > 0);
            let hot = !adaptive || entry >= 3;
            assert_eq!(snapshot.peak_bytes > 0, hot);
            assert_eq!(store.register_dispatches() > before, hot);
            assert_eq!(snapshot.peak_frames, usize::from(hot));
            peaks.push(snapshot.peak_bytes);
        }
        assert_eq!(peaks[2], peaks[3]);
        all_peaks.extend(peaks);
    }
    assert_eq!(all_peaks[0], all_peaks[6]);
    eprintln!(
        "P144_METRIC {{\"case\":\"entry_peaks\",\"peaks\":{all_peaks:?},\"per_invoke_reset\":true}}"
    );
}

#[test]
fn p144_declined_fallback_reports_zero_register_activation_capacity() {
    let engine = Engine::with_register_entry_threshold(Config::default(), 1).unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    for name in ["ref_local", "vector_local"] {
        store.reset_register_activation_snapshot().unwrap();
        let before = store.register_fallback_frames();
        assert_eq!(
            store.invoke(function(&store, instance, name), &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        let snapshot = store.register_activation_snapshot();
        inactive(snapshot);
        assert_eq!(snapshot.peak_bytes, 0);
        assert!(store.register_fallback_frames() > before);
    }
}

#[test]
fn p144_nested_reentry_restores_scope_and_rejects_reset() {
    for cancel in [false, true] {
        let engine =
            Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
        let module = engine
            .compile(include_bytes!(
                "../fixtures/performance/p064/host_call.wasm"
            ))
            .unwrap();
        let mut store = Store::new(&engine);
        let child = Arc::new(Mutex::new(None));
        let host_child = child.clone();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let host_observed = observed.clone();
        let callback = store
            .create_host_func_with_caller(
                FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, args| {
                    let before = caller.store().register_activation_snapshot();
                    assert_eq!(before.active_scopes, 1);
                    assert_eq!(before.current_bytes, 0); // Call handoff has released the outer Register vectors.
                    assert!(caller.store().reset_register_activation_snapshot().is_err());
                    let target = host_child.lock().unwrap().unwrap();
                    assert_eq!(
                        caller.invoke(target, args)?,
                        RunOutcome::Returned(vec![Value::I32(44)])
                    );
                    let after = caller.store().register_activation_snapshot();
                    assert_eq!((after.current_bytes, after.active_scopes), (0, 1));
                    assert!(after.peak_bytes >= before.peak_bytes);
                    host_observed.lock().unwrap().push(after);
                    if cancel {
                        caller.store().cancellation_token().cancel();
                    }
                    Ok(vec![Value::I32(44)])
                },
            )
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker
            .define("env", "callback", External::Func(callback))
            .unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        *child.lock().unwrap() = Some(function(&store, instance, "child"));
        store.reset_register_activation_snapshot().unwrap();
        let result = store.invoke(function(&store, instance, "run"), &[Value::I32(21)]);
        if cancel {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ResourceLimit);
        } else {
            assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(66)]));
        }
        let snapshot = store.register_activation_snapshot();
        inactive(snapshot);
        assert!(snapshot.peak_bytes > 0);
        assert_eq!(snapshot.peak_frames, 1);
        assert_eq!(observed.lock().unwrap().len(), 1);
        store.reset_register_activation_snapshot().unwrap();
        assert_eq!(
            store.register_activation_snapshot(),
            RegisterActivationSnapshot::default()
        );
        eprintln!("P144_METRIC {{\"case\":\"reentry\",\"cancel\":{cancel},\"peak_frames\":1,\"outer_released_before_callback\":true,\"cleanup\":true}}");
    }
}

#[test]
fn p144_eviction_pin_and_cancel_cleanup_keep_capacity_diagnostic_bounded() {
    let engine = Engine::with_register_entry_threshold(Config::default(), 1).unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p138/uniform66.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let mut peak = None;
    for index in (0..66).chain(std::iter::once(0)) {
        store.reset_register_activation_snapshot().unwrap();
        assert_eq!(
            store
                .invoke(
                    function(&store, instance, &format!("f{index:02}")),
                    &[Value::I32(7)]
                )
                .unwrap(),
            RunOutcome::Returned(vec![Value::I32(7 + index)])
        );
        let snapshot = store.register_activation_snapshot();
        inactive(snapshot);
        assert_eq!(
            *peak.get_or_insert(snapshot.peak_bytes),
            snapshot.peak_bytes
        );
        assert!(snapshot.peak_bytes > 0 && snapshot.peak_bytes <= 2 * 1024 * 1024);
    }
    let cache = module.register_cache_snapshot().unwrap();
    assert!(cache.lifecycle.evictions >= 3);
    assert!(cache.retained_bytes <= cache.byte_limit);
    store.reset_register_activation_snapshot().unwrap();
    store.cancellation_token().cancel();
    assert_eq!(
        store
            .invoke(function(&store, instance, "f00"), &[Value::I32(7)])
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    inactive(store.register_activation_snapshot());
    assert_eq!(store.register_activation_snapshot().peak_bytes, 0);
    eprintln!(
        "P144_METRIC {{\"case\":\"bounded_churn\",\"working_peak\":{},\"code_bytes\":{},\"evictions\":{},\"cancel_peak\":0}}",
        peak.unwrap(),
        cache.retained_bytes,
        cache.lifecycle.evictions
    );
}
