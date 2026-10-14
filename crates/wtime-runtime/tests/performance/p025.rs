use wtime_runtime::*;

const SCALAR_READS: &[u8] = include_bytes!("../fixtures/performance/p025/scalar_reads.wasm");
const BOUNDS: &[u8] = include_bytes!("../fixtures/performance/p025/bounds.wasm");
const SHARED_READ: &[u8] = include_bytes!("../fixtures/performance/p025/shared_read.wasm");

fn engine() -> Engine {
    Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap()
}

fn instantiate(engine: &Engine, bytes: &[u8]) -> (Store, Instance) {
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(engine);
    let instance = Linker::new(engine)
        .instantiate(&mut store, &module)
        .unwrap();
    drop(module);
    (store, instance)
}

fn exported_func(store: &Store, instance: Instance, name: &str) -> Func {
    match instance.export(store, name).unwrap() {
        External::Func(function) => function,
        other => panic!("expected function export, got {other:?}"),
    }
}

fn exported_memory(store: &Store, instance: Instance) -> Memory {
    match instance.export(store, "memory").unwrap() {
        External::Memory(memory) => memory,
        other => panic!("expected memory export, got {other:?}"),
    }
}

#[test]
fn p025_unaligned_scalar_widths_sign_extension_and_float_bits_match() {
    let engine = engine();
    let (mut store, instance) = instantiate(&engine, SCALAR_READS);
    let run = exported_func(&store, instance, "run");
    #[cfg(feature = "performance-counters")]
    store.reset_performance_counters();

    let actual = store.invoke(run, &[]).unwrap();
    assert_eq!(
        actual,
        RunOutcome::Returned(vec![
            Value::I32(-128),
            Value::I32(128),
            Value::I32(-32767),
            Value::I32(32769),
            Value::I32(-2147483647),
            Value::I64(-2),
            Value::I64(254),
            Value::I64(-32767),
            Value::I64(32769),
            Value::I64(-2147483647),
            Value::I64(2147483649),
            Value::I64(-9223372036854775807),
            Value::F32(0x7fc00001),
            Value::F64(0x7ff8000000000001),
        ])
    );

    #[cfg(feature = "performance-counters")]
    {
        let counters = store.performance_counters();
        // The allocation-removal candidate was rejected by its build budget.
        // Its fixed-read diagnostic is retained with the historical candidate;
        // the restored production path keeps the original allocating reads.
        assert_eq!(counters.read_fixed_operations, 0);
        assert_eq!(counters.read_vec_operations, 14);
    }

    // Host reads still return an owned Vec, independent from the memory.
    let memory = exported_memory(&store, instance);
    let mut bytes = store.read_memory(memory, 1, 1).unwrap();
    assert_eq!(bytes, [0x80]);
    bytes[0] = 0;
    assert_eq!(store.read_memory(memory, 1, 1).unwrap(), [0x80]);
}

#[test]
fn p025_load_boundaries_and_effective_address_overflow_preserve_prior_store() {
    let engine = engine();
    for export in ["boundary", "effective_overflow"] {
        let (mut store, instance) = instantiate(&engine, BOUNDS);
        let run = exported_func(&store, instance, export);
        let error = store.invoke(run, &[]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Trap, "{export}: {error}");
        assert_eq!(
            error.trap_code,
            Some(TrapCode::MemoryOutOfBounds),
            "{export}"
        );
        assert!(error.message.contains("out of bounds"), "{export}: {error}");

        let memory = exported_memory(&store, instance);
        assert_eq!(store.read_memory(memory, 8, 2).unwrap(), [0xaa, 0]);
    }
}

#[test]
fn p025_shared_ordinary_load_and_public_host_read_keep_their_contracts() {
    let engine = engine();
    let module = engine.compile(SHARED_READ).unwrap();
    let mut store = Store::new(&engine);
    let memory = store
        .create_memory(MemoryType {
            limits: Limits::new(1, Some(1)),
            index_type: IndexType::I32,
            shared: true,
            page_size_log2: 16,
        })
        .unwrap();
    store
        .write_memory(memory, 0, &[0xaa, 0x78, 0x56, 0x34, 0x12])
        .unwrap();
    let mut linker = Linker::new(&engine);
    linker
        .define("env", "memory", External::Memory(memory))
        .unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let run = exported_func(&store, instance, "run");
    #[cfg(feature = "performance-counters")]
    store.reset_performance_counters();

    assert_eq!(
        store.invoke(run, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(0x12345678)])
    );
    #[cfg(feature = "performance-counters")]
    {
        let before_host_read = store.performance_counters();
        assert_eq!(before_host_read.read_fixed_operations, 0);
        assert_eq!(before_host_read.read_vec_operations, 1);
    }

    let mut host_copy = store.read_memory(memory, 1, 4).unwrap();
    assert_eq!(host_copy, [0x78, 0x56, 0x34, 0x12]);
    #[cfg(feature = "performance-counters")]
    assert_eq!(store.performance_counters().read_vec_operations, 2);
    host_copy.fill(0);
    assert_eq!(
        store.read_memory(memory, 1, 4).unwrap(),
        [0x78, 0x56, 0x34, 0x12]
    );
}

#[test]
fn p025_memory64_checked_add_and_custom_page_boundaries_preserve_prefix() {
    let e = engine();
    let (mut s, i) = instantiate(
        &e,
        include_bytes!("../fixtures/performance/p025/memory64_boundary.wasm"),
    );
    assert_eq!(
        s.invoke(exported_func(&s, i, "last_byte"), &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(-128)])
    );
    let error = s.invoke(exported_func(&s, i, "overflow"), &[]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Trap);
    assert_eq!(error.trap_code, Some(TrapCode::MemoryOutOfBounds));
    assert_eq!(error.message, "address overflow");
    assert_eq!(
        s.read_memory(exported_memory(&s, i), 8, 2).unwrap(),
        [170, 0]
    );
    let (mut s, i) = instantiate(
        &e,
        include_bytes!("../fixtures/performance/p025/custom_page.wasm"),
    );
    let f = exported_func(&s, i, "run");
    assert_eq!(
        s.invoke(f, &[Value::I32(255)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(-128)])
    );
    assert_eq!(
        s.invoke(f, &[Value::I32(256)]).unwrap_err().trap_code,
        Some(TrapCode::MemoryOutOfBounds)
    );
    assert_eq!(
        s.read_memory(exported_memory(&s, i), 255, 1).unwrap(),
        [128]
    );
}
