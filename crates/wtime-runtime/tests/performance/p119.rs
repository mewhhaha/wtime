#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const BYTES: &[u8] = include_bytes!("../fixtures/performance/p119/resume-throw.wasm");
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
fn load(policy: ExecutionPolicy, fuel: Option<u64>) -> (Store, Instance) {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.fuel = fuel;
    let e = Engine::with_execution_policy(c, policy).unwrap();
    let m = e.compile(BYTES).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    drop(m);
    drop(e);
    (s, i)
}
fn captured(s: &mut Store, i: Instance, name: &str) -> (RootToken, Value, usize) {
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = call(s, i, name, &[]).unwrap()
    else {
        panic!()
    };
    assert!(payload.is_empty());
    let root = s.root(continuation).unwrap();
    let value = Value::Ref(RefValue::Gc(root.reference()));
    s.release_published(&[value]);
    assert_eq!(s.collect().unwrap().live, 1);
    let bytes = s.continuation_bytes();
    assert!(bytes > 0);
    (root, value, bytes)
}
fn clean(s: &mut Store, root: RootToken) {
    drop(root);
    s.collect().unwrap();
    assert_eq!(s.continuation_bytes(), 0);
    assert_eq!(s.gc_payload_bytes(), 0);
}
#[test]
fn p119_resume_restores_original_next_source_and_saved_fuel_without_prefix_reentry() {
    let mut snapshots = vec![];
    for policy in POLICIES {
        let (mut s, i) = load(policy, None);
        let (root, value, bytes) = captured(&mut s, i, "run_plain");
        if policy != ExecutionPolicy::Reference {
            let f = function(&s, i, "run_plain");
            s.request_materialization_roundtrip(f, 5).unwrap();
        }
        let before = s.compact_prefix_counts();
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(23)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(23)])
        );
        if policy != ExecutionPolicy::Reference {
            let snapshot = s.execution_snapshot().unwrap().clone();
            assert_eq!(snapshot.cursor.source_pc, 5);
            assert!(snapshot.pending_boundary_charged);
            snapshots.push(snapshot);
        }
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, last) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 2);
            assert_eq!(exits[2] - before.1[2], 1);
            assert_eq!(last, Some((2, 0xe3)));
        }
        assert!(s.continuation_bytes() < bytes);
        assert_eq!(
            call(&mut s, i, "resume", &[value, Value::I32(23)])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
        clean(&mut s, root);
    }
    assert_eq!(snapshots[0], snapshots[1]);
    for fuel in 0..=9 {
        for policy in POLICIES {
            let (mut s, i) = load(policy, Some(fuel));
            if fuel < 5 {
                assert_eq!(
                    call(&mut s, i, "run_plain", &[]).unwrap_err().message,
                    "fuel exhausted"
                );
                continue;
            }
            let (root, value, _) = captured(&mut s, i, "run_plain");
            let result = call(&mut s, i, "resume", &[value, Value::I32(23)]);
            if fuel < 7 {
                assert_eq!(result.unwrap_err().message, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(23)]));
            }
            assert_eq!(
                call(&mut s, i, "resume", &[value, Value::I32(23)])
                    .unwrap_err()
                    .kind,
                ErrorKind::Trap
            );
            clean(&mut s, root);
        }
    }
}
#[test]
fn p119_caught_resume_throw_unwinds_nested_frames_at_exact_fuel_boundaries() {
    for fuel in 0..=17 {
        for policy in POLICIES {
            let (mut s, i) = load(policy, Some(fuel));
            if fuel < 10 {
                assert_eq!(
                    call(&mut s, i, "run_caught", &[]).unwrap_err().message,
                    "fuel exhausted"
                );
                assert_eq!(s.continuation_bytes(), 0);
                continue;
            }
            let (root, value, bytes) = captured(&mut s, i, "run_caught");
            let before = s.compact_prefix_counts();
            let result = call(&mut s, i, "resume_throw", &[value, Value::I32(29)]);
            if fuel < 13 {
                assert_eq!(result.unwrap_err().message, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(29)]));
            }
            if policy == ExecutionPolicy::Compact {
                let (steps, exits, last) = s.compact_prefix_counts();
                assert_eq!(before.0, 4);
                assert_eq!(steps - before.0, 2);
                assert_eq!(exits[2] - before.1[2], 1);
                assert_eq!(last, Some((2, 0xe4)));
            }
            assert!(s.continuation_bytes() < bytes);
            assert_eq!(
                call(&mut s, i, "resume_throw", &[value, Value::I32(29)])
                    .unwrap_err()
                    .kind,
                ErrorKind::Trap
            );
            clean(&mut s, root);
        }
    }
}
#[test]
fn p119_unhandled_resume_throw_cancellation_and_foreign_handles_keep_one_shot_cleanup() {
    for policy in POLICIES {
        for fuel in 5..=10 {
            let (mut s, i) = load(policy, Some(fuel));
            let (root, value, bytes) = captured(&mut s, i, "run_plain");
            let before = s.compact_prefix_counts();
            s.cancellation_token().cancel();
            assert_eq!(
                call(&mut s, i, "resume_throw", &[value, Value::I32(31)])
                    .unwrap_err()
                    .message,
                "execution cancelled"
            );
            assert_eq!(s.compact_prefix_counts(), before);
            assert_eq!(s.continuation_bytes(), bytes);
            s.cancellation_token().reset();
            let RunOutcome::Exception(ex) =
                call(&mut s, i, "resume_throw", &[value, Value::I32(31)]).unwrap()
            else {
                panic!()
            };
            let External::Tag(tag) = i.export(&s, "failure").unwrap() else {
                panic!()
            };
            assert_eq!(s.exception_tag(ex).unwrap(), tag);
            assert_eq!(s.exception_payload(ex).unwrap(), [Value::I32(31)]);
            s.release_published(&[Value::Ref(RefValue::Exn(ex))]);
            assert_eq!(
                call(&mut s, i, "resume_throw", &[value, Value::I32(31)])
                    .unwrap_err()
                    .kind,
                ErrorKind::Trap
            );
            clean(&mut s, root);
            assert!(s.exception_tag(ex).is_err());
            assert_eq!(
                call(&mut s, i, "resume_throw", &[value, Value::I32(31)])
                    .unwrap_err()
                    .kind,
                ErrorKind::Host
            );
        }
        let (mut s, i) = load(policy, None);
        let (root, value, _) = captured(&mut s, i, "run_plain");
        let (mut foreign, j) = load(policy, None);
        assert_eq!(
            call(&mut foreign, j, "resume_throw", &[value, Value::I32(31)])
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
        assert_eq!(foreign.compact_prefix_counts(), (0, [0; 4], None));
        clean(&mut s, root);
        // The handler targets the resumer's function label and returns before
        // another source boundary can consume the installed segment fuel.
        let (mut s, i) = load(policy, Some(8));
        let (root, value, _) = captured(&mut s, i, "run_plain");
        let before = s.compact_prefix_counts();
        assert_eq!(
            call(
                &mut s,
                i,
                "resume_throw_caught_here",
                &[value, Value::I32(31)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(31)])
        );
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, last) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 4);
            assert_eq!(exits[1] - before.1[1], 1);
            assert_eq!(last, Some((4, 0x1f)));
        }
        clean(&mut s, root);
        let (root, _, _) = captured(&mut s, i, "run_plain");
        clean(&mut s, root); // Fresh independent fuel, not the saved three.
    }
}
