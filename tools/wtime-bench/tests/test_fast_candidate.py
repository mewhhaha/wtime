"""Focused calibration and candidate decision checks using synthetic reports."""
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
REPO = TOOL.parents[1]
VERIFY_TOOLS = REPO / "tools"
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))
sys.path.insert(0, str(VERIFY_TOOLS))

import candidate_gate
import cold_products
import focused_compare
from verification.artifacts import ArtifactStore, digest
from verification.runner import test_results


class FastCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-fast-candidate-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    def test_calibration_reaches_twenty_ms_on_both_sides_at_shared_batch(self):
        calls = []

        def observe(side, batch):
            calls.append((side, batch))
            return {"batch": batch, "ns_per_operation": 10_000 if side == "baseline" else 20_000}

        batch, audit = focused_compare.calibrate({"oracle": {"module": "scalar_workloads.py"}}, observe)

        final = audit[-1]["observations"]
        self.assertEqual(final["baseline"]["batch"], batch)
        self.assertEqual(final["candidate"]["batch"], batch)
        self.assertGreaterEqual(final["baseline"]["ns_per_operation"] * batch, 20_000_000)
        self.assertGreaterEqual(final["candidate"]["ns_per_operation"] * batch, 20_000_000)
        self.assertEqual(calls[-2:], [("baseline", batch), ("candidate", batch)])

    def test_calibration_fails_when_workload_cap_cannot_reach_target(self):
        def observe(_side, batch):
            return {"batch": batch, "ns_per_operation": 1}

        with self.assertRaisesRegex(ValueError, "cannot reach 20 ms"):
            focused_compare.calibrate({"oracle": {"module": "scalar_workloads.py"}}, observe)

    def test_calibration_rejects_a_side_that_changes_the_requested_batch(self):
        def observe(side, batch):
            return {"batch": batch if side == "baseline" else batch + 1,
                    "ns_per_operation": 30_000_000}

        with self.assertRaisesRegex(ValueError, "batch mismatch"):
            focused_compare.calibrate({"oracle": {"module": "scalar_workloads.py"}}, observe)

    def test_target_warm_and_cold_stages_keep_warm_as_primary(self):
        report = self.make_report()
        for row in report['declaration']['rows']:
            if row['role'] == 'targeted':
                row['stages'] = ['prepare', 'first_call', 'warm_call', 'load_and_call']
        result = candidate_gate.decision(report)
        self.assertEqual(result['primary'][0]['stages'],
                         {'large_target': 'warm_call', 'small_target': 'warm_call'})

    def make_report(self, overrides=None):
        overrides = overrides or {}
        rows = [
            {"id": "small_target", "role": "targeted", "stages": ["warm_call"]},
            {"id": "large_target", "role": "targeted", "stages": ["warm_call"]},
            {"id": "held_out_control", "role": "held_out"},
            {"id": "protected_tiny", "role": "protected"},
            {"id": "prepare_large", "role": "protected"},
            {"id": "prepare_large_alias", "role": "protected"},
        ]
        specs = [
            ("small_target", "warm_call"),
            ("large_target", "warm_call"),
            ("held_out_control", "first_call"),
            ("protected_tiny", "warm_call"),
            ("prepare_large", "prepare"),
            ("prepare_large_alias", "prepare"),
        ]
        settings = {"profile": "release", "instrumented": False, "fuel": False,
                    "cancellation_quantum": 1}
        products = []
        for workload, stage in specs:
            shared_prepare = stage == "prepare"
            products.append({
                "workload": workload,
                "stage": stage,
                "wasm_sha256": "a" * 64 if shared_prepare else ("b" * 62 + format(len(workload) % 256, "02x")),
                "profile": "release",
                "runtime_settings": settings,
                "timer": {"driver": "synthetic-driver", "source_sha256": "c" * 64},
                "setup": ({"recipe": "same-canonical-preflight"} if shared_prepare
                          else {"recipe": workload}),
            })
        product_plan = cold_products.plan(products)
        raw_counts = {f"{group['sample_workload']}\0{group['stage']}": 60
                      for group in product_plan["groups"]}
        report = {
            "schema_version": 2,
            "passed": True,
            "source_stable": True,
            "metadata": {"settings": settings},
            "declaration": {"rows": rows},
            "cold_products": product_plan,
            "counts": cold_products.summarize(product_plan, raw_counts),
            "runs": [],
        }
        for run_id, seed in enumerate((31, 97)):
            pairs = []
            for workload, stage in specs:
                if workload == "prepare_large_alias":
                    # Aliases share the canonical sample's raw pairs and have no duplicate rows.
                    continue
                ratio_values = overrides.get((workload, stage))
                for sample in range(30):
                    if ratio_values is None:
                        ratio = 0.90 if workload in ("small_target", "large_target") else 1.0
                    elif isinstance(ratio_values, list):
                        ratio = ratio_values[sample % len(ratio_values)]
                    else:
                        ratio = ratio_values
                    expected = 7
                    pairs.append({
                        "pair_id": f"{run_id}-{workload}-{stage}-{sample}",
                        "workload": workload,
                        "stage": stage,
                        "input": 0,
                        "wasm_sha256": "a" * 64,
                        "order": ["baseline", "candidate"] if sample % 2 == 0
                                 else ["candidate", "baseline"],
                        "expected": expected,
                        "baseline": {"ns_per_operation": 100.0, "batch": 1, "result": expected},
                        "candidate": {"ns_per_operation": 100.0 * ratio, "batch": 1, "result": expected},
                    })
            report["runs"].append({"run_id": run_id, "seed": seed, "pairs": pairs})
        return report

    def test_candidate_decision_adopts_supported_small_and_large_target_win(self):
        result = candidate_gate.decision(self.make_report())

        self.assertEqual(result["decision"], "adopted")
        self.assertEqual(len(result["primary"]), 2)
        self.assertTrue(all(row["median"] < 0.95 and row["upper"] < 1 for row in result["primary"]))

    def test_protected_tiny_supported_regression_rejects_candidate(self):
        result = candidate_gate.decision(self.make_report({("protected_tiny", "warm_call"): 1.10}))

        self.assertEqual(result["decision"], "rejected")
        self.assertTrue(any("protected_tiny/warm_call: supported regression" in reason
                            for reason in result["reasons"]))

    def test_large_target_regression_cannot_be_masked_by_small_target_win(self):
        result = candidate_gate.decision(self.make_report({
            ("small_target", "warm_call"): 0.10,
            ("large_target", "warm_call"): 2.0,
        }))

        self.assertAlmostEqual(result["primary"][0]["median"], 0.2 ** 0.5)
        self.assertEqual(result["decision"], "rejected")
        self.assertTrue(any("large_target/warm_call: supported regression" in reason
                            for reason in result["reasons"]))

    def test_crossing_protected_tiny_regression_interval_is_inconclusive(self):
        ratios = [1.01 + index * 0.08 / 29 for index in range(30)]
        result = candidate_gate.decision(
            self.make_report({("protected_tiny", "warm_call"): ratios}))

        self.assertEqual(result["decision"], "inconclusive")
        self.assertTrue(any("protected_tiny/warm_call: regression bound is inconclusive" in reason
                            for reason in result["reasons"]))

    def test_aliased_cold_pairs_are_counted_once_without_alias_pair_rows(self):
        report = self.make_report()
        cold_group = next(group for group in report["cold_products"]["groups"]
                          if group["stage"] == "prepare")

        self.assertEqual(cold_group["members"], ["prepare_large", "prepare_large_alias"])
        self.assertEqual(report["counts"]["unique_pair_count"], 5 * 60)
        self.assertEqual(report["counts"]["logical_pair_references"], 6 * 60)
        for run in report["runs"]:
            self.assertEqual(sum(pair["workload"] == "prepare_large" and pair["stage"] == "prepare"
                                 for pair in run["pairs"]), 30)
            self.assertFalse(any(pair["workload"] == "prepare_large_alias" for pair in run["pairs"]))
        result = candidate_gate.decision(report)
        self.assertEqual(result["sample_counts"]["unique_pair_count"], 5 * 60)
        self.assertEqual(len(result["analysis"]["per_run"]), 10)


class CandidateSemanticReceiptTests(unittest.TestCase):
    CASES = ["semantic_scalar_result", "semantic_fuel_prefix", "semantic_trap_priority"]
    LOG = b"""running 3 tests
test runtime::semantic_scalar_result ... ok
test runtime::semantic_fuel_prefix ... ok
test runtime::semantic_trap_priority ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-fast-candidate-receipt-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.store_root = self.inside("target/verification")
        self.store = ArtifactStore(self.store_root)
        self.source_hashes = {
            "Cargo.toml": "a" * 64,
            "crates/wtime-runtime/src/lib.rs": "b" * 64,
        }
        self.current_sources = dict(self.source_hashes)
        self.declaration = {"task": "P047", "semantic_cases": list(self.CASES)}
        self.receipt_path = self.inside("receipt.json")
        self.receipt = self.make_receipt()
        self.write_json(self.receipt_path, self.receipt)

    def tearDown(self):
        self.temporary.cleanup()

    def inside(self, relative: str) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic output escaped fresh temp root")
        return path

    def write_json(self, path: Path, value):
        self.assertTrue(path.resolve().is_relative_to(self.root), "synthetic output escaped fresh temp root")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

    def store_log(self, name: str, content: bytes):
        source = self.inside(f"logs/{name}")
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_bytes(content)
        return self.store.add(source)

    def make_receipt(self):
        group = {"command": "runtime-tests", "kind": "cargo",
                 "required_names": list(self.CASES), "minimum_passed": 3, "features": []}
        stdout = self.store_log("stdout.log", self.LOG)
        stderr = self.store_log("stderr.log", b"")
        actual = test_results(self.LOG.decode(), group)
        inputs = {
            "components": {"runtime": {"files": dict(self.source_hashes)}},
            "definition": {"tests": [group]},
        }
        return {
            "schema": "wtime-verification-phase-v1",
            "task": "P047",
            "phase": "verify",
            "status": "passed",
            "lane": "full",
            "synthetic": False,
            "inputs": inputs,
            "input_key": digest(inputs),
            "artifacts": {"runtime-tests.stdout": stdout, "runtime-tests.stderr": stderr},
            "commands": [{"name": "runtime-tests", "exit_code": 0,
                           "stdout": stdout, "stderr": stderr}],
            "tests": {"runtime-tests": actual},
        }

    def validate(self):
        with mock.patch.object(focused_compare, "ROOT", self.root):
            return candidate_gate.validate_semantics(self.receipt_path, self.declaration,
                                                      self.current_sources)

    def save_receipt(self, receipt):
        self.write_json(self.receipt_path, receipt)

    def test_valid_full_receipt_parses_real_stored_test_logs(self):
        result = self.validate()

        self.assertEqual(result, {"status": "passed", "required_cases": self.CASES})

    def test_missing_required_semantic_case_is_rejected(self):
        declaration = {"task": "P047", "semantic_cases": self.CASES + ["missing_case"]}
        with mock.patch.object(focused_compare, "ROOT", self.root):
            with self.assertRaisesRegex(ValueError, "required semantic cases did not execute"):
                candidate_gate.validate_semantics(self.receipt_path, declaration, self.current_sources)

    def test_stale_core_source_hash_is_rejected(self):
        stale = dict(self.current_sources, **{"crates/wtime-runtime/src/lib.rs": "c" * 64})
        with mock.patch.object(focused_compare, "ROOT", self.root):
            with self.assertRaisesRegex(ValueError, "different or incompletely bound"):
                candidate_gate.validate_semantics(self.receipt_path, self.declaration, stale)

    def test_development_and_synthetic_receipts_are_rejected(self):
        for changes in ({"lane": "development"}, {"synthetic": True}):
            with self.subTest(changes=changes):
                self.save_receipt({**self.receipt, **changes})
                with mock.patch.object(focused_compare, "ROOT", self.root):
                    with self.assertRaisesRegex(ValueError, "successful full semantic verification receipt"):
                        candidate_gate.validate_semantics(self.receipt_path, self.declaration,
                                                          self.current_sources)
        self.save_receipt(self.receipt)

    def test_altered_stored_log_artifact_is_rejected(self):
        reference = self.receipt["artifacts"]["runtime-tests.stdout"]
        artifact = (self.store_root / reference["path"]).resolve()
        self.assertTrue(artifact.is_relative_to(self.root), "stored artifact escaped fresh temp root")
        damaged = bytearray(self.LOG)
        damaged[0] ^= 1
        artifact.chmod(0o644)
        artifact.write_bytes(damaged)
        artifact.chmod(0o444)

        with mock.patch.object(focused_compare, "ROOT", self.root):
            with self.assertRaisesRegex(ValueError, "stored artifact changed"):
                candidate_gate.validate_semantics(self.receipt_path, self.declaration,
                                                  self.current_sources)

    def test_candidate_report_rejects_development_lane_before_other_dependencies(self):
        report_path = self.inside("development-report.json")
        self.write_json(report_path, {"schema": focused_compare.SCHEMA, "lane": "development"})

        with self.assertRaisesRegex(ValueError, "only full candidate evidence"):
            candidate_gate.validate(report_path)


if __name__ == "__main__":
    unittest.main()
