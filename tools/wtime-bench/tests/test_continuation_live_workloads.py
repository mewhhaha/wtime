"""Meaningful validator and generic registry compatibility checks for P072."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(REPO / "tools/wtime-bench"))

import workloads

spec = importlib.util.spec_from_file_location("continuation_live_workloads", HERE.parent / "continuation_live_workloads.py")
model = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(model)


def sample(entry, stage="capture"):
    n = entry["params"][0]["baseline"]
    expected = model.oracle(entry["id"], n)
    return {"schema": model.SCHEMA, "schema_version": 1, "case": entry["id"],
            "stage": stage, "live_count": n, "mode": model.MODE,
            "profile": model.PROFILE, "batch": 1, "instrumented": False,
            "report_complete": True, "setup_ns": 100, "capture_ns": 20,
            "payloads": expected["payloads"], "results": expected["results"],
            "roots_before": expected["roots_before"], "roots_peak": expected["roots_peak"],
            "gc_live_before": expected["gc_live_before"], "gc_live_peak": expected["gc_live_peak"],
            "gc_live_after_drop": 0, "bytes_before": 3,
            "bytes_after_capture": 17, "bytes_after_drop": 0}


class LiveWorkloadTests(unittest.TestCase):
    def setUp(self):
        self.entry = model.entries()[0]

    def test_valid_observation_returns_selected_raw_interval(self):
        row = sample(self.entry)
        result = model.validate_observation(self.entry, row, "capture")
        self.assertEqual(result["ns_per_operation"], 20)
        self.assertEqual(result["batch"], 1)
        self.assertEqual(result["raw_wire_observation"], row)

    def test_wrong_count_and_wrong_oracle_are_rejected(self):
        row = sample(self.entry)
        row["results"].pop()
        with self.assertRaises(model.LiveWorkloadError):
            model.validate_observation(self.entry, row, "capture")
        row = sample(self.entry)
        row["payloads"][0] = "i32:8"
        with self.assertRaises(model.LiveWorkloadError):
            model.validate_observation(self.entry, row, "capture")

    def test_cleanup_and_type_contracts_are_rejected(self):
        row = sample(self.entry)
        row["gc_live_after_drop"] = 1
        with self.assertRaises(model.LiveWorkloadError):
            model.validate_observation(self.entry, row, "capture")
        row = sample(self.entry)
        row["capture_ns"] = True
        with self.assertRaises(model.LiveWorkloadError):
            model.validate_observation(self.entry, row, "capture")

    def test_wire_identity_timing_and_byte_fields_are_rejected(self):
        for key, value in (("schema", "other"), ("batch", 2),
                           ("instrumented", True), ("report_complete", False),
                           ("bytes_after_capture", 3), ("bytes_after_drop", 1)):
            row = sample(self.entry)
            row[key] = value
            with self.subTest(key=key), self.assertRaises(model.LiveWorkloadError):
                model.validate_observation(self.entry, row, "capture")

    def test_registry_rows_validate_inside_fresh_contained_copy(self):
        with tempfile.TemporaryDirectory(prefix="p072-registry-") as directory:
            root = Path(directory).resolve()
            registry_path = (root / "workloads.json").resolve()
            self.assertTrue(registry_path.is_relative_to(root))
            registry = json.loads((REPO / "tools/wtime-bench/workloads.json").read_text())
            rows = model.entries()
            row_ids = {entry["id"] for entry in rows}
            registry["workloads"] = [entry for entry in registry["workloads"]
                                      if entry["id"] not in row_ids]
            registry["workloads"].extend(rows)
            registry["selections"]["p072-live-draft"] = {
                "status": "ready", "workloads": [entry["id"] for entry in rows]}
            registry_path.write_text(json.dumps(registry))
            validated = workloads.load_registry(registry_path)
            self.assertEqual([row["id"] for row in workloads.selection("p072-live-draft", validated)],
                             [row["id"] for row in rows])
            self.assertTrue(all(Path(row["source"]["path"]).is_relative_to(Path("fixtures"))
                                for row in rows))


if __name__ == "__main__":
    unittest.main()
