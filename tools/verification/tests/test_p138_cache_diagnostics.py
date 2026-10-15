"""Contained source-checker tests for P138's cache diagnostic evidence."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "producers/p138.py"
spec = importlib.util.spec_from_file_location("p138_producer_tests", SOURCE)
p138 = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(p138)


class P138CacheDiagnosticsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p138-diagnostics-")
        self.root = Path(self.temp.name).resolve()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def put(self, relative: str, content: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        self.assertTrue(path.resolve().is_relative_to(self.root))
        return path

    @staticmethod
    def trace(case: str, event: str, *, threshold: int = 3, rows=None, lifecycle=None,
              remaining=1000, retained_work=1000, retained=64, active_body=None) -> dict:
        return {
            "case": case, "event": event, "instrumented": True, "timing_evidence": False,
            "engine": 1, "threshold": threshold, "entry_limit": 64, "active_body": active_body,
            "remaining_attempted_work": remaining, "remaining_retained_work": retained_work,
            "retained_bytes": retained, "byte_limit": 4096,
            "lifecycle": lifecycle or {key: 0 for key in p138.LIFECYCLE_FIELDS},
            "rows": [] if rows is None else rows,
        }

    @staticmethod
    def row(body: int, entries: int, attempts: int, state: str) -> dict:
        return {"body": body, "entries": entries, "attempts": attempts, "state": state,
                "cooldown": 0, "ready_bytes": 8 if state == "ready" else 0}

    def trace_matrix(self) -> list[dict]:
        cold = p138.CASE_NAMES[0]
        churn = p138.CASE_NAMES[1]
        records = [self.trace(cold, "initial")]
        for call in range(1, 6):
            built = int(call >= 3)
            lifecycle = {key: 0 for key in p138.LIFECYCLE_FIELDS}
            lifecycle.update(preparation_starts=built, builder_starts=built,
                             completed=built, published=built)
            records.append(self.trace(
                cold, f"call-{call}", rows=[self.row(0, call, built, "ready" if built else "cold")],
                lifecycle=lifecycle,
            ))
        records.append(self.trace(churn, "initial", threshold=1))
        for call in range(1, 68):
            body = 0 if call == 67 else call - 1
            lifecycle = {key: 0 for key in p138.LIFECYCLE_FIELDS}
            lifecycle.update(preparation_starts=call, builder_starts=call, completed=call,
                             published=call, evictions=max(0, call - 64))
            residents = list(range(max(0, call - 64), min(call, 66)))
            if call == 67:
                residents.append(0)
            rows = [self.row(resident, 1, 1, "ready") for resident in residents]
            records.append(self.trace(churn, f"call-{call}-body-{body}", threshold=1,
                                      rows=rows, lifecycle=lifecycle,
                                      remaining=1000 - call, retained_work=1000 - call))
        pinned = p138.CASE_NAMES[3]
        one = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        one.update(preparation_starts=1, builder_starts=1, completed=1, published=1)
        ready0 = self.row(0, 1, 1, "ready")
        records.append(self.trace(pinned, "pinned", rows=[ready0], lifecycle=one, retained=80))
        three = dict(one)
        three.update(preparation_starts=3, builder_starts=3, completed=3, published=3, evictions=1)
        resident12 = [self.row(1, 1, 1, "ready"), self.row(2, 1, 1, "ready")]
        records.append(self.trace(pinned, "evicted-pinned", rows=resident12, lifecycle=three, retained=96))
        records.append(self.trace(pinned, "released", rows=resident12, lifecycle=three, retained=88))

        failures = p138.CASE_NAMES[4]
        decline1 = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        decline1.update(preparation_starts=1, builder_starts=1, declined=1)
        records.append(self.trace(failures, "decline-first",
                                  rows=[{**self.row(0, 1, 1, "cooldown"), "cooldown": 128}],
                                  lifecycle=decline1))
        records.append(self.trace(failures, "decline-cooldown",
                                  rows=[{**self.row(0, (1 << 64) - 1, 1, "cooldown"), "cooldown": 1}],
                                  lifecycle=decline1))
        decline2 = dict(decline1)
        decline2.update(preparation_starts=2, builder_starts=2, declined=2)
        records.append(self.trace(failures, "decline-negative",
                                  rows=[self.row(0, (1 << 64) - 1, 2, "negative")], lifecycle=decline2))
        aborted = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        aborted.update(preparation_starts=1, builder_starts=1, aborted=1)
        records.append(self.trace(failures, "aborted",
                                  rows=[self.row(0, 1, 1, "negative")], lifecycle=aborted,
                                  remaining=990))
        foreign = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        foreign.update(preparation_starts=1, builder_starts=1, completed=1, publication_failures=1)
        records.append(self.trace(failures, "foreign-publication-refused",
                                  rows=[self.row(0, 1, 1, "negative")], lifecycle=foreign))
        invalid = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        invalid.update(preparation_starts=1, builder_starts=1, invalid=1)
        records.append(self.trace(failures, "invalid",
                                  rows=[self.row(0, 1, 1, "negative")], lifecycle=invalid))
        exhausted = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        exhausted.update(preparation_starts=1, builder_starts=1, exhausted=1)
        records.append(self.trace(failures, "exhausted-work",
                                  rows=[self.row(0, 1, 1, "negative")], lifecycle=exhausted))
        wrapper = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        wrapper.update(preparation_starts=1, exhausted=1)
        records.append(self.trace(failures, "exhausted-wrapper",
                                  rows=[self.row(0, 1, 1, "negative")], lifecycle=wrapper))

        active_case = p138.CASE_NAMES[5]
        active_lifecycle = {key: 0 for key in p138.LIFECYCLE_FIELDS}
        active_lifecycle.update(preparation_starts=1, builder_starts=1)
        records.append(self.trace(active_case, "active", rows=[self.row(0, 1, 1, "compiling")],
                                  lifecycle=active_lifecycle, active_body=0))
        published = dict(active_lifecycle)
        published.update(completed=1, published=1)
        records.append(self.trace(active_case, "published", rows=[self.row(0, 1, 1, "ready")],
                                  lifecycle=published))
        saturated = {key: (1 << 64) - 1 for key in p138.LIFECYCLE_FIELDS}
        records.append(self.trace(active_case, "saturated",
                                  rows=[self.row(1, 1, 1, "ready"), self.row(2, 1, 1, "ready")],
                                  lifecycle=saturated))
        return records

    def case_records(self) -> list[dict]:
        expected = {
            p138.CASE_NAMES[0]: {"calls": 5, "cold_calls": 2, "builders": 1},
            p138.CASE_NAMES[1]: {"calls": 67, "distinct_bodies": 66, "builders": 67, "evictions": 3},
            p138.CASE_NAMES[2]: {"fuel_prefixes": 5, "policies": 4},
            p138.CASE_NAMES[3]: {"snapshot_clones_artifact": False, "work_refunded": False},
            p138.CASE_NAMES[4]: {"cooldown_entries": 128, "declines": 2,
                                 "abort_work_charged": True, "poisoned_snapshots_refused": 2,
                                 "foreign_snapshot_refused": True},
            p138.CASE_NAMES[5]: {"active_observed": True, "builder_unlocked": True,
                                 "counters_saturate": True},
        }
        records = []
        for name in p138.CASE_NAMES:
            records.append({"name": name, "passed": True, "instrumented": True,
                            "timing_evidence": False, **expected.get(name, {})})
        return records

    def write_markers(self, relative: str = "logs/p138.stderr", cases=None, trace_cases=None) -> Path:
        selected_cases = self.case_records()
        traces = self.trace_matrix()
        if cases is not None:
            selected_cases = [item for item in selected_cases if item["name"] in cases]
        if trace_cases is not None:
            traces = [item for item in traces if item["case"] in trace_cases]
        lines = ["P138_CASE " + json.dumps(item) for item in selected_cases]
        lines += ["P138_TRACE " + json.dumps(item) for item in traces]
        return self.put(relative, "\n".join(lines) + "\n")

    def test_accepts_complete_bounded_snapshot_trace_sequences(self) -> None:
        path = self.write_markers()
        summary = p138.validate_traces(path)
        self.assertEqual(summary["trace_count"], 88)
        self.assertEqual(summary["per_case"], {
            p138.CASE_NAMES[0]: 6, p138.CASE_NAMES[1]: 68,
            p138.CASE_NAMES[3]: 3, p138.CASE_NAMES[4]: 8, p138.CASE_NAMES[5]: 3,
        })

    def test_rejects_timing_fields_and_unmarked_instrumentation(self) -> None:
        record = self.trace(p138.CASE_NAMES[0], "initial")
        record["total_ns"] = 0
        with self.assertRaisesRegex(p138.P138Error, "schema fields"):
            p138.validate_trace(record)
        record = self.trace(p138.CASE_NAMES[0], "initial")
        record["timing_evidence"] = True
        with self.assertRaisesRegex(p138.P138Error, "non-timing"):
            p138.validate_trace(record)

    def test_rejects_boolean_counters_unknown_rows_and_budget_overflow(self) -> None:
        record = self.trace(p138.CASE_NAMES[0], "initial")
        record["lifecycle"]["builder_starts"] = True
        with self.assertRaisesRegex(p138.P138Error, "integer"):
            p138.validate_trace(record)
        record = self.trace(p138.CASE_NAMES[0], "initial", retained=5000)
        with self.assertRaisesRegex(p138.P138Error, "exceed byte limit"):
            p138.validate_trace(record)
        record = self.trace(p138.CASE_NAMES[0], "initial", rows=[self.row(66, 1, 0, "cold")])
        with self.assertRaisesRegex(p138.P138Error, "exceeds fixture"):
            p138.validate_trace(record)

    def test_accepts_exact_named_case_markers_and_rejects_missing_or_duplicate(self) -> None:
        path = self.put("logs/cases.stderr", "\n".join(
            "P138_CASE " + json.dumps(item) for item in self.case_records()) + "\n")
        self.assertEqual(set(p138.validate_case_records(path)), set(p138.CASE_NAMES))
        duplicate = self.put("logs/duplicate.stderr", path.read_text() + path.read_text().splitlines()[0] + "\n")
        with self.assertRaisesRegex(p138.P138Error, "duplicate"):
            p138.validate_case_records(duplicate)
        missing = self.put("logs/missing.stderr", "\n".join(
            "P138_CASE " + json.dumps(item) for item in self.case_records()[:-1]) + "\n")
        with self.assertRaisesRegex(p138.P138Error, "incomplete"):
            p138.validate_case_records(missing)

    def test_rejects_fifo_reordering_and_recompilation_counter_mismatch(self) -> None:
        records = self.trace_matrix()
        records[73]["rows"][0]["body"] = 1
        path = self.put("logs/reordered.stderr", "\n".join(
            "P138_TRACE " + json.dumps(item) for item in records) + "\n")
        with self.assertRaisesRegex(p138.P138Error, "FIFO resident body order"):
            p138.validate_traces(path)
        records = self.trace_matrix()
        records[3]["lifecycle"]["builder_starts"] = 0
        path = self.put("logs/builder-count.stderr", "\n".join(
            "P138_TRACE " + json.dumps(item) for item in records) + "\n")
        with self.assertRaisesRegex(p138.P138Error, "builder/publication transition"):
            p138.validate_traces(path)

    def test_public_checker_accepts_contained_receipt_and_rejects_stale_log_hash(self) -> None:
        register_path = self.put("target/verification/logs/register.stdout", "\n".join(
            f"test tests::performance::{name} ... ok" for name in p138.FULL_CASES + p138.CASE_NAMES) + "\n")
        p138_path = self.put("target/verification/logs/p138.stdout", register_path.read_text())
        combined_path = self.put("target/verification/logs/combined.stdout", register_path.read_text())
        default_path = self.put("target/verification/logs/default.stdout", "test tests::performance::p132_module_preparation_remains_policy_neutral ... ok\n")
        stderr_path = self.write_markers("target/verification/logs/p138.stderr")
        synthetic_root = self.root
        verification_root = synthetic_root / "target/verification"
        frozen_path = verification_root / "freeze/report.json"
        frozen_path.parent.mkdir(parents=True)
        predecessor = {"commit": "synthetic-p137", "runtime_source_map": {}}
        fixture = {"fixture_sha256": "c" * 64}
        source_map = {"src/lib.rs": "d" * 64}
        map_path = verification_root / "freeze/runtime-source-map.json"
        map_path.write_text(json.dumps({"source_map": source_map,
                                        "source_map_sha256": p138.canonical_sha(source_map)}),
                            encoding="utf-8")
        frozen_path.write_text(json.dumps({
            "schema": "wtime-p138-freeze-v1", "status": "passed", "outcome": "contract_only",
            "timing_evidence": False, "instrumented": True, "gate_eligible": False,
            "predecessor": predecessor, "fixture": fixture,
            "runtime_source_map": p138.ref(map_path),
            "current_source_map_sha256": p138.canonical_sha(source_map),
        }), encoding="utf-8")
        helper_locations = {
            "baseline.py": (synthetic_root / "tools/wtime-bench/baseline.py",
                            p138.ROOT / "tools/wtime-bench/baseline.py"),
            "build_budget.py": (synthetic_root / "tools/checks/build_budget.py",
                                p138.ROOT / "tools/checks/build_budget.py"),
            "dependency_budget.py": (synthetic_root / "tools/checks/dependency_budget.py",
                                     p138.ROOT / "tools/checks/dependency_budget.py"),
        }
        for destination, source in helper_locations.values():
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read_bytes())
        helper_dir = self.root / "target/verification/proof/helpers"
        helper_dir.mkdir(parents=True)
        helper_sources = {
            "p138.py": SOURCE,
            **{name: source for name, (_, source) in helper_locations.items()},
        }
        helper_refs = {}
        for name, source in helper_sources.items():
            destination = helper_dir / name
            destination.write_bytes(source.read_bytes())
            helper_refs[name] = p138.ref(destination)
        build_paths = {}
        builds = {}
        class FakeBudget:
            @staticmethod
            def cargo_metadata(features=None):
                return {"features": features}

            @staticmethod
            def source_inputs(_metadata):
                return {"src/lib.rs": "a" * 64}

            @staticmethod
            def hash_inputs(_inputs):
                return "b" * 64

            @staticmethod
            def check_budget(_record):
                return True

        for key, mode, features in (("default", "default", []),
                                    ("register", "optional-feature", ["wtime-runtime/register-executor"])):
            build_path = self.put(f"target/verification/build/{key}.json", json.dumps({
                "schema_version": 2, "mode": mode, "features": features,
                "source_stable": True, "budget_pass": True,
                "default_dependency_closure_pass": True, "default_dependency_violations": [],
                "source_inputs": {"src/lib.rs": "a" * 64},
                "source_inputs_after": {"src/lib.rs": "a" * 64},
                "isolated_source_inputs_after": {"src/lib.rs": "a" * 64},
                "stripped_bytes": 1200, "source_sha256": "b" * 64,
                "source_sha256_after": "b" * 64,
                "isolated_workspace_source_sha256": "b" * 64,
            }))
            build_paths[key] = build_path
            with patch.object(p138, "_build_budget_module", return_value=FakeBudget):
                builds[key] = p138.check_build(build_path, mode, features)

        log_refs = {"p138": p138.ref(p138_path), "p138_stderr": p138.ref(stderr_path),
                    "register": p138.ref(register_path), "combined": p138.ref(combined_path),
                    "default": p138.ref(default_path)}
        report = {
            "schema": "wtime-p138-verification-v1", "status": "passed", "outcome": "contract_only",
            "decision": None, "instrumented": True, "timing_evidence": False,
            "gate_eligible": False, "candidate_promoted": False,
            "benchmark_samples": 0, "threshold_selection": None,
            "case_records": p138.validate_case_records(stderr_path),
            "trace_summary": p138.validate_traces(stderr_path),
            "fixture": fixture, "predecessor": predecessor,
            "freeze_report": p138.ref(frozen_path), "producer_helpers": helper_refs,
            "runtime_source_map_sha256": p138.canonical_sha(source_map),
            "runtime_source_file_count": 1,
            "test_logs": log_refs, "build_budgets": builds,
        }
        report_path = self.put("target/verification/proof/report.json", json.dumps(report))
        with patch.object(p138, "ROOT", synthetic_root), \
             patch.object(p138, "validate_predecessor", return_value=predecessor), \
             patch.object(p138, "_current_fixture_refs", return_value=report["fixture"]), \
             patch.object(p138, "current_runtime_source_map", return_value={"src/lib.rs": "d" * 64}), \
             patch.object(p138, "_build_budget_module", return_value=FakeBudget):
            self.assertEqual(p138.verify_evidence(report_path), report)
            report["timing_evidence"] = True
            report_path.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaisesRegex(p138.P138Error, "invalid or promoting"):
                p138.verify_evidence(report_path)
            report["timing_evidence"] = False
            report["fixture"] = {"fixture_sha256": "0" * 64}
            report_path.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaisesRegex(p138.P138Error, "freeze report does not bind"):
                p138.verify_evidence(report_path)
            report["fixture"] = fixture
            report["test_logs"]["p138"]["sha256"] = "0" * 64
            report_path.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaisesRegex(p138.P138Error, "test log reference is stale"):
                p138.verify_evidence(report_path)


if __name__ == "__main__":
    unittest.main()
