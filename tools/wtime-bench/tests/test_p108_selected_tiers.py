"""Mocked integration checks for P108's selected-tier expansion command."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "tools"))

from verification.artifacts import normal, verify_normal
from verification.producers import p108

TIER_CHECK_SPEC = importlib.util.spec_from_file_location(
    "p108_selected_tier_check", ROOT / "tools/wtime-bench/tier_check.py")
assert TIER_CHECK_SPEC is not None and TIER_CHECK_SPEC.loader is not None
tier_check = importlib.util.module_from_spec(TIER_CHECK_SPEC)
TIER_CHECK_SPEC.loader.exec_module(tier_check)

SOURCE_MAP = {"crates/wtime-runtime/src/lib.rs": "a" * 64}


class P108SelectedTierTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="p108-selected-tiers-", dir="/tmp")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.output = self.root / "selected"
        self.policy_path = self.root / "policy.json"
        self.policy = {"task": "P108", "status": "passed", "timing_evidence": False,
                       "sources": dict(SOURCE_MAP)}
        self.policy_path.write_text(json.dumps(self.policy))
        self.calls = []

    def validate_policy_stub(self, policy, *, sources):
        if policy.get("sources") != sources:
            raise ValueError("P108 policy sources are stale")
        return copy.deepcopy(p108.TIERS)

    def fake_check_tier(self, *, tier, selection, output, runner, builder,
                        register_entry_threshold):
        self.assertEqual(selection, "integration")
        output.mkdir(parents=True, exist_ok=False)
        ordinal = len(self.calls) + 1
        artifacts = {}
        for role in ("benchmark", "conformance", "diagnostic"):
            executable = self.root / "shared-cargo-output" / f"{role}.bin"
            executable.parent.mkdir(parents=True, exist_ok=True)
            executable.write_bytes(f"build-{ordinal}:{role}".encode())
            artifacts[role] = (executable, hashlib.sha256(executable.read_bytes()).hexdigest())
        report = {
            "passed": True,
            "source_stable": True,
            "requested_tier": tier,
            "register_entry_threshold": register_entry_threshold,
            "counts": {"selected_cases": 3, "mode_runs_executed": 6,
                       "failed": 0, "skipped": 0, "unsupported": 0,
                       "all_checks_executed": 89},
            "binary": {"path": str(artifacts["benchmark"][0]),
                       "sha256": artifacts["benchmark"][1]},
            "conformance_binary": {"path": str(artifacts["conformance"][0]),
                                   "sha256": artifacts["conformance"][1]},
            "diagnostic_instrumentation": {"binary": str(artifacts["diagnostic"][0]),
                                           "binary_sha256": artifacts["diagnostic"][1]},
        }
        (output / "report.json").write_text(json.dumps(report))
        self.calls.append((tier, register_entry_threshold, report))
        return report

    def invoke(self, *, stale_policy=False, unstable_report=False):
        if stale_policy:
            self.policy_path.write_text(json.dumps(dict(
                self.policy, sources={"crates/wtime-runtime/src/lib.rs": "b" * 64})))

        checker = self.fake_check_tier
        if unstable_report:
            def checker(**kwargs):
                report = self.fake_check_tier(**kwargs)
                report["source_stable"] = False
                (kwargs["output"] / "report.json").write_text(json.dumps(report))
                return report

        with mock.patch.object(p108, "source_identity", return_value=dict(SOURCE_MAP)), \
             mock.patch.object(p108, "validate_policy", side_effect=self.validate_policy_stub), \
             mock.patch.object(tier_check, "ROOT", self.root), \
             mock.patch.object(tier_check, "check_tier", side_effect=checker):
            return tier_check._selected_policy(
                self.policy_path, output=self.output, selection="integration",
                runner=mock.Mock(), builder=mock.Mock())

    def test_selected_expansion_matches_the_exact_four_portfolio_configurations(self):
        selected = self.invoke()
        self.assertEqual(selected["counts"]["selected_tiers"], 4)
        self.assertEqual(selected["configurations"], p108.TIERS)
        self.assertEqual(set(selected["tiers"]), {row["name"] for row in p108.TIERS})
        self.assertEqual([call[:2] for call in self.calls], [
            (row["mode"], row["register_entry_threshold"]) for row in p108.TIERS])

    def test_selected_expansion_rejects_stale_policy_and_source_drift(self):
        with self.assertRaises(tier_check.TierCheckError):
            self.invoke(stale_policy=True)
        self.assertEqual(self.calls, [])
        self.assertFalse(self.output.exists())

        self.output = self.root / "unstable-selected"
        self.policy_path = self.root / "current-policy.json"
        self.policy_path.write_text(json.dumps(self.policy))
        self.calls = []
        with self.assertRaises(tier_check.TierCheckError):
            self.invoke(unstable_report=True)
        self.assertFalse((self.output / "report.json").exists())

    def test_retained_executable_hashes_survive_overwritten_shared_cargo_outputs(self):
        selected = self.invoke()
        self.assertEqual(set(selected["executables"]), {row["name"] for row in p108.TIERS})
        for tier in p108.TIERS:
            name = tier["name"]
            self.assertEqual(set(selected["executables"][name]),
                             {"benchmark", "conformance", "diagnostic"})
            for reference in selected["executables"][name].values():
                path = verify_normal(reference)
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), reference["sha256"])
        for role in ("benchmark", "conformance", "diagnostic"):
            path = self.root / "shared-cargo-output" / f"{role}.bin"
            self.assertEqual(path.read_bytes(), b"build-4:" + role.encode())
        self.assertEqual(len({
            selected["executables"][row["name"]]["benchmark"]["sha256"]
            for row in p108.TIERS
        }), 4)


if __name__ == "__main__":
    unittest.main()
