"""Register policy support in the state probes, with isolated fake reports."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import tier_check


class Completed:
    def __init__(self, value: dict) -> None:
        self.returncode = 0
        self.stdout = json.dumps(value)
        self.stderr = ""


class RegisterStatePolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p065-tier-policy-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.manifest_path = self.root / "state" / "expected.json"
        self.wasm_dir = self.root / "state" / "wasm"
        self.binary = self.root / "wtime-bench"
        self._write_inside(self.binary, b"isolated test binary")
        self.binary.chmod(0o755)
        self._write_inside(self.wasm_dir / "state_normal.wasm", b"isolated fixture")
        memory = bytearray(65536)
        struct.pack_into("<I", memory, 0, 11)
        self.memory = bytes(memory)
        self.expected = {
            "expected_outcome": "returned:i32:42",
            "global_state_i32": 11,
            "memory": {
                "byte_length": len(self.memory),
                "sha256": hashlib.sha256(self.memory).hexdigest(),
                "prefix_u32": 11,
            },
            "root_graph": None,
        }
        self._write_inside(self.manifest_path, json.dumps({
            "schema_version": 1,
            "cases": {"state_normal": self.expected},
        }).encode())
        self._patches = [
            patch.object(tier_check, "STATE_MANIFEST", self.manifest_path),
            patch.object(tier_check, "STATE_WASM_DIR", self.wasm_dir),
        ]
        for item in self._patches:
            item.start()
            self.addCleanup(item.stop)

    def _write_inside(self, path: Path, data: bytes) -> None:
        resolved = path.resolve()
        self.assertTrue(resolved.is_relative_to(self.root), f"path escapes temp root: {resolved}")
        resolved.parent.mkdir(parents=True, exist_ok=True)
        resolved.write_bytes(data)

    def _response(self, *, mode: str = "register", fuel: int = tier_check.STATE_FUEL,
                  resolved_policy: str = "Register", outcome: str = "returned") -> dict:
        return {
            "schema_version": 1,
            "diagnostic": True,
            "instrumented": False,
            "case": "state_normal",
            "mode": mode,
            "resolved_policy": resolved_policy,
            "fuel_budget": fuel,
            "outcome": outcome,
            "trap": None,
            "values": ["i32:42"] if outcome == "returned" else [],
            "memory_hex": self.memory.hex(),
            "global": "i32:11",
            "table": ["function", "null"],
            "root": None,
            "graph": None,
            "collection": {"live": 0, "reclaimed": 0},
            "cursor": None,
        }

    def _runner(self, response: dict):
        return lambda *_args, **_kwargs: Completed(response)

    def test_p065_state_run_accepts_register_full_state(self) -> None:
        result = tier_check._state_run(self.binary, "state_normal", "register",
                                       self.expected, self._runner(self._response()))
        self.assertEqual(result["outcome"], "returned")
        self.assertEqual(result["memory_byte_length"], 65536)
        self.assertEqual(result["memory_prefix_u32"], 11)
        self.assertEqual(result["global"], "i32:11")

    def test_p065_state_run_rejects_decoded_policy_under_register(self) -> None:
        response = self._response(resolved_policy="Decoded")
        with self.assertRaisesRegex(tier_check.TierCheckError, "resolved policy differs"):
            tier_check._state_run(self.binary, "state_normal", "register", self.expected,
                                  self._runner(response))

    def test_p065_state_fuel_probe_accepts_register_minimum_fuel(self) -> None:
        success = tier_check._state_fuel_probe(
            self.binary, "state_normal", "register", 7,
            self._runner(self._response(fuel=7)))
        boundary = tier_check._state_fuel_probe(
            self.binary, "state_normal", "register", 6,
            self._runner(self._response(fuel=6, outcome="ResourceLimit")))
        self.assertEqual(success["outcome"], "returned")
        self.assertEqual(boundary["outcome"], "ResourceLimit")
        self.assertEqual(success["memory_sha256"], hashlib.sha256(self.memory).hexdigest())

    def test_p065_state_fuel_probe_rejects_decoded_policy_under_register(self) -> None:
        response = self._response(fuel=7, resolved_policy="Decoded")
        with self.assertRaisesRegex(tier_check.TierCheckError, "malformed fuel-boundary"):
            tier_check._state_fuel_probe(self.binary, "state_normal", "register", 7,
                                         self._runner(response))


if __name__ == "__main__":
    unittest.main()
