"""Contained synthetic tests for the P078 shared-bulk focused bridge."""
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

import focused_compare
import execution_identity
import selection_manifest
import shared_bulk_compare
import shared_bulk_workloads as model


class SharedBulkBridgeTests(unittest.TestCase):
    STAGE = "steady_state_group"
    MAX_BATCH = 1024

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-p078-bridge-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    @staticmethod
    def entry():
        return model.entries()[0]

    def test_profile_product_binds_the_exact_model_driver_and_fixture(self):
        entry = self.entry()
        self.assertEqual(focused_compare.profile(entry), model.PROFILE)
        declaration = {"rows": [{"id": entry["id"], "stages": [self.STAGE]}]}
        with mock.patch.object(focused_compare.compare, "sha", return_value="a" * 64):
            product = focused_compare.products([entry], declaration,
                {"baseline_mode": model.MODE, "candidate_mode": model.MODE})[0]
        timer = product["timer"]
        self.assertEqual(product["stage"], "steady_state_group")
        self.assertEqual(timer["shared_bulk_model_sha256"], "a" * 64)
        self.assertEqual(timer["shared_bulk_driver_sha256"], "a" * 64)
        self.assertEqual(timer["shared_bulk_fixture_wat_sha256"], entry["source"]["sha256"])
        self.assertEqual(timer["shared_bulk_fixture_wasm_sha256"], entry["source"]["wasm_sha256"])
        self.assertIn("full-memory validation outside elapsed intervals", timer["measurement_policy"])

    def test_memory_retainer_is_bound_only_in_the_focused_execution_identity(self):
        names = set(execution_identity._FOCUSED_TIMING) | {
            "shared_bulk_workloads.py", "shared_bulk_compare.py"}
        source_map = {f"tools/wtime-bench/{name}": "a" * 64 for name in names}
        focused = execution_identity.project(source_map, focused=True)
        legacy = execution_identity.project(source_map, focused=False)
        for name in ("shared_bulk_workloads.py", "shared_bulk_compare.py"):
            path = f"tools/wtime-bench/{name}"
            self.assertIn(path, focused)
            self.assertNotIn(path, legacy)

    def test_wait_control_binds_latency_and_separate_untimed_cleanup_policy(self):
        import thread_compare
        registry = json.loads(selection_manifest.DEFAULT_REGISTRY.read_text())
        entry = next(row for row in registry['workloads']
                     if row['id'] == 'threads_wait_notify_latency')
        declaration = {'rows': [{'id': entry['id'], 'stages': [self.STAGE]}]}
        with mock.patch.object(focused_compare.compare, 'sha', return_value='a' * 64):
            product = focused_compare.products([entry], declaration,
                {'baseline_mode': 'decoded', 'candidate_mode': 'decoded'})[0]
        timer = product['timer']
        self.assertEqual(timer['wait_samples'], 1024)
        self.assertEqual(timer['wait_timeout_ns'], 1_000_000_000)
        self.assertEqual(timer['wait_cleanup_timeout_ns'], thread_compare.WAIT_CLEANUP_TIMEOUT_NS)
        self.assertEqual(timer['wait_cleanup_timeout_ns'], 1_000_000)
        self.assertIn('1024 accepted guest notify-call-to-waiter-return', timer['measurement_policy'])
        self.assertIn('cleanup checks untimed', timer['measurement_policy'])

    def test_fixture_preparation_reuses_only_the_source_bound_encoded_guest(self):
        entries = model.entries()
        output = self.root / "fixture-output"
        self.assertTrue(output.resolve().is_relative_to(self.root))
        fixtures = focused_compare.prepare_fixtures(entries, self.root, output, self.root / "unused-encoder")
        self.assertEqual(set(fixtures), {entry["id"] for entry in entries})
        for entry in entries:
            wasm = Path(fixtures[entry["id"]]).resolve(strict=True)
            self.assertEqual(wasm, (TOOL / entry["source"]["path"]).resolve(strict=True).with_suffix(".wasm"))
            self.assertEqual(focused_compare.compare.sha(wasm), model.WASM_SHA256)

    def test_observer_uses_fixed_cli_and_binds_worker_affinity_to_validator(self):
        entry = self.entry()
        affinity = [0, 1]
        sentinel = {"stage": self.STAGE, "batch": 3, "result": model.oracle(entry["id"], 1)}
        output = self._output_root("observations")
        with mock.patch.object(focused_compare, "thread_process", return_value={"wire": "retained"}) as run, \
             mock.patch.object(model, "validate_observation", return_value=sentinel) as validate, \
             mock.patch.object(shared_bulk_compare, "retain_memory_images", return_value=sentinel) as retain:
            observed = focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE, 3,
                model.MODE, 0, output, thread_cpus=affinity)
        self.assertIs(observed, sentinel)
        args = run.call_args.args
        self.assertEqual(args[0], "candidate")
        self.assertEqual(args[1], ["--shared-bulk-copy", "fixture.wasm", entry["id"],
            entry["oracle"]["backing"], entry["oracle"]["workers"],
            entry["oracle"]["bytes"], self.STAGE, 3, model.MODE, model.PROFILE])
        self.assertEqual(args[2], affinity)
        validate.assert_called_once_with(entry, {"wire": "retained"}, expected_batch=3,
                                          expected_thread_cpus=affinity)
        retain.assert_called_once_with(sentinel, output)

    def test_observer_rejects_other_modes_before_any_process_call(self):
        entry = self.entry()
        with mock.patch.object(focused_compare, "thread_process") as run:
            with self.assertRaisesRegex(ValueError, "fixed decoded all-phase3-core"):
                focused_compare.observe("candidate", entry, "fixture.wasm", self.STAGE, 1,
                    "compact", 0, self.root, thread_cpus=[0, 1])
        run.assert_not_called()

    def test_batch_calibration_uses_complete_group_elapsed_and_1024_cap(self):
        entry = self.entry()
        self.assertEqual(focused_compare.batch_limit(entry), self.MAX_BATCH)
        calls = []

        def observe(side, batch):
            calls.append((side, batch))
            # A small complete group needs more than 128 repeats; workers and
            # repetitions are already part of that group-level measurement.
            return {"batch": batch, "ns_per_operation": 40_000.0}

        batch, audit = focused_compare.calibrate(entry, observe)
        self.assertGreaterEqual(batch * 40_000, 20_000_000)
        self.assertGreater(batch, 128)
        self.assertLessEqual(batch, self.MAX_BATCH)
        self.assertEqual(len(audit[-1]["observations"]), 2)
        self.assertEqual(calls[-2:], [("baseline", batch), ("candidate", batch)])

    def _selection(self):
        registry = json.loads(selection_manifest.DEFAULT_REGISTRY.read_text())
        registry_sha = selection_manifest._sha(selection_manifest.DEFAULT_REGISTRY.read_bytes())
        by_id = {entry["id"]: entry for entry in registry["workloads"]}
        family = "shared-bulk"
        shared_rows = model.entries()
        chosen = {}
        declared = []

        def add(entry, selected_family, role, scale, stages):
            declared.append({"family": selected_family, "id": entry["id"], "role": role,
                             "scale": scale, "stages": stages,
                             "row_sha256": selection_manifest._canonical_sha(entry)})

        for entry in shared_rows:
            byte_count = entry["oracle"]["bytes"]
            role_scale = ({"role": "targeted", "scale": "small"} if byte_count == 512 else
                          {"role": "targeted", "scale": "large"} if byte_count == 32_768 else
                          {"role": "held_out", "scale": "other"})
            if entry["id"] == "shared_bulk_copy_distinct_w1_b512":
                role_scale = {"role": "protected", "scale": "other"}
            add(entry, family, role_scale["role"], role_scale["scale"], [self.STAGE])
        add(by_id["prepare_large"], "original-mvp", "protected", "other", ["prepare"])
        add(by_id["call_overhead"], "original-mvp", "protected", "other",
            ["first_call", "load_and_call"])
        return {"schema": selection_manifest.SCHEMA, "task": "P078", "lane": "candidate",
                "rationale": "synthetic shared-bulk family and stage binding",
                "registry_sha256": registry_sha, "rows": declared,
                "semantic_cases": ["shared-bulk fixed registry rows"]}

    def test_selection_manifest_binds_registry_family_and_exact_stage(self):
        declaration = self._selection()
        self.assertEqual(selection_manifest.load_data(declaration), declaration)
        bad_family = copy.deepcopy(declaration)
        shared = next(row for row in bad_family["rows"] if row["id"].startswith("shared_bulk_copy_"))
        shared["family"] = "threads"
        with self.assertRaisesRegex(ValueError, "shared-bulk family"):
            selection_manifest.load_data(bad_family)
        bad_stage = copy.deepcopy(declaration)
        shared = next(row for row in bad_stage["rows"] if row["id"].startswith("shared_bulk_copy_"))
        shared["stages"] = ["load_and_call"]
        with self.assertRaisesRegex(ValueError, "stages must be a unique subset"):
            selection_manifest.load_data(bad_stage)

    def _validated_wire(self):
        entry = self.entry()
        workers = entry["oracle"]["workers"]
        images = model.expected(entry)
        wire = {"schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
                "case": entry["id"], "mode": model.MODE, "profile": model.PROFILE,
                "stage": self.STAGE, "backing": entry["oracle"]["backing"],
                "workers": workers, "bytes": entry["oracle"]["bytes"],
                "repetitions": model.REPETITIONS, "batch": 1, "instrumented": False,
                "report_complete": True, "elapsed_ns": 101, "calls_checked": workers,
                "every_group_full_memory_checked": True,
                "worker_returns": [model.REPETITIONS] * workers,
                "worker_affinity": [[0, 1] for _ in range(workers)],
                "backings_hex": {name: image.hex() for name, image in images.items()}}
        return entry, model.validate_observation(entry, wire, expected_batch=1,
                                                  expected_thread_cpus=[0, 1])

    def _output_root(self, name):
        path = self.root / name
        self.assertTrue(path.resolve().is_relative_to(self.root))
        path.mkdir()
        return path

    def test_full_images_are_deduplicated_and_revalidated_from_content_refs(self):
        entry, validated = self._validated_wire()
        root = self._output_root("retained-images")
        stored = shared_bulk_compare.retain_memory_images(validated, root)
        again = shared_bulk_compare.retain_memory_images(validated, root)
        self.assertEqual(stored, again)
        self.assertNotIn("backings_hex", stored["raw_wire_observation"])
        self.assertEqual(stored["raw_wire_observation"]["backings_image_refs"],
                         stored["memory_image_refs"])
        self.assertEqual(len(list((root / "memory-images").iterdir())), 1)
        restored = shared_bulk_compare.revalidate_observation(entry, stored, root,
            expected_batch=1, expected_thread_cpus=[0, 1])
        self.assertEqual(restored, stored)

    def test_reference_traversal_corrupt_bytes_and_normalized_type_drift_fail(self):
        entry, validated = self._validated_wire()
        root = self._output_root("tamper-images")
        stored = shared_bulk_compare.retain_memory_images(validated, root)
        traversal = copy.deepcopy(stored)
        name = next(iter(traversal["memory_image_refs"]))
        traversal["memory_image_refs"][name]["path"] = "../escape.bin"
        traversal["raw_wire_observation"]["backings_image_refs"][name]["path"] = "../escape.bin"
        with self.assertRaises(ValueError):
            shared_bulk_compare.revalidate_observation(entry, traversal, root,
                expected_batch=1, expected_thread_cpus=[0, 1])

        ref = stored["memory_image_refs"][name]
        artifact = root / ref["path"]
        self.assertTrue(artifact.resolve().is_relative_to(root))
        artifact.write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "bytes or digest differ"):
            shared_bulk_compare.revalidate_observation(entry, stored, root,
                expected_batch=1, expected_thread_cpus=[0, 1])

        clean_root = self._output_root("type-drift-images")
        clean = shared_bulk_compare.retain_memory_images(validated, clean_root)
        clean["batch"] = True  # Python equality alone would treat this as integer one.
        with self.assertRaisesRegex(ValueError, "normalized view differs"):
            shared_bulk_compare.revalidate_observation(entry, clean, clean_root,
                expected_batch=1, expected_thread_cpus=[0, 1])


if __name__ == "__main__":
    unittest.main()
