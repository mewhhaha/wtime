use wtime_runtime::{Config, Engine, ErrorKind, ExecutionPolicy};
#[test]
fn p057_register_policy_stays_unavailable_until_execution_lands() {
    assert_eq!(
        Engine::new(Config::default()).unwrap().execution_policy(),
        ExecutionPolicy::Decoded
    );
    assert_eq!(
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register)
            .unwrap_err()
            .kind,
        ErrorKind::DisabledFeature
    );
}
