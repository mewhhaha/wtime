"""P045 corpus validation and batched external-oracle checks."""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1]
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import p045_corpus


# (param i32) -> i32 { local.get 0 }, with an exported function named `run`.
IDENTITY_MODULE = "0061736d0100000001060160017f017f030201000707010372756e00000a0601040020000b"
IDENTITY_BODY = "0020000b"
# One-page memory, with `run(param i32) -> i32` storing its argument at byte zero.
MEMORY_MODULE = (
    "0061736d01000000"
    "01060160017f017f"
    "03020100"
    "0503010001"
    "0710020372756e0000066d656d6f72790200"
    "0a0d010b004100200036020020000b"
)
MEMORY_BODY = "004100200036020020000b"


def make_cases(count: int = 1000, family: str = "scalar") -> list[dict]:
    rows = []
    for index in range(count):
        rows.append({
            "schema": "p045-corpus-v1",
            "case_index": index,
            "seed": 20445,
            "body_hex": MEMORY_BODY if family == "memory" else IDENTITY_BODY,
            "module_hex": MEMORY_MODULE if family == "memory" else IDENTITY_MODULE,
            "input_i32": index - 500,
            "expected_i32": index - 500,
            "family": family,
            "expected_kind": "returned",
            "export": "run",
            "events": 1,
            "policy_count": 3,
            "fuel_probes": 3,
            "compact_steps": 1,
        })
        if family == "memory":
            rows[-1]["memory_export"] = "memory"
            rows[-1]["expected_memory"] = {
                "bytes": 65536,
                "writes": [{
                    "offset": 0,
                    "data_hex": (index - 500 & 0xFFFFFFFF).to_bytes(4, "little").hex(),
                }],
            }
    return rows


def write_log(path: Path, cases: list[dict], binary_path: Path) -> None:
    with path.open("w", encoding="utf-8") as output:
        for case in cases:
            output.write("P045_CASE " + json.dumps(case, sort_keys=True) + "\n")
        summary = {"schema": "p045-corpus-v1", "case_count": len(cases), "binary_path": str(binary_path)}
        output.write("P045_SUMMARY " + json.dumps(summary) + "\n")


class P045CompactFuzzTests(unittest.TestCase):
    def _isolated_paths(self, root: Path):
        log = (root / "rust.stdout.log").resolve()
        output = (root / "report").resolve()
        binary = (root / "rust-binary").resolve()
        self.assertTrue(log.is_relative_to(root.resolve()))
        self.assertTrue(output.is_relative_to(root.resolve()))
        self.assertTrue(binary.is_relative_to(root.resolve()))
        binary.write_bytes(b"synthetic P045 binary identity")
        return log, output, binary

    def test_valid_thousand_case_corpus_replays_in_one_batched_node_run(self):
        with tempfile.TemporaryDirectory(prefix="p045-valid-") as directory:
            root = Path(directory).resolve()
            log, output, binary = self._isolated_paths(root)
            write_log(log, make_cases(), binary)
            report = p045_corpus.execute(log, output, min_cases=1000)
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["tested"]["case_count"], 1000)
            self.assertEqual(report["tested"]["unique_modules"], 1)
            self.assertEqual(set(report["family_counts"]), p045_corpus.FAMILIES)
            self.assertEqual(report["families"], sorted(p045_corpus.FAMILIES))
            self.assertEqual(report["family_counts"]["scalar"], 1000)
            self.assertEqual(len(report["module_sha256"]), 1)
            self.assertEqual(report["rust_binary"]["path"], str(binary))
            self.assertTrue(report["checks"]["external_oracle"])
            self.assertTrue(report["checks"]["identity_stable"])
            self.assertEqual(report["checks"]["value_cases"], 1000)
            self.assertTrue((output / "p045_corpus.py").is_file())
            self.assertTrue((output / "command.json").is_file())
            self.assertTrue((output / "report.json").resolve().is_relative_to(root))

    def test_rejects_duplicate_seed_and_insufficient_compact_coverage(self):
        bad_rows = make_cases()
        bad_rows[1]["case_index"] = bad_rows[0]["case_index"]
        with tempfile.TemporaryDirectory(prefix="p045-invalid-") as directory:
            root = Path(directory).resolve()
            log, _, binary = self._isolated_paths(root)
            write_log(log, bad_rows, binary)
            with self.assertRaisesRegex(p045_corpus.CorpusError, "duplicate case_index"):
                p045_corpus.parse_log(log)
            bad_rows = make_cases()
            bad_rows[0]["seed"] = 7
            write_log(log, bad_rows, binary)
            with self.assertRaisesRegex(p045_corpus.CorpusError, "seed must be exactly 20445"):
                p045_corpus.parse_log(log)
            bad_rows = make_cases()
            bad_rows[0]["compact_steps"] = 0
            write_log(log, bad_rows, binary)
            with self.assertRaisesRegex(p045_corpus.CorpusError, "compact_steps must be positive"):
                p045_corpus.parse_log(log)

    @unittest.skipUnless(shutil.which("node"), "Node.js executable is required for the external oracle")
    def test_node_oracle_rejects_an_altered_expected_result(self):
        with tempfile.TemporaryDirectory(prefix="p045-altered-") as directory:
            root = Path(directory).resolve()
            log, output, binary = self._isolated_paths(root)
            rows = make_cases()
            rows[0]["expected_i32"] = 999
            write_log(log, rows, binary)
            report = p045_corpus.execute(log, output)
            self.assertEqual(report["status"], "failed")
            self.assertIn("Node result mismatch in case 0", report["errors"][0])
            self.assertTrue((output / "report.json").resolve().is_relative_to(root))

    def test_rejects_forged_body_before_node_replay(self):
        with tempfile.TemporaryDirectory(prefix="p045-forged-body-") as directory:
            root = Path(directory).resolve()
            log, output, binary = self._isolated_paths(root)
            rows = make_cases()
            rows[0]["body_hex"] = "000b"
            write_log(log, rows, binary)
            report = p045_corpus.execute(log, output)
            self.assertEqual(report["status"], "failed")
            self.assertIn("does not match exported function code body", report["errors"][0])
            self.assertTrue((output / "p045_corpus.py").resolve().is_relative_to(root))
            self.assertTrue((output / "command.json").resolve().is_relative_to(root))

    @unittest.skipUnless(shutil.which("node"), "Node.js executable is required for the external oracle")
    def test_node_rejects_memory_mismatch_and_retains_hash_summary(self):
        with tempfile.TemporaryDirectory(prefix="p045-memory-mismatch-") as directory:
            root = Path(directory).resolve()
            log, output, binary = self._isolated_paths(root)
            rows = make_cases(family="memory")
            write_log(log, rows, binary)
            valid = p045_corpus.execute(log, output)
            self.assertEqual(valid["status"], "passed")
            self.assertEqual(valid["checks"]["memory_checks"], 1000)
            self.assertEqual(len(valid["node_result_summary"]["memory_sha256"]), 1000)

            rows[0]["expected_memory"]["writes"][0]["data_hex"] = "00000000"
            mismatch_output = (root / "memory-mismatch").resolve()
            self.assertTrue(mismatch_output.is_relative_to(root))
            write_log(log, rows, binary)
            mismatch = p045_corpus.execute(log, mismatch_output)
            self.assertEqual(mismatch["status"], "failed")
            self.assertIn("Node memory mismatch in case 0", mismatch["errors"][0])
            self.assertEqual(mismatch["node_result_summary"]["memory_checks"], 1000)
            self.assertEqual(len(mismatch["node_result_summary"]["memory_sha256"]), 1000)
            self.assertTrue((mismatch_output / "report.json").resolve().is_relative_to(root))


if __name__ == "__main__":
    unittest.main()
