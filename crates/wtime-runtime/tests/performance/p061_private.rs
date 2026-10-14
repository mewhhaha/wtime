#[test]
fn p061_preparation_and_activation_guards_remain_transactional() {
    let bytes = include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm");
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let module = engine.compile(bytes).unwrap();
    let prepared = module.data.register.as_ref().unwrap();
    assert_eq!(prepared.count(), 1);
    let function = prepared.function(0).unwrap();
    let foreign = engine.compile(bytes).unwrap();
    assert!(
        function
            .lowered
            .program
            .materialization(&foreign, 0)
            .is_err()
    );
    assert!(
        function
            .lowered
            .program
            .materialization(&module.clone(), 0)
            .is_ok()
    );
    assert_eq!(std::sync::Arc::strong_count(&module.data), 1);
    assert!(activate(&module, 0, &[Value::I32(0)], &mut vec![]).is_none());
    assert!(activate(&module, 0, &[Value::F32(0); 3], &mut vec![]).is_none());
    assert!(activate(&module, 0, &[Value::I32(0); 3], &mut vec![]).is_some());
    let mut missing = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    std::sync::Arc::get_mut(&mut missing.data).unwrap().plans = None;
    assert!(PreparedModule::prepare(&missing).is_none());
    let mut corrupt = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    std::sync::Arc::get_mut(&mut corrupt.data).unwrap().control[0][7] = Some((0, None));
    assert_eq!(PreparedModule::prepare(&corrupt).unwrap().count(), 0);
}
