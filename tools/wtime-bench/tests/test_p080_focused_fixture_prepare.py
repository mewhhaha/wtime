"""Mocked fixture-preparation checks for the private P080 focused_compare draft."""
from pathlib import Path
import json
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
PRIVATE_ROOT = TOOL.parents[1]
REPOSITORY = Path("/workspace/wtime")
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import continuation_workloads  # noqa: E402
import focused_compare  # noqa: E402

with (TOOL / "workloads.json").open(encoding="utf-8") as registry_file:
    _REGISTRY = json.load(registry_file)
_REGISTRY_ROWS = {row["id"]: row for row in _REGISTRY["workloads"]}


class ContinuationFixturePreparationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-p080-fixtures-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.rows = {row["id"]: row for row in continuation_workloads.workload_entries()}

    def inside(self, relative):
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), f"path escapes temporary root: {path}")
        return path

    def write(self, relative, data):
        path = self.inside(relative)
        parent = self.inside(path.parent.relative_to(self.root))
        parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def source_wat(self, entry):
        source = TOOL / entry["source"]["path"]
        return self.write(Path("source") / Path(entry["source"]["path"]).name,
                          source.read_bytes())

    def expected_wasm(self, entry):
        return (REPOSITORY / "target/performance/P012-fixtures" /
                (Path(entry["source"]["path"]).stem + ".wasm")).read_bytes()

    def test_matching_sibling_is_reused_and_mismatched_sibling_is_rejected(self):
        entry = self.rows["continuation_depth_0"]
        wat = self.source_wat(entry)
        sibling = self.write(wat.with_suffix(".wasm").relative_to(self.root),
                             self.expected_wasm(entry))
        output = self.inside("run/fixtures")
        output.mkdir(parents=True)
        records = []
        with mock.patch.object(focused_compare.subprocess, "run") as run:
            result = focused_compare._continuation_wasm(
                entry, wat, output, PRIVATE_ROOT / "target/debug/wtime-conformance", records)
        run.assert_not_called()
        self.assertEqual(result, sibling)
        self.assertEqual(records[0]["method"], "source_sibling")
        self.assertIsNone(records[0]["returncode"])
        self.assertEqual(records[0]["observed_wasm_sha256"], entry["source"]["wasm_sha256"])

        stale = self.write(wat.with_suffix(".wasm").relative_to(self.root), b"stale wasm")
        with mock.patch.object(focused_compare.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "sibling Wasm bytes changed"):
                focused_compare._continuation_wasm(
                    entry, wat, output, PRIVATE_ROOT / "target/debug/wtime-conformance", [])
        run.assert_not_called()
        self.assertTrue(stale.is_file())

    def test_missing_one_shot_consumer_and_provider_siblings_encode_to_output(self):
        ids = ("continuation_one_shot", "continuation_cross_module_consumer",
               "continuation_cross_module_provider")
        entries = [self.rows[workload_id] for workload_id in ids]
        output = self.inside("run/fixtures")
        output.mkdir(parents=True)
        encoder = PRIVATE_ROOT / "target/debug/wtime-conformance"
        records = []
        expected = {entry["id"]: self.expected_wasm(entry) for entry in entries}
        wat_by_path = {self.source_wat(entry): entry for entry in entries}

        def fake_encode(argv, *, cwd, text, capture_output, timeout):
            self.assertEqual(argv[0], str(encoder))
            self.assertEqual(argv[1], "encode")
            self.assertEqual(cwd, focused_compare.ROOT)
            self.assertTrue(text)
            self.assertTrue(capture_output)
            self.assertEqual(timeout, 60)
            wat = Path(argv[2])
            self.assertIn(wat, wat_by_path)
            destination = self.inside(Path(argv[4]).relative_to(self.root))
            self.assertEqual(destination.parent, output)
            destination.write_bytes(expected[wat_by_path[wat]["id"]])
            return subprocess.CompletedProcess(argv, 0, "mock encoded", "")

        with mock.patch.object(focused_compare.subprocess, "run", side_effect=fake_encode) as run:
            results = {}
            for entry in entries:
                wat = wat_by_path_inv = next(path for path, row in wat_by_path.items()
                                             if row["id"] == entry["id"])
                results[entry["id"]] = focused_compare._continuation_wasm(
                    entry, wat_by_path_inv, output, encoder, records)

        self.assertEqual(run.call_count, 3)
        self.assertEqual(set(results), set(ids))
        for entry in entries:
            result = results[entry["id"]]
            self.assertEqual(result.parent, output)
            self.assertEqual(focused_compare.compare.sha(result), entry["source"]["wasm_sha256"])
            source = next(path for path, row in wat_by_path.items()
                          if row["id"] == entry["id"])
            self.assertFalse(source.with_suffix(".wasm").exists())
        self.assertEqual([record["returncode"] for record in records], [0, 0, 0])
        self.assertTrue(all(record["method"] == "private_encoder" for record in records))
        self.assertTrue(all(record["observed_wasm_sha256"] == record["expected_wasm_sha256"]
                            for record in records))
        self.assertTrue(all(record["stdout"] == "mock encoded" for record in records))

    def test_prepare_fixtures_binds_cross_module_provider_as_setup_only(self):
        entry = _REGISTRY_ROWS["continuation_cross_module_consumer"]
        output = self.inside("run/fixtures")
        output.mkdir(parents=True)
        encoder = PRIVATE_ROOT / "target/debug/wtime-conformance"
        expected_wasm = {
            "cross_module_consumer": (REPOSITORY / "target/performance/P012-fixtures/"
                                       "cross_module_consumer.wasm").read_bytes(),
            "cross_module_provider": (REPOSITORY / "target/performance/P012-fixtures/"
                                      "cross_module_provider.wasm").read_bytes(),
        }
        records = []

        def fake_encode(argv, **kwargs):
            fixture_name = Path(argv[2]).stem.removeprefix("cross_module_")
            fixture_name = "cross_module_" + fixture_name
            destination = self.inside(Path(argv[4]).relative_to(self.root))
            destination.write_bytes(expected_wasm[fixture_name])
            return subprocess.CompletedProcess(argv, 0, "mock encoded", "")

        with mock.patch.object(focused_compare.subprocess, "run", side_effect=fake_encode) as run:
            fixtures = focused_compare.prepare_fixtures(
                [entry], self.root, output, encoder, encoding_records=records)

        self.assertEqual(run.call_count, 2)
        self.assertEqual(set(fixtures), {entry["id"]})
        self.assertEqual(set(fixtures[entry["id"]]), {"path", "setup_modules"})
        self.assertEqual(len(fixtures[entry["id"]]["setup_modules"]), 1)
        consumer_wasm = fixtures[entry["id"]]["path"]
        provider_wasm = fixtures[entry["id"]]["setup_modules"][0]
        self.assertEqual(consumer_wasm.parent, output)
        self.assertEqual(provider_wasm.parent, output)
        self.assertEqual(focused_compare.compare.sha(consumer_wasm), entry["source"]["wasm_sha256"])
        provider = {row["id"]: row for row in continuation_workloads.workload_entries()}[
            "continuation_cross_module_provider"]
        self.assertEqual(focused_compare.compare.sha(provider_wasm),
                         provider["source"]["wasm_sha256"])
        self.assertEqual({record["workload"] for record in records},
                         {"continuation_cross_module_consumer", "continuation_cross_module_provider"})
        self.assertTrue(all(record["returncode"] == 0 for record in records))

    def test_wat_encoder_exit_and_encoded_hash_are_fail_closed(self):
        entry = self.rows["continuation_one_shot"]
        wat = self.source_wat(entry)
        encoder = PRIVATE_ROOT / "target/debug/wtime-conformance"

        output = self.inside("run/nonzero")
        output.mkdir(parents=True)
        records = []
        with mock.patch.object(focused_compare.subprocess, "run",
                               return_value=subprocess.CompletedProcess([], 3, "", "encoder error")):
            with self.assertRaisesRegex(ValueError, "fixture encoder failed"):
                focused_compare._continuation_wasm(entry, wat, output, encoder, records)
        self.assertEqual(records[0]["returncode"], 3)
        self.assertFalse(wat.with_suffix(".wasm").exists())

        output = self.inside("run/wrong-bytes")
        output.mkdir(parents=True)
        records = []

        def wrong_encode(argv, **kwargs):
            self.inside(Path(argv[4]).relative_to(self.root)).write_bytes(b"wrong wasm")
            return subprocess.CompletedProcess(argv, 0, "", "")

        with mock.patch.object(focused_compare.subprocess, "run", side_effect=wrong_encode):
            with self.assertRaisesRegex(ValueError, "encoded Wasm bytes differ from model"):
                focused_compare._continuation_wasm(entry, wat, output, encoder, records)
        self.assertEqual(records[0]["returncode"], 0)
        self.assertNotEqual(records[0]["observed_wasm_sha256"], entry["source"]["wasm_sha256"])

        bad_wat = self.write("bad-source/one_shot.wat", b"not the model fixture")
        output = self.inside("run/bad-source")
        output.mkdir(parents=True)
        with mock.patch.object(focused_compare.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "WAT source changed"):
                focused_compare._continuation_wasm(entry, bad_wat, output, encoder, [])
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
