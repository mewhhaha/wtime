use wtime_runtime::*;
#[test]
fn p131_default_policy_remains_decoded() {
    assert_eq!(
        Engine::new(Config::default()).unwrap().execution_policy(),
        ExecutionPolicy::Decoded
    );
    assert_eq!(
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Portable)
            .unwrap()
            .execution_policy(),
        ExecutionPolicy::Decoded
    );
    #[cfg(not(feature = "register-executor"))]
    assert!(Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).is_err());
}
