// P031 public API behavioral checks.
// Private ActivationPool reuse/retention assertions stay in runtime tests.
use std::sync::{Arc, Mutex};
use wtime_runtime::*;

const GUEST_FRAME: &[u8] = include_bytes!("../fixtures/performance/frames/guest_frame.wasm");
const IMPORT_CONTROL: &[u8] = include_bytes!("../fixtures/performance/frames/import_control.wasm");
const RECURSION: &[u8] = include_bytes!("../fixtures/performance/frames/recursion.wasm");
const CAUGHT_GC: &[u8] = include_bytes!("../fixtures/exception_root/caught_ref_to_function.wasm");
const PROVIDER: &[u8] =
    include_bytes!("../fixtures/performance/bench_wasm/integration/cross_module_provider.wasm");
const CONSUMER: &[u8] =
    include_bytes!("../fixtures/performance/bench_wasm/integration/cross_module_consumer.wasm");

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

#[test]
fn p031_nested_host_reentry_preserves_outer_arguments_and_result() {
    let engine = engine(16, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let nested_instance = load(&engine, &mut store, GUEST_FRAME);
    let nested = function(&store, nested_instance, "run");
    let observed = Arc::new(Mutex::new(None::<(i32, RunOutcome)>));
    let in_callback = observed.clone();
    let host = store
        .create_host_func_with_caller(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            move |caller, args| {
                let Value::I32(outer_arg) = args[0] else {
                    panic!("host expected the outer i32 argument");
                };
                let nested_result = caller.invoke(nested, &[])?;
                assert_eq!(nested_result, RunOutcome::Returned(vec![Value::I32(84)]));
                *in_callback.lock().unwrap() = Some((outer_arg, nested_result));
                Ok(vec![Value::I32(outer_arg + 1)])
            },
        )
        .unwrap();
    let outer_module = engine.compile(IMPORT_CONTROL).unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("env", "bump", External::Func(host)).unwrap();
    let outer = linker.instantiate(&mut store, &outer_module).unwrap();

    assert_eq!(
        call(&mut store, outer, "run", &[Value::I32(5)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(6)])
    );
    assert_eq!(
        *observed.lock().unwrap(),
        Some((5, RunOutcome::Returned(vec![Value::I32(84)])))
    );
}

#[test]
fn p031_errors_return_to_reuse_and_caught_gc_result_releases_exactly() {
    let engine = engine(2, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let recursion = load(&engine, &mut store, RECURSION);
    let depth_error = call(&mut store, recursion, "ordinary", &[Value::I32(7)]).unwrap_err();
    assert_eq!(depth_error.kind, ErrorKind::ResourceLimit);

    // Minimal validated MVP () -> () body with unreachable followed by end.
    let trap_module = load(
        &engine,
        &mut store,
        &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 7, 7, 1, 3, 114, 117, 110,
            0, 0, 10, 5, 1, 3, 0, 0, 11,
        ],
    );
    assert_eq!(
        call(&mut store, trap_module, "run", &[]).unwrap_err().kind,
        ErrorKind::Trap
    );

    let host = store
        .create_host_func(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            |_| Err(Error::new(ErrorKind::Host, "p031 test callback failure")),
        )
        .unwrap();
    let imported_module = engine.compile(IMPORT_CONTROL).unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("env", "bump", External::Func(host)).unwrap();
    let imported = linker.instantiate(&mut store, &imported_module).unwrap();
    let host_error = call(&mut store, imported, "run", &[Value::I32(5)]).unwrap_err();
    assert_eq!(host_error.kind, ErrorKind::Host);
    assert_eq!(host_error.message, "p031 test callback failure");

    let plain = load(&engine, &mut store, GUEST_FRAME);
    assert_eq!(
        call(&mut store, plain, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(84)])
    );

    let caught = load(&engine, &mut store, CAUGHT_GC);
    let RunOutcome::Returned(values) = call(&mut store, caught, "run", &[]).unwrap() else {
        panic!("caught exception did not return its reference");
    };
    assert_eq!(values.len(), 1);
    assert!(matches!(values[0], Value::Ref(RefValue::Gc(_))));
    assert_eq!(store.collect().unwrap().live, 1);
    assert_eq!(
        call(&mut store, caught, "read", &values).unwrap(),
        RunOutcome::Returned(vec![Value::I32(91)])
    );
    store.release_published(&values);
    let collected = store.collect().unwrap();
    assert_eq!((collected.reclaimed, collected.live), (1, 0));
    assert_eq!(store.gc_payload_bytes(), 0);
}

#[test]
fn p031_detached_continuation_survives_unrelated_call_and_is_one_shot() {
    let engine = engine(16, ExecutionPolicy::Decoded);
    let mut store = Store::new(&engine);
    let provider = load(&engine, &mut store, PROVIDER);
    let consumer_module = engine.compile(CONSUMER).unwrap();
    let mut linker = Linker::new(&engine);
    linker
        .define(
            "provider",
            "yield",
            provider.export(&store, "yield").unwrap(),
        )
        .unwrap();
    let consumer = linker.instantiate(&mut store, &consumer_module).unwrap();
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = call(&mut store, consumer, "run", &[]).unwrap()
    else {
        panic!("expected detached continuation");
    };
    assert!(payload.is_empty());
    let captured_bytes = store.continuation_bytes();
    assert!(captured_bytes > 0);

    let root = store.root(continuation).unwrap();
    let continuation_arg = Value::Ref(RefValue::Gc(root.reference()));
    store.release_published(&[continuation_arg]);
    let unrelated = load(&engine, &mut store, GUEST_FRAME);
    assert_eq!(
        call(&mut store, unrelated, "run", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(84)])
    );
    assert!(store.collect().unwrap().live > 0);

    assert_eq!(
        call(&mut store, consumer, "resume", &[continuation_arg]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(77)])
    );
    assert_eq!(
        call(&mut store, consumer, "resume", &[continuation_arg])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    drop(root);
    let collected = store.collect().unwrap();
    assert_eq!(collected.live, 0);
    assert_eq!(store.continuation_bytes(), 0);
    assert_eq!(store.gc_payload_bytes(), 0);
}

#[cfg(feature = "execution-reference")]
#[test]
fn p031_reference_policy_runs_the_same_public_reentry_case() {
    // This is a semantic compatibility check, not an independent oracle for
    // activation-pool allocation or retention.
    let engine = engine(16, ExecutionPolicy::Reference);
    let mut store = Store::new(&engine);
    let nested_instance = load(&engine, &mut store, GUEST_FRAME);
    let nested = function(&store, nested_instance, "run");
    let host = store
        .create_host_func_with_caller(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            move |caller, args| {
                assert_eq!(
                    caller.invoke(nested, &[])?,
                    RunOutcome::Returned(vec![Value::I32(84)])
                );
                let Value::I32(value) = args[0] else {
                    panic!("expected i32")
                };
                Ok(vec![Value::I32(value + 1)])
            },
        )
        .unwrap();
    let module = engine.compile(IMPORT_CONTROL).unwrap();
    let mut linker = Linker::new(&engine);
    linker.define("env", "bump", External::Func(host)).unwrap();
    let outer = linker.instantiate(&mut store, &module).unwrap();
    assert_eq!(
        call(&mut store, outer, "run", &[Value::I32(5)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(6)])
    );
}
