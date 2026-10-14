import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

import memory_workloads
import workloads


OLD_WORKLOADS_SHA256 = "54c4faf544274f2539ec6c749c4b02d5205dcd12d3c14042356ea8bc3240b0dc"
OLD_SELECTIONS_SHA256 = "632eebe4740859ebd4c8b8097ec9559c88b69dd562525c1948e313d67ec7be07"
OLD_MEMORY_SELECTION_SHA256 = "344588d37eb8d12a3fc6f130526ff23601445d789a9972f08f562dd714e8700d"
OLD_ROW_COUNT = 441
ALIAS_COUNTS = (0, 1, 17, 32, 256, 1024)


def canonical_sha(value):
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


class P126MemoryScalingTests(unittest.TestCase):
    def setUp(self):
        self.registry = json.loads((HERE / "workloads.json").read_text())
        self.by_id = {row["id"]: row for row in self.registry["workloads"]}
        self.alias_ids = {
            f"memory_widths_{family}_{count}"
            for family in ("predictable", "shuffled", "read_scan")
            for count in ALIAS_COUNTS
        }

    def test_old_rows_and_selection_membership_remain_identical(self):
        old_rows = self.registry["workloads"][:OLD_ROW_COUNT]
        self.assertEqual(canonical_sha(old_rows), OLD_WORKLOADS_SHA256)

        selections = self.registry["selections"]
        memory_members = selections["memory"]["workloads"]
        self.assertEqual(canonical_sha(memory_members[:35]), OLD_MEMORY_SELECTION_SHA256)
        self.assertEqual(set(memory_members[35:]), self.alias_ids)
        self.assertEqual(len(memory_members), 35 + len(self.alias_ids))

        old_selections = json.loads(json.dumps(selections))
        old_selections["memory"]["workloads"] = old_selections["memory"]["workloads"][:35]
        self.assertEqual(canonical_sha(old_selections), OLD_SELECTIONS_SHA256)
        self.assertEqual(len(self.registry["workloads"]), OLD_ROW_COUNT + len(self.alias_ids))

    def test_alias_rows_match_generated_metadata_and_full_registry(self):
        modeled = {row["id"]: row for row in memory_workloads.entries()}
        memory_members = set(self.registry["selections"]["memory"]["workloads"])
        self.assertEqual(memory_members, set(modeled) | {"memory_fill_scan"})
        self.assertEqual(set(modeled) & self.alias_ids, self.alias_ids)

        for wid in sorted(self.alias_ids):
            row = self.by_id[wid]
            self.assertEqual(row, modeled[wid], wid)
            count = int(wid.rsplit("_", 1)[1])
            self.assertEqual(row["params"][0]["min"], count)
            self.assertEqual(row["params"][0]["max"], count)
            self.assertEqual(row["params"][0]["baseline"], count)
            self.assertEqual(row["reset_policy"],
                             "fresh_instance_for_cold_stages_reused_instance_for_warm_call")
            self.assertIn("warm_call", row["stages"])

        with tempfile.TemporaryDirectory(prefix="p126-registry-") as tmp:
            root = Path(tmp).resolve()
            synthetic = (root / "workloads.json").resolve()
            self.assertTrue(synthetic.is_relative_to(root))
            synthetic.write_text(json.dumps(self.registry))
            workloads.validate_registry(json.loads(synthetic.read_text()))

    def test_alias_oracles_return_complete_independent_memory_images(self):
        for family, shuffled in (("predictable", False), ("shuffled", True)):
            for count in ALIAS_COUNTS:
                wid = f"memory_widths_{family}_{count}"
                self.assertEqual(memory_workloads._workload_family(wid), f"memory_widths_{family}")
                actual, image = memory_workloads._widths_model(count, shuffled)
                expected = memory_workloads.expected_memory(wid, count)
                self.assertEqual(memory_workloads.oracle(wid, count), actual)
                self.assertEqual(expected["result"], actual)
                self.assertEqual(len(image), memory_workloads.PAGE)
                self.assertEqual(expected["full_memory_sha256"], hashlib.sha256(image).hexdigest())
                self.assertEqual(self.by_id[wid]["work_units"], {
                    "count": count * 19, "unit": "memory_operations"})

        for count in ALIAS_COUNTS:
            wid = f"memory_widths_read_scan_{count}"
            self.assertEqual(memory_workloads._workload_family(wid), "memory_widths_read_scan")
            actual, image = memory_workloads._read_scan_model(count)
            expected = memory_workloads.expected_memory(wid, count)
            self.assertEqual(memory_workloads.oracle(wid, count), actual)
            self.assertEqual(expected["result"], actual)
            self.assertEqual(len(image), memory_workloads.PAGE)
            self.assertEqual(expected["full_memory_sha256"], hashlib.sha256(image).hexdigest())
            self.assertEqual(self.by_id[wid]["work_units"], {
                "count": count * 2, "unit": "memory_operations"})

    def test_page_bounds_are_explicit(self):
        self.assertEqual(len(memory_workloads._widths_model(2048, False)[1]),
                         memory_workloads.PAGE)
        with self.assertRaisesRegex(ValueError, "exceeds one memory page"):
            memory_workloads._widths_model(2049, False)
        self.assertEqual(len(memory_workloads._read_scan_model(memory_workloads.PAGE)[1]),
                         memory_workloads.PAGE)
        with self.assertRaisesRegex(ValueError, "exceeds one memory page"):
            memory_workloads._read_scan_model(memory_workloads.PAGE + 1)


if __name__ == "__main__":
    unittest.main()
