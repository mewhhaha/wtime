#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const BYTES: &[u8] = include_bytes!("../fixtures/performance/p115/handoff.wasm");
fn load(policy: ExecutionPolicy, fuel: Option<u64>, name: &str) -> (Store, Func, Memory) {
    let mut config = Config { features: FeatureSet::all_phase3_core(), ..Config::default() };
    config.limits.fuel = fuel;
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(BYTES).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    let External::Func(func) = instance.export(&store, name).unwrap() else { panic!() };
    let External::Memory(memory) = instance.export(&store, "memory").unwrap() else { panic!() };
    (store, func, memory)
}
fn view(result: Result<RunOutcome>) -> std::result::Result<RunOutcome, (ErrorKind, String)> {
    result.map_err(|e| (e.kind, e.message))
}
const POLICIES: [ExecutionPolicy; 3] = [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact];

#[test]
fn p115_every_fuel_prefix_preserves_memory_and_single_charge() {
    for fuel in 0..=15 {
        let mut observed = vec![];
        for policy in POLICIES {
            let (mut s, f, m) = load(policy, Some(fuel), "store_after_prefix");
            let result = view(s.invoke(f, &[Value::I32(8)]));
            let expected = if fuel < 14 { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                else { Ok(RunOutcome::Returned(vec![Value::I32(17)])) };
            assert_eq!(result, expected, "{policy:?}/fuel={fuel}");
            let image = s.read_memory(m, 0, 65536).unwrap();
            let mut expected_memory = vec![0; 65536];
            if fuel >= 7 { expected_memory[..4].copy_from_slice(&11i32.to_le_bytes()); }
            assert_eq!(image, expected_memory);
            let (steps, exits, last) = s.compact_prefix_counts();
            if policy == ExecutionPolicy::Compact {
                assert_eq!(steps, fuel.min(6));
                assert_eq!(s.compact_dispatches(), steps);
                assert_eq!(exits, [0, 0, 0, u64::from(fuel >= 7)]);
                assert_eq!(last, if fuel >= 7 { Some((6, 0x36)) } else { None });
            } else { assert_eq!((steps, exits, last), (0, [0; 4], None)); }
            observed.push((result, image));
        }
        assert_eq!(observed[0], observed[1]); assert_eq!(observed[0], observed[2]);
    }
}

#[test]
fn p115_branch_over_gc_uses_one_way_decoded_suffix() {
    for (input, events) in [(0, 10), (3, 14)] {
        for fuel in 0..=15 {
            let mut observed = vec![];
            for policy in POLICIES {
                let (mut s, f, m) = load(policy, Some(fuel), "branch_over_complex");
                let result = view(s.invoke(f, &[Value::I32(input)]));
                assert_eq!(result, if fuel < events { Err((ErrorKind::ResourceLimit, "fuel exhausted".into())) }
                    else { Ok(RunOutcome::Returned(vec![Value::I32(input * 2 + 5)])) });
                let collection = s.collect().unwrap();
                assert_eq!(collection.live, 0);
                assert_eq!(collection.reclaimed, usize::from(input != 0 && fuel >= 9));
                let image = s.read_memory(m, 0, 65536).unwrap(); assert!(image.iter().all(|&b| b == 0));
                if policy == ExecutionPolicy::Compact {
                    assert_eq!(s.compact_dispatches(), fuel.min(3));
                    let (steps, exits, last) = s.compact_prefix_counts();
                    assert_eq!(steps, fuel.min(3)); assert_eq!(exits, [0,0,0,u64::from(fuel >= 4)]);
                    assert_eq!(last, if fuel >= 4 { Some((3, 2)) } else { None });
                }
                observed.push((result, image, collection.reclaimed));
            }
            assert_eq!(observed[0], observed[1]); assert_eq!(observed[0], observed[2]);
        }
    }
}

#[test]
fn p115_cancellation_priority_and_pending_exit_snapshot_are_exact() {
    let mut snapshots = vec![];
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let (mut s, f, m) = load(policy, Some(7), "store_after_prefix");
        s.request_materialization_roundtrip(f, 6).unwrap();
        assert_eq!(view(s.invoke(f, &[Value::I32(8)])), Err((ErrorKind::ResourceLimit, "fuel exhausted".into())));
        let snapshot = s.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc, 6); assert_eq!(snapshot.remaining_fuel, Some(0));
        assert!(snapshot.pending_boundary_charged); assert_eq!(s.materialization_roundtrips(), 1);
        assert_eq!(&s.read_memory(m, 0, 4).unwrap(), &11i32.to_le_bytes()); snapshots.push(snapshot);
        let (mut s, f, m) = load(policy, Some(0), "store_after_prefix");
        s.cancellation_token().cancel();
        assert_eq!(view(s.invoke(f, &[Value::I32(8)])), Err((ErrorKind::ResourceLimit, "execution cancelled".into())));
        assert!(s.read_memory(m, 0, 65536).unwrap().iter().all(|&b| b == 0));
        assert_eq!(s.compact_prefix_counts(), (0,[0;4],None));
    }
    assert_eq!(snapshots[0], snapshots[1]);
}
