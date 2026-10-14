use super::*;
fn admitted() -> (Engine,Module) {
    let e=Engine::new(Config::default()).unwrap();let m=e.compile(include_bytes!("../fixtures/performance/cursor/store_successor.wasm")).unwrap();(e,m)
}
fn certificate(m: &Module) -> PreparedEffect {PreparedEffect::from_module(m,0,100,4096).unwrap()}
#[test]
fn p019_unknown_opcode_is_root_materialization_barrier() {
    let e=classify(0xffff);assert_eq!(e.family,EffectFamily::Barrier);assert!(e.roots && e.materialize && e.may_trap);
    let (_,m)=admitted();let p=certificate(&m);assert!(!p.numeric_region_eligible());
}
#[test]
fn p019_missing_and_reordered_source_effects_are_rejected() {
    let (_,m)=admitted();let mut p=certificate(&m);assert!(p.verify(&m,0));p.source[2].origin=None;assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source.swap(2,3);assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source.pop();assert!(!p.verify(&m,0));
}
#[test]
fn p019_store_trap_and_fuel_metadata_cannot_be_erased() {
    let (_,m)=admitted();let mut p=certificate(&m);p.source[2].effect=classify(0x41);assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source[4].materialization_point=false;assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source[1].fuel_charge=0;assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source[3].cancellation_checkpoint=false;assert!(!p.verify(&m,0));
}
#[test]
fn p019_wrong_control_target_and_fabricated_stack_bound_are_rejected() {
    let (_,m)=admitted();let mut p=certificate(&m);p.source[0].control_destinations=Some((1,None));assert!(!p.verify(&m,0));
    let mut p=certificate(&m);p.source[0].transient_stack_peak=Some(0);assert!(!p.verify(&m,0));
}
#[test]
fn p019_budget_decline_keeps_decoded_module_usable() {
    let (e,m)=admitted();assert!(metadata_bytes(usize::MAX).is_none());assert!(PreparedEffect::from_module(&m,0,1,4096).is_none());assert!(PreparedEffect::from_module(&m,0,100,1).is_none());
    let mut s=Store::new(&e);let i=Linker::new(&e).instantiate(&mut s,&m).unwrap();let External::Func(f)=i.export(&s,"run").unwrap() else {panic!()};assert_eq!(s.invoke(f,&[]).unwrap(),RunOutcome::Returned(vec![Value::I32(7)]));
}
