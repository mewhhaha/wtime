"""Fast P080 selection and typed-comparator policy checks; no guests run here."""
from __future__ import annotations

import copy
import json
import math
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools" / "wtime-bench"
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(ROOT / "tools" / "verification" / "producers"))

import p080_policy  # noqa: E402
import selection_manifest  # noqa: E402


THREAD_SELECTION = ROOT / "docs/benchmarks/performance/P080-threads-selection.json"
CONTINUATION_SELECTION = ROOT / "docs/benchmarks/performance/P080-continuations-selection.json"


def load_pair(path: Path, family: str):
    declaration = selection_manifest.load(path)
    selected_ids = {item["id"] for item in declaration["rows"] if item["family"] == family}
    return declaration, {row["id"]: row for row in declaration["rows"] if row["id"] in selected_ids}


class P080SpecialistGateTests(unittest.TestCase):
    def test_full_thread_and_continuation_families_use_only_declared_timed_stages(self):
        registry = json.loads((BENCH / "workloads.json").read_text(encoding="utf-8"))
        thread_ids = set(registry["selections"]["threads"]["workloads"])
        continuation_ids = set(registry["selections"]["continuations"]["workloads"])

        thread_decl, thread_rows = load_pair(THREAD_SELECTION, "threads")
        continuation_decl, continuation_rows = load_pair(CONTINUATION_SELECTION, "continuations")
        self.assertEqual(set(thread_rows), thread_ids)
        self.assertEqual(set(continuation_rows), continuation_ids)
        self.assertEqual(len(thread_rows), 18)
        self.assertEqual(len(continuation_rows), 7)
        self.assertTrue(all(row["stages"] == ["steady_state_group"] for row in thread_rows.values()))

        expected_timed = {
            "continuation_depth_0": ["setup", "capture", "resume"],
            "continuation_depth_4": ["setup", "capture", "resume"],
            "continuation_depth_32": ["setup", "capture", "resume"],
            "continuation_captured_reference": ["setup", "capture", "resume"],
            "continuation_bound_nested": ["setup", "bind", "resume"],
            "continuation_one_shot": ["setup", "capture", "resume_once", "resume_reuse"],
            "continuation_cross_module_consumer": ["setup", "capture", "resume"],
        }
        self.assertEqual({key: value["stages"] for key, value in continuation_rows.items()}, expected_timed)
        registry_rows = {row["id"]: row for row in registry["workloads"]}
        untimed = {
            "continuation_depth_0": {"accounting"},
            "continuation_depth_4": {"accounting"},
            "continuation_depth_32": {"accounting"},
            "continuation_captured_reference": {"accounting"},
            "continuation_bound_nested": {"accounting"},
            "continuation_one_shot": {"cleanup"},
            "continuation_cross_module_consumer": {"nominal_context"},
        }
        for key, excluded in untimed.items():
            self.assertTrue(excluded.isdisjoint(continuation_rows[key]["stages"]))
            self.assertTrue(excluded.issubset(registry_rows[key]["stages"]))
        self.assertEqual(thread_decl["task"], continuation_decl["task"])

    def test_target_roles_and_protected_controls_are_explicit_in_both_selections(self):
        for path, family, small, large, held_out in (
            (THREAD_SELECTION, "threads", "threads_atomic_add_same_w1_n32",
             "threads_atomic_add_same_w4_n1024", "threads_ordinary_rw_n32"),
            (CONTINUATION_SELECTION, "continuations", "continuation_depth_4",
             "continuation_depth_32", "continuation_captured_reference"),
        ):
            declaration = selection_manifest.load(path)
            rows = {row["id"]: row for row in declaration["rows"]}
            self.assertEqual(rows[small]["role"], "targeted")
            self.assertEqual(rows[small]["scale"], "small")
            self.assertEqual(rows[large]["role"], "targeted")
            self.assertEqual(rows[large]["scale"], "large")
            self.assertEqual(rows[held_out]["role"], "held_out")
            self.assertEqual(rows[held_out]["scale"], "other")
            self.assertEqual(sum(row["role"] == "held_out" for row in rows.values()), 1)
            self.assertEqual(sum(row["family"] == family for row in declaration["rows"]),
                             18 if family == "threads" else 7)
            by_id = {row["id"]: row for row in declaration["rows"]}
            self.assertEqual(by_id["call_overhead"]["role"], "protected")
            self.assertEqual(set(by_id["call_overhead"]["stages"]), {"first_call", "load_and_call"})
            self.assertEqual(by_id["prepare_large"]["role"], "protected")
            self.assertEqual(by_id["prepare_large"]["stages"], ["prepare"])
        policy_cases = (
            "test_full_thread_and_continuation_families_use_only_declared_timed_stages",
            "test_target_roles_and_protected_controls_are_explicit_in_both_selections",
            "test_unavailable_wasmtime_comparators_never_claim_ratios",
        )
        self.assertEqual(selection_manifest.load(THREAD_SELECTION)["semantic_cases"], list(policy_cases))
        self.assertEqual(selection_manifest.load(CONTINUATION_SELECTION)["semantic_cases"], list(policy_cases))

    def test_unavailable_wasmtime_comparators_never_claim_ratios(self):
        rows = [
            {"family": family, "engine": "wasmtime", "status": "unsupported",
             "reason": "proposal is unavailable in this comparator build", "ratio": None}
            for family in sorted(p080_policy.COMPARATOR_FAMILIES)
        ]
        self.assertEqual(len(p080_policy.validate_comparators(rows)), 4)
        for changed in (1.0, True, math.inf):
            malformed = copy.deepcopy(rows)
            malformed[0]["ratio"] = changed
            with self.subTest(ratio=changed), self.assertRaises(ValueError):
                p080_policy.validate_comparators(malformed)
        incomplete = copy.deepcopy(rows[:-1])
        with self.assertRaises(ValueError):
            p080_policy.validate_comparators(incomplete)

        compared = copy.deepcopy(rows)
        compared[0] = {
            "family": compared[0]["family"], "engine": "wasmtime", "status": "compared",
            "reason": "comparison completed", "ratio": 0.98,
            "artifact_refs": [{"path": "evidence/comparison.json", "sha256": "a" * 64}],
        }
        self.assertEqual(p080_policy.validate_comparators(compared)[0]["ratio"], 0.98)
        compared[0]["artifact_refs"] = []
        with self.assertRaises(ValueError):
            p080_policy.validate_comparators(compared)


if __name__ == "__main__":
    unittest.main()
