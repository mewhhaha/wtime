fn movement(from: u32, to: u32, ty: NumericType) -> EdgeMove {
    EdgeMove { from, to, ty }
}
fn prepared(requests: &[EdgeMove], slots: u32) -> MovePlan {
    let mut budget = Budget::new(100_000, 1024 * 1024);
    let plan = MovePlan::prepare(requests, slots, &mut budget).unwrap();
    let (work, bytes) = plan.costs();
    assert_eq!(budget.remaining(), (100_000 - work, 1024 * 1024 - bytes));
    assert!(plan.scratch_slots() <= 1);
    plan
}
fn applied(plan: &MovePlan, values: &[(NumericType, u64)]) -> Vec<(NumericType, u64)> {
    let mut out = values.to_vec();
    let mut scratch = None;
    for step in plan.steps() {
        match *step {
            MoveStep::Save { from, ty } => {
                assert_eq!(out[from as usize].0, ty);
                scratch = Some(out[from as usize]);
            }
            MoveStep::Copy { from, to, ty } => {
                let value = match from {
                    MoveSource::Slot(slot) => out[slot as usize],
                    MoveSource::Scratch => scratch.expect("initialized scratch"),
                };
                assert_eq!(value.0, ty);
                out[to as usize] = value;
            }
        }
    }
    out
}
fn simultaneous(requests: &[EdgeMove], values: &[(NumericType, u64)]) -> Vec<(NumericType, u64)> {
    let mut out = values.to_vec();
    for request in requests {
        out[request.to as usize] = values[request.from as usize];
    }
    out
}
#[test]
fn p130_identity_and_acyclic_moves_preserve_simultaneous_results() {
    for requests in [vec![], vec![movement(0, 0, NumericType::I32)]] {
        let plan = prepared(&requests, 1);
        assert!(plan.steps().is_empty());
        assert_eq!(plan.scratch_slots(), 0);
    }
    let requests = [
        movement(0, 1, NumericType::I32),
        movement(1, 2, NumericType::I32),
        movement(0, 3, NumericType::I32),
    ];
    let values = [
        (NumericType::I32, 10),
        (NumericType::I32, 20),
        (NumericType::I32, 30),
        (NumericType::I32, 40),
    ];
    let plan = prepared(&requests, 4);
    assert_eq!(applied(&plan, &values), simultaneous(&requests, &values));
    assert_eq!(plan.scratch_slots(), 0);
    let reversed: Vec<_> = requests.iter().rev().copied().collect();
    assert_eq!(prepared(&reversed, 4).steps(), plan.steps());
    // Wide indices are checked as u32, never truncated to a small slot width.
    let wide = prepared(
        &[movement(u32::MAX - 2, u32::MAX - 1, NumericType::F64)],
        u32::MAX,
    );
    assert_eq!(
        wide.steps(),
        &[MoveStep::Copy {
            from: MoveSource::Slot(u32::MAX - 2),
            to: u32::MAX - 1,
            ty: NumericType::F64
        }]
    );
}
#[test]
fn p130_two_and_three_slot_cycles_use_bounded_scratch() {
    for requests in [
        vec![
            movement(0, 1, NumericType::I32),
            movement(1, 0, NumericType::I32),
        ],
        vec![
            movement(0, 1, NumericType::I32),
            movement(1, 2, NumericType::I32),
            movement(2, 0, NumericType::I32),
        ],
    ] {
        let plan = prepared(&requests, 3);
        let values = [
            (NumericType::I32, 10),
            (NumericType::I32, 20),
            (NumericType::I32, 30),
        ];
        assert_eq!(applied(&plan, &values), simultaneous(&requests, &values));
        assert_eq!(plan.scratch_slots(), 1);
        assert_eq!(plan.steps().len(), requests.len() + 1);
        assert_eq!(
            plan.steps()
                .iter()
                .filter(|step| matches!(step, MoveStep::Save { .. }))
                .count(),
            1
        );
    }
    // One scratch value is reused across disconnected cycles of different types.
    let requests = [
        movement(0, 1, NumericType::F32),
        movement(1, 0, NumericType::F32),
        movement(2, 3, NumericType::F64),
        movement(3, 2, NumericType::F64),
    ];
    let values = [
        (NumericType::F32, 0x80000000),
        (NumericType::F32, 0x7fc12345),
        (NumericType::F64, 0x8000000000000000),
        (NumericType::F64, 0x7ff8000000001234),
    ];
    let plan = prepared(&requests, 4);
    assert_eq!(applied(&plan, &values), simultaneous(&requests, &values));
    assert_eq!(plan.scratch_slots(), 1);
    assert_eq!(
        plan.steps()
            .iter()
            .filter(|step| matches!(step, MoveStep::Save { .. }))
            .count(),
        2
    );
    // Fanout must read the old cycle value before the rotation overwrites it.
    let requests = [
        movement(0, 1, NumericType::I64),
        movement(1, 0, NumericType::I64),
        movement(0, 2, NumericType::I64),
    ];
    let values = [
        (NumericType::I64, 11),
        (NumericType::I64, 22),
        (NumericType::I64, 33),
    ];
    assert_eq!(
        applied(&prepared(&requests, 3), &values),
        simultaneous(&requests, &values)
    );
    let requests = [
        movement(0, 1, NumericType::I32),
        movement(1, 0, NumericType::I64),
    ];
    let values = [
        (NumericType::I32, 11),
        (NumericType::I64, 0x8000000000000022),
    ];
    assert_eq!(
        applied(&prepared(&requests, 2), &values),
        simultaneous(&requests, &values)
    );
    // Real P058 branch results shift i32/i64 values into earlier temporary
    // heights. Height 1 changes from i32 to i64 after its old value is read.
    let module = crate::Engine::new(crate::Config::default())
        .unwrap()
        .compile(include_bytes!(
            "../fixtures/performance/p058/edge_values.wasm"
        ))
        .unwrap();
    let mut budget = Budget::new(100_000, 1024 * 1024);
    let lowered = super::super::lower::LoweredFunction::lower(&module, 0, &mut budget).unwrap();
    let requests = &lowered.edge(4, 6).unwrap().moves;
    let plan = MovePlan::prepare(requests, 3, &mut budget).unwrap();
    let values = [
        (NumericType::I32, 99),
        (NumericType::I32, 7),
        (NumericType::I64, 9),
    ];
    assert_eq!(applied(&plan, &values), simultaneous(requests, &values));
    assert_eq!(
        applied(&plan, &values)[..2],
        [(NumericType::I32, 7), (NumericType::I64, 9)]
    );
}
#[test]
fn p130_invalid_requests_and_budget_failures_publish_nothing() {
    for requests in [
        vec![movement(3, 0, NumericType::I32)],
        vec![movement(0, 3, NumericType::I32)],
        vec![
            movement(0, 1, NumericType::I32),
            movement(2, 1, NumericType::I32),
        ],
        vec![
            movement(0, 1, NumericType::I32),
            movement(1, 0, NumericType::I64),
            movement(0, 2, NumericType::F64),
        ],
        vec![
            movement(0, 1, NumericType::I32),
            movement(0, 2, NumericType::F32),
        ],
        vec![
            movement(0, 0, NumericType::I32),
            movement(0, 1, NumericType::I64),
        ],
    ] {
        let mut budget = Budget::new(100_000, 1024 * 1024);
        assert!(matches!(
            MovePlan::prepare(&requests, 3, &mut budget),
            Err(Declined::Metadata)
        ));
        assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    }
    let requests = [
        movement(0, 1, NumericType::I32),
        movement(1, 0, NumericType::I32),
    ];
    let plan = prepared(&requests, 2);
    for (work, bytes) in [
        (0, 1024 * 1024),
        (100_000, 0),
        (plan.costs().0 - 1, 1024 * 1024),
        (100_000, plan.costs().1),
    ] {
        let mut budget = Budget::new(work, bytes);
        assert!(matches!(
            MovePlan::prepare(&requests, 2, &mut budget),
            Err(Declined::Budget)
        ));
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let oversized: Vec<_> = (0..128)
        .map(|slot| movement(slot, (slot + 1) % 128, NumericType::I32))
        .collect();
    let mut budget = Budget::new(100_000, 1024 * 1024);
    assert!(matches!(
        MovePlan::prepare(&oversized, 128, &mut budget),
        Err(Declined::Budget)
    ));
    assert_eq!(budget.remaining(), (100_000, 1024 * 1024));
    let mut shared = Budget::new(plan.costs().0, 1024 * 1024);
    MovePlan::prepare(&requests, 2, &mut shared).unwrap();
    let before = shared.remaining();
    assert!(matches!(
        MovePlan::prepare(&requests, 2, &mut shared),
        Err(Declined::Budget)
    ));
    assert_eq!(shared.remaining(), before);
    assert_eq!(mul(usize::MAX, 2), Err(Declined::Budget));
    assert_eq!(add(usize::MAX, 1), Err(Declined::Budget));
}
#[test]
fn p130_bounded_generated_graphs_match_simultaneous_oracle() {
    let mut seed = 0x123456789abcdef0u64;
    let mut steps = 0;
    let mut cycles = 0;
    let mut work = 0;
    let mut bytes = 0;
    for case in 0..4096 {
        let count = 2 + case % 7;
        let mut requests = vec![];
        for to in 0..count {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            requests.push(movement(
                (seed >> 32) as u32 % count as u32,
                to as u32,
                NumericType::I32,
            ));
        }
        let values: Vec<_> = (0..count)
            .map(|slot| (NumericType::I32, ((case as u64) << 32) | slot as u64))
            .collect();
        let plan = prepared(&requests, count as u32);
        assert_eq!(
            applied(&plan, &values),
            simultaneous(&requests, &values),
            "graph {case}"
        );
        requests.reverse();
        assert_eq!(prepared(&requests, count as u32).steps(), plan.steps());
        steps += plan.steps().len();
        cycles += plan
            .steps()
            .iter()
            .filter(|step| matches!(step, MoveStep::Save { .. }))
            .count();
        work += plan.costs().0;
        bytes += plan.costs().1;
    }
    assert!(steps > 0 && cycles > 0 && work > 0 && bytes > 0);
    eprintln!(
        "P130: 4096 generated move graphs; {steps} scheduled steps; {cycles} scratch cycles; {work} charged work; {bytes} retained bytes"
    );
}
