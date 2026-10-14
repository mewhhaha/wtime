// Source-only P043 adversarial checks, included inside ir::memory.
const WIDTHS: &[u8] = include_bytes!("../fixtures/performance/p043/scalar-widths.wasm");
fn prepared() -> (Store, Module, Instance) {
    let e = Engine::with_execution_policy(Config {
        features: FeatureSet::all_phase3_core(), ..Config::default()
    }, ExecutionPolicy::Compact).unwrap();
    let module = e.compile(WIDTHS).unwrap(); let mut store = Store::new(&e);
    let instance = Linker::new(&e).instantiate(&mut store, &module).unwrap();
    (store, module, instance)
}
fn function(module: &Module) -> CompactFunction {
    CompactFunction::from_module(module,0,MAX_WORK,MAX_BYTES).unwrap()
}
fn accesses(function: &CompactFunction, module: &Module, types: &MemoryTypes) -> MemoryAccesses {
    MemoryAccesses::prepare(function,module,types,MAX_WORK,MAX_BYTES).unwrap().unwrap()
}

#[test]
fn p043_private_preparation_work_bytes_and_binding_peak_are_bounded() {
    let (s,m,i)=prepared();let inst=&s.instances[i.0.index];let f=function(&m);
    let types=MemoryTypes::prepare(&m,MAX_WORK,MAX_BYTES).unwrap();
    assert!(MemoryTypes::prepare(&m,0,MAX_BYTES).is_none());
    assert!(MemoryTypes::prepare(&m,MAX_WORK,types.bytes-1).is_none());
    let map=accesses(&f,&m,&types);
    assert_eq!(map.entries.iter().flatten().count(),14);
    assert!(matches!(MemoryAccesses::prepare(&f,&m,&types,map.work-1,MAX_BYTES),Err(Declined::Budget)));
    assert!(matches!(MemoryAccesses::prepare(&f,&m,&types,MAX_WORK,map.bytes-1),Err(Declined::Budget)));
    assert!(!map.verify(&f,&m,&types,map.work-1,MAX_BYTES));
    assert!(!map.verify(&f,&m,&types,MAX_WORK,map.bytes-1));
    let binding=MemoryBindings::prepare(&s,&m,&inst.mems).unwrap();
    assert!(MemoryBindings::bounded(&s,&m,&inst.mems,binding.work-1,MAX_BYTES).is_none());
    assert!(MemoryBindings::bounded(&s,&m,&inst.mems,MAX_WORK,binding.bytes).is_none());
    assert!(MemoryBindings::prepare(&s,&m,&[]).is_none());
    assert!(map.bytes<=MAX_BYTES && map.work<=MAX_WORK && binding.bytes<=MAX_BYTES && binding.work<=MAX_WORK);
}

#[test]
fn p043_private_operand_source_width_offset_and_type_corruption_fail_closed() {
    let (_,m,_)=prepared();let f=function(&m);let types=MemoryTypes::prepare(&m,MAX_WORK,MAX_BYTES).unwrap();
    for damage in 0..10 {
        let mut map=accesses(&f,&m,&types);
        match damage {
            0=>map.entries[1].as_mut().unwrap().width=8,
            1=>map.entries[1].as_mut().unwrap().offset=u64::MAX,
            2=>map.entries[1].as_mut().unwrap().memory=1,
            3=>map.entries[1].as_mut().unwrap().kind=ScalarKind::I32U8,
            4=>map.entries[1].as_mut().unwrap().index_type=IndexType::I64,
            5=>map.entries[1]=None,
            6=>map.entries[0]=map.entries[1],
            7=>map.work+=1,
            8=>map.bytes+=1,
            _=>{map.entries.pop();},
        }
        assert!(!map.verify(&f,&m,&types,MAX_WORK,MAX_BYTES),"damage={damage}");
    }
    let map=accesses(&f,&m,&types);let mut wrong_source=function(&m);
    wrong_source.source=Arc::from(wrong_source.source.as_ref());
    assert!(!map.verify(&wrong_source,&m,&types,MAX_WORK,MAX_BYTES));
    let mut wrong_opcode=function(&m);wrong_opcode.ops[1].opcode=0x2d;
    assert!(!map.verify(&wrong_opcode,&m,&types,MAX_WORK,MAX_BYTES));
    let mut wrong_types=MemoryTypes::prepare(&m,MAX_WORK,MAX_BYTES).unwrap();wrong_types.entries[0].index_type=IndexType::I64;
    assert!(!map.verify(&f,&m,&wrong_types,MAX_WORK,MAX_BYTES));
    let another=m.clone();
    assert!(map.verify(&f,&another,&types,MAX_WORK,MAX_BYTES));
    let copy=Engine::new(Config{features:FeatureSet::all_phase3_core(),..Config::default()}).unwrap().compile(WIDTHS).unwrap();
    assert!(!map.verify(&f,&copy,&types,MAX_WORK,MAX_BYTES));
}

#[test]
fn p043_private_binding_identity_generations_classes_and_unshared_growth_are_live() {
    let (mut s,m,i)=prepared();let f=function(&m);let types=MemoryTypes::prepare(&m,MAX_WORK,MAX_BYTES).unwrap();
    let op=accesses(&f,&m,&types).get(1).unwrap();let inst=s.instances[i.0.index].clone();
    let plan=MemoryBindings::prepare(&s,&m,&inst.mems).unwrap();
    assert_eq!(plan.resolve(&s,&inst,op).unwrap(),inst.mems[0]);
    let other=Store::new(&s.engine);assert!(plan.resolve(&other,&inst,op).is_err());
    let mut copy=inst.clone();copy.module=s.engine.compile(WIDTHS).unwrap();
    assert!(plan.resolve(&s,&copy,op).is_err());
    for damage in 0..6 {
        let mut binding=MemoryBindings::prepare(&s,&m,&inst.mems).unwrap();
        match damage {
            0=>binding.entries[0].index_type=IndexType::I64,
            1=>binding.entries[0].shared=true,
            2=>binding.entries[0].page_size_log2=0,
            3=>binding.bytes+=1,
            4=>binding.work+=1,
            _=>binding.source=Arc::from(binding.source.as_ref()),
        }
        assert!(binding.resolve(&s,&inst,op).is_err(),"binding damage={damage}");
    }
    let at_growth=MemoryOp{kind:ScalarKind::I32U8,width:1,memory:0,offset:65536,index_type:IndexType::I32};
    let mut stack=vec![Value::I32(0)];
    assert_eq!(s.compact_memory_access(i.0.index,at_growth,&mut stack).unwrap_err().trap_code,Some(TrapCode::MemoryOutOfBounds));
    assert_eq!(s.compact_memory_accesses,0);
    let memory=inst.mems[0];let live=s.check(memory.0,&s.mems).unwrap();
    s.mems[live].bytes.resize(65537,0);s.mems[live].bytes[65536]=91;
    let mut stack=vec![Value::I32(0)];s.compact_memory_access(i.0.index,at_growth,&mut stack).unwrap();
    assert_eq!(stack,[Value::I32(91)]);assert_eq!(s.compact_memory_accesses,1);
    let removed=s.mems.remove(memory.0.index,memory.0.generation).unwrap();
    assert_eq!(plan.resolve(&s,&inst,op).unwrap_err().kind,ErrorKind::Host);
    let (index,generation)=s.mems.insert(removed);assert_eq!(index,memory.0.index);assert_ne!(generation,memory.0.generation);
    assert_eq!(plan.resolve(&s,&inst,op).unwrap_err().kind,ErrorKind::Host);
    // Atomics are allowed on unshared memory and promote its backing to the
    // synchronized representation. Static Wasm type and local handle persist.
    let (mut promoted,_,pi)=prepared();
    let instruction=Instruction{opcode:0xfe12,immediate:Immediate::MemArg{align:0,offset:0,memory:0},span:Span{start:0,end:0}};
    let mut stack=vec![Value::I32(0)];
    promoted.atomic(pi.0.index,&instruction,&mut stack).unwrap();
    assert_eq!(stack,[Value::I32(128)]);
    let pm=promoted.instances[pi.0.index].mems[0];
    assert!(!promoted.mems[pm.0.index].ty.shared && promoted.mems[pm.0.index].shared.is_some());
    let mut stack=vec![Value::I32(0)];promoted.compact_memory_access(pi.0.index,op,&mut stack).unwrap();
    assert_eq!(stack,[Value::I32(-128)]);
    let write=MemoryOp{kind:ScalarKind::Store32U8,width:1,memory:0,offset:7,index_type:IndexType::I32};
    let mut stack=vec![Value::I32(0),Value::I32(91)];
    promoted.compact_memory_access(pi.0.index,write,&mut stack).unwrap();
    assert_eq!(promoted.read_memory(pm,7,1).unwrap(),[91]);
    assert_eq!(promoted.compact_memory_accesses,2);
}
