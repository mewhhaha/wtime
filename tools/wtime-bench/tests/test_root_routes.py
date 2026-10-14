"""Synthetic routing checks for the focused P125 root workload lane."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import candidate_gate
import execution_identity
import focused_compare
import root_compare
import root_workloads


def root_wire(entry, stage="warm_call", batch=1):
    n = entry["params"][0]["baseline"]
    callbacks = (batch + 1 if stage == "warm_call" else 1) * n
    prep, instantiate, first = 11.0, 13.0, 17.0
    measured = 23.0 * batch if stage == "warm_call" else first
    sample = {"prepare": prep, "instantiate": instantiate, "first_call": first,
              "warm_call": measured / batch, "load_and_call": prep + instantiate + first}[stage]
    payload = 88 if callbacks else 0
    return {
        "schema_version": 2, "driver": "gc_root_stages", "case": entry["id"],
        "family": entry["family"], "input": n, "mode": "decoded",
        "profile": "all-phase3-core", "stage": stage, "batch": batch,
        "max_batch": entry["max_batch"], "instrumented": False,
        "measurement_policy": root_compare.MEASUREMENT_POLICY,
        "report_complete": True, "shape": entry["gc_expectation"]["shape"],
        "bank_slots": entry["gc_expectation"]["bank_slots"],
        "result": f"i32:{entry['expected_result']}", "sample_ns": sample,
        "prepare_ns": prep, "instantiate_ns": instantiate, "first_call_ns": first,
        "measured_guest_ns": measured, "reset_ns": 7.0,
        "calls_checked": batch + 1 if stage == "warm_call" else 1,
        "total_reclaimed": batch + 1 if stage == "warm_call" else 1,
        "reclaimed_per_call": 1, "live_after_reset": 0,
        "payload_bytes_after_reset": 0, "continuation_bytes_after_reset": 0,
        "callbacks_checked": callbacks, "nested_calls_checked": callbacks,
        "callback_live": 1 if callbacks else 0, "callback_reclaimed": 0,
        "callback_payload_min": payload, "callback_payload_max": payload,
    }


class RootRouteTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="p125-root-routes-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.assertTrue(self.root.is_dir())

    def test_observe_routes_root_cli_and_validates_complete_proof(self):
        entry = next(row for row in root_workloads.entries()
                     if row["id"] == "gc_root_snapshots_reference_64_n17")
        wasm = self.root / "synthetic.wasm"
        output = self.root / "evidence"
        self.assertTrue(wasm.resolve().is_relative_to(self.root))
        self.assertTrue(output.resolve().is_relative_to(self.root))
        wire = root_wire(entry, "warm_call", 3)
        with mock.patch.object(focused_compare.compare, "process", return_value=wire) as process:
            observation = focused_compare.observe(
                "synthetic-executable", entry, wasm, "warm_call", 3, "decoded", 7, output)
        process.assert_called_once_with(
            "synthetic-executable",
            ["--gc-root-driver", wasm, entry["id"], entry["family"], 17,
             "decoded", "all-phase3-core", "warm_call", 3], 7)
        self.assertEqual(observation["driver_result"], wire)
        self.assertEqual(observation["result"], entry["expected_result"])
        self.assertEqual(observation["driver_result"]["callbacks_checked"], 68)
        self.assertEqual(root_compare.revalidate_observation(entry, observation), observation)

    def test_root_calibration_shares_batches_and_stops_at_model_cap(self):
        entry = next(row for row in root_workloads.entries()
                     if row["id"] == "gc_root_snapshots_numeric_512_n32")
        cap = root_workloads.max_batch(512, 32)
        self.assertEqual(entry["max_batch"], cap)
        calls = []

        def observe(side, batch):
            calls.append((side, batch))
            return {"batch": batch, "ns_per_operation": 1.0}

        with self.assertRaisesRegex(ValueError, "workload limit"):
            focused_compare.calibrate(entry, observe)
        self.assertGreater(cap, 1)
        self.assertEqual([batch for side, batch in calls if side == "baseline"],
                         [batch for side, batch in calls if side == "candidate"])
        self.assertTrue(all(batch <= cap for _side, batch in calls))
        self.assertIn(("baseline", cap), calls)
        self.assertIn(("candidate", cap), calls)

    def test_candidate_gate_rejects_tampered_copied_wire_and_normalized_sample(self):
        entry = next(row for row in root_workloads.entries()
                     if row["id"] == "gc_root_snapshots_numeric_64_n1")
        observation = root_compare.validate_observation(
            entry, root_wire(entry, "warm_call", 2), 2, expected_stage="warm_call")
        candidate_gate.check_observation(entry, observation, "decoded", self.root)

        tampered_wire = dict(observation)
        tampered_wire["driver_result"] = dict(observation["driver_result"], callbacks_checked=0)
        with self.assertRaises(root_compare.RootComparisonError):
            candidate_gate.check_observation(entry, tampered_wire, "decoded", self.root)

        tampered_sample = dict(observation, ns_per_operation=observation["ns_per_operation"] + 1)
        with self.assertRaises(root_compare.RootComparisonError):
            candidate_gate.check_observation(entry, tampered_sample, "decoded", self.root)

    def test_focused_identity_protects_root_timing_sources(self):
        prefix = "tools/wtime-bench/"
        sources = {prefix + name: "a" * 64 for name in execution_identity._FOCUSED_TIMING}
        for name in ("root_workloads.py", "root_compare.py"):
            changed = dict(sources, **{prefix + name: "b" * 64})
            with self.subTest(source=name), self.assertRaises(ValueError):
                execution_identity.verify(sources, changed, focused=True)

    def test_analysis_only_gate_source_is_excluded_from_focused_identity(self):
        prefix = "tools/wtime-bench/"
        sources = {prefix + name: "a" * 64 for name in execution_identity._FOCUSED_TIMING}
        sources[prefix + "candidate_gate.py"] = "b" * 64
        sources[prefix + "analyze.py"] = "c" * 64
        changed = dict(sources)
        changed[prefix + "candidate_gate.py"] = "d" * 64
        changed[prefix + "analyze.py"] = "e" * 64
        binding = execution_identity.verify(sources, changed, focused=True)
        self.assertTrue(binding["excluded_analysis_inputs"][prefix + "analyze.py"]["changed"])
        self.assertTrue(binding["excluded_analysis_inputs"][prefix + "candidate_gate.py"]["changed"])


if __name__ == "__main__":
    unittest.main()
