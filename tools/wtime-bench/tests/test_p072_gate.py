"""Focused P072 lifecycle routing and execution-identity checks."""
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import candidate_gate
import continuation_live_workloads as model
import execution_identity
import analyze
import selection_manifest
import workloads


class P072GateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="p072-gate-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.assertTrue(self.root.is_dir())

    def entry(self):
        return model.entries()[0]

    def wire(self, entry, stage="capture"):
        n = entry["params"][0]["baseline"]
        expected = model.oracle(entry["id"], n)
        return {
            "schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
            "case": entry["id"], "stage": stage, "live_count": n,
            "mode": model.MODE, "profile": model.PROFILE, "batch": 1,
            "instrumented": False, "report_complete": True,
            "setup_ns": 17, "capture_ns": 23,
            "payloads": expected["payloads"], "results": expected["results"],
            "roots_before": expected["roots_before"],
            "roots_peak": expected["roots_peak"],
            "gc_live_before": expected["gc_live_before"],
            "gc_live_peak": expected["gc_live_peak"],
            "gc_live_after_drop": expected["gc_live_after_drop"],
            "bytes_before": 100, "bytes_after_capture": 150,
            "bytes_after_drop": 0,
        }

    def observation(self, entry, stage="capture"):
        return model.validate_observation(entry, self.wire(entry, stage), stage)

    def test_live_observation_uses_full_wire_oracle_and_exact_capture_timer(self):
        entry = self.entry()
        observation = self.observation(entry)
        self.assertEqual(observation["ns_per_operation"], 23)
        self.assertEqual(observation["batch"], 1)
        candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                         stage="capture")

    def test_live_observation_rejects_tampered_lifecycle_and_timer(self):
        entry = self.entry()
        observation = self.observation(entry)
        altered = dict(observation)
        altered["raw_wire_observation"] = dict(observation["raw_wire_observation"],
                                               gc_live_after_drop=1)
        with self.assertRaises(ValueError):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="capture")
        altered = dict(observation, ns_per_operation=24)
        with self.assertRaisesRegex(ValueError, "normalized timer/oracle"):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="capture")

    def test_live_observation_requires_raw_wire_and_selected_stage(self):
        entry = self.entry()
        observation = self.observation(entry)
        without_raw = dict(observation)
        del without_raw["raw_wire_observation"]
        with self.assertRaisesRegex(ValueError, "raw wire"):
            candidate_gate.check_observation(entry, without_raw, "decoded", self.root,
                                             stage="capture")
        with self.assertRaisesRegex(ValueError, "mode/stage"):
            candidate_gate.check_observation(entry, observation, "jit", self.root,
                                             stage="capture")
        with self.assertRaisesRegex(ValueError, "case/stage"):
            candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                             stage="setup")

    def test_live_workload_model_is_protected_execution_identity(self):
        path = "tools/wtime-bench/continuation_live_workloads.py"
        self.assertEqual(execution_identity._classify(path, focused=True), "protected")
        required = {"tools/wtime-bench/" + name: "a" * 64
                    for name in execution_identity._FOCUSED_TIMING}
        before = dict(required, **{path: "a" * 64})
        after = dict(before, **{path: "b" * 64})
        with self.assertRaisesRegex(ValueError, "execution source identity is stale"):
            execution_identity.verify(before, after, focused=True)

    def test_capture_is_the_primary_stage_for_live_continuations(self):
        entry = self.entry()
        declaration_row = {"id": entry["id"], "role": "targeted",
                           "family": "continuations", "stages": ["setup", "capture"]}
        pairs = [{"workload": entry["id"], "stage": "capture",
                  "baseline": {"ns_per_operation": 10.0},
                  "candidate": {"ns_per_operation": 9.0}}
                 for _ in range(30)]
        report = {"declaration": {"rows": [declaration_row]},
                  "runs": [{"seed": 1, "pairs": pairs}, {"seed": 2, "pairs": pairs}],
                  "cold_products": [], "counts": {}}
        with mock.patch.object(candidate_gate.analyze, "analyze", return_value={"per_run": []}):
            result = candidate_gate.decision(report)
        self.assertEqual(len(result["primary"]), 2)
        self.assertTrue(all(row["stages"] == {entry["id"]: "capture"}
                            for row in result["primary"]))

    def live_report(self):
        selected = workloads.selection("p072-live")
        runs = []
        for run_index, seed in enumerate((72001, 72002)):
            pairs = []
            pair_index = 0
            for entry in selected:
                expected = model.oracle(entry["id"], entry["params"][0]["baseline"])
                for stage in entry["stages"]:
                    for _ in range(30):
                        pair_index += 1
                        wire = self.wire(entry, stage)
                        wire["setup_ns"] += run_index
                        wire["capture_ns"] += run_index
                        observation = model.validate_observation(entry, wire, stage)
                        pairs.append({
                            "pair_id": f"{run_index}-{pair_index}",
                            "workload": entry["id"], "stage": stage,
                            "input": entry["params"][0]["baseline"],
                            "fixture": entry["source"]["wasm_sha256"],
                            "expected": expected,
                            "order": ["baseline", "candidate"],
                            "baseline": observation,
                            "candidate": observation,
                        })
            runs.append({"seed": seed, "pairs": pairs})
        return {"schema_version": 2, "passed": True, "source_stable": True,
                "selection": "p072-live",
                "metadata": {"settings": {"instrumented": False,
                                             "baseline_mode": "decoded",
                                             "candidate_mode": "decoded"}},
                "runs": runs}

    def focused_live_report(self):
        report = self.live_report()
        manifest_path = TOOL.parents[1] / "docs/benchmarks/performance/P072-selection.json"
        declaration = selection_manifest.load(manifest_path)
        selected = selection_manifest.rows(declaration)
        for run_index, run in enumerate(report["runs"]):
            for entry in selected:
                if entry["oracle"].get("module") == "continuation_live_workloads.py":
                    continue
                input_value = entry["params"][0]["baseline"] if entry["params"] else 0
                for stage in next(row["stages"] for row in declaration["rows"]
                                  if row["id"] == entry["id"]):
                    for sample in range(30):
                        pair_id = f"control-{run_index}-{entry['id']}-{stage}-{sample}"
                        result = 0
                        observation = {"batch": 1, "ns_per_operation": 10.0,
                                       "result": result, "mode": "decoded"}
                        run["pairs"].append({
                            "pair_id": pair_id, "workload": entry["id"],
                            "stage": stage, "input": input_value,
                            "fixture": entry["source"]["wasm_sha256"],
                            "expected": result, "order": ["baseline", "candidate"],
                            "baseline": observation, "candidate": observation,
                        })
        report["schema"] = "wtime-focused-comparison-v1"
        report["selection"] = "P072"
        report["declaration"] = declaration
        return report

    def test_analyzer_revalidates_all_live_lifecycle_pairs_from_registry(self):
        report = self.focused_live_report()
        _metadata, runs = analyze._validated_runs(report)
        self.assertEqual(len(runs), 2)
        self.assertEqual(len(runs[0]["groups"]), 11)
        self.assertEqual(report["selection"], "P072")

    def test_analyzer_keeps_legacy_live_registry_route(self):
        _metadata, runs = analyze._validated_runs(self.live_report())
        self.assertEqual(len(runs[0]["groups"]), 8)

    def test_analyzer_rejects_tampered_live_wire_and_full_expected_oracle(self):
        report = self.focused_live_report()
        bad_wire = json.loads(json.dumps(report))
        bad_wire["runs"][0]["pairs"][0]["baseline"]["raw_wire_observation"]["gc_live_after_drop"] = 1
        with self.assertRaisesRegex(analyze.ReportError, "live-continuation wire is invalid"):
            analyze._validated_runs(bad_wire)

        bad_expected = json.loads(json.dumps(report))
        bad_expected["runs"][0]["pairs"][0]["expected"]["roots_before"] += 1
        with self.assertRaisesRegex(analyze.ReportError, "expected lifecycle"):
            analyze._validated_runs(bad_expected)


if __name__ == "__main__":
    unittest.main()
