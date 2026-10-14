#[test]
fn p062_float_preparation_and_activation_guards() {
    let bytes = include_bytes!("../fixtures/performance/p062/mixed_slots.wasm");
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let module = engine.compile(bytes).unwrap();
    assert_eq!(module.data.register.as_ref().unwrap().count(), 1);
    let raw = [
        Value::I32(-1),
        Value::I64(i64::MIN),
        Value::F32(0xff81_2345),
        Value::F64(0xfff0_0000_0000_1234),
        Value::I32(0),
        Value::I64(0),
        Value::F32(0),
        Value::F64(0),
    ];
    let mut labels = vec![];
    let activation = activate(&module, 0, &raw, &mut labels).unwrap();
    assert_eq!(
        &activation.slots[..4],
        &[
            u32::MAX as u64,
            i64::MIN as u64,
            0xff81_2345,
            0xfff0_0000_0000_1234
        ]
    );
    let mut wrong = raw;
    wrong[2] = Value::I32(0xff81_2345u32 as i32);
    assert!(activate(&module, 0, &wrong, &mut vec![]).is_none());
    let mut budget = Budget::new(0, MAX_BYTES);
    assert!(LoweredFunction::lower(&module, 0, &mut budget).is_err());
    assert_eq!(budget.remaining(), (0, MAX_BYTES));
    assert!(!float::supported(0xfd00));
    assert!(float::unary(0xfd00, 0).is_err());
    assert!(float::binary(0xfd00, 0, 0).is_err());
}
