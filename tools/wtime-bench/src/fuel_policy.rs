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
}

impl FuelArgument {
    pub const OMITTED: Self = Self {
        policy: SourceFuel::Unmetered,
        explicit: false,
    };

    pub fn parse(args: &[String], base: usize) -> Result<Self, &'static str> {
        if args.len() == base {
            return Ok(Self::OMITTED);
        }
        if args.len() != base + 2 || args[base] != "--source-fuel" {
            return Err("expected optional --source-fuel none|50000000");
        }
        let policy = match args[base + 1].as_str() {
            "none" => SourceFuel::Unmetered,
            "50000000" => SourceFuel::Metered,
            _ => return Err("source fuel must be none or 50000000"),
        };
        Ok(Self { policy, explicit: true })
    }

    pub fn configure(self, config: &mut Config) {
        config.limits.fuel = self.policy.budget();
    }

    pub fn json_suffix(self) -> &'static str {
        match (self.explicit, self.policy) {
            (false, _) => "",
            (true, SourceFuel::Unmetered) => ",\"fuel_policy\":\"unmetered\",\"source_fuel\":null",
            (true, SourceFuel::Metered) => ",\"fuel_policy\":\"metered\",\"source_fuel\":50000000",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(base: usize, suffix: &[&str]) -> Vec<String> {
        (0..base).map(|_| "base".to_owned())
            .chain(suffix.iter().map(|value| (*value).to_owned())).collect()
    }

    #[test]
    fn p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity() {
        for base in [8, 10] {
            let old = FuelArgument::parse(&args(base, &[]), base).unwrap();
            assert_eq!(old, FuelArgument::OMITTED);
            assert_eq!(old.json_suffix(), "");
            let explicit = FuelArgument::parse(&args(base, &["--source-fuel", "none"]), base).unwrap();
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
        assert!(argument.json_suffix().contains("\"fuel_policy\":\"metered\""));
        FuelArgument::OMITTED.configure(&mut config);
        assert_eq!(config.limits.fuel, None);
    }

    #[test]
    fn p127_malformed_or_unsupported_fuel_arguments_are_rejected() {
        for base in [8, 10] {
            for token in ["true", "false", "-1", "0", "1", "50000000.0", "50000001", "None", "", "18446744073709551616"] {
                assert!(FuelArgument::parse(&args(base, &["--source-fuel", token]), base).is_err());
            }
            assert!(FuelArgument::parse(&args(base, &["--source-fuel"]), base).is_err());
            assert!(FuelArgument::parse(&args(base, &["--fuel", "none"]), base).is_err());
            assert!(FuelArgument::parse(&args(base, &["--source-fuel", "none", "extra"]), base).is_err());
            assert!(FuelArgument::parse(&[], base).is_err());
        }
    }
}
