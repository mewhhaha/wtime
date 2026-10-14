use wtime_runtime::*;

fn loaded(config: Config, bytes: &[u8]) -> (Store, Func) {
    let engine = Engine::new(config).unwrap();
    assert_eq!(engine.execution_policy(), ExecutionPolicy::Decoded);
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else {
        panic!("run must be a function")
    };
    (store, function)
}

#[test]
fn p036_public_default_and_portable_keep_decoded_policy() {
    assert_eq!(Engine::new(Config::default()).unwrap().execution_policy(), ExecutionPolicy::Decoded);
    assert_eq!(Engine::with_execution_policy(Config::default(), ExecutionPolicy::Portable).unwrap().execution_policy(), ExecutionPolicy::Decoded);
}

#[test]
fn p036_public_long_function_remains_invocable_with_exact_decoded_fuel() {
    let bytes = include_bytes!("../fixtures/performance/compact/long_metadata.wasm");
    // 20,000 nops, one constant, and the implicit function-end instruction.
    for fuel in [0, 1, 20_001, 20_002] {
        let mut config = Config::default();
        config.limits.fuel = Some(fuel);
        let (mut store, function) = loaded(config, bytes);
        let result = store.invoke(function, &[]);
        if fuel == 20_002 {
            assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ResourceLimit);
        }
    }
}

#[test]
fn p036_public_wide_local_keeps_full_index_and_decoded_result() {
    let mut config = Config::default();
    config.limits.max_locals = 70_001;
    let (mut store, function) = loaded(config, include_bytes!("../fixtures/performance/plans/wide_local.wasm"));
    assert_eq!(store.invoke(function, &[]).unwrap(), RunOutcome::Returned(vec![Value::I32(42)]));
}

#[cfg(not(feature = "compact-executor"))]
#[test]
fn p036_public_default_build_does_not_expose_compact_policy() {
    assert_eq!(Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap_err().kind, ErrorKind::DisabledFeature);
}
