#![cfg(all(feature = "compact-executor", feature = "performance-counters"))]

// Include the report accumulator directly so overflow behavior is tested without
// reaching into or changing the runtime.
#[allow(dead_code)]
mod adapter {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct PreparationCounts {
        pub compile_count: usize,
        pub published_artifacts: usize,
        pub accepted: usize,
        pub declined: usize,
        pub attempted_work_known_sum: usize,
        pub unpublished_artifacts: usize,
        pub attempted_work_unknown_artifacts: usize,
        pub artifact_retained_budget_bytes: usize,
        pub peak_artifact_retained_budget_bytes: usize,
        pub overflowed: bool,
    }
}
#[allow(dead_code)]
#[path = "../src/report.rs"]
mod report;

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "wtime-p121-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write_inside(root: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = root.join(name);
    let parent = path.parent().unwrap().canonicalize().unwrap();
    assert!(
        parent.starts_with(root),
        "synthetic output escaped its temp root"
    );
    assert!(
        path.starts_with(root),
        "synthetic output escaped its temp root"
    );
    fs::write(&path, contents).unwrap();
    path
}

fn field<'a>(json: &'a str, name: &str) -> &'a str {
    let needle = format!("\"{name}\":");
    let start = json.find(&needle).unwrap() + needle.len();
    let tail = json[start..].trim_start();
    let end = tail
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(tail.len());
    &tail[..end]
}

fn run_cli_raw(source: &str, report_name: &str) -> (PathBuf, bool, String) {
    let root = temp_root();
    let input = write_inside(&root, "suite.wast", source);
    let report = root.join(report_name);
    assert!(report.starts_with(&root));
    let binary = env!("CARGO_BIN_EXE_wtime-conformance");
    let output = Command::new(binary)
        .current_dir(env!("CARGO_MANIFEST_DIR").to_owned() + "/../..")
        .args([
            "run",
            "--mode",
            "compact",
            "--profile",
            "all-phase3-core",
            "--suite",
            input.to_str().unwrap(),
            "--filter",
            "all",
            "--report",
            report.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let json = fs::read_to_string(&report).unwrap();
    (root, output.status.success(), json)
}

fn run_cli(source: &str, report_name: &str) -> (PathBuf, String) {
    let (root, success, json) = run_cli_raw(source, report_name);
    assert!(success, "conformance command should pass: {json}");
    (root, json)
}

#[test]
fn p121_compact_counts_are_per_module_and_checked() {
    let (_root, output) = run_cli(
        "(module (func (export \"a\") (result i32) i32.const 1)) \
         (assert_return (invoke \"a\") (i32.const 1)) \
         (module (func (export \"b\") (result i32) i32.const 2)) \
         (assert_return (invoke \"b\") (i32.const 2))",
        "report.json",
    );
    assert_eq!(field(&output, "compile_count"), "2");
    assert_eq!(field(&output, "published_artifacts"), "2");
    assert!(field(&output, "accepted").parse::<usize>().unwrap() > 0);
    assert!(
        field(&output, "attempted_work_known_sum")
            .parse::<usize>()
            .unwrap()
            > 0
    );
    assert!(
        field(&output, "artifact_retained_budget_bytes")
            .parse::<usize>()
            .unwrap()
            > 0
    );
    let features = output
        .split("--features ")
        .nth(1)
        .and_then(|rest| rest.split([' ', '"']).next())
        .map(|list| list.split(',').collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        features.contains(&"compact-executor") && features.contains(&"performance-counters"),
        "the replay command retains both diagnostic features: {features:?}"
    );
    assert!(
        field(&output, "optimized_compact_source_events")
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn p121_worker_snapshots_merge_once_with_checked_arithmetic() {
    let (_root, output) = run_cli(
        "(module) \
         (thread $first \
             (module (func (export \"a\") (result i32) i32.const 1)) \
             (assert_return (invoke \"a\") (i32.const 1)) \
             (thread $nested \
                 (module (func (export \"b\") (result i32) i32.const 2)) \
                 (assert_return (invoke \"b\") (i32.const 2))) \
             (wait $nested)) \
         (wait $first) \
         (thread $second \
             (module (func (export \"c\") (result i32) i32.const 3)) \
             (assert_return (invoke \"c\") (i32.const 3)))",
        "report.json",
    );
    let total = field(&output, "total_dispatched_source_events")
        .parse::<u64>()
        .unwrap();
    let optimized = field(&output, "optimized_compact_source_events")
        .parse::<u64>()
        .unwrap();
    let fallback = field(&output, "decoded_fallback_source_events")
        .parse::<u64>()
        .unwrap();
    // Each worker's const function dispatches i32.const and its implicit end.
    // The nested worker merges at its Wait, the outer worker at its Wait, and
    // the last worker at the final join.
    assert_eq!(optimized, 6);
    assert_eq!(total, 6);
    assert_eq!(fallback, 0);
    assert_eq!(field(&output, "compile_count"), "4");
    assert_eq!(field(&output, "worker_stores_observed"), "3");

    let mut report = report::Report::default();
    report.merge_execution_counts(report::ExecutionCounts {
        total_dispatched_source_events: u64::MAX,
        optimized_compact_source_events: 1,
        decoded_fallback_source_events: u64::MAX - 1,
    });
    report.merge_execution_counts(report::ExecutionCounts {
        total_dispatched_source_events: 1,
        optimized_compact_source_events: 0,
        decoded_fallback_source_events: 1,
    });
    assert_eq!(
        report.failed, 1,
        "checked aggregation overflow must fail the report"
    );

    let mut preparation = report::Report::default();
    preparation.merge_preparation_counts(report::PreparationSummary {
        compile_count: usize::MAX,
        published_artifacts: usize::MAX,
        accepted: usize::MAX,
        declined: usize::MAX,
        attempted_work_known_sum: usize::MAX,
        unpublished_artifacts: usize::MAX,
        attempted_work_unknown_artifacts: usize::MAX,
        artifact_retained_budget_bytes: usize::MAX,
        peak_artifact_retained_budget_bytes: usize::MAX,
    });
    preparation.merge_preparation_counts(report::PreparationSummary {
        compile_count: 1,
        published_artifacts: 1,
        accepted: 1,
        declined: 1,
        attempted_work_known_sum: 1,
        unpublished_artifacts: 1,
        attempted_work_unknown_artifacts: 1,
        artifact_retained_budget_bytes: 1,
        peak_artifact_retained_budget_bytes: 1,
    });
    assert_eq!(
        preparation.failed, 1,
        "checked preparation aggregation overflow must fail the report"
    );
}

#[test]
fn p121_fallback_is_derived_without_double_count_and_hidden_failures_fail() {
    let (_root, output) = run_cli(
        "(module (func (export \"f\") (result i32) i32.const 7)) \
         (assert_return (invoke \"f\") (i32.const 7))",
        "report.json",
    );
    let total = field(&output, "total_dispatched_source_events")
        .parse::<u64>()
        .unwrap();
    let optimized = field(&output, "optimized_compact_source_events")
        .parse::<u64>()
        .unwrap();
    let fallback = field(&output, "decoded_fallback_source_events")
        .parse::<u64>()
        .unwrap();
    assert_eq!(
        total,
        optimized + fallback,
        "optimized events are a subset of total dispatches"
    );
    assert!(optimized > 0);

    let mut report = report::Report::default();
    report.execution_mode = "compact".into();
    report.selected = 1;
    report.executed = 1;
    report.execution_counts = Some(report::ExecutionCounts::default());
    assert!(
        report.ok(),
        "directive outcome status remains independent of coverage gating"
    );
    report.execution_counts = Some(report::ExecutionCounts {
        total_dispatched_source_events: 1,
        optimized_compact_source_events: 1,
        decoded_fallback_source_events: 0,
    });
    report.skipped = 1;
    assert!(
        !report.ok(),
        "a skipped directive remains visible despite optimized work"
    );

    let (_root, success, failed) = run_cli_raw(
        "(module (func (export \"f\") (result i32) i32.const 7)) \
         (assert_return (invoke \"f\") (i32.const 8))",
        "failed-report.json",
    );
    assert!(!success, "a failed assertion must fail the CLI status");
    assert_eq!(field(&failed, "failed"), "1");
    assert!(
        failed.contains("result mismatch"),
        "failure detail must remain in the report"
    );
}
