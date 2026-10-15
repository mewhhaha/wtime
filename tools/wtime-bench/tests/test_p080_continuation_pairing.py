"""Synthetic, subprocess-free contract checks for the P073 pairing adapter."""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools" / "wtime-bench"
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import continuation_pairing as pairing  # noqa: E402
import continuation_workloads  # noqa: E402


with (BENCH / "workloads.json").open(encoding="utf-8") as _registry_file:
    _REGISTRY = json.load(_registry_file)
_ROWS = {row["id"]: row for row in _REGISTRY["workloads"]}


def _depth_wire(workload="continuation_depth_0"):
    entry = _ROWS[workload]
    oracle = entry["oracle"]
    frames = oracle["captured_logical_frames"] if workload.startswith("continuation_depth_") else 1
    cont = {"before_capture": 0, "after_capture": 64, "after_root_transfer": 64,
            "after_resume": 0, "after_drop_and_collect": 0}
    gc = {"before_capture": 0, "after_capture": 96, "after_root_transfer": 96,
          "after_resume": 24, "after_drop_and_collect": 0}
    live = 2 if workload == "continuation_captured_reference" else 1
    return {
        "schema_version": 2, "case": workload,
        "driver": "depth" if workload.startswith("continuation_depth_") else "root",
        "mode": pairing.MODE, "profile": pairing.PROFILE, "instrumented": False,
        "measurement_boundary": pairing.BOUNDARY, "setup_ns": 10.0,
        "capture_ns": 20.0, "resume_ns": 30.0,
        "payload": [f"{v['type']}:{v['value']}" for v in oracle["expected_suspension"]["payload"]],
        "result": [f"{v['type']}:{v['value']}" for v in oracle["expected_resume"]["result"]],
        "logical_frames": frames, "bytes_per_captured_frame": 64 / frames,
        "numeric_locals": frames, "continuation_bytes": cont, "gc_payload_bytes": gc,
        "collection_with_root": {"live": live, "reclaimed": 0},
        "collection_after_drop": {"live": 0, "reclaimed": live},
    }


def _bind_wire():
    return {
        "schema_version": 2, "case": "continuation_bound_nested", "driver": "bind",
        "mode": pairing.MODE, "profile": pairing.PROFILE, "instrumented": False,
        "measurement_boundary": pairing.BOUNDARY, "setup_ns": 10.0, "bind_ns": 20.0,
        "resume_ns": 30.0, "bind_result": "i32:0", "result": ["i32:0", "i32:1122"],
        "worker_calls": 1,
        "continuation_bytes": {"before": 0, "after_bind": 48, "after_run": 0, "after_collect": 0},
        "gc_payload_bytes": {"before_bind": 0, "after_bind": 80, "after_run": 16, "after_collect": 0},
        "collection": {"live": 0, "reclaimed": 1},
    }


def _one_shot_wire():
    return {
        "schema_version": 2, "case": "continuation_one_shot", "driver": "oneshot",
        "mode": pairing.MODE, "profile": pairing.PROFILE, "instrumented": False,
        "measurement_boundary": pairing.BOUNDARY, "setup_ns": 10.0, "capture_ns": 20.0,
        "resume_once_ns": 30.0, "resume_reuse_ns": 40.0, "first_result": "i32:1",
        "result": "trap", "error_kind": "Trap", "error_message": "continuation already consumed",
        "continuation_bytes": {"before": 0, "after_capture": 48, "after_trap": 0, "after_collect": 0},
        "gc_payload_bytes": {"before_capture": 0, "after_capture": 72, "after_trap": 24,
                             "after_collect": 0},
        "collection": {"live": 0, "reclaimed": 1},
    }


def _cross_module_wire():
    return {
        "schema_version": 2, "case": "continuation_cross_module_consumer",
        "driver": "cross_module", "exact_host_result_rejected": True,
        "exact_host_error_kind": "Host", "mode": pairing.MODE, "profile": pairing.PROFILE,
        "instrumented": False, "measurement_boundary": pairing.BOUNDARY,
        "setup_ns": 10.0, "capture_ns": 20.0, "resume_ns": 30.0,
        "payload": [], "result": ["i32:77"],
        "continuation_bytes": {"before_capture": 0, "after_capture": 48,
                                "after_root_transfer": 48, "after_resume": 0,
                                "after_drop_and_collect": 0},
        "gc_payload_bytes": {"before_capture": 0, "after_capture": 72,
                              "after_root_transfer": 72, "after_resume": 24,
                              "after_drop_and_collect": 0},
        "collection_with_root": {"live": 2, "reclaimed": 0},
        "collection_after_drop": {"live": 0, "reclaimed": 2},
    }


class ContinuationPairingTests(unittest.TestCase):
    def test_real_registry_rows_match_expected_lifecycle_and_cli(self):
        for workload in pairing._SUPPORTED:
            with self.subTest(workload=workload):
                entry = _ROWS[workload]
                self.assertTrue(pairing.matches(entry))
                outcome = pairing.expected(entry)
                self.assertIn(outcome["kind"], ("suspend_then_resume", "bind_then_resume", "one_shot_then_trap"))
                self.assertEqual(entry["result"], {"type": "none"})
                args = pairing.argv(entry, "/contained/fixture.wasm", pairing.MODE,
                    "/contained/provider.wasm" if workload == "continuation_cross_module_consumer" else None)
                if workload == "continuation_one_shot":
                    self.assertEqual(args[-1], "oneshot")
                elif workload == "continuation_cross_module_consumer":
                    self.assertEqual(args[0], "--continuation-cross-module")
                else:
                    self.assertEqual(args[-1], "bind" if workload.endswith("bound_nested") else "depth" if "depth_" in workload else "root")

    def test_capture_resume_observation_round_trips_full_typed_lifecycle(self):
        entry = _ROWS["continuation_depth_4"]
        wire = _depth_wire(entry["id"])
        observation = pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                                   expected_stage="capture")
        self.assertEqual(observation["result"]["kind"], "suspend_then_resume")
        self.assertEqual(observation["ns_per_operation"], 20.0)
        self.assertEqual(pairing.revalidate_observation(entry, observation), observation)

    def test_captured_reference_collection_oracle_keeps_both_objects(self):
        entry = _ROWS["continuation_captured_reference"]
        observation = pairing.validate_observation(entry, _depth_wire(entry["id"]), 1,
                                                   expected_mode=pairing.MODE, expected_stage="resume")
        self.assertEqual(observation["collection_with_root"]["live"], 2)
        self.assertEqual(observation["result"]["captured_reference"]["field_i32"], 77)

    def test_bound_chain_allows_consumption_before_cleanup(self):
        entry = _ROWS["continuation_bound_nested"]
        observation = pairing.validate_observation(entry, _bind_wire(), 1,
                                                   expected_mode=pairing.MODE, expected_stage="bind")
        self.assertEqual(observation["continuation_bytes"]["after_run"], 0)
        self.assertEqual(observation["gc_payload_bytes"]["after_run"], 16)
        self.assertEqual(observation["result"]["kind"], "bind_then_resume")

    def test_rejects_wrong_stage_batch_boundary_and_tampered_retained_wire(self):
        entry = _ROWS["continuation_depth_0"]
        wire = _depth_wire()
        with self.assertRaises(pairing.PairingError):
            pairing.validate_observation(entry, wire, 2, expected_mode=pairing.MODE, expected_stage="capture")
        with self.assertRaises(pairing.PairingError):
            pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE, expected_stage="accounting")
        changed = copy.deepcopy(wire)
        changed["measurement_boundary"] = "timer includes collection"
        with self.assertRaises(pairing.PairingError):
            pairing.validate_observation(entry, changed, 1, expected_mode=pairing.MODE, expected_stage="capture")
        observation = pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                                   expected_stage="capture")
        observation["raw_wire_observation"]["gc_payload_bytes"]["after_drop_and_collect"] = 1
        with self.assertRaises(pairing.PairingError):
            pairing.revalidate_observation(entry, observation)

    def test_cross_module_oracle_declares_nominal_context_rejection(self):
        entry = _ROWS["continuation_cross_module_consumer"]
        self.assertTrue(pairing.matches(entry))
        self.assertEqual(pairing.expected(entry)["resume"]["result"], ["i32:77"])

    def test_one_shot_timed_stages_and_cleanup_semantics_are_separate(self):
        entry = _ROWS["continuation_one_shot"]
        wire = _one_shot_wire()
        self.assertEqual(pairing.argv(entry, "/contained/one-shot.wasm", pairing.MODE),
                         ["--continuation-driver", "/contained/one-shot.wasm", entry["id"],
                          pairing.MODE, pairing.PROFILE, "oneshot"])
        observation = pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                                   expected_stage="resume_reuse")
        self.assertEqual(observation["ns_per_operation"], 40.0)
        self.assertEqual(observation["continuation_lifecycle"]["reuse"],
                         {"kind": "trap", "category": "Trap"})
        self.assertEqual(pairing.revalidate_observation(entry, observation), observation)
        for semantic_only in ("cleanup", "accounting"):
            with self.subTest(stage=semantic_only), self.assertRaises(pairing.PairingError):
                pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                             expected_stage=semantic_only)

    def test_one_shot_rejects_bad_trap_accounting_and_unknown_wire(self):
        entry = _ROWS["continuation_one_shot"]
        for mutate in (
            lambda row: row.update(error_kind="Host"),
            lambda row: row["continuation_bytes"].update(after_collect=1),
            lambda row: row.update(unexpected=True),
        ):
            wire = _one_shot_wire()
            mutate(wire)
            with self.subTest(wire=wire), self.assertRaises(pairing.PairingError):
                pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                             expected_stage="resume_once")

    def test_cross_module_requires_provider_binding_and_checks_nominal_context(self):
        entry = _ROWS["continuation_cross_module_consumer"]
        wire = _cross_module_wire()
        with self.assertRaises(pairing.PairingError):
            pairing.argv(entry, "/contained/consumer.wasm", pairing.MODE)
        self.assertEqual(pairing.argv(entry, "/contained/consumer.wasm", pairing.MODE,
                                      "/contained/provider.wasm"),
                         ["--continuation-cross-module", "/contained/provider.wasm",
                          "/contained/consumer.wasm", pairing.MODE, pairing.PROFILE])
        observation = pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                                   expected_stage="resume")
        self.assertEqual(observation["ns_per_operation"], 30.0)
        self.assertEqual(observation["continuation_lifecycle"]["nominal_probe"],
                         entry["oracle"]["nominal_probe"])
        self.assertEqual(pairing.revalidate_observation(entry, observation), observation)
        with self.assertRaises(pairing.PairingError):
            pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                         expected_stage="nominal_context")

    def test_cross_module_rejects_wrong_nominal_result_or_leaked_cleanup(self):
        entry = _ROWS["continuation_cross_module_consumer"]
        for mutate in (
            lambda row: row.update(exact_host_error_kind="Type"),
            lambda row: row["continuation_bytes"].update(after_drop_and_collect=1),
            lambda row: row["collection_with_root"].update(live=0),
            lambda row: row.update(unknown_field=False),
        ):
            wire = _cross_module_wire()
            mutate(wire)
            with self.subTest(wire=wire), self.assertRaises(pairing.PairingError):
                pairing.validate_observation(entry, wire, 1, expected_mode=pairing.MODE,
                                             expected_stage="capture")


if __name__ == "__main__":
    unittest.main()
