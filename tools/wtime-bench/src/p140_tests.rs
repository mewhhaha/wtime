use super::{
    fuel_policy::{FuelArgument, SourceFuel},
    *,
};
use wtime_runtime::RegisterCacheEntryState;

const ADD: &[u8] = &[
    0, 97, 115, 109, 1, 0, 0, 0, 1, 6, 1, 96, 1, 127, 1, 127, 3, 2, 1, 0, 7, 7, 1, 3, 114, 117,
    110, 0, 0, 10, 9, 1, 7, 0, 32, 0, 65, 1, 106, 11,
];

fn arguments(base: usize, tail: &[&str]) -> Vec<String> {
    (0..base)
        .map(|_| "base".to_owned())
        .chain(tail.iter().map(|value| value.to_string()))
        .collect()
}

#[test]
fn p140_parser_composes_entry_threshold_and_source_fuel() {
    for base in [8, 10] {
        for tail in [
            vec![
                "--register-entry-threshold",
                "3",
                "--source-fuel",
                "50000000",
            ],
            vec![
                "--source-fuel",
                "50000000",
                "--register-entry-threshold",
                "3",
            ],
        ] {
            let setting = FuelArgument::parse(&arguments(base, &tail), base).unwrap();
            assert_eq!(setting.register_entry_threshold, Some(3));
            assert_eq!(setting.policy, SourceFuel::Metered);
            assert!(setting.explicit);
            assert!(setting.validate_mode("register").is_ok());
            assert_eq!(
                setting.json_suffix(),
                ",\"fuel_policy\":\"metered\",\"source_fuel\":50000000,\"register_entry_threshold\":3"
            );
        }
        let max = FuelArgument::parse(
            &arguments(
                base,
                &["--register-entry-threshold", "18446744073709551615"],
            ),
            base,
        )
        .unwrap();
        assert_eq!(max.register_entry_threshold, Some(u64::MAX));
    }
}

#[test]
fn p140_actual_adaptive_engines_preserve_threshold_and_fuel() {
    for scalar in [false, true] {
        for policy in [SourceFuel::Unmetered, SourceFuel::Metered] {
            let setting = FuelArgument {
                policy,
                explicit: true,
                register_entry_threshold: Some(3),
            };
            let engine = if scalar {
                scalar_driver::p140_engine("register", setting)
            } else {
                engine_with_fuel("register", setting)
            };
            let module = engine.compile(ADD).unwrap();
            assert_eq!(module.register_cache_snapshot().unwrap().threshold, 3);
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let function = exported(&store, instance);
            for entry in 1..=3 {
                let dispatches = store.register_dispatches();
                assert_eq!(call(&mut store, function, 7), 8);
                let snapshot = module.register_cache_snapshot().unwrap();
                assert_eq!(snapshot.entries[0].entries, entry);
                assert_eq!(snapshot.lifecycle.builder_starts, u64::from(entry == 3));
                assert_eq!(
                    store.register_dispatches() - dispatches,
                    if entry == 3 { 4 } else { 0 }
                );
                if entry < 3 {
                    assert_eq!(snapshot.entries[0].state, RegisterCacheEntryState::Cold);
                } else {
                    assert!(matches!(
                        snapshot.entries[0].state,
                        RegisterCacheEntryState::Ready { .. }
                    ));
                }
            }
            store
                .request_materialization_roundtrip(function, 0)
                .unwrap();
            assert_eq!(call(&mut store, function, 7), 8);
            let snapshot = store.execution_snapshot().unwrap();
            assert_eq!(
                snapshot.remaining_fuel,
                policy.budget().map(|budget| budget - 1)
            );
            assert!(snapshot.pending_boundary_charged);
            assert_eq!(
                module
                    .register_cache_snapshot()
                    .unwrap()
                    .lifecycle
                    .builder_starts,
                1
            );
        }
    }
}

#[test]
fn p140_bad_entry_settings_are_rejected_before_fixture_io() {
    for base in [8, 10] {
        for value in [
            "0",
            "-1",
            "+3",
            "true",
            "3.0",
            " 3",
            "",
            "18446744073709551616",
        ] {
            assert!(
                FuelArgument::parse(
                    &arguments(base, &["--register-entry-threshold", value]),
                    base
                )
                .is_err()
            );
        }
        for tail in [
            vec!["--register-entry-threshold"],
            vec![
                "--register-entry-threshold",
                "3",
                "--register-entry-threshold",
                "4",
            ],
        ] {
            assert!(FuelArgument::parse(&arguments(base, &tail), base).is_err());
        }
        for mode in ["decoded", "compact", "unrecognized"] {
            let mut args = arguments(base, &["--register-entry-threshold", "3"]);
            args[0] = if base == 8 {
                "--single-case"
            } else {
                "--scalar-driver"
            }
            .into();
            args[1] = "p140-must-not-read-missing.wasm".into();
            args[7] = mode.into();
            let panic = std::panic::catch_unwind(|| {
                if base == 8 {
                    super::try_main(&args)
                } else {
                    scalar_driver::try_main(&args)
                }
            })
            .unwrap_err();
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("");
            assert!(
                message.contains("register entry setting"),
                "unexpected failure: {message}"
            );
        }
    }
}

#[test]
fn p140_omitted_threshold_preserves_legacy_wire() {
    assert_eq!(FuelArgument::OMITTED.json_suffix(), "");
    for (token, suffix) in [
        (
            "none",
            ",\"fuel_policy\":\"unmetered\",\"source_fuel\":null",
        ),
        (
            "50000000",
            ",\"fuel_policy\":\"metered\",\"source_fuel\":50000000",
        ),
    ] {
        let setting = FuelArgument::parse(&arguments(8, &["--source-fuel", token]), 8).unwrap();
        assert_eq!(setting.json_suffix(), suffix);
        for scalar in [false, true] {
            let engine = if scalar {
                scalar_driver::p140_engine("register", setting)
            } else {
                engine_with_fuel("register", setting)
            };
            assert!(
                engine
                    .compile(ADD)
                    .unwrap()
                    .register_cache_snapshot()
                    .is_none()
            );
        }
    }
}
