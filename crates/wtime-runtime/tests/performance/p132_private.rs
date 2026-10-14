#[test]
fn p132_unsupported_and_overbudget_functions_decline_without_publication() {
    let engine = Engine::new(Config {
        features: crate::FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    let mut attempted = MAX_WORK;
    assert!(matches!(
        prepare_function(&module, usize::MAX, &mut budget, &mut attempted),
        Preparation::Invalid
    ));
    assert_eq!(attempted, MAX_WORK);
    assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    for body in [0, 1] {
        let before = attempted;
        assert!(matches!(
            prepare_function(&module, body, &mut budget, &mut attempted),
            Preparation::Declined
        ));
        assert!(attempted < before);
        assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    }
    let mut attempted = 0;
    assert!(matches!(
        prepare_function(&module, 2, &mut budget, &mut attempted),
        Preparation::Exhausted
    ));
    assert_eq!(attempted, 0);
    assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    let mut budget = Budget::new(MAX_WORK, 0);
    let mut attempted = MAX_WORK;
    assert!(matches!(
        prepare_function(&module, 2, &mut budget, &mut attempted),
        Preparation::Declined
    ));
    assert!(attempted < MAX_WORK);
    assert_eq!(budget.remaining(), (MAX_WORK, 0));
    let mut oversized = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    std::sync::Arc::get_mut(&mut oversized.data)
        .unwrap()
        .plans
        .as_mut()
        .unwrap()
        .functions[2]
        .stack_peak = usize::MAX;
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    let mut attempted = MAX_WORK;
    assert!(matches!(
        prepare_function(&oversized, 2, &mut budget, &mut attempted),
        Preparation::Invalid
    ));
    assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    assert_eq!(attempted, MAX_WORK);
    assert!(PreparedModule::prepare(&oversized).is_none());
}

#[test]
fn p132_module_attempts_and_aggregate_budget_match_predecessor() {
    let engine = Engine::new(Config {
        features: crate::FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p065/barrier_guards.wasm"
        ))
        .unwrap();
    let count = module.data.inner.decoded().code.len();
    assert_eq!(count, 4);
    let prepared = PreparedModule::prepare(&module).unwrap();
    assert_eq!(prepared.count(), 2);
    let mut index: Vec<Option<PreparedFunction>> = Vec::new();
    index.try_reserve_exact(count).unwrap();
    let bytes =
        index.capacity() * size_of::<Option<PreparedFunction>>() + size_of::<PreparedModule>();
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    budget.reserve(count, bytes).unwrap();
    let mut attempted = MAX_WORK - count;
    let initial = budget.remaining();
    for body in 0..count {
        let before_work = attempted;
        let before_budget = budget.remaining();
        let result = prepare_function(&module, body, &mut budget, &mut attempted);
        assert!(attempted < before_work);
        match result {
            Preparation::Complete(function) => {
                assert!(body >= 2);
                let expected = prepared.function(body).unwrap();
                assert_eq!(
                    function.lowered.program.instruction_count(),
                    expected.lowered.program.instruction_count()
                );
                assert_eq!(
                    function.lowered.program.costs(),
                    expected.lowered.program.costs()
                );
                assert!(budget.remaining().0 < before_budget.0);
                assert!(budget.remaining().1 < before_budget.1);
                assert!(function.lowered.program.materialization(&module, 0).is_ok());
            }
            Preparation::Declined => {
                assert!(body < 2);
                assert!(prepared.function(body).is_none());
                assert_eq!(budget.remaining(), before_budget);
            }
            _ => panic!("unexpected bounded module outcome"),
        }
    }
    assert!(budget.remaining().0 < initial.0 && budget.remaining().1 < initial.1);
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    let mut attempted = MAX_WORK;
    let Preparation::Complete(_) = prepare_function(&module, 2, &mut budget, &mut attempted) else {
        panic!()
    };
    let before = budget.remaining();
    let mut limited = Budget::new(before.0, 0);
    assert!(matches!(
        prepare_function(&module, 3, &mut limited, &mut attempted),
        Preparation::Declined
    ));
    assert_eq!(limited.remaining(), (before.0, 0));
    eprintln!(
        "P132: four ordered body outcomes; declined attempts spend work; retained metadata commits only on success"
    );
}
