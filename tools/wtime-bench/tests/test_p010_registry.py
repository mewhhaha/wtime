"""Draft regression checks for ready-selection and full registry validation."""
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

BENCH_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH_ROOT))
import simd_scaling
import simd_workloads
import workloads


class P010RegistryReadyDraft(unittest.TestCase):
    def test_simd_selection_is_ready_and_contains_the_complete_model(self):
        registry = workloads.load_registry()
        selected = workloads.selection("simd", registry)
        model = simd_workloads.entries()
        self.assertEqual(registry["selections"]["simd"]["status"], "ready")
        self.assertEqual(len(selected), 114)
        self.assertEqual([row["id"] for row in selected], [row["id"] for row in model])
        self.assertEqual(selected, model)

    def test_runner_rejects_pending_selection_even_when_ids_are_populated(self):
        registry = workloads.load_registry()
        registry["selections"]["simd"] = {
            "status": "pending",
            "owner_task": "P010",
            "workloads": [row["id"] for row in simd_workloads.entries()],
        }
        with tempfile.TemporaryDirectory() as temp:
            registry_path = Path(temp) / "workloads.json"
            registry_path.write_text(json.dumps(registry))
            with self.assertRaises((workloads.WorkloadError, simd_scaling.MeasurementError)):
                simd_scaling._selected_entries(
                    simd_workloads, registry_path, "simd"
                )

    def test_runner_rejects_incomplete_ready_selection(self):
        registry = copy.deepcopy(workloads.load_registry())
        registry["selections"]["simd"]["workloads"] = registry["selections"]["simd"]["workloads"][:-1]
        workloads.validate_registry(registry)  # Generic schema permits explicit subsets.
        with tempfile.TemporaryDirectory() as temp:
            registry_path = Path(temp) / "workloads.json"
            registry_path.write_text(json.dumps(registry))
            with self.assertRaisesRegex(simd_scaling.MeasurementError, "model workload mismatch"):
                simd_scaling._selected_entries(
                    simd_workloads, registry_path, "simd"
                )


if __name__ == "__main__":
    unittest.main()
