fn p036_module(bytes: &[u8]) -> (Engine, Module) {
    let mut config = Config::default();
    config.features = FeatureSet::all_phase3_core();
    config.limits.max_locals = 70_001;
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(bytes).unwrap();
    (engine, module)
}

#[test]
fn p036_measured_record_sizes_and_lossless_wide_inline_fields() {
    eprintln!("Instruction={} Immediate={} Value={} CompactOp={} SideImmediate={} SourceOrigin={}",
        std::mem::size_of::<wtime_validate::Instruction>(), std::mem::size_of::<Immediate>(),
        std::mem::size_of::<Value>(), std::mem::size_of::<CompactOp>(),
        std::mem::size_of::<SideImmediate>(), std::mem::size_of::<SourceOrigin>());
    assert_eq!(std::mem::size_of::<CompactOp>(), 16);
    let (engine, module) = p036_module(include_bytes!("../fixtures/performance/plans/wide_local.wasm"));
    assert_eq!(engine.policy, ExecutionPolicy::Decoded);
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    assert_eq!(compact.ops.iter().filter(|op| matches!(op.opcode, 0x20 | 0x21) && op.form == INDEX && op.aux == 70_000).count(), 2);
    for immediate in [Immediate::Index(u32::MAX), Immediate::Pair(u32::MAX, u32::MAX),
                      Immediate::I32(i32::MIN), Immediate::I64(i64::MIN),
                      Immediate::F32(0x7fc1_2345), Immediate::F64(0xfff8_1234_5678_9abc), Immediate::Block(i64::MAX)] {
        let op = inline(0x01, &immediate).unwrap();
        match immediate {
            Immediate::Index(value) => assert_eq!(op.aux, value),
            Immediate::Pair(a, b) => assert_eq!((op.aux, op.payload), (a, u64::from(b))),
            Immediate::I32(value) => assert_eq!(op.payload, u64::from(value as u32)),
            Immediate::I64(value) | Immediate::Block(value) => assert_eq!(op.payload as i64, value),
            Immediate::F32(value) => assert_eq!(op.payload, u64::from(value)),
            Immediate::F64(value) => assert_eq!(op.payload, value),
            _ => unreachable!(),
        }
    }
}

#[test]
fn p036_validated_memory64_side_keeps_maximum_offset() {
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/p025/memory64_boundary.wasm"));
    let compact = CompactFunction::from_module(&module, 1, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    assert!(compact.sides.iter().any(|side| matches!(side, SideImmediate::Memory { align: 2, offset: u64::MAX, memory: 0 })));
    assert!(compact.ops.iter().any(|op| op.opcode == 0x28 && op.form == SIDE));
}

// Original project-licensed test generator. The ordinary validator admits the
// binary before the private factory can inspect its large branch vector.
fn p036_branch_table_bytes(target_count: u32) -> Vec<u8> {
    fn leb(mut value: u32, bytes: &mut Vec<u8>) {
        loop {
            let part = (value & 0x7f) as u8;
            value >>= 7;
            bytes.push(part | if value == 0 { 0 } else { 0x80 });
            if value == 0 { break; }
        }
    }
    let mut module = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0".to_vec();
    let mut body = vec![0, 2, 0x40, 0x41, 0, 0x0e];
    leb(target_count, &mut body);
    body.extend(std::iter::repeat_n(0, target_count as usize));
    body.extend([0, 0x0b, 0x0b]); // default depth, block end, function end
    let mut code = vec![1];
    leb(body.len() as u32, &mut code);
    code.extend(body);
    module.push(0x0a);
    leb(code.len() as u32, &mut module);
    module.extend(code);
    module
}

#[test]
fn p036_branch_and_catch_tables_round_trip_from_admitted_modules() {
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/borrow/vector_immediates.wasm"));
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    assert!(compact.sides.iter().any(|side| matches!(side, SideImmediate::Branches { targets, default: 0 } if targets == &vec![0; 32])));
    let (_, large) = p036_module(&p036_branch_table_bytes(10_000));
    let admitted = CompactFunction::from_module(&large, 0, MAX_WORK, MAX_BYTES).unwrap();
    assert!(admitted.verify(&large));
    assert!(admitted.sides.iter().any(|side| matches!(side,
        SideImmediate::Branches { targets, default: 0 } if targets.len() == 10_000 && targets.iter().all(|target| *target == 0))));
    assert!(matches!(CompactFunction::from_module(&large, 0, admitted.work - 1, MAX_BYTES), Err(Declined::Budget)));
    assert!(matches!(CompactFunction::from_module(&large, 0, MAX_WORK, admitted.retained_bytes - 1), Err(Declined::Budget)));
    let (_, oversized) = p036_module(&p036_branch_table_bytes(40_000));
    assert!(matches!(CompactFunction::from_module(&oversized, 0, usize::MAX, usize::MAX), Err(Declined::Budget)));
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/branch/try_table_normal.wasm"));
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    let catches = compact.sides.iter().find_map(|side| match side { SideImmediate::Catches { block: BlockImmediate::Signed(-64), catches } => Some(catches), _ => None }).unwrap();
    assert_eq!(catches.len(), 3);
    for (index, catch) in catches.iter().enumerate() {
        assert_eq!((catch.kind, catch.tag, catch.label), (0, Some(index as u32), 0));
    }
    assert!(matches!(CompactFunction::from_module(&module, 0, 1, usize::MAX), Err(Declined::Budget)));
    assert!(matches!(CompactFunction::from_module(&module, 0, usize::MAX, compact.retained_bytes - 1), Err(Declined::Budget)));
}

#[test]
fn p036_wide_compound_storage_preserves_bits_and_declines_unknown_shapes() {
    // Private storage helpers do not grant executable admission. Their varied
    // values complement the validated-module constructor cases above.
    let cases = [
        Immediate::Bytes((0..16).map(|byte| byte * 17).collect()),
        Immediate::Types(vec![ValType::I32, ValType::V128, ValType::Ref(RefType::new(false, HeapType::ExactDefined(TypeIdx(u32::MAX))))].into_boxed_slice()),
        Immediate::Resume(Box::new(wtime_validate::ResumeImmediate { ty: u32::MAX, tag: Some(u32::MAX), handlers: vec![(u32::MAX, Some(u32::MAX)), (1, None)] })),
        Immediate::Cast { flags: 3, label: u32::MAX, from: HeapType::ExactDefined(TypeIdx(u32::MAX)), to: HeapType::Defined(TypeIdx(70_000)) },
        Immediate::MemLane { align: u32::MAX, offset: u64::MAX, memory: u32::MAX, lane: 15 },
        Immediate::Heap(HeapType::ExactDefined(TypeIdx(u32::MAX))),
    ];
    for immediate in cases {
        assert!(inline(0x01, &immediate).is_none());
        let side = SideImmediate::from_immediate(&immediate).unwrap();
        assert!(side.matches(&immediate));
        assert!(side.dynamic_bytes().is_some());
    }
    assert!(matches!(SideImmediate::from_immediate(&Immediate::Bytes(vec![0; 17].into_boxed_slice())), Err(Declined::Unsupported)));
    assert!(matches!(SideImmediate::from_immediate(&Immediate::TryTable(Box::new(wtime_validate::TryTableImmediate { block: Immediate::I32(0), catches: vec![] }))), Err(Declined::Unsupported)));
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/simd/lane_loads_oob.wasm"));
    for (body, lane) in [(0, 5), (1, 3), (2, 2), (3, 1)] {
        let function = CompactFunction::from_module(&module, body, MAX_WORK, MAX_BYTES).unwrap();
        assert!(function.verify(&module));
        assert!(function.sides.iter().any(|side| matches!(side,
            SideImmediate::Lane { offset: 0, memory: 0, lane: actual, .. } if *actual == lane)));
    }
}

#[test]
fn p036_verifier_rejects_corrupt_tags_side_indices_events_work_and_ownership() {
    let bytes = include_bytes!("../fixtures/performance/p025/memory64_boundary.wasm");
    let (engine, module) = p036_module(bytes);
    let mut compact = CompactFunction::from_module(&module, 1, 100_000, 1024 * 1024).unwrap();
    let index = compact.ops.iter().position(|op| op.form == SIDE).unwrap();
    let original = compact.ops[index];
    compact.ops[index].aux = u32::MAX;
    assert!(!compact.verify(&module));
    compact.ops[index] = original;
    compact.ops[index].form = u16::MAX;
    assert!(!compact.verify(&module));
    compact.ops[index] = original;
    compact.ops[index].opcode ^= 1;
    assert!(!compact.verify(&module));
    compact.ops[index] = original;
    let origin = compact.origins[index];
    compact.origins[index].pc = usize::MAX;
    assert!(!compact.verify(&module));
    compact.origins[index] = origin;
    compact.origins[index].effect = classify(0xffff);
    assert!(!compact.verify(&module));
    compact.origins[index] = origin;
    compact.origins[index].fuel = 0;
    assert!(!compact.verify(&module));
    compact.origins[index] = origin;
    compact.origins[index].checkpoint = false;
    assert!(!compact.verify(&module));
    compact.origins[index] = origin;
    let work = compact.work;
    compact.work = 0;
    assert!(!compact.verify(&module));
    compact.work = work;
    compact.retained_bytes += 1;
    assert!(!compact.verify(&module));
    compact.retained_bytes -= 1;
    assert!(compact.verify(&module.clone()));
    assert!(!compact.verify(&engine.compile(bytes).unwrap()));
    let (_, foreign) = p036_module(bytes);
    assert!(!compact.verify(&foreign));
}

#[test]
fn p036_budget_decline_drops_partial_metadata_and_leaves_decoded_fallback() {
    let (engine, module) = p036_module(include_bytes!("../fixtures/performance/cursor/multivalue_branch.wasm"));
    let source_count = Arc::strong_count(&module.data.inner.decoded().bytes);
    assert!(matches!(CompactFunction::from_module(&module, 0, 0, usize::MAX), Err(Declined::Budget)));
    assert!(matches!(CompactFunction::from_module(&module, 0, usize::MAX, 0), Err(Declined::Budget)));
    assert!(matches!(CompactFunction::from_module(&module, usize::MAX, usize::MAX, usize::MAX), Err(Declined::Plan)));
    assert_eq!(Arc::strong_count(&module.data.inner.decoded().bytes), source_count);
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert_eq!(Arc::strong_count(&module.data.inner.decoded().bytes), source_count + 1);
    drop(compact);
    assert_eq!(Arc::strong_count(&module.data.inner.decoded().bytes), source_count);
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else { panic!() };
    assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)]));
}

// These cases exercise admitted-module construction, not execution.

#[test]
fn p036_validated_factory_admits_typed_select_types_side() {
    // Defined body 1 is `select (result i32)` in vector_immediates.wat.
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/borrow/vector_immediates.wasm"));
    let compact = CompactFunction::from_module(&module, 1, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    let op = compact.ops.iter().find(|op| op.opcode == 0x1c).unwrap();
    assert_eq!(op.form, SIDE);
    assert!(matches!(compact.sides.get(op.aux as usize), Some(SideImmediate::Types(types)) if types == &vec![ValType::I32]));
}

#[test]
fn p036_validated_factory_admits_nonempty_resume_handlers() {
    // P011's source-matched encoded artifact; body 1 is `run` and contains
    // `resume $worker_cont (on $yield $paused ...)`.
    let bytes = include_bytes!("../fixtures/performance/compact/continuation_root.wasm");
    let (_, module) = p036_module(bytes);
    let compact = CompactFunction::from_module(&module, 1, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    let op = compact.ops.iter().find(|op| op.opcode == 0xe3).unwrap();
    assert_eq!(op.form, SIDE);
    assert!(matches!(compact.sides.get(op.aux as usize),
        Some(SideImmediate::Resume { ty: 3, tag: None, handlers }) if handlers == &vec![(0, Some(0))]));
}

#[test]
fn p036_validated_factory_admits_nonzero_v128_constants_and_shuffle_bytes() {
    // P026/P010 source-matched shuffle artifact, body 0. The three byte
    // vectors are the two v128.const payloads and the shuffle lane selectors.
    let bytes = include_bytes!("../fixtures/performance/compact/shuffle.wasm");
    let (_, module) = p036_module(bytes);
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    let vectors: Vec<[u8; 16]> = compact.sides.iter().filter_map(|side| match side {
        SideImmediate::Vector(bytes) => Some(*bytes),
        _ => None,
    }).collect();
    assert_eq!(vectors, vec![
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31],
        [31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16],
    ]);
    assert_eq!(compact.ops.iter().filter(|op| matches!(op.opcode, 0xfd0c | 0xfd0d) && op.form == SIDE).count(), 3);
}

#[test]
fn p036_validated_factory_admits_heap_side_from_ref_null() {
    let (_, module) = p036_module(include_bytes!("../fixtures/feature_profiles/reference_types.wasm"));
    let compact = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(compact.verify(&module));
    let op = compact.ops.iter().find(|op| op.opcode == 0xd0).unwrap();
    assert_eq!(op.form, SIDE);
    assert!(matches!(compact.sides.get(op.aux as usize), Some(SideImmediate::Heap(HeapType::Func))));
}

#[test]
fn p036_constructor_hard_work_limit_cannot_be_overridden() {
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/compact/long_metadata.wasm"));
    assert!(module.data.plans.is_some());
    assert!(matches!(CompactFunction::from_module(&module, 0, usize::MAX, usize::MAX), Err(Declined::Budget)));
    let (engine, module) = p036_module(include_bytes!("../fixtures/performance/compact/long_metadata.wasm"));
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else { panic!() };
    assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
}

#[test]
fn p036_actual_capacity_boundary_and_resume_payload_remain_transactional() {
    let (_, module) = p036_module(include_bytes!("../fixtures/performance/compact/continuation_root.wasm"));
    let compact = CompactFunction::from_module(&module, 1, MAX_WORK, MAX_BYTES).unwrap();
    assert!(compact.sides.iter().any(|s| matches!(s, SideImmediate::Resume { handlers, .. } if !handlers.is_empty())));
    let admitted = CompactFunction::from_module(&module, 1, compact.work, compact.retained_bytes).unwrap();
    assert_eq!(admitted.retained_bytes, compact.retained_bytes);
    assert!(matches!(CompactFunction::from_module(&module, 1, compact.work - 1, MAX_BYTES), Err(Declined::Budget)));
    assert!(matches!(CompactFunction::from_module(&module, 1, MAX_WORK, compact.retained_bytes - 1), Err(Declined::Budget)));
    assert!(compact.verify(&module));
}

#[test]
fn p036_validated_factory_preserves_cast_branch_metadata() {
    let mut config = Config::default();
    config.features = FeatureSet::all_phase3_core();
    let engine = Engine::new(config).unwrap();
    let bytes = include_bytes!("../fixtures/performance/compact/cast_branch.wasm");
    let module = engine.compile(bytes).unwrap();
    let function = CompactFunction::from_module(&module, 0, 100_000, 1024 * 1024).unwrap();
    assert!(function.verify(&module));

    let source = module.data.inner.decoded().code.get(0).unwrap().instructions.iter()
        .find(|instruction| matches!(instruction.opcode, 0xfb18 | 0xfb19))
        .expect("fixture must contain br_on_cast");
    assert!(matches!(source.immediate, Immediate::Cast { .. }));
    let (index, op) = function.ops.iter().enumerate()
        .find(|(_, op)| matches!(op.opcode, 0xfb18 | 0xfb19)).unwrap();
    assert_eq!(op.form, SIDE);
    let side = function.sides.get(op.aux as usize).unwrap();
    assert!(matches!(side, SideImmediate::Cast { .. }));
    assert!(side.matches(&source.immediate));
    assert!(function.sides.iter().any(|side| matches!(side, SideImmediate::BlockType(ValType::Ref(_)))));
    assert_eq!(function.origins[index].pc, index);
    assert_eq!(function.origins[index].opcode, source.opcode);
}
