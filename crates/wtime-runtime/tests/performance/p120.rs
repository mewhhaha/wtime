#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const SWITCH: &[u8] = include_bytes!("../fixtures/performance/p120/switch.wasm");
const CONT: &[u8] = include_bytes!(
    "../fixtures/performance/bench_wasm/integration_p113/integration_cont_prefix.wasm"
);
const POLICIES: [ExecutionPolicy; 3] = [
    ExecutionPolicy::Decoded,
    ExecutionPolicy::Reference,
    ExecutionPolicy::Compact,
];
fn function(s: &Store, i: Instance, name: &str) -> Func {
    let External::Func(f) = i.export(s, name).unwrap() else {
        panic!()
    };
    f
}
fn call(s: &mut Store, i: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    s.invoke(function(s, i, name), args)
}
fn loaded(
    policy: ExecutionPolicy,
    bytes: &[u8],
    fuel: Option<u64>,
    depth: usize,
    budget: usize,
) -> (Store, Instance) {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.fuel = fuel;
    c.limits.max_call_depth = depth;
    c.limits.max_continuation_bytes = budget;
    let e = Engine::with_execution_policy(c, policy).unwrap();
    let m = e.compile(bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    drop(m);
    drop(e);
    (s, i)
}
fn cleanup(s: &mut Store) {
    let stats = s.collect().unwrap();
    assert_eq!(stats.live, 0);
    assert_eq!(s.continuation_bytes(), 0);
    assert_eq!(s.gc_payload_bytes(), 0);
}
#[test]
fn p120_switch_preserves_outgoing_local_and_every_source_fuel_boundary() {
    for fuel in 0..=19 {
        for policy in POLICIES {
            let (mut s, i) = loaded(policy, SWITCH, Some(fuel), 3, usize::MAX);
            let result = call(&mut s, i, "run", &[]);
            if fuel < 18 {
                assert_eq!(
                    result.unwrap_err().message,
                    "fuel exhausted",
                    "{policy:?}/{fuel}"
                );
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(9)]));
            }
            if policy == ExecutionPolicy::Compact {
                let steps = fuel.saturating_sub(6).min(3) + u64::from(fuel >= 11);
                let exits = u64::from(fuel >= 10) + u64::from(fuel >= 12);
                assert_eq!(s.compact_dispatches(), steps);
                assert_eq!(
                    s.compact_prefix_counts(),
                    (
                        steps,
                        [0, 0, exits, 0],
                        if fuel >= 12 {
                            Some((1, 0xe3))
                        } else if fuel >= 10 {
                            Some((3, 0xe6))
                        } else {
                            None
                        }
                    )
                );
            }
            cleanup(&mut s);
        }
    }
}
#[test]
fn p120_switch_byte_and_frame_refusal_follow_prefix_and_preserve_cleanup() {
    for policy in POLICIES {
        let (mut probe, i) = loaded(policy, SWITCH, None, 3, usize::MAX);
        assert_eq!(
            call(&mut probe, i, "run", &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(9)])
        );
        let limit = probe.continuation_bytes();
        assert!(limit > 0);
        cleanup(&mut probe);
        for (fuel, depth, budget, message, steps, exits, last) in [
            (
                None,
                2,
                usize::MAX,
                "continuation frame budget",
                4,
                2,
                Some((1, 0xe3)),
            ),
            (
                None,
                3,
                limit,
                "continuation byte budget",
                3,
                1,
                Some((3, 0xe6)),
            ),
            (Some(9), 3, limit, "fuel exhausted", 3, 0, None),
        ] {
            let (mut s, i) = loaded(policy, SWITCH, fuel, depth, budget);
            let error = call(&mut s, i, "run", &[]).unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert_eq!(error.message, message);
            if policy == ExecutionPolicy::Compact {
                assert_eq!(s.compact_prefix_counts(), (steps, [0, 0, exits, 0], last));
            }
            cleanup(&mut s);
        }
    }
}
#[test]
fn p120_stale_foreign_and_cancelled_continuations_do_no_prefix_work() {
    for policy in POLICIES {
        let (mut s, i) = loaded(policy, CONT, None, 16, usize::MAX);
        let (mut foreign, j) = loaded(policy, CONT, None, 16, usize::MAX);
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = call(&mut s, i, "run", &[Value::I32(32)]).unwrap()
        else {
            panic!()
        };
        assert_eq!(payload, vec![Value::I32(64)]);
        let root = s.root(continuation).unwrap();
        let value = Value::Ref(RefValue::Gc(root.reference()));
        s.release_published(&[value]);
        let before = s.compact_prefix_counts();
        assert_eq!(
            call(&mut foreign, j, "resume", &[value, Value::I32(7)])
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
        assert_eq!(foreign.compact_prefix_counts(), (0, [0; 4], None));
        s.cancellation_token().cancel();
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(7)])
                .unwrap_err()
                .message,
            "execution cancelled"
        );
        assert_eq!(s.compact_prefix_counts(), before);
        s.cancellation_token().reset();
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(7)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(7)])
        );
        let External::Memory(mem) = i.export(&s, "memory").unwrap() else {
            panic!()
        };
        let image = s.read_memory(mem, 0, 65536).unwrap();
        let mut expected = vec![0; 65536];
        expected[..4].copy_from_slice(&7i32.to_le_bytes());
        assert_eq!(image, expected);
        let External::Memory(mem) = j.export(&foreign, "memory").unwrap() else {
            panic!()
        };
        assert!(
            foreign
                .read_memory(mem, 0, 65536)
                .unwrap()
                .iter()
                .all(|&b| b == 0)
        );
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(7)])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
        drop(root);
        cleanup(&mut s);
        let before = s.compact_prefix_counts();
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(7)])
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
        assert_eq!(s.compact_prefix_counts(), before);
    }
}
