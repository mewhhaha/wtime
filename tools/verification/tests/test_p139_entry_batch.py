"""Fast, contained checks for the P139 evidence producer."""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools.verification.producers import p139


class P139ProducerTests(unittest.TestCase):
    def test_smoke_rows_bind_both_adaptive_threshold_settings(self):
        rows = p139.smoke_rows()
        self.assertEqual(len(rows), 8)
        self.assertEqual({row["calls"] for row in rows}, {1, 10, 100, 10000})
        self.assertEqual({row["baseline_strategy"] for row in rows}, {"eager"})
        self.assertEqual({row["baseline_threshold"] for row in rows}, {0})
        self.assertEqual({row["candidate_strategy"] for row in rows}, {"adaptive"})
        self.assertEqual({row["candidate_threshold"] for row in rows}, {3, 10})
        self.assertEqual({(row["input"], row["expected"]) for row in rows}, {(7, 8)})

    def test_row_validation_rejects_order_change_and_omission(self):
        rows = p139.smoke_rows()
        self.assertEqual(p139.validate_rows(rows), rows)
        with self.assertRaises(p139.P139Error):
            p139.validate_rows(rows[:-1])
        with self.assertRaises(p139.P139Error):
            p139.validate_rows(list(reversed(rows)))

    def test_archive_identity_rejects_wrong_commit_and_manifest(self):
        manifest = {"schema": 1, "mode": "register", "repository_commit": p139.ARCHIVE_COMMIT}
        p139.validate_archive_identity(manifest, p139.ARCHIVE_MANIFEST_SHA256)
        with self.assertRaises(p139.P139Error):
            p139.validate_archive_identity({**manifest, "repository_commit": "0" * 40},
                                           p139.ARCHIVE_MANIFEST_SHA256)
        with self.assertRaises(p139.P139Error):
            p139.validate_archive_identity(manifest, "0" * 64)

    def test_fixture_validator_binds_actual_provenance_and_validator_logs(self):
        refs = p139._validate_fixture_provenance()
        self.assertEqual(set(refs), {"provenance", "proof", "wasm", "wat", "license", "validator_outputs"})
        self.assertEqual(set(refs["validator_outputs"]), {"build_validate", "decoder_listing"})
        for receipt in refs["validator_outputs"].values():
            self.assertEqual(set(receipt), {"stdout", "exit"})

    def test_output_paths_stay_inside_fresh_verification_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            output = root / "fresh" / "result.json"
            self.assertTrue(p139._inside(output, root))
            p139.write_new(output, {"status": "passed"})
            self.assertEqual(json.loads(output.read_text()), {"status": "passed"})
            self.assertFalse(p139._inside(root.parent / "outside.json", root))

    def test_source_map_requires_complete_owned_closure(self):
        sources = p139._current_p139_sources()
        self.assertEqual(set(sources), set(p139.P139_FILES))
        self.assertIn("tools/wtime-bench/tests/test_p139_entry_settings.py", sources)
        self.assertIn("tools/verification/tests/test_p139_entry_batch.py", sources)

    def test_public_receipt_checker_rejects_unbound_helpers(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / "proof"
            helper_dir = output / "helpers"
            helper_dir.mkdir(parents=True)
            refs = {}
            for name, source in p139._expected_helper_sources().items():
                target = helper_dir / name
                shutil.copyfile(source, target)
                refs[name] = p139.ref(target)
            p139._validate_helper_refs(refs, output)
            altered = dict(refs)
            altered["p139.py"] = {**refs["p139.py"], "sha256": "0" * 64}
            with self.assertRaises(p139.P139Error):
                p139._validate_helper_refs(altered, output)
            missing = dict(refs)
            del missing["p135.py"]
            with self.assertRaises(p139.P139Error):
                p139._validate_helper_refs(missing, output)


if __name__ == "__main__":
    unittest.main()
