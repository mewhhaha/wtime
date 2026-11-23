//! Source-only P043 tests. These become evidence only after fresh fixture validation.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;

const WIDTHS: &[u8] = include_bytes!("../fixtures/performance/p043/scalar-widths.wasm");
const BOUNDARIES: &[u8] = include_bytes!("../fixtures/performance/p043/access-boundaries.wasm");
const WIDE: &[u8] = include_bytes!("../fixtures/performance/p043/memory64-overflow.wasm");
const CUSTOM: &[u8] = include_bytes!("../fixtures/performance/p043/custom-page-one-byte.wasm");
const SHARED: &[u8] = include_bytes!("../fixtures/performance/p043/shared-multi-memory.wasm");
const GROWTH: &[u8] = include_bytes!("../fixtures/performance/p043/growth-caller.wasm");
const POLICIES: [ExecutionPolicy; 3] = [
    ExecutionPolicy::Reference,
    ExecutionPolicy::Decoded,
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
fn load(engine: &Engine, bytes: &[u8]) -> (Store, Instance) {
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(engine);
    let instance = Linker::new(engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
}
fn func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(value) = instance.export(store, name).unwrap() else {
        panic!()
    };
    value
}
fn memory(store: &Store, instance: Instance, name: &str) -> Memory {
    let External::Memory(value) = instance.export(store, name).unwrap() else {
        panic!()
    };
    value
}
fn run(store: &mut Store, instance: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    store.invoke(func(store, instance, name), args)
}
fn fuel_error(value: Result<RunOutcome>) {
    let error = value.unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (ErrorKind::ResourceLimit, "fuel exhausted")
    );
}
fn seed() -> Vec<u8> {
    let mut bytes = vec![0; 65536];
    bytes[..34].copy_from_slice(&[
        0x80, 0x7f, 1, 0x80, 2, 3, 4, 0x80, 1, 0x80, 0x80, 1, 2, 0x80, 1, 2, 3, 4, 5, 6, 7, 8, 0,
        0, 0x80, 0x3f, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f,
    ]);
    bytes
}
fn compact_counts(store: &Store, policy: ExecutionPolicy, sources: u64, accesses: u64) {
    assert_eq!(
        store.compact_dispatches(),
        if policy == ExecutionPolicy::Compact {
            sources
        } else {
            0
        }
    );
    assert_eq!(
        store.compact_memory_accesses(),
        if policy == ExecutionPolicy::Compact {
            accesses
        } else {
            0
        }
    );
}

#[test]
fn p043_every_scalar_load_width_sign_and_raw_float_has_exact_fuel_prefix() {
    let expected = vec![
        Value::I32(-128),
        Value::I32(128),
        Value::I32(383),
        Value::I32(383),
        Value::I32(0x04030280),
        Value::I64(-128),
        Value::I64(128),
        Value::I64(-32767),
        Value::I64(32769),
        Value::I64(0x80020180u32 as i32 as i64),
        Value::I64(0x80020180),
        Value::I64(0x0807060504030201),
        Value::F32(0x3f800000),
        Value::F64(0x3ff0000000000000),
    ];
    for fuel in 0u64..=30 {
        for policy in POLICIES {
            let e = engine(policy, Some(fuel));
            let (mut s, i) = load(&e, WIDTHS);
            let m = memory(&s, i, "memory");
            let result = run(&mut s, i, "run", &[]);
            if fuel < 29 {
                fuel_error(result);
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(expected.clone()));
            }
            compact_counts(&s, policy, fuel.min(29), (fuel / 2).min(14));
            // Observe guest counters before the public full-image read, which
            // deliberately records its own allocating diagnostic read.
            #[cfg(feature = "performance-counters")]
            if policy == ExecutionPolicy::Compact {
                assert_eq!(s.performance_counters().read_vec_operations, 0);
                assert_eq!(
                    s.performance_counters().read_fixed_operations,
                    (fuel / 2).min(14)
                );
            }
            assert_eq!(s.read_memory(m, 0, 65536).unwrap(), seed());
        }
    }
}

#[test]
fn p043_all_nine_store_kinds_have_independent_full_memory_prefix_images() {
    let args = [
        Value::I32(-2),
        Value::I64(0x01020304800180ff),
        Value::F32(0x80000000),
        Value::F64(0xfff8000000012345),
    ];
    for fuel in 0u64..=29 {
        let oracle = std::fs::read(format!(
            "{}/tests/fixtures/performance/p043/expected-memory/store-kinds/fuel-{fuel:02}.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(oracle.len(), 65536);
        for policy in POLICIES {
            let e = engine(policy, Some(fuel));
            let (mut s, i) = load(&e, WIDTHS);
            let m = memory(&s, i, "memory");
            let result = run(&mut s, i, "store_kinds", &args);
            if fuel < 28 {
                fuel_error(result);
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![]));
            }
            assert_eq!(
                s.read_memory(m, 0, 65536).unwrap(),
                oracle,
                "{policy:?} fuel={fuel}"
            );
            compact_counts(&s, policy, fuel.min(28), (fuel / 3).min(9));
        }
    }
}

#[test]
fn p043_full_boundary_ranges_trap_before_side_effect_and_vector_memory_falls_back() {
    for policy in POLICIES {
        let e = engine(policy, None);
        let (mut s, i) = load(&e, BOUNDARIES);
        let m = memory(&s, i, "memory");
        let before = s.read_memory(m, 0, 65536).unwrap();
        for (name, want) in [
            ("first_bytes", vec![Value::I32(127), Value::I32(-128)]),
            ("last_valid_byte", vec![Value::I32(0)]),
            ("last_valid_unaligned_i32", vec![Value::I32(0)]),
            ("last_valid_i64", vec![Value::I64(0)]),
        ] {
            assert_eq!(
                run(&mut s, i, name, &[]).unwrap(),
                RunOutcome::Returned(want)
            );
        }
        for name in [
            "oob_i16_at_last_byte",
            "oob_i32_crossing_end",
            "oob_i64_crossing_end",
        ] {
            let error = run(&mut s, i, name, &[]).unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (ErrorKind::Trap, "out of bounds")
            );
        }
        assert_eq!(s.read_memory(m, 0, 65536).unwrap(), before);
        assert_eq!(
            s.compact_memory_accesses(),
            if policy == ExecutionPolicy::Compact {
                5
            } else {
                0
            }
        );
        let (mut vector, vi) = load(
            &e,
            include_bytes!("../fixtures/performance/simd/extension_loads.wasm"),
        );
        assert_eq!(
            run(&mut vector, vi, "v128.load8x8_u", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::V128(V128([
                128, 0, 255, 0, 127, 0, 129, 0, 254, 0, 2, 0, 128, 0, 255, 0
            ]))])
        );
        compact_counts(&vector, policy, 1, 0);
        if policy == ExecutionPolicy::Compact {
            assert_eq!(
                vector.compact_prefix_counts(),
                (1, [0, 0, 0, 1], Some((1, 0xfd02)))
            );
        }
    }
}

#[test]
fn p043_memory64_overflow_trap_keeps_every_preceding_store_and_fuel_priority() {
    for fuel in 0u64..=12 {
        for policy in POLICIES {
            let e = engine(policy, Some(fuel));
            let (mut s, i) = load(&e, WIDE);
            let m = memory(&s, i, "memory");
            let result = run(&mut s, i, "effective_address_overflow", &[]);
            if fuel < 5 {
                fuel_error(result);
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    (error.kind, error.message.as_str()),
                    (ErrorKind::Trap, "address overflow")
                );
            }
            let mut expected = vec![0; 65536];
            if fuel >= 3 {
                expected[8] = 0xaa;
            }
            assert_eq!(s.read_memory(m, 0, 65536).unwrap(), expected);
            compact_counts(&s, policy, fuel.min(5), u64::from(fuel >= 3));
        }
    }
    for policy in POLICIES {
        let e = engine(policy, None);
        let (mut s, i) = load(&e, WIDE);
        assert_eq!(
            run(&mut s, i, "last_byte", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(-128)])
        );
        let before = s.read_memory(memory(&s, i, "memory"), 0, 65536).unwrap();
        assert_eq!(
            run(&mut s, i, "last_valid_i64", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I64(i64::MIN)])
        );
        assert_eq!(
            run(&mut s, i, "first_oob_i64", &[]).unwrap_err().trap_code,
            Some(TrapCode::MemoryOutOfBounds)
        );
        assert_eq!(
            s.read_memory(memory(&s, i, "memory"), 0, 65536).unwrap(),
            before
        );
    }
}

#[test]
fn p043_one_byte_pages_use_byte_lengths_and_preserve_last_byte_store_prefix() {
    for fuel in 0u64..=7 {
        for policy in POLICIES {
            let e = engine(policy, Some(fuel));
            let (mut s, i) = load(&e, CUSTOM);
            let m = memory(&s, i, "memory");
            let result = run(&mut s, i, "write_last_byte", &[]);
            if fuel < 6 {
                fuel_error(result);
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(127)]));
            }
            let mut expected = vec![0; 256];
            expected[..3].copy_from_slice(&[128, 1, 2]);
            if fuel >= 3 {
                expected[255] = 127;
            }
            assert_eq!(s.read_memory(m, 0, 256).unwrap(), expected);
            compact_counts(
                &s,
                policy,
                fuel.min(6),
                u64::from(fuel >= 3) + u64::from(fuel >= 5),
            );
        }
    }
    let e = engine(ExecutionPolicy::Compact, None);
    let (mut s, i) = load(&e, CUSTOM);
    assert_eq!(
        run(&mut s, i, "read_seed", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(-128), Value::I32(513)])
    );
    assert_eq!(
        run(&mut s, i, "oob_i16_at_last_byte", &[])
            .unwrap_err()
            .trap_code,
        Some(TrapCode::MemoryOutOfBounds)
    );
    assert_eq!(s.compact_memory_accesses(), 2);
}

#[test]
fn p043_shared_aliases_distinct_instances_and_host_growth_refresh_live_ranges() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let e = engine(policy, None);
        let mut s = Store::new(&e);
        let ty = MemoryType {
            index_type: IndexType::I32,
            limits: Limits::new(1, Some(2)),
            shared: true,
            page_size_log2: 16,
        };
        let left = s.create_memory(ty).unwrap();
        let right = s.create_memory(ty).unwrap();
        let token = s.share_memory(left).unwrap();
        let alias = s.import_shared_memory(&token).unwrap();
        s.write_memory(left, 0, &[11, 0x80, 1, 2, 3]).unwrap();
        s.write_memory(right, 0, &[99, 7, 8, 9, 10]).unwrap();
        let module = e.compile(SHARED).unwrap();
        let mut exports = vec![];
        for (a, b) in [(left, right), (right, alias)] {
            let mut linker = Linker::new(&e);
            linker.define("mem", "left", External::Memory(a)).unwrap();
            linker.define("mem", "right", External::Memory(b)).unwrap();
            exports.push(linker.instantiate(&mut s, &module).unwrap());
        }
        assert_eq!(
            run(&mut s, exports[0], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(11)])
        );
        assert_eq!(
            run(&mut s, exports[1], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(99)])
        );
        run(
            &mut s,
            exports[1],
            "write_right",
            &[Value::I32(0), Value::I32(42)],
        )
        .unwrap();
        assert_eq!(
            run(&mut s, exports[0], "read_left", &[Value::I32(0)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        assert_eq!(
            run(
                &mut s,
                exports[0],
                "read_unaligned_left_i32",
                &[Value::I32(1)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(0x03020180)])
        );
        let before = s.read_memory(left, 0, 65536).unwrap();
        let error = run(
            &mut s,
            exports[0],
            "write_left",
            &[Value::I32(65536), Value::I32(123)],
        )
        .unwrap_err();
        assert_eq!(error.trap_code, Some(TrapCode::MemoryOutOfBounds));
        assert_eq!(s.read_memory(alias, 0, 65536).unwrap(), before);
        let grow = func(&s, exports[0], "grow_and_seed");
        let host = s
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
        let mut linker = Linker::new(&e);
        linker
            .define("mem", "left", External::Memory(alias))
            .unwrap();
        linker
            .define("env", "grow_and_seed", External::Func(host))
            .unwrap();
        let caller = linker
            .instantiate(&mut s, &e.compile(GROWTH).unwrap())
            .unwrap();
        assert_eq!(
            run(&mut s, caller, "read_before_reentry", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        assert_eq!(
            run(&mut s, caller, "read_after_host_growth", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(s.read_memory(left, 65536, 1).unwrap(), [77]);
        assert_eq!(s.read_memory(right, 0, 5).unwrap(), [99, 7, 8, 9, 10]);
        if policy == ExecutionPolicy::Compact {
            assert!(s.compact_memory_accesses() >= 7);
            assert!(s.compact_bound_calls() > 0);
        }
        #[cfg(feature = "performance-counters")]
        if policy == ExecutionPolicy::Compact {
            assert!(s.performance_counters().compact_shared_memory_accesses >= 7);
        }
    }
}

#[test]
fn p043_unshared_growth_and_memory_cursors_materialize_identical_source_state() {
    let fixture = include_bytes!("../fixtures/memory-proposals.wasm");
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let e = engine(policy, None);
        let (mut s, i) = load(&e, fixture);
        let wide = memory(&s, i, "wide");
        assert_eq!(
            run(
                &mut s,
                i,
                "write-read",
                &[Value::I64(65528), Value::I64(0x0102030405060708)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I64(0x0102030405060708)])
        );
        assert_eq!(
            run(&mut s, i, "wide-grow", &[Value::I64(1)]).unwrap(),
            RunOutcome::Returned(vec![Value::I64(1)])
        );
        assert_eq!(
            run(
                &mut s,
                i,
                "write-read",
                &[Value::I64(65536), Value::I64(-17)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I64(-17)])
        );
        assert_eq!(
            s.read_memory(wide, 65536, 8).unwrap(),
            (-17i64).to_le_bytes()
        );
        assert_eq!(
            s.compact_memory_accesses(),
            if policy == ExecutionPolicy::Compact {
                4
            } else {
                0
            }
        );
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let e = engine(policy, None);
        let (mut s, i) = load(&e, WIDTHS);
        assert_eq!(
            run(&mut s, i, "atomic_promote", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(128)])
        );
        compact_counts(&s, policy, u64::from(policy == ExecutionPolicy::Compact), 0);
        #[cfg(feature = "performance-counters")]
        s.reset_performance_counters();
        assert_eq!(
            run(&mut s, i, "float_raw_bits", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(0x3f800000), Value::I64(0x3ff0000000000000)])
        );
        let args = [
            Value::I32(-2),
            Value::I64(0x01020304800180ff),
            Value::F32(0x80000000),
            Value::F64(0xfff8000000012345),
        ];
        assert_eq!(
            run(&mut s, i, "store_kinds", &args).unwrap(),
            RunOutcome::Returned(vec![])
        );
        let image = std::fs::read(format!(
            "{}/tests/fixtures/performance/p043/expected-memory/store-kinds/fuel-29.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(
            s.compact_memory_accesses(),
            if policy == ExecutionPolicy::Compact {
                11
            } else {
                0
            }
        );
        #[cfg(feature = "performance-counters")]
        if policy == ExecutionPolicy::Compact {
            let c = s.performance_counters();
            assert_eq!(c.compact_shared_memory_accesses, 11);
            assert_eq!(c.read_vec_operations, 2);
            assert_eq!(c.read_fixed_operations, 0);
        }
        assert_eq!(
            s.read_memory(memory(&s, i, "memory"), 0, 65536).unwrap(),
            image
        );
    }
    for pc in [0, 2, 4, 5] {
        let mut observed = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let e = engine(policy, Some(6));
            let (mut s, i) = load(&e, fixture);
            let f = func(&s, i, "write-read");
            s.request_materialization_roundtrip(f, pc).unwrap();
            assert_eq!(
                s.invoke(f, &[Value::I64(17), Value::I64(-91)]).unwrap(),
                RunOutcome::Returned(vec![Value::I64(-91)])
            );
            assert_eq!(s.materialization_roundtrips(), 1);
            observed.push(s.execution_snapshot().unwrap().clone());
            compact_counts(&s, policy, 6, 2);
        }
        assert_eq!(observed[0], observed[1], "pc={pc}");
    }
}
