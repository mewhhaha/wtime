#[test]
fn p060_swaps_rotations_and_backedges() {
    use super::super::lower::EdgeMove;
    for requests in [
        vec![
            EdgeMove {
                from: 0,
                to: 1,
                ty: NumericType::I32,
            },
            EdgeMove {
                from: 1,
                to: 0,
                ty: NumericType::I64,
            },
        ],
        vec![
            EdgeMove {
                from: 0,
                to: 1,
                ty: NumericType::I64,
            },
            EdgeMove {
                from: 1,
                to: 2,
                ty: NumericType::I64,
            },
            EdgeMove {
                from: 2,
                to: 0,
                ty: NumericType::I64,
            },
        ],
    ] {
        let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
        let plan = MovePlan::prepare(&requests, 3, &mut budget).unwrap();
        let original = [0x80000000u64, 0xdeadbeef12345678, 0xffffffffffffffff];
        let mut expected = original;
        for request in &requests {
            expected[request.to as usize] = original[request.from as usize];
        }
        let mut actual = original;
        apply_moves(&plan, &mut actual).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(plan.scratch_slots(), 1);
    }
    // Existing P061 integration covers multivalue backedge visits. This case
    // uses the actual production move applicator for otherwise latent cycles.
}

#[test]
fn p060_invalid_or_oversized_control_declines() {
    let bytes = include_bytes!("../fixtures/performance/p060/multivalue_table.wasm");
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let mut source = engine.compile(bytes).unwrap();
    assert_eq!(source.data.register.as_ref().unwrap().count(), 1);
    let function = source.data.register.as_ref().unwrap().function(0).unwrap();
    let Some(Operation::Table { targets, .. }) = function.lowered.program.operation(6) else {
        panic!()
    };
    assert_eq!(
        function.lowered.program.targets(targets.clone()).unwrap(),
        &[8, 9, 10]
    );
    assert!(
        function
            .lowered
            .program
            .targets(u32::MAX..u32::MAX)
            .is_none()
    );
    // A cached program moved onto separately compiled bytes cannot obtain an
    // activation certificate, even if the bytes and Engine are otherwise equal.
    let mut foreign = engine.compile(bytes).unwrap();
    let cached = std::sync::Arc::get_mut(&mut source.data)
        .unwrap()
        .register
        .take();
    std::sync::Arc::get_mut(&mut foreign.data).unwrap().register = cached;
    assert!(activate(&foreign, 0, &[Value::I32(0)], &mut vec![]).is_none());
    let module = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    for (work, bytes) in [(0, MAX_BYTES), (MAX_WORK, 0), (MAX_WORK, 128)] {
        let mut budget = Budget::new(work, bytes);
        assert!(matches!(
            LoweredFunction::lower(&module, 0, &mut budget),
            Err(super::super::ir::Declined::Budget)
        ));
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let mut malformed = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    std::sync::Arc::get_mut(&mut malformed.data)
        .unwrap()
        .control.set(0, 0, Some((0, None)));
    assert_eq!(PreparedModule::prepare(&malformed).unwrap().count(), 0);
    let mut excessive = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    std::sync::Arc::get_mut(&mut excessive.data)
        .unwrap()
        .plans
        .as_mut()
        .unwrap()
        .functions[0]
        .stack_peak = usize::MAX;
    assert!(PreparedModule::prepare(&excessive).is_none());
}
