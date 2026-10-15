"""Contained unit tests for the proposed P140 policy validation helper."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import p140_entry_policy as policy


def inside(root: Path, path: Path) -> Path:
    root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise AssertionError(f"synthetic output escapes temp root: {resolved}")
    return resolved


class P140EntryPolicyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="p140-entry-policy-")
        self.root = Path(self.tmp.name).resolve()
        # Preserve the executed test/helper sources and exact invocation before
        # any synthetic file is written or test subprocess could be introduced.
        audit = inside(self.root, self.root / "audit")
        audit.mkdir()
        inside(self.root, audit / "test_source.py").write_bytes(Path(__file__).read_bytes())
        helper = inside(self.root, audit / "p140_entry_policy.py")
        helper.write_bytes(Path(policy.__file__).read_bytes())
        inside(self.root, audit / "arguments.json").write_text(json.dumps({
            "test": self.id(), "helper": str(helper), "argv": list(sys.argv),
        }, sort_keys=True, indent=2) + "\n")

    def tearDown(self):
        self.tmp.cleanup()

    def test_threshold_contract_and_integer_domain_are_exact(self):
        self.assertEqual(policy.CONTRACT, {
            "schema_version": 1,
            "argument": "--register-entry-threshold",
            "mode": "register",
            "minimum": 1,
            "maximum": 18446744073709551615,
        })
        for value in (1, 18446744073709551615):
            self.assertEqual(policy.positive_threshold(value), value)
        for value in (True, False, 0, -1, 18446744073709551616, "3", 3.0, None):
            with self.subTest(value=value), self.assertRaises(ValueError):
                policy.positive_threshold(value)

    def test_declaration_accepts_only_exact_candidate_only_shape(self):
        self.assertIsNone(policy.declaration({"schema_version": 1}))
        manifest = {"execution_settings": {"register_entry_threshold": {
            "baseline": None, "candidate": 3,
        }}}
        self.assertEqual(policy.declaration(manifest), {"baseline": None, "candidate": 3})
        malformed = [
            {"execution_settings": {}},
            {"execution_settings": {"register_entry_threshold": {"baseline": 0, "candidate": 3}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": True}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": 0}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": 2**64}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": 3, "extra": 4}}},
            {"execution_settings": {"register_entry_threshold": {"baseline": None, "candidate": 3}, "extra": 4}},
        ]
        for item in malformed:
            with self.subTest(item=item), self.assertRaises(ValueError):
                policy.declaration(item)

    def test_request_enforces_mode_frozen_match_and_omission_distinction(self):
        undeclared = {"schema_version": 1}
        declared = {"execution_settings": {"register_entry_threshold": {
            "baseline": None, "candidate": 3,
        }}}
        self.assertIsNone(policy.request(undeclared, "register", None))
        self.assertEqual(policy.request(declared, "register", 3), {"baseline": None, "candidate": 3})
        for manifest, mode, threshold in (
            (declared, "register", None),
            (undeclared, "register", 3),
            (declared, "register", 4),
            (declared, "compact", 3),
            (undeclared, "compact", None),
        ):
            if manifest is undeclared and threshold is None:
                continue
            with self.subTest(mode=mode, threshold=threshold), self.assertRaises(ValueError):
                policy.request(manifest, mode, threshold)

    def test_wire_candidate_only_legacy_and_capability_tamper(self):
        legacy = {"result": 8, "case": "tiny"}
        self.assertIs(policy.validate_wire(legacy, None), legacy)
        with self.assertRaisesRegex(ValueError, "unexpected"):
            policy.validate_wire({**legacy, "register_entry_threshold": 3}, None)
        candidate = {**legacy, "register_entry_threshold": 3}
        self.assertIs(policy.validate_wire(candidate, 3), candidate)
        for wire in (legacy, {**candidate, "register_entry_threshold": True},
                     {**candidate, "register_entry_threshold": 4}):
            with self.subTest(wire=wire), self.assertRaises(ValueError):
                policy.validate_wire(wire, 3)
        self.assertIs(policy.capability(policy.CONTRACT), policy.CONTRACT)
        tampered = copy.deepcopy(policy.CONTRACT)
        tampered["maximum"] -= 1
        with self.assertRaisesRegex(ValueError, "capability"):
            policy.capability(tampered)
        with self.assertRaises(ValueError):
            policy.capability({**policy.CONTRACT, "extra": 1})


if __name__ == "__main__":
    unittest.main()
