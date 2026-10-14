//! Unexecuted root tests. Freshly encode and validate every fixture before use.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;

const CONSTANTS: &[u8] = include_bytes!("../fixtures/performance/p039/constant-bits.wasm");
const FLOATS: &[u8] = include_bytes!("../fixtures/performance/p039/float-rules.wasm");
const VECTOR: &[u8] = include_bytes!("../fixtures/performance/p039/shuffle.wasm");
const TRAPS: &[u8] = include_bytes!("../fixtures/performance/p039/traps.wasm");
const POLICIES: [ExecutionPolicy; 3] = [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact];

fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8], name: &str) -> (Store, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(function) = instance.export(&store, name).unwrap() else { panic!("missing function") };
    (store, function)
}
fn observed(outcome: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    outcome.map_err(|error| (error.kind, error.message))
}
fn returned(outcome: RunOutcome) -> Value {
    let RunOutcome::Returned(values) = outcome else { panic!("non-returned outcome") };
    assert_eq!(values.len(), 1);
    values[0]
}
fn counts(store: &Store, policy: ExecutionPolicy, events: u64) {
    assert_eq!(store.compact_dispatches(), if policy == ExecutionPolicy::Compact { events } else { 0 });
    if policy == ExecutionPolicy::Reference { assert_eq!(store.reference_dispatches(), events); }
}

#[test]
fn p039_raw_float_constants_preserve_signed_zero_and_nan_payloads() {
    for (name, expected) in [
        ("f32_negative_zero", Value::I32(i32::MIN)),
        ("f64_negative_zero", Value::I64(i64::MIN)),
        ("f32_payload", Value::I32(0x7fc1_2345)),
        ("f64_payload", Value::I64(0xfff8_0000_0000_1234u64 as i64)),
    ] {
        for policy in POLICIES {
            let (mut store, function) = loaded(policy, Config::default(), CONSTANTS, name);
            assert_eq!(returned(store.invoke(function, &[]).unwrap()), expected, "{name} {policy:?}");
            counts(&store, policy, 3);
        }
    }
}

#[test]
fn p039_strict_min_max_nan_sets_and_copysign_keep_wasm_bits() {
    for policy in POLICIES {
        for (name, exact, mask) in [
            ("minimum_zero", Value::I32(i32::MIN), None),
            ("maximum_zero", Value::I64(0), None),
            ("copy_nan_sign", Value::I32(0xffc1_2345u32 as i32), None),
            ("minimum_nan", Value::I32(0), Some(0x7fc0_0000u64)),
            ("maximum_signaling_nan", Value::I64(0), Some(0x7ff8_0000_0000_0000u64)),
        ] {
            let (mut store, function) = loaded(policy, Config::default(), FLOATS, name);
            let value = returned(store.invoke(function, &[]).unwrap());
            if let Some(mask) = mask {
                let bits = match value { Value::I32(bits) => u64::from(bits as u32), Value::I64(bits) => bits as u64, _ => panic!("wrong NaN result type") };
                // Arithmetic NaN class: exponent and quiet bit set; sign and
                // lower payload remain unconstrained by the Wasm specification.
                assert_eq!(bits & mask, mask, "{name} {policy:?}");
            } else { assert_eq!(value, exact, "{name} {policy:?}"); }
            counts(&store, policy, 5);
        }
    }
}

#[test]
fn p039_vector_shuffle_fuel_and_materialized_operands_match_source_events() {
    let expected = Value::V128(V128([116, 0, 102, 14, 107, 5, 101, 15, 8, 110, 3, 115, 104, 12, 112, 6]));
    for fuel in 0..=5 {
        let mut results = Vec::new();
        for policy in POLICIES {
            let mut config = Config::default(); config.limits.fuel = Some(fuel);
            let (mut store, function) = loaded(policy, config, VECTOR, "run");
            results.push(observed(store.invoke(function, &[])));
            counts(&store, policy, fuel.min(4));
        }
        assert_eq!(results[0], results[1]); assert_eq!(results[0], results[2]);
        if fuel < 4 { assert_eq!(results[0], Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))); }
        else { assert_eq!(results[0], Ok(RunOutcome::Returned(vec![expected]))); }
    }
    let mut snapshots = Vec::new();
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let mut config = Config::default(); config.limits.fuel = Some(4);
        let (mut store, function) = loaded(policy, config, VECTOR, "run");
        store.request_materialization_roundtrip(function, 2).unwrap();
        assert_eq!(returned(store.invoke(function, &[]).unwrap()), expected);
        let snapshot = store.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc, 2); assert_eq!(snapshot.cursor.prepared_pc, 2);
        assert_eq!(snapshot.remaining_fuel, Some(1)); assert!(snapshot.pending_boundary_charged);
        assert_eq!(snapshot.operand_count, 2); assert_eq!(store.materialization_roundtrips(), 1);
        snapshots.push(snapshot);
        counts(&store, policy, 4);
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

#[test]
fn p039_division_conversion_traps_and_fuel_priority_are_original_operations() {
    for (name, trap, events) in [
        ("division_overflow", Some("integer overflow"), 3),
        ("division_zero", Some("integer divide by zero"), 3),
        ("division_success", None, 4),
        ("conversion_nan", Some("invalid integer conversion"), 2),
        ("conversion_overflow", Some("integer overflow"), 2),
        ("conversion_success", None, 3),
    ] {
        for fuel in 0..=events + 1 {
            let mut results = Vec::new();
            for policy in POLICIES {
                let mut config = Config::default(); config.limits.fuel = Some(fuel);
                let (mut store, function) = loaded(policy, config, TRAPS, name);
                results.push(observed(store.invoke(function, &[])));
                counts(&store, policy, fuel.min(events));
            }
            assert_eq!(results[0], results[1], "reference {name} fuel {fuel}");
            assert_eq!(results[0], results[2], "compact {name} fuel {fuel}");
            let expected = if fuel < events { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                else if let Some(trap) = trap { Err((ErrorKind::Trap, trap.into())) }
                else { Ok(RunOutcome::Returned(vec![Value::I32(42)])) };
            assert_eq!(results[0], expected, "{name} fuel {fuel}");
        }
    }
    for (name, source_pc, fuel) in [("division_overflow", 2, 3), ("conversion_nan", 1, 2)] {
        let mut snapshots = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let mut config = Config::default(); config.limits.fuel = Some(fuel);
            let (mut store, function) = loaded(policy, config, TRAPS, name);
            store.request_materialization_roundtrip(function, source_pc).unwrap();
            assert!(store.invoke(function, &[]).is_err());
            let snapshot = store.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc, source_pc);
            assert_eq!(snapshot.remaining_fuel, Some(0)); assert!(snapshot.pending_boundary_charged);
            assert_eq!(store.materialization_roundtrips(), 1);
            snapshots.push(snapshot);
        }
        assert_eq!(snapshots[0], snapshots[1]);
    }
}

#[test]
fn p039_vector_block_parameters_and_results_keep_source_control_and_prefixes() {
    // Independent original binary: ()->v128 with an indexed (v128)->v128 block.
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    fn section(bytes: &mut Vec<u8>, id: u8, payload: &[u8]) {
        assert!(payload.len() < 128); bytes.extend([id, payload.len() as u8]); bytes.extend(payload);
    }
    section(&mut bytes, 1, &[2,0x60,0,1,0x7b,0x60,1,0x7b,1,0x7b]);
    section(&mut bytes, 3, &[1,0]);
    section(&mut bytes, 7, &[1,3,b'r',b'u',b'n',0,0]);
    let mut body = vec![0,0xfd,0x0c];
    body.extend(0u8..16); body.extend([2,1,0xfd,0x0c]);
    body.extend(101u8..117); body.extend([0xfd,0x0d]);
    body.extend([31,0,17,14,22,5,16,15,8,25,3,30,19,12,27,6]);
    body.extend([0x0b,0x0b]);
    let mut code = vec![1,body.len() as u8]; code.extend(body); section(&mut bytes,10,&code);
    let expected = Value::V128(V128([116,0,102,14,107,5,101,15,8,110,3,115,104,12,112,6]));
    for fuel in 0..=7 {
        for policy in POLICIES {
            let mut config = Config::default(); config.limits.fuel = Some(fuel);
            let (mut store,function) = loaded(policy,config,&bytes,"run");
            assert_eq!(observed(store.invoke(function,&[])),
                if fuel < 6 { Err((ErrorKind::ResourceLimit,"fuel exhausted".into())) }
                else { Ok(RunOutcome::Returned(vec![expected])) });
            counts(&store,policy,fuel.min(6));
        }
    }
    for pc in 0..6 {
        let mut snapshots = Vec::new();
        for policy in [ExecutionPolicy::Decoded,ExecutionPolicy::Compact] {
            let mut config = Config::default(); config.limits.fuel = Some(6);
            let (mut store,function) = loaded(policy,config,&bytes,"run");
            store.request_materialization_roundtrip(function,pc).unwrap();
            assert_eq!(store.invoke(function,&[]).unwrap(),RunOutcome::Returned(vec![expected]));
            let snapshot = store.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc,pc);
            assert_eq!(snapshot.remaining_fuel,Some(5-pc as u64));
            assert!(snapshot.pending_boundary_charged);
            counts(&store,policy,6); snapshots.push(snapshot);
        }
        assert_eq!(snapshots[0],snapshots[1],"vector block pc {pc}");
    }
}
