const NUMERIC: &[u8] = include_bytes!("../fixtures/performance/p058/numeric_slots.wasm");
const EDGE: &[u8] = include_bytes!("../fixtures/performance/p058/edge_values.wasm");
const MULTIVALUE: &[u8] = include_bytes!("../fixtures/performance/p040/multivalue_loop.wasm");
const LOOP: &[u8] = include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm");
const NESTED_IF: &[u8] = include_bytes!("../fixtures/performance/p040/nested_if_results.wasm");
fn diagnostic(value: Value) -> DiagnosticValue {
    match value {
        Value::I32(n) => DiagnosticValue::I32(n),
        Value::I64(n) => DiagnosticValue::I64(n),
        Value::F32(n) => DiagnosticValue::F32(n),
        Value::F64(n) => DiagnosticValue::F64(n),
        _ => panic!("non-numeric slot"),
    }
}
fn value_type(value: Value) -> NumericType {
    match value {
        Value::I32(_) => NumericType::I32,
        Value::I64(_) => NumericType::I64,
        Value::F32(_) => NumericType::F32,
        Value::F64(_) => NumericType::F64,
        _ => panic!("non-numeric slot"),
    }
}
fn zero(ty: NumericType) -> Value {
    match ty {
        NumericType::I32 => Value::I32(0),
        NumericType::I64 => Value::I64(0),
        NumericType::F32 => Value::F32(0),
        NumericType::F64 => Value::F64(0),
    }
}
fn invoke(
    bytes: &[u8],
    args: &[Value],
    fuel: u64,
    request: Option<(usize, usize)>,
) -> (Store, Result<RunOutcome>) {
    let mut config = Config::default();
    config.limits.fuel = Some(fuel);
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    if let Some((pc, visits)) = request {
        store
            .request_materialization_roundtrip_after_visits(function, pc, visits)
            .unwrap();
    }
    let outcome = store.invoke(function, args);
    (store, outcome)
}
// Deliberately a test-only slot model: reuse numeric semantics and apply edge
// requests simultaneously. Production register execution lands in P061/P062.
fn compare_prefixes(
    bytes: &[u8],
    args: &[Value],
    expected: &[Value],
    count: usize,
) -> LoweredFunction {
    let module = Engine::new(Config::default())
        .unwrap()
        .compile(bytes)
        .unwrap();
    let mut budget = Budget::new(100_000, 1024 * 1024);
    let lowered = LoweredFunction::lower(&module, 0, &mut budget).unwrap();
    assert_eq!(
        budget.remaining(),
        (100_000 - lowered.work, 1024 * 1024 - lowered.retained_bytes)
    );
    let local_types = lowered.program.materialization(&module, 0).unwrap().locals;
    let locals = local_types.len();
    let peak = module.data.plans.as_ref().unwrap().functions[0].stack_peak;
    let mut slots: Vec<Value> = local_types.iter().copied().map(zero).collect();
    slots.extend((0..peak).map(|_| Value::I32(0)));
    slots[..args.len()].copy_from_slice(args);
    let mut pc = 0;
    let mut visits = vec![0; module.data.inner.decoded().code[0].instructions.len()];
    let mut trace = 0;
    let result = loop {
        assert!(trace < 256, "unbounded fixture trace");
        let recipe = lowered.program.materialization(&module, pc).unwrap();
        assert_eq!(
            (recipe.source_pc, recipe.prepared_pc, recipe.body),
            (pc, pc, 0)
        );
        for (ty, value) in recipe.locals.iter().zip(&slots[..locals]) {
            assert_eq!(*ty, value_type(*value));
        }
        let operands: Vec<_> = recipe
            .operands
            .iter()
            .map(|entry| {
                assert_eq!(entry.ty, value_type(slots[entry.slot as usize]));
                diagnostic(slots[entry.slot as usize])
            })
            .collect();
        let (store, outcome) = invoke(bytes, args, trace as u64 + 1, Some((pc, visits[pc])));
        let snapshot = store
            .execution_snapshot()
            .expect("requested dynamic source visit");
        assert_eq!(
            (snapshot.cursor.source_pc, snapshot.cursor.prepared_pc),
            (pc, pc)
        );
        assert!(!snapshot.truncated && snapshot.pending_boundary_charged);
        assert_eq!(snapshot.remaining_fuel, Some(0));
        assert_eq!(snapshot.frame_count, 1);
        assert_eq!(
            snapshot.frames[0].locals,
            slots[..locals]
                .iter()
                .copied()
                .map(diagnostic)
                .collect::<Vec<_>>(),
            "locals at visit {trace} PC {pc}"
        );
        assert_eq!(
            snapshot.operands, operands,
            "operands at visit {trace} PC {pc}"
        );
        assert_eq!(snapshot.operand_count, operands.len());
        assert_eq!(store.materialization_roundtrips(), 1);
        visits[pc] += 1;
        trace += 1;
        let next = match lowered.program.operation(pc).unwrap() {
            Operation::Move { from, to, .. } => {
                slots[*to as usize] = slots[*from as usize];
                pc + 1
            }
            Operation::Const { constant, to } => {
                slots[*to as usize] = match lowered.program.constant(*constant).unwrap() {
                    Constant::I32(n) => Value::I32(n as i32),
                    Constant::I64(n) => Value::I64(n as i64),
                    Constant::F32(n) => Value::F32(n),
                    Constant::F64(n) => Value::F64(n),
                };
                pc + 1
            }
            Operation::Unary {
                opcode, from, to, ..
            } => {
                let mut values = vec![slots[*from as usize]];
                crate::numeric::exec(*opcode, &mut values).unwrap();
                slots[*to as usize] = values[0];
                pc + 1
            }
            Operation::Binary {
                opcode,
                left,
                right,
                to,
                ..
            } => {
                let mut values = vec![slots[*left as usize], slots[*right as usize]];
                crate::numeric::exec(*opcode, &mut values).unwrap();
                slots[*to as usize] = values[0];
                pc + 1
            }
            Operation::Jump { target } => *target as usize,
            Operation::Branch { condition, yes, no } => {
                let Value::I32(condition) = slots[*condition as usize] else {
                    panic!()
                };
                if condition != 0 {
                    *yes as usize
                } else {
                    *no as usize
                }
            }
            Operation::Exit { results } => {
                let results: Vec<_> = lowered
                    .program
                    .result_slots(results.clone())
                    .unwrap()
                    .iter()
                    .map(|slot| slots[*slot as usize])
                    .collect();
                assert_eq!(outcome.unwrap(), RunOutcome::Returned(results.clone()));
                break results;
            }
            Operation::Memory { .. } => panic!("unsupported operation admitted"),
        };
        assert_eq!(outcome.unwrap_err().kind, ErrorKind::ResourceLimit);
        if let Some(edge) = lowered.edge(pc, next) {
            let values: Vec<_> = edge
                .moves
                .iter()
                .map(|movement| {
                    assert_eq!(movement.ty, value_type(slots[movement.from as usize]));
                    slots[movement.from as usize]
                })
                .collect();
            for (movement, value) in edge.moves.iter().zip(values) {
                slots[movement.to as usize] = value;
            }
        }
        pc = next;
    };
    assert_eq!(result, expected);
    assert_eq!(trace, count);
    let (_, full) = invoke(bytes, args, count as u64, None);
    assert_eq!(full.unwrap(), RunOutcome::Returned(expected.to_vec()));
    let (_, short) = invoke(bytes, args, count as u64 - 1, None);
    assert_eq!(short.unwrap_err().kind, ErrorKind::ResourceLimit);
    eprintln!(
        "P058: {trace} dynamic source prefixes; {} source events; {} bytes; {} work",
        visits.len(),
        lowered.retained_bytes,
        lowered.work
    );
    lowered
}

#[test]
fn p058_local_numeric_slots_match_every_decoded_prefix() {
    let lowered = compare_prefixes(
        NUMERIC,
        &[Value::I32(21), Value::I32(4)],
        &[Value::I32(14)],
        16,
    );
    assert!(matches!(
        lowered.program.operation(3),
        Some(Operation::Binary {
            left: 3,
            right: 4,
            to: 3,
            ..
        })
    ));
    assert!(matches!(
        lowered.program.operation(10),
        Some(Operation::Unary {
            input: NumericType::F32,
            result: NumericType::I32,
            ..
        })
    ));
}
#[test]
fn p058_multivalue_edges_and_loop_visits_match_decoded_prefixes() {
    let shifted = compare_prefixes(EDGE, &[], &[Value::I32(7), Value::I64(9)], 6);
    assert_eq!(
        shifted.edge(4, 6).unwrap().moves,
        vec![
            EdgeMove {
                from: 1,
                to: 0,
                ty: NumericType::I32
            },
            EdgeMove {
                from: 2,
                to: 1,
                ty: NumericType::I64
            }
        ]
    );
    let looped = compare_prefixes(MULTIVALUE, &[], &[Value::I32(2), Value::I64(40)], 27);
    assert_eq!(looped.edge(12, 4).unwrap().moves.len(), 2);
    assert_eq!(looped.edge(7, 15).unwrap().moves.len(), 2);
    assert!(looped.block_entries.contains(&4));
    compare_prefixes(LOOP, &[], &[Value::I32(32)], 66);
    compare_prefixes(
        NESTED_IF,
        &[Value::I32(1), Value::I32(1)],
        &[Value::I32(41), Value::I64(420)],
        11,
    );
    compare_prefixes(
        NESTED_IF,
        &[Value::I32(1), Value::I32(0)],
        &[Value::I32(42), Value::I64(421)],
        10,
    );
    compare_prefixes(
        NESTED_IF,
        &[Value::I32(0), Value::I32(0)],
        &[Value::I32(-1), Value::I64(-2)],
        6,
    );
}
#[test]
fn p058_declines_invalid_joins_and_preserves_shared_budgets() {
    let engine = Engine::new(Config::default()).unwrap();
    let module = engine.compile(NUMERIC).unwrap();
    for (work, bytes) in [(0, 1024 * 1024), (100_000, 0), (100_000, 64)] {
        let mut budget = Budget::new(work, bytes);
        assert!(matches!(
            LoweredFunction::lower(&module, 0, &mut budget),
            Err(Declined::Budget)
        ));
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let mut budget = Budget::new(100_000, 1024 * 1024);
    let lowered = LoweredFunction::lower(&module, 0, &mut budget).unwrap();
    let remaining = budget.remaining();
    let foreign = engine.compile(NUMERIC).unwrap();
    assert!(matches!(
        lowered.program.materialization(&foreign, 0),
        Err(Declined::Metadata)
    ));
    assert!(lowered.program.materialization(&module.clone(), 0).is_ok());
    assert_eq!(std::sync::Arc::strong_count(&module.data), 1);
    budget.reserve(remaining.0, remaining.1).unwrap();
    assert!(matches!(
        LoweredFunction::lower(&module, 0, &mut budget),
        Err(Declined::Budget)
    ));
    assert_eq!(budget.remaining(), (0, 0));
    let mut stack = vec![NumericType::I32, NumericType::I64];
    assert_eq!(tail(&stack, &[NumericType::I32]), Err(Declined::Metadata));
    assert_eq!(
        replace_shape(&mut stack, &[NumericType::I32], &[NumericType::I64], 1),
        Err(Declined::Metadata)
    );
    assert_eq!(stack, vec![NumericType::I32, NumericType::I64]);
    let mut corrupt = engine.compile(MULTIVALUE).unwrap();
    std::sync::Arc::get_mut(&mut corrupt.data).unwrap().control[0][3] = Some((0, None));
    let mut budget = Budget::new(100_000, 1024 * 1024);
    assert!(matches!(
        LoweredFunction::lower(&corrupt, 0, &mut budget),
        Err(Declined::Metadata)
    ));
    assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    for bytes in [
        include_bytes!("../fixtures/performance/p047/p047_mixed_externref_local.wasm").as_slice(),
        include_bytes!("../fixtures/performance/p047/p047_effects_prefix.wasm").as_slice(),
    ] {
        let module = engine.compile(bytes).unwrap();
        assert!(matches!(
            LoweredFunction::lower(&module, 0, &mut budget),
            Err(Declined::Unsupported)
        ));
        assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    }
}
