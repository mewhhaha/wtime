fn module() -> (Engine,Module) { let e=Engine::new(Config::default()).unwrap(); let m=e.compile(include_bytes!("../fixtures/performance/cursor/multivalue_branch.wasm")).unwrap(); (e,m) }
#[test]
fn p021_multivalue_nesting_and_peak_come_from_validation() {
    let (_,m)=module();let all=m.data.plans.as_ref().unwrap();let p=&all.functions[0];
    assert_eq!((p.body,p.type_index,p.local_count,p.stack_peak,p.maximum_nesting),(0,0,0,2,2));
    assert_eq!((p.labels[0].source_pc,p.labels[0].params,p.labels[0].results),(0,0,2));assert!(p.labels[0].handlers.is_empty());assert!(all.work>0);
}
#[test]
fn p021_wide_local_index_uses_full_validated_layout() {
    let mut config=Config::default();config.limits.max_locals=70_001;
    let e=Engine::new(config).unwrap();let m=e.compile(include_bytes!("../fixtures/performance/plans/wide_local.wasm")).unwrap();
    let p=&m.data.plans.as_ref().unwrap().functions[0];assert_eq!(p.local_count,70_001);assert_eq!(p.stack_peak,1);
    let mut s=Store::new(&e);let i=Linker::new(&e).instantiate(&mut s,&m).unwrap();let External::Func(f)=i.export(&s,"run").unwrap() else {panic!()};assert_eq!(s.invoke(f,&[]).unwrap(),RunOutcome::Returned(vec![Value::I32(42)]));
}
#[test]
fn p021_deep_multivalue_arity_and_retention_are_shared() {
    let e=Engine::new(Config::default()).unwrap();let m=e.compile(include_bytes!("../fixtures/performance/plans/nested_multivalue.wasm")).unwrap();let p=&m.data.plans.as_ref().unwrap().functions[0];
    assert_eq!(p.maximum_nesting,5);assert_eq!(p.stack_peak,2);assert_eq!(p.labels.len(),4);assert!(p.labels.iter().all(|l|l.params==0&&l.results==2));
    eprintln!("plan retained bytes: {}; analysis work: {}",m.function_plan_bytes(),m.data.plans.as_ref().unwrap().work);
    assert_eq!(slot_class(ValType::Ref(RefType::new(false,HeapType::ExactDefined(TypeIdx(70_000))))),SlotClass::Reference);assert_eq!(slot_class(ValType::V128),SlotClass::Vector);
}
#[test]
fn p021_budget_decline_is_transactional_and_decoded_remains_usable() {
    let (e,m)=module();assert!(FunctionPlans::bounded(&m.data.inner,0,usize::MAX).is_none());assert!(FunctionPlans::bounded(&m.data.inner,usize::MAX,0).is_none());
    let original=m.function_plan_bytes();let clone=m.clone();assert_eq!(clone.function_plan_bytes(),original);assert!(Arc::ptr_eq(&m.data,&clone.data));
    let mut s=Store::new(&e);let i=Linker::new(&e).instantiate(&mut s,&m).unwrap();let External::Func(f)=i.export(&s,"run").unwrap() else {panic!()};assert_eq!(s.invoke(f,&[]).unwrap(),RunOutcome::Returned(vec![Value::I32(7),Value::I64(9)]));
}

#[test]
fn p021_reference_layout_borrows_admitted_nominal_context() {
    let e=Engine::new(Config::default()).unwrap();let m=e.compile(include_bytes!("../fixtures/performance/cursor/root_after_restore.wasm")).unwrap();
    let p=&m.data.plans.as_ref().unwrap().functions[0];let layout=m.function_layout(p.body).unwrap();
    assert!(layout.locals.iter().any(|t|slot_class(*t)==SlotClass::Reference));
    assert_eq!(layout.results,&[ValType::I32]);assert!(layout.params.is_empty());
    assert!(std::ptr::eq(layout.locals.as_ptr(),m.data.inner.decoded().code[0].locals.as_ptr()));
    assert!(m.source_effect(p.body,0).is_some());
    let (_,nested)=module();let p=&nested.data.plans.as_ref().unwrap().functions[0];assert!(nested.control_destination(p.body,0).is_some());
}
