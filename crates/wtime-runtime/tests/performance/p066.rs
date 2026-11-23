#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;

const CASES: [(&[u8], &[i32]); 3] = [
    (
        include_bytes!("../fixtures/performance/bench_wasm/representative/parser_hash.wasm"),
        &[16, 1024, 257],
    ),
    (
        include_bytes!("../fixtures/performance/bench_wasm/representative/image_filter.wasm"),
        &[16, 256, 73],
    ),
    (
        include_bytes!("../fixtures/performance/bench_wasm/representative/graph_bfs.wasm"),
        &[12, 64, 23],
    ),
];

fn engine(kind: usize, fuel: Option<u64>) -> Engine {
    let mut config = Config {
        features: FeatureSet::mvp(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    match kind {
        0 => Engine::new(config).unwrap(),
        1 => Engine::with_execution_policy(config, ExecutionPolicy::Register).unwrap(),
        _ => Engine::with_register_entry_threshold(config, 3).unwrap(),
    }
}

fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(value) = instance.export(store, name).unwrap() else {
        panic!()
    };
    value
}

fn memory(store: &Store, instance: Instance) -> Memory {
    let External::Memory(value) = instance.export(store, "memory").unwrap() else {
        panic!()
    };
    value
}

fn invoke(
    store: &mut Store,
    func: Func,
    args: &[Value],
) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    store.invoke(func, args).map_err(|e| (e.kind, e.message))
}

#[test]
fn p066_representative_fresh_store_and_hot_cache_equivalence() {
    let mut checked = 0;
    for (bytes, sizes) in CASES {
        for &size in sizes {
            let mut reference = None;
            for kind in 0..3 {
                let engine = engine(kind, None);
                let module = engine.compile(bytes).unwrap();
                let mut dispatches = 0;
                for _ in 0..5 {
                    let mut store = Store::new(&engine);
                    let instance = Linker::new(&engine)
                        .instantiate(&mut store, &module)
                        .unwrap();
                    let prepare = function(&store, instance, "prepare");
                    let run = function(&store, instance, "run");
                    let memory = memory(&store, instance);
                    invoke(&mut store, prepare, &[Value::I32(size)]).unwrap();
                    let result = invoke(&mut store, run, &[Value::I32(size)]).unwrap();
                    let image = store.read_memory(memory, 0, 65536).unwrap();
                    let observation = (result, image);
                    if let Some(expected) = &reference {
                        assert_eq!(&observation, expected);
                    } else {
                        reference = Some(observation);
                    }
                    dispatches += store.register_dispatches();
                    checked += 1;
                }
                if kind != 0 {
                    assert!(dispatches > 0, "requested Register path never executed");
                }
                if kind == 2 {
                    let snapshot = module.register_cache_snapshot().unwrap();
                    assert!(snapshot.lifecycle.published > 0);
                    assert_eq!(snapshot.active_body, None);
                }
            }
        }
    }
    assert_eq!(checked, 135);
    eprintln!(
        "P066_CASE fresh_store_calls={checked} policies=3 independent_model_in_production_protocol=true"
    );
}

fn partial_prepare(
    bytes: &[u8],
    size: i32,
    kind: usize,
    fuel: u64,
) -> (
    std::result::Result<RunOutcome, (ErrorKind, String)>,
    Vec<u8>,
) {
    let engine = engine(kind, Some(fuel));
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let prepare = function(&store, instance, "prepare");
    let memory = memory(&store, instance);
    store.write_memory(memory, 0, &vec![0x5a; 65536]).unwrap();
    let result = invoke(&mut store, prepare, &[Value::I32(size)]);
    (result, store.read_memory(memory, 0, 65536).unwrap())
}

#[test]
fn p066_representative_source_fuel_prefix_equivalence() {
    let mut changed = 0;
    for (bytes, sizes) in CASES {
        for fuel in 0..=128 {
            let reference = partial_prepare(bytes, sizes[0], 0, fuel);
            changed += usize::from(reference.1.iter().any(|&byte| byte != 0x5a));
            for kind in 1..3 {
                assert_eq!(partial_prepare(bytes, sizes[0], kind, fuel), reference);
            }
        }
    }
    assert!(
        changed > 0,
        "fuel tests never reached an observable memory write"
    );
    eprintln!("P066_CASE source_fuel_prefixes=387 policies=3 partial_memory_images={changed}");
}

#[test]
fn p066_representative_cancellation_and_error_memory() {
    for (bytes, sizes) in CASES {
        let mut reference = None;
        for kind in 0..3 {
            let engine = engine(kind, None);
            let module = engine.compile(bytes).unwrap();
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let prepare = function(&store, instance, "prepare");
            let run = function(&store, instance, "run");
            let memory = memory(&store, instance);
            invoke(&mut store, prepare, &[Value::I32(sizes[0])]).unwrap();
            let before = store.read_memory(memory, 0, 65536).unwrap();
            let invalid = invoke(&mut store, run, &[]);
            assert!(invalid.is_err());
            assert_eq!(store.read_memory(memory, 0, 65536).unwrap(), before);
            store.cancellation_token().cancel();
            let cancelled = invoke(&mut store, run, &[Value::I32(sizes[0])]);
            assert_eq!(
                cancelled,
                Err((ErrorKind::ResourceLimit, "execution cancelled".into()))
            );
            let after = store.read_memory(memory, 0, 65536).unwrap();
            assert_eq!(after, before);
            let observation = (invalid, cancelled, after);
            if let Some(expected) = &reference {
                assert_eq!(&observation, expected);
            } else {
                reference = Some(observation);
            }
        }
    }
    eprintln!("P066_CASE cancellation_and_argument_errors=18 full_memory_preserved=true");
}
