use wtime_runtime::*;

const WASM: &[u8] = include_bytes!("../fixtures/performance/numeric/integer_numeric.wasm");

fn load_with(fuel: Option<u64>, policy: ExecutionPolicy) -> (Store, Instance) {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Default::default()
    };
    config.limits.fuel = fuel;
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(WASM).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    drop(module);
    (store, instance)
}

fn load(fuel: Option<u64>) -> (Store, Instance) {
    load_with(fuel, ExecutionPolicy::Decoded)
}

fn run(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!("{name} is not a function export");
    };
    store.invoke(function, args)
}

fn assert_fuel_prefix(policy: ExecutionPolicy) {
    let arguments = [Value::I32(i32::MAX), Value::I32(2)];
    for fuel in 0..=17 {
        let (mut store, instance) = load_with(Some(fuel), policy);
        let outcome = run(&mut store, instance, "fuel_prefix", &arguments);
        if fuel < 17 {
            let error = outcome.unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit, "fuel {fuel}");
            assert_eq!(error.message, "fuel exhausted", "fuel {fuel}");
        } else {
            assert_eq!(outcome.unwrap(), RunOutcome::Returned(vec![Value::I32(42)]));
        }

        let expected_words = [
            if fuel >= 5 { 0x8000_0001u32 } else { 0 },
            if fuel >= 10 { 0x7fff_fffd } else { 0 },
            if fuel >= 15 { 0xffff_fffe } else { 0 },
        ];
        let mut expected_prefix = [0u8; 12];
        for (index, word) in expected_words.into_iter().enumerate() {
            expected_prefix[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let External::Memory(memory) = instance.export(&store, "memory").unwrap() else {
            panic!("memory export missing");
        };
        let image = store.read_memory(memory, 0, 65536).unwrap();
        assert_eq!(&image[..12], &expected_prefix, "fuel {fuel}");
        assert!(image[12..].iter().all(|byte| *byte == 0), "fuel {fuel}");
    }
}

#[test]
fn p028_wrap() {
    let (mut store, instance) = load(None);
    assert_eq!(
        run(&mut store, instance, "wrap32", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(i32::MIN),
            Value::I32(i32::MAX),
            Value::I32(0),
        ])
    );
    assert_eq!(
        run(&mut store, instance, "wrap64", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I64(i64::MIN),
            Value::I64(i64::MAX),
            Value::I64(0)
        ])
    );
}

#[test]
fn p028_bitwise() {
    let (mut store, instance) = load(None);
    assert_eq!(
        run(&mut store, instance, "bitwise32", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(15_732_480),
            Value::I32(-983_281),
            Value::I32(-16_715_761)
        ])
    );
    assert_eq!(
        run(&mut store, instance, "bitwise64", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I64(67_570_487_084_974_080),
            Value::I64(-4_486_007_692_984_321),
            Value::I64(-72_056_494_777_958_401),
        ])
    );
}

#[test]
fn p028_shifts() {
    let (mut store, instance) = load(None);
    assert_eq!(
        run(&mut store, instance, "shifts32", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(1),
            Value::I32(2),
            Value::I32(-8),
            Value::I32(i32::MIN)
        ])
    );
    assert_eq!(
        run(&mut store, instance, "shifts64", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I64(1),
            Value::I64(2),
            Value::I64(-8),
            Value::I64(i64::MIN)
        ])
    );
}

#[test]
fn p028_compare() {
    let (mut store, instance) = load(None);
    let expected = RunOutcome::Returned(vec![
        Value::I32(1),
        Value::I32(0),
        Value::I32(0),
        Value::I32(1),
    ]);
    assert_eq!(
        run(&mut store, instance, "compare32", &[]).unwrap(),
        expected
    );
    assert_eq!(
        run(&mut store, instance, "compare64", &[]).unwrap(),
        expected
    );
}

#[test]
fn p028_unary() {
    let (mut store, instance) = load(None);
    assert_eq!(
        run(&mut store, instance, "unary32", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(8), Value::I32(24), Value::I32(5)])
    );
    assert_eq!(
        run(&mut store, instance, "unary64", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I64(32), Value::I64(32), Value::I64(2)])
    );
}

#[test]
fn p028_fuel_prefix_full_memory() {
    assert_fuel_prefix(ExecutionPolicy::Decoded);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p028_reference_matches_numeric_and_fuel_boundaries() {
    let (mut store, instance) = load_with(None, ExecutionPolicy::Reference);
    assert_eq!(
        run(&mut store, instance, "wrap32", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(i32::MIN),
            Value::I32(i32::MAX),
            Value::I32(0)
        ])
    );
    assert_eq!(
        run(&mut store, instance, "compare32", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(1),
            Value::I32(0),
            Value::I32(0),
            Value::I32(1)
        ])
    );
    assert_eq!(
        run(&mut store, instance, "compare64", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(1),
            Value::I32(0),
            Value::I32(0),
            Value::I32(1)
        ])
    );
    assert_fuel_prefix(ExecutionPolicy::Reference);
}
