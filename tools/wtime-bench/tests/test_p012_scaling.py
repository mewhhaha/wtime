"""Mock-only fail-closed tests for the staged P012 runner."""
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import continuation_scaling as runner


class ContinuationRunnerTests(unittest.TestCase):
    def test_numeric_capture_resume_oracle_and_frame_accounting(self):
        entry = {"id": "continuation_depth_0", "oracle": {
            "expected_suspension": {"kind": "suspended", "payload": [{"type": "i32", "value": 7}]},
            "expected_resume": {"kind": "returned", "result": [{"type": "i32", "value": 1009}]}}}
        model = SimpleNamespace(depth_oracle=lambda _depth: {
            "captured_logical_frames": 1, "captured_numeric_locals": 1})
        row = {"schema_version": 2, "case": entry["id"], "driver": "depth",
               "mode": "decoded", "profile": "all-phase3-core", "instrumented": False,
               "payload": ["i32:7"], "result": ["i32:1009"],
               "logical_frames": 1, "numeric_locals": 1,
               "setup_ns": 1, "capture_ns": 2, "resume_ns": 3,
               "continuation_bytes": {"before_capture": 0, "after_capture": 64,
                   "after_root_transfer": 64, "after_resume": 32,
                   "after_drop_and_collect": 0},
               "collection_with_root": {"live": 1, "reclaimed": 0},
               "collection_after_drop": {"live": 0, "reclaimed": 1}}
        checked = runner._validate_capture_resume(entry, row, model)
        self.assertEqual(checked["bytes_per_captured_frame"], 64.0)
        row["result"] = ["i32:1010"]
        with self.assertRaisesRegex(runner.MeasurementError, "differs from oracle"):
            runner._validate_capture_resume(entry, row, model)

    def test_rooted_reference_requires_both_continuation_and_object(self):
        entry = {"id": "continuation_captured_reference", "oracle": {
            "expected_suspension": {"kind": "suspended", "payload": [{"type": "i32", "value": 5}]},
            "expected_resume": {"kind": "returned", "result": [{"type": "i32", "value": 86}]},
            "captured_reference": {"type": "struct", "field_i32": 77}}}
        model = SimpleNamespace()
        row = {"payload": ["i32:5"], "result": ["i32:86"],
               "collection_with_root": {"live": 2, "reclaimed": 0},
               "continuation_bytes": {"before_capture": 0, "after_capture": 80,
                   "after_root_transfer": 80, "after_resume": 32,
                   "after_drop_and_collect": 0},
               "collection_after_drop": {"live": 0},
               "setup_ns": 1, "capture_ns": 2, "resume_ns": 3}
        runner._validate_capture_resume(entry, row, model)
        row["collection_with_root"]["live"] = 1
        with self.assertRaisesRegex(runner.MeasurementError, r"exactly continuation\+struct"):
            runner._validate_capture_resume(entry, row, model)

    def test_boundary_probe_requires_exact_error_and_transactional_cleanup(self):
        fuel = {"schema_version": 2, "case": "continuation_depth_0", "driver": "probe_fuel",
                "mode": "decoded", "profile": "all-phase3-core", "instrumented": False,
                "error_kind": "ResourceLimit", "error_message": "fuel exhausted",
                "continuation_bytes": 0, "continuation_bytes_after_collect": 0, "gc_payload_bytes_after_collect": 0}
        runner._validate_probe(fuel, "fuel")
        budget = {**fuel, "driver": "probe_budget", "error_message": "continuation byte budget",
                  "gc_payload_bytes_before": 0, "gc_payload_bytes_after": 0}
        runner._validate_probe(budget, "budget")
        budget["gc_payload_bytes_after_collect"] = 16
        with self.assertRaisesRegex(runner.MeasurementError, "leaked state"):
            runner._validate_probe(budget, "budget")

    def test_registry_must_cover_exactly_seven_cases_and_keep_provider_setup_only(self):
        model_entries = []
        for workload in sorted(runner.MEASURED_IDS | {runner.PROVIDER_ID}):
            model_entries.append({"id": workload, "source": {"kind": "wat_file", "path": f"fixtures/{workload}.wat",
                                "sha256": "a" * 64, "wasm_sha256": "b" * 64},
                                 "required_features": ["mvp", "stack_switching"], "stages": ["setup"]})
        selected = sorted(runner.MEASURED_IDS)
        registry = {"schema_version": 2,
                    "selections": {"continuations": {"status": "ready", "workloads": selected}},
                    "workloads": [row for row in model_entries if row["id"] in runner.MEASURED_IDS]}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "workloads.json"
            path.write_text(json.dumps(registry))
            _, entries = runner._registry(path, "continuations", model_entries)
            self.assertEqual({entry["id"] for entry in entries}, runner.MEASURED_IDS)
            registry["selections"]["continuations"]["workloads"].append(runner.PROVIDER_ID)
            path.write_text(json.dumps(registry))
            with self.assertRaisesRegex(runner.MeasurementError, "exactly seven"):
                runner._registry(path, "continuations", model_entries)

    def test_source_hash_change_invalidates_the_measurement(self):
        before = {"/candidate": "a" * 64, "/runner.py": "b" * 64}
        runner._assert_stable(before, dict(before))
        changed = dict(before)
        changed["/candidate"] = "c" * 64
        with self.assertRaisesRegex(runner.MeasurementError, "identity changed"):
            runner._assert_stable(before, changed)


if __name__ == "__main__":
    unittest.main()
