"""Validate retained P135 CLI output and admitted fixture source identities."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "tools/wtime-bench/fixtures/p135"
OBSERVATIONS: Path | None = None
sys.path.insert(0, str(ROOT))


class P135EntryBatchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.provenance = json.loads((FIXTURES / "provenance.json").read_text(encoding="utf-8"))

    def test_p135_fixture_bytes_are_bound_by_provenance(self) -> None:
        for name, record in self.provenance["fixtures"].items():
            with self.subTest(fixture=name):
                wat = FIXTURES / record["wat"]
                wasm = FIXTURES / record["wasm"]
                self.assertTrue(wat.is_file())
                self.assertTrue(wasm.is_file())
                self.assertEqual(hashlib.sha256(wat.read_bytes()).hexdigest(), record["wat_sha256"])
                self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(), record["wasm_sha256"])
                self.assertEqual(wasm.read_bytes()[:4], b"\x00asm")

    def test_p135_fixture_profile_is_mvp_compatible(self) -> None:
        self.assertEqual(self.provenance["profile"],
                         "strict MVP; both fixtures use only MVP memory and integer instructions")
        self.assertEqual(self.provenance["official_validator"], "wasm-tools validate BINARY --features mvp")
        self.assertEqual(self.provenance["errors_count"], 0)
        counter = self.provenance["fixtures"]["counter"]
        self.assertEqual(counter["exports"]["run"], "(i32)->i32")
        self.assertEqual(counter["source_opcode_count"], 10)
        self.assertTrue(counter["exports"]["count"].startswith("memory "))

    def test_p135_cli_matrix_proves_fresh_eager_and_adaptive_lifetimes(self) -> None:
        self.assertIsNotNone(OBSERVATIONS, "pass --observations from the retained source-bound producer")
        observation = json.loads(OBSERVATIONS.read_text(encoding="utf-8"))
        from tools.verification.producers.p135 import validate_observations

        validate_observations(observation, root=ROOT)
        self.assertEqual(len(observation["timing_rows"]), 8)
        self.assertEqual(len(observation["diagnostic_rows"]), 8)


def main() -> None:
    global OBSERVATIONS
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--observations", type=Path)
    args, unittest_args = parser.parse_known_args()
    OBSERVATIONS = args.observations.resolve() if args.observations else None
    sys.argv = [sys.argv[0], *unittest_args]
    unittest.main()


if __name__ == "__main__":
    main()
