use wtime_runtime::*;
fn setup(bytes: &[u8]) -> (Store, Instance) {
    let e = Engine::new(Config::default()).unwrap();
    let m = e.compile(bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    (s, i)
}
fn call(s: &mut Store, i: Instance) -> Result<RunOutcome> {
    let External::Func(f) = i.export(s, "f")? else {
        panic!()
    };
    s.invoke(f, &[])
}
#[test]
fn t122_cross_call_catch() {
    let (mut s, i) = setup(include_bytes!("fixtures/exceptions/cross_call.wasm"));
    assert_eq!(
        call(&mut s, i).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
}
#[test]
fn t123_rethrow_reference() {
    let (mut s, i) = setup(include_bytes!("fixtures/exceptions/rethrow.wasm"));
    assert_eq!(
        call(&mut s, i).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
}
#[test]
fn t124_uncaught_identity_payload() {
    let (mut s, i) = setup(include_bytes!("fixtures/exceptions/uncaught.wasm"));
    let RunOutcome::Exception(ex) = call(&mut s, i).unwrap() else {
        panic!()
    };
    let External::Tag(tag) = i.export(&s, "e").unwrap() else {
        panic!()
    };
    assert_eq!(s.exception_tag(ex).unwrap(), tag);
    assert_eq!(s.exception_payload(ex).unwrap(), [Value::I32(42)]);
}
#[test]
fn t125_traps_are_not_catchable() {
    let (mut s, i) = setup(include_bytes!("fixtures/exceptions/trap.wasm"));
    assert_eq!(call(&mut s, i).unwrap_err().kind, ErrorKind::Trap);
    let (mut s, i) = setup(include_bytes!("fixtures/exceptions/null.wasm"));
    assert_eq!(call(&mut s, i).unwrap_err().kind, ErrorKind::Trap);
}
#[test]
fn t126_host_throw() {
    let e = Engine::new(Config::default()).unwrap();
    let m = e
        .compile(include_bytes!("fixtures/exceptions/host.wasm"))
        .unwrap();
    let mut s = Store::new(&e);
    let tag = s
        .create_tag(FuncType {
            params: vec![ValType::I32],
            results: vec![],
        })
        .unwrap();
    let ex = s.create_exception(tag, &[Value::I32(7)]).unwrap();
    let f = s
        .create_host_func_with_outcome(FuncType::default(), move |_, _| {
            Ok(RunOutcome::Exception(ex))
        })
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("m", "raise", External::Func(f)).unwrap();
    l.define("m", "e", External::Tag(tag)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    assert_eq!(
        call(&mut s, i).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7)])
    );
}
