"""Integrity checks for the native no-go and its provisional future limits."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools" / "verification" / "producers"))
import p094_policy  # noqa: E402


POLICY_PATH = ROOT / "docs/architecture/native-policy-skeleton.json"
ADR_PATH = ROOT / "docs/architecture/performance-native-adr.md"


def policy_document():
    return json.loads(POLICY_PATH.read_text(encoding="utf-8"))


class NativePolicyTests(unittest.TestCase):
    def test_no_go_keeps_decoded_default_and_native_aot_disabled(self):
        policy = p094_policy.validate_policy(policy_document())
        self.assertEqual(policy["decision"], "no_go")
        self.assertEqual(policy["native_status"], "not_selected")
        self.assertIs(policy["native_enabled"], False)
        self.assertIs(policy["aot_enabled"], False)
        self.assertEqual(policy["default_policy"]["engine"], "decoded")
        self.assertEqual(policy["default_policy"]["external_dependencies"], 0)
        self.assertIs(policy["default_policy"]["unsafe"], False)
        self.assertEqual(policy["native_measurements"], "none")
        adr = ADR_PATH.read_text(encoding="utf-8")
        self.assertIn("Decision: `no_go`", adr)
        self.assertIn("Native implementation: none. Native measurements: none.", adr)
        self.assertIn("The source-bound P094 proof", adr)
        self.assertIn("../benchmarks/performance/P067-proof.json", adr)
        self.assertIn("../benchmarks/performance/P080-proof.json", adr)

    def test_no_go_defers_descendants_and_rejects_dependency_unsafe_or_cycle_drift(self):
        baseline = policy_document()
        descendants = {f"P{i:03d}": "deferred_not_selected" for i in range(95, 108)}
        self.assertEqual(p094_policy.validate_policy(baseline)["descendants"], descendants)
        mutations = []

        enabled_child = copy.deepcopy(baseline)
        enabled_child["descendants"]["P095"] = "enabled"
        mutations.append(enabled_child)

        default_external = copy.deepcopy(baseline)
        default_external["default_policy"]["external_dependencies"] = 1
        mutations.append(default_external)

        active_scanner_exception = copy.deepcopy(baseline)
        active_scanner_exception["unsafe_policy"]["scanner_exception_activated"] = True
        mutations.append(active_scanner_exception)

        unsafe_default = copy.deepcopy(baseline)
        unsafe_default["default_policy"]["unsafe"] = True
        mutations.append(unsafe_default)

        cycle = copy.deepcopy(baseline)
        cycle["dependency_policy"]["optional_crates"][0]["depends_on"] = ["wtime-native"]
        mutations.append(cycle)

        native_external = copy.deepcopy(baseline)
        native_external["dependency_policy"]["optional_crates"][1]["external_dependencies"] = 1
        mutations.append(native_external)

        for candidate in mutations:
            with self.subTest(candidate=candidate):
                with self.assertRaises(ValueError):
                    p094_policy.validate_policy(candidate)

    def test_reopen_policy_requires_finite_code_wx_abi_and_transactional_bounds(self):
        baseline = policy_document()
        self.assertIs(p094_policy.validate_policy(baseline)["provisional_resource_ceilings"]["measured"], False)
        mutations = []

        unbounded_code = copy.deepcopy(baseline)
        unbounded_code["provisional_resource_ceilings"]["maximum_module_code_bytes"] = None
        mutations.append(unbounded_code)

        unbounded_mapping = copy.deepcopy(baseline)
        unbounded_mapping["mapping_policy"]["maximum_bytes"] = None
        mutations.append(unbounded_mapping)

        wx = copy.deepcopy(baseline)
        wx["mapping_policy"]["writable_and_executable_simultaneously"] = True
        mutations.append(wx)

        abi = copy.deepcopy(baseline)
        abi["abi_policy"]["stack_alignment_bytes"] = None
        mutations.append(abi)

        accounting = copy.deepcopy(baseline)
        accounting["provisional_resource_ceilings"]["checked_arithmetic"] = False
        mutations.append(accounting)

        rollback = copy.deepcopy(baseline)
        rollback["provisional_resource_ceilings"]["budget_exhaustion_is_transactional"] = False
        mutations.append(rollback)

        async_compile = copy.deepcopy(baseline)
        async_compile["provisional_resource_ceilings"]["worker_pool"] = True
        mutations.append(async_compile)

        for candidate in mutations:
            with self.subTest(candidate=candidate):
                with self.assertRaises(ValueError):
                    p094_policy.validate_policy(candidate)


if __name__ == "__main__":
    unittest.main()
