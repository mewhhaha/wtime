"""Pure validator tests for P145's untimed register-path observations."""
from __future__ import annotations

import copy
from collections import Counter
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.verification.producers import p145


def row(entry=1, dispatches=1, exits=0, fallback=0, *, cache=None):
    hot = dispatches > 0 or exits > 0
    return {
        "entry": entry, "result": "i32:42", "register_dispatches": dispatches,
        "register_exits": exits, "fallback_frames": fallback,
        "classification": p145.classify(dispatches, exits, fallback),
        "working_peak_bytes": 24 if hot else 0,
        "working_peak_frames": int(hot), "capacity_samples": 1,
        "cache_retained_bytes": cache, "active_scopes_after": 0,
    }


def wire(strategy="eager", *, rows=None, compile_only=False):
    threshold = 0 if strategy == "eager" else 3
    if rows is None:
        rows = [] if compile_only else [row(i + 1, cache=None if strategy == "eager" else 0)
                                         for i in range(1 if strategy == "eager" else 4)]
    return {"schema": "wtime-workload-path-v1", "instrumented": True,
            "timing_evidence": False, "compiled": True,
            "compile_only": compile_only, "strategy": strategy,
            "threshold": threshold, "invocations": rows}


class P145ValidatorTests(unittest.TestCase):
    def test_wire_requires_exact_schema_strategy_and_typed_fields(self):
        good = wire()
        p145.validate_wire(good, "eager", 0, "i32:42")
        for mutate in (
            lambda value: value.update(extra=True),
            lambda value: value.update(instrumented=1),
            lambda value: value.update(strategy="adaptive"),
            lambda value: value["invocations"][0].update(extra=1),
        ):
            bad = copy.deepcopy(good)
            mutate(bad)
            with self.subTest(bad=bad), self.assertRaises((ValueError, p145.VerificationError)):
                p145.validate_wire(bad, "eager", 0, "i32:42")

    def test_eager_classifies_all_four_execution_paths(self):
        examples = ((1, 0, 0, "optimized"), (0, 1, 0, "prefix_exit"),
                    (1, 0, 1, "mixed"), (0, 0, 1, "whole_function_fallback"))
        for dispatches, exits, fallback, classification in examples:
            with self.subTest(classification=classification):
                observed = row(dispatches=dispatches, exits=exits, fallback=fallback)
                p145.validate_wire(wire(rows=[observed]), "eager", 0, "i32:42")
                observed["classification"] = "mixed" if classification != "mixed" else "optimized"
                with self.assertRaises(p145.VerificationError):
                    p145.validate_wire(wire(rows=[observed]), "eager", 0, "i32:42")

    def test_adaptive_uses_threshold_plus_one_invocations(self):
        payload = wire("adaptive")
        self.assertEqual(len(p145.validate_wire(payload, "adaptive", 3, "i32:42")), 4)
        payload["invocations"].pop()
        with self.assertRaises(p145.VerificationError):
            p145.validate_wire(payload, "adaptive", 3, "i32:42")

    def test_compile_only_requires_zero_invocations(self):
        self.assertEqual(p145.validate_wire(wire(compile_only=True), "eager", 0, "-", True), [])
        payload = wire(compile_only=True)
        payload["invocations"].append(row())
        with self.assertRaises(p145.VerificationError):
            p145.validate_wire(payload, "eager", 0, "-", True)

    def test_wire_rejects_oracle_mismatch_booleans_and_u64_overflow(self):
        payload = wire()
        with self.assertRaises(p145.VerificationError):
            p145.validate_wire(payload, "eager", 0, "i32:43")
        for bad_value in (True, -1, 1 << 64):
            bad = wire()
            bad["invocations"][0]["register_dispatches"] = bad_value
            with self.subTest(value=bad_value), self.assertRaises(p145.VerificationError):
                p145.validate_wire(bad, "eager", 0, "i32:42")

    def test_wire_rejects_dirty_cleanup_and_stale_activation_capacity(self):
        mutations = (("active_scopes_after", 1), ("capacity_samples", 0),
                     ("working_peak_bytes", 0), ("working_peak_frames", 2))
        for field, value in mutations:
            bad = wire()
            bad["invocations"][0][field] = value
            with self.subTest(field=field), self.assertRaises(p145.VerificationError):
                p145.validate_wire(bad, "eager", 0, "i32:42")
        cold = wire(rows=[row(dispatches=0, exits=0, fallback=1)])
        cold["invocations"][0]["working_peak_bytes"] = 24
        with self.assertRaises(p145.VerificationError):
            p145.validate_wire(cold, "eager", 0, "i32:42")

    def _cases(self, root: Path):
        from workloads import selection
        entries = selection("scalar")
        entries = [entry for entry in entries if entry["id"] == "integer_loop"]
        self.assertEqual(len(entries), 1)
        entry = entries[0]
        case = p145.case_settings(entry)
        bench = Path(__file__).resolve().parents[2] / "wtime-bench"
        wasm_src = bench / entry["source"]["path"].replace(".wat", ".wasm")
        wasm = root / "fixture.wasm"
        self.assertTrue(wasm.resolve().is_relative_to(root))
        wasm.write_bytes(wasm_src.read_bytes())
        case["fixture"] = {"path": str(wasm), "sha256": hashlib.sha256(wasm.read_bytes()).hexdigest(),
                           "bytes": wasm.stat().st_size}
        wat = bench / entry["source"]["path"]
        source = root / "source.wat"
        self.assertTrue(source.resolve().is_relative_to(root))
        source.write_bytes(wat.read_bytes())
        case["source"] = {"path": str(source), "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                          "bytes": source.stat().st_size}
        return [entry], [case]

    def test_case_validation_rejects_missing_duplicate_and_unknown_ids(self):
        with tempfile.TemporaryDirectory(prefix="p145-cases-") as temp:
            root = Path(temp).resolve()
            entries, cases = self._cases(root)
            p145.validate_cases(cases, entries)
            for candidate in ([], [cases[0], cases[0]], [{**cases[0], "id": "unknown"}]):
                with self.subTest(count=len(candidate)), self.assertRaises(p145.VerificationError):
                    p145.validate_cases(candidate, entries)

    def test_case_validation_binds_profile_arguments_and_fixture_hashes(self):
        from selection_manifest import rows
        selection = json.loads((p145.ROOT / "docs/benchmarks/performance/P067-eager-selection.json").read_text())
        settings = [p145.case_settings(entry) for entry in rows(selection)]
        self.assertEqual(Counter(case['driver'] for case in settings), {
            'plain': 138, 'host_tick': 6, 'table_a': 6, 'representative': 9,
            'alias': 3, 'multi': 1, 'seeded-copy': 5, 'seeded-fill': 5, 'compile_only': 1})
        with tempfile.TemporaryDirectory(prefix="p145-bindings-") as temp:
            root = Path(temp).resolve()
            entries, cases = self._cases(root)
            for field, value in (("profile", "wrong-profile"), ("arguments", "i32:999")):
                bad = copy.deepcopy(cases)
                bad[0][field] = value
                with self.subTest(field=field), self.assertRaises(p145.VerificationError):
                    p145.validate_cases(bad, entries)
            for ref in ("fixture", "source"):
                bad = copy.deepcopy(cases)
                bad[0][ref]["sha256"] = "0" * 64
                with self.subTest(ref=ref), self.assertRaises(p145.VerificationError):
                    p145.validate_cases(bad, entries)


if __name__ == "__main__":
    unittest.main()
