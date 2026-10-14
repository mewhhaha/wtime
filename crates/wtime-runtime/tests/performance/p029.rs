use std::sync::{Arc, Mutex};
use wtime_runtime::*;

fn engine_with(policy: ExecutionPolicy, depth: usize) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}

fn engine(depth: usize) -> Engine {
    engine_with(ExecutionPolicy::Decoded, depth)
}

fn compile(engine: &Engine, name: &str) -> Module {
    let bytes: &[u8] = match name {
        "direct-recursive-multivalue" => {
            include_bytes!("../fixtures/performance/calls/direct-recursive-multivalue.wasm")
        }
        "host-type-import" => include_bytes!("../fixtures/performance/calls/host-type-import.wasm"),
        "imported-direct" => include_bytes!("../fixtures/performance/calls/imported-direct.wasm"),
        "indirect-bad-signature" => {
            include_bytes!("../fixtures/performance/calls/indirect-bad-signature.wasm")
        }
        "indirect-good" => include_bytes!("../fixtures/performance/calls/indirect-good.wasm"),
        "reentry-gc-roots" => include_bytes!("../fixtures/performance/calls/reentry-gc-roots.wasm"),
        "tail-recursive" => include_bytes!("../fixtures/performance/calls/tail-recursive.wasm"),
        _ => panic!("unknown P029 fixture: {name}"),
    };
    engine.compile(bytes).unwrap()
}

fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!("{name} is not a function export");
    };
    function
}

fn plain_with(name: &str, depth: usize, policy: ExecutionPolicy) -> (Store, Instance) {
    let engine = engine_with(policy, depth);
    let module = compile(&engine, name);
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
}

fn plain(name: &str, depth: usize) -> (Store, Instance) {
    plain_with(name, depth, ExecutionPolicy::Decoded)
}

#[test]
fn p029_recursive_multivalue() {
    let (mut store, instance) = plain("direct-recursive-multivalue", 128);
    assert_eq!(
        store.invoke(func(&store, instance, "run"), &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );
}

#[test]
fn p029_imported_direct_and_host_signature() {
    let engine = engine(128);
    let module = compile(&engine, "imported-direct");
    let mut store = Store::new(&engine);
    let host = store
        .create_host_func(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            |values| {
                let Value::I32(value) = values[0] else {
                    panic!("host.bump expects i32");
                };
                Ok(vec![Value::I32(value + 1)])
            },
        )
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("host", "bump", External::Func(host)).unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    assert_eq!(
        store
            .invoke(func(&store, instance, "run"), &[Value::I32(5)])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(10)])
    );
    assert_eq!(
        store.invoke(host, &[Value::I64(5)]).unwrap_err().kind,
        ErrorKind::Host
    );
}

#[test]
fn p029_tail_depth() {
    let (mut store, instance) = plain("tail-recursive", 2);
    assert_eq!(
        store
            .invoke(func(&store, instance, "run"), &[Value::I32(128)])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(23)])
    );
}

#[test]
fn p029_indirect_checks() {
    for (name, good) in [("indirect-good", true), ("indirect-bad-signature", false)] {
        let (mut store, instance) = plain(name, 128);
        let function = func(&store, instance, "run");
        let arguments = if good {
            vec![Value::I32(5)]
        } else {
            Vec::new()
        };
        let outcome = store.invoke(function, &arguments);
        if good {
            assert_eq!(outcome.unwrap(), RunOutcome::Returned(vec![Value::I32(12)]));
        } else {
            let error = outcome.unwrap_err();
            assert_eq!(error.trap_code, Some(TrapCode::IndirectCallTypeMismatch));
        }
    }
}

#[test]
fn p029_foreign_function_and_wrong_import() {
    let engine = engine(128);
    let mut first_store = Store::new(&engine);
    let mut second_store = Store::new(&engine);
    let foreign = first_store
        .create_host_func(FuncType::new(vec![], vec![]), |_| Ok(vec![]))
        .unwrap();
    assert_eq!(
        second_store.invoke(foreign, &[]).unwrap_err().kind,
        ErrorKind::Host
    );

    let module = compile(&engine, "host-type-import");
    let wrong = second_store
        .create_host_func(
            FuncType::new(vec![ValType::I64], vec![ValType::I64]),
            |arguments| Ok(arguments.to_vec()),
        )
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("host", "f", External::Func(wrong)).unwrap();
    let error = linker.instantiate(&mut second_store, &module).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Link);
    assert_eq!(error.message, "function import type mismatch");
}

#[test]
fn p029_reentry_growth_gc_root() {
    let engine = engine(128);
    let module = compile(&engine, "reentry-gc-roots");
    let mut store = Store::new(&engine);
    let nested = Arc::new(Mutex::new(None::<Func>));
    let slot = nested.clone();
    let host = store
        .create_module_host_func(&module, TypeIdx(1), move |caller, arguments| {
            assert_eq!(caller.collect()?.live, 1);
            for _ in 0..1024 {
                caller
                    .store()
                    .create_host_func(FuncType::new(vec![], vec![]), |_| Ok(vec![]))?;
            }
            let read = slot.lock().unwrap().unwrap();
            assert_eq!(
                caller.invoke(read, arguments)?,
                RunOutcome::Returned(vec![Value::I32(41)])
            );
            assert_eq!(caller.collect()?.live, 1);
            Ok(RunOutcome::Returned(vec![]))
        })
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker
        .define("host", "visit", External::Func(host))
        .unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    *nested.lock().unwrap() = Some(func(&store, instance, "read"));
    drop(module);
    assert_eq!(
        store.invoke(func(&store, instance, "run"), &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(41)])
    );
    assert_eq!(store.collect().unwrap().live, 0);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p029_reference_reuses_public_call_helpers() {
    let (mut store, instance) = plain_with(
        "direct-recursive-multivalue",
        128,
        ExecutionPolicy::Reference,
    );
    assert_eq!(
        store.invoke(func(&store, instance, "run"), &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );

    let (mut store, instance) = plain_with("tail-recursive", 2, ExecutionPolicy::Reference);
    assert_eq!(
        store
            .invoke(func(&store, instance, "run"), &[Value::I32(128)])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(23)])
    );

    let (mut store, instance) = plain_with("indirect-good", 128, ExecutionPolicy::Reference);
    assert_eq!(
        store
            .invoke(func(&store, instance, "run"), &[Value::I32(5)])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(12)])
    );

    let (mut store, instance) =
        plain_with("indirect-bad-signature", 128, ExecutionPolicy::Reference);
    let error = store
        .invoke(func(&store, instance, "run"), &[])
        .unwrap_err();
    assert_eq!(error.trap_code, Some(TrapCode::IndirectCallTypeMismatch));
}
