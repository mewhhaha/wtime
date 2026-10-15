"""Strict timing identity tests; all synthetic artifacts stay in temp roots."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


identity = load(BENCH / "execution_identity.py", "fast_execution_identity_tests")
adapter = load(BENCH / "p123_adapter.py", "fast_execution_adapter_tests")
baseline = load(BENCH / "baseline.py", "fast_execution_baseline_tests")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_map():
    return {
        "crates/wtime-runtime/src/lib.rs": "1" * 64,
        "tools/wtime-bench/Cargo.lock": "2" * 64,
        "tools/wtime-bench/fixtures/scalar/x.wat": "3" * 64,
        "tools/wtime-bench/workloads.json": "4" * 64,
        "tools/wtime-bench/compare.py": "5" * 64,
        "tools/wtime-bench/run.py": "6" * 64,
        "tools/wtime-bench/workloads.py": "7" * 64,
        "tools/wtime-bench/scalar_workloads.py": "8" * 64,
        "tools/wtime-bench/scalar_p128_workloads.py": "9" * 64,
        "tools/wtime-bench/p128_scalar_scaling.py": "a" * 64,
        "tools/wtime-bench/analyze.py": "9" * 64,
        "tools/wtime-bench/gates.py": "b" * 64,
        "tools/wtime-bench/p046_gate.py": "c" * 64,
        "tools/wtime-bench/p123_adapter.py": "d" * 64,
        "tools/wtime-bench/baseline.py": "e" * 64,
        "tools/wtime-bench/execution_identity.py": "f" * 64,
        "tools/wtime-bench/focused_compare.py": "0" * 64,
        "tools/wtime-bench/selection_manifest.py": "0" * 64,
        "tools/wtime-bench/cold_products.py": "1" * 64,
        "tools/wtime-bench/semantic_receipt.py": "2" * 64,
        "tools/wtime-bench/root_workloads.py": "3" * 64,
        "tools/wtime-bench/root_compare.py": "4" * 64,
        "tools/wtime-bench/thread_compare.py": "5" * 64,
        "tools/wtime-bench/shared_bulk_compare.py": "6" * 64,
        "tools/wtime-bench/shared_bulk_workloads.py": "7" * 64,
        "tools/wtime-bench/wait_queue_workloads.py": "8" * 64,
        "tools/wtime-bench/gc_collect_workloads.py": "9" * 64,
        "tools/wtime-bench/gc_collect_compare.py": "a" * 64,
    }


class ExecutionIdentityTests(unittest.TestCase):
    def test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs(self):
        recorded = source_map()
        current = dict(recorded)
        current["tools/wtime-bench/analyze.py"] = "2" * 64
        current["tools/wtime-bench/gates.py"] = "3" * 64
        binding = identity.verify(recorded, current)
        self.assertEqual(binding["execution_inputs"], identity.project(recorded))
        self.assertEqual(binding["excluded_analysis_inputs"]["tools/wtime-bench/analyze.py"], {
            "recorded_sha256": "9" * 64, "current_sha256": "2" * 64, "changed": True,
        })

    def test_runtime_timer_fixture_and_oracle_edits_stale_legacy_pairs(self):
        recorded = source_map()
        for path in ("crates/wtime-runtime/src/lib.rs", "tools/wtime-bench/compare.py",
                     "tools/wtime-bench/fixtures/scalar/x.wat",
                     "tools/wtime-bench/run.py", "tools/wtime-bench/workloads.py",
                     "tools/wtime-bench/scalar_p128_workloads.py",
                     "tools/wtime-bench/p128_scalar_scaling.py"):
            with self.subTest(path=path):
                changed = dict(recorded)
                changed[path] = "f" * 64
                with self.assertRaisesRegex(ValueError, "execution source identity is stale"):
                    identity.verify(recorded, changed)

    def test_membership_changes_and_unknown_python_modules_fail_closed(self):
        recorded = source_map()
        added = dict(recorded)
        added["tools/wtime-bench/workloads_new.py"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "unclassified benchmark Python identity"):
            identity.verify(recorded, added)
        missing = dict(recorded)
        del missing["tools/wtime-bench/scalar_workloads.py"]
        with self.assertRaisesRegex(ValueError, "execution source identity is stale"):
            identity.verify(recorded, missing)

    def test_focused_producer_changes_stale_focused_pairs(self):
        recorded = source_map()
        binding = identity.verify(recorded, dict(recorded), focused=True)
        self.assertTrue(binding["focused"])
        for name in ("selection_manifest.py", "semantic_receipt.py", "root_workloads.py", "root_compare.py",
                     "thread_compare.py"):
            with self.subTest(name=name):
                changed = dict(recorded)
                changed["tools/wtime-bench/" + name] = "a" * 64
                with self.assertRaisesRegex(ValueError, "execution source identity is stale"):
                    identity.verify(recorded, changed, focused=True)

    def test_verify_inputs_uses_live_sources_without_running_a_comparison(self):
        recorded = source_map()
        report = {"selection": "scalar", "metadata": {"candidate_sources": recorded}}
        compare = mock.Mock()
        compare.selection_uses_oracle_module.return_value = False
        compare.sources.return_value = dict(recorded)
        with mock.patch.dict(sys.modules, {"compare": compare}):
            result = identity.verify_inputs(report)
        compare.sources.assert_called_once_with(include_memory=False, include_simd=False,
                                                include_representative=False,
                                                include_integration=False)
        self.assertEqual(result["execution_inputs"], identity.project(recorded))

    def test_focused_identity_uses_task_declaration_without_registry_selection_lookup(self):
        recorded = source_map()
        report = {"selection": "P047", "metadata": {"candidate_sources": recorded}}
        compare = mock.Mock()
        compare.selection_uses_oracle_module.side_effect = AssertionError("P047 is not a registry family")
        compare.sources.return_value = dict(recorded)
        with mock.patch.dict(sys.modules, {"compare": compare}):
            result = identity.verify_inputs(report, focused=True)
        compare.sources.assert_called_once_with(include_memory=True, include_simd=True,
                                                include_representative=True, include_integration=True)
        self.assertTrue(result["focused"])

    def test_adapter_archive_corruption_is_still_rejected(self):
        with tempfile.TemporaryDirectory(prefix="fast-adapter-corruption-") as raw:
            root = Path(raw).resolve()
            archive = root / "archive"
            archive.mkdir()
            payload = b"source snapshot"
            source = archive / "source/tools/wtime-bench/analyze.py"
            source.parent.mkdir(parents=True)
            source.write_bytes(payload)
            (archive / "manifest.json").write_text(json.dumps({
                "schema": 1,
                "files": {"source/tools/wtime-bench/analyze.py": {
                    "sha256": sha(payload), "size_bytes": len(payload),
                    "mode_bits": stat.S_IMODE(source.stat().st_mode), "role": "source",
                    "source_path": "tools/wtime-bench/analyze.py",
                }},
            }))
            source.write_bytes(b"tampered snapshot")
            with mock.patch.object(adapter, "HERE", root / "bench"):
                with self.assertRaisesRegex(ValueError, "archive hash mismatch"):
                    adapter.verify(archive)


if __name__ == "__main__":
    unittest.main()
