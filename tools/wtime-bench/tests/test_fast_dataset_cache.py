from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))

from dataset_cache import (  # noqa: E402
    CacheConflictError,
    DatasetCacheError,
    admit,
    lookup,
)


def report_value(*, passed=True, source_stable=True, runs=None):
    return {
        "passed": passed,
        "source_stable": source_stable,
        "runs": runs if runs is not None else [
            {"pairs": [{"case": "a", "value": 1}]},
            {"pairs": [{"case": "a", "value": 2}]},
        ],
    }


class FastDatasetCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.dataset = self.root / "source"
        self.dataset.mkdir()
        self.cache = self.root / "cache"
        self.report = self.dataset / "report.json"
        self.write_report(report_value())
        self.identity = {"selection": "small", "seed": 7, "binary_sha256": "b" * 64}
        self.validator_calls = 0

    def tearDown(self):
        self.temp.cleanup()

    def write_report(self, value):
        self.report.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

    def validator(self, path):
        self.validator_calls += 1
        value = json.loads(path.read_text(encoding="utf-8"))
        if value.get("passed") is not True:
            raise ValueError("validator rejected report")

    def check_receipt_inside_temp_root(self, path):
        self.assertTrue(path.is_relative_to(self.root))

    def test_admit_then_reuse_without_running_dataset_producer(self):
        produced = []

        def produce_if_missing():
            produced.append(True)
            return admit(self.report, self.cache, self.identity, validator=self.validator)

        receipt = produce_if_missing()
        self.check_receipt_inside_temp_root(receipt)
        self.assertEqual(receipt, self.cache / receipt.name)
        first_validation_count = self.validator_calls
        hit = lookup(self.cache, self.identity, validator=self.validator)
        self.assertEqual(hit, self.report.resolve())
        self.assertEqual(len(produced), 1)
        self.assertGreater(self.validator_calls, first_validation_count)

    def test_corrupt_report_and_changed_file_and_extra_member_fail_closed(self):
        receipt = admit(self.report, self.cache, self.identity, validator=self.validator)
        self.write_report(report_value(passed=False))
        with self.assertRaises(DatasetCacheError):
            lookup(self.cache, self.identity, validator=self.validator)
        self.write_report(report_value())
        payload = self.dataset / "payload.bin"
        payload.write_bytes(b"one")
        with self.assertRaises(DatasetCacheError):
            lookup(self.cache, self.identity, validator=self.validator)

        # Admit the expanded tree, then verify that both content changes and
        # newly added files invalidate the exact membership receipt.
        second_identity = {**self.identity, "revision": 2}
        second = admit(self.report, self.cache, second_identity, validator=self.validator)
        payload.write_bytes(b"two")
        with self.assertRaises(DatasetCacheError):
            lookup(self.cache, second_identity, validator=self.validator)
        payload.write_bytes(b"one")
        (self.dataset / "extra").write_text("unexpected", encoding="utf-8")
        with self.assertRaises(DatasetCacheError):
            lookup(self.cache, second_identity, validator=self.validator)
        self.check_receipt_inside_temp_root(receipt)
        self.check_receipt_inside_temp_root(second)

    def test_identity_and_receipt_tampering_are_not_cache_misses(self):
        receipt = admit(self.report, self.cache, self.identity, validator=self.validator)
        other_identity = {**self.identity, "seed": 8}
        self.assertIsNone(lookup(self.cache, other_identity, validator=self.validator))
        record = json.loads(receipt.read_text(encoding="utf-8"))
        record["identity"]["seed"] = 8
        receipt.write_text(json.dumps(record), encoding="utf-8")
        with self.assertRaises(DatasetCacheError):
            lookup(self.cache, self.identity, validator=self.validator)

    def test_symlink_escape_and_cache_inside_dataset_are_rejected(self):
        outside = self.root / "outside.bin"
        outside.write_bytes(b"outside")
        (self.dataset / "escape.bin").symlink_to(outside)
        with self.assertRaises(DatasetCacheError):
            admit(self.report, self.cache, self.identity, validator=self.validator)
        (self.dataset / "escape.bin").unlink()
        with self.assertRaises(DatasetCacheError):
            admit(self.report, self.dataset / "cache", self.identity, validator=self.validator)

    def test_empty_partial_report_and_validator_rejection_are_refused(self):
        for value in (
            report_value(passed=False),
            report_value(source_stable=False),
            report_value(runs=[]),
            report_value(runs=[{"pairs": []}, {"pairs": [{"case": "a"}]}]),
        ):
            self.write_report(value)
            with self.assertRaises(DatasetCacheError):
                admit(self.report, self.cache, {"case": "invalid"}, validator=self.validator)

        self.write_report(report_value())

        def reject(_path):
            raise ValueError("rejected")

        with self.assertRaisesRegex(ValueError, "rejected"):
            admit(self.report, self.cache, {"case": "validator"}, validator=reject)
        self.assertEqual(list(self.cache.glob("*.json")) if self.cache.exists() else [], [])

    def test_conflicting_dataset_for_same_identity_is_not_overwritten(self):
        first = admit(self.report, self.cache, self.identity, validator=self.validator)
        original = first.read_bytes()
        self.write_report(report_value(runs=[
            {"pairs": [{"case": "different", "value": 1}]},
            {"pairs": [{"case": "different", "value": 2}]},
        ]))
        with self.assertRaises(CacheConflictError):
            admit(self.report, self.cache, self.identity, validator=self.validator)
        self.assertEqual(first.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
