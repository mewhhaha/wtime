fn p133_engine() -> Engine {
    Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap()
}
fn p133_ledger(cache: &Cache) -> (usize, usize, usize) {
    let ledger = cache.ledger.lock().unwrap();
    (ledger.attempted, ledger.work, ledger.bytes)
}

#[test]
fn p133_cold_threshold_and_single_publish() {
    use crate::{ExecutionPolicy, External, Linker, RunOutcome, Store, Value};
    let mut config = Config::default();
    config.limits.fuel = Some(3);
    let engine = Engine::new(config).unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    let cache = Cache::with_limits(&module, 64, 3, MAX_WORK, MAX_BYTES).unwrap();
    let builds = std::sync::atomic::AtomicUsize::new(0);
    let builder = |module: &Module, body, budget: &mut Budget, work: &mut usize| {
        builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        prepare_function(module, body, budget, work)
    };
    assert!(cache.enter_with(&module, 0, &builder).is_none());
    assert!(cache.enter_with(&module, 0, &builder).is_none());
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    let artifact = cache.enter_with(&module, 0, &builder).unwrap();
    assert!(artifact.function.matches(&module, 0));
    assert!(!artifact.function.matches(&module, 1));
    let costs = p133_ledger(&cache);
    for _ in 0..20 {
        let same = cache.enter_with(&module, 0, &builder).unwrap();
        assert!(Arc::ptr_eq(&artifact, &same));
    }
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(p133_ledger(&cache), costs);
    assert_eq!(engine.execution_policy(), ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "i32_eqz").unwrap() else {
        panic!()
    };
    for _ in 0..2 {
        assert_eq!(
            store.invoke(function, &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(1)])
        );
    }
    #[cfg(feature = "execution-reference")]
    assert_eq!(store.register_dispatches(), 0);
    eprintln!(
        "P133_CASE {{\"name\":\"p133_cold_threshold_and_single_publish\",\"passed\":true,\"builders\":1,\"threshold\":3,\"same_artifact\":true,\"decoded_policy_and_fuel_preserved\":true}}"
    );
}

#[test]
fn p133_concurrent_duplicate_uses_one_unlocked_builder() {
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    let engine = p133_engine();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p062/float_ops.wasm"
        ))
        .unwrap();
    let cache = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let builders = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            cache
                .enter_with(&module, 0, |module, body, budget, work| {
                    builders.fetch_add(1, Ordering::SeqCst);
                    assert!(cache.state.try_lock().is_ok());
                    assert!(engine.canonical.try_lock().is_ok());
                    entered.wait();
                    release.wait();
                    prepare_function(module, body, budget, work)
                })
                .unwrap()
        });
        entered.wait();
        assert!(
            cache
                .enter_with(&module, 0, |_, _, _, _| panic!("duplicate builder"))
                .is_none()
        );
        assert!(
            cache
                .enter_with(&module, 1, |_, _, _, _| panic!("second body builder"))
                .is_none()
        );
        assert_eq!(builders.load(Ordering::SeqCst), 1);
        // The source/canonical registry can still service another compilation.
        engine
            .compile(include_bytes!(
                "../fixtures/performance/p064/direct_call.wasm"
            ))
            .unwrap();
        release.wait();
        let prepared = handle.join().unwrap();
        assert!(prepared.function.matches(&module, 0));
        assert!(Arc::ptr_eq(&prepared, &cache.enter(&module, 0).unwrap()));
    });
    assert_eq!(builders.load(Ordering::SeqCst), 1);
    assert_eq!(cache.state.lock().unwrap().active, None);
    eprintln!(
        "P133_CASE {{\"name\":\"p133_concurrent_duplicate_uses_one_unlocked_builder\",\"passed\":true,\"builders\":1,\"duplicate_fallbacks\":2,\"cache_lock_available\":true,\"canonical_lock_available\":true}}"
    );
}

#[test]
fn p133_decline_poison_foreign_and_aborted_lease_fail_closed() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let engine = p133_engine();
    let bytes = include_bytes!("../fixtures/performance/p065/barrier_guards.wasm");
    let module = engine.compile(bytes).unwrap();
    let cache = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let mut builds = 0;
    assert!(
        cache
            .enter_with(&module, 0, |module, body, budget, work| {
                builds += 1;
                prepare_function(module, body, budget, work)
            })
            .is_none()
    );
    let first = p133_ledger(&cache);
    // A saturated hotness counter must still wait all 128 subsequent entries.
    cache.state.lock().unwrap().entries[0].entries = u64::MAX;
    for _ in 0..127 {
        assert!(
            cache
                .enter_with(&module, 0, |_, _, _, _| panic!("cooldown retry"))
                .is_none()
        );
    }
    assert_eq!(p133_ledger(&cache), first);
    assert!(
        cache
            .enter_with(&module, 0, |module, body, budget, work| {
                builds += 1;
                prepare_function(module, body, budget, work)
            })
            .is_none()
    );
    let second = p133_ledger(&cache);
    assert!(second.0 < first.0 && second.1 == first.1 && second.2 == first.2);
    for _ in 0..256 {
        assert!(
            cache
                .enter_with(&module, 0, |_, _, _, _| panic!("negative retry"))
                .is_none()
        );
    }
    assert_eq!(builds, 2);
    assert_eq!(p133_ledger(&cache), second);
    cache.state.lock().unwrap().entries[0].entries = u64::MAX;
    assert!(cache.enter(&module, 0).is_none());
    assert_eq!(cache.state.lock().unwrap().entries[0].entries, u64::MAX);
    let same_engine_other_source = engine.compile(bytes).unwrap();
    let foreign_engine = p133_engine().compile(bytes).unwrap();
    for foreign in [&same_engine_other_source, &foreign_engine] {
        assert!(
            cache
                .enter_with(foreign, 2, |_, _, _, _| panic!("foreign builder"))
                .is_none()
        );
    }
    assert!(cache.enter(&module, usize::MAX).is_none());
    assert_eq!(p133_ledger(&cache), second);
    // Revalidate the returned artifact too, independently of the lookup owner.
    let bad_artifact = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let before = p133_ledger(&bad_artifact);
    assert!(
        bad_artifact
            .enter_with(&module, 2, |_, body, budget, work| prepare_function(
                &same_engine_other_source,
                body,
                budget,
                work
            ))
            .is_none()
    );
    assert_eq!(p133_ledger(&bad_artifact).2, before.2);
    assert!(
        bad_artifact
            .enter_with(&module, 2, |_, _, _, _| panic!("bad owner retried"))
            .is_none()
    );
    let mut malformed = engine.compile(bytes).unwrap();
    Arc::get_mut(&mut malformed.data)
        .unwrap()
        .plans
        .as_mut()
        .unwrap()
        .functions[2]
        .stack_peak = usize::MAX;
    let malformed_cache = Cache::with_limits(&malformed, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    assert!(malformed_cache.enter(&malformed, 2).is_none());
    assert!(
        malformed_cache
            .enter_with(&malformed, 2, |_, _, _, _| panic!("invalid retried"))
            .is_none()
    );

    let direct = engine
        .compile(include_bytes!(
            "../fixtures/performance/p064/direct_call.wasm"
        ))
        .unwrap();
    let abort = Cache::with_limits(&direct, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let before = p133_ledger(&abort);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            abort.enter_with(&direct, 0, |module, body, budget, work| {
                let result = prepare_function(module, body, budget, work);
                assert!(matches!(result, Preparation::Complete(_)));
                panic!("intentional aborted builder")
            });
        }))
        .is_err()
    );
    let after = p133_ledger(&abort);
    assert!(after.0 < before.0 && after.1 < before.1 && after.2 == before.2);
    assert_eq!(abort.state.lock().unwrap().active, None);
    assert!(
        abort
            .enter_with(&direct, 0, |_, _, _, _| panic!("abort retried"))
            .is_none()
    );
    assert!(abort.enter(&direct, 1).is_some());

    let poisoned = Cache::new(&module).unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _guard = poisoned.state.lock().unwrap();
            panic!("intentional state poison");
        }))
        .is_err()
    );
    assert!(poisoned.enter(&module, 2).is_none());
    let poisoned_ledger = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let artifact = poisoned_ledger.enter(&module, 2).unwrap();
    let charged = p133_ledger(&poisoned_ledger).2;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _guard = poisoned_ledger.ledger.lock().unwrap();
            panic!("intentional ledger poison");
        }))
        .is_err()
    );
    assert!(poisoned_ledger.enter(&module, 2).is_none());
    poisoned_ledger.state.lock().unwrap().entries.clear();
    drop(artifact);
    assert_eq!(
        poisoned_ledger
            .ledger
            .lock()
            .err()
            .unwrap()
            .into_inner()
            .bytes,
        charged
    );
    eprintln!(
        "P133_CASE {{\"name\":\"p133_decline_poison_foreign_and_aborted_lease_fail_closed\",\"passed\":true,\"decline_builders\":2,\"cooldown_entries\":128,\"foreign_owner_refused\":true,\"invalid_refused\":true,\"abort_cleared_lease\":true,\"poison_fail_closed\":true,\"counter_saturates\":true}}"
    );
}

#[test]
fn p133_fifo_eviction_pinned_lifetime_and_budget_churn() {
    let engine = p133_engine();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    assert_eq!(module.data.inner.decoded().code.len(), 66);
    let cache = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let overhead = p133_ledger(&cache).2;
    let pinned = cache.enter(&module, 0).unwrap();
    let pinned_bytes = pinned._retention.bytes;
    for body in 1..64 {
        assert!(cache.enter(&module, body).is_some());
    }
    let before = p133_ledger(&cache);
    let next = cache.enter(&module, 64).unwrap();
    let retained = p133_ledger(&cache);
    assert_eq!(retained.2, before.2 + next._retention.bytes);
    assert!(retained.0 < before.0 && retained.1 < before.1);
    assert!(pinned.function.matches(&module, 0));
    assert_eq!(Arc::strong_count(&pinned), 1);
    {
        let state = cache.state.lock().unwrap();
        assert_eq!(state.entries.len(), MAX_ENTRIES);
        assert_eq!(state.entries[0].body, 1);
        assert_eq!(state.entries[63].body, 64);
        let cached_bytes = state
            .entries
            .iter()
            .map(|entry| match &entry.state {
                EntryState::Ready(function) => function._retention.bytes,
                _ => 0,
            })
            .sum::<usize>();
        assert_eq!(retained.2, overhead + cached_bytes + pinned_bytes);
    }
    drop(pinned);
    let released = p133_ledger(&cache);
    assert_eq!(
        released,
        (retained.0, retained.1, retained.2 - pinned_bytes)
    );
    assert!(cache.enter(&module, 65).is_some());
    assert_eq!(cache.state.lock().unwrap().entries[0].body, 2);
    let tiny_base = Cache::with_limits(&module, 2, 1, MAX_WORK, MAX_BYTES).unwrap();
    let base_bytes = p133_ledger(&tiny_base).2;
    let no_wrapper = Cache::with_limits(&module, 2, 1, MAX_WORK, base_bytes).unwrap();
    let unchanged = p133_ledger(&no_wrapper);
    assert!(
        no_wrapper
            .enter_with(&module, 0, |_, _, _, _| panic!("unreserved wrapper built"))
            .is_none()
    );
    for _ in 0..256 {
        assert!(
            no_wrapper
                .enter_with(&module, 0, |_, _, _, _| panic!(
                    "wrapper exhaustion retried"
                ))
                .is_none()
        );
    }
    assert!(matches!(
        no_wrapper.state.lock().unwrap().entries[0].state,
        EntryState::Negative
    ));
    assert_eq!(p133_ledger(&no_wrapper), unchanged);
    let refused =
        Cache::with_limits(&module, 2, 1, MAX_WORK, base_bytes + pinned_bytes - 1).unwrap();
    assert!(refused.enter(&module, 0).is_none());
    assert_eq!(p133_ledger(&refused).2, base_bytes);
    let no_work = Cache::with_limits(&module, 2, 1, 2, MAX_BYTES).unwrap();
    assert!(no_work.enter(&module, 0).is_none());
    assert!(
        no_work
            .enter_with(&module, 0, |_, _, _, _| panic!("exhaustion retried"))
            .is_none()
    );
    assert_eq!(p133_ledger(&no_work).0, 0);
    let churn = Cache::with_limits(&module, 2, 1, 1000, MAX_BYTES).unwrap();
    let mut previous = p133_ledger(&churn);
    for round in 0..200 {
        let _ = churn.enter(&module, round % 66);
        let current = p133_ledger(&churn);
        assert!(current.0 <= previous.0 && current.1 <= previous.1 && current.2 <= MAX_BYTES);
        assert!(churn.state.lock().unwrap().entries.len() <= 2);
        previous = current;
    }
    // Negative/cooldown entries stay indexed, so capacity pressure cannot erase
    // the retry cap or turn unsupported bodies into an every-entry scan.
    let guards = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    let sticky = Cache::with_limits(&guards, 2, 1, MAX_WORK, MAX_BYTES).unwrap();
    for _ in 0..129 {
        let _ = sticky.enter(&guards, 0);
        let _ = sticky.enter(&guards, 1);
    }
    let stable = p133_ledger(&sticky);
    for _ in 0..256 {
        assert!(
            sticky
                .enter_with(&guards, 0, |_, _, _, _| panic!("negative evicted"))
                .is_none()
        );
        assert!(
            sticky
                .enter_with(&guards, 2, |_, _, _, _| panic!("capacity unbounded"))
                .is_none()
        );
    }
    assert_eq!(p133_ledger(&sticky), stable);
    assert_eq!(sticky.state.lock().unwrap().entries.len(), 2);
    eprintln!(
        "P133_CASE {{\"name\":\"p133_fifo_eviction_pinned_lifetime_and_budget_churn\",\"passed\":true,\"max_entries\":64,\"fixture_bodies\":66,\"pinned_bytes\":{pinned_bytes},\"last_drop_refunds_only_bytes\":true,\"fifo_preserved\":true,\"metadata_refusal\":true,\"monotonic_work\":true,\"negative_churn_bounded\":true}}"
    );
}
