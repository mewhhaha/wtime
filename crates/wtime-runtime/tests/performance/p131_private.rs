const P131_CALL: &[u8] = include_bytes!("../fixtures/performance/accounting/transient_stack.wasm");
const P131_LOOP: &[u8] = include_bytes!("../fixtures/performance/p060/multivalue_caller.wasm");
const P131_MEMORY: &[u8] = include_bytes!("../fixtures/performance/p047/p047_effects_prefix.wasm");
#[derive(Default)]
struct P131Counts {
    boundaries: usize,
    work: usize,
    bytes: usize,
    labels: usize,
    memory: usize,
}
fn p131_compare(bytes: &[u8], body: usize, args: &[Value]) -> P131Counts {
    let engine = Engine::new(Config::default()).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut budget = Budget::new(100_000, 1024 * 1024);
    let lowered = LoweredFunction::lower_prefix(&module, body, &mut budget).unwrap();
    assert_eq!(
        budget.remaining(),
        (100_000 - lowered.work, 1024 * 1024 - lowered.retained_bytes)
    );
    let (initial, _) = invoke(bytes, args, 0, None);
    let mut images: Vec<Vec<u8>> = initial
        .mems
        .iter()
        .map(|mem| mem.read(0, mem.len().unwrap()).unwrap())
        .collect();
    let locals = lowered.program.materialization(&module, 0).unwrap().locals;
    let mut slots: Vec<Value> = locals.iter().copied().map(zero).collect();
    let local_count = slots.len();
    slots.resize(lowered.program.slots() as usize, Value::I32(0));
    slots[..args.len()].copy_from_slice(args);
    let mut labels = vec![];
    let mut charged = 0usize;
    let mut pc = 0;
    let mut result = P131Counts {
        work: lowered.work,
        bytes: lowered.retained_bytes,
        ..Default::default()
    };
    loop {
        assert!(pc < 128);
        let recipe = lowered.program.materialization(&module, pc).unwrap();
        assert_eq!(
            (recipe.body, recipe.source_pc, recipe.prepared_pc),
            (body, pc, pc)
        );
        let (snapshot_store, _) = invoke(bytes, args, pc as u64 + 1, Some((pc, 0)));
        let view = snapshot_store.execution_snapshot().unwrap();
        assert!(view.pending_boundary_charged);
        assert_eq!(view.remaining_fuel, Some(0));
        assert_eq!(view.frames.len(), view.frame_count);
        assert_eq!(view.operands.len(), view.operand_count);
        assert_eq!(
            view.frames[0].locals,
            slots[..local_count]
                .iter()
                .copied()
                .map(diagnostic)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            view.operands,
            recipe
                .operands
                .iter()
                .map(|entry| {
                    assert_eq!(value_type(slots[entry.slot as usize]), entry.ty);
                    diagnostic(slots[entry.slot as usize])
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(view.frames[0].labels, labels);
        result.labels = result.labels.max(labels.len());
        let (before, _) = invoke(bytes, args, pc as u64, None);
        assert_eq!(
            before
                .mems
                .iter()
                .map(|mem| mem.read(0, mem.len().unwrap()).unwrap())
                .collect::<Vec<_>>(),
            images
        );
        result.boundaries += 1;
        if matches!(
            lowered.program.operation(pc),
            Some(Operation::ExitToDecoded)
        ) {
            assert_eq!(charged, pc); // The pending opcode is not executed or charged.
            assert!(lowered.program.operation(pc + 1).is_none());
            break;
        }
        match lowered.program.operation(pc).unwrap() {
            Operation::Move { from, to, .. } => slots[*to as usize] = slots[*from as usize],
            Operation::Const { constant, to } => {
                slots[*to as usize] = match lowered.program.constant(*constant).unwrap() {
                    Constant::I32(n) => Value::I32(n as i32),
                    Constant::I64(n) => Value::I64(n as i64),
                    Constant::F32(n) => Value::F32(n),
                    Constant::F64(n) => Value::F64(n),
                }
            }
            Operation::Unary {
                opcode, from, to, ..
            } => {
                let mut stack = vec![slots[*from as usize]];
                crate::numeric::exec(*opcode, &mut stack).unwrap();
                slots[*to as usize] = stack[0];
            }
            Operation::Binary {
                opcode,
                left,
                right,
                to,
                ..
            } => {
                let mut stack = vec![slots[*left as usize], slots[*right as usize]];
                crate::numeric::exec(*opcode, &mut stack).unwrap();
                slots[*to as usize] = stack[0];
            }
            Operation::Jump { target } => assert_eq!(*target as usize, pc + 1),
            Operation::Memory {
                argument,
                address,
                value,
            } => {
                let argument = lowered.program.memory_argument(*argument).unwrap();
                let Value::I32(address) = slots[*address as usize] else {
                    panic!()
                };
                let offset = (address as u32 as usize)
                    .checked_add(argument.offset as usize)
                    .unwrap();
                let (_, width, store) =
                    super::super::memory::signature(lowered.program.opcode(pc).unwrap()).unwrap();
                assert!(store); // These independent prefix fixtures only store.
                let bits = match slots[*value as usize] {
                    Value::I32(n) => n as u32 as u64,
                    Value::I64(n) => n as u64,
                    Value::F32(n) => n as u64,
                    Value::F64(n) => n,
                    _ => panic!(),
                };
                images[argument.memory as usize][offset..offset + width]
                    .copy_from_slice(&bits.to_le_bytes()[..width]);
                result.memory += 1;
            }
            _ => panic!("nonlinear prefix operation admitted"),
        }
        let source = &module.data.inner.decoded().code[body].instructions[pc];
        match source.opcode {
            2 | 3 => {
                let (params, returns) = crate::execution::arity(&source.immediate, &module);
                let (end, _) = module.data.control.get(body, pc).unwrap();
                labels.push((
                    pc + 1,
                    end,
                    recipe.operands.len() - params,
                    params,
                    returns,
                    source.opcode == 3,
                    0,
                ));
            }
            0xb => {
                labels.pop().unwrap();
            }
            _ => {}
        }
        charged += 1;
        pc += 1;
    }
    result
}
#[test]
fn p131_prefix_values_and_source_boundaries_match() {
    assert_eq!(p131_compare(P131_CALL, 1, &[]).boundaries, 2);
    for n in [0, 17, 10000] {
        assert_eq!(p131_compare(P131_LOOP, 1, &[Value::I32(n)]).boundaries, 8);
    }
}
#[test]
fn p131_open_labels_and_memory_prefixes_match() {
    let mut totals = P131Counts::default();
    for (bytes, body, args) in [
        (P131_MEMORY, 0, vec![Value::I32(21)]),
        (P131_MEMORY, 0, vec![Value::I32(-15)]),
        (P131_LOOP, 1, vec![Value::I32(17)]),
    ] {
        let result = p131_compare(bytes, body, &args);
        totals.boundaries += result.boundaries;
        totals.work = totals.work.max(result.work);
        totals.bytes = totals.bytes.max(result.bytes);
        totals.labels = totals.labels.max(result.labels);
        totals.memory += result.memory;
    }
    assert_eq!(
        (totals.boundaries, totals.labels, totals.memory),
        (26, 2, 2)
    );
    eprintln!(
        "P131: {} prefix source boundaries; max_work={}; max_bytes={}; open_labels={}; memory_prefixes={}",
        totals.boundaries, totals.work, totals.bytes, totals.labels, totals.memory
    );
}
#[test]
fn p131_invalid_barriers_and_budgets_decline() {
    let engine = Engine::new(Config::default()).unwrap();
    let module = engine.compile(P131_LOOP).unwrap();
    let foreign = engine.compile(P131_LOOP).unwrap();
    for end in [0, usize::MAX] {
        let budget = Budget::new(100_000, 1024 * 1024);
        assert!(Admission::inspect_prefix(&module, 1, &budget, end).is_err());
        assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    }
    let first_call = engine
        .compile(include_bytes!(
            "../fixtures/performance/p063/growth-caller.wasm"
        ))
        .unwrap();
    let mut budget = Budget::new(100_000, 1024 * 1024);
    assert!(LoweredFunction::lower_prefix(&first_call, 1, &mut budget).is_err());
    assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    for corruption in 0..8 {
        let lowered =
            LoweredFunction::lower_prefix(&module, 1, &mut Budget::new(100_000, 1024 * 1024))
                .unwrap();
        let mut budget = Budget::new(100_000, 1024 * 1024);
        let owner = if corruption == 7 { &foreign } else { &module };
        assert!(
            lowered
                .program
                .p131_revalidate(owner, 7, corruption, &mut budget)
                .is_err()
        );
        assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    }
    for (work, bytes) in [
        (0, 1024 * 1024),
        (100_000, 0),
        (10, 1024 * 1024),
        (100_000, 128),
    ] {
        let mut budget = Budget::new(work, bytes);
        assert!(LoweredFunction::lower_prefix(&module, 1, &mut budget).is_err());
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let mut malformed = engine.compile(P131_LOOP).unwrap();
    std::sync::Arc::get_mut(&mut malformed.data)
        .unwrap()
        .control.set(1, 2, Some((0, None)));
    let mut budget = Budget::new(100_000, 1024 * 1024);
    assert!(LoweredFunction::lower_prefix(&malformed, 1, &mut budget).is_err());
    assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    let reg = Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let unsupported = reg.compile(P131_MEMORY).unwrap();
    assert_eq!(unsupported.data.register.as_ref().unwrap().count(), 0); // Foundation does not select prefixes.
    assert!(
        LoweredFunction::lower_prefix(
            &engine.compile(NUMERIC).unwrap(),
            0,
            &mut Budget::new(100_000, 1024 * 1024)
        )
        .is_err()
    );
}
