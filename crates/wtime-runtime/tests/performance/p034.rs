use wtime_runtime::*;
const HIGH: [&[u8]; 3] = [
    include_bytes!("../fixtures/performance/gc/packed_gc_types_16.wasm"),
    include_bytes!("../fixtures/performance/gc/packed_gc_types_256.wasm"),
    include_bytes!("../fixtures/performance/gc/packed_gc_types_1024.wasm"),
];
const EQUAL_FIELDS: &[u8] = include_bytes!("../fixtures/performance/gc/equal_heap_fields_256.wasm");
const SEED: &[u8] = include_bytes!("../fixtures/performance/gc/canonical_seed.wasm");
fn engine() -> Engine {
    Engine::new(Config { features: FeatureSet::all_phase3_core(), ..Config::default() }).unwrap()
}
fn loaded(e: &Engine, bytes: &[u8]) -> (Store, Instance) {
    let module = e.compile(bytes).unwrap();
    let shared = module.clone();
    let mut store = Store::new(e);
    let instance = Linker::new(e).instantiate(&mut store, &shared).unwrap();
    drop(module);
    drop(shared);
    (store, instance)
}
fn invoke(store: &mut Store, instance: Instance, export: &str, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(f) = instance.export(store, export).unwrap() else { panic!("wrong export kind") };
    store.invoke(f, args)
}
fn check(store: &mut Store, instance: Instance, export: &str, args: &[Value], answer: i32, objects: usize) {
    let RunOutcome::Returned(values) = invoke(store, instance, export, args).unwrap() else { panic!("wrong outcome") };
    assert_eq!(values, vec![Value::I32(answer)]);
    store.release_published(&values);
    let stats = store.collect().unwrap();
    assert_eq!((stats.live, stats.reclaimed), (0, objects));
    assert_eq!(store.gc_payload_bytes(), 0);
}
#[test]
fn p034_high_local_array_and_struct_indices_keep_packed_field_semantics() {
    for bytes in HIGH {
        for name in ["run", "run_struct"] {
            for n in [0, 17] {
                let e = engine(); let (mut s, i) = loaded(&e, bytes);
                check(&mut s, i, name, &[Value::I32(n)], 255*n, 1);
            }
        }
    }
}
#[test]
fn p034_equal_heap_type_fields_keep_high_module_local_reference_context() {
    let e = engine(); let (mut s, i) = loaded(&e, EQUAL_FIELDS);
    // Two references alias the same leaf, not two leaf allocations. Both fields
    // have the same high local heap type; values 7+7 and2 total objects are independent.
    check(&mut s, i, "run", &[], 14, 2);
}
#[test]
fn p034_prior_engine_canonical_history_does_not_replace_module_local_indices() {
    let fresh = engine(); let seeded = engine();
    // Retain64 distinct structural identities in the second Engine before loading
    // the same module. Public API tests behavior, not the private numeric IDs.
    let retained_seed = seeded.compile(SEED).unwrap();
    for e in [&fresh, &seeded] {
        let (mut s, i) = loaded(e, HIGH[2]);
        check(&mut s, i, "run", &[Value::I32(17)], 4335, 1);
        check(&mut s, i, "run_struct", &[Value::I32(17)], 4335, 1);
        let (mut s, i) = loaded(e, EQUAL_FIELDS);
        check(&mut s, i, "run", &[], 14, 2);
    }
    drop(retained_seed);
}
#[test]
fn p034_layout_preparation_preserves_gc_resource_failure_without_live_objects() {
    let mut config = Config { features: FeatureSet::all_phase3_core(), ..Config::default() };
    config.limits.max_gc_bytes = 0;
    let e = Engine::new(config).unwrap();
    let (mut s, i) = loaded(&e, HIGH[1]);
    // Even n=0 allocates one setup object; runtime budget must still reject it.
    for name in ["run", "run_struct"] {
        assert_eq!(invoke(&mut s, i, name, &[Value::I32(0)]).unwrap_err().kind, ErrorKind::ResourceLimit);
        let stats = s.collect().unwrap();
        assert_eq!((stats.live, stats.reclaimed), (0, 0));
        assert_eq!(s.gc_payload_bytes(), 0);
    }
    // This is GC allocation failure, NOT forced optional-layout absence.
}
