use wtime_runtime::*;
const STORE: &[u8] = include_bytes!("../fixtures/performance/cursor/store_successor.wasm");
fn invoke(s: &mut Store, i: Instance, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(f) = i.export(s, "run")? else {
        panic!()
    };
    s.invoke(f, args)
}
#[test]
fn p020_every_fuel_prefix_preserves_completed_stores() {
    for fuel in 0..=7 {
        let mut c = Config::default();
        c.limits.fuel = Some(fuel);
        let e = Engine::new(c).unwrap();
        let m = e.compile(STORE).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let result = invoke(&mut s, i, &[]);
        if fuel < 6 {
            let error = result.unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert_eq!(error.message, "fuel exhausted");
        } else {
            assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
        }
        let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
            panic!()
        };
        assert_eq!(
            s.read_memory(mem, 0, 4).unwrap(),
            if fuel >= 3 {
                vec![7, 0, 0, 0]
            } else {
                vec![0, 0, 0, 0]
            },
            "fuel {fuel}"
        );
    }
}
#[test]
fn p020_fuel_failure_precedes_transient_stack_limit() {
    for (fuel, expected) in [(4, "fuel exhausted"), (5, "operand stack limit")] {
        let mut c = Config::default();
        c.limits.fuel = Some(fuel);
        c.limits.max_stack_height = 2;
        let e = Engine::new(c).unwrap();
        let m = e
            .compile(include_bytes!(
                "../fixtures/performance/accounting/transient_stack.wasm"
            ))
            .unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        assert_eq!(invoke(&mut s, i, &[]).unwrap_err().message, expected);
    }
}
#[test]
fn p020_cancellation_after_host_prefix_prevents_later_store() {
    let e = Engine::new(Config::default()).unwrap();
    let m = e
        .compile(include_bytes!(
            "../fixtures/performance/accounting/host_then_store.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let cancellation = s.cancellation_token();
    let f = s
        .create_host_func(FuncType::new(vec![], vec![]), move |_| {
            cancellation.cancel();
            Ok(vec![])
        })
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("host", "stop", External::Func(f)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    assert_eq!(
        invoke(&mut s, i, &[]).unwrap_err().message,
        "execution cancelled"
    );
    let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(mem, 0, 4).unwrap(), [0, 0, 0, 0]);
}
#[test]
fn p020_tail_recursion_preserves_logical_depth_policy() {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.max_call_depth = 16;
    let e = Engine::new(c).unwrap();
    for (wasm, tail) in [
        (
            include_bytes!("../fixtures/performance/accounting/tail_calls.wasm").as_slice(),
            true,
        ),
        (
            include_bytes!("../fixtures/performance/accounting/ordinary_recursion.wasm").as_slice(),
            false,
        ),
    ] {
        let m = e.compile(wasm).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let result = invoke(&mut s, i, &[Value::I32(1024)]);
        if tail {
            let mut expected = 1i32;
            for _ in 0..1024 {
                expected = expected.wrapping_mul(3).wrapping_add(1)
            }
            assert_eq!(
                result.unwrap(),
                RunOutcome::Returned(vec![Value::I32(expected)])
            );
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert!(error.message.contains("call depth"));
        }
    }
}
