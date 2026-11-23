// Public-API behavioral controls for future tests/performance/p032.rs.
// Private argument-range/counter proofs remain in the execution-module tests.
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const GUEST_FRAME: &[u8] = include_bytes!("../fixtures/performance/frames/guest_frame.wasm");
const TAIL_RECURSIVE: &[u8] = include_bytes!("../fixtures/performance/calls/tail-recursive.wasm");
const RECURSIVE_MULTIVALUE: &[u8] =
    include_bytes!("../fixtures/performance/calls/direct-recursive-multivalue.wasm");
const IMPORTED_DIRECT: &[u8] = include_bytes!("../fixtures/performance/calls/imported-direct.wasm");
const REFERENCE_CONTROL: &[u8] =
    include_bytes!("../fixtures/performance/frames/reference_control.wasm");

fn engine(depth: usize, policy: ExecutionPolicy) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Default::default()
    };
    config.limits.max_call_depth = depth;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn load(engine: &Engine, store: &mut Store, bytes: &[u8]) -> Instance {
    let module = engine.compile(bytes).unwrap();
    Linker::new(engine).instantiate(store, &module).unwrap()
}
fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(f) = instance.export(store, name).unwrap() else {
        panic!("{name} is not a function export");
    };
    f
}
fn call(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let f = function(store, instance, name);
    store.invoke(f, args)
}
fn v128_i32x4(lanes: [i32; 4]) -> Value {
    let mut bytes = [0u8; 16];
    for (index, lane) in lanes.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&lane.to_le_bytes());
    }
    Value::V128(V128(bytes))
}

#[test]
fn p032_guest_frame_and_recursive_multivalue_preserve_exact_arguments() {
    let engine = engine(128, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let guest = load(&engine, &mut store, GUEST_FRAME);
    assert_eq!(
        call(&mut store, guest, "echo_run", &[]).unwrap(),
        RunOutcome::Returned(vec![
            Value::I32(-17),
            Value::I64(i64::MAX),
            Value::F32((-0.0f32).to_bits()),
            Value::F64((-2.25f64).to_bits()),
            v128_i32x4([-1, 0, 17, 99]),
        ])
    );

    let multivalue = load(&engine, &mut store, RECURSIVE_MULTIVALUE);
    assert_eq!(
        call(&mut store, multivalue, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );
}

#[test]
fn p032_tail_recursive_arguments_overlap_safely_at_depth_two() {
    let engine = engine(2, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let tail = load(&engine, &mut store, TAIL_RECURSIVE);
    assert_eq!(
        call(&mut store, tail, "run", &[Value::I32(128)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(23)])
    );
    for n in [0, 1, 7] {
        assert_eq!(
            call(&mut store, tail, "run", &[Value::I32(n)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(23)])
        );
    }
}

#[test]
fn p032_imported_host_owns_arguments_during_nested_guest_reentry() {
    let engine = engine(32, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let nested_instance = load(&engine, &mut store, GUEST_FRAME);
    let nested = function(&store, nested_instance, "run");
    let observed = Arc::new(Mutex::new(None::<(Vec<Value>, RunOutcome)>));
    let in_callback = observed.clone();
    let host = store
        .create_host_func_with_caller(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            move |caller, arguments| {
                assert_eq!(arguments, &[Value::I32(5)]);
                let nested_outcome = caller.invoke(nested, &[])?;
                assert_eq!(nested_outcome, RunOutcome::Returned(vec![Value::I32(84)]));
                *in_callback.lock().unwrap() = Some((arguments.to_vec(), nested_outcome));
                Ok(vec![Value::I32(6)])
            },
        )
        .unwrap();
    let module = engine.compile(IMPORTED_DIRECT).unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("host", "bump", External::Func(host)).unwrap();
    let imported = linker.instantiate(&mut store, &module).unwrap();

    assert_eq!(
        call(&mut store, imported, "run", &[Value::I32(5)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(10)])
    );
    assert_eq!(
        *observed.lock().unwrap(),
        Some((
            vec![Value::I32(5)],
            RunOutcome::Returned(vec![Value::I32(84)])
        ))
    );
    assert_eq!(
        store.invoke(host, &[Value::I64(5)]).unwrap_err().kind,
        ErrorKind::Host
    );
}

#[test]
fn p032_public_reference_arguments_keep_nominal_and_store_checks() {
    let engine = engine(16, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let instance = load(&engine, &mut store, REFERENCE_CONTROL);
    assert_eq!(
        call(&mut store, instance, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(41)])
    );
    let after_run = store.collect().unwrap();
    assert_eq!((after_run.reclaimed, after_run.live), (1, 0));
    assert_eq!(store.gc_payload_bytes(), 0);

    let identity = function(&store, instance, "identity");
    let RunOutcome::Returned(s_values) = call(&mut store, instance, "make_s", &[]).unwrap() else {
        panic!("make_s did not return a reference");
    };
    let value_s = s_values[0];
    let RunOutcome::Returned(u_values) = call(&mut store, instance, "make_u", &[]).unwrap() else {
        panic!("make_u did not return a reference");
    };
    let value_u = u_values[0];
    assert!(matches!(value_s, Value::Ref(RefValue::Gc(_))));
    assert!(matches!(value_u, Value::Ref(RefValue::Gc(_))));
    assert_eq!(
        store.invoke(identity, &[Value::I32(41)]).unwrap_err().kind,
        ErrorKind::Host
    );
    assert_eq!(
        store.invoke(identity, &[value_u]).unwrap_err().kind,
        ErrorKind::Host
    );

    let mut foreign_store = Store::new(&engine);
    let foreign_instance = load(&engine, &mut foreign_store, REFERENCE_CONTROL);
    let RunOutcome::Returned(foreign_values) =
        call(&mut foreign_store, foreign_instance, "make_s", &[]).unwrap()
    else {
        panic!("foreign make_s did not return a reference");
    };
    let foreign_value = foreign_values[0];
    assert_eq!(
        store.invoke(identity, &[foreign_value]).unwrap_err().kind,
        ErrorKind::Host
    );

    store.release_published(&[value_s, value_u]);
    foreign_store.release_published(&[foreign_value]);
    let local = store.collect().unwrap();
    let foreign = foreign_store.collect().unwrap();
    assert_eq!((local.reclaimed, local.live), (2, 0));
    assert_eq!((foreign.reclaimed, foreign.live), (1, 0));
    assert_eq!(store.gc_payload_bytes(), 0);
    assert_eq!(foreign_store.gc_payload_bytes(), 0);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p032_reference_policy_repeats_public_guest_argument_results() {
    // Correctness compatibility only; this does not prove the candidate's
    // argument-range path or counter behavior.
    let engine = engine(128, ExecutionPolicy::Reference);
    let mut store = Store::new(&engine);
    let tail = load(&engine, &mut store, TAIL_RECURSIVE);
    assert_eq!(
        call(&mut store, tail, "run", &[Value::I32(128)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(23)])
    );
    let multi = load(&engine, &mut store, RECURSIVE_MULTIVALUE);
    assert_eq!(
        call(&mut store, multi, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );
}
