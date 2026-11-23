// A reference caught at function depth must stay rooted until explicit release.
use wtime_runtime::*;

fn assert_caught_reference_root(policy: ExecutionPolicy) {
    let engine = Engine::with_execution_policy(Config::default(), policy).unwrap();
    let module = engine
        .compile(include_bytes!(
            "fixtures/exception_root/caught_ref_to_function.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(run) = instance.export(&store, "run").unwrap() else {
        panic!("run must be a function");
    };
    let RunOutcome::Returned(values) = store.invoke(run, &[]).unwrap() else {
        panic!("caught exception should return from the function");
    };
    assert_eq!(values.len(), 1);
    let reference = values[0];

    // Exercise the ownership boundary before another runtime operation can
    // publish this reference as a root.
    let retained = store.collect().unwrap();
    assert_eq!(retained.live, 1);

    let External::Func(read) = instance.export(&store, "read").unwrap() else {
        panic!("read must be a function");
    };
    assert_eq!(
        store.invoke(read, &[reference]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(91)])
    );
    store.release_published(&values);
    let released = store.collect().unwrap();
    assert_eq!(released.reclaimed, 1);
    assert_eq!(released.live, 0);
    assert_eq!(store.gc_payload_bytes(), 0);
}

#[test]
fn caught_reference_root_decoded() {
    assert_caught_reference_root(ExecutionPolicy::Decoded);
}
#[cfg(feature = "execution-reference")]
#[test]
fn caught_reference_root_reference() {
    assert_caught_reference_root(ExecutionPolicy::Reference);
}
