"""Contained contract tests for the final release evidence aggregator."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "tools/verification/producers"))
import p112
from verification.artifacts import ArtifactStore, normal


def put_json(path: Path, value: dict) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")
    return path


def make_case(base: Path, *, task="P108", synthetic=False, stale_ref=False,
              wrong_task=False, candidate_promoted=False):
    root = (base / "repo").resolve()
    (root / "crates/core/src").mkdir(parents=True)
    (root / "Cargo.toml").write_text("workspace\n")
    (root / "Cargo.lock").write_text("lock\n")
    (root / "rust-toolchain.toml").write_text("channel = 'stable'\n")
    (root / "crates/core/Cargo.toml").write_text("[package]\n")
    (root / "crates/core/src/lib.rs").write_text("pub fn core() {}\n")
    sources = p112.source_identity(root)
    outcome = "adopted" if task == "P110" else "contract_only"
    evidence_dir = root / f"target/performance/{task}/shared-finalization/attempt-0001"
    evidence_dir.mkdir(parents=True)
    policy_path = put_json(root / "docs/benchmarks/performance/decisions.json", {
        "schema": "wtime-performance-portfolio-v1", "task": task, "status": "passed",
        "timing_evidence": False, "default": "decoded", "tiers": p112.p108.TIERS,
        "native_enabled": False, "aot_enabled": False, "cancellation_policy": "exact_source_events",
        "default_external_dependencies": 0, "default_unsafe": False,
        "milestones": {name: {"status": "done", "outcome": "contract_only",
                               "evidence": [normal(put_json(root / f"target/{name}-evidence.json", {"proof": name}))]}
                        for name in p112.p108.MILESTONES},
        "retained_optimizations": p112.p108.RETAINED, "unselected": [], "sources": sources,
        "limitations": ["synthetic test policy"]})
    receipt_public = put_json(root / "target/verification-receipt.json", {
        "task": task, "status": "passed", "lane": "full", "synthetic": False})
    selected_tiers = {}
    for tier in p112.p108.TIERS:
        selected_tiers[tier["name"]] = normal(put_json(root / f"target/selected/{tier['name']}.json", {"tier": tier["name"]}))
    if task == "P108":
        proof = {"schema": "wtime-p108-portfolio-v1", "task": task, "status": "passed",
                 "timing_evidence": False, "policy": normal(policy_path), "selected_tiers": selected_tiers,
                 "verification_receipt": normal(receipt_public), "sources": sources,
                 "limitations": ["synthetic test proof"]}
    else:
        proof = {"schema": "wtime-p110-final-budgets-v1", "task": task, "status": "passed",
                 "timing_evidence": True, "outcome": outcome, "hard_gate_passed": True,
                 "core_sources": sources}
    proof_path = put_json(root / f"docs/benchmarks/performance/{task}-proof.json", proof)
    store_root = root / "target/verification"
    store = ArtifactStore(store_root)
    receipt_raw = put_json(root / "target/receipt.json", {
        "task": task, "status": "passed", "lane": "full", "synthetic": False,
        "original_outputs": {"proof": normal(proof_path)}})
    receipt_ref = store.add(receipt_raw)
    evidence = {"schema": "wtime-verification-evidence-v1", "task": task, "status": "passed",
                "passed": True, "synthetic": synthetic, "outcome": outcome,
                "candidate_promoted": candidate_promoted,
                "store": str(store_root), "phases": {"analyze": receipt_ref}}
    evidence_path = put_json(evidence_dir / "evidence.json", evidence)
    exported_evidence = root / f"docs/benchmarks/performance/{task}-evidence.json"
    exported_evidence.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(evidence_path, exported_evidence)
    evidence_ref = normal(evidence_path)
    completion_path = put_json(root / f"docs/benchmarks/performance/{task}-completion.json", {
        "task": task if not wrong_task else "P109", "status": "done", "outcome": outcome,
        "evidence": [evidence_ref], "artifact_hashes": {
            str(evidence_path.resolve()): "0" * 64 if stale_ref else evidence_ref["sha256"]}})
    return root, sources, completion_path, evidence_path, proof_path


def make_final_case(base: Path, *, wrong_default=False, malformed_stats=False):
    root = (base / "repo").resolve()
    (root / "crates/core/src").mkdir(parents=True)
    (root / "Cargo.toml").write_text("workspace\n")
    (root / "Cargo.lock").write_text("lock\n")
    (root / "rust-toolchain.toml").write_text("channel = 'stable'\n")
    (root / "crates/core/Cargo.toml").write_text("[package]\n")
    (root / "crates/core/src/lib.rs").write_text("pub fn core() {}\n")
    sources = p112.source_identity(root)
    docs = root / "docs/benchmarks/performance"
    store_root = root / "target/verification"
    store = ArtifactStore(store_root)

    def ref_json(relative, value):
        return normal(put_json(root / relative, value))

    def make_milestone(task, proof, outcome, candidate_promoted=False):
        proof_path = put_json(docs / f"{task}-proof.json", proof)
        phase_receipt = put_json(root / f"target/{task}-analyze-receipt.json", {
            "task": task, "status": "passed", "lane": "full", "synthetic": False,
            "original_outputs": {"proof": normal(proof_path)}})
        phase_ref = store.add(phase_receipt)
        evidence = {"schema": "wtime-verification-evidence-v1", "task": task,
                    "status": "passed", "passed": True, "synthetic": False,
                    "outcome": outcome, "candidate_promoted": candidate_promoted,
                    "store": str(store_root), "phases": {"analyze": phase_ref}}
        evidence_path = put_json(root / f"target/performance/{task}/shared-finalization/attempt-0001/evidence.json", evidence)
        exported = put_json(docs / f"{task}-evidence.json", evidence)
        completion = {"task": task, "status": "done", "outcome": outcome,
                     "evidence": [normal(evidence_path)],
                     "artifact_hashes": {str(evidence_path.resolve()): normal(evidence_path)["sha256"]}}
        put_json(docs / f"{task}-completion.json", completion)
        if evidence_path.read_bytes() != exported.read_bytes():
            raise AssertionError("synthetic evidence export differs from retained evidence")

    policy = {"schema": "wtime-performance-portfolio-v1", "task": "P108", "status": "passed",
              "timing_evidence": False, "default": "compact" if wrong_default else "decoded",
              "tiers": p112.p108.TIERS, "native_enabled": False, "aot_enabled": False,
              "cancellation_policy": "exact_source_events", "default_external_dependencies": 0,
              "default_unsafe": False,
              "milestones": {name: {"status": "done", "outcome": "contract_only",
                                    "evidence": [ref_json(f"target/{name}-milestone-evidence.json", {"task": name})]}
                             for name in p112.p108.MILESTONES},
              "retained_optimizations": p112.p108.RETAINED, "unselected": [], "sources": sources,
              "limitations": ["synthetic policy fixture"]}
    policy_ref = ref_json("docs/benchmarks/performance/decisions.json", policy)
    p108_receipt_ref = ref_json("target/P108-verification-receipt.json", {
        "task": "P108", "status": "passed", "lane": "full", "synthetic": False})
    tier_refs = {row["name"]: ref_json(f"target/P108-selected/{row['name']}.json", {"passed": True})
                 for row in p112.p108.TIERS}
    p108_proof = {"schema": "wtime-p108-portfolio-v1", "task": "P108", "status": "passed",
                  "timing_evidence": False, "policy": policy_ref, "selected_tiers": tier_refs,
                  "verification_receipt": p108_receipt_ref, "sources": sources,
                  "limitations": ["synthetic release fixture"]}
    make_milestone("P108", p108_proof, "contract_only")

    p109_receipt_ref = ref_json("target/P109-verification-receipt.json", {
        "task": "P109", "status": "passed", "lane": "full", "synthetic": False})
    p109_proof = {"schema": "wtime-p109-selected-conformance-v1", "task": "P109",
                  "status": "passed", "timing_evidence": False,
                  "core_sources": sources, "verification_receipt": p109_receipt_ref}
    make_milestone("P109", p109_proof, "contract_only")

    artifact_payload = put_json(root / "target/P110-artifact-payload.json", {"archived": True})
    artifact_manifest = put_json(root / "target/P110-artifacts.json", {
        "payload": store.add(artifact_payload)})
    cold_report_refs = {
        "default-vs-p001": ref_json("target/P110-cold/default-report.json", {
            "passed": True, "source_stable": True, "selection": "original-mvp"}),
        "compact-vs-decoded": ref_json("target/P110-cold/compact-report.json", {
            "passed": True, "source_stable": True, "selection": "original-mvp"}),
    }
    cold_collection_ref = ref_json("target/P110-cold/report.json", {
        "schema": "wtime-p110-cold-collection-v1", "task": "P110", "status": "passed",
        "source_stable": True, "timing_evidence": True, "reports": cold_report_refs,
        "source_inputs": sources, "artifacts": normal(artifact_manifest)})
    footprints_ref = ref_json("target/P110-footprints.json", {"schema": "wtime-p110-footprints-v1", "status": "passed"})
    build_report_refs = {}
    command_refs = {}
    for name, mode in (("default", "default"), ("register", "optional-feature"), ("compact", "optional-feature")):
        build_report_refs[name] = ref_json(f"target/P110-builds/{name}.json", {
            "mode": mode, "source_stable": True, "budget_pass": True})
        command_refs[name] = ref_json(f"target/P110-builds/{name}-command.json", {"exit_code": 0})
    dependency_ref = normal(put_json(root / "target/P110-dependency-report.json", {"passed": True}))
    builds = {"schema": "wtime-p110-build-budgets-v1", "status": "passed",
              "source_inputs_current": True, "dependency_budget": {"status": "passed"},
              "retained_reports": build_report_refs, "retained_commands": command_refs,
              "retained_dependency_report": dependency_ref}
    stats_refs = {}
    for name in ("default-vs-p001", "compact-vs-decoded", "eager-vs-decoded", "adaptive-vs-decoded"):
        stats_value = {"schema_version": 1, "analysis": {"bootstrap_resamples": 10000}, "per_run": []}
        if name in ("eager-vs-decoded", "adaptive-vs-decoded"):
            stats_value = {"decision": "reused P067 proof", "analysis": stats_value}
        if malformed_stats and name == "adaptive-vs-decoded":
            stats_value = {"decision": "reused P067 proof", "analysis": {"unexpected": True}}
        stats_refs[name] = ref_json(f"target/P110-statistics/{name}.json", stats_value)
    p110_proof = {"schema": "wtime-p110-final-budgets-v1", "task": "P110", "status": "passed",
                  "hard_gate_passed": True, "outcome": "adopted", "timing_evidence": True,
                  "core_sources": sources, "policy": policy_ref, "footprints": footprints_ref,
                  "cold_collection": cold_collection_ref, "builds": builds,
                  "statistics": stats_refs,
                  "cold_reviews": {name: {"requires_review": False} for name in stats_refs}}
    make_milestone("P110", p110_proof, "adopted", candidate_promoted=True)

    p111_receipt_ref = ref_json("target/P111-verification-receipt.json", {
        "task": "P111", "status": "passed", "lane": "full", "synthetic": False})
    smoke_ref = ref_json("target/P111-smoke.json", {"status": "passed", "nonzero_invocations": 3})
    reference_ref = ref_json("target/P111-reference.json", {"status": "unrun", "decision": "unrun"})
    p111_proof = {"schema": "wtime-p111-ci-v1", "task": "P111", "status": "passed",
                  "timing_evidence": False, "core_sources": sources,
                  "verification_receipt": p111_receipt_ref, "smoke": smoke_ref,
                  "reference": reference_ref}
    make_milestone("P111", p111_proof, "contract_only")
    return root, sources


class P112ContractTests(unittest.TestCase):
    def test_complete_current_nonsynthetic_milestone_is_bound(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            root, sources, _, _, _ = make_case(Path(td))
            result = p112._validate_completion(root, "P108", sources)
            self.assertEqual(result["task"], "P108")
            self.assertEqual(result["proof_schema"], "wtime-p108-portfolio-v1")
            self.assertEqual(result["proof"]["sha256"], hashlib.sha256(
                (root / "docs/benchmarks/performance/P108-proof.json").read_bytes()).hexdigest())
            policy = p112._validate_portfolio(root, result["proof"], sources)
            self.assertEqual(policy["default"], "decoded")
            self.assertFalse(policy["native_enabled"] or policy["aot_enabled"])

    def test_synthetic_proof_evidence_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            root, sources, *_ = make_case(Path(td), synthetic=True)
            with self.assertRaisesRegex(ValueError, "synthetic, incomplete"):
                p112._validate_completion(root, "P108", sources)

    def test_wrong_task_and_stale_completion_hash_fail_closed(self):
        for option in ({"wrong_task": True}, {"stale_ref": True}):
            with self.subTest(option=option), tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
                root, sources, *_ = make_case(Path(td), **option)
                with self.assertRaises(ValueError):
                    p112._validate_completion(root, "P108", sources)

    def test_current_source_drift_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            root, sources, *_ = make_case(Path(td))
            (root / "crates/core/src/lib.rs").write_text("pub fn changed() {}\n")
            sources = p112.source_identity(root)
            with self.assertRaisesRegex(ValueError, "stale against current core sources"):
                p112._validate_completion(root, "P108", sources)

    def test_p110_adopted_budget_can_mark_candidate_promoted_without_promoting_a_tier(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            root, sources, *_ = make_case(Path(td), task="P110", candidate_promoted=True)
            result = p112._validate_completion(root, "P110", sources)
            self.assertEqual(result["task"], "P110")

    def test_finalize_writes_report_appendix_and_schema_for_all_four_milestones(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            base = Path(td).resolve()
            root, sources = make_final_case(base)
            output = root / "target/recovery/P112-report"
            report = p112.finalize(root, output)
            self.assertTrue(output.is_relative_to(root))
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["current_core_sources"], sources)
            self.assertEqual(set(report["milestones"]), {"P108", "P109", "P110", "P111"})
            self.assertEqual(report["policy"]["default"], "decoded")
            self.assertIn("T166 and T169 remains unrun", " ".join(report["limitations"]))
            self.assertTrue((output / "report.json").is_file())
            self.assertIn("Historical release proof", (output / "release-appendix.md").read_text())
            self.assertEqual(json.loads((output / "release-schema.json").read_text())["$id"],
                             "wtime-p112-release-report-v1")

    def test_finalize_rejects_malformed_nested_p067_statistics(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            base = Path(td).resolve()
            root, _ = make_final_case(base, malformed_stats=True)
            with self.assertRaisesRegex(ValueError, "statistics artifact is malformed"):
                p112.finalize(root, root / "target/recovery/P112-report")

    def test_finalize_rejects_wrong_default_policy(self):
        with tempfile.TemporaryDirectory(prefix="p112-", dir="/tmp") as td:
            base = Path(td).resolve()
            root, sources = make_final_case(base, wrong_default=True)
            milestones = {task: p112._validate_completion(root, task, sources) for task in p112.TASKS}
            with self.assertRaisesRegex(ValueError, "unaccepted default policy"):
                p112._validate_portfolio(root, milestones["P108"]["proof"], sources)


if __name__ == "__main__":
    unittest.main()
