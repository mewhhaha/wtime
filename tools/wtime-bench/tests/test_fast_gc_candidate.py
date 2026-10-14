"""Synthetic checks for the focused legacy GC candidate lane."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import candidate_gate
import focused_compare


class FocusedGcCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-fast-gc-candidate-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    def test_legacy_gc_rows_keep_typed_profile_and_full_cold_timer_identity(self):
        entry = {
            "id": "gc_host_reentry_collect_n1",
            "oracle": {"module": "gc_workloads.py"},
            "driver": {"kind": "gc_scalar_stages"},
            "source": {"wasm_sha256": "a" * 64},
        }
        self.assertEqual(focused_compare.profile(entry), "all-phase3-core")
        with mock.patch.object(focused_compare.compare, "sha", return_value="b" * 64):
            product = focused_compare.products([entry], {"rows": [{"id": entry["id"], "stages": ["load_and_call"]}]}, {})[0]
        self.assertEqual(product["stage"], "load_and_call")
        self.assertEqual(product["timer"]["measurement_policy"],
                         "load_and_call; full typed GC/exception state oracle")
        self.assertIn("adapter_sha256", product["timer"])
        self.assertIn("gc_validator_sha256", product["timer"])

    def test_observer_routes_gc_through_existing_cold_adapter(self):
        entry = {"id": "gc_host_reentry_collect_n1", "oracle": {"module": "gc_workloads.py"},
                 "params": [{"baseline": 1}]}
        fixture = {"path": Path("unused.wasm"), "setup_modules": []}
        sample = {"workload": entry["id"], "stage": "load_and_call", "input": 1,
                  "expected": 1, "result": 1, "outcome": "returned", "tag": None,
                  "payload": None, "ns_per_operation": 10.0, "batch": 1,
                  "mode": "decoded", "profile": "all-phase3-core", "wire_observation": {}}
        with mock.patch("gc_cold_adapter.cold_observation", return_value=sample) as cold, \
             mock.patch("gc_compare._normalize_cold", return_value={"result": 1}) as normalize:
            result = focused_compare.observe("unused", entry, fixture, "load_and_call", 1,
                                             "decoded", 0, self.root)
        cold.assert_called_once()
        normalize.assert_called_once()
        self.assertEqual(result, {"result": 1})

    def test_normalized_gc_result_revalidates_raw_wire_and_cold_intervals(self):
        entry = {'id':'gc_host_reentry_collect_n1', 'oracle':{'module':'gc_workloads.py'},
                 'params':[{'name':'n','type':'i32','baseline':1,'min':1,'max':1}],
                 'source':{'sha256':'a'*64,'wasm_sha256':'b'*64}}
        proof = {'driver_reported_times_ns':{'setup_ns':10.0,'invoke_ns':20.0}}
        sample = {'workload':entry['id'],'expected':1,'result':1,'outcome':'returned',
                  'tag':None,'payload':None,'stage':'load_and_call','batch':1,
                  'mode':'decoded','profile':'all-phase3-core','ns_per_operation':30.0,
                  'wire_observation':{'result':'i32:1','outcome':'returned'},
                  'validated_driver_proof':proof,
                  'cold_components_ns':{'setup_ns':10.0,'invoke_ns':20.0}}
        with mock.patch('gc_scaling._validate_row',return_value=proof):
            candidate_gate.check_observation(entry,sample,'decoded',self.root)
            with self.assertRaisesRegex(ValueError,'validated driver intervals'):
                candidate_gate.check_observation(entry,dict(sample,ns_per_operation=31.0),'decoded',self.root)
            with self.assertRaisesRegex(ValueError,'state oracle proof differs'):
                candidate_gate.check_observation(entry,dict(sample,validated_driver_proof={}), 'decoded',self.root)

    def test_cold_gc_target_can_drive_candidate_decision(self):
        rows = [
            {"id": "gc_small", "family": "gc-exceptions", "role": "targeted", "stages": ["load_and_call"]},
            {"id": "gc_large", "family": "gc-exceptions", "role": "targeted", "stages": ["load_and_call"]},
        ]
        report = {"declaration": {"rows": rows}, "runs": [],
                  "cold_products": {"groups": []}, "counts": {}}
        for run_id, seed in enumerate((3, 7)):
            pairs = []
            for wid in ("gc_small", "gc_large"):
                for sample in range(30):
                    pairs.append({"workload": wid, "stage": "load_and_call",
                                  "baseline": {"ns_per_operation": 100.0},
                                  "candidate": {"ns_per_operation": 90.0}})
            report["runs"].append({"run_id": run_id, "seed": seed, "pairs": pairs})
        summary = {"per_run": []}
        with mock.patch("candidate_gate.analyze.analyze", return_value=summary), \
             mock.patch("candidate_gate.draw_median", side_effect=lambda values, _rng: sorted(values)[len(values)//2]):
            result = candidate_gate.decision(report)
        self.assertEqual(result["decision"], "adopted")
        self.assertEqual(result["primary"][0]["stages"],
                         {"gc_large": "load_and_call", "gc_small": "load_and_call"})


if __name__ == "__main__":
    unittest.main()
