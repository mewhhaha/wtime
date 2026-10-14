import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
PRODUCER = ROOT / "tools/verification/producers/p121.py"
spec = importlib.util.spec_from_file_location("p121_producer", PRODUCER)
p121 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p121)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class P121ReportValidation(unittest.TestCase):
    def fixture(self, root):
        root = Path(root).resolve()
        self.safe_write(root, root / "docs/architecture/corpus-lock.json", json.dumps({
            "suites": {
                "mvp": ["corpus/mvp.wast"],
                "core3": ["corpus/core3.wast"],
                "phase3": ["corpus/phase3.wast"],
                "interactions": ["corpus/interactions.wast"],
            }
        }).encode())
        for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
            self.safe_write(root, root / f"crates/{crate}/src/lib.rs", f"{crate}\n".encode())
        for index in range(31):
            self.safe_write(root, root / f"crates/wtime-runtime/src/module-{index:02}.rs", f"module-{index}\n".encode())
        binary = root / "wtime-conformance"
        self.safe_write(root, binary, b"synthetic instrumented binary")
        corpus = root / "docs/architecture/corpus-lock.json"
        corpus_data = json.loads(corpus.read_text())
        reports = []
        pairs = (("mvp", "mvp"), ("all-phase3-core", "core3"),
                 ("all-phase3-core", "phase3"), ("all-phase3-core", "interactions"))
        for index, (profile, suite) in enumerate(pairs):
            selected = corpus_data["suites"][suite]
            corpus_files = []
            for relative in selected:
                source_path = root / ".cache" / relative
                self.safe_write(root, source_path, f"{suite}\n".encode())
                corpus_files.append(source_path)
            source_entries = [f"corpus-lock:{sha(corpus)}"]
            source_entries.extend(f"{path}:{sha(path)}" for path in corpus_files)
            source_hash = ";".join(source_entries)
            value = {
                "profile": profile,
                "execution_mode": "compact",
                "instrumented": True,
                "suite": suite,
                "source_hash": source_hash,
                "binary_sha256": sha(binary),
                "implementation_sources": p121.expected_implementation_sources(root),
                "selected": 10,
                "executed": 10,
                "worker_stores_observed": 0,
                "failed": 0,
                "skipped": 0,
                "unsupported": 0,
                "text_tool_errors": 0,
                "errors": [],
                "execution_counts": {
                    "total_dispatched_source_events": 15,
                    "optimized_compact_source_events": 12,
                    "decoded_fallback_source_events": 3,
                },
                "preparation_summary": {
                    "compile_count": 3,
                    "published_artifacts": 3,
                    "accepted": 4,
                    "declined": 2,
                    "attempted_work_known_sum": 32,
                    "unpublished_artifacts": 0,
                    "attempted_work_unknown_artifacts": 0,
                    "artifact_retained_budget_bytes": 1024,
                    "peak_artifact_retained_budget_bytes": 512,
                    "artifact_retained_budget_bytes_basis": "conservative capacity plus headers, not allocator physical bytes",
                },
            }
            path = root / f"report-{index}.json"
            self.safe_write(root, path, json.dumps(value).encode())
            reports.append(path)
        output = root / "summary.json"
        self.assertTrue(output.is_relative_to(root))
        return root, binary, reports, output

    def safe_write(self, root, path, data):
        path = Path(path)
        root = Path(root).resolve()
        self.assertTrue(path.is_relative_to(root))
        path.parent.mkdir(parents=True, exist_ok=True)
        self.assertTrue(path.parent.resolve().is_relative_to(root))
        path.write_bytes(data)

    def test_four_cli_reports_validate_with_real_profile_and_hash_inputs(self):
        with tempfile.TemporaryDirectory(prefix="p121-report-") as directory:
            root, binary, reports, output = self.fixture(directory)
            result = p121.validate(reports, binary, output, root)
            self.assertIsNone(result)
            summary = json.loads(output.read_text())
            self.assertEqual(summary["status"], "passed")
            self.assertFalse(summary["timing_evidence"])
            self.assertEqual(summary["totals"]["optimized_compact_source_events"], 48)
            self.assertEqual(summary["totals"]["worker_stores_observed"], 0)
            self.assertEqual(summary["totals"]["preparation"]["compile_count"], 12)
            self.assertEqual(len(summary["reports"]), 4)
            self.assertTrue(output.resolve().is_relative_to(root))

    def test_wrong_counts_and_hidden_failures_are_rejected(self):
        with tempfile.TemporaryDirectory(prefix="p121-counts-") as directory:
            _root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[0].read_text())
            value["executed"] = 9
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "selected/executed"):
                p121.validate(reports, binary, output, _root)

            value["executed"] = 10
            value["failed"] = 1
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "failed/skipped/unsupported"):
                p121.validate(reports, binary, output, _root)

    def test_zero_compact_work_and_foreign_mode_are_rejected(self):
        with tempfile.TemporaryDirectory(prefix="p121-mode-") as directory:
            _root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[0].read_text())
            value["execution_counts"]["optimized_compact_source_events"] = 0
            value["execution_counts"]["decoded_fallback_source_events"] = 15
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "zero Compact work"):
                p121.validate(reports, binary, output, _root)

            value["execution_counts"]["optimized_compact_source_events"] = 12
            value["execution_counts"]["decoded_fallback_source_events"] = 3
            value["execution_mode"] = "decoded"
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "foreign execution mode"):
                p121.validate(reports, binary, output, _root)

    def test_locked_membership_and_preparation_unknowns_are_strict(self):
        with tempfile.TemporaryDirectory(prefix="p121-membership-") as directory:
            root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[1].read_text())
            foreign = root / ".cache/corpus/foreign.wast"
            self.safe_write(root, foreign, b"foreign selection\n")
            value["source_hash"] = ";".join((
                f"corpus-lock:{sha(root / 'docs/architecture/corpus-lock.json')}",
                f"{foreign}:{sha(foreign)}",
            ))
            reports[1].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "membership differs"):
                p121.validate(reports, binary, output, root)

            root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[0].read_text())
            value["preparation_summary"]["attempted_work_unknown_artifacts"] = 1
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "inconsistent preparation"):
                p121.validate(reports, binary, output, root)

    def test_four_command_coverage_and_unsigned_bounds_are_enforced(self):
        with tempfile.TemporaryDirectory(prefix="p121-bounds-") as directory:
            root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[3].read_text())
            value["suite"] = "phase3"
            phase3 = root / ".cache/corpus/phase3.wast"
            value["source_hash"] = ";".join((
                f"corpus-lock:{sha(root / 'docs/architecture/corpus-lock.json')}",
                f"{phase3}:{sha(phase3)}",
            ))
            reports[3].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "cover the locked suite/profile"):
                p121.validate(reports, binary, output, root)

            root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[0].read_text())
            value["execution_counts"]["total_dispatched_source_events"] = p121.MAX_U64 + 1
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "malformed execution counts"):
                p121.validate(reports, binary, output, root)

            root, binary, reports, output = self.fixture(directory)
            value = json.loads(reports[0].read_text())
            value["selected"] = True
            reports[0].write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "malformed selected"):
                p121.validate(reports, binary, output, root)


if __name__ == "__main__":
    unittest.main()
