"""Synthetic matrix checks for the P109 executed-opcode evidence validator."""
from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "tools/checks"))
sys.path.insert(0, str(ROOT / "tools/verification/producers"))

import p109_coverage  # noqa: E402
import p109  # noqa: E402
from verification.artifacts import VerificationError,digest  # noqa: E402


with (ROOT / "docs/architecture/opcodes.json").open(encoding="utf-8") as _ledger_file:
    _historical = json.load(_ledger_file)
_ENCODINGS = [row["encoding"] for row in _historical["instructions"]]
_LEDGER = {"instructions": [{"encoding": encoding} for encoding in _ENCODINGS]}
_ASSIGNED = {p109_coverage.mapping.runtime_opcode(encoding) for encoding in _ENCODINGS}


def _report(suite: str, configuration: str, opcodes=None, **updates):
    mode, threshold = p109.CONFIGURATIONS[configuration]
    strategy = "disabled" if mode != "register" else "adaptive" if threshold is not None else "eager"
    report = {
        "suite": suite, "profile": p109.SUITES[suite], "execution_mode": mode,
        "register_entry_threshold": threshold, "register_strategy": strategy,
        "selected": 12, "executed": 12, "failed": 0, "skipped": 0,
        "unsupported": 0, "text_tool_errors": 0, "errors": [],
        "executed_opcodes": sorted(_ASSIGNED) if opcodes is None else opcodes,
    }
    report.update(updates)
    return report


def _matrix():
    return {
        configuration: {
            suite: _report(suite, configuration)
            for suite in p109.SUITES
        }
        for configuration in p109.CONFIGURATIONS
    }


class P109OpcodeCoverageTests(unittest.TestCase):
    def test_passing_test_receipt_binds_current_runtime_and_test_sources(self):
        current={'crates/wtime-runtime/src/lib.rs':'a'*64,'crates/wtime-runtime/tests/performance.rs':'b'*64}
        inputs={'components':{'runtime_sources':{'files':current}}}
        receipt={'schema':'wtime-verification-phase-v1','task':'P109','phase':'verify','status':'passed',
                 'lane':'full','synthetic':False,'inputs':inputs,'input_key':digest(inputs)}
        p109_coverage.validate_receipt_sources(receipt,current)
        for name in current:
            changed=dict(current);changed[name]='c'*64
            with self.subTest(source=name),self.assertRaises(VerificationError):
                p109_coverage.validate_receipt_sources(receipt,changed)

    def test_development_synthetic_or_tampered_receipts_cannot_refresh_premises(self):
        current={'crates/wtime-runtime/tests/performance.rs':'a'*64}
        inputs={'components':{'runtime_sources':{'files':current}}}
        original={'schema':'wtime-verification-phase-v1','task':'P109','phase':'verify','status':'passed',
                  'lane':'full','synthetic':False,'inputs':inputs,'input_key':digest(inputs)}
        for updates in ({'lane':'development'},{'synthetic':True},{'task':'P108'},{'phase':'analyze'},
                        {'input_key':'0'*64}):
            with self.subTest(updates=updates),self.assertRaises(VerificationError):
                p109_coverage.validate_receipt_sources(dict(original,**updates),current)

    def test_complete_584_opcode_matrix_covers_each_configuration_and_suite(self):
        self.assertEqual(len(_ENCODINGS), 584)
        self.assertEqual(len(_ASSIGNED), 584)
        coverage = p109_coverage.opcode_sets(_LEDGER, _matrix())
        self.assertEqual(set(coverage), set(p109.CONFIGURATIONS))
        for configuration in p109.CONFIGURATIONS:
            with self.subTest(configuration=configuration):
                self.assertEqual(coverage[configuration],
                                 {"assigned": 584, "executed": 584, "missing": []})

    def test_matrix_rejects_missing_configuration_or_suite(self):
        reports = _matrix()
        del reports["compact"]
        with self.assertRaises(VerificationError):
            p109_coverage.opcode_sets(_LEDGER, reports)

        reports = _matrix()
        del reports["register-adaptive-3"]["interactions"]
        with self.assertRaises(VerificationError):
            p109_coverage.opcode_sets(_LEDGER, reports)

    def test_each_configuration_requires_every_assigned_opcode_and_no_extras(self):
        cases = (
            (sorted(_ASSIGNED - {min(_ASSIGNED)}), "missing assigned opcode"),
            (sorted(_ASSIGNED | {max(_ASSIGNED) + 1}), "extra opcode"),
        )
        for opcodes, label in cases:
            reports = _matrix()
            for report in reports["register-eager"].values():
                report["executed_opcodes"] = opcodes
            with self.subTest(case=label), self.assertRaises(VerificationError):
                p109_coverage.opcode_sets(_LEDGER, reports)

    def test_malformed_opcode_ids_and_empty_or_incomplete_execution_are_rejected(self):
        mutations = (
            {"executed_opcodes": [True, *sorted(_ASSIGNED - {min(_ASSIGNED)})]},
            {"selected": 0, "executed": 0},
            {"selected": 12, "executed": 11},
        )
        for mutation in mutations:
            reports = _matrix()
            reports["decoded"]["core3"].update(mutation)
            with self.subTest(mutation=mutation), self.assertRaises(VerificationError):
                p109_coverage.opcode_sets(_LEDGER, reports)

    def test_per_suite_mode_threshold_strategy_metadata_is_exact(self):
        reports = _matrix()
        reports["register-adaptive-3"]["mvp"]["register_entry_threshold"] = None
        with self.assertRaises(VerificationError):
            p109_coverage.opcode_sets(_LEDGER, reports)

        reports = _matrix()
        reports["compact"]["phase3"]["register_strategy"] = "eager"
        with self.assertRaises(VerificationError):
            p109_coverage.opcode_sets(_LEDGER, reports)


if __name__ == "__main__":
    unittest.main()
