"""Synthetic checks for the P109 normative conformance evidence validators."""
from __future__ import annotations

import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "tools/verification/producers"))

import p109  # noqa: E402
from verification.artifacts import VerificationError  # noqa: E402


def _counts(**updates):
    report = {"selected": 7, "executed": 7, "failed": 0, "skipped": 0,
              "unsupported": 0, "text_tool_errors": 0, "errors": []}
    report.update(updates)
    return report


def _configuration(suite: str, config: str):
    mode, threshold = p109.CONFIGURATIONS[config]
    strategy = "disabled" if mode != "register" else "adaptive" if threshold is not None else "eager"
    return {"suite": suite, "profile": p109.SUITES[suite], "execution_mode": mode,
            "register_entry_threshold": threshold, "register_strategy": strategy}


class P109SelectedConformanceTests(unittest.TestCase):
    def test_counts_require_nonzero_exact_execution_and_zero_strict_integer_errors(self):
        p109.validate_counts(_counts())
        rejected = (
            {"selected": 0}, {"executed": 6}, {"failed": 1}, {"skipped": 1},
            {"unsupported": 1}, {"text_tool_errors": 1}, {"errors": ["diagnostic"]},
            {"selected": True}, {"executed": 7.0}, {"failed": False},
            {"skipped": 0.0}, {"unsupported": "0"}, {"text_tool_errors": None},
        )
        for mutation in rejected:
            with self.subTest(mutation=mutation), self.assertRaises(VerificationError):
                p109.validate_counts(_counts(**mutation))

    def test_all_suite_profile_and_four_mode_threshold_strategy_bindings(self):
        for suite in p109.SUITES:
            for config in p109.CONFIGURATIONS:
                report = _counts(**_configuration(suite, config))
                with self.subTest(suite=suite, configuration=config):
                    p109.validate_configuration(report, suite, config)
                    for field, wrong in (
                        ("suite", "wrong-suite"), ("profile", "wrong-profile"),
                        ("execution_mode", "wrong-mode"),
                        ("register_entry_threshold", 3 if report["register_entry_threshold"] is None else None),
                        ("register_strategy", "wrong-strategy"),
                    ):
                        changed = dict(report)
                        changed[field] = wrong
                        with self.subTest(field=field), self.assertRaises(VerificationError):
                            p109.validate_configuration(changed, suite, config)

    def test_source_map_accepts_current_contained_files_and_rejects_missing_stale_or_escaping(self):
        with tempfile.TemporaryDirectory(prefix="p109-source-map-") as temporary:
            base = Path(temporary).resolve()
            root = base / "contained"
            root.mkdir()
            good = root / "src" / "input.rs"
            good.parent.mkdir()
            good.write_bytes(b"current source\n")
            digest = hashlib.sha256(good.read_bytes()).hexdigest()
            p109.source_matches({"src/input.rs": digest}, root=root)

            with self.subTest(kind="missing"), self.assertRaises(VerificationError):
                p109.source_matches({"src/missing.rs": "0" * 64}, root=root)
            with self.subTest(kind="stale"), self.assertRaises(VerificationError):
                p109.source_matches({"src/input.rs": "0" * 64}, root=root)

            outside = base / "outside.rs"
            outside.write_bytes(b"still within temporary root, outside allowed source root\n")
            outside_digest = hashlib.sha256(outside.read_bytes()).hexdigest()
            with self.subTest(kind="escaping"), self.assertRaises(VerificationError):
                p109.source_matches({"../outside.rs": outside_digest}, root=root)


if __name__ == "__main__":
    unittest.main()
