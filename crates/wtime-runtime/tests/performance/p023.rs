use wtime_runtime::*;

fn guest(bytes: &[u8], fuel: Option<u64>) -> (Store, Instance) {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    drop(module);
    (store, instance)
}
fn call(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(function) = instance.export(store, name)? else {
        panic!()
    };
    store.invoke(function, args)
}
const VECTORS: &[u8] = include_bytes!("../fixtures/performance/borrow/vector_immediates.wasm");

#[test]
fn p023_br_table_vector_and_default_preserve_results() {
    let (mut store, instance) = guest(VECTORS, None);
    for index in [0, 17, 31, 32, 999, -1] {
        assert_eq!(
            call(&mut store, instance, "table", &[Value::I32(index)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
    }
}
#[test]
fn p023_typed_select_vector_preserves_both_choices() {
    let (mut store, instance) = guest(VECTORS, None);
    for (condition, expected) in [(0, 9), (1, 7), (-1, 7)] {
        assert_eq!(
            call(&mut store, instance, "select", &[Value::I32(condition)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(expected)])
        );
    }
}
#[test]
fn p023_try_table_catches_survive_callee_unwind_and_module_drop() {
    let (mut store, instance) = guest(
        include_bytes!("../fixtures/exceptions/cross_call.wasm"),
        None,
    );
    assert_eq!(
        call(&mut store, instance, "f", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
}
#[test]
fn p023_resume_keeps_owned_capture_across_collection() {
    let (mut store, instance) = guest(include_bytes!("../../examples/suspension.wasm"), None);
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = call(&mut store, instance, "run", &[]).unwrap()
    else {
        panic!()
    };
    assert_eq!(payload, vec![Value::I32(7)]);
    store.collect().unwrap();
    assert_eq!(
        call(
            &mut store,
            instance,
            "resume",
            &[Value::Ref(RefValue::Gc(continuation))]
        )
        .unwrap(),
        RunOutcome::Returned(vec![Value::I32(9)])
    );
}
#[test]
fn p023_source_fetch_preserves_every_fuel_boundary() {
    for fuel in 0..=5 {
        let (mut store, instance) = guest(VECTORS, Some(fuel));
        let outcome = call(&mut store, instance, "select", &[Value::I32(1)]);
        if fuel < 5 {
            let error = outcome.unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert_eq!(error.message, "fuel exhausted");
        } else {
            assert_eq!(outcome.unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
        }
        #[cfg(feature = "performance-counters")]
        assert_eq!(store.performance_counters().source_dispatches, fuel.min(5));
    }
}
