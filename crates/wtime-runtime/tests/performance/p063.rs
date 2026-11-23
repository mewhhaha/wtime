#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const LOADS: &[u8] = include_bytes!("../fixtures/performance/p063/scalar_loads.wasm");
const WIDTHS: &[u8] = include_bytes!("../fixtures/performance/p063/scalar-widths.wasm");
const STORES: &[u8] = include_bytes!("../fixtures/performance/p063/scalar_stores.wasm");
const EFFECTS: &[u8] = include_bytes!("../fixtures/performance/p063/prior-effects.wasm");
const BINDINGS: &[u8] = include_bytes!("../fixtures/performance/p063/unshared-bindings.wasm");
const POLICIES: &[ExecutionPolicy] = &[
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Register,
    #[cfg(feature = "compact-executor")]
    ExecutionPolicy::Compact,
];
fn engine(policy: ExecutionPolicy, fuel: Option<u64>) -> Engine {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Default::default()
    };
    config.limits.fuel = fuel;
    Engine::with_execution_policy(config, policy).unwrap()
}
fn loaded(engine: &Engine, bytes: &[u8]) -> (Store, Instance) {
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(engine);
    let instance = Linker::new(engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
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
fn run(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    store.invoke(function(store, instance, name), args)
}
fn observed(
    result: Result<RunOutcome>,
) -> std::result::Result<RunOutcome, (ErrorKind, String, Option<TrapCode>)> {
    result.map_err(|error| (error.kind, error.message, error.trap_code))
}
fn fuel_error() -> (ErrorKind, String, Option<TrapCode>) {
    (ErrorKind::ResourceLimit, "fuel exhausted".into(), None)
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut view = store.execution_snapshot().unwrap().clone();
    for frame in &mut view.frames {
        frame.module_fingerprint = 0;
    }
    // The diagnostic intentionally caps memory prefixes at 64 bytes. Check
    // full logical state here; each test separately compares complete images.
    assert!(view.pending_boundary_charged);
    assert_eq!(view.frames.len(), view.frame_count);
    assert_eq!(view.operands.len(), view.operand_count);
    for frame in &view.frames {
        assert_eq!(frame.locals.len(), frame.local_count);
        assert_eq!(frame.labels.len(), frame.label_count);
    }
    assert_eq!(view.memory_lengths, [65536]);
    assert_eq!(view.memory_prefixes[0].len(), 64);
    assert!(view.truncated);
    view
}
fn seed() -> Vec<u8> {
    let mut bytes = vec![0; 65536];
    bytes[..34].copy_from_slice(&[
        128, 127, 1, 128, 2, 3, 4, 128, 1, 128, 128, 1, 2, 128, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 128,
        63, 0, 0, 0, 0, 0, 0, 240, 63,
    ]);
    bytes
}

#[test]
fn p063_all_load_widths_and_signed_bits_match() {
    let cases = [
        ("i32_load8_s", 0, 1, Value::I32(-128)),
        ("i32_load8_u", 0, 1, Value::I32(128)),
        ("i32_load16_s", 1, 2, Value::I32(383)),
        ("i32_load16_u", 1, 2, Value::I32(383)),
        ("i32_load", 3, 4, Value::I32(0x0403_0280)),
        ("i64_load8_s", 7, 1, Value::I64(-128)),
        ("i64_load8_u", 7, 1, Value::I64(128)),
        ("i64_load16_s", 8, 2, Value::I64(-32767)),
        ("i64_load16_u", 8, 2, Value::I64(32769)),
        (
            "i64_load32_s",
            10,
            4,
            Value::I64(0x8002_0180u32 as i32 as i64),
        ),
        ("i64_load32_u", 10, 4, Value::I64(0x8002_0180)),
        ("i64_load", 14, 8, Value::I64(0x0807_0605_0403_0201)),
        ("f32_load", 22, 4, Value::F32(1f32.to_bits())),
        ("f64_load", 26, 8, Value::F64(1f64.to_bits())),
    ];
    let mut prefixes = 0;
    for (name, address, width, expected) in cases {
        for fuel in 0..=4 {
            let mut outcomes = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(fuel)), LOADS);
                let mem = memory(&store, instance);
                let function = function(&store, instance, name);
                if (1..=3).contains(&fuel) {
                    store
                        .request_materialization_roundtrip(function, fuel as usize - 1)
                        .unwrap();
                }
                let result = observed(store.invoke(function, &[Value::I32(address)]));
                assert_eq!(
                    result,
                    if fuel < 3 {
                        Err(fuel_error())
                    } else {
                        Ok(RunOutcome::Returned(vec![expected]))
                    }
                );
                let view = if (1..=3).contains(&fuel) {
                    Some(snapshot(&store))
                } else {
                    None
                };
                assert_eq!(store.read_memory(mem, 0, 65536).unwrap(), seed());
                if policy == ExecutionPolicy::Register {
                    assert_eq!(store.register_dispatches(), fuel.min(3));
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                outcomes.push((result, view));
            }
            for result in &outcomes[1..] {
                assert_eq!(&outcomes[0], result, "{name} fuel{fuel}");
            }
            prefixes += usize::from((1..=3).contains(&fuel));
        }
        for &policy in POLICIES {
            let (mut store, instance) = loaded(&engine(policy, None), LOADS);
            let zero = match expected {
                Value::I32(_) => Value::I32(0),
                Value::I64(_) => Value::I64(0),
                Value::F32(_) => Value::F32(0),
                Value::F64(_) => Value::F64(0),
                _ => panic!(),
            };
            assert_eq!(
                run(&mut store, instance, name, &[Value::I32(65536 - width)]).unwrap(),
                RunOutcome::Returned(vec![zero])
            );
            for address in [65537 - width, -1] {
                assert_eq!(
                    run(&mut store, instance, name, &[Value::I32(address)])
                        .unwrap_err()
                        .trap_code,
                    Some(TrapCode::MemoryOutOfBounds)
                );
            }
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), 7);
                assert_eq!(store.register_fallback_frames(), 0);
            }
        }
    }
    assert_eq!(prefixes, 42);
    eprintln!(
        "P063: {prefixes} load source snapshots; all 14 typed load kinds and full-width memory-end bounds"
    );
}

#[test]
fn p063_all_store_widths_and_fuel_images_match() {
    let args = [
        Value::I32(-2),
        Value::I64(0x0102_0304_8001_80ff),
        Value::F32(0x8000_0000),
        Value::F64(0xfff8_0000_0001_2345),
    ];
    let mut prefixes = 0;
    for fuel in 0..=29 {
        let oracle = std::fs::read(format!(
            "{}/tests/fixtures/performance/p063/expected-memory/store-kinds/fuel-{fuel:02}.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(oracle.len(), 65536);
        let mut outcomes = vec![];
        for &policy in POLICIES {
            let (mut store, instance) = loaded(&engine(policy, Some(fuel)), STORES);
            let mem = memory(&store, instance);
            let function = function(&store, instance, "store_kinds");
            if (1..=28).contains(&fuel) {
                store
                    .request_materialization_roundtrip(function, fuel as usize - 1)
                    .unwrap();
            }
            let result = observed(store.invoke(function, &args));
            assert_eq!(
                result,
                if fuel < 28 {
                    Err(fuel_error())
                } else {
                    Ok(RunOutcome::Returned(vec![]))
                }
            );
            assert_eq!(
                store.read_memory(mem, 0, 65536).unwrap(),
                oracle,
                "fuel{fuel} {policy:?}"
            );
            let view = if (1..=28).contains(&fuel) {
                Some(snapshot(&store))
            } else {
                None
            };
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), fuel.min(28));
                assert_eq!(store.register_fallback_frames(), 0);
            }
            outcomes.push((result, view));
        }
        for result in &outcomes[1..] {
            assert_eq!(&outcomes[0], result, "fuel{fuel}");
        }
        prefixes += usize::from((1..=28).contains(&fuel));
    }
    assert_eq!(prefixes, 28);
    eprintln!(
        "P063: {prefixes} store source snapshots; 30 full memory-prefix images across all nine stores"
    );
}

#[test]
fn p063_traps_preserve_prior_effects_and_boundary_priority() {
    for fuel in 0..=8 {
        let mut outcomes = vec![];
        for &policy in POLICIES {
            let (mut store, instance) = loaded(&engine(policy, Some(fuel)), EFFECTS);
            let mem = memory(&store, instance);
            let function = function(&store, instance, "run");
            if (1..=5).contains(&fuel) {
                store
                    .request_materialization_roundtrip(function, fuel as usize - 1)
                    .unwrap();
            }
            let result = observed(store.invoke(function, &[]));
            assert_eq!(
                result,
                if fuel < 5 {
                    Err(fuel_error())
                } else {
                    Err((
                        ErrorKind::Trap,
                        "out of bounds".into(),
                        Some(TrapCode::MemoryOutOfBounds),
                    ))
                }
            );
            let mut expected = vec![0; 65536];
            if fuel >= 3 {
                expected[8] = 0xaa;
            }
            assert_eq!(store.read_memory(mem, 0, 65536).unwrap(), expected);
            let view = if (1..=5).contains(&fuel) {
                Some(snapshot(&store))
            } else {
                None
            };
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), fuel.min(5));
                assert_eq!(store.register_fallback_frames(), 0);
            }
            outcomes.push((result, view));
        }
        for result in &outcomes[1..] {
            assert_eq!(&outcomes[0], result);
        }
    }
    let (mut store, instance) = loaded(&engine(ExecutionPolicy::Register, None), EFFECTS);
    store.cancellation_token().cancel();
    assert_eq!(
        run(&mut store, instance, "run", &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(store.register_dispatches(), 0);
    assert_eq!(
        store
            .read_memory(memory(&store, instance), 0, 65536)
            .unwrap(),
        vec![0; 65536]
    );
    eprintln!(
        "P063: five trapping source snapshots; earlier store survives; failing/later accesses absent; fuel/cancel precede effects"
    );
}

#[test]
fn p063_shared_wide_and_custom_pages_decline() {
    for (bytes, name) in [
        (
            include_bytes!("../fixtures/performance/p043/memory64-overflow.wasm").as_slice(),
            "effective_address_overflow",
        ),
        (
            include_bytes!("../fixtures/performance/p043/custom-page-one-byte.wasm").as_slice(),
            "write_last_byte",
        ),
    ] {
        for fuel in 0..=7 {
            let mut outcomes = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, Some(fuel)), bytes);
                let result = observed(run(&mut store, instance, name, &[]));
                let mem = memory(&store, instance);
                let image = store
                    .read_memory(
                        mem,
                        0,
                        if name == "write_last_byte" {
                            256
                        } else {
                            65536
                        },
                    )
                    .unwrap();
                if policy == ExecutionPolicy::Register {
                    assert_eq!(store.register_dispatches(), 0);
                    assert_eq!(store.register_fallback_frames(), 1);
                }
                outcomes.push((result, image));
            }
            for result in &outcomes[1..] {
                assert_eq!(&outcomes[0], result);
            }
        }
    }
    for &policy in POLICIES {
        let engine = engine(policy, None);
        let mut store = Store::new(&engine);
        let ty = MemoryType {
            index_type: IndexType::I32,
            limits: Limits::new(1, Some(2)),
            shared: true,
            page_size_log2: 16,
        };
        let left = store.create_memory(ty).unwrap();
        let right = store.create_memory(ty).unwrap();
        store.write_memory(left, 0, &[11]).unwrap();
        store.write_memory(right, 0, &[99]).unwrap();
        let mut linker = Linker::new(&engine);
        linker
            .define("mem", "left", External::Memory(left))
            .unwrap();
        linker
            .define("mem", "right", External::Memory(right))
            .unwrap();
        let module = engine
            .compile(include_bytes!(
                "../fixtures/performance/p043/shared-multi-memory.wasm"
            ))
            .unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        assert_eq!(
            run(&mut store, instance, "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(11)])
        );
        assert_eq!(store.read_memory(right, 0, 1).unwrap(), [99]);
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 0);
            assert_eq!(store.register_fallback_frames(), 1);
        }
    }
    // An atomic access can promote an ordinary unshared backing between calls.
    // A later scalar frame must decline instead of accessing the old Vec.
    for &policy in POLICIES {
        let (mut store, instance) = loaded(&engine(policy, None), WIDTHS);
        assert_eq!(
            run(&mut store, instance, "atomic_promote", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(128)])
        );
        assert_eq!(
            run(&mut store, instance, "float_raw_bits", &[]).unwrap(),
            RunOutcome::Returned(vec![
                Value::I32(1f32.to_bits() as i32),
                Value::I64(1f64.to_bits() as i64)
            ])
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 0);
            assert_eq!(store.register_fallback_frames(), 2);
        }
        assert_eq!(
            store
                .read_memory(memory(&store, instance), 0, 65536)
                .unwrap(),
            seed()
        );
    }
}

#[test]
fn p063_bindings_growth_and_aliases_use_current_bytes() {
    for &policy in POLICIES {
        let engine = engine(policy, None);
        let mut store = Store::new(&engine);
        let ty = MemoryType {
            index_type: IndexType::I32,
            limits: Limits::new(1, Some(2)),
            shared: false,
            page_size_log2: 16,
        };
        let left = store.create_memory(ty).unwrap();
        let right = store.create_memory(ty).unwrap();
        store.write_memory(left, 0, &[11, 128, 1, 2, 3]).unwrap();
        store.write_memory(right, 0, &[99, 7, 8, 9, 10]).unwrap();
        let module = engine.compile(BINDINGS).unwrap();
        let mut instances = vec![];
        for (a, b) in [(left, right), (right, left), (left, left)] {
            let mut linker = Linker::new(&engine);
            linker.define("mem", "left", External::Memory(a)).unwrap();
            linker.define("mem", "right", External::Memory(b)).unwrap();
            instances.push(linker.instantiate(&mut store, &module).unwrap());
        }
        assert_eq!(
            run(&mut store, instances[0], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(11)])
        );
        assert_eq!(
            run(&mut store, instances[1], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(99)])
        );
        run(
            &mut store,
            instances[2],
            "write_right",
            &[Value::I32(0), Value::I32(42)],
        )
        .unwrap();
        assert_eq!(
            run(&mut store, instances[0], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        assert_eq!(
            run(
                &mut store,
                instances[0],
                "read_unaligned_left_i32",
                &[Value::I32(1)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(0x0302_0180)])
        );
        let before = store.read_memory(left, 0, 65536).unwrap();
        assert_eq!(
            run(
                &mut store,
                instances[0],
                "write_left",
                &[Value::I32(65536), Value::I32(123)]
            )
            .unwrap_err()
            .trap_code,
            Some(TrapCode::MemoryOutOfBounds)
        );
        assert_eq!(store.read_memory(left, 0, 65536).unwrap(), before);
        if policy == ExecutionPolicy::Register {
            assert!(store.register_dispatches() > 0);
            assert_eq!(store.register_fallback_frames(), 0);
        }
        let grow = function(&store, instances[0], "grow_and_seed");
        let host = store
            .create_host_func_with_caller(
                FuncType {
                    params: vec![],
                    results: vec![],
                },
                move |caller, _| {
                    assert_eq!(
                        caller.invoke(grow, &[])?,
                        RunOutcome::Returned(vec![Value::I32(77)])
                    );
                    Ok(vec![])
                },
            )
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker
            .define("mem", "left", External::Memory(left))
            .unwrap();
        linker
            .define("env", "grow_and_seed", External::Func(host))
            .unwrap();
        let caller = linker
            .instantiate(
                &mut store,
                &engine
                    .compile(include_bytes!(
                        "../fixtures/performance/p063/growth-caller.wasm"
                    ))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            run(&mut store, caller, "read_after_host_growth", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        let count = store.register_dispatches();
        assert_eq!(
            run(&mut store, instances[0], "read_left", &[Value::I32(65536)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches() - count, 3);
            assert_eq!(store.register_fallback_frames(), 2);
        }
        assert_eq!(store.read_memory(left, 65536, 1).unwrap(), [77]);
        assert_eq!(store.read_memory(right, 0, 5).unwrap(), [99, 7, 8, 9, 10]);
    }
    for bytes in [
        include_bytes!("../fixtures/performance/p063/benchmark_predictable.wasm").as_slice(),
        include_bytes!("../fixtures/performance/p063/benchmark_shuffled.wasm").as_slice(),
        include_bytes!("../fixtures/performance/p063/benchmark_read_scan.wasm").as_slice(),
    ] {
        for n in [0, 17, 1024] {
            let mut outcomes = vec![];
            for &policy in POLICIES {
                let (mut store, instance) = loaded(&engine(policy, None), bytes);
                let result = observed(run(&mut store, instance, "run", &[Value::I32(n)]));
                assert!(result.is_ok());
                let image = store
                    .read_memory(memory(&store, instance), 0, 65536)
                    .unwrap();
                if policy == ExecutionPolicy::Register {
                    assert!(store.register_dispatches() > 0);
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                outcomes.push((result, image));
            }
            for outcome in &outcomes[1..] {
                assert_eq!(&outcomes[0], outcome);
            }
        }
    }
    eprintln!(
        "P063: nine actual width/read-scan kernel cases execute Register; complete memory matches Decoded"
    );
}
