//! An optional source-fuel argument, parsed before any observation starts.
use wtime_runtime::Config;

pub const CONTRACT: &str = "{\"schema_version\":1,\"accepted\":[{\"name\":\"unmetered\",\"source_fuel\":null},{\"name\":\"metered\",\"source_fuel\":50000000}]}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFuel {
    Unmetered,
    Metered,
}

impl SourceFuel {
    pub fn budget(self) -> Option<u64> {
        match self {
            Self::Unmetered => None,
            Self::Metered => Some(50_000_000),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuelArgument {
    pub policy: SourceFuel,
    pub explicit: bool,
    pub register_entry_threshold: Option<u64>,
}

impl FuelArgument {
    pub const OMITTED: Self = Self {
        policy: SourceFuel::Unmetered,
        explicit: false,
        register_entry_threshold: None,
    };

    pub fn parse(args: &[String], base: usize) -> Result<Self, &'static str> {
        if args.len() < base || args.len() > base + 4 || (args.len() - base) % 2 != 0 {
            return Err("expected optional source-fuel and register-entry-threshold pairs");
        }
        let mut result = Self::OMITTED;
        for pair in args[base..].chunks_exact(2) {
            match pair[0].as_str() {
                "--source-fuel" if !result.explicit => {
                    result.policy = match pair[1].as_str() {
                        "none" => SourceFuel::Unmetered,
                        "50000000" => SourceFuel::Metered,
                        _ => return Err("source fuel must be none or 50000000"),
                    };
                    result.explicit = true;
                }
                "--register-entry-threshold" if result.register_entry_threshold.is_none() => {
                    if pair[1].is_empty() || !pair[1].bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err("register entry threshold must be a positive u64");
                    }
                    let threshold = pair[1]
                        .parse::<u64>()
                        .map_err(|_| "register entry threshold must be a positive u64")?;
                    if threshold == 0 {
                        return Err("register entry threshold must be positive");
                    }
                    result.register_entry_threshold = Some(threshold);
                }
                _ => return Err("unknown or duplicate timing option"),
            }
        }
        Ok(result)
    }

    pub fn validate_mode(self, mode: &str) -> Result<(), &'static str> {
        if self.register_entry_threshold.is_some()
            && (mode != "register"
                || !cfg!(all(
                    feature = "register-executor",
                    not(feature = "legacy-api")
                )))
        {
            return Err("register entry threshold requires available Register mode");
        }
        Ok(())
    }

    pub fn configure(self, config: &mut Config) {
        config.limits.fuel = self.policy.budget();
    }

    pub fn json_suffix(self) -> String {
        let mut suffix = match (self.explicit, self.policy) {
            (false, _) => "",
            (true, SourceFuel::Unmetered) => ",\"fuel_policy\":\"unmetered\",\"source_fuel\":null",
            (true, SourceFuel::Metered) => ",\"fuel_policy\":\"metered\",\"source_fuel\":50000000",
        }
        .to_owned();
        if let Some(threshold) = self.register_entry_threshold {
            suffix.push_str(&format!(",\"register_entry_threshold\":{threshold}"));
        }
        suffix
    }
}

pub fn entry_capability_suffix() -> &'static str {
    if cfg!(all(
        feature = "register-executor",
        not(feature = "legacy-api")
    )) {
        ",\"register_entry_threshold_contract\":{\"schema_version\":1,\"argument\":\"--register-entry-threshold\",\"mode\":\"register\",\"minimum\":1,\"maximum\":18446744073709551615}"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(base: usize, suffix: &[&str]) -> Vec<String> {
        (0..base)
            .map(|_| "base".to_owned())
            .chain(suffix.iter().map(|value| (*value).to_owned()))
            .collect()
    }

    #[test]
    fn p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity() {
        for base in [8, 10] {
            let old = FuelArgument::parse(&args(base, &[]), base).unwrap();
            assert_eq!(old, FuelArgument::OMITTED);
            assert_eq!(old.json_suffix(), "");
            let explicit =
                FuelArgument::parse(&args(base, &["--source-fuel", "none"]), base).unwrap();
            assert_eq!(explicit.policy.budget(), None);
            assert!(explicit.explicit);
            assert!(explicit.json_suffix().contains("\"source_fuel\":null"));
        }
    }

    #[test]
    fn p127_fixed_metered_policy_sets_fuel_without_changing_other_limits() {
        let argument = FuelArgument::parse(&args(10, &["--source-fuel", "50000000"]), 10).unwrap();
        let mut config = Config::default();
        config.limits.max_call_depth = 7;
        config.limits.max_stack_height = 13;
        argument.configure(&mut config);
        assert_eq!(config.limits.fuel, Some(50_000_000));
        assert_eq!(config.limits.max_call_depth, 7);
        assert_eq!(config.limits.max_stack_height, 13);
        assert!(
            argument
                .json_suffix()
                .contains("\"fuel_policy\":\"metered\"")
        );
        FuelArgument::OMITTED.configure(&mut config);
        assert_eq!(config.limits.fuel, None);
    }

    #[test]
    fn p127_malformed_or_unsupported_fuel_arguments_are_rejected() {
        for base in [8, 10] {
            for token in [
                "true",
                "false",
                "-1",
                "0",
                "1",
                "50000000.0",
                "50000001",
                "None",
                "",
                "18446744073709551616",
            ] {
                assert!(FuelArgument::parse(&args(base, &["--source-fuel", token]), base).is_err());
            }
            assert!(FuelArgument::parse(&args(base, &["--source-fuel"]), base).is_err());
            assert!(FuelArgument::parse(&args(base, &["--fuel", "none"]), base).is_err());
            assert!(
                FuelArgument::parse(&args(base, &["--source-fuel", "none", "extra"]), base)
                    .is_err()
            );
            assert!(FuelArgument::parse(&[], base).is_err());
        }
    }
}
