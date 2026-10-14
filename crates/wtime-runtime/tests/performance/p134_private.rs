#[test]
fn p134_activation_pin_survives_eviction() {
    let engine = Engine::with_register_entry_threshold(Config::default(), 1).unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    assert!(module.data.register.is_none());
    let cache = module.data.register_cache.as_ref().unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "i32_eqz").unwrap() else {
        panic!()
    };
    let mut frame = store.frame(function, vec![Value::I32(0)], 0).unwrap();
    assert_eq!(frame.pc, 0);
    let bytes = frame
        .register
        .as_ref()
        .unwrap()
        .artifact
        .as_ref()
        .unwrap()
        .retained_bytes();
    assert_eq!(cache.test_entry(0), Some((1, 1, true)));
    assert_eq!(
        Arc::strong_count(frame.register.as_ref().unwrap().artifact.as_ref().unwrap()),
        2
    );
    for body in 1..=64 {
        assert!(cache.enter(&module, body).is_some());
    }
    assert!(cache.test_entry(0).is_none());
    let before = cache.test_budget();
    let mut stack = Vec::new();
    for pc in 0..3 {
        assert_eq!(frame.pc, pc);
        assert_eq!(
            Arc::strong_count(frame.register.as_ref().unwrap().artifact.as_ref().unwrap()),
            1
        );
        let result = step(&mut frame, &mut stack, &mut store).unwrap();
        assert!(matches!(result, Step::Continue) == (pc < 2));
        assert!(matches!(result, Step::Return) == (pc == 2));
        assert_eq!(
            Arc::strong_count(frame.register.as_ref().unwrap().artifact.as_ref().unwrap()),
            1
        );
    }
    assert_eq!(stack, vec![Value::I32(1)]);
    assert_eq!(cache.test_budget(), before);
    drop(frame);
    assert_eq!(cache.test_budget(), (before.0, before.1, before.2 - bytes));
    // Every store entry shares the same module cache, not a Store-local cache.
    let External::Func(other) = instance.export(&store, "i64_eqz").unwrap() else {
        panic!()
    };
    let _ = store.frame(other, vec![Value::I64(0)], 0).unwrap();
    assert_eq!(cache.test_entry(1).unwrap().1, 1);
    // A ready artifact from another source cannot activate here.
    let foreign = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    assert!(cache.enter(&foreign, 0).is_none());
    assert!(
        activate_prepared(
            &foreign,
            0,
            &[Value::I32(0)],
            &mut vec![],
            cache.enter(&module, 2)
        )
        .is_none()
    );
    // Debit exactly one P132 attempt while independent Stores share this body.
    let shared = engine
        .compile(include_bytes!(
            "../fixtures/performance/p061/integer_ops.wasm"
        ))
        .unwrap();
    let shared_cache = shared.data.register_cache.as_ref().unwrap();
    let initial = shared_cache.test_budget();
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    let mut work = MAX_WORK;
    assert!(matches!(
        prepare_function(&shared, 0, &mut budget, &mut work),
        Preparation::Complete(_)
    ));
    let attempted_cost = MAX_WORK - work;
    let retained_cost = MAX_WORK - budget.remaining().0;
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let engine = engine.clone();
            let module = shared.clone();
            let barrier = &barrier;
            workers.push(scope.spawn(move || {
                let mut store = Store::new(&engine);
                let instance = Linker::new(&engine)
                    .instantiate(&mut store, &module)
                    .unwrap();
                let External::Func(function) = instance.export(&store, "i32_eqz").unwrap() else {
                    panic!()
                };
                barrier.wait();
                for _ in 0..20 {
                    assert_eq!(
                        store.invoke(function, &[Value::I32(0)]).unwrap(),
                        RunOutcome::Returned(vec![Value::I32(1)])
                    );
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let entry = shared_cache.test_entry(0).unwrap();
    assert!(entry.0 > 0 && entry.0 <= 40);
    assert_eq!((entry.1, entry.2), (1, true));
    let spent = shared_cache.test_budget();
    assert_eq!(
        (spent.0, spent.1),
        (initial.0 - attempted_cost, initial.1 - retained_cost)
    );

    // Resuming the saved decoded frame must not look up its source body again.
    let capture_engine = Engine::with_register_entry_threshold(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        1,
    )
    .unwrap();
    let provider = capture_engine
        .compile(include_bytes!("../fixtures/performance/p065/provider.wasm"))
        .unwrap();
    let consumer = capture_engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/numeric_capture.wasm"
        ))
        .unwrap();
    let consumer_cache = consumer.data.register_cache.as_ref().unwrap();
    let mut capture_store = Store::new(&capture_engine);
    let provider_instance = Linker::new(&capture_engine)
        .instantiate(&mut capture_store, &provider)
        .unwrap();
    let External::Tag(tag) = provider_instance.export(&capture_store, "yield").unwrap() else {
        panic!()
    };
    let mut linker = Linker::new(&capture_engine);
    linker
        .define("provider", "yield", External::Tag(tag))
        .unwrap();
    let instance = linker.instantiate(&mut capture_store, &consumer).unwrap();
    let External::Func(run) = instance.export(&capture_store, "run").unwrap() else {
        panic!()
    };
    let RunOutcome::Suspended { continuation, .. } = capture_store.invoke(run, &[]).unwrap() else {
        panic!()
    };
    let root = capture_store.root(continuation).unwrap();
    let continuation = Value::Ref(RefValue::Gc(root.reference()));
    let External::Func(make) = instance.export(&capture_store, "make").unwrap() else {
        panic!()
    };
    let RunOutcome::Returned(objects) = capture_store.invoke(make, &[]).unwrap() else {
        panic!()
    };
    let before_resume = consumer_cache.test_entry(0).unwrap();
    assert_eq!(before_resume, (1, 1, true));
    let External::Func(resume) = instance.export(&capture_store, "resume").unwrap() else {
        panic!()
    };
    assert_eq!(
        capture_store
            .invoke(resume, &[continuation, objects[0]])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(112)])
    );
    assert_eq!(consumer_cache.test_entry(0), Some(before_resume));
    eprintln!(
        "P134_CASE {{\"name\":\"p134_activation_pin_survives_eviction\",\"passed\":true,\"eager_artifact_absent\":true,\"builders_per_body\":1,\"pinned_bytes\":{bytes},\"dispatch_arc_count\":1,\"last_drop_refunds_only_bytes\":true,\"foreign_owner_refused\":true,\"shared_concurrent_builders\":1,\"resume_entry_lookups\":0}}"
    );
}
