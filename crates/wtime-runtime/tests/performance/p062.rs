#![cfg(all(feature = "register-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const OPS: &[u8] = include_bytes!("../fixtures/performance/p062/float_ops.wasm");
const MIXED: &[u8] = include_bytes!("../fixtures/performance/p062/mixed_slots.wasm");
const CONSTANTS: &[u8] = include_bytes!("../fixtures/performance/p039/constant-bits.wasm");
const RULES: &[u8] = include_bytes!("../fixtures/performance/p039/float-rules.wasm");
const TRAPS: &[u8] = include_bytes!("../fixtures/performance/p039/traps.wasm");

fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8]) -> (Store, Instance) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, instance)
}
fn exported_func(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else {
        panic!()
    };
    function
}
fn observed(result: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    result.map_err(|error| (error.kind, error.message))
}
fn returned(result: RunOutcome) -> Value {
    let RunOutcome::Returned(values) = result else {
        panic!()
    };
    assert_eq!(values.len(), 1);
    values[0]
}
fn snapshot(store: &Store) -> DiagnosticSnapshot {
    let mut result = store.execution_snapshot().unwrap().clone();
    for frame in &mut result.frames {
        frame.module_fingerprint = 0;
    }
    result
}

#[test]
fn p062_raw_bits_and_strict_float_rules() {
    for (bytes, name, expected, events) in [
        (CONSTANTS, "f32_negative_zero", Value::I32(i32::MIN), 3),
        (CONSTANTS, "f64_negative_zero", Value::I64(i64::MIN), 3),
        (CONSTANTS, "f32_payload", Value::I32(0x7fc1_2345), 3),
        (
            CONSTANTS,
            "f64_payload",
            Value::I64(0xfff8_0000_0000_1234u64 as i64),
            3,
        ),
        (RULES, "minimum_zero", Value::I32(i32::MIN), 5),
        (RULES, "maximum_zero", Value::I64(0), 5),
        (RULES, "copy_nan_sign", Value::I32(0xffc1_2345u32 as i32), 5),
    ] {
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let (mut store, instance) = loaded(policy, Config::default(), bytes);
            let function = exported_func(&store, instance, name);
            assert_eq!(returned(store.invoke(function, &[]).unwrap()), expected);
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), events);
                assert_eq!(store.register_fallback_frames(), 0);
            }
        }
    }
    for (name, mask) in [
        ("minimum_nan", 0x7fc0_0000),
        ("maximum_signaling_nan", 0x7ff8_0000_0000_0000),
    ] {
        let (mut store, instance) = loaded(ExecutionPolicy::Register, Config::default(), RULES);
        let function = exported_func(&store, instance, name);
        let value = returned(store.invoke(function, &[]).unwrap());
        let bits = match value {
            Value::I32(n) => n as u32 as u64,
            Value::I64(n) => n as u64,
            _ => panic!(),
        };
        assert_eq!(bits & mask, mask);
        assert_eq!(store.register_dispatches(), 5);
        assert_eq!(store.register_fallback_frames(), 0);
    }
}

#[test]
fn p062_trapping_and_saturating_conversion_boundaries() {
    for (name, message, events) in [
        ("conversion_nan", Some("invalid integer conversion"), 2),
        ("conversion_overflow", Some("integer overflow"), 2),
        ("conversion_success", None, 3),
    ] {
        for fuel in 0..=events + 1 {
            let mut outcomes = vec![];
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut config = Config::default();
                config.limits.fuel = Some(fuel);
                let (mut store, instance) = loaded(policy, config, TRAPS);
                let function = exported_func(&store, instance, name);
                let result = observed(store.invoke(function, &[]));
                let expected = if fuel < events {
                    Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                } else if let Some(message) = message {
                    Err((ErrorKind::Trap, message.into()))
                } else {
                    Ok(RunOutcome::Returned(vec![Value::I32(42)]))
                };
                assert_eq!(result, expected);
                if policy == ExecutionPolicy::Register {
                    assert_eq!(store.register_dispatches(), fuel.min(events));
                    assert_eq!(store.register_fallback_frames(), 0);
                }
                outcomes.push(result);
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
        let mut snapshots = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut config = Config::default();
            config.limits.fuel = Some(2);
            let (mut store, instance) = loaded(policy, config, TRAPS);
            let function = exported_func(&store, instance, name);
            store
                .request_materialization_roundtrip(function, 1)
                .unwrap();
            let result = observed(store.invoke(function, &[]));
            snapshots.push((result, snapshot(&store)));
        }
        assert_eq!(snapshots[0], snapshots[1]);
    }
    for (name, input, expected) in [
        (
            "i32_trunc_sat_f32_s",
            Value::F32(f32::NAN.to_bits()),
            Value::I32(0),
        ),
        (
            "i32_trunc_sat_f64_s",
            Value::F64(f64::INFINITY.to_bits()),
            Value::I32(i32::MAX),
        ),
        (
            "i32_trunc_sat_f64_u",
            Value::F64(f64::NEG_INFINITY.to_bits()),
            Value::I32(0),
        ),
        (
            "i64_trunc_sat_f64_s",
            Value::F64(f64::NEG_INFINITY.to_bits()),
            Value::I64(i64::MIN),
        ),
        (
            "i64_trunc_sat_f64_u",
            Value::F64(f64::INFINITY.to_bits()),
            Value::I64(-1),
        ),
        (
            "i32_trunc_f64_s",
            Value::F64((-2147483648.75f64).to_bits()),
            Value::I32(i32::MIN),
        ),
        (
            "i32_trunc_f64_u",
            Value::F64((-0.75f64).to_bits()),
            Value::I32(0),
        ),
    ] {
        let (mut store, instance) = loaded(ExecutionPolicy::Register, Config::default(), OPS);
        let function = exported_func(&store, instance, name);
        assert_eq!(
            returned(store.invoke(function, &[input]).unwrap()),
            expected
        );
        assert_eq!(store.register_dispatches(), 3);
        assert_eq!(store.register_fallback_frames(), 0);
    }
}

fn prefixes(bytes: &[u8], args: &[Value], path: &[usize], expected: &[Value]) -> usize {
    let mut visits = vec![0; 128];
    for (index, pc) in path.iter().copied().enumerate() {
        let mut outcomes = vec![];
        for policy in [
            ExecutionPolicy::Decoded,
            ExecutionPolicy::Register,
            #[cfg(feature = "compact-executor")]
            ExecutionPolicy::Compact,
        ] {
            let mut config = Config::default();
            config.limits.fuel = Some(index as u64 + 1);
            let (mut store, instance) = loaded(policy, config, bytes);
            let function = exported_func(&store, instance, "run");
            store
                .request_materialization_roundtrip_after_visits(function, pc, visits[pc])
                .unwrap();
            let result = observed(store.invoke(function, args));
            let view = snapshot(&store);
            assert_eq!(view.cursor.source_pc, pc);
            assert!(view.pending_boundary_charged && !view.truncated);
            assert_eq!(view.remaining_fuel, Some(0));
            if policy == ExecutionPolicy::Register {
                assert_eq!(store.register_dispatches(), index as u64 + 1);
                assert_eq!(store.register_fallback_frames(), 0);
            }
            outcomes.push((result, view));
        }
        for result in &outcomes[1..] {
            assert_eq!(&outcomes[0], result, "visit {index}, PC {pc}");
        }
        visits[pc] += 1;
    }
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let mut config = Config::default();
        config.limits.fuel = Some(path.len() as u64);
        let (mut store, instance) = loaded(policy, config, bytes);
        let function = exported_func(&store, instance, "run");
        assert_eq!(
            store.invoke(function, args).unwrap(),
            RunOutcome::Returned(expected.to_vec())
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), path.len() as u64);
        }
    }
    path.len()
}

#[test]
fn p062_mixed_numeric_prefixes_and_vector_fallback() {
    let args = [
        Value::I32(-21),
        Value::I64(i64::MIN),
        Value::F32(0xff81_2345),
        Value::F64(0x8000_0000_0000_0000),
    ];
    let mut count = prefixes(MIXED, &args, &(0..14).collect::<Vec<_>>(), &args);
    let mut float_path: Vec<_> = (0..10).collect();
    for _ in 0..2 {
        float_path.extend(10..43);
    }
    float_path.extend(10..14);
    float_path.extend(45..57);
    count += prefixes(
        include_bytes!("../fixtures/performance/p062/float_loop.wasm"),
        &[Value::I32(2)],
        &float_path,
        &[Value::I32(1)],
    );
    let mut double_path: Vec<_> = (0..6).collect();
    for _ in 0..2 {
        double_path.extend(6..27);
    }
    double_path.extend(6..10);
    double_path.extend(29..35);
    count += prefixes(
        include_bytes!("../fixtures/performance/p062/double_loop.wasm"),
        &[Value::I32(2)],
        &double_path,
        &[Value::I32(-1)],
    );
    assert_eq!(count, 164);
    let (mut store, instance) = loaded(ExecutionPolicy::Register, Config::default(), MIXED);
    let function = exported_func(&store, instance, "run");
    assert_eq!(
        store.invoke(function, &args).unwrap(),
        RunOutcome::Returned(args.to_vec())
    );
    assert_eq!(store.register_dispatches(), 14);
    store.cancellation_token().cancel();
    assert_eq!(
        observed(store.invoke(function, &args)),
        Err((ErrorKind::ResourceLimit, "execution cancelled".into()))
    );
    assert_eq!(store.register_dispatches(), 14);
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
        let (mut store, instance) = loaded(
            policy,
            Config::default(),
            include_bytes!("../fixtures/performance/p039/shuffle.wasm"),
        );
        let function = exported_func(&store, instance, "run");
        let result = returned(store.invoke(function, &[]).unwrap());
        assert_eq!(
            result,
            Value::V128(V128([
                116, 0, 102, 14, 107, 5, 101, 15, 8, 110, 3, 115, 104, 12, 112, 6
            ]))
        );
        if policy == ExecutionPolicy::Register {
            assert_eq!(store.register_dispatches(), 0);
            assert_eq!(store.register_fallback_frames(), 1);
        }
    }
    eprintln!(
        "P062: {count} mixed numeric source prefixes; raw typed locals/results; f32/f64 repeated kernel visits; vector whole-function fallback"
    );
}

fn boundary_values(ty: &str) -> Vec<Value> {
    match ty {
        "f32" => [
            0.,
            -0.,
            1.,
            -1.,
            1.5,
            -1.5,
            2.5,
            -2.5,
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            2147483648.,
            -2147483648.,
            4294967296.,
            9223372036854775808.,
            -9223372036854775808.,
            18446744073709551616.,
        ]
        .into_iter()
        .map(|n| Value::F32(n.to_bits()))
        .chain([1, 0x8000_0001, 0x7f81_2345, 0xffc1_2345].map(Value::F32))
        .collect(),
        "f64" => [
            0.,
            -0.,
            1.,
            -1.,
            1.5,
            -1.5,
            2.5,
            -2.5,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            2147483648.,
            -2147483648.,
            4294967296.,
            9223372036854775808.,
            -9223372036854775808.,
            18446744073709551616.,
            -2147483648.75,
            -0.75,
            2147483647.75,
            4294967295.75,
        ]
        .into_iter()
        .map(|n| Value::F64(n.to_bits()))
        .chain(
            [
                1,
                0x8000_0000_0000_0001,
                0x7ff0_0000_0000_1234,
                0xfff8_0000_0000_1234,
            ]
            .map(Value::F64),
        )
        .collect(),
        "i32" => [0, 1, -1, i32::MIN, i32::MAX, 16777217, -16777217]
            .into_iter()
            .map(Value::I32)
            .collect(),
        "i64" => [
            0,
            1,
            -1,
            i64::MIN,
            i64::MAX,
            9007199254740993,
            -9007199254740993,
        ]
        .into_iter()
        .map(Value::I64)
        .collect(),
        _ => panic!(),
    }
}

#[test]
fn p062_opcode_boundaries_match_decoded() {
    let mut functions = vec![];
    for ty in ["f32", "f64"] {
        for name in [
            "eq", "ne", "lt", "gt", "le", "ge", "add", "sub", "mul", "div", "min", "max",
            "copysign",
        ] {
            functions.push((format!("{ty}_{name}"), ty, true));
        }
        for name in ["abs", "neg", "ceil", "floor", "trunc", "nearest", "sqrt"] {
            functions.push((format!("{ty}_{name}"), ty, false));
        }
    }
    for output in ["i32", "i64"] {
        for input in ["f32", "f64"] {
            for sign in ["s", "u"] {
                functions.push((format!("{output}_trunc_{input}_{sign}"), input, false));
                functions.push((format!("{output}_trunc_sat_{input}_{sign}"), input, false));
            }
        }
    }
    for output in ["f32", "f64"] {
        for input in ["i32", "i64"] {
            for sign in ["s", "u"] {
                functions.push((format!("{output}_convert_{input}_{sign}"), input, false));
            }
        }
    }
    functions.extend([
        ("f32_demote_f64".into(), "f64", false),
        ("f64_promote_f32".into(), "f32", false),
        ("i32_reinterpret_f32".into(), "f32", false),
        ("i64_reinterpret_f64".into(), "f64", false),
        ("f32_reinterpret_i32".into(), "i32", false),
        ("f64_reinterpret_i64".into(), "i64", false),
    ]);
    assert_eq!(functions.len(), 70);
    let (mut decoded, decoded_instance) = loaded(ExecutionPolicy::Decoded, Config::default(), OPS);
    let (mut register, register_instance) =
        loaded(ExecutionPolicy::Register, Config::default(), OPS);
    let mut cases = 0;
    for (name, ty, binary) in functions {
        let first = exported_func(&decoded, decoded_instance, &name);
        let second = exported_func(&register, register_instance, &name);
        let values = boundary_values(ty);
        for a in &values {
            for b in if binary {
                values.as_slice()
            } else {
                &values[..1]
            } {
                let args = if binary { vec![*a, *b] } else { vec![*a] };
                let before = register.register_dispatches();
                let expected = observed(decoded.invoke(first, &args));
                let actual = observed(register.invoke(second, &args));
                assert_eq!(actual, expected, "{name}, {args:?}");
                let events = if actual.is_err() {
                    2
                } else if binary {
                    4
                } else {
                    3
                };
                assert_eq!(register.register_dispatches() - before, events, "{name}");
                cases += 1;
            }
        }
    }
    assert_eq!(register.register_fallback_frames(), 0);
    assert!(cases > 15000);
    eprintln!(
        "P062: {cases} scalar float boundary cases across 70 opcodes; {} register source events",
        register.register_dispatches()
    );
}
