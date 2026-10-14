use wtime_runtime::*;
const WASM: &[u8] = b"\0asm\x01\0\0\0\x01\x06\x01\x60\x01\x7f\x01\x7f\x03\x02\x01\0\x07\x07\x01\x03run\0\0\x0a\x09\x01\x07\0\x20\0\x41\x01\x6a\x0b";
#[test]
fn p017_clone_retains_executable_after_original_drop() {
    let e = Engine::new(Config::default()).unwrap();
    let original = e.compile(WASM).unwrap();
    let cloned = original.clone();
    drop(original);
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &cloned).unwrap();
    drop(cloned);
    let External::Func(f) = i.export(&s, "run").unwrap() else {
        panic!()
    };
    assert_eq!(
        s.invoke(f, &[Value::I32(41)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
}
#[test]
fn p017_foreign_engine_rejects_shared_module() {
    let e = Engine::new(Config::default()).unwrap();
    let m = e.compile(WASM).unwrap().clone();
    let other = Engine::new(Config::default()).unwrap();
    let mut s = Store::new(&other);
    assert_eq!(
        Linker::new(&other)
            .instantiate(&mut s, &m)
            .unwrap_err()
            .kind,
        ErrorKind::Link
    );
    assert_eq!(
        s.create_module_host_func(&m, TypeIdx(0), |_, _| Ok(RunOutcome::Returned(vec![
            Value::I32(0)
        ])))
        .unwrap_err()
        .kind,
        ErrorKind::Host
    );
}
#[test]
fn p017_suspended_frame_retains_module_context_after_drop() {
    let config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    let e = Engine::new(config).unwrap();
    let m = e
        .compile(include_bytes!("../../examples/suspension.wasm"))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    drop(m);
    let External::Func(run) = i.export(&s, "run").unwrap() else {
        panic!()
    };
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = s.invoke(run, &[]).unwrap()
    else {
        panic!()
    };
    assert_eq!(payload, vec![Value::I32(7)]);
    s.collect().unwrap();
    let External::Func(resume) = i.export(&s, "resume").unwrap() else {
        panic!()
    };
    assert_eq!(
        s.invoke(resume, &[Value::Ref(RefValue::Gc(continuation))])
            .unwrap(),
        RunOutcome::Returned(vec![Value::I32(9)])
    );
}
