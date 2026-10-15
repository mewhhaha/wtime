"""Focused policy validation tests for the P108 portable portfolio."""
from __future__ import annotations

import copy
import importlib.util
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from verification.artifacts import VerificationError, normal

_SPEC = importlib.util.spec_from_file_location(
    "p108_under_test", TOOLS / "verification" / "producers" / "p108.py"
)
assert _SPEC is not None and _SPEC.loader is not None
p108 = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(p108)


class P108SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p108-selection-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sources = {"crates/wtime-runtime/src/lib.rs": "a" * 64}
        self.value = self.policy()

    def policy(self):
        milestones = {}
        for task in p108.MILESTONES:
            proof = self.root / f"{task}-proof.json"
            proof.write_text('{"task":"' + task + '","status":"passed"}\n')
            milestones[task] = {
                "status": "done",
                "outcome": "complete" if task not in p108.RETAINED else "adopted",
                "evidence": [normal(proof)],
            }
        return {
            "schema": "wtime-performance-portfolio-v1",
            "default": "decoded",
            "tiers": copy.deepcopy(p108.TIERS),
            "native_enabled": False,
            "aot_enabled": False,
            "cancellation_policy": "exact_source_events",
            "default_external_dependencies": 0,
            "default_unsafe": False,
            "retained_optimizations": dict(p108.RETAINED),
            "milestones": milestones,
            "unselected": [
                {"id": "P071", "outcome": "rejected", "enabled": False,
                 "reason": "Candidate was rejected and its runtime change was restored."},
                {"id": "P080", "outcome": "not_selected", "enabled": False,
                 "reason": "Optional specialist work is not part of the portable portfolio."},
            ],
            "sources": dict(self.sources),
        }

    def rejects(self, value, *, sources=None):
        with self.assertRaises(VerificationError):
            p108.validate_policy(value, sources=self.sources if sources is None else sources)

    def test_complete_policy_keeps_decoded_default_and_only_explicit_experimental_opt_ins(self):
        tiers = p108.validate_policy(self.value, sources=self.sources)
        self.assertEqual(tiers[0], {
            "name": "decoded", "mode": "decoded", "register_entry_threshold": None,
            "support": "default", "features": [],
        })
        self.assertEqual(tiers[1:], p108.TIERS[1:])
        self.assertTrue(all(tier["support"] == "experimental" for tier in tiers[1:]))

    def test_missing_or_pending_required_milestones_are_rejected(self):
        missing = copy.deepcopy(self.value)
        del missing["milestones"]["P067"]
        self.rejects(missing)

        pending = copy.deepcopy(self.value)
        pending["milestones"]["P080"]["status"] = "todo"
        self.rejects(pending)

    def test_rejected_experimental_tier_cannot_become_default(self):
        for mutation in (
            lambda value: value.update(default="compact"),
            lambda value: value["tiers"][1].update(support="default"),
            lambda value: value["tiers"].__setitem__(0, copy.deepcopy(value["tiers"][1])),
        ):
            value = copy.deepcopy(self.value)
            mutation(value)
            self.rejects(value)

    def test_optional_tier_feature_mode_threshold_tuples_are_exact(self):
        mutations = (
            lambda row: row.update(features=["compact-executor", "register-executor"]),
            lambda row: row.update(mode="decoded"),
            lambda row: row.update(register_entry_threshold=3),
            lambda row: row.update(features=[]),
            lambda row: row.update(register_entry_threshold=2),
        )
        for mutation in mutations:
            value = copy.deepcopy(self.value)
            mutation(value["tiers"][1])
            self.rejects(value)

        value = copy.deepcopy(self.value)
        value["tiers"][3]["register_entry_threshold"] = 0
        self.rejects(value)

    def test_native_and_aot_branches_cannot_be_enabled(self):
        for field in ("native_enabled", "aot_enabled"):
            value = copy.deepcopy(self.value)
            value[field] = True
            self.rejects(value)

    def test_stale_milestone_proof_and_source_identity_are_rejected(self):
        value = copy.deepcopy(self.value)
        value["milestones"]["P035"]["evidence"][0]["sha256"] = "0" * 64
        self.rejects(value)

        self.rejects(self.value, sources={"crates/wtime-runtime/src/lib.rs": "b" * 64})

    def test_duplicate_or_unexplained_unselected_rows_are_rejected(self):
        duplicate = copy.deepcopy(self.value)
        duplicate["unselected"].append(copy.deepcopy(duplicate["unselected"][0]))
        self.rejects(duplicate)

        unexplained = copy.deepcopy(self.value)
        unexplained["unselected"][0]["reason"] = ""
        self.rejects(unexplained)

        enabled = copy.deepcopy(self.value)
        enabled["unselected"][1]["enabled"] = True
        self.rejects(enabled)


class P108RetainedExecutableTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p108-retained-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        source = self.write("crates/wtime-runtime/src/lib.rs", b"source")
        source_map = {str(source.relative_to(self.root)): hashlib.sha256(source.read_bytes()).hexdigest()}
        self.selected = {"schema": "wtime-selected-tier-check-v1", "passed": True,
                         "source_stable": True, "timing_evidence": False,
                         "configurations": copy.deepcopy(p108.TIERS), "tiers": {}, "executables": {}}
        for tier in p108.TIERS:
            name = tier["name"]
            refs = {}
            for logical in ("benchmark", "conformance", "diagnostic"):
                refs[logical] = normal(self.write(f"{name}/{logical}", (name + logical).encode()))
            report = {"passed": True, "source_stable": True, "requested_tier": tier["mode"],
                      "register_entry_threshold": tier["register_entry_threshold"],
                      "counts": {"failed": 0, "skipped": 0, "unsupported": 0,
                          **{name+'_'+suffix: count for name,count in
                             {'mode_runs':6,'state_mode_runs':6,'state_fuel_probes':68,
                              'conformance_mode_runs':4,'all_checks':89}.items()
                             for suffix in ('expected','executed')}},
                      "implementation_sources": source_map,
                      "binary": {"path": str(self.root / "shared-binary"), "sha256": refs["benchmark"]["sha256"]},
                      "conformance_binary": {"sha256": refs["conformance"]["sha256"]},
                      "diagnostic_instrumentation": {"binary_sha256": refs["diagnostic"]["sha256"]}}
            self.selected["tiers"][name] = normal(self.write(f"{name}/report.json", json.dumps(report).encode()))
            self.selected["executables"][name] = refs

    def write(self, name, data):
        path = (self.root / name).resolve()
        path.relative_to(self.root)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def check(self, selected=None):
        return p108.validate_selected_tiers(self.selected if selected is None else selected,
                                           p108.TIERS, root=self.root)

    def test_retained_executables_survive_original_cargo_path_replacement(self):
        self.write("shared-binary", b"a later Cargo configuration")
        self.assertEqual(set(self.check()), {row["name"] for row in p108.TIERS})

    def test_retained_executable_tampering_or_different_tested_digest_is_rejected(self):
        value = copy.deepcopy(self.selected)
        value["executables"]["decoded"]["benchmark"] = value["executables"]["compact"]["benchmark"]
        with self.assertRaises(VerificationError):
            self.check(value)
        self.write("decoded/benchmark", b"tampered")
        with self.assertRaises(VerificationError):
            self.check()

    def test_missing_mode_wrong_configuration_and_stale_source_are_rejected(self):
        for kind in ("missing", "configuration", "source"):
            with self.subTest(kind=kind):
                value = copy.deepcopy(self.selected)
                if kind == "missing":
                    del value["executables"]["register-eager"]
                elif kind == "configuration":
                    value["configurations"][3]["register_entry_threshold"] = 4
                else:
                    self.write("crates/wtime-runtime/src/lib.rs", b"changed source")
                with self.assertRaises(VerificationError):
                    self.check(value)

    def test_incomplete_source_maps_or_partial_and_boolean_counts_are_rejected(self):
        for mutation in ('sources','partial','boolean'):
            with self.subTest(mutation=mutation):
                value=copy.deepcopy(self.selected)
                report=json.loads(Path(value['tiers']['decoded']['path']).read_text())
                if mutation=='sources': report['implementation_sources']={}
                elif mutation=='partial': report['counts']['state_fuel_probes_executed']=0
                else: report['counts']['mode_runs_expected']=True
                value['tiers']['decoded']=normal(self.write('mutated-report.json',json.dumps(report).encode()))
                with self.assertRaises(VerificationError):
                    self.check(value)


if __name__ == "__main__":
    unittest.main()
