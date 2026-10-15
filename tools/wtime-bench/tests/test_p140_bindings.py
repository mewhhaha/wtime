"""Contained binding tests for the P140 Register threshold evidence path."""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
ROOT = BENCH.parents[1]
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(ROOT))

import compare
import candidate_gate
import focused_compare
import p140_entry_policy
import selection_manifest
from tools.verification.tests import test_production_attestation as production_fixtures
from tools.verification import production_attestation


def inside(root: Path, path: Path) -> Path:
    root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise AssertionError(f"synthetic write escapes fresh temp root: {resolved}")
    return resolved


class P140BindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p140-bindings-")
        self.root = Path(self.temp.name).resolve()
        audit = inside(self.root, self.root / "audit")
        audit.mkdir()
        # Keep exactly the sources and invocation that define each synthetic check.
        inside(self.root, audit / "test_p140_bindings.py").write_bytes(Path(__file__).read_bytes())
        helper_copy = inside(self.root, audit / "p140_entry_policy.py")
        helper_copy.write_bytes(Path(p140_entry_policy.__file__).read_bytes())
        inside(self.root, audit / "arguments.json").write_text(json.dumps({
            "test": self.id(), "argv": list(sys.argv), "helper": str(helper_copy),
        }, sort_keys=True, indent=2) + "\n")

    def tearDown(self):
        self.temp.cleanup()

    def test_compare_forwards_threshold_after_fuel_and_keeps_baseline_legacy(self):
        payload = {"schema_version": 2, "case": "scalar_add", "stage": "warm",
                   "input": "i32:2", "expected": "i32:3", "mode": "register",
                   "profile": "mvp", "driver": "module", "diagnostic": False,
                   "result": "i32:3", "ns_per_operation": 1.0, "batch": 4,
                   "source_fuel": 77}
        candidate = {**payload, "register_entry_threshold": 9}
        fuel = {"name": "metered", "source_fuel": 50_000_000}
        payload.update({"source_fuel": 50_000_000, "fuel_policy": "metered"})
        candidate.update({"source_fuel": 50_000_000, "fuel_policy": "metered"})
        calls = []

        def process(_exe, argv, _cpu):
            calls.append(list(argv))
            return candidate if "--register-entry-threshold" in argv else payload

        entry = {"id": "scalar_add", "driver": {"kind": "module", "profile": "mvp"}}
        with mock.patch.object(compare, "process", side_effect=process):
            base_result = compare.scalar_observation("baseline", "fixture", entry, 2, 3,
                "warm", 4, "register", 0, fuel_setting=fuel)
            cand_result = compare.scalar_observation("candidate", "fixture", entry, 2, 3,
                "warm", 4, "register", 0, fuel_setting=fuel, register_entry_threshold=9)
        self.assertEqual(base_result["wire_observation"], payload)
        self.assertEqual(cand_result["wire_observation"], candidate)
        self.assertNotIn("--register-entry-threshold", calls[0])
        self.assertEqual(calls[0][-2:], ["--source-fuel", "50000000"])
        self.assertEqual(calls[1][-4:], ["--source-fuel", "50000000", "--register-entry-threshold", 9])

    def test_focused_request_rejects_mismatch_omission_and_nonregister_before_process(self):
        declaration = {"execution_settings": {"register_entry_threshold": {
            "baseline": None, "candidate": 9,
        }}}
        invalids = (("register", None), ("register", 8), ("compact", 9))
        with mock.patch.object(focused_compare.selection_manifest, "verify_frozen", return_value=declaration), \
             mock.patch.object(focused_compare.baseline, "verify_archive", side_effect=AssertionError("archive read")), \
             mock.patch.object(focused_compare.compare, "process", side_effect=AssertionError("observation launched")):
            for mode, threshold in invalids:
                args = types.SimpleNamespace(smoke=True, frozen=Path("synthetic-freeze"), manifest=Path("manifest"),
                    semantic_receipt=None, candidate_mode=mode, register_entry_threshold=threshold)
                with self.subTest(mode=mode, threshold=threshold), self.assertRaises(ValueError):
                    focused_compare.run(args)

    def test_selection_manifest_and_gate_binding_reject_setting_command_and_wire_drift(self):
        declaration = {"execution_settings": {"register_entry_threshold": {
            "baseline": None, "candidate": 9,
        }}}
        self.assertEqual(p140_entry_policy.declaration(declaration), {"baseline": None, "candidate": 9})
        self.assertEqual(p140_entry_policy.request(declaration, "register", 9)["candidate"], 9)
        draft_path = inside(ROOT, ROOT / "docs/benchmarks/performance/P047-selection.json")
        draft = json.loads(draft_path.read_text())
        draft["registry_sha256"] = hashlib.sha256(selection_manifest.DEFAULT_REGISTRY.read_bytes()).hexdigest()
        draft["execution_settings"] = copy.deepcopy(declaration["execution_settings"])
        validated = selection_manifest.load_data(draft)
        self.assertEqual(validated["execution_settings"], declaration["execution_settings"])
        for tampered in (
            {"execution_settings": {"register_entry_threshold": {"baseline": 0, "candidate": 9}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": True}}},
        ):
            malformed = copy.deepcopy(draft)
            malformed.update(tampered)
            with self.subTest(tampered=tampered), self.assertRaises(ValueError):
                selection_manifest.load_data(malformed)

        command = inside(self.root, self.root / "command.json")
        declaration = validated
        settings = {"candidate_mode": "register", "register_entry_threshold": {
            "baseline": None, "candidate": 9,
        }}
        capabilities = {"candidate": {"register_entry_threshold_contract": dict(p140_entry_policy.CONTRACT)}}

        def gate(argv, *, selected=declaration, metadata=settings, caps=capabilities):
            command.write_text(json.dumps({"argv": argv}))
            return candidate_gate.validate_entry_setting(
                selected, metadata, {"command": {"path": str(command)}}, caps)

        self.assertEqual(gate(["focused_compare.py", "--register-entry-threshold", "9"]),
                         {"baseline": None, "candidate": 9})
        drift_cases = (
            (["focused_compare.py"], declaration, settings, capabilities),
            (["focused_compare.py", "--register-entry-threshold", "8"], declaration, settings, capabilities),
            (["focused_compare.py", "--register-entry-threshold", "9", "--register-entry-threshold", "9"], declaration, settings, capabilities),
            (["focused_compare.py", "--register-entry-threshold", "9"], declaration,
             {"candidate_mode": "register", "register_entry_threshold": {"baseline": None, "candidate": 8}}, capabilities),
            (["focused_compare.py", "--register-entry-threshold", "9"], declaration,
             {"candidate_mode": "compact", "register_entry_threshold": {"baseline": None, "candidate": 9}}, capabilities),
            (["focused_compare.py", "--register-entry-threshold", "9"], declaration, settings,
             {"candidate": {"register_entry_threshold_contract": {**p140_entry_policy.CONTRACT, "maximum": 8}}}),
        )
        for argv, selected, metadata, caps in drift_cases:
            with self.subTest(argv=argv, metadata=metadata, caps=caps), self.assertRaises(ValueError):
                gate(argv, selected=selected, metadata=metadata, caps=caps)
        one = copy.deepcopy(declaration)
        one["execution_settings"]["register_entry_threshold"]["candidate"] = 1
        for value in (True, 1.0):
            with self.subTest(value=value), self.assertRaises(ValueError):
                gate(["focused_compare.py", "--register-entry-threshold", "1"], selected=one,
                     metadata={"candidate_mode": "register", "register_entry_threshold": {
                         "baseline": None, "candidate": value}})
        good_wire = {"result": "i32:3", "register_entry_threshold": 9}
        self.assertIs(p140_entry_policy.validate_wire(good_wire, 9), good_wire)
        for bad_wire in ({"result": "i32:3"}, {"result": "i32:3", "register_entry_threshold": 8}):
            with self.subTest(bad_wire=bad_wire), self.assertRaises(ValueError):
                p140_entry_policy.validate_wire(bad_wire, 9)

    def test_runtime_attestation_requires_adaptive_register_capability(self):
        # Reuse the production attestation's fully contained fake Cargo/runtime tree.
        old_create_case = production_fixtures.ProductionAttestationTests.create_case
        real_tempdir = tempfile.TemporaryDirectory

        def contained_tempdir(*, prefix="tmp", dir=None, **kwargs):
            return real_tempdir(prefix=prefix, dir=self.root, **kwargs)

        with mock.patch.object(production_fixtures.tempfile, "TemporaryDirectory", contained_tempdir):
            case = old_create_case(production_fixtures.ProductionAttestationTests(), "register")
        try:
            temporary, root, record_path, record, metadata, report = case
            helper_path = inside(root, root / "tools/wtime-bench/p140_entry_policy.py")
            helper_path.parent.mkdir(parents=True, exist_ok=True)
            helper_path.write_bytes(Path(p140_entry_policy.__file__).read_bytes())
            contract = dict(p140_entry_policy.CONTRACT)
            protocol = {**record["protocol"], "register_entry_threshold_contract": contract}
            record["protocol"] = protocol
            metadata["capabilities"]["candidate"] = protocol
            report.update({"declaration": {"execution_settings": {"register_entry_threshold": {
                "baseline": None, "candidate": 9,
            }}}})
            metadata["settings"]["register_entry_threshold"] = {"baseline": None, "candidate": 9}
            protocol_path = record_path.parent / "protocol.stdout.json"
            protocol_bytes = json.dumps(protocol).encode()
            production_fixtures.safe_write(root, protocol_path, protocol_bytes)
            record["protocol_sha256"] = production_fixtures.digest(protocol_bytes)
            production_fixtures.safe_write(root, record_path.parent / "protocol.stdout", protocol_bytes)
            record["logs"]["protocol.stdout"]["sha256"] = production_fixtures.digest(protocol_bytes)
            record["protocol_stdout"] = str(record_path.parent / "protocol.stdout")
            record["protocol_stderr"] = str(record_path.parent / "protocol.stderr")
            production_fixtures.safe_write(root, record_path.parent / "protocol.stderr", b"")
            production_fixtures.safe_write(root, record_path, json.dumps(record).encode())
            result = production_attestation.validate_runtime_record(record_path, metadata, report, root)
            self.assertEqual(result["protocol"]["register_entry_threshold_contract"], contract)
            report["declaration"]["execution_settings"]["register_entry_threshold"]["candidate"] = 1
            for value in (True, 1.0):
                metadata["settings"]["register_entry_threshold"]["candidate"] = value
                with self.subTest(value=value), self.assertRaises(production_attestation.AttestationError):
                    production_attestation.validate_runtime_record(record_path, metadata, report, root)
            report["declaration"]["execution_settings"]["register_entry_threshold"]["candidate"] = 9
            metadata["settings"]["register_entry_threshold"]["candidate"] = 9
            for invalid in ({**contract, "maximum": contract["maximum"] - 1}, None):
                broken = copy.deepcopy(protocol)
                broken["register_entry_threshold_contract"] = invalid
                broken_bytes = json.dumps(broken).encode()
                production_fixtures.safe_write(root, protocol_path, broken_bytes)
                production_fixtures.safe_write(root, record_path.parent / "protocol.stdout", broken_bytes)
                record["logs"]["protocol.stdout"]["sha256"] = production_fixtures.digest(broken_bytes)
                record["protocol"] = broken
                record["protocol_sha256"] = production_fixtures.digest(broken_bytes)
                metadata["capabilities"]["candidate"] = broken
                production_fixtures.safe_write(root, record_path, json.dumps(record).encode())
                with self.assertRaises(production_attestation.AttestationError):
                    production_attestation.validate_runtime_record(record_path, metadata, report, root)
        finally:
            case[0].cleanup()


if __name__ == "__main__":
    unittest.main()
