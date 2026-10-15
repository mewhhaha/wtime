"""Focused tests for the P080 cross-module continuation setup-fixture binding."""
import hashlib
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools" / "wtime-bench"
sys.path.insert(0, str(BENCH))

import candidate_gate  # noqa: E402
import continuation_workloads  # noqa: E402


with (BENCH / "workloads.json").open(encoding="utf-8") as _registry_file:
    _REGISTRY = json.load(_registry_file)
_ROWS = {row["id"]: row for row in _REGISTRY["workloads"]}


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _provider_ref(source):
    # The continuation and integration fixtures have identical provider WAT;
    # this checked-in integration Wasm is the available encoded artifact.
    continuation_wat = BENCH / source["path"]
    integration_wat = BENCH / "fixtures" / "integration" / "cross_module_provider.wat"
    wasm = BENCH / "fixtures" / "integration" / "cross_module_provider.wasm"
    if _sha(continuation_wat) != source["sha256"] or _sha(integration_wat) != source["sha256"]:
        raise AssertionError("provider WAT source does not match the fixed model")
    return {"path": str(wasm), "sha256": _sha(wasm)}


def _check_gate_setup_inventory(entries, refs, *, check_content=True):
    """Exercise the candidate gate's exact setup-ID/count/SHA validation slice."""
    expected = candidate_gate.expected_setup_modules(entries)
    candidate_gate.need(set(refs) == set(expected), "setup fixture inventory differs")
    for workload_id, setups in expected.items():
        actual = refs[workload_id]
        candidate_gate.need(len(actual) == len(setups), "setup fixture count differs")
        for reference, setup in zip(actual, setups):
            candidate_gate.need(reference["sha256"] == setup["wasm_sha256"],
                                "setup fixture bytes differ from declaration")
            if check_content:
                candidate_gate.need(_sha(Path(reference["path"])) == reference["sha256"],
                                    "setup fixture reference content is stale")


class P080ProviderBindingTests(unittest.TestCase):
    def test_cross_module_consumer_binds_real_provider_source_and_wasm_sha(self):
        consumer = _ROWS["continuation_cross_module_consumer"]
        provider = {row["id"]: row for row in continuation_workloads.workload_entries()}[
            "continuation_cross_module_provider"]

        expected = candidate_gate.expected_setup_modules([consumer])
        self.assertEqual(expected, {consumer["id"]: [provider["source"]]})

        reference = _provider_ref(provider["source"])
        self.assertEqual(reference["sha256"], provider["source"]["wasm_sha256"])
        refs = {consumer["id"]: [reference]}
        _check_gate_setup_inventory([consumer], refs)
        self.assertEqual(refs[consumer["id"]][0]["sha256"], provider["source"]["wasm_sha256"])

    def test_expected_setup_modules_preserves_existing_gc_setup_rows(self):
        gc_entry = _ROWS["gc_descriptor_cross_module_n1"]
        consumer = _ROWS["continuation_cross_module_consumer"]
        self.assertTrue(gc_entry["setup_modules"])

        expected = candidate_gate.expected_setup_modules([gc_entry, consumer])
        provider = {row["id"]: row for row in continuation_workloads.workload_entries()}[
            "continuation_cross_module_provider"]
        self.assertEqual(expected[gc_entry["id"]], gc_entry["setup_modules"])
        self.assertEqual(expected[consumer["id"]], [provider["source"]])
        # Candidate reports retain the pre-existing GC module list alongside
        # the new Register continuation provider requirement.
        provider_ref = _provider_ref(provider["source"])
        refs = {gc_entry["id"]: [{"path": "retained-gc-provider.wasm",
                                  "sha256": gc_entry["setup_modules"][0]["wasm_sha256"]}],
                consumer["id"]: [provider_ref]}
        _check_gate_setup_inventory([gc_entry, consumer], refs, check_content=False)

    def test_gate_inventory_rejects_missing_or_tampered_provider_reference(self):
        consumer = _ROWS["continuation_cross_module_consumer"]
        entries = [consumer]
        expected = candidate_gate.expected_setup_modules(entries)
        provider = {row["id"]: row for row in continuation_workloads.workload_entries()}[
            "continuation_cross_module_provider"]
        valid = {consumer["id"]: [_provider_ref(provider["source"])]}
        _check_gate_setup_inventory(entries, valid)

        with self.assertRaisesRegex(ValueError, "setup fixture inventory differs"):
            _check_gate_setup_inventory(entries, {})

        wrong_hash = json.loads(json.dumps(valid))
        wrong_hash[consumer["id"]][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "setup fixture bytes differ"):
            _check_gate_setup_inventory(entries, wrong_hash)

        wrong_path = json.loads(json.dumps(valid))
        wrong_path[consumer["id"]][0]["path"] = str(
            BENCH / "fixtures" / "integration" / "cross_module_consumer.wasm")
        with self.assertRaisesRegex(ValueError, "setup fixture reference content is stale"):
            _check_gate_setup_inventory(entries, wrong_path)


if __name__ == "__main__":
    unittest.main()
