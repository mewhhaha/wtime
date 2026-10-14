import hashlib
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import representative_workloads as representative


class RepresentativeWorkloadTests(unittest.TestCase):
    def test_registry_has_three_scales_and_predeclared_held_out_cases(self):
        entries = representative.entries()
        self.assertEqual(len(entries), 9)
        self.assertEqual({entry["family"] for entry in entries}, set(representative.KERNELS))
        for workload_id, kernel in representative.KERNELS.items():
            cases = [entry for entry in entries if entry["family"] == workload_id]
            self.assertEqual({entry["validation_role"] for entry in cases},
                             {"small", "medium", "held_out"})
            self.assertEqual(
                {entry["params"][0]["baseline"] for entry in cases},
                set(kernel["cases"].values()),
            )

    def test_independent_references_match_pinned_application_outputs(self):
        parser = representative.oracle("representative_parser_hash_small", 16)
        image = representative.oracle("representative_image_filter_small", 16)
        graph = representative.oracle("representative_graph_bfs_small", 12)
        self.assertEqual(parser, {
            "result": -540878575,
            "output_sha256": "f6ebed2fcdfd545dd8225321b411a07879fcc67c18eca8347a598a08e65954a6",
            "output_bytes": 16,
        })
        self.assertEqual(image, {
            "result": 1797,
            "output_sha256": "74fc95c09301988c09b45478de813b22c50062e84460e45671e016d92f827119",
            "output_bytes": 16,
        })
        self.assertEqual(graph, {
            "result": -926913441,
            "output_sha256": "6135fa5fa77518c02bb8f3a686675ab0bbed9219665360887dadd20877ac6316",
            "output_bytes": 12,
        })

    def test_sources_are_original_cc0_import_free_and_bind_encoded_artifacts(self):
        provenance = representative.provenance()
        self.assertEqual(len(representative.ENCODER_SHA256), 64)
        for entry in representative.entries():
            expected = representative.oracle(
                entry["id"], **entry["oracle_arguments"]
            )
            self.assertEqual(entry["expected_result"], expected["result"])
            self.assertEqual(entry["expected_output_sha256"], expected["output_sha256"])
            source_path = HERE / entry["source"]["path"]
            source_bytes = source_path.read_bytes()
            source = source_bytes.decode()
            filename = source_path.name
            self.assertEqual(hashlib.sha256(source_bytes).hexdigest(),
                             entry["source"]["sha256"])
            self.assertEqual(entry["source"]["wasm_sha256"],
                             representative.WASM_SHA256[filename])
            self.assertEqual(entry["source"]["encoding_status"], "encoded")
            self.assertNotIn("(import", source)
            self.assertIn("SPDX-License-Identifier: CC0-1.0", source)
            self.assertIn("(export \"prepare\")", source)
            self.assertIn("(export \"run\")", source)
            evidence = provenance["kernels"][entry["family"]]
            self.assertEqual(evidence["license"], "CC0-1.0")
            self.assertFalse(evidence["external_source_or_download"])
            self.assertEqual(evidence["source_sha256"], entry["source"]["sha256"])

    def test_setup_is_outside_timing_and_oracle_rejects_unfrozen_sizes(self):
        for entry in representative.entries():
            self.assertEqual(entry["entrypoint"], "run")
            self.assertEqual(entry["setup_export"], "prepare")
            self.assertFalse(entry["setup_policy"]["timed"])
            self.assertEqual(entry["setup_policy"]["frequency"], "before_each_timed_run")
        with self.assertRaisesRegex(ValueError, "unknown representative workload"):
            representative.oracle("representative_unknown_small", 16)
        with self.assertRaisesRegex(ValueError, "size must be one of"):
            representative.oracle("representative_image_filter", 17)


if __name__ == "__main__":
    unittest.main()
