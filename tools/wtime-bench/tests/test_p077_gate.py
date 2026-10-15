"""Focused P077 wait-queue candidate-gate checks with contained synthetic images."""
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
import shared_bulk_compare
import wait_queue_workloads as model


class P077GateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="p077-gate-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.assertTrue(self.root.is_dir())
        self.cpus = [1, 2, 3, 4]

    def entry(self, waiters=32, addresses=8):
        workload_id = f"wait_queue_cohort_w{waiters}_a{addresses}"
        return next(row for row in model.entries() if row["id"] == workload_id)

    def wire(self, entry, batch=3):
        waiters = entry["params"][0]["baseline"]
        addresses = entry["params"][1]["baseline"]
        images = model.expected(entry)
        return {
            "schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
            "case": entry["id"], "mode": model.MODE, "profile": model.PROFILE,
            "stage": "steady_state_group", "waiters": waiters,
            "addresses": addresses, "batch": batch,
            "timeout_ns": model.WAIT_TIMEOUT_NS, "instrumented": False,
            "report_complete": True, "every_group_full_memory_checked": True,
            "elapsed_ns": 80_000 * batch,
            "calls_checked": batch * (waiters + 1 + addresses),
            "worker_returns": [0] * (batch * waiters),
            "notify_totals": [waiters] * batch,
            "notify_attempts": [waiters + 2] * batch,
            "cleanup_counts": [0] * (batch * addresses),
            "pre_wait_ready_counts": [waiters] * batch,
            "worker_affinity": [self.cpus[:] for _ in range(waiters)],
            "readiness_evidence": model.READINESS_EVIDENCE,
            "timing_boundary": model.TIMING_BOUNDARY,
            "backings_hex": {name: image.hex() for name, image in images.items()},
        }

    def observation(self, entry, batch=3):
        validated = model.validate_observation(
            entry, self.wire(entry, batch), expected_batch=batch,
            expected_thread_cpus=self.cpus)
        return shared_bulk_compare.retain_memory_images(validated, self.root)

    def check(self, entry, observation, stage="steady_state_group", cpus=None):
        candidate_gate.check_observation(entry, observation, model.MODE, self.root,
                                         stage=stage,
                                         thread_cpus=self.cpus if cpus is None else cpus)

    def test_gate_revalidates_complete_cohort_timer_and_retained_memory(self):
        entry = self.entry()
        observation = self.observation(entry)
        self.assertEqual(observation["ns_per_operation"], 80_000)
        self.assertEqual(observation["result"]["notify_total"], 32)
        self.assertEqual(observation["result"]["pre_wait_ready_counts"], [32])
        self.assertEqual(observation["memory_proof"]["full_memory_bytes_checked"], 65_536)
        self.assertNotIn("backings_hex", observation["raw_wire_observation"])
        self.check(entry, observation)

    def test_gate_rejects_modified_content_addressed_full_memory(self):
        entry = self.entry(waiters=2, addresses=1)
        observation = self.observation(entry, batch=1)
        reference = observation["memory_image_refs"]["shared"]
        image_path = (self.root / reference["path"]).resolve(strict=True)
        self.assertTrue(image_path.is_relative_to(self.root))
        content = bytearray(image_path.read_bytes())
        content[model.READY_OFFSET] ^= 1
        image_path.write_bytes(content)
        with self.assertRaisesRegex(ValueError, "bytes or digest differ"):
            self.check(entry, observation)

    def test_gate_rejects_escaping_or_mismatched_full_memory_references(self):
        entry = self.entry(waiters=2, addresses=1)
        observation = self.observation(entry, batch=1)
        reference = observation["memory_image_refs"]["shared"]
        bad_ref = dict(reference, path="../outside.bin")
        refs = {"shared": bad_ref}
        raw = dict(observation["raw_wire_observation"], backings_image_refs=refs)
        altered = dict(observation, raw_wire_observation=raw, memory_image_refs=refs)
        with self.assertRaises(ValueError):
            self.check(entry, altered)
        altered = dict(observation, memory_image_refs={})
        with self.assertRaises(ValueError):
            self.check(entry, altered)

    def test_gate_rejects_bad_notify_results_attempts_and_cleanup_accounting(self):
        entry = self.entry(waiters=2, addresses=1)
        for key, bad in (("notify_totals", [1]),
                         ("worker_returns", [0]),
                         ("notify_attempts", [model.MAX_NOTIFY_ATTEMPTS + 1]),
                         ("cleanup_counts", [1]),
                         ("calls_checked", 1)):
            observation = self.observation(entry, batch=1)
            wire = dict(observation["raw_wire_observation"], **{key: bad})
            altered = dict(observation, raw_wire_observation=wire)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.check(entry, altered)

    def test_gate_rejects_wrong_timing_batch_stage_and_instrumentation(self):
        entry = self.entry(waiters=2, addresses=1)
        observation = self.observation(entry, batch=1)
        raw = dict(observation["raw_wire_observation"], elapsed_ns=80_001)
        altered = dict(observation, raw_wire_observation=raw)
        with self.assertRaisesRegex(ValueError, "normalized view differs"):
            self.check(entry, altered)
        raw = dict(observation["raw_wire_observation"], batch=4097)
        altered = dict(observation, raw_wire_observation=raw, batch=4097)
        with self.assertRaises(ValueError):
            self.check(entry, altered)
        with self.assertRaisesRegex(ValueError, "decoded steady-state groups"):
            self.check(entry, observation, stage="load_and_call")
        raw = dict(observation["raw_wire_observation"], instrumented=True)
        with self.assertRaises(ValueError):
            self.check(entry, dict(observation, raw_wire_observation=raw))

    def test_gate_binds_affinity_and_keeps_readiness_claim_honest(self):
        entry = self.entry(waiters=2, addresses=1)
        observation = self.observation(entry, batch=1)
        raw = dict(observation["raw_wire_observation"])
        raw["worker_affinity"] = [[0, 1, 2, 3] for _ in range(2)]
        with self.assertRaisesRegex(ValueError, "predeclared CPU mask"):
            self.check(entry, dict(observation, raw_wire_observation=raw))
        with self.assertRaisesRegex(ValueError, "predeclared CPU mask"):
            self.check(entry, observation, cpus=[0, 1, 2, 3])
        raw = dict(observation["raw_wire_observation"],
                   readiness_evidence="all waiters were queued before the timer")
        with self.assertRaises(ValueError):
            self.check(entry, dict(observation, raw_wire_observation=raw))
        raw = dict(observation["raw_wire_observation"], pre_wait_ready_counts=[1])
        with self.assertRaises(ValueError):
            self.check(entry, dict(observation, raw_wire_observation=raw))
        self.assertIn("queue registration is not observed", model.READINESS_EVIDENCE)

    def test_calibration_and_primary_routing_use_full_waiter_cohorts(self):
        entries = [self.entry(waiters=2, addresses=1), self.entry(waiters=32, addresses=1)]
        for entry in entries:
            self.assertTrue(candidate_gate.requires_batch_calibration(entry, "steady_state_group"))
            self.assertFalse(candidate_gate.requires_batch_calibration(entry, "prepare"))
        selected = [{"id": entry["id"], "role": "targeted", "family": "wait-queues",
                     "stages": ["steady_state_group"]} for entry in entries]
        pairs = [{"workload": entry["id"], "stage": "steady_state_group",
                  "baseline": {"ns_per_operation": 10.0},
                  "candidate": {"ns_per_operation": 9.0}}
                 for entry in entries for _ in range(30)]
        report = {"declaration": {"rows": selected},
                  "runs": [{"seed": 77001, "pairs": pairs},
                           {"seed": 77002, "pairs": pairs}],
                  "cold_products": [], "counts": {}}
        with mock.patch.object(candidate_gate.analyze, "analyze", return_value={"per_run": []}):
            result = candidate_gate.decision(report)
        self.assertTrue(all(row["stages"] == {entry["id"]: "steady_state_group"
                                                for entry in entries}
                            for row in result["primary"]))

    def test_primary_rejects_a_different_waiter_size_or_fanout(self):
        entry = self.entry(waiters=8, addresses=1)
        selected = {"id": entry["id"], "role": "targeted", "family": "wait-queues",
                    "stages": ["steady_state_group"]}
        report = {"declaration": {"rows": [selected]}, "runs": [],
                  "cold_products": [], "counts": {}}
        with mock.patch.object(candidate_gate.analyze, "analyze", return_value={"per_run": []}):
            with self.assertRaisesRegex(ValueError, "same-address cohorts with 2 and 32"):
                candidate_gate.decision(report)

    def test_wait_model_is_focused_execution_input_but_not_legacy_input(self):
        name = "wait_queue_workloads.py"
        path = "tools/wtime-bench/" + name
        self.assertIn(name, execution_identity._KNOWN_FAST_PATH)
        self.assertIn(name, execution_identity._FOCUSED_TIMING)
        self.assertNotIn(name, execution_identity._LEGACY_TIMING)
        self.assertEqual(execution_identity._classify(path, focused=True), "protected")
        required = {"tools/wtime-bench/" + item: "a" * 64
                    for item in execution_identity._FOCUSED_TIMING}
        before = dict(required, **{path: "a" * 64})
        after = dict(before, **{path: "b" * 64})
        with self.assertRaisesRegex(ValueError, "execution source identity is stale"):
            execution_identity.verify(before, after, focused=True)

    def test_wait_queue_protocol_capability_is_required_on_both_sides(self):
        entry = self.entry(waiters=2, addresses=1)
        caps = {"baseline": {"wait_queue_cohort_driver": True},
                "candidate": {"wait_queue_cohort_driver": True}}
        candidate_gate.validate_wait_queue_capability([entry], caps)
        for side, value in (("baseline", False), ("candidate", 1)):
            altered = {key: dict(protocol) for key, protocol in caps.items()}
            altered[side]["wait_queue_cohort_driver"] = value
            with self.subTest(side=side), self.assertRaisesRegex(ValueError, "lacks the wait-queue cohort driver"):
                candidate_gate.validate_wait_queue_capability([entry], altered)


if __name__ == "__main__":
    unittest.main()
