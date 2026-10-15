"""Pure validation tests for retained P110 build-budget reports."""
from __future__ import annotations

from copy import deepcopy
import unittest

from tools.verification.producers import p110_builds


METRICS = p110_builds.METRICS


def fixture_set():
    current = {
        "Cargo.toml": "a" * 64,
        "Cargo.lock": "b" * 64,
        "crates/wtime-cli/src/main.rs": "c" * 64,
        "crates/wtime-runtime/src/numeric.rs": "d" * 64,
    }
    digest = p110_builds._load_build_budget().hash_inputs(current)
    reports = {}
    results = {}
    base_metrics = {
        "clean_seconds": 2.5,
        "unchanged_seconds": 0.05,
        "leaf_seconds": 0.15,
        "runtime_leaf_seconds": 0.4,
        "release_seconds": 5.5,
        "stripped_bytes": 1_000_000,
        "clean_peak_rss_kib": 220_000,
        "unchanged_peak_rss_kib": 25_000,
    }
    for name, (mode, feature) in p110_builds.MODES.items():
        features = [] if feature is None else [feature]
        report = {
            "schema_version": 2,
            "mode": mode,
            "features": features,
            "feature_package": None if feature is None else "wtime-runtime",
            "feature_name": None if feature is None else feature.split("/", 1)[1],
            "source_inputs": dict(current),
            "source_inputs_after": dict(current),
            "isolated_source_inputs_after": dict(current),
            "source_sha256": digest,
            "source_sha256_after": digest,
            "isolated_workspace_source_sha256": digest,
            "source_stable": True,
            "lock_sha256": current["Cargo.lock"],
            "jobs": 5,
            "logical_cpus": 8,
            "build_command": ["cargo", "build", "--locked", "--offline", "-j", "5"]
                + ([] if feature is None else ["--features", feature]),
            "target_initially_empty": True,
            "cargo_version": "cargo 1.99.0",
            "rustc": "rustc 1.99.0 (fixture)",
            "cpu": "x86_64",
            "compile_environment": {},
            "resolved_graph": [{"name": "wtime-runtime", "features": features}],
            "budgets": dict(p110_builds.LIMITS),
            "default_dependency_closure_pass": True,
            "default_dependency_violations": [],
            "budget_pass": True,
            **dict(base_metrics),
        }
        reports[name] = report
        argv = ["/usr/bin/python3", "-B", "/workspace/wtime/tools/checks/build_budget.py",
                "--output", f"/workspace/wtime/target/verification/runs/P067/verify/attempt-0001/{name}-build.json"]
        if feature is not None:
            argv += ["--features", feature]
        results[name] = {"name": f"{name}-budget", "argv": argv,
                         "cwd": str(p110_builds.ROOT), "exit_code": 0}
    dependency = "Production dependency/unsafe budget: 0 violations\n"
    return reports, current, results, dependency


class P110BuildReaderTests(unittest.TestCase):
    def test_accepts_source_current_default_and_optional_reports(self):
        reports, current, results, dependency = fixture_set()
        summary = p110_builds.validate_reports(reports, current, results, dependency)
        self.assertEqual(summary["status"], "passed")
        self.assertEqual(summary["baseline"]["kind"], "same-source default build report")
        self.assertEqual(summary["p001_build_comparison"]["available"], False)
        self.assertFalse(summary["optional_tier_review"]["speedup_claim"])
        self.assertEqual(set(summary["reports"]), {"default", "register", "compact"})

    def test_optional_ratios_are_review_only_and_never_create_a_speedup_claim(self):
        reports, current, results, dependency = fixture_set()
        reports["register"]["clean_seconds"] *= 1.3
        summary = p110_builds.validate_reports(reports, current, results, dependency)
        self.assertTrue(summary["optional_tier_review"]["review_required"])
        self.assertEqual(summary["optional_tier_review"]["ratios"]["register"]["clean_seconds"], 1.3)
        self.assertFalse(summary["optional_tier_review"]["speedup_claim"])

    def test_rejects_stale_current_or_post_build_source_maps_and_hashes(self):
        reports, current, results, dependency = fixture_set()
        changed_current = dict(current)
        changed_current["crates/wtime-cli/src/main.rs"] = "e" * 64
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(reports, changed_current, results, dependency)

        for field in ("source_inputs_after", "isolated_source_inputs_after"):
            mutated = deepcopy(reports)
            mutated["default"][field]["Cargo.toml"] = "f" * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                p110_builds.validate_reports(mutated, current, results, dependency)

        mutated = deepcopy(reports)
        mutated["compact"]["source_sha256_after"] = "0" * 64
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(mutated, current, results, dependency)

    def test_rejects_mixed_toolchains_jobs_and_wrong_optional_features(self):
        reports, current, results, dependency = fixture_set()
        changed = deepcopy(reports)
        changed["register"]["rustc"] = "rustc different"
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(changed, current, results, dependency)

        changed = deepcopy(reports)
        changed["compact"]["jobs"] = 4
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(changed, current, results, dependency)

        changed = deepcopy(reports)
        changed["register"]["features"] = ["wtime-runtime/compact-executor"]
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(changed, current, results, dependency)

        changed_results = deepcopy(results)
        changed_results["compact"]["argv"][-1] = "wtime-runtime/register-executor"
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(reports, current, changed_results, dependency)

    def test_rejects_failed_build_stages_hard_budget_and_unrestored_leaf(self):
        reports, current, results, dependency = fixture_set()
        changed_results = deepcopy(results)
        changed_results["default"]["exit_code"] = 1
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(reports, current, changed_results, dependency)

        for mutate in (
            lambda row: row.__setitem__("clean_seconds", 61.0),
            lambda row: row.__setitem__("budget_pass", False),
            lambda row: row["source_inputs_after"].__setitem__("crates/wtime-runtime/src/numeric.rs", "0" * 64),
        ):
            changed = deepcopy(reports)
            mutate(changed["default"])
            with self.assertRaises(ValueError):
                p110_builds.validate_reports(changed, current, results, dependency)

    def test_requires_the_existing_zero_violation_dependency_report(self):
        reports, current, results, dependency = fixture_set()
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(reports, current, results, "0 violations")
        changed = deepcopy(reports)
        changed["default"]["default_dependency_violations"] = ["external dependency"]
        with self.assertRaises(ValueError):
            p110_builds.validate_reports(changed, current, results, dependency)


if __name__ == "__main__":
    unittest.main()
