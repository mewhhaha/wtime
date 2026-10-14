use wtime_runtime::*;
fn instance(bytes: &[u8]) -> (Store, Instance) {
    let e = Engine::new(Config::default()).unwrap();
    let m = e.compile(bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    (s, i)
}
fn invoke(s: &mut Store, i: Instance, n: &str, args: &[Value]) -> Result<Vec<Value>> {
    let External::Func(f) = i.export(s, n)? else {
        panic!("expected function")
    };
    match s.invoke(f, args)? {
        RunOutcome::Returned(v) => Ok(v),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn t086_byte_pages_and_growth() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    assert_eq!(
        invoke(&mut s, i, "byte-size", &[]).unwrap(),
        [Value::I32(4)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-grow", &[Value::I32(3)]).unwrap(),
        [Value::I32(4)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-size", &[]).unwrap(),
        [Value::I32(7)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-grow", &[Value::I32(10)]).unwrap(),
        [Value::I32(-1)]
    );
    let External::Memory(m) = i.export(&s, "bytes").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 0, 7).unwrap(), [0; 7]);
}
#[test]
fn t082_memory64_addresses_and_width() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    assert_eq!(
        invoke(&mut s, i, "wide-size", &[]).unwrap(),
        [Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "write-read", &[Value::I64(3), Value::I64(-7)]).unwrap(),
        [Value::I64(-7)]
    );
    assert_eq!(
        invoke(&mut s, i, "write-read", &[Value::I64(-1), Value::I64(0)])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    assert_eq!(
        invoke(&mut s, i, "wide-grow", &[Value::I64(1)]).unwrap(),
        [Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "wide-grow", &[Value::I64(i64::MAX)]).unwrap(),
        [Value::I64(-1)]
    );
}
#[test]
fn t083_mixed_width_copy_is_prechecked() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    invoke(
        &mut s,
        i,
        "write-read",
        &[Value::I64(0), Value::I64(0x01020304)],
    )
    .unwrap();
    invoke(
        &mut s,
        i,
        "copy",
        &[Value::I32(0), Value::I64(0), Value::I32(4)],
    )
    .unwrap();
    let External::Memory(m) = i.export(&s, "bytes").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 0, 4).unwrap(), [4, 3, 2, 1]);
    assert_eq!(
        invoke(
            &mut s,
            i,
            "copy",
            &[Value::I32(1), Value::I64(0), Value::I32(4)]
        )
        .unwrap_err()
        .kind,
        ErrorKind::Trap
    );
    assert_eq!(s.read_memory(m, 0, 4).unwrap(), [4, 3, 2, 1]);
}
#[test]
fn t092_wide_arithmetic_public_execution() {
    let (mut s, i) = instance(include_bytes!("fixtures/wide.wasm"));
    assert_eq!(
        invoke(
            &mut s,
            i,
            "add",
            &[Value::I64(-1), Value::I64(0), Value::I64(1), Value::I64(0)]
        )
        .unwrap(),
        [Value::I64(0), Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "mul-u", &[Value::I64(-1), Value::I64(-1)]).unwrap(),
        [Value::I64(1), Value::I64(-2)]
    );
}
