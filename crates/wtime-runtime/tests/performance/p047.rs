#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const ADD: &[u8] = include_bytes!("../fixtures/performance/p047/p047_local_add.wasm");
const SET: &[u8] = include_bytes!("../fixtures/performance/p047/p047_const_local_set.wasm");
const EFFECTS: &[u8] = include_bytes!("../fixtures/performance/p047/p047_effects_prefix.wasm");
fn loaded(policy: ExecutionPolicy, config: Config, bytes: &[u8]) -> (Store, Instance, Func) {
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(function) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    (store, instance, function)
}
fn view(outcome: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    outcome.map_err(|e| (e.kind, e.message))
}
fn effects(store: &Store, instance: Instance) -> (Vec<u8>, Value) {
    let External::Memory(memory) = instance.export(store, "memory").unwrap() else {
        panic!()
    };
    let External::Global(global) = instance.export(store, "state").unwrap() else {
        panic!()
    };
    (
        store.read_memory(memory, 0, 65536).unwrap(),
        store.get_global(global).unwrap(),
    )
}

#[test]
fn p047_fused_results_and_overflow() {
    let mut inputs = vec![i32::MIN, i32::MAX, -1, 0, 1, 7, 0x40000000];
    let mut seed = 0xf1175a01u32;
    for _ in 0..256 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        inputs.push(seed as i32);
    }
    for policy in [
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Reference,
        ExecutionPolicy::Compact,
    ] {
        for (bytes, events, two_args) in [(ADD, 4, true), (SET, 6, false)] {
            let (mut store, _, function) = loaded(policy, Config::default(), bytes);
            for &left in &inputs {
                let right = left.rotate_left(11).wrapping_add(0x1234567);
                let args = if two_args {
                    vec![Value::I32(left), Value::I32(right)]
                } else {
                    vec![Value::I32(left)]
                };
                let expected = left.wrapping_add(if two_args { right } else { i32::MAX });
                let before = store.compact_dispatches();
                let groups = store.compact_fused_groups();
                assert_eq!(
                    store.invoke(function, &args).unwrap(),
                    RunOutcome::Returned(vec![Value::I32(expected)])
                );
                assert_eq!(
                    store.compact_dispatches() - before,
                    if policy == ExecutionPolicy::Compact {
                        events
                    } else {
                        0
                    }
                );
                assert_eq!(
                    store.compact_fused_groups() - groups,
                    u64::from(policy == ExecutionPolicy::Compact)
                );
            }
        }
    }
}

#[test]
fn p047_selected_accumulator_and_dependent_shapes_execute_verified_groups() {
    for (bytes, accumulator) in [
        (
            include_bytes!("../fixtures/performance/p047/accumulator.wasm").as_slice(),
            true,
        ),
        (
            include_bytes!("../fixtures/performance/p047/dependent.wasm").as_slice(),
            false,
        ),
    ] {
        for policy in [
            ExecutionPolicy::Decoded,
            ExecutionPolicy::Reference,
            ExecutionPolicy::Compact,
        ] {
            let (mut store, _, run) = loaded(policy, Config::default(), bytes);
            for n in [0i32, 1, 17, 32, 10000] {
                let expected = if accumulator {
                    n.wrapping_mul(n - 1) / 2
                } else {
                    let mut value = 1i32;
                    for _ in 0..n {
                        value = value.wrapping_mul(1664525).wrapping_add(1013904223);
                    }
                    value
                };
                let before = store.compact_fused_groups();
                assert_eq!(
                    store.invoke(run, &[Value::I32(n)]).unwrap(),
                    RunOutcome::Returned(vec![Value::I32(expected)])
                );
                assert_eq!(
                    store.compact_fused_groups() - before,
                    if policy == ExecutionPolicy::Compact {
                        n as u64 * if accumulator { 2 } else { 1 }
                    } else {
                        0
                    }
                );
            }
        }
    }
}

#[test]
fn p047_every_source_fuel_prefix() {
    for (bytes, events, fused_len, args, expected) in [
        (
            ADD,
            4u64,
            3u64,
            vec![Value::I32(i32::MAX), Value::I32(1)],
            i32::MIN,
        ),
        (SET, 6u64, 4u64, vec![Value::I32(i32::MAX)], -2),
    ] {
        for fuel in 0..=events + 1 {
            let mut observations = vec![];
            for policy in [
                ExecutionPolicy::Decoded,
                ExecutionPolicy::Reference,
                ExecutionPolicy::Compact,
            ] {
                let mut config = Config::default();
                config.limits.fuel = Some(fuel);
                let (mut store, _, function) = loaded(policy, config, bytes);
                let result = view(store.invoke(function, &args));
                let oracle = if fuel < events {
                    Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                } else {
                    Ok(RunOutcome::Returned(vec![Value::I32(expected)]))
                };
                assert_eq!(result, oracle);
                assert_eq!(
                    store.compact_dispatches(),
                    if policy == ExecutionPolicy::Compact {
                        fuel.min(events)
                    } else {
                        0
                    }
                );
                assert_eq!(
                    store.compact_fused_groups(),
                    u64::from(policy == ExecutionPolicy::Compact && fuel >= fused_len)
                );
                observations.push(result);
            }
            assert_eq!(observations[0], observations[1]);
            assert_eq!(observations[0], observations[2]);
        }
        for pc in 0..events as usize {
            let mut snapshots = vec![];
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
                let mut config = Config::default();
                config.limits.fuel = Some(events);
                let (mut store, _, function) = loaded(policy, config, bytes);
                store
                    .request_materialization_roundtrip(function, pc)
                    .unwrap();
                assert_eq!(
                    store.invoke(function, &args).unwrap(),
                    RunOutcome::Returned(vec![Value::I32(expected)])
                );
                let snapshot = store.execution_snapshot().unwrap().clone();
                assert_eq!(snapshot.cursor.source_pc, pc);
                assert_eq!(snapshot.remaining_fuel, Some(events - 1 - pc as u64));
                assert!(snapshot.pending_boundary_charged && !snapshot.truncated);
                assert_eq!(store.materialization_roundtrips(), 1);
                if pc > 0 && pc < fused_len as usize {
                    assert_eq!(store.compact_fused_groups(), 0);
                }
                snapshots.push(snapshot);
            }
            assert_eq!(snapshots[0], snapshots[1], "fused source {pc}");
        }
    }
    // This separately protected program hits the global-op barrier; it must
    // preserve every committed memory/global effect on the decoded fallback.
    for fuel in 0..=15 {
        let mut observations = vec![];
        for policy in [
            ExecutionPolicy::Decoded,
            ExecutionPolicy::Reference,
            ExecutionPolicy::Compact,
        ] {
            let mut config = Config::default();
            config.limits.fuel = Some(fuel);
            let (mut store, instance, function) = loaded(policy, config, EFFECTS);
            let result = view(store.invoke(function, &[Value::I32(i32::MAX)]));
            let (memory, global) = effects(&store, instance);
            let mut expected = vec![0; 65536];
            if fuel >= 3 {
                expected[..4].copy_from_slice(&42i32.to_le_bytes());
            }
            if fuel >= 12 {
                expected[4..8].copy_from_slice(&(-2147483642i32).to_le_bytes());
            }
            assert_eq!(memory, expected, "fuel {fuel}");
            assert_eq!(
                global,
                Value::I32(if fuel >= 9 { -2147483642 } else { -19 })
            );
            if fuel < 14 {
                assert_eq!(
                    result,
                    Err((ErrorKind::ResourceLimit, "fuel exhausted".into()))
                );
            } else {
                assert_eq!(
                    result,
                    Ok(RunOutcome::Returned(vec![Value::I32(-2147483642)]))
                );
            }
            assert_eq!(
                store.compact_dispatches(),
                if policy == ExecutionPolicy::Compact {
                    fuel.min(2)
                } else {
                    0
                }
            );
            assert_eq!(store.compact_fused_groups(), 0);
            observations.push((result, memory, global));
        }
        assert_eq!(observations[0], observations[1]);
        assert_eq!(observations[0], observations[2]);
    }
    // Independent canonical-state oracle at each original source boundary,
    // including each interior point that forces a source-prefix fallback.
    for pc in 0..14 {
        let mut snapshots = vec![];
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
            let mut config = Config::default();
            config.limits.fuel = Some(14);
            let (mut store, _, function) = loaded(policy, config, EFFECTS);
            store
                .request_materialization_roundtrip(function, pc)
                .unwrap();
            assert_eq!(
                store.invoke(function, &[Value::I32(i32::MAX)]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(-2147483642)])
            );
            let snapshot = store.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc, pc);
            assert_eq!(snapshot.remaining_fuel, Some(13 - pc as u64));
            assert!(snapshot.pending_boundary_charged && snapshot.truncated);
            assert_eq!(snapshot.memory_lengths, [65536]);
            assert_eq!(snapshot.memory_prefixes[0].len(), 64);
            assert_eq!(store.materialization_roundtrips(), 1);
            if (4..7).contains(&pc) {
                assert_eq!(store.compact_fused_groups(), 0);
            }
            snapshots.push(snapshot);
        }
        assert_eq!(snapshots[0], snapshots[1], "source {pc}");
    }
}

#[test]
fn p047_cancel_and_materialize() {
    for policy in [
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Reference,
        ExecutionPolicy::Compact,
    ] {
        let (mut store, instance, function) = loaded(policy, Config::default(), EFFECTS);
        store.cancellation_token().cancel();
        assert_eq!(
            view(store.invoke(function, &[Value::I32(37)])),
            Err((ErrorKind::ResourceLimit, "execution cancelled".into()))
        );
        assert_eq!(effects(&store, instance), (vec![0; 65536], Value::I32(-19)));
        assert_eq!(store.compact_fused_groups(), 0);
    }
    // Cancellation from a host callback must stop before the later store.
    // The separate private test injects cancellation at all internal fused
    // boundaries and inspects the fully reconstructed pending operands/PC.
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let engine = Engine::with_execution_policy(Config::default(), policy).unwrap();
        let module = engine
            .compile(include_bytes!(
                "../fixtures/performance/accounting/host_then_store.wasm"
            ))
            .unwrap();
        let mut store = Store::new(&engine);
        let cancellation = store.cancellation_token();
        let function = store
            .create_host_func(FuncType::new(vec![], vec![]), move |_| {
                cancellation.cancel();
                Ok(vec![])
            })
            .unwrap();
        let mut linker = Linker::new(&engine);
        linker
            .define("host", "stop", External::Func(function))
            .unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let External::Func(run) = instance.export(&store, "run").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.invoke(run, &[]).unwrap_err().message,
            "execution cancelled"
        );
        let External::Memory(memory) = instance.export(&store, "memory").unwrap() else {
            panic!()
        };
        assert_eq!(store.read_memory(memory, 0, 4).unwrap(), [0, 0, 0, 0]);
    }
}

#[test]
fn p047_branch_entry_and_peak_stack() {
    for policy in [
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Reference,
        ExecutionPolicy::Compact,
    ] {
        let (mut store, _, run) = loaded(
            policy,
            Config::default(),
            include_bytes!("../fixtures/performance/p047/p047_branch_loop.wasm"),
        );
        assert_eq!(
            store.invoke(run, &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(32)])
        );
        assert_eq!(
            store.compact_fused_groups(),
            if policy == ExecutionPolicy::Compact {
                8
            } else {
                0
            }
        );
        let (mut store, _, run) = loaded(
            policy,
            Config::default(),
            include_bytes!("../fixtures/performance/p047/p047_mixed_externref_local.wasm"),
        );
        assert_eq!(
            store.invoke(run, &[Value::I32(1)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(i32::MIN)])
        );
        assert_eq!(store.compact_fused_groups(), 0);
        for (limit, fuel, expected) in [(2, 5, "fuel exhausted"), (2, 6, "operand stack limit")] {
            let mut config = Config::default();
            config.limits.max_stack_height = limit;
            config.limits.fuel = Some(fuel);
            let (mut store, _, run) = loaded(
                policy,
                config,
                include_bytes!("../fixtures/performance/p047/p047_retained_value_call_peak.wasm"),
            );
            assert_eq!(
                store
                    .invoke(run, &[Value::I32(5), Value::I32(7)])
                    .unwrap_err()
                    .message,
                expected
            );
            assert_eq!(store.compact_fused_groups(), 0);
        }
        let mut config = Config::default();
        config.limits.max_stack_height = 3;
        config.limits.fuel = Some(9);
        let (mut store, _, run) = loaded(
            policy,
            config,
            include_bytes!("../fixtures/performance/p047/p047_retained_value_call_peak.wasm"),
        );
        assert_eq!(
            store.invoke(run, &[Value::I32(5), Value::I32(7)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(19)])
        );
        assert_eq!(
            store.compact_fused_groups(),
            u64::from(policy == ExecutionPolicy::Compact)
        );
        let (mut store, _, run) = loaded(
            policy,
            Config::default(),
            include_bytes!("../fixtures/performance/compact/long_metadata.wasm"),
        );
        let _ = store.invoke(run, &[]);
        assert_eq!(store.compact_dispatches(), 0);
        assert_eq!(store.compact_fused_groups(), 0);
    }
}
