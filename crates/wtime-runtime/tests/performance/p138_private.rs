fn p138_module() -> Module {
    p133_engine()
        .compile(include_bytes!(
            "../fixtures/performance/p138/uniform66.wasm"
        ))
        .unwrap()
}

#[test]
fn p138_private_snapshots_do_not_pin_evicted_artifacts() {
    let case = "p138_private_snapshots_do_not_pin_evicted_artifacts";
    let module = p138_module();
    let cache = Cache::with_limits(&module, 2, 1, MAX_WORK, MAX_BYTES).unwrap();
    let initial = cache.snapshot(&module).unwrap();
    let artifact = cache.enter(&module, 0).unwrap();
    let bytes = artifact.retained_bytes();
    assert_eq!(Arc::strong_count(&artifact), 2);
    let pinned = cache.snapshot(&module).unwrap();
    assert_eq!(Arc::strong_count(&artifact), 2);
    p138_emit(case, "pinned", &pinned);
    drop(cache.enter(&module, 1).unwrap());
    drop(cache.enter(&module, 2).unwrap());
    let evicted = cache.snapshot(&module).unwrap();
    assert_eq!(
        evicted
            .entries
            .iter()
            .map(|entry| entry.body)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(Arc::strong_count(&artifact), 1);
    assert_eq!(evicted.lifecycle.evictions, 1);
    assert!(evicted.retained_bytes >= initial.retained_bytes + bytes);
    p138_emit(case, "evicted-pinned", &evicted);
    drop(artifact);
    let released = cache.snapshot(&module).unwrap();
    assert_eq!(released.retained_bytes + bytes, evicted.retained_bytes);
    assert_eq!(
        released.remaining_attempted_work,
        evicted.remaining_attempted_work
    );
    assert_eq!(
        released.remaining_retained_work,
        evicted.remaining_retained_work
    );
    assert_eq!(released.lifecycle, evicted.lifecycle);
    p138_emit(case, "released", &released);
    eprintln!(
        "P138_CASE {{\"name\":\"{case}\",\"passed\":true,\"pinned_bytes\":{bytes},\"snapshot_clones_artifact\":false,\"work_refunded\":false,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}

#[test]
fn p138_private_failures_cooldown_abort_poison_and_foreign_are_observed() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let case = "p138_private_failures_cooldown_abort_poison_and_foreign_are_observed";
    let engine = p133_engine();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    let declined = Cache::with_limits(&module, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    assert!(declined.enter(&module, 0).is_none());
    let first = declined.snapshot(&module).unwrap();
    assert_eq!(first.lifecycle.declined, 1);
    assert_eq!(first.lifecycle.builder_starts, 1);
    assert_eq!(
        first.entries[0].state,
        RegisterCacheEntryState::Cooldown { remaining: 128 }
    );
    p138_emit(case, "decline-first", &first);
    declined.state.lock().unwrap().entries[0].entries = u64::MAX;
    for _ in 0..127 {
        assert!(
            declined
                .enter_with(&module, 0, |_, _, _, _| panic!("early retry"))
                .is_none()
        );
    }
    let cooldown = declined.snapshot(&module).unwrap();
    assert_eq!(
        cooldown.entries[0].state,
        RegisterCacheEntryState::Cooldown { remaining: 1 }
    );
    assert_eq!(cooldown.lifecycle, first.lifecycle);
    p138_emit(case, "decline-cooldown", &cooldown);
    assert!(declined.enter(&module, 0).is_none());
    let negative = declined.snapshot(&module).unwrap();
    assert_eq!(negative.entries[0].state, RegisterCacheEntryState::Negative);
    assert_eq!(negative.entries[0].attempts, 2);
    assert_eq!(negative.lifecycle.declined, 2);
    assert_eq!(negative.lifecycle.builder_starts, 2);
    p138_emit(case, "decline-negative", &negative);
    for _ in 0..100 {
        assert!(
            declined
                .enter_with(&module, 0, |_, _, _, _| panic!("negative retry"))
                .is_none()
        );
    }
    assert_eq!(declined.snapshot(&module).unwrap(), negative);

    let direct = p138_module();
    let aborted = Cache::with_limits(&direct, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    let baseline = aborted.snapshot(&direct).unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _ = aborted.enter_with(&direct, 0, |module, body, budget, work| {
                assert!(matches!(
                    prepare_function(module, body, budget, work),
                    Preparation::Complete(_)
                ));
                panic!("intentional diagnostic aborted builder");
            });
        }))
        .is_err()
    );
    let observed = aborted.snapshot(&direct).unwrap();
    assert_eq!(observed.active_body, None);
    assert_eq!(observed.lifecycle.aborted, 1);
    assert_eq!(observed.lifecycle.builder_starts, 1);
    assert_eq!(observed.entries[0].state, RegisterCacheEntryState::Negative);
    assert!(observed.remaining_attempted_work < baseline.remaining_attempted_work);
    assert!(observed.remaining_retained_work < baseline.remaining_retained_work);
    assert_eq!(observed.retained_bytes, baseline.retained_bytes);
    p138_emit(case, "aborted", &observed);

    let foreign = p138_module();
    assert!(aborted.snapshot(&foreign).is_none());
    let invalid = Cache::with_limits(&direct, 64, 1, MAX_WORK, MAX_BYTES).unwrap();
    assert!(
        invalid
            .enter_with(&direct, 0, |_, body, budget, work| prepare_function(
                &foreign, body, budget, work
            ))
            .is_none()
    );
    let observed = invalid.snapshot(&direct).unwrap();
    assert_eq!(observed.lifecycle.completed, 1);
    assert_eq!(observed.lifecycle.published, 0);
    assert_eq!(observed.lifecycle.publication_failures, 1);
    assert_eq!(observed.entries[0].state, RegisterCacheEntryState::Negative);
    p138_emit(case, "foreign-publication-refused", &observed);

    let bad_plan = Cache::with_limits(&direct, 1, 1, MAX_WORK, MAX_BYTES).unwrap();
    assert!(
        bad_plan
            .enter_with(&direct, 0, |_, _, _, _| Preparation::Invalid)
            .is_none()
    );
    let observed = bad_plan.snapshot(&direct).unwrap();
    assert_eq!(observed.lifecycle.invalid, 1);
    p138_emit(case, "invalid", &observed);

    let zero_work = Cache::with_limits(&direct, 1, 1, 1, MAX_BYTES).unwrap();
    assert!(zero_work.enter(&direct, 0).is_none());
    let observed = zero_work.snapshot(&direct).unwrap();
    assert_eq!(observed.lifecycle.exhausted, 1);
    assert_eq!(observed.lifecycle.builder_starts, 1);
    p138_emit(case, "exhausted-work", &observed);

    let overhead = Cache::with_limits(&direct, 1, 1, MAX_WORK, MAX_BYTES)
        .unwrap()
        .snapshot(&direct)
        .unwrap()
        .retained_bytes;
    let zero_bytes = Cache::with_limits(&direct, 1, 1, MAX_WORK, overhead).unwrap();
    assert!(
        zero_bytes
            .enter_with(&direct, 0, |_, _, _, _| panic!(
                "wrapper exhausted before builder"
            ))
            .is_none()
    );
    let observed = zero_bytes.snapshot(&direct).unwrap();
    assert_eq!(observed.lifecycle.preparation_starts, 1);
    assert_eq!(observed.lifecycle.builder_starts, 0);
    assert_eq!(observed.lifecycle.exhausted, 1);
    p138_emit(case, "exhausted-wrapper", &observed);

    for ledger in [false, true] {
        let poisoned = Cache::with_limits(&direct, 1, 1, MAX_WORK, MAX_BYTES).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                if ledger {
                    let _guard = poisoned.ledger.lock().unwrap();
                    panic!("intentional ledger poison");
                } else {
                    let _guard = poisoned.state.lock().unwrap();
                    panic!("intentional state poison");
                }
            }))
            .is_err()
        );
        assert!(poisoned.snapshot(&direct).is_none());
        assert!(poisoned.enter(&direct, 0).is_none());
    }
    eprintln!(
        "P138_CASE {{\"name\":\"{case}\",\"passed\":true,\"cooldown_entries\":128,\"declines\":2,\"abort_work_charged\":true,\"poisoned_snapshots_refused\":2,\"foreign_snapshot_refused\":true,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}

#[test]
fn p138_private_active_builder_snapshot_and_saturating_counters() {
    use std::sync::Barrier;
    let case = "p138_private_active_builder_snapshot_and_saturating_counters";
    let module = p138_module();
    let cache = Cache::with_limits(&module, 2, 1, MAX_WORK, MAX_BYTES).unwrap();
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            cache.enter_with(&module, 0, |module, body, budget, work| {
                assert!(cache.state.try_lock().is_ok());
                assert!(cache.ledger.try_lock().is_ok());
                entered.wait();
                release.wait();
                prepare_function(module, body, budget, work)
            })
        });
        entered.wait();
        let observed = cache.snapshot(&module).unwrap();
        assert_eq!(observed.active_body, Some(0));
        assert_eq!(observed.lifecycle.preparation_starts, 1);
        assert_eq!(observed.lifecycle.builder_starts, 1);
        assert_eq!(observed.lifecycle.completed, 0);
        assert_eq!(
            observed.entries[0].state,
            RegisterCacheEntryState::Compiling
        );
        p138_emit(case, "active", &observed);
        assert!(cache.enter(&module, 1).is_none());
        release.wait();
        drop(handle.join().unwrap().unwrap());
    });
    let observed = cache.snapshot(&module).unwrap();
    assert_eq!(observed.active_body, None);
    assert_eq!(observed.lifecycle.published, 1);
    p138_emit(case, "published", &observed);
    cache.state.lock().unwrap().lifecycle = RegisterCacheLifecycle {
        preparation_starts: u64::MAX,
        builder_starts: u64::MAX,
        completed: u64::MAX,
        published: u64::MAX,
        publication_failures: u64::MAX,
        declined: u64::MAX,
        exhausted: u64::MAX,
        invalid: u64::MAX,
        aborted: u64::MAX,
        evictions: u64::MAX,
    };
    drop(cache.enter(&module, 1).unwrap());
    drop(cache.enter(&module, 2).unwrap());
    let observed = cache.snapshot(&module).unwrap();
    assert_eq!(observed.lifecycle.evictions, u64::MAX);
    assert_eq!(observed.lifecycle.preparation_starts, u64::MAX);
    assert_eq!(observed.lifecycle.builder_starts, u64::MAX);
    assert_eq!(observed.lifecycle.completed, u64::MAX);
    assert_eq!(observed.lifecycle.published, u64::MAX);
    p138_emit(case, "saturated", &observed);
    eprintln!(
        "P138_CASE {{\"name\":\"{case}\",\"passed\":true,\"active_observed\":true,\"builder_unlocked\":true,\"counters_saturate\":true,\"instrumented\":true,\"timing_evidence\":false}}"
    );
}
