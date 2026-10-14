use wtime_runtime::*;
#[test]
fn p058_decoded_fixture_results_preserve_default_policy() {
    let engine = Engine::new(Config::default()).unwrap();
    assert_eq!(engine.execution_policy(), ExecutionPolicy::Decoded);
    for (bytes, args, expected) in [
        (
            include_bytes!("../fixtures/performance/p058/numeric_slots.wasm").as_slice(),
            vec![Value::I32(21), Value::I32(4)],
            vec![Value::I32(14)],
        ),
        (
            include_bytes!("../fixtures/performance/p058/edge_values.wasm").as_slice(),
            vec![],
            vec![Value::I32(7), Value::I64(9)],
        ),
    ] {
        let module = engine.compile(bytes).unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        let External::Func(function) = instance.export(&store, "run").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.invoke(function, &args).unwrap(),
            RunOutcome::Returned(expected)
        );
    }
}
