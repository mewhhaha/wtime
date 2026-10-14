use wtime_runtime::*;
const WASM: &[u8] = b"\0asm\x01\0\0\0\x01\x06\x01\x60\x01\x7f\x01\x7f\x03\x02\x01\0\x07\x07\x01\x03run\0\0\x0a\x09\x01\x07\0\x20\0\x41\x01\x6a\x0b";
fn guest(e: &Engine) -> (Store, Func) {
    let m = e.compile(WASM).unwrap();
    let mut s = Store::new(e);
    let i = Linker::new(e).instantiate(&mut s, &m).unwrap();
    let External::Func(f) = i.export(&s, "run").unwrap() else {
        panic!()
    };
    (s, f)
}
#[test]
fn p016_default_and_portable_keep_decoded() {
    for e in [
        Engine::new(Config::default()).unwrap(),
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Portable).unwrap(),
    ] {
        assert_eq!(e.execution_policy(), ExecutionPolicy::Decoded);
        let (mut s, f) = guest(&e);
        assert!(
            matches!(s.invoke(f,&[Value::I32(5)]).unwrap(), RunOutcome::Returned(v) if v == [Value::I32(6)])
        );
        #[cfg(feature = "execution-reference")]
        assert_eq!(s.reference_dispatches(), 0);
    }
}
#[test]
fn p016_absent_tiers_fail_at_configuration() {
    for policy in [
        #[cfg(not(feature = "compact-executor"))]
        ExecutionPolicy::Compact,
        ExecutionPolicy::Register,
        ExecutionPolicy::Experimental,
        ExecutionPolicy::Native,
    ] {
        let e = Engine::with_execution_policy(Config::default(), policy).unwrap_err();
        assert_eq!(e.kind, ErrorKind::DisabledFeature);
        assert!(e.message.contains("unavailable execution policy"));
    }
    #[cfg(not(feature = "execution-reference"))]
    assert!(Engine::with_execution_policy(Config::default(), ExecutionPolicy::Reference).is_err());
}
#[cfg(feature = "execution-reference")]
#[test]
fn p016_reference_executes_archived_dispatch() {
    let e = Engine::with_execution_policy(Config::default(), ExecutionPolicy::Reference).unwrap();
    let (mut s, f) = guest(&e);
    assert!(
        matches!(s.invoke(f,&[Value::I32(i32::MAX)]).unwrap(), RunOutcome::Returned(v) if v == [Value::I32(i32::MIN)])
    );
    assert_eq!(s.reference_dispatches(), 4);
}
#[cfg(feature = "execution-reference")]
#[test]
fn p016_reference_preserves_fuel_failure_boundary() {
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Reference] {
        let mut config = Config::default();
        config.limits.fuel = Some(3);
        let e = Engine::with_execution_policy(config, policy).unwrap();
        let (mut s, f) = guest(&e);
        let error = s.invoke(f, &[Value::I32(5)]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert_eq!(error.message, "fuel exhausted");
        if policy == ExecutionPolicy::Reference {
            assert_eq!(s.reference_dispatches(), 3);
        }
    }
}
