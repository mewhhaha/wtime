"""Synthetic routing checks for focused repeated-GC-collection observations."""
from pathlib import Path
import hashlib
import json
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
import gc_collect_compare
import gc_collect_workloads
import selection_manifest


def collection_wire(entry, stage="warm_call", batch=1):
    repetitions = entry["params"][0]["baseline"]
    family = entry["family"]
    record = gc_collect_workloads.FAMILIES[family]
    nodes = record["count"]
    live = nodes + 1 if record["shape"] == "wide" else nodes
    calls = batch + 1 if stage == "warm_call" else 1
    prepare, instantiate, first = 11.0, 13.0, 17.0
    measured = 23.0 * batch if stage == "warm_call" else first
    sample = {"prepare": prepare, "instantiate": instantiate,
              "first_call": first, "warm_call": measured / batch,
              "load_and_call": prepare + instantiate + first}[stage]
    return {
        "schema_version": 2, "driver": "gc_collect_stages", "case": entry["id"],
        "family": family, "shape": record["shape"], "nodes": nodes,
        "live_objects": live, "input": repetitions, "mode": "decoded",
        "profile": "all-phase3-core", "stage": stage, "batch": batch,
        "max_batch": gc_collect_workloads.max_batch(nodes, repetitions),
        "instrumented": False,
        "measurement_policy": gc_collect_compare.MEASUREMENT_POLICY,
        "report_complete": True, "result": f"i32:{entry['expected_result']}",
        "sample_ns": sample, "prepare_ns": prepare,
        "instantiate_ns": instantiate, "first_call_ns": first,
        "measured_guest_ns": measured, "cleanup_ns": 7.0,
        "calls_checked": calls, "callbacks_checked": calls * repetitions,
        "callback_live": live, "callback_reclaimed": 0,
        "callback_payload_min": 88, "callback_payload_max": 88,
        "final_live": 0, "final_reclaimed": live,
        "final_payload_bytes": 0, "final_continuation_bytes": 0,
    }


class GcCollectRouteTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="p071-gc-collect-routes-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.assertTrue(self.root.is_dir())

    def entry(self, workload_id="gc_collect_wide_32_n17"):
        return next(row for row in gc_collect_workloads.entries()
                    if row["id"] == workload_id)

    def test_observation_routes_driver_and_revalidates_complete_wire(self):
        entry = self.entry()
        wasm = self.root / "synthetic.wasm"
        evidence = self.root / "evidence"
        self.assertTrue(wasm.resolve().is_relative_to(self.root))
        self.assertTrue(evidence.resolve().is_relative_to(self.root))
        wire = collection_wire(entry, "warm_call", 3)
        with mock.patch.object(focused_compare.compare, "process", return_value=wire) as process:
            observation = focused_compare.observe(
                "synthetic-executable", entry, wasm, "warm_call", 3,
                "decoded", 7, evidence)
        process.assert_called_once_with(
            "synthetic-executable",
            ["--gc-collect-driver", wasm, entry["id"], entry["family"],
             17, "decoded", "all-phase3-core", "warm_call", 3], 7)
        self.assertEqual(observation["driver_result"], wire)
        self.assertEqual(observation["result"], entry["expected_result"])
        self.assertEqual(observation["driver_result"]["callbacks_checked"], 68)
        self.assertEqual(gc_collect_compare.revalidate_observation(entry, observation), observation)

    def test_warm_calibration_uses_declared_cap_and_reaches_both_sides(self):
        entry = self.entry("gc_collect_wide_1024_n32")
        calls = []

        def observe(side, batch):
            calls.append((side, batch))
            return {"batch": batch, "ns_per_operation": 1.0}

        with self.assertRaisesRegex(ValueError, "calibration reached batch cap"):
            focused_compare.calibrate(entry, observe)
        self.assertEqual(focused_compare.batch_limit(entry), entry["max_batch"])
        self.assertEqual([batch for side, batch in calls if side == "baseline"],
                         [batch for side, batch in calls if side == "candidate"])
        self.assertTrue(all(batch <= entry["max_batch"] for _, batch in calls))
        self.assertIn(("baseline", entry["max_batch"]), calls)
        self.assertIn(("candidate", entry["max_batch"]), calls)

    def test_candidate_gate_rejects_tampered_gc_proof_and_sample(self):
        entry = self.entry("gc_collect_chain_256_n1")
        observation = gc_collect_compare.validate_observation(
            entry, collection_wire(entry, "warm_call", 2), 2,
            expected_stage="warm_call")
        candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                         stage="warm_call")
        changed_wire = dict(observation)
        changed_wire["driver_result"] = dict(observation["driver_result"],
                                             final_reclaimed=0)
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            candidate_gate.check_observation(entry, changed_wire, "decoded", self.root,
                                             stage="warm_call")
        changed_sample = dict(observation,
                              ns_per_operation=observation["ns_per_operation"] + 1)
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            candidate_gate.check_observation(entry, changed_sample, "decoded", self.root,
                                             stage="warm_call")

    def test_focused_identity_protects_collection_model_and_validator(self):
        prefix = "tools/wtime-bench/"
        sources = {prefix + name: "a" * 64
                   for name in execution_identity._FOCUSED_TIMING}
        for name in ("gc_collect_workloads.py", "gc_collect_compare.py"):
            changed = dict(sources, **{prefix + name: "b" * 64})
            with self.subTest(source=name), self.assertRaises(ValueError):
                execution_identity.verify(sources, changed, focused=True)

    def test_selection_binds_collection_family_to_actual_registry_membership(self):
        registry = json.loads((TOOL / "workloads.json").read_text())
        by_id = {row["id"]: row for row in registry["workloads"]}
        declarations = [
            ("gc-collections", "gc_collect_wide_32_n32", "targeted", "small", ["warm_call"]),
            ("gc-collections", "gc_collect_wide_1024_n32", "targeted", "large", ["warm_call"]),
            ("gc-collections", "gc_collect_chain_256_n17", "held_out", "other", ["warm_call"]),
            ("original-mvp", "call_overhead", "protected", "small", ["first_call", "load_and_call"]),
            ("original-mvp", "prepare_large", "protected", "large", ["prepare"]),
        ]
        rows = [{"family": family, "id": wid, "role": role, "scale": scale,
                 "stages": stages, "row_sha256": selection_manifest._canonical_sha(by_id[wid])}
                for family, wid, role, scale, stages in declarations]
        manifest = {"schema": selection_manifest.SCHEMA, "task": "P071", "lane": "candidate",
                    "rationale": "Validate exact collection-family membership.",
                    "registry_sha256": hashlib.sha256((TOOL / "workloads.json").read_bytes()).hexdigest(),
                    "rows": rows, "semantic_cases": ["p071_fixture_contract"]}
        self.assertEqual(selection_manifest.load_data(manifest), manifest)
        rows[0]["family"] = "gc-exceptions"
        with self.assertRaisesRegex(ValueError, "not in registry selection"):
            selection_manifest.load_data(manifest)


if __name__ == "__main__":
    unittest.main()
