const LOOP: &[u8] = include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm");
fn admitted_loop() -> (Module, Admission, Draft, Budget) {
    let module = Engine::new(Config::default())
        .unwrap()
        .compile(LOOP)
        .unwrap();
    let budget = Budget::new(MAX_WORK, MAX_BYTES);
    let admission = Admission::inspect(&module, 0, &budget).unwrap();
    assert_eq!(
        (
            admission.locals.len(),
            admission.stack_peak,
            admission.slots
        ),
        (3, 2, 5)
    );
    let heights = [
        0, 1, 0, 1, 0, 1, 0, 0, 0, 1, 2, 1, 0, 1, 2, 1, 0, 1, 2, 1, 0, 0, 0, 0, 1,
    ];
    let source = &module.data.inner.decoded().code[0].instructions;
    assert_eq!(source.len(), heights.len());
    let mut draft = Draft {
        operations: vec![],
        events: vec![],
        stack_slots: vec![],
        targets: vec![],
        constants: vec![],
        memory: vec![],
        results: vec![3],
    };
    for (pc, (instruction, height)) in source.iter().zip(heights).enumerate() {
        let stack_start = draft.stack_slots.len() as u32;
        draft.stack_slots.extend((0..height).map(|h| StackSlot {
            slot: 3 + h,
            ty: NumericType::I32,
        }));
        let edge_start = draft.targets.len() as u32;
        let targets = match pc {
            11 => vec![23, 12],
            20 => vec![8],
            _ => vec![pc as u32 + 1],
        };
        draft.targets.extend(&targets);
        draft.events.push(SourceEvent {
            source_pc: pc,
            opcode: instruction.opcode,
            effect: classify(instruction.opcode),
            fuel_charge: 1,
            cancellation_checkpoint: true,
            stack: stack_start..draft.stack_slots.len() as u32,
            successors: edge_start..draft.targets.len() as u32,
        });
        let operation = match (&instruction.immediate, instruction.opcode) {
            (Immediate::Index(local), 0x20) => Operation::Move {
                from: *local,
                to: 3 + height,
                ty: NumericType::I32,
            },
            (Immediate::Index(local), 0x21) => Operation::Move {
                from: 3 + height - 1,
                to: *local,
                ty: NumericType::I32,
            },
            (Immediate::I32(value), 0x41) => {
                let constant = draft.constants.len() as u32;
                draft.constants.push(Constant::I32(*value as u32));
                Operation::Const {
                    constant,
                    to: 3 + height,
                }
            }
            (_, 0x4e | 0x6a) => Operation::Binary {
                opcode: instruction.opcode,
                left: 3,
                right: 4,
                to: 3,
                input: NumericType::I32,
                result: NumericType::I32,
            },
            (_, 0xd) => Operation::Branch {
                condition: 3,
                yes: targets[0],
                no: targets[1],
            },
            (_, 0xb) if pc == 24 => Operation::Exit { results: 0..1 },
            _ => Operation::Jump { target: targets[0] },
        };
        draft.operations.push(operation);
    }
    (module, admission, draft, budget)
}

#[test]
fn p057_numeric_loop_has_bounded_typed_slots_and_source_boundaries() {
    let (module, admission, draft, mut budget) = admitted_loop();
    let function = RegisterFunction::from_draft(&module, admission, draft, &mut budget).unwrap();
    assert_eq!(function.draft.events.len(), 25);
    let recipe = function.materialization(&module, 10).unwrap();
    assert_eq!(
        (recipe.body, recipe.source_pc, recipe.prepared_pc),
        (0, 10, 10)
    );
    assert_eq!(recipe.locals, &[NumericType::I32; 3]);
    assert_eq!(
        recipe.operands,
        &[
            StackSlot {
                slot: 3,
                ty: NumericType::I32
            },
            StackSlot {
                slot: 4,
                ty: NumericType::I32
            }
        ]
    );
    assert!(function.materialization(&module, 25).is_err());
    assert_eq!(
        function.draft.events[10].stack.end - function.draft.events[10].stack.start,
        2
    );
    let edges = &function.draft.events[20].successors;
    assert_eq!(
        &function.draft.targets[edges.start as usize..edges.end as usize],
        &[8]
    );
    assert!(function.work > 0 && function.retained_bytes > 0);
    assert_eq!(budget.work + function.work, MAX_WORK);
    assert_eq!(budget.bytes + function.retained_bytes, MAX_BYTES);
    assert!(function.admission.matches(&module.clone()));
    let foreign = Engine::new(Config::default())
        .unwrap()
        .compile(LOOP)
        .unwrap();
    assert!(!function.admission.matches(&foreign));
    assert_eq!(Arc::strong_count(&module.data), 1); // program retains bytes, never ModuleData
    eprintln!(
        "P057: 25 source events; 5 physical slots; logical peak 2; {} bytes; {} work",
        function.retained_bytes, function.work
    );
}

#[test]
fn p057_unsupported_types_and_effects_decline_without_guest_work() {
    let engine = Engine::new(Config::default()).unwrap();
    for bytes in [
        include_bytes!("../fixtures/performance/p047/p047_mixed_externref_local.wasm").as_slice(),
        include_bytes!("../fixtures/performance/p047/p047_effects_prefix.wasm").as_slice(),
        include_bytes!("../fixtures/performance/simd/splat_zero_loads.wasm").as_slice(),
    ] {
        let module = engine.compile(bytes).unwrap();
        let budget = Budget::new(MAX_WORK, MAX_BYTES);
        assert!(matches!(
            Admission::inspect(&module, 0, &budget),
            Err(Declined::Unsupported)
        ));
        assert_eq!((budget.work, budget.bytes), (MAX_WORK, MAX_BYTES));
        // Admission has no Store, frame, fuel or guest invocation path.
    }
}

#[test]
fn p057_bad_metadata_and_aggregate_budgets_are_transactional() {
    for case in 0..10 {
        let (module, admission, mut draft, mut budget) = admitted_loop();
        match case {
            0 => {
                draft.operations[0] = Operation::Const {
                    constant: 0,
                    to: u32::MAX,
                }
            }
            1 => draft.events[0].source_pc = 1,
            2 => draft.events[0].opcode = 0x42,
            3 => draft.events[0].fuel_charge = 0,
            4 => draft.events[0].cancellation_checkpoint = false,
            5 => draft.targets[0] = 2,
            6 => draft.stack_slots[0].slot = 4,
            7 => draft.events[0].stack.end = u32::MAX,
            8 => {
                draft.operations[10] = Operation::Unary {
                    opcode: 0x4e,
                    from: 4,
                    to: 4,
                    input: NumericType::I32,
                    result: NumericType::I32,
                }
            }
            _ => {
                draft.operations[10] = Operation::Binary {
                    opcode: 0x4e,
                    left: 3,
                    right: 3,
                    to: 3,
                    input: NumericType::I32,
                    result: NumericType::I32,
                }
            }
        }
        assert!(matches!(
            RegisterFunction::from_draft(&module, admission, draft, &mut budget),
            Err(Declined::Metadata)
        ));
        assert_eq!((budget.work, budget.bytes), (MAX_WORK, MAX_BYTES));
    }
    for (work, bytes) in [(0, MAX_BYTES), (MAX_WORK, 0)] {
        let (module, admission, draft, _) = admitted_loop();
        let mut budget = Budget::new(work, bytes);
        assert!(matches!(
            RegisterFunction::from_draft(&module, admission, draft, &mut budget),
            Err(Declined::Budget)
        ));
        assert_eq!((budget.work, budget.bytes), (work, bytes));
    }
    let (module, admission, draft, mut budget) = admitted_loop();
    let engine = Engine::new(Config::default()).unwrap();
    let foreign = engine.compile(LOOP).unwrap();
    assert!(matches!(
        RegisterFunction::from_draft(&foreign, admission, draft, &mut budget),
        Err(Declined::Metadata)
    ));
    assert_eq!((budget.work, budget.bytes), (MAX_WORK, MAX_BYTES));
    assert!(Admission::inspect(&module, 0, &Budget::new(0, MAX_BYTES)).is_err());
    let first = engine.compile(LOOP).unwrap();
    let second = engine.compile(LOOP).unwrap();
    let (_, _, draft, _) = admitted_loop();
    let admission = Admission::inspect(&first, 0, &budget).unwrap();
    assert_eq!(first.data.engine, second.data.engine);
    assert!(matches!(
        RegisterFunction::from_draft(&second, admission, draft, &mut budget),
        Err(Declined::Metadata)
    ));
    let (module, admission, draft, mut original_budget) = admitted_loop();
    let function =
        RegisterFunction::from_draft(&module, admission, draft, &mut original_budget).unwrap();
    let (module, admission, draft, _) = admitted_loop();
    let mut shared = Budget::new(function.work, MAX_BYTES);
    RegisterFunction::from_draft(&module, admission, draft, &mut shared).unwrap();
    assert_eq!(shared.work, 0);
    let (_, _, draft, _) = admitted_loop();
    let admission = Admission::inspect(&module, 0, &Budget::new(MAX_WORK, MAX_BYTES)).unwrap();
    let unchanged = (shared.work, shared.bytes);
    assert!(matches!(
        RegisterFunction::from_draft(&module, admission, draft, &mut shared),
        Err(Declined::Budget)
    ));
    assert_eq!((shared.work, shared.bytes), unchanged);
}
