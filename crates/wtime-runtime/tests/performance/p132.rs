use wtime_runtime::*;

#[test]
fn p132_module_preparation_remains_policy_neutral() {
    let bytes = include_bytes!("../fixtures/performance/p064/direct_call.wasm");
    for policy in [
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Portable,
        #[cfg(feature = "register-executor")]
        ExecutionPolicy::Register,
    ] {
        let engine = Engine::with_execution_policy(Config::default(), policy).unwrap();
        let module = engine.compile(bytes).unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        let External::Func(function) = instance.export(&store, "run").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.invoke(function, &[Value::I32(21)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(66)])
        );
        let External::Memory(memory) = instance.export(&store, "memory").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.read_memory(memory, 8, 4).unwrap(),
            21i32.to_le_bytes()
        );
        #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 11);
            assert_eq!(store.register_exits(), 1);
        }
    }
}
