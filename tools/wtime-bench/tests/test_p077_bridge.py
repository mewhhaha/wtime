"""Contained synthetic tests for the P077 focused wait-queue bridge."""
from pathlib import Path
import copy
import json
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import execution_identity
import focused_compare
import selection_manifest
import shared_bulk_compare
import wait_queue_workloads as model


class WaitQueueBridgeTests(unittest.TestCase):
    STAGE = "steady_state_group"
    CPUS = [0, 1]

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-p077-bridge-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    @staticmethod
    def entry(workload_id="wait_queue_cohort_w2_a1"):
        return next(row for row in model.entries() if row["id"] == workload_id)

    def _output_root(self, name):
        path = self.root / name
        self.assertTrue(path.resolve().is_relative_to(self.root))
        path.mkdir()
        return path

    def test_profile_product_binds_wait_queue_timer_driver_fixture_and_retainer(self):
        entry = self.entry()
        self.assertEqual(focused_compare.profile(entry), model.PROFILE)
        declaration = {"rows": [{"id": entry["id"], "stages": [self.STAGE]}]}
        with mock.patch.object(focused_compare.compare, "sha", return_value="a" * 64):
            product = focused_compare.products([entry], declaration,
                {"baseline_mode": model.MODE, "candidate_mode": model.MODE})[0]
        timer = product["timer"]
        self.assertEqual(timer["wait_queue_model_sha256"], "a" * 64)
        self.assertEqual(timer["wait_queue_driver_sha256"], "a" * 64)
        self.assertEqual(timer["memory_retainer_sha256"], "a" * 64)
        self.assertEqual(timer["wait_queue_fixture_wat_sha256"], model.WAT_SHA256)
        self.assertEqual(timer["wait_queue_fixture_wasm_sha256"], model.WASM_SHA256)
        self.assertIn(model.TIMING_BOUNDARY, timer["measurement_policy"])
        self.assertIn("full-memory validation outside elapsed interval", timer["measurement_policy"])

    def test_execution_identity_binds_queue_model_and_memory_retainer_only_for_focused(self):
        names = set(execution_identity._FOCUSED_TIMING) | {"wait_queue_workloads.py"}
        source_map = {f"tools/wtime-bench/{name}": "a" * 64 for name in names}
        focused = execution_identity.project(source_map, focused=True)
        legacy = execution_identity.project(source_map, focused=False)
        for name in ("wait_queue_workloads.py", "shared_bulk_compare.py"):
            path = f"tools/wtime-bench/{name}"
            self.assertIn(path, focused)
            self.assertNotIn(path, legacy)

    def test_fixture_preparation_reuses_only_source_bound_encoded_guest(self):
        entries = model.entries()
        output = self._output_root("fixtures")
        fixtures = focused_compare.prepare_fixtures(
            entries, self.root, output, self.root / "unused-encoder")
        self.assertEqual(set(fixtures), {row["id"] for row in entries})
        for entry in entries:
            actual = Path(fixtures[entry["id"]]).resolve(strict=True)
            wanted = (TOOL / entry["source"]["path"]).resolve(strict=True).with_suffix(".wasm")
            self.assertEqual(actual, wanted)
            self.assertEqual(focused_compare.compare.sha(actual), model.WASM_SHA256)

    def test_observer_uses_fixed_cohort_cli_affinity_validator_and_memory_retainer(self):
        entry = self.entry("wait_queue_cohort_w32_a8")
        output = self._output_root("observations")
        sentinel = {"mode": model.MODE, "stage": self.STAGE, "result": model.oracle(entry["id"], 32)}
        with mock.patch.object(focused_compare, "thread_process", return_value={"wire": "raw"}) as process, \
             mock.patch.object(model, "validate_observation", return_value=sentinel) as validate, \
             mock.patch.object(shared_bulk_compare, "retain_memory_images", return_value=sentinel) as retain:
            observed = focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE,
                7, model.MODE, 0, output, thread_cpus=self.CPUS)
        self.assertIs(observed, sentinel)
        argv = process.call_args.args[1]
        self.assertEqual(argv, ["--wait-queue-cohort", "fixture.wasm", entry["id"],
            32, 8, self.STAGE, 7, model.MODE, model.PROFILE])
        self.assertEqual(process.call_args.args[2], self.CPUS)
        validate.assert_called_once_with(entry, {"wire": "raw"}, expected_batch=7,
                                          expected_thread_cpus=self.CPUS)
        retain.assert_called_once_with(sentinel, output)

    def test_observer_rejects_wrong_mode_batch_and_threshold_before_process(self):
        entry = self.entry()
        with mock.patch.object(focused_compare, "thread_process") as process:
            with self.assertRaisesRegex(ValueError, "fixed decoded threads-profile cohort lane"):
                focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE,
                    1, "compact", 0, self.root, thread_cpus=self.CPUS)
            with self.assertRaisesRegex(ValueError, "fixed decoded threads-profile cohort lane"):
                focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE,
                    model.MAX_BATCH + 1, model.MODE, 0, self.root, thread_cpus=self.CPUS)
            with self.assertRaisesRegex(ValueError, "fixed decoded threads-profile cohort lane"):
                focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE,
                    1, model.MODE, 0, self.root, thread_cpus=self.CPUS,
                    register_entry_threshold=3)
        process.assert_not_called()

    def test_batch_calibration_uses_whole_cohort_duration_and_4096_cap(self):
        entry = self.entry()
        self.assertEqual(focused_compare.batch_limit(entry), model.MAX_BATCH)
        calls = []

        def observe(side, batch):
            calls.append((side, batch))
            return {"batch": batch, "ns_per_operation": 7_000.0}

        batch, audit = focused_compare.calibrate(entry, observe)
        self.assertGreaterEqual(batch * 7_000, 20_000_000)
        self.assertGreater(batch, 128)
        self.assertLessEqual(batch, model.MAX_BATCH)
        self.assertEqual(len(audit[-1]["observations"]), 2)
        self.assertEqual(calls[-2:], [("baseline", batch), ("candidate", batch)])

    def test_selection_manifest_binds_six_registry_rows_to_exact_family_and_stage(self):
        registry = json.loads(selection_manifest.DEFAULT_REGISTRY.read_text())
        by_id = {row["id"]: row for row in registry["workloads"]}
        selected = []
        roles = (("targeted", "small"), ("targeted", "large"), ("held_out", "other"),
                 ("protected", "other"), ("protected", "other"), ("protected", "other"))
        for entry, (role, scale) in zip(model.entries(), roles):
            selected.append({"family": "wait-queues", "id": entry["id"], "role": role,
                "scale": scale, "stages": [self.STAGE],
                "row_sha256": selection_manifest._canonical_sha(entry)})
        for workload_id, stages in (("prepare_large", ["prepare"]),
                                    ("call_overhead", ["first_call", "load_and_call"])):
            entry = by_id[workload_id]
            selected.append({"family": "original-mvp", "id": workload_id,
                "role": "protected", "scale": "other", "stages": stages,
                "row_sha256": selection_manifest._canonical_sha(entry)})
        declaration = {"schema": selection_manifest.SCHEMA, "task": "P077", "lane": "candidate",
            "rationale": "synthetic fixed queue row binding", "registry_sha256":
            selection_manifest._sha(selection_manifest.DEFAULT_REGISTRY.read_bytes()),
            "rows": selected, "semantic_cases": ["fixed wait-queue cohorts"]}
        self.assertEqual(selection_manifest.load_data(declaration), declaration)
        bad_family = copy.deepcopy(declaration)
        next(row for row in bad_family["rows"] if row["id"].startswith("wait_queue_"))["family"] = "threads"
        with self.assertRaisesRegex(ValueError, "wait-queues family"):
            selection_manifest.load_data(bad_family)
        bad_stage = copy.deepcopy(declaration)
        next(row for row in bad_stage["rows"] if row["id"].startswith("wait_queue_"))["stages"] = ["load_and_call"]
        with self.assertRaisesRegex(ValueError, "stages must be a unique subset"):
            selection_manifest.load_data(bad_stage)

    def _raw(self, entry, batch=2):
        waiters, addresses = model.WAITERS_ADDRESSES[
            next(i for i, row in enumerate(model.entries()) if row["id"] == entry["id"])]
        return {"schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
            "case": entry["id"], "mode": model.MODE, "profile": model.PROFILE,
            "stage": self.STAGE, "waiters": waiters, "addresses": addresses,
            "batch": batch, "timeout_ns": model.WAIT_TIMEOUT_NS,
            "instrumented": False, "report_complete": True,
            "every_group_full_memory_checked": True, "elapsed_ns": 2_000_000,
            "calls_checked": batch * (waiters + 1 + addresses),
            "worker_returns": [0] * (batch * waiters),
            "notify_totals": [waiters] * batch,
            "notify_attempts": [waiters] * batch,
            "cleanup_counts": [0] * (batch * addresses),
            "pre_wait_ready_counts": [waiters] * batch,
            "worker_affinity": [self.CPUS[:] for _ in range(waiters)],
            "readiness_evidence": model.READINESS_EVIDENCE,
            "timing_boundary": model.TIMING_BOUNDARY,
            "backings_hex": {name: image.hex() for name, image in model.expected(entry).items()}}

    def test_retained_full_memory_proof_revalidates_with_supplied_model_validator(self):
        entry = self.entry()
        validated = model.validate_observation(entry, self._raw(entry), expected_batch=2,
                                               expected_thread_cpus=self.CPUS)
        root = self._output_root("retained-memory")
        stored = shared_bulk_compare.retain_memory_images(validated, root)
        self.assertNotIn("backings_hex", stored["raw_wire_observation"])
        validator = mock.Mock(wraps=model.validate_observation)
        rebuilt = shared_bulk_compare.revalidate_observation(entry, stored, root,
            expected_batch=2, expected_thread_cpus=self.CPUS, validator=validator)
        self.assertEqual(rebuilt, stored)
        validator.assert_called_once()
        self.assertEqual(len(list((root / "memory-images").iterdir())), 1)

    def test_retained_full_memory_rejects_changed_image_and_normalized_values(self):
        entry = self.entry()
        validated = model.validate_observation(entry, self._raw(entry), expected_batch=2,
                                               expected_thread_cpus=self.CPUS)
        root = self._output_root("tamper-memory")
        stored = shared_bulk_compare.retain_memory_images(validated, root)
        clean = copy.deepcopy(stored)
        image_name, ref = next(iter(clean["memory_image_refs"].items()))
        image_path = root / ref["path"]
        self.assertTrue(image_path.resolve().is_relative_to(root))
        image_path.write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "bytes or digest differ"):
            shared_bulk_compare.revalidate_observation(entry, clean, root,
                expected_batch=2, expected_thread_cpus=self.CPUS, validator=model.validate_observation)

        fresh = self._output_root("normalized-tamper")
        clean = shared_bulk_compare.retain_memory_images(validated, fresh)
        clean["batch"] = True
        with self.assertRaisesRegex(ValueError, "normalized view differs"):
            shared_bulk_compare.revalidate_observation(entry, clean, fresh,
                expected_batch=2, expected_thread_cpus=self.CPUS, validator=model.validate_observation)


if __name__ == "__main__":
    unittest.main()
