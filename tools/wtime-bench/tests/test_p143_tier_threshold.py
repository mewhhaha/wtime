"""Contained tests for adaptive Register threshold wiring in tier_check."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
import tier_check


class Completed:
    def __init__(self, value: dict) -> None:
        self.returncode = 0
        self.stdout = json.dumps(value)
        self.stderr = ""


class P143TierThresholdTests(unittest.TestCase):
    def test_p143_duplicate_cli_threshold_fails_before_io(self) -> None:
        import contextlib
        import io
        output = self.inside(self.root / "duplicate-output")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            tier_check.main(["--tier", "register", "--output", str(output),
                             "--register-entry-threshold", "3", "--register-entry-threshold", "4"])
        self.assertEqual(error.exception.code, 2)
        self.assertFalse(output.exists())

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p143-tier-threshold-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def inside(self, path: Path) -> Path:
        resolved = path.resolve()
        self.assertTrue(resolved.is_relative_to(self.root), f"test output escaped temp root: {resolved}")
        return resolved

    def write_inside(self, path: Path, data: bytes) -> Path:
        path = self.inside(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_p143_threshold_bounds_and_mode_fail_before_output_creation(self) -> None:
        invalid = (True, False, 0, -1, tier_check.MAX_REGISTER_ENTRY_THRESHOLD + 1)
        for threshold in invalid:
            with self.subTest(threshold=threshold):
                output = self.inside(self.root / f"invalid-{threshold!s}")
                with self.assertRaisesRegex(tier_check.TierCheckError, "positive u64"):
                    tier_check.check_tier(
                        tier="register", output=output,
                        register_entry_threshold=threshold,
                    )
                self.assertFalse(output.exists())

        output = self.inside(self.root / "decoded-threshold")
        with self.assertRaisesRegex(tier_check.TierCheckError, "requires --tier register"):
            tier_check.check_tier(
                tier="decoded", output=output, register_entry_threshold=3,
            )
        self.assertFalse(output.exists())

    def test_p143_typed_case_forwards_only_register_and_checks_echo(self) -> None:
        binary = self.inside(self.root / "wtime-bench")
        case = {
            "wasm": self.inside(self.root / "fixture.wasm"),
            "id": "case_a", "input": "i32:1", "expected": "i32:2",
        }
        calls: list[list[str]] = []

        def runner(argv, **_kwargs):
            calls.append(argv)
            mode = argv[argv.index("first_call") + 2]
            response = {
                "schema_version": 2, "case": "case_a", "stage": "first_call",
                "input": "i32:1", "expected": "i32:2", "result": "i32:2",
                "batch": 1, "mode": mode, "profile": "mvp", "ns_per_operation": 12.0,
            }
            if "--register-entry-threshold" in argv:
                response["register_entry_threshold"] = int(argv[-1])
            return Completed(response)

        adaptive = tier_check._case_run(binary, case, "register", runner, 3)
        reference = tier_check._case_run(binary, case, "reference", runner)
        self.assertEqual(calls[0][-2:], ["--register-entry-threshold", "3"])
        self.assertNotIn("--register-entry-threshold", calls[1])
        self.assertEqual(adaptive["register_entry_threshold"], 3)
        self.assertIsNone(reference["register_entry_threshold"])

        def wrong_echo(argv, **_kwargs):
            response = json.loads(runner(argv, **_kwargs).stdout)
            response["register_entry_threshold"] = 4
            return Completed(response)

        with self.assertRaisesRegex(tier_check.TierCheckError, "expected 3"):
            tier_check._case_run(binary, case, "register", wrong_echo, 3)

    def _state_fixture(self) -> tuple[Path, dict]:
        fixture_dir = self.inside(self.root / "state-fixtures")
        self.write_inside(fixture_dir / "state_normal.wasm", b"fixture bytes")
        memory = bytearray(65536)
        memory[:4] = (11).to_bytes(4, "little")
        raw_memory = bytes(memory)
        return fixture_dir, {
            "expected_outcome": "returned:i32:42", "global_state_i32": 11,
            "memory": {"byte_length": len(raw_memory),
                       "sha256": hashlib.sha256(raw_memory).hexdigest(), "prefix_u32": 11},
            "root_graph": None,
        }

    def _state_response(self, memory_dir: Path, threshold: int | None) -> dict:
        memory = bytearray(65536)
        memory[:4] = (11).to_bytes(4, "little")
        response = {
            "schema_version": 1, "diagnostic": True, "instrumented": False,
            "case": "state_normal", "mode": "register", "resolved_policy": "Register",
            "fuel_budget": tier_check.STATE_FUEL, "outcome": "returned", "trap": None,
            "values": ["i32:42"], "memory_hex": bytes(memory).hex(), "global": "i32:11",
            "table": ["function", "null"], "root": None, "graph": None,
            "collection": {"live": 0, "reclaimed": 0}, "cursor": None,
        }
        if threshold is not None:
            response["register_entry_threshold"] = threshold
        return response

    def test_p143_state_driver_appends_threshold_and_rejects_wrong_echo(self) -> None:
        fixture_dir, expected = self._state_fixture()
        binary = self.inside(self.root / "wtime-bench")
        binary.write_bytes(b"isolated test binary")
        with patch.object(tier_check, "STATE_WASM_DIR", fixture_dir):
            calls: list[list[str]] = []

            def runner(argv, **_kwargs):
                calls.append(argv)
                return Completed(self._state_response(fixture_dir, 3))

            result = tier_check._state_run(binary, "state_normal", "register", expected, runner, 3)
            self.assertEqual(calls[0][-2:], ["--register-entry-threshold", "3"])
            self.assertEqual(result["register_entry_threshold"], 3)

            def wrong_echo(argv, **_kwargs):
                return Completed(self._state_response(fixture_dir, 4))

            with self.assertRaisesRegex(tier_check.TierCheckError, "expected 3"):
                tier_check._state_run(binary, "state_normal", "register", expected, wrong_echo, 3)

    def test_p143_conformance_threshold_cli_and_report_binding(self) -> None:
        binary = self.inside(self.root / "wtime-conformance")
        report_path = self.inside(self.root / "reports" / "register.json")
        source_hash = "corpus-lock:" + "a" * 64 + ";fixture.wast:" + "b" * 64
        calls: list[list[str]] = []

        def runner(argv, **_kwargs):
            calls.append(argv)
            report = {
                "execution_mode": "register", "instrumented": True,
                "selected": 2, "executed": 2, "failed": 0, "skipped": 0,
                "unsupported": 0, "executed_opcodes": [11], "source_hash": source_hash,
                "register_entry_threshold": 3,
            }
            path = self.inside(Path(argv[argv.index("--report") + 1]))
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(report), encoding="utf-8")
            return Completed({})

        with patch.object(tier_check, "_resolve_corpus_sources", return_value={"fixture.wast": "b" * 64}):
            result = tier_check._conformance_run(binary, "interactions", "register", report_path,
                                                 runner, 3)
            self.assertEqual(calls[0][-2:], ["--register-entry-threshold", "3"])
            self.assertEqual(result["register_entry_threshold"], 3)

            def wrong_echo(argv, **kwargs):
                runner(argv, **kwargs)
                path = self.inside(Path(argv[argv.index("--report") + 1]))
                report = json.loads(path.read_text(encoding="utf-8"))
                report["register_entry_threshold"] = 4
                path.write_text(json.dumps(report), encoding="utf-8")
                return Completed({})

            with self.assertRaisesRegex(tier_check.TierCheckError, "expected 3"):
                tier_check._conformance_run(binary, "interactions", "register", report_path,
                                             wrong_echo, 3)

    def test_p143_omitted_threshold_accepts_legacy_missing_field_but_rejects_nonnull(self) -> None:
        report_path = self.inside(self.root / "reports" / "reference.json")
        binary = self.inside(self.root / "wtime-conformance")
        source_hash = "corpus-lock:" + "a" * 64 + ";fixture.wast:" + "b" * 64
        report = {
            "execution_mode": "reference", "instrumented": True,
            "selected": 1, "executed": 1, "failed": 0, "skipped": 0,
            "unsupported": 0, "executed_opcodes": [11], "source_hash": source_hash,
        }

        def runner_for(value):
            def runner(argv, **_kwargs):
                path = self.inside(Path(argv[argv.index("--report") + 1]))
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(json.dumps(value), encoding="utf-8")
                return Completed({})
            return runner

        with patch.object(tier_check, "_resolve_corpus_sources", return_value={"fixture.wast": "b" * 64}):
            accepted = tier_check._conformance_run(binary, "interactions", "reference",
                                                   report_path, runner_for(report))
            self.assertIsNone(accepted["register_entry_threshold"])
            unexpected = dict(report, register_entry_threshold=3)
            with self.assertRaisesRegex(tier_check.TierCheckError, "unexpected register entry threshold"):
                tier_check._conformance_run(binary, "interactions", "reference",
                                            report_path, runner_for(unexpected))


if __name__ == "__main__":
    unittest.main()
