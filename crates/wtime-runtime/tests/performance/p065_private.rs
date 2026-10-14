#[test]
fn p065_root_barrier_recipes_are_complete() {
    let engine = Engine::with_execution_policy(
        Config {
            features: crate::FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        crate::ExecutionPolicy::Register,
    )
    .unwrap();
    for (bytes, body, end, locals, operands) in [
        (
            include_bytes!("../fixtures/performance/p065/numeric_capture.wasm").as_slice(),
            0,
            5,
            1,
            1,
        ),
        (
            include_bytes!("../fixtures/performance/p065/exact_capture.wasm").as_slice(),
            0,
            5,
            1,
            1,
        ),
        (
            include_bytes!("../fixtures/performance/p065/numeric_exception.wasm").as_slice(),
            0,
            3,
            0,
            1,
        ),
    ] {
        let module = engine.compile(bytes).unwrap();
        let lowered = &module
            .data
            .register
            .as_ref()
            .unwrap()
            .function(body)
            .unwrap()
            .lowered;
        let program = &lowered.program;
        assert_eq!(program.instruction_count(), end + 1);
        for pc in 0..end {
            assert!(!matches!(
                program.operation(pc),
                Some(Operation::ExitToDecoded)
            ));
            let recipe = program.materialization(&module, pc).unwrap();
            assert_eq!(recipe.source_pc, pc);
            assert_eq!(recipe.locals.len(), locals);
            assert_eq!(recipe.body, body);
        }
        assert!(matches!(
            program.operation(end),
            Some(Operation::ExitToDecoded)
        ));
        let recipe = program.materialization(&module, end).unwrap();
        assert_eq!(recipe.locals.len(), locals);
        assert_eq!(recipe.operands.len(), operands);
        assert_eq!(recipe.source_pc, end);
        let other = engine.compile(bytes).unwrap();
        assert!(program.materialization(&other, end).is_err());
        for corruption in [0, 1, 3, 4, 5, 6] {
            let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
            let prefix =
                super::super::lower::LoweredFunction::lower_prefix(&module, body, &mut budget)
                    .unwrap();
            let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
            assert!(
                prefix
                    .program
                    .p131_revalidate(&module, end, corruption, &mut budget)
                    .is_err()
            );
            assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
        }
    }
    for opcode in [
        0x08, 0x0a, 0x10, 0x15, 0x1f, 0xd0, 0xe2, 0xe6, 0xfb00, 0xfb1a,
    ] {
        assert!(super::super::exits::transition(opcode));
    }
}
