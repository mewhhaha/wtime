import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import integration_workloads as integration


class P113IntegrationModelTests(unittest.TestCase):
    def _observation(self, case, stage="first_call", batch=1):
        state = integration.expected(case, 32)
        lifecycle = dict(state["lifecycle"])
        if "capture_bytes_minimum" in lifecycle:
            lifecycle["capture_bytes"] = lifecycle.pop("capture_bytes_minimum")
            lifecycle["after_resume_bytes"] = lifecycle.pop("after_resume_bytes_minimum")
            collection = dict(lifecycle["collection_after_root_release"])
            collection["reclaimed"] = collection.pop("reclaimed_minimum")
            lifecycle["collection_after_root_release"] = collection
        return {
            "schema_version": 2,
            **state,
            "lifecycle": lifecycle,
            "initial_memory_hex": state["initial_memory_hex"],
            "stage": stage,
            "batch": batch,
            "ns_per_operation": 12.5,
            "mode": "compact",
            "profile": "all-phase3-core",
            "instrumented": False,
            "diagnostic": False,
        }

    def test_selected_rows_and_signed_i32_prefix_oracle(self):
        self.assertEqual(integration.SELECTED, (
            "integration_gc_prefix", "integration_eh_prefix",
            "integration_cont_prefix", "integration_numeric_control",
            "integration_ref_entry_control", "integration_cont_entry_control",
        ))
        self.assertEqual(integration.expected("integration_gc_prefix", 32)["result"],
                         {"kind": "returned", "values": ["i32:64"]})
        self.assertEqual(integration.expected("integration_numeric_control", 0x7fff_ffff)["result"],
                         {"kind": "returned", "values": ["i32:-2147483617"]})
        self.assertEqual(integration.expected("integration_eh_prefix", -32)["result"],
                         {"kind": "returned", "values": ["i32:0"]})
        self.assertEqual(integration.expected("integration_cont_prefix")["payload"],
                         {"values": [{"type": "i32", "value": 64}]})
        rows = integration.entries()
        self.assertEqual([row["id"] for row in rows], list(integration.SELECTED))
        self.assertTrue(all(row["stages"] == integration.STAGES for row in rows))
        self.assertTrue(all(row["result"]["type"] == "i32" for row in rows))
        self.assertTrue(all("sha256" not in row["source"] for row in rows))
        self.assertTrue(all("expected_state" not in row for row in rows))
        import workloads
        self.assertEqual([row['id'] for row in workloads.selection('integration')], list(integration.SELECTED))

    def test_full_memory_image_rejects_unchanged_state_and_hash_tampering(self):
        observation = self._observation("integration_gc_prefix")
        integration.validate_observation(observation, "integration_gc_prefix", "first_call")
        observation["memory_hex"] = bytes(integration.PAGE_BYTES).hex()
        with self.assertRaisesRegex(ValueError, "full memory image"):
            integration.validate_observation(observation, "integration_gc_prefix", "first_call")
        observation = self._observation("integration_gc_prefix")
        observation["memory_hex"] = "00gg"
        with self.assertRaisesRegex(ValueError, "malformed memory_hex"):
            integration.validate_observation(observation, "integration_gc_prefix", "first_call")
        observation = self._observation("integration_gc_prefix")
        observation.pop("memory_sha256")
        integration.validate_observation(observation, "integration_gc_prefix", "first_call")
        observation["memory_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "supplied memory_sha256"):
            integration.validate_observation(observation, "integration_gc_prefix", "first_call")

    def test_lifecycle_positive_byte_counts_are_variable_but_zero_is_rejected(self):
        for capture_bytes in (2, 97, 4096):
            observation = self._observation("integration_cont_prefix")
            observation["lifecycle"]["capture_bytes"] = capture_bytes
            observation["lifecycle"]["collection_after_root_release"]["reclaimed"] = 3
            integration.validate_observation(observation, "integration_cont_prefix", "first_call")
        observation = self._observation("integration_cont_prefix")
        observation["lifecycle"]["capture_bytes"] = 0
        with self.assertRaisesRegex(ValueError, "capture_bytes"):
            integration.validate_observation(observation, "integration_cont_prefix", "first_call")
        for after_resume in (0, 2, 3):
            observation = self._observation('integration_cont_prefix')
            observation['lifecycle']['after_resume_bytes'] = after_resume
            with self.assertRaisesRegex(ValueError, 'after_resume_bytes'):
                integration.validate_observation(observation, 'integration_cont_prefix', 'first_call')

    def test_production_settings_batch_and_continuation_lifecycle_are_required(self):
        observation = self._observation("integration_cont_prefix", "warm_call", batch=8)
        integration.validate_observation(observation, "integration_cont_prefix", "warm_call",
                                          expected_batch=8)
        for changed in (
            {"mode": "decoded"},
            {"instrumented": True},
            {"lifecycle": {}},
        ):
            invalid = dict(observation, **changed)
            with self.assertRaises(ValueError):
                integration.validate_observation(invalid, "integration_cont_prefix", "warm_call",
                                                  expected_batch=8)
        with self.assertRaisesRegex(ValueError, "shared paired batch"):
            integration.validate_observation(observation, "integration_cont_prefix", "warm_call")

    def test_untimed_preflight_checks_complete_state_on_its_own(self):
        observation = self._observation("integration_cont_entry_control")
        integration.validate_preflight(observation, "integration_cont_entry_control")
        observation["memory_hex"] = bytes(integration.PAGE_BYTES).hex()
        with self.assertRaisesRegex(ValueError, "full memory image"):
            integration.validate_preflight(observation, "integration_cont_entry_control")


if __name__ == "__main__":
    unittest.main()
