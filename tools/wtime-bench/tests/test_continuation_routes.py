"""Subprocess-free routing tests for the staged P073 continuation adapter."""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools" / "wtime-bench"
sys.dont_write_bytecode = True
sys.path[:0] = [str(BENCH), str(Path(__file__).resolve().parent)]

import candidate_gate  # noqa: E402
import cold_products  # noqa: E402
import continuation_pairing as pairing  # noqa: E402
import continuation_workloads as model  # noqa: E402
import focused_compare  # noqa: E402

import test_continuation_pairing as _helpers

with (BENCH / "workloads.json").open(encoding="utf-8") as _f:
    _REGISTRY = json.load(_f)
_ROWS = {row["id"]: row for row in _REGISTRY["workloads"]}


class ContinuationRouteTests(unittest.TestCase):
    def test_depth4_and_bound_nested_registry_fixture_identities(self):
        for workload_id in ("continuation_depth_4", "continuation_bound_nested"):
            with self.subTest(workload=workload_id):
                entry = _ROWS[workload_id]
                self.assertTrue(pairing.matches(entry))
                self.assertEqual(pairing._modeled(entry), next(
                    row for row in model.workload_entries() if row["id"] == workload_id
                ))
                wat = BENCH / entry["source"]["path"]
                wasm = wat.with_suffix(".wasm")
                self.assertEqual(hashlib.sha256(wat.read_bytes()).hexdigest(),
                                 entry["source"]["sha256"])
                self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(),
                                 entry["source"]["wasm_sha256"])

    def test_cli_identity_mode_profile_and_batch_are_exact(self):
        depth = _ROWS["continuation_depth_4"]
        bound = _ROWS["continuation_bound_nested"]
        with tempfile.TemporaryDirectory(prefix="p073-route-cli-") as tmp:
            wasm = Path(tmp) / "fixture.wasm"
            wasm.write_bytes(b"synthetic path sentinel")
            self.assertEqual(
                pairing.argv(depth, wasm, pairing.MODE),
                ["--continuation-driver", str(wasm), depth["id"], pairing.MODE,
                 pairing.PROFILE, "depth"],
            )
            self.assertEqual(pairing.argv(bound, wasm, pairing.MODE)[-1], "bind")
        observation = pairing.validate_observation(
            depth, _helpers._depth_wire(depth["id"]), 1,
            expected_mode=pairing.MODE, expected_stage="capture",
        )
        self.assertEqual((observation["mode"], observation["stage"], observation["batch"]),
                         (pairing.MODE, "capture", 1))
        with self.assertRaises(pairing.PairingError):
            pairing.argv(depth, "/tmp/fixture.wasm", "compact")
        with self.assertRaises(pairing.PairingError):
            pairing.validate_observation(
                depth, _helpers._depth_wire(depth["id"]), 2,
                expected_mode=pairing.MODE, expected_stage="capture",
            )

    def test_independent_oracles_keep_suspension_and_binding_lifecycles_typed(self):
        depth = _ROWS["continuation_depth_4"]
        depth_expected = candidate_gate.expected(depth)
        oracle = model.depth_oracle(4)
        self.assertEqual(depth_expected, {
            "kind": "suspend_then_resume",
            "suspension": {"kind": "suspended", "payload": ["i32:7"]},
            "resume": {"kind": "returned", "result": [
                f"i32:{oracle['expected_resume']['result'][0]['value']}"
            ]},
        })
        self.assertEqual(depth["result"], {"type": "none"})

        bound = _ROWS["continuation_bound_nested"]
        bound_expected = candidate_gate.expected(bound)
        bound_oracle = next(row for row in model.workload_entries()
                            if row["id"] == bound["id"])["oracle"]
        self.assertEqual(bound_expected["kind"], "bind_then_resume")
        self.assertEqual(bound_expected["bind"], {"kind": "returned", "result": ["i32:0"]})
        self.assertEqual(bound_expected["resume"]["result"], [
            f"{v['type']}:{v['value']}" for v in bound_oracle["expected_resume"]["result"]
        ])
        self.assertEqual(bound_expected["worker_calls"], bound_oracle["expected_worker_calls"])

    def test_gate_revalidates_the_entire_normalized_observation(self):
        entry = _ROWS["continuation_depth_4"]
        wire = _helpers._depth_wire(entry["id"])
        observation = pairing.validate_observation(
            entry, wire, 1, expected_mode=pairing.MODE, expected_stage="capture",
        )
        with tempfile.TemporaryDirectory(prefix="p073-route-gate-") as tmp:
            evidence_root = Path(tmp).resolve()
            candidate_gate.check_observation(
                entry, observation, pairing.MODE, evidence_root, stage="capture",
            )
            mutations = (
                ("result", lambda value: value["result"]["resume"].update(result=["i32:999"])),
                ("expected", lambda value: value["expected"]["suspension"].update(payload=["i32:999"])),
                ("time", lambda value: value.update(ns_per_operation=999.0)),
                ("checkpoint", lambda value: value["gc_payload_bytes"].update(after_capture=1)),
                ("retained wire", lambda value: value["raw_wire_observation"].update(
                    measurement_boundary="includes collection")),
            )
            for label, mutate in mutations:
                with self.subTest(field=label):
                    changed = copy.deepcopy(observation)
                    mutate(changed)
                    with self.assertRaises(ValueError):
                        candidate_gate.check_observation(
                            entry, changed, pairing.MODE, evidence_root, stage="capture",
                        )

    def test_product_identity_binds_all_sources_and_is_stage_specific(self):
        entry = _ROWS["continuation_depth_4"]
        stages = ["setup", "capture", "resume"]
        declaration = {"rows": [{"id": entry["id"], "role": "target", "stages": stages}]}
        settings = {
            "instrumented": False, "profile": "release", "fuel_policy": False,
            "cancellation_policy": 1, "baseline_mode": "decoded",
            "candidate_mode": "decoded", "thread_cpu_affinity": None,
            "workload_profiles": {entry["id"]: focused_compare.profile(entry)},
        }
        products = focused_compare.products([entry], declaration, settings)
        self.assertEqual([product["stage"] for product in products], stages)
        plan = cold_products.plan(products)
        self.assertEqual(plan["logical_groups"], 3)
        by_stage = {group["stage"]: group for group in plan["groups"]}
        self.assertEqual(set(by_stage), set(stages))
        identities = set()
        for stage in stages:
            product = by_stage[stage]
            identities.add(product["product_sha256"])
            timer = product["product"]["timer"]
            self.assertEqual(timer["model_sha256"], focused_compare.compare.sha(
                BENCH / "continuation_workloads.py"))
            self.assertEqual(timer["validator_sha256"], focused_compare.compare.sha(
                BENCH / "continuation_pairing.py"))
            self.assertEqual(timer["lifecycle_validator_sha256"], focused_compare.compare.sha(
                BENCH / "continuation_scaling.py"))
            self.assertEqual(timer["driver_sha256"], focused_compare.compare.sha(
                BENCH / "src/continuation_driver.rs"))
            self.assertEqual(timer["fixture_wat_sha256"], entry["source"]["sha256"])
            self.assertEqual(timer["fixture_wasm_sha256"], entry["source"]["wasm_sha256"])
            self.assertEqual(product["product"]["runtime_settings"]["baseline_mode"], "decoded")
        self.assertEqual(len(identities), len(stages))


if __name__ == "__main__":
    unittest.main()
