use wtime_runtime::*;
fn setup() -> (Store, Instance) {
    let engine = Engine::new(Config::default()).unwrap();
    let module = engine
        .compile(include_bytes!("fixtures/core.wasm"))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
}
fn call(s: &mut Store, i: Instance, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    let External::Func(f) = i.export(s, name)? else {
        panic!("expected function")
    };
    let RunOutcome::Returned(v) = s.invoke(f, args)? else {
        return Err(Error::new(ErrorKind::Host, "unexpected guest exception"));
    };
    Ok(v)
}
#[test]
fn t048_wrapping_and_division_boundaries() {
    let (mut s, i) = setup();
    assert_eq!(
        call(&mut s, i, "add", &[Value::I32(i32::MAX), Value::I32(1)]).unwrap(),
        [Value::I32(i32::MIN)]
    );
    assert_eq!(
        call(&mut s, i, "div", &[Value::I32(i32::MIN), Value::I32(-1)])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    assert_eq!(
        call(&mut s, i, "div", &[Value::I32(1), Value::I32(0)])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
}
#[test]
fn t054_recursive_calls_and_branch_loops() {
    let (mut s, i) = setup();
    assert_eq!(
        call(&mut s, i, "factorial", &[Value::I32(8)]).unwrap(),
        [Value::I32(40320)]
    );
    assert_eq!(
        call(&mut s, i, "sum", &[Value::I32(100)]).unwrap(),
        [Value::I32(5050)]
    );
}
#[test]
fn t055_unaligned_access_and_bounds() {
    let (mut s, i) = setup();
    assert_eq!(
        call(
            &mut s,
            i,
            "store-load",
            &[Value::I32(3), Value::I32(0x12345678)]
        )
        .unwrap(),
        [Value::I32(0x12345678)]
    );
    assert_eq!(
        call(&mut s, i, "store-load", &[Value::I32(65534), Value::I32(7)])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    let External::Memory(m) = i.export(&s, "memory").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 3, 4).unwrap(), [0x78, 0x56, 0x34, 0x12]);
    assert_eq!(s.read_memory(m, 65534, 2).unwrap(), [0, 0]);
}
#[test]
fn t056_growth_failure_and_zero_initialization() {
    let (mut s, i) = setup();
    assert_eq!(
        call(&mut s, i, "grow", &[Value::I32(1)]).unwrap(),
        [Value::I32(1)]
    );
    assert_eq!(
        call(&mut s, i, "grow", &[Value::I32(1)]).unwrap(),
        [Value::I32(-1)]
    );
    let External::Memory(m) = i.export(&s, "memory").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 65536, 16).unwrap(), [0; 16]);
}
#[test]
fn t052_conversions_trap_and_unsigned_range() {
    let (mut s, i) = setup();
    assert_eq!(
        call(
            &mut s,
            i,
            "trunc",
            &[Value::F64((-2147483648.75f64).to_bits())]
        )
        .unwrap(),
        [Value::I32(i32::MIN)]
    );
    assert_eq!(
        call(&mut s, i, "trunc", &[Value::F64(f64::NAN.to_bits())])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    assert_eq!(
        call(&mut s, i, "unsigned-f64", &[Value::I64(-1)]).unwrap(),
        [Value::F64(18446744073709551616f64.to_bits())]
    );
}
#[test]
fn t051_nearest_and_min_preserve_signed_zero() {
    let (mut s, i) = setup();
    for (input, expected) in [(2.5f64, 2f64), (3.5, 4.), (-0.5, -0.)] {
        assert_eq!(
            call(&mut s, i, "nearest", &[Value::F64(input.to_bits())]).unwrap(),
            [Value::F64(expected.to_bits())]
        );
    }
    assert_eq!(
        call(
            &mut s,
            i,
            "min",
            &[Value::F64(0f64.to_bits()), Value::F64((-0f64).to_bits())]
        )
        .unwrap(),
        [Value::F64((-0f64).to_bits())]
    );
}
#[test]
fn t039_instance_state_is_mutable() {
    let (mut s, i) = setup();
    assert_eq!(call(&mut s, i, "count", &[]).unwrap(), [Value::I32(1)]);
    assert_eq!(call(&mut s, i, "count", &[]).unwrap(), [Value::I32(2)]);
}
#[test]
fn t035_foreign_store_and_argument_types_rejected() {
    let (mut s, i) = setup();
    let External::Func(f) = i.export(&s, "add").unwrap() else {
        panic!()
    };
    assert_eq!(s.invoke(f, &[]).unwrap_err().kind, ErrorKind::Host);
    let e = Engine::new(Config::default()).unwrap();
    let mut foreign = Store::new(&e);
    assert_eq!(
        foreign
            .invoke(f, &[Value::I32(1), Value::I32(2)])
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
}

/// A written memory grown by one byte keeps its contents when an atomic
/// access converts it to a shared backing (regression: the grown page was
/// copied past the memory's length).
#[test]
fn grow_then_atomic_access_keeps_contents() {
    let e = Engine::new(Config { features: FeatureSet::all_phase3_core(), ..Config::default() }).unwrap();
    let m = e.compile(include_bytes!("fixtures/grow-then-atomic.wasm")).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    let External::Memory(memory) = i.export(&s, "m").unwrap() else { panic!() };
    s.write_memory(memory, 0, &[1, 2, 3, 4]).unwrap();
    assert_eq!(call(&mut s, i, "grow", &[Value::I32(1)]).unwrap(), [Value::I32(4)]);
    assert_eq!(call(&mut s, i, "load", &[Value::I32(0)]).unwrap(), [Value::I32(0x04030201)]);
    assert_eq!(call(&mut s, i, "aload", &[Value::I32(0)]).unwrap(), [Value::I32(0x04030201)]);
    assert_eq!(s.read_memory(memory, 0, 5).unwrap(), [1, 2, 3, 4, 0]);
}

/// Every instruction of straight-line code is charged exactly once, through
/// the lowered and fused paths alike: the least fuel that completes the
/// function is its instruction count, and every smaller budget runs out.
#[test]
fn fused_pairs_charge_each_instruction_once() {
    let bytes = include_bytes!("fixtures/fused-pairs.wasm");
    let run = |fuel| {
        let mut config = Config::default();
        config.limits.fuel = Some(fuel);
        let e = Engine::new(config).unwrap();
        let m = e.compile(bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        call(&mut s, i, "f", &[Value::I32(10)])
    };
    // b = 10; c = b + 3 = 13; c = c + b = 23; g = c = 23; b = g = 23;
    // g = b = 23; c = 7 + g + c = 53.
    assert_eq!(run(24).unwrap(), [Value::I32(53)]);
    for fuel in 0..24 {
        let error = run(fuel).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit, "fuel {fuel}");
        assert!(error.message.contains("fuel"), "fuel {fuel}: {error}");
    }
}

/// Fused expressions behave exactly as the instructions they cover run one
/// at a time: the same fuel, the same memory effects before fuel runs out,
/// the same operand-stack limit and the same traps. The unfused reference
/// is the same code with a nop after every instruction, which no group of
/// the lowered Decoded tier contains.
#[test]
fn fused_expressions_match_unfused_instructions() {
    let fused = include_bytes!("fixtures/fused-expressions.wasm").as_slice();
    let plain = include_bytes!("fixtures/fused-expressions-nops.wasm").as_slice();
    let run = |bytes: &[u8], name: &str, args: &[Value], fuel: Option<u64>, height: Option<usize>| {
        let mut config = Config::default();
        config.limits.fuel = fuel;
        if let Some(height) = height {
            config.limits.max_stack_height = height;
        }
        let e = Engine::new(config).unwrap();
        let m = match e.compile(bytes) {
            Ok(m) => m,
            Err(error) => return (Err(error.message), vec![]),
        };
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let result = call(&mut s, i, name, args).map_err(|error| format!("{:?}: {}", error.kind, error.message));
        let External::Memory(memory) = i.export(&s, "mem").unwrap() else { panic!() };
        (result, s.read_memory(memory, 0, 128).unwrap())
    };
    let args = [Value::I32(5), Value::I64(100)];
    // 44 instructions and the function's end.
    let complete = run(fused, "f", &args, Some(45), None);
    assert_eq!(complete.0, Ok(vec![Value::I64(787)]));
    assert_eq!(complete, run(plain, "f", &args, Some(89), None));
    for fuel in 0..45 {
        let partial = run(fused, "f", &args, Some(fuel), None);
        assert!(partial.0.as_ref().unwrap_err().contains("fuel"), "fuel {fuel}: {:?}", partial.0);
        assert_eq!(partial, run(plain, "f", &args, Some(2 * fuel), None), "fuel {fuel}");
        if fuel > 0 {
            assert_eq!(partial, run(plain, "f", &args, Some(2 * fuel - 1), None), "fuel {fuel}");
        }
    }
    for height in 0..16 {
        let limited = run(fused, "deep", &args, None, Some(height));
        assert_eq!(limited, run(plain, "deep", &args, None, Some(height)), "height {height}");
    }
    assert!(run(fused, "deep", &args, None, Some(6)).0.unwrap_err().contains("stack"));
    assert_eq!(run(fused, "deep", &args, None, Some(16)).0, Ok(vec![Value::I64(787)]));
    for (name, address) in [("load", 65532), ("load", 65536), ("store", 65534), ("store", -1)] {
        let fused = run(fused, name, &[Value::I32(address)], None, None);
        assert_eq!(fused, run(plain, name, &[Value::I32(address)], None, None), "{name} {address}");
        assert!(fused.0.is_err() || name == "load" && address == 65532, "{name} {address}");
    }
}
