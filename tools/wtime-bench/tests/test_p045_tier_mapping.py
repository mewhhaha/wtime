"""P045 compact policy mapping and build-feature regression checks."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import tier_check


class Completed:
    def __init__(self, payload: dict, returncode: int = 0) -> None:
        self.returncode = returncode
        self.stdout = json.dumps(payload)
        self.stderr = ""


def state_wire(case: str, mode: str, resolved_policy: str, fuel: int = tier_check.STATE_FUEL) -> dict:
    memory = bytearray(65536)
    struct.pack_into("<I", memory, 0, 42)
    return {
        "schema_version": 1,
        "diagnostic": True,
        "instrumented": False,
        "case": case,
        "mode": mode,
        "resolved_policy": resolved_policy,
        "fuel_budget": fuel,
        "outcome": "returned",
        "trap": None,
        "values": ["i32:42"],
        "memory_hex": memory.hex(),
        "global": "i32:7",
        "table": ["function", "null"],
        "root": None,
        "graph": None,
        "collection": {"live": 0, "reclaimed": 0},
        "cursor": None,
    }


def state_expected() -> dict:
    memory = bytearray(65536)
    struct.pack_into("<I", memory, 0, 42)
    return {
        "expected_outcome": "returned:i32:42",
        "memory": {
            "byte_length": len(memory),
            "sha256": hashlib.sha256(memory).hexdigest(),
            "prefix_u32": 42,
        },
        "global_state_i32": 7,
        "root_graph": None,
    }


class P045TierMappingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p045-tier-map-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.binary = self.root / "synthetic-binary"
        self.binary.write_bytes(b"temporary P045 policy-test binary")
        self.state_root = (self.root / "state-fixtures").resolve()
        self.assertTrue(self.state_root.is_relative_to(self.root))

    def _runner(self, wire):
        def run(command, **_kwargs):
            wasm = Path(command[2]).resolve()
            self.assertTrue(wasm.is_relative_to(self.state_root))
            return Completed(wire)
        return run

    def test_compact_state_run_accepts_compact_policy_and_full_memory(self):
        wire = state_wire("state_normal", "compact", "Compact")
        with mock.patch.object(tier_check, "STATE_WASM_DIR", self.state_root):
            result = tier_check._state_run(
                self.binary, "state_normal", "compact", state_expected(), self._runner(wire)
            )
        self.assertEqual(result["mode"], "compact")
        self.assertEqual(result["memory_byte_length"], 65536)
        self.assertEqual(result["memory_prefix_u32"], 42)
        self.assertTrue(result["remaining_memory_zero"])

    def test_compact_fuel_probe_accepts_compact_policy_and_full_memory(self):
        wire = state_wire("state_normal", "compact", "Compact", fuel=9)
        with mock.patch.object(tier_check, "STATE_WASM_DIR", self.state_root):
            result = tier_check._state_fuel_probe(
                self.binary, "state_normal", "compact", 9, self._runner(wire)
            )
        self.assertEqual(result["mode"], "compact")
        self.assertEqual(result["fuel"], 9)
        self.assertEqual(result["memory_prefix_hex"], "2a00000000000000")
        self.assertEqual(result["memory_sha256"], state_expected()["memory"]["sha256"])

    def test_foreign_and_invalid_resolved_policies_are_rejected_without_fallback(self):
        with mock.patch.object(tier_check, "STATE_WASM_DIR", self.state_root):
            foreign = state_wire("state_normal", "compact", "Portable")
            with self.assertRaisesRegex(tier_check.TierCheckError, "resolved policy differs"):
                tier_check._state_run(
                    self.binary, "state_normal", "compact", state_expected(), self._runner(foreign)
                )
            with self.assertRaisesRegex(tier_check.TierCheckError, "malformed fuel-boundary"):
                tier_check._state_fuel_probe(
                    self.binary, "state_normal", "compact", tier_check.STATE_FUEL,
                    self._runner(foreign),
                )
            invalid = state_wire("state_normal", "made-up", "Decoded")
            with self.assertRaisesRegex(tier_check.TierCheckError, "unsupported state-driver mode"):
                tier_check._state_run(
                    self.binary, "state_normal", "made-up", state_expected(), self._runner(invalid)
                )
            with self.assertRaisesRegex(tier_check.TierCheckError, "malformed fuel-boundary"):
                tier_check._state_fuel_probe(
                    self.binary, "state_normal", "made-up", tier_check.STATE_FUEL,
                    self._runner(invalid),
                )

    def test_compact_auto_build_forwards_policy_to_bench_and_conformance(self):
        conformance = self.root / "synthetic-conformance"
        counters = self.root / "synthetic-counters"
        calls = []

        def builder(command, **_kwargs):
            calls.append(command)
            target = (self.binary, conformance, counters)[len(calls) - 1]
            target.write_bytes(b"synthetic feature build")
            target.chmod(0o755)
            return Completed({})

        with (mock.patch.object(tier_check, "DEFAULT_CONFORMANCE", conformance),
              mock.patch.object(tier_check, "DEFAULT_COUNTERS_BINARY", counters)):
            info = tier_check._build_reference(self.binary, builder, tier="compact")
        self.assertEqual(len(calls), 3)
        benchmark_features = calls[0][calls[0].index("--features") + 1]
        self.assertEqual(benchmark_features, "execution-reference,compact-executor,runtime-only")
        conformance_features = calls[1][calls[1].index("--features") + 1]
        self.assertEqual(conformance_features,
                         "execution-reference,compact-executor,opcode-coverage")
        diagnostic_features = calls[2][calls[2].index("--features") + 1]
        self.assertNotIn("compact-executor", diagnostic_features)
        self.assertEqual(info["feature_sets"]["policy"],
                         ["execution-reference", "compact-executor"])

    def test_invalid_auto_build_tier_is_rejected_before_builder_work(self):
        calls = []
        with self.assertRaisesRegex(tier_check.TierCheckError, "unsupported execution mode"):
            tier_check._build_reference(self.binary, lambda *args, **kwargs: calls.append(args),
                                        tier="not-a-tier")
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
