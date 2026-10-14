// P030 public API correctness controls. Private validated-frame path proof
// lives in execution.rs and cannot be inferred from public outcomes alone.
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const GUEST_FRAME: &[u8] = include_bytes!("../fixtures/performance/frames/guest_frame.wasm");
const TAIL_CALL: &[u8] = include_bytes!("../fixtures/performance/frames/tail_call.wasm");
const RECURSION: &[u8] = include_bytes!("../fixtures/performance/frames/recursion.wasm");
const IMPORT_CONTROL: &[u8] = include_bytes!("../fixtures/performance/frames/import_control.wasm");
const REFERENCE_CONTROL: &[u8] =
    include_bytes!("../fixtures/performance/frames/reference_control.wasm");

fn load_with(bytes: &[u8], depth: usize, policy: ExecutionPolicy) -> (Store, Instance) {
    let mut config = Config::default();
    config.features = FeatureSet::all_phase3_core();
    config.limits.max_call_depth = depth;
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    drop(module);
    (store, instance)
}

fn load(bytes: &[u8], depth: usize) -> (Store, Instance) {
    load_with(bytes, depth, ExecutionPolicy::Decoded)
}

fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!("{name} is not a function export");
    };
    function
}

fn invoke_named(
    store: &mut Store,
    instance: Instance,
    name: &str,
    arguments: &[Value],
) -> Result<RunOutcome> {
    store.invoke(function(store, instance, name), arguments)
}

fn v128_i32x4(lanes: [i32; 4]) -> Value {
    let mut bytes = [0u8; 16];
    for (index, lane) in lanes.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&lane.to_le_bytes());
    }
    Value::V128(V128(bytes))
}

#[test]
fn p030_direct_guest_calls_check_zeroed_and_reset_locals() {
    let (mut store, instance) = load(GUEST_FRAME, 32);
    let expected_arguments = [
        Value::I32(7),
        Value::I64(9),
        Value::F32(1.5f32.to_bits()),
        Value::F64((-2.25f64).to_bits()),
        v128_i32x4([1, 2, 3, 4]),
    ];
    assert_eq!(
        invoke_named(&mut store, instance, "checked_target", &expected_arguments).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
    assert_eq!(
        invoke_named(&mut store, instance, "echo_run", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(-17),
            Value::I64(i64::MAX),
            Value::F32((-0.0f32).to_bits()),
            Value::F64((-2.25f64).to_bits()),
            v128_i32x4([-1, 0, 17, 99]),
        ])
    );
    // run invokes the target twice in one guest activation; the target writes
    // every nonparameter local nonzero before returning, so the second call
    // detects a reused frame slot that was not initialized.
    for _ in 0..2 {
        assert_eq!(
            invoke_named(&mut store, instance, "run", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(84)])
        );
    }
}

#[test]
fn p030_tail_call_forwards_and_checks_all_arguments_at_depth_two() {
    let (mut store, instance) = load(TAIL_CALL, 2);
    // The target compares i32/i64 values, exact f32/f64 bits, and all four
    // v128 lanes before returning 42, so success demonstrates forwarding.
    assert_eq!(
        invoke_named(&mut store, instance, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(42)])
    );
}

#[test]
fn p030_ordinary_and_tail_recursion_preserve_depth_contract() {
    let (mut store, instance) = load(RECURSION, 64);
    for n in [0, 1, 7, 32] {
        let expected = Value::I32(n + 42);
        assert_eq!(
            invoke_named(&mut store, instance, "ordinary", &[Value::I32(n)]).unwrap(),
            RunOutcome::Returned(vec![expected])
        );
    }
    let (mut tail_store, tail_instance) = load(RECURSION, 2);
    for n in [0, 1, 7, 32] {
        assert_eq!(
            invoke_named(&mut tail_store, tail_instance, "tail", &[Value::I32(n)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(n + 42)])
        );
    }
    let (mut shallow_store, shallow_instance) = load(RECURSION, 2);
    let error = invoke_named(
        &mut shallow_store,
        shallow_instance,
        "ordinary",
        &[Value::I32(7)],
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
}

#[test]
fn p030_imported_direct_call_stays_at_host_boundary() {
    let mut config = Config::default();
    config.features = FeatureSet::all_phase3_core();
    let engine = Engine::new(config).unwrap();
    let module = engine.compile(IMPORT_CONTROL).unwrap();
    let mut store = Store::new(&engine);
    let observed = Arc::new(Mutex::new(Vec::<i32>::new()));
    let callback_observed = observed.clone();
    let host = store
        .create_host_func(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            move |arguments| {
                let Value::I32(value) = arguments[0] else {
                    panic!("host.bump expects i32");
                };
                callback_observed.lock().unwrap().push(value);
                Ok(vec![Value::I32(value + 3)])
            },
        )
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("env", "bump", External::Func(host)).unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    assert_eq!(
        invoke_named(&mut store, instance, "run", &[Value::I32(5)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(8)])
    );
    assert_eq!(*observed.lock().unwrap(), vec![5]);

    let wrong_host = store
        .create_host_func(
            FuncType::new(vec![ValType::I64], vec![ValType::I64]),
            |arguments| Ok(arguments.to_vec()),
        )
        .unwrap();
    let mut wrong_linker = Linker::new(&engine);
    wrong_linker
        .define("env", "bump", External::Func(wrong_host))
        .unwrap();
    let error = wrong_linker.instantiate(&mut store, &module).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Link);
}

#[test]
fn p030_nominal_reference_arguments_remain_store_checked() {
    let (mut store, instance) = load(REFERENCE_CONTROL, 16);
    assert_eq!(
        invoke_named(&mut store, instance, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(41)])
    );
    let after_run = store.collect().unwrap();
    assert_eq!(after_run.live, 0);
    assert_eq!(after_run.reclaimed, 1);
    assert_eq!(store.gc_payload_bytes(), 0);

    let identity = function(&store, instance, "identity");
    let make_s_outcome = invoke_named(&mut store, instance, "make_s", &[]).unwrap();
    let RunOutcome::Returned(make_s_values) = make_s_outcome else {
        panic!("make_s did not return normally");
    };
    let value_s = make_s_values[0];
    assert!(matches!(value_s, Value::Ref(RefValue::Gc(_))));
    let value_u_outcome = invoke_named(&mut store, instance, "make_u", &[]).unwrap();
    let RunOutcome::Returned(value_u_values) = value_u_outcome else {
        panic!("make_u did not return normally");
    };
    let value_u = value_u_values[0];
    assert_eq!(
        store.invoke(identity, &[Value::I32(41)]).unwrap_err().kind,
        ErrorKind::Host
    );
    assert_eq!(
        store.invoke(identity, &[value_u]).unwrap_err().kind,
        ErrorKind::Host
    );

    let engine = Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let module = engine.compile(REFERENCE_CONTROL).unwrap();
    let mut foreign_store = Store::new(&engine);
    let foreign_instance = Linker::new(&engine)
        .instantiate(&mut foreign_store, &module)
        .unwrap();
    let foreign_outcome =
        invoke_named(&mut foreign_store, foreign_instance, "make_s", &[]).unwrap();
    let RunOutcome::Returned(foreign_values) = foreign_outcome else {
        panic!("foreign make_s did not return normally");
    };
    let foreign_value = foreign_values[0];
    assert_eq!(
        store.invoke(identity, &[foreign_value]).unwrap_err().kind,
        ErrorKind::Host
    );

    store.release_published(&[value_s, value_u]);
    foreign_store.release_published(&[foreign_value]);
    let local_stats = store.collect().unwrap();
    let foreign_stats = foreign_store.collect().unwrap();
    assert_eq!(local_stats.live, 0);
    assert_eq!(local_stats.reclaimed, 2);
    assert_eq!(store.gc_payload_bytes(), 0);
    assert_eq!(foreign_stats.live, 0);
    assert_eq!(foreign_stats.reclaimed, 1);
    assert_eq!(foreign_store.gc_payload_bytes(), 0);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p030_reference_policy_matches_public_call_outcomes() {
    let (mut store, instance) = load_with(GUEST_FRAME, 32, ExecutionPolicy::Reference);
    assert_eq!(
        invoke_named(&mut store, instance, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(84)])
    );
    let (mut store, instance) = load_with(RECURSION, 2, ExecutionPolicy::Reference);
    assert_eq!(
        invoke_named(&mut store, instance, "tail", &[Value::I32(32)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(74)])
    );
}
