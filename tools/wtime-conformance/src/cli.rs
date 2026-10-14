use crate::{adapter::Adapter, corpus, report::Report, script};
use std::{path::Path, process::Command};
use wtime_types::FeatureSet;
pub fn encode(input: &Path, output: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
    let b = wast::parser::ParseBuffer::new(&src).map_err(|e| e.to_string())?;
    let mut wat = wast::parser::parse::<wast::Wat<'_>>(&b).map_err(|e| e.to_string())?;
    std::fs::write(output, wat.encode().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn execution_policy(mode: &str) -> Result<wtime_runtime::ExecutionPolicy, String> {
    match mode {
        "decoded" => Ok(wtime_runtime::ExecutionPolicy::Decoded),
        "portable" => Ok(wtime_runtime::ExecutionPolicy::Portable),
        "reference" => Ok(wtime_runtime::ExecutionPolicy::Reference),
        "compact" => Ok(wtime_runtime::ExecutionPolicy::Compact),
        "register" => Ok(wtime_runtime::ExecutionPolicy::Register),
        _ => Err(format!("unknown execution mode {mode}")),
    }
}

fn replay_feature_args(
    opcode_coverage: bool,
    execution_reference: bool,
    compact_executor: bool,
    register_executor: bool,
    performance_counters: bool,
) -> String {
    let mut features = Vec::new();
    if opcode_coverage {
        features.push("opcode-coverage");
    }
    if execution_reference {
        features.push("execution-reference");
    }
    if compact_executor {
        features.push("compact-executor");
    }
    if register_executor {
        features.push("register-executor");
    }
    if performance_counters {
        features.push("performance-counters");
    }
    if features.is_empty() {
        String::new()
    } else {
        format!("--features {} ", features.join(","))
    }
}

pub fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "encode") {
        if args.len() != 4 || args[2] != "-o" {
            return Err("usage: encode input.wat -o output.wasm".into());
        }
        return encode(Path::new(&args[1]), Path::new(&args[3]));
    }
    if args.first().is_none_or(|s| s != "run") {
        return Err("usage: run --mode decoded|portable|reference|compact|register --profile mvp|stable-core|all-phase3-core --suite name --filter prefix|all --report path".into());
    }
    let mut mode = "decoded";
    let mut profile = None;
    let mut suite = None;
    let mut filter = None;
    let mut report_path = None;
    for pair in args[1..].chunks(2) {
        if pair.len() != 2 {
            return Err("missing option value".into());
        }
        match pair[0].as_str() {
            "--mode" => mode = pair[1].as_str(),
            "--profile" => profile = Some(pair[1].as_str()),
            "--suite" => suite = Some(pair[1].as_str()),
            "--filter" => filter = Some(pair[1].as_str()),
            "--report" => report_path = Some(pair[1].as_str()),
            _ => return Err(format!("unknown option {}", pair[0])),
        }
    }
    let policy = execution_policy(mode)?;
    let profile = profile.ok_or("missing profile")?;
    let features = match profile {
        "mvp" => FeatureSet::mvp(),
        "stable-core" => FeatureSet::stable_core(),
        "all-phase3-core" => FeatureSet::default(),
        _ => return Err("unknown profile".into()),
    };
    let suite = suite.ok_or("missing suite")?;
    let report_path = report_path.ok_or("missing report")?;
    let mut report = Report::default();
    report.execution_mode = mode.to_owned();
    Adapter::with_policy(features, policy).map_err(|e| e.to_string())?;
    let proc_exe = std::path::PathBuf::from(format!("/proc/{}/exe", std::process::id()));
    report.binary_sha256 = corpus::hash(&if proc_exe.exists() {
        proc_exe
    } else {
        std::env::current_exe().map_err(|e| e.to_string())?
    })?;
    #[cfg(feature = "opcode-coverage")]
    wtime_runtime::clear_opcode_coverage();
    let mut hashes = vec![format!(
        "corpus-lock:{}",
        corpus::hash(Path::new("docs/architecture/corpus-lock.json"))?
    )];
    match corpus::select(suite, filter.unwrap_or("all")) {
        Ok(files) => {
            for p in files {
                eprintln!("running {}", p.display());
                hashes.push(format!("{}:{}", p.display(), corpus::hash(&p)?));
                let source = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
                let historical = p.components().any(|c| c.as_os_str() == "mvp");
                let mut effective_features = features;
                if historical {
                    effective_features.mutable_globals = true;
                    if report.compatibility.is_empty() {
                        report.compatibility.push("Historical wg-1.0 corpus enables mutable-global imports/exports; strict mvp policy remains the default for other suites. Historical segment syntax and NaN directives are normalized before encoding. Historical segment-does-not-fit unlinkable assertions use current trapping-instantiation outcomes.".into());
                    }
                }
                let mut adapter =
                    Adapter::with_policy(effective_features, policy).map_err(|e| e.to_string())?;
                adapter.legacy_syntax = historical;
                let error_start = report.errors.len();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    script::run(&source, &mut adapter, &mut report)
                }))
                .is_err()
                {
                    report.failed += 1;
                    report
                        .errors
                        .push("internal engine panic (not a guest trap)".into());
                }
                for error in &mut report.errors[error_start..] {
                    *error = format!("{}: {error}", p.display());
                }
                let _captured_print_count = adapter
                    .prints
                    .lock()
                    .map_err(|_| "print sink poisoned")?
                    .len();
            }
        }
        Err(e) => {
            report.failed += 1;
            report.errors.push(e);
        }
    }
    #[cfg(feature = "opcode-coverage")]
    {
        report.executed_opcodes = Some(wtime_runtime::opcode_coverage());
    }
    let mut revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "uncommitted".into());
    if Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|o| !o.stdout.is_empty())
    {
        revision.push_str("-dirty");
    }
    let replay = format!(
        "cargo run --locked -p wtime-conformance {}-- {}",
        replay_feature_args(
            cfg!(feature = "opcode-coverage"),
            cfg!(feature = "execution-reference"),
            cfg!(feature = "compact-executor"),
            cfg!(feature = "register-executor"),
            cfg!(feature = "performance-counters"),
        ),
        args.iter()
            .map(|s| format!("'{}'", s.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Some(parent) = Path::new(report_path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        report_path,
        report.json(profile, suite, &hashes.join(";"), &revision, &replay),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "selected={} executed={} failed={} skipped={} unsupported={}",
        report.selected, report.executed, report.failed, report.skipped, report.unsupported
    );
    if report.ok() {
        Ok(())
    } else {
        Err(format!("conformance failed; see {report_path}"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t012_encode() {
        let base = std::env::temp_dir().join(format!("wtime-encode-{}", std::process::id()));
        let wat = base.with_extension("wat");
        let wasm = base.with_extension("wasm");
        std::fs::write(&wat, "(module)").unwrap();
        encode(&wat, &wasm).unwrap();
        assert_eq!(std::fs::read(&wasm).unwrap(), b"\0asm\x01\0\0\0");
        std::fs::remove_file(wat).unwrap();
        std::fs::remove_file(wasm).unwrap();
    }

    #[test]
    fn compact_mode_selects_compact_execution_policy() {
        assert!(matches!(
            execution_policy("compact"),
            Ok(wtime_runtime::ExecutionPolicy::Compact)
        ));
    }

    #[test]
    fn register_mode_selects_register_execution_policy() {
        assert!(matches!(
            execution_policy("register"),
            Ok(wtime_runtime::ExecutionPolicy::Register)
        ));
    }

    #[test]
    fn unknown_mode_is_rejected() {
        assert_eq!(
            execution_policy("missing").unwrap_err(),
            "unknown execution mode missing"
        );
    }

    #[test]
    fn replay_features_preserve_existing_features_and_add_compact() {
        assert_eq!(
            replay_feature_args(true, true, true, false, false),
            "--features opcode-coverage,execution-reference,compact-executor "
        );
        assert_eq!(
            replay_feature_args(false, false, true, false, false),
            "--features compact-executor "
        );
        assert_eq!(replay_feature_args(false, false, false, false, false), "");
    }

    #[test]
    fn register_replay_features_preserve_optional_tiers() {
        assert_eq!(
            replay_feature_args(true, true, true, true, true),
            "--features opcode-coverage,execution-reference,compact-executor,register-executor,performance-counters "
        );
        assert_eq!(
            replay_feature_args(false, true, false, true, false),
            "--features execution-reference,register-executor "
        );
    }
}

#[cfg(test)]
mod compact_text_tests {
    use wtime_types::{FeatureSet, ResourceLimits};
    fn encode(text: &str) -> Vec<u8> {
        let buffer = wast::parser::ParseBuffer::new(text).unwrap();
        let mut module = wast::parser::parse::<wast::Wat<'_>>(&buffer).unwrap();
        module.encode().unwrap()
    }
    #[test]
    fn t089_case_01() {
        for text in [
            r#"(module (import "m" (item "a" (func)) (item "b" (func))))"#,
            r#"(module (import "m" (item "a") (item "b") (func)))"#,
        ] {
            let bytes = encode(text);
            let decoded = wtime_decode::decode(
                &bytes,
                &FeatureSet::all_phase3_core(),
                &ResourceLimits::default(),
            )
            .unwrap();
            assert_eq!(
                decoded
                    .imports
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>(),
                ["a", "b"]
            );
        }
    }
    #[test]
    fn t089_case_02() {
        let bytes = encode(
            r#"(module (import "m" "z" (func)) (import "m" (item "a") (item "b") (func)) (import "m" "q" (func)))"#,
        );
        let decoded = wtime_decode::decode(
            &bytes,
            &FeatureSet::all_phase3_core(),
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(
            decoded
                .imports
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>(),
            ["z", "a", "b", "q"]
        );
    }
    #[test]
    fn t089_case_03() {
        let mixed = encode(r#"(module (import "m" (item "a" (func)) (item "b" (func))))"#);
        let common = encode(r#"(module (import "m" (item "a") (item "b") (func)))"#);
        assert!(mixed.windows(4).any(|b| b == [1, b'm', 0, 0x7f]));
        assert!(common.windows(4).any(|b| b == [1, b'm', 0, 0x7e]));
        let buffer = wast::parser::ParseBuffer::new(r#"(module (import "m" (item "a")))"#).unwrap();
        assert!(wast::parser::parse::<wast::Wat<'_>>(&buffer).is_err());
    }
}
