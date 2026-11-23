#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p118/provider.wasm");
const CONSUMER: &[u8] = include_bytes!("../fixtures/performance/p118/consumer.wasm");
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
fn load(policy: ExecutionPolicy, fuel: Option<u64>, limit: usize) -> (Store, Instance, Tag) {
    let mut c = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    c.limits.fuel = fuel;
    c.limits.max_continuation_bytes = limit;
    let e = Engine::with_execution_policy(c, policy).unwrap();
    let pm = e.compile(PROVIDER).unwrap();
    let cm = e.compile(CONSUMER).unwrap();
    let mut s = Store::new(&e);
    let p = Linker::new(&e).instantiate(&mut s, &pm).unwrap();
    let External::Tag(tag) = p.export(&s, "yield").unwrap() else {
        panic!()
    };
    let mut l = Linker::new(&e);
    l.define("provider", "yield", External::Tag(tag)).unwrap();
    let host = s
        .create_host_func(FuncType::new(vec![], vec![]), |_| {
            Err(Error::new(ErrorKind::Host, "deliberate host failure"))
        })
        .unwrap();
    l.define("env", "fail", External::Func(host)).unwrap();
    let i = l.instantiate(&mut s, &cm).unwrap();
    drop(pm);
    drop(cm);
    drop(e);
    (s, i, tag)
}
fn capture(s: &mut Store, i: Instance, tag: Tag) -> (RootToken, Value, usize) {
    let RunOutcome::Suspended {
        tag: actual,
        payload,
        continuation,
    } = call(s, i, "run", &[]).unwrap()
    else {
        panic!()
    };
    assert_eq!(actual, tag);
    assert!(payload.is_empty());
    let bytes = s.continuation_bytes();
    assert!(bytes > 0);
    let root = s.root(continuation).unwrap();
    let value = Value::Ref(RefValue::Gc(root.reference()));
    s.release_published(&[value]);
    assert_eq!(s.collect().unwrap().live, 1);
    (root, value, bytes)
}
fn scalar_ref(s: &mut Store, i: Instance) -> Value {
    let RunOutcome::Returned(values) = call(s, i, "make", &[]).unwrap() else {
        panic!()
    };
    assert_eq!(values.len(), 1);
    values[0]
}
#[test]
fn p118_cross_module_capture_retains_source_context_roots_and_fuel() {
    let mut snapshots = vec![];
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact] {
        let (mut s, i, tag) = load(policy, None, usize::MAX);
        let run = function(&s, i, "run");
        s.request_materialization_roundtrip(run, 4).unwrap();
        let (root, value, bytes) = capture(&mut s, i, tag);
        let snapshot = s.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc, 4);
        assert!(snapshot.pending_boundary_charged);
        snapshots.push(snapshot);
        if policy == ExecutionPolicy::Compact {
            assert_eq!(
                s.compact_prefix_counts(),
                (4, [0, 0, 1, 0], Some((4, 0xe2)))
            );
        }
        let object = scalar_ref(&mut s, i);
        s.release_published(&[object]);
        let before = s.compact_prefix_counts();
        assert_eq!(
            call(&mut s, i, "resume", &[value, object]).unwrap(),
            RunOutcome::Returned(vec![object])
        );
        assert!(s.continuation_bytes() < bytes);
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, last) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 2);
            assert_eq!(exits[2] - before.1[2], 1);
            assert_eq!(last, Some((2, 0xe3)));
        }
        assert_eq!(
            call(&mut s, i, "get", &[object]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(
            call(&mut s, i, "resume", &[value, object])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
        s.release_published(&[object]);
        drop(root);
        s.collect().unwrap();
        assert_eq!(s.continuation_bytes(), 0);
        assert_eq!(s.gc_payload_bytes(), 0);
    }
    assert_eq!(snapshots[0], snapshots[1]);
    for fuel in 0..=8 {
        for policy in POLICIES {
            let (mut s, i, tag) = load(policy, Some(fuel), usize::MAX);
            if fuel < 5 {
                assert_eq!(
                    call(&mut s, i, "run", &[]).unwrap_err().message,
                    "fuel exhausted"
                );
                assert_eq!(s.continuation_bytes(), 0);
                if policy == ExecutionPolicy::Compact {
                    assert_eq!(s.compact_prefix_counts().0, fuel.min(4));
                }
                continue;
            }
            let (root, value, _) = capture(&mut s, i, tag);
            let object = scalar_ref(&mut s, i);
            s.release_published(&[object]);
            let result = call(&mut s, i, "resume", &[value, object]);
            if fuel < 7 {
                assert_eq!(result.unwrap_err().message, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![object]));
            }
            assert_eq!(
                call(&mut s, i, "resume", &[value, object])
                    .unwrap_err()
                    .kind,
                ErrorKind::Trap
            );
            s.release_published(&[object]);
            drop(root);
            s.collect().unwrap();
            assert_eq!(s.continuation_bytes(), 0);
            assert_eq!(s.gc_payload_bytes(), 0);
        }
    }
}
#[test]
fn p118_successful_bind_retains_bound_reference_and_consumes_old_capability() {
    for policy in POLICIES {
        let (mut s, i, tag) = load(policy, None, usize::MAX);
        let (old, value, _) = capture(&mut s, i, tag);
        let object = scalar_ref(&mut s, i);
        let before = s.compact_prefix_counts();
        let RunOutcome::Returned(values) = call(&mut s, i, "bind", &[value, object]).unwrap()
        else {
            panic!()
        };
        let [Value::Ref(RefValue::Gc(bound))] = values.as_slice() else {
            panic!()
        };
        let root = s.root(*bound).unwrap();
        let bound = Value::Ref(RefValue::Gc(root.reference()));
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, last) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 2);
            assert_eq!(exits[2] - before.1[2], 1);
            assert_eq!(last, Some((2, 0xe1)));
        }
        s.release_published(&[object, bound]);
        drop(old);
        assert_eq!(s.collect().unwrap().live, 2);
        assert_eq!(
            call(&mut s, i, "resume_bound", &[bound]).unwrap(),
            RunOutcome::Returned(vec![object])
        );
        assert_eq!(
            call(&mut s, i, "get", &[object]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(
            call(&mut s, i, "resume_bound", &[bound]).unwrap_err().kind,
            ErrorKind::Trap
        );
        s.release_published(&[object]);
        drop(root);
        s.collect().unwrap();
        assert_eq!(s.continuation_bytes(), 0);
        assert_eq!(s.gc_payload_bytes(), 0);
    }
}
#[test]
fn p118_failed_bind_budget_and_cancellation_preserve_ready_original() {
    for policy in POLICIES {
        let (mut probe, i, tag) = load(policy, None, usize::MAX);
        let (token, _, limit) = capture(&mut probe, i, tag);
        drop(token);
        probe.collect().unwrap();
        let (mut s, i, tag) = load(policy, None, limit);
        let (root, value, bytes) = capture(&mut s, i, tag);
        let object = scalar_ref(&mut s, i);
        let object_root = s.root_value(object).unwrap();
        s.release_published(&[object]);
        let before = s.compact_prefix_counts();
        let error = call(&mut s, i, "host_failure", &[value, object]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Host);
        assert_eq!(error.message, "deliberate host failure");
        assert_eq!(s.continuation_bytes(), bytes);
        assert_eq!(s.collect().unwrap().live, 2);
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, _) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 3);
            assert_eq!(exits, before.1);
        }
        let before = s.compact_prefix_counts();
        let error = call(&mut s, i, "bind", &[value, object]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert_eq!(error.message, "continuation byte budget");
        assert_eq!(s.continuation_bytes(), bytes);
        assert_eq!(s.collect().unwrap().live, 2);
        if policy == ExecutionPolicy::Compact {
            let (steps, exits, last) = s.compact_prefix_counts();
            assert_eq!(steps - before.0, 2);
            assert_eq!(exits[2] - before.1[2], 1);
            assert_eq!(last, Some((2, 0xe1)));
        }
        let before = s.compact_prefix_counts();
        s.cancellation_token().cancel();
        assert_eq!(
            call(&mut s, i, "bind", &[value, object])
                .unwrap_err()
                .message,
            "execution cancelled"
        );
        assert_eq!(s.compact_prefix_counts(), before);
        assert_eq!(s.continuation_bytes(), bytes);
        s.cancellation_token().reset();
        assert_eq!(
            call(&mut s, i, "resume", &[value, object]).unwrap(),
            RunOutcome::Returned(vec![object])
        );
        assert_eq!(
            call(&mut s, i, "resume", &[value, object])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
        s.release_published(&[object]);
        drop(object_root);
        drop(root);
        s.collect().unwrap();
        assert_eq!(s.continuation_bytes(), 0);
        assert_eq!(s.gc_payload_bytes(), 0);
    }
}
