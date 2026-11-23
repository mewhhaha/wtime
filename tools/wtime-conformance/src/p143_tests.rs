use super::*;
use wtime_runtime::ExecutionPolicy;
#[cfg(feature = "register-executor")]
use wtime_runtime::{RunOutcome, Value};

#[test]
fn p143_threshold_option_requires_one_positive_u64() {
    for value in ["", "0", "-1", "+3", "3.0", "18446744073709551616"] {
        assert!(entry_threshold_option(&mut None, value).is_err(), "{value}");
    }
    let mut threshold = None;
    entry_threshold_option(&mut threshold, "3").unwrap();
    assert_eq!(threshold, Some(3));
    assert!(entry_threshold_option(&mut threshold, "4").is_err());
    assert_eq!(threshold, Some(3));
}

#[test]
fn p143_non_register_threshold_is_rejected_and_default_stays_decoded() {
    for policy in [
        ExecutionPolicy::Decoded,
        ExecutionPolicy::Reference,
        ExecutionPolicy::Compact,
    ] {
        assert!(
            Adapter::with_policy_and_threshold(FeatureSet::default(), policy, Some(3)).is_err()
        );
    }
    assert!(
        Adapter::with_policy_and_threshold(
            FeatureSet::default(),
            ExecutionPolicy::Register,
            Some(0)
        )
        .is_err()
    );
    let adapter = Adapter::with_policy(FeatureSet::default(), ExecutionPolicy::Decoded).unwrap();
    assert_eq!(adapter.engine.execution_policy(), ExecutionPolicy::Decoded);
}

#[test]
fn p143_report_identifies_threshold_and_strategy() {
    let mut report = Report {
        execution_mode: "register".into(),
        ..Default::default()
    };
    let eager = report.json("all-phase3-core", "test", "hash", "rev", "replay");
    assert!(eager.contains("\"register_entry_threshold\":null"));
    assert!(eager.contains("\"register_strategy\":\"eager\""));
    report.register_entry_threshold = Some(3);
    let adaptive = report.json("all-phase3-core", "test", "hash", "rev", "replay");
    assert!(adaptive.contains("\"register_entry_threshold\":3"));
    assert!(adaptive.contains("\"register_strategy\":\"adaptive\""));
}

#[cfg(all(feature = "register-executor", feature = "execution-reference"))]
#[test]
fn p143_adapter_threshold_promotes_on_third_entry_and_preserves_fallback() {
    use wtime_runtime::{External, RegisterCacheEntryState};
    fn encode(text: &str) -> Vec<u8> {
        let buffer = wast::parser::ParseBuffer::new(text).unwrap();
        wast::parser::parse::<wast::Wat<'_>>(&buffer)
            .unwrap()
            .encode()
            .unwrap()
    }
    let mut adapter = Adapter::with_policy_and_threshold(
        FeatureSet::default(),
        ExecutionPolicy::Register,
        Some(3),
    )
    .unwrap();
    let bytes = encode(
        "(module (func (export \"run\") (param i32) (result i32) local.get 0 i32.const 7 i32.add))",
    );
    let module = adapter.engine.compile(&bytes).unwrap();
    assert_eq!(module.register_cache_snapshot().unwrap().threshold, 3);
    assert_eq!(
        module
            .register_cache_snapshot()
            .unwrap()
            .lifecycle
            .published,
        0
    );
    let instance = adapter
        .linker
        .instantiate(&mut adapter.store, &module)
        .unwrap();
    let External::Func(run) = instance.export(&adapter.store, "run").unwrap() else {
        panic!()
    };
    for turn in 1..=3 {
        let before = adapter.store.register_dispatches();
        assert_eq!(
            adapter.store.invoke(run, &[Value::I32(35)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(42)])
        );
        let snapshot = module.register_cache_snapshot().unwrap();
        assert_eq!(snapshot.lifecycle.published, u64::from(turn == 3));
        if turn < 3 {
            assert_eq!(adapter.store.register_dispatches(), before)
        } else {
            assert!(adapter.store.register_dispatches() > before)
        }
    }
    assert!(matches!(
        module.register_cache_snapshot().unwrap().entries[0].state,
        RegisterCacheEntryState::Ready { .. }
    ));
    let fallback = adapter
        .engine
        .compile(&encode(
            "(module (func (export \"run\") (result v128) v128.const i32x4 0 0 0 0))",
        ))
        .unwrap();
    let instance = adapter
        .linker
        .instantiate(&mut adapter.store, &fallback)
        .unwrap();
    let External::Func(run) = instance.export(&adapter.store, "run").unwrap() else {
        panic!()
    };
    let before = adapter.store.register_dispatches();
    let fallbacks = adapter.store.register_fallback_frames();
    for _ in 0..3 {
        assert!(matches!(
            adapter.store.invoke(run, &[]).unwrap(),
            RunOutcome::Returned(_)
        ));
    }
    assert_eq!(adapter.store.register_dispatches(), before);
    assert!(adapter.store.register_fallback_frames() > fallbacks);
    assert_eq!(
        fallback
            .register_cache_snapshot()
            .unwrap()
            .lifecycle
            .published,
        0
    );
}
