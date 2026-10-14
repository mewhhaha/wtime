#[test]
fn p064_prefix_preparation_limits_and_owner_guards() {
    let bytes = include_bytes!("../fixtures/performance/p064/direct_call.wasm");
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let module = engine.compile(bytes).unwrap();
    assert_eq!(module.data.register.as_ref().unwrap().count(), 2);
    let program = &module
        .data
        .register
        .as_ref()
        .unwrap()
        .function(1)
        .unwrap()
        .lowered
        .program;
    assert_eq!(program.instruction_count(), 8);
    assert!(matches!(
        program.operation(7),
        Some(Operation::ExitToDecoded)
    ));
    let foreign = engine.compile(bytes).unwrap();
    assert!(program.materialization(&foreign, 7).is_err());
    assert!(activate(&module, 1, &[Value::I32(21), Value::I32(0)], &mut vec![]).is_some());
    assert!(activate(&module, 1, &[Value::I32(21), Value::F32(0)], &mut vec![]).is_none());
    for (work, bytes) in [(0, MAX_BYTES), (MAX_WORK, 0), (MAX_WORK, 128)] {
        let mut budget = Budget::new(work, bytes);
        assert!(exits::prepare(&module, 1, &mut budget).is_none());
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
    assert!(exits::prepare(&module, 0, &mut budget).is_none());
    assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    let mut oversized = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    std::sync::Arc::get_mut(&mut oversized.data)
        .unwrap()
        .plans
        .as_mut()
        .unwrap()
        .functions[1]
        .stack_peak = usize::MAX;
    assert!(PreparedModule::prepare(&oversized).is_none());
}
