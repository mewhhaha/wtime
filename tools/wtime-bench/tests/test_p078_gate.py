"""Focused P078 shared-bulk candidate-gate checks using synthetic wires."""
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
import shared_bulk_workloads as model
import shared_bulk_compare as adapter


class P078GateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="p078-gate-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.assertTrue(self.root.is_dir())
        self.cpus = [2, 4]

    def entry(self, backing="alias", workers=4, byte_count=512):
        workload_id = f"shared_bulk_copy_{backing}_w{workers}_b{byte_count}"
        return next(row for row in model.entries() if row["id"] == workload_id)

    def wire(self, entry, batch=4):
        oracle = model.oracle(entry["id"], entry["params"][0]["baseline"])
        images = model.expected(entry)
        workers = entry["params"][0]["baseline"]
        return {
            "schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
            "case": entry["id"], "mode": model.MODE, "profile": model.PROFILE,
            "stage": "steady_state_group", "backing": oracle["backing"],
            "workers": workers, "bytes": entry["params"][1]["baseline"],
            "repetitions": model.REPETITIONS, "batch": batch,
            "instrumented": False, "report_complete": True,
            "elapsed_ns": 20_000_000 * batch, "calls_checked": batch * workers,
            "every_group_full_memory_checked": True,
            "worker_returns": oracle["worker_returns"] * batch,
            "worker_affinity": [self.cpus[:] for _ in range(workers)],
            "backings_hex": {name: data.hex() for name, data in images.items()},
        }

    def observation(self, entry, batch=4):
        validated = model.validate_observation(entry, self.wire(entry, batch),
                                              expected_batch=batch,
                                              expected_thread_cpus=self.cpus)
        return adapter.retain_memory_images(validated, self.root)

    def test_gate_revalidates_exact_timer_and_full_memory_view(self):
        entry = self.entry()
        observation = self.observation(entry)
        self.assertEqual(observation["ns_per_operation"], 20_000_000)
        self.assertEqual(observation["batch"], 4)
        candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                         stage="steady_state_group", thread_cpus=self.cpus)
        altered = dict(observation, ns_per_operation=observation["ns_per_operation"] + 1)
        with self.assertRaisesRegex(ValueError, "normalized view differs from revalidated raw image bytes"):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="steady_state_group", thread_cpus=self.cpus)

    def test_gate_rejects_corrupted_full_backing_image_and_compact_result(self):
        entry = self.entry(backing="distinct", byte_count=4097)
        observation = self.observation(entry, batch=1)
        altered = dict(observation, result={"workers": 4})
        with self.assertRaises(ValueError):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="steady_state_group", thread_cpus=self.cpus)

        name, ref = next(iter(observation["memory_image_refs"].items()))
        invalid_ref = dict(ref, path="../escape.bin")
        invalid_refs = dict(observation["memory_image_refs"], **{name: invalid_ref})
        raw = dict(observation["raw_wire_observation"], backings_image_refs=invalid_refs)
        altered = dict(observation, raw_wire_observation=raw, memory_image_refs=invalid_refs)
        with self.assertRaises(ValueError):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="steady_state_group", thread_cpus=self.cpus)

    def test_gate_rejects_modified_content_addressed_image(self):
        entry = self.entry(backing="distinct", workers=1, byte_count=512)
        observation = self.observation(entry, batch=1)
        reference = next(iter(observation["memory_image_refs"].values()))
        image_path = (self.root / reference["path"]).resolve(strict=True)
        self.assertTrue(image_path.is_relative_to(self.root))
        contents = bytearray(image_path.read_bytes())
        contents[0] ^= 1
        image_path.write_bytes(contents)
        with self.assertRaisesRegex(ValueError, "bytes or digest differ"):
            candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                             stage="steady_state_group", thread_cpus=self.cpus)

    def test_gate_binds_each_worker_affinity_to_selected_mask(self):
        entry = self.entry(backing="alias", workers=1, byte_count=32768)
        observation = self.observation(entry, batch=1)
        raw = dict(observation["raw_wire_observation"])
        raw["worker_affinity"] = [[3, 5]]
        altered = dict(observation, raw_wire_observation=raw)
        with self.assertRaisesRegex(ValueError, "worker affinity differs from the predeclared CPU mask"):
            candidate_gate.check_observation(entry, altered, "decoded", self.root,
                                             stage="steady_state_group", thread_cpus=self.cpus)
        with self.assertRaisesRegex(ValueError, "selected steady-state stage"):
            candidate_gate.check_observation(entry, observation, "decoded", self.root,
                                             stage="capture", thread_cpus=self.cpus)

    def test_primary_and_calibration_routes_cover_shared_bulk(self):
        entry = self.entry(backing="distinct", workers=4, byte_count=32768)
        self.assertTrue(candidate_gate.requires_batch_calibration(entry, "steady_state_group"))
        self.assertFalse(candidate_gate.requires_batch_calibration(entry, "prepare"))
        selected = {"id": entry["id"], "role": "targeted", "family": "shared-bulk",
                    "stages": ["steady_state_group"]}
        pairs = [{"workload": entry["id"], "stage": "steady_state_group",
                  "baseline": {"ns_per_operation": 10.0},
                  "candidate": {"ns_per_operation": 9.0}}
                 for _ in range(30)]
        report = {"declaration": {"rows": [selected]},
                  "runs": [{"seed": 78001, "pairs": pairs},
                           {"seed": 78002, "pairs": pairs}],
                  "cold_products": [], "counts": {}}
        with mock.patch.object(candidate_gate.analyze, "analyze", return_value={"per_run": []}):
            result = candidate_gate.decision(report)
        self.assertTrue(all(row["stages"] == {entry["id"]: "steady_state_group"}
                            for row in result["primary"]))

    def test_shared_bulk_model_is_protected_execution_identity(self):
        for name in ("shared_bulk_workloads.py", "shared_bulk_compare.py"):
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


if __name__ == "__main__":
    unittest.main()
