use wtime_runtime::*;

const TABLE_TARGETS: &[u8] =
    include_bytes!("../fixtures/performance/gc/changed_indirect_target.wasm");
const NOMINAL_EXACT: &[u8] = include_bytes!("../fixtures/performance/gc/nominal_exact_casts.wasm");
const TYPED_FUNCTION_REFS: &[u8] =
    include_bytes!("../fixtures/performance/gc/typed_function_refs.wasm");

fn engine() -> Engine {
    Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap()
}

fn load(engine: &Engine, store: &mut Store, bytes: &[u8]) -> Instance {
    let module = engine.compile(bytes).unwrap();
    Linker::new(engine).instantiate(store, &module).unwrap()
}

fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!("{name} is not a function export");
    };
    function
}

fn run_no_args(store: &mut Store, instance: Instance) -> i32 {
    let function = function(store, instance, "run");
    let outcome = store.invoke(function, &[]).unwrap();
    let RunOutcome::Returned(values) = outcome else {
        panic!("run must return normally");
    };
    let [Value::I32(result)] = values.as_slice() else {
        panic!("run must return one i32");
    };
    let result = *result;
    store.release_published(&values);
    result
}

fn run_i32(store: &mut Store, instance: Instance, n: i32) -> i32 {
    let function = function(store, instance, "run");
    let outcome = store.invoke(function, &[Value::I32(n)]).unwrap();
    let RunOutcome::Returned(values) = outcome else {
        panic!("run must return normally");
    };
    let [Value::I32(result)] = values.as_slice() else {
        panic!("run must return one i32");
    };
    let result = *result;
    store.release_published(&values);
    result
}

#[test]
fn p033_indirect_call_observes_each_table_target_mutation() {
    let engine = engine();
    let mut store = Store::new(&engine);
    let instance = load(&engine, &mut store, TABLE_TARGETS);
    let External::Table(table) = instance.export(&store, "table").unwrap() else {
        panic!("table export missing");
    };

    for _ in 0..2 {
        assert_eq!(run_no_args(&mut store, instance), 11);
    }

    let set_second = function(&store, instance, "set_second");
    store.invoke(set_second, &[]).unwrap();
    for _ in 0..2 {
        assert_eq!(run_no_args(&mut store, instance), 22);
    }

    let set_bad = function(&store, instance, "set_bad");
    store.invoke(set_bad, &[]).unwrap();
    let run = function(&store, instance, "run");
    let error = store.invoke(run, &[]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Trap);
    assert_eq!(error.trap_code, Some(TrapCode::IndirectCallTypeMismatch));

    let set_first = function(&store, instance, "set_first");
    store.invoke(set_first, &[]).unwrap();
    assert_eq!(run_no_args(&mut store, instance), 11);
    assert!(matches!(store.read_table(table, 0), Ok(Value::Ref(_))));
}

fn assert_gc_lifecycle(bytes: &[u8], counts: &[(i32, i32, usize)]) {
    let engine = engine();
    let mut store = Store::new(&engine);
    let instance = load(&engine, &mut store, bytes);
    for &(n, expected, allocations) in counts {
        assert_eq!(run_i32(&mut store, instance, n), expected, "input n={n}");
        let stats = store.collect().unwrap();
        assert_eq!(stats.live, 0, "live GC objects after n={n}");
        assert_eq!(
            stats.reclaimed, allocations,
            "reclaimed objects after n={n}"
        );
        assert_eq!(store.gc_payload_bytes(), 0, "retained payload after n={n}");
    }
}

#[test]
fn p033_nominal_exact_casts_match_oracle_and_collect_every_allocation() {
    assert_gc_lifecycle(
        NOMINAL_EXACT,
        &[(0, 0, 0), (1, 2, 1), (17, 34, 17), (32, 64, 32)],
    );
}

#[test]
fn p033_typed_function_reference_checks_match_oracle_without_gc_leaks() {
    assert_gc_lifecycle(
        TYPED_FUNCTION_REFS,
        &[(0, 0, 0), (1, 1, 0), (17, 153, 0), (32, 528, 0)],
    );
}

#[test]
fn p033_wrong_argument_and_foreign_store_errors_allow_later_valid_call() {
    let engine = engine();
    let mut store = Store::new(&engine);
    let instance = load(&engine, &mut store, NOMINAL_EXACT);
    let run = function(&store, instance, "run");

    assert_eq!(
        store.invoke(run, &[Value::I64(1)]).unwrap_err().kind,
        ErrorKind::Host
    );

    let mut foreign_store = Store::new(&engine);
    assert_eq!(
        foreign_store
            .invoke(run, &[Value::I32(1)])
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );

    assert_eq!(run_i32(&mut store, instance, 1), 2);
    let stats = store.collect().unwrap();
    assert_eq!((stats.live, stats.reclaimed), (0, 1));
    assert_eq!(store.gc_payload_bytes(), 0);
}
