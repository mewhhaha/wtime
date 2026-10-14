"""Focused tests for P014's typed-protocol tier checker (no runtime subprocesses)."""

from __future__ import annotations

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
    def __init__(self, value: dict, returncode: int = 0, stderr: str = "") -> None:
        self.returncode = returncode
        self.stdout = json.dumps(value)
        self.stderr = stderr


class TypedProtocolRunner:
    def __init__(self, *, modes=("decoded", "reference"), mismatch: bool = False,
                 bad_snapshot: bool = False, bad_state: bool = False,
                 failure_mode: str | None = None) -> None:
        self.modes = list(modes)
        self.mismatch = mismatch
        self.bad_snapshot = bad_snapshot
        self.bad_state = bad_state
        self.failure_mode = failure_mode
        self.commands: list[list[str]] = []

    def __call__(self, command, **_kwargs):
        self.commands.append([str(value) for value in command])
        args = command[1:]
        if args == ["--protocol"]:
            return Completed({"schema_version": 2, "engine": "wtime",
                              "modes": self.modes, "instrumented": True})
        if args[0] == "run":
            mode = args[args.index("--mode") + 1]
            lock_hash = tier_check._sha(tier_check.ROOT / "docs/architecture/corpus-lock.json")
            source_path = tier_check.ROOT / "tools/wtime-conformance/src/corpus.rs"
            report_path = Path(args[args.index("--report") + 1])
            report_path.parent.mkdir(parents=True, exist_ok=True)
            report_path.write_text(json.dumps({
                "execution_mode": mode, "instrumented": True,
                "selected": 1, "executed": 1, "failed": 0, "skipped": 0,
                "unsupported": 0, "compatibility_assertions": 1, "errors": [],
                "trap_codes": [], "executed_opcodes": [65],
                "source_hash": f"corpus-lock:{lock_hash};{source_path}:{tier_check._sha(source_path)}",
            }))
            return Completed({})
        if args[0] == "--tier-state-driver":
            _, _path, case, mode, _profile, fuel, pc = args
            manifest = json.loads(tier_check.STATE_MANIFEST.read_text())
            expected = manifest["cases"][case]
            expected_outcome = expected["expected_outcome"].split(":", 1)
            fuel_value = int(fuel)
            threshold = 7 if case == "state_normal" else 8
            if fuel_value < threshold:
                outcome, trap, values = "ResourceLimit", None, []
            elif expected_outcome[0] == "returned":
                outcome, trap, values = "returned", None, [expected_outcome[1]]
            else:
                outcome, trap, values = "Trap", expected_outcome[1], []
            memory = bytearray(65536)
            struct.pack_into("<I", memory, 0, expected["memory"]["prefix_u32"])
            if self.bad_state and mode == "decoded":
                memory[128] = 1
            graph = expected["root_graph"]
            if self.bad_state and mode == "decoded":
                memory[128] = 1
            return Completed({
                "schema_version": 1, "diagnostic": True, "instrumented": False,
                "case": case, "mode": mode,
                "resolved_policy": {"reference": "Reference", "decoded": "Decoded",
                                    "portable": "Portable"}[mode],
                "fuel_budget": fuel_value, "outcome": outcome, "values": values, "trap": trap,
                "memory_hex": memory.hex(), "global": f"i32:{expected['global_state_i32']}",
                "table": ["function", "null"], "root": None if graph is None else "gc",
                "graph": None if graph is None else [f"i32:{n}" for n in graph["normalized_guest_projection"]],
                "collection": {"live": 0, "reclaimed": 0} if graph is None else {
                    "live": graph["after_collection"]["live_gc_objects"],
                    "reclaimed": graph["after_collection"]["reclaimed_gc_objects"],
                }, "cursor": ({"source_pc": 3, "prepared_pc": 3, "remaining_fuel": 99_990,
                                "charged": True, "state": "bounded-state"} if pc != "none" else None),
            })
        if args[0] == "--typed-case":
            _, _path, case, input_value, expected, stage, _batch, mode, profile = args
            if mode not in self.modes or self.failure_mode == mode:
                return Completed({}, returncode=2, stderr="unavailable execution policy")
            result = "i32:0" if self.mismatch and mode == "decoded" else expected
            return Completed({"schema_version": 2, "case": case, "stage": stage,
                              "input": input_value, "expected": expected, "result": result,
                              "batch": 1, "mode": mode, "profile": profile,
                              "ns_per_operation": 100.0})
        if args[0] == "--snapshot":
            return Completed({"schema_version": 1, "diagnostic": True,
                              "case": tier_check.SNAPSHOT["case"],
                              "source_pc": tier_check.SNAPSHOT["source_pc"],
                              "prepared_pc": tier_check.SNAPSHOT["source_pc"],
                              "pending_boundary_charged": True,
                              "materialization_roundtrips": 0 if self.bad_snapshot else 1,
                              "normalized_state": "bounded-state"})
        if args[0] == "--diagnostic":
            return Completed({"schema_version": 1, "diagnostic": True,
                              "result": int(args[3]),
                              "counters": {"source_dispatches": 4}})
        return Completed({}, returncode=2, stderr=f"unknown protocol command: {args!r}")


class TierCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p014-tier-check-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / "wtime-bench"
        self.binary.write_bytes(b"test binary identity")
        self.binary.chmod(0o755)
        self.diagnostic_binary = self.root / "wtime-bench-counters"
        self.diagnostic_binary.write_bytes(b"test counter binary identity")
        self.diagnostic_binary.chmod(0o755)
        self.conformance_binary = self.root / "wtime-conformance"
        self.conformance_binary.write_bytes(b"test conformance binary identity")
        self.conformance_binary.chmod(0o755)
        self.output = self.root / "report"

    def run_checker(self, runner: TypedProtocolRunner) -> dict:
        return tier_check.check_tier(output=self.output, binary=self.binary,
                                     conformance_binary=self.conformance_binary,
                                     diagnostic_binary=self.diagnostic_binary, runner=runner)

    def test_p014_three_semantic_cases_run_against_reference_and_decoded(self) -> None:
        runner = TypedProtocolRunner()
        report = self.run_checker(runner)
        self.assertTrue(report["passed"], report["errors"])
        self.assertEqual(report["counts"]["semantic_cases"], 3)
        self.assertEqual(report["counts"]["mode_runs_executed"], 6)
        self.assertEqual(report["counts"]["state_mode_runs_executed"], 6)
        self.assertEqual(report["counts"]["conformance_mode_runs_executed"], 4)
        self.assertEqual(report["counts"]["state_fuel_probes_executed"], 68)
        self.assertEqual(report["fuel_boundaries"]["state_normal"]["minimal_success_fuel"],
                         {"reference": 7, "decoded": 7})
        self.assertEqual(len(report["cursor_checks"]), 1)
        self.assertEqual(report["counts"]["skipped"], 0)
        self.assertEqual(report["counts"]["unsupported"], 0)
        self.assertEqual([case["id"] for case in report["cases"]], list(tier_check.INTEGRATION_CASES))
        self.assertTrue(all(case["equal"] for case in report["cases"]))
        typed = [command for command in runner.commands if len(command) > 1 and command[1] == "--typed-case"]
        self.assertEqual(len(typed), 6)
        self.assertEqual({command[-2] for command in typed}, {"decoded", "reference"})
        snapshot = [command for command in runner.commands if len(command) > 1 and command[1] == "--snapshot"]
        self.assertEqual(len(snapshot), 1)
        diagnostics = [command for command in runner.commands if len(command) > 1 and command[1] == "--diagnostic"]
        self.assertEqual(len(diagnostics), 3)
        self.assertEqual(len(report["decoded_source_op_diagnostics"]), 3)
        self.assertEqual(len(report["state_checks"]), 6)
        self.assertEqual(len(report["conformance"]), 4)
        saved = json.loads((self.output / "report.json").read_text())
        self.assertEqual(saved["binary"]["sha256"], report["binary"]["sha256"])
        self.assertEqual(saved["fixtures"], report["fixtures"])
        self.assertTrue(saved["implementation_sources"])

    def test_p014_missing_reference_mode_is_a_failure_not_unsupported_mask(self) -> None:
        report = self.run_checker(TypedProtocolRunner(modes=("decoded",)))
        self.assertFalse(report["passed"])
        self.assertEqual(report["counts"]["mode_runs_executed"], 3)
        self.assertEqual(report["counts"]["failed"], 4)
        self.assertEqual(report["counts"]["unsupported"], 0)
        self.assertIn("not built/forwarded", report["errors"][0])

    def test_p014_mode_process_error_fails_case_and_is_not_skipped(self) -> None:
        report = self.run_checker(TypedProtocolRunner(failure_mode="reference"))
        self.assertFalse(report["passed"])
        self.assertEqual(report["counts"]["mode_runs_executed"], 3)
        self.assertEqual(report["counts"]["failed"], 3)
        self.assertEqual(report["counts"]["skipped"], 0)
        self.assertEqual(report["counts"]["unsupported"], 0)

    def test_p014_reference_decoded_mismatch_fails(self) -> None:
        report = self.run_checker(TypedProtocolRunner(mismatch=True))
        self.assertFalse(report["passed"])
        self.assertTrue(any("result=" in error and "decoded" in error for error in report["errors"]))
        self.assertTrue(any(not case["equal"] for case in report["cases"]))

    def test_p014_snapshot_requires_actual_p018_roundtrip(self) -> None:
        report = self.run_checker(TypedProtocolRunner(bad_snapshot=True))
        self.assertFalse(report["passed"])
        self.assertTrue(any("snapshot" in error for error in report["errors"]))
        self.assertIsNone(report["snapshot"])

    def test_p014_full_memory_state_mismatch_fails(self) -> None:
        report = self.run_checker(TypedProtocolRunner(bad_state=True))
        self.assertFalse(report["passed"])
        self.assertTrue(any("full memory state differs" in error for error in report["errors"]))

    def test_p014_reference_is_not_a_candidate_tier(self) -> None:
        runner = TypedProtocolRunner()
        with self.assertRaisesRegex(tier_check.TierCheckError, "comparison baseline"):
            tier_check.check_tier(tier="reference", output=self.output,
                                  binary=self.binary, runner=runner)
        self.assertEqual(runner.commands, [])

    def test_p014_existing_output_is_rejected_before_build_or_run(self) -> None:
        self.output.mkdir()
        report = self.output / "report.json"
        report.write_text("historical evidence")
        calls = []
        with self.assertRaisesRegex(tier_check.TierCheckError, "refusing to overwrite"):
            tier_check.check_tier(output=self.output, builder=lambda *a, **k: calls.append(a))
        self.assertEqual(calls, [])
        self.assertEqual(report.read_text(), "historical evidence")

    def test_p014_reference_build_records_forwarded_feature(self) -> None:
        calls = []

        def builder(command, **kwargs):
            calls.append((command, kwargs))
            if command == tier_check.REFERENCE_BUILD:
                self.binary.write_bytes(b"reference-feature build output")
                target = self.binary
            elif command == tier_check.CONFORMANCE_BUILD:
                target = self.root / "wtime-conformance"
            else:
                target = self.root / "wtime-bench-counters"
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b"built feature output")
            target.chmod(0o755)
            return Completed({}, returncode=0)

        with patch.object(tier_check, "DEFAULT_CONFORMANCE", self.root / "wtime-conformance"), \
                patch.object(tier_check, "DEFAULT_COUNTERS_BINARY", self.root / "wtime-bench-counters"):
            info = tier_check._build_reference(self.binary, builder)
        self.assertEqual(info["feature_sets"]["policy"], ["execution-reference"])
        self.assertEqual(info["feature_sets"]["benchmark_harness"], ["runtime-only"])
        self.assertEqual(len(info["commands"]), 3)
        feature_arg = calls[0][0][calls[0][0].index("--features") + 1]
        self.assertIn("execution-reference", feature_arg)
        self.assertIn("runtime-only", feature_arg)
        conformance_features = calls[1][0][calls[1][0].index("--features") + 1]
        self.assertEqual(conformance_features, "execution-reference,opcode-coverage")
        diagnostic_features = calls[2][0][calls[2][0].index("--features") + 1]
        self.assertIn("execution-reference", diagnostic_features)
        self.assertIn("performance-counters", diagnostic_features)
        self.assertIn("runtime-only", diagnostic_features)
        self.assertEqual(calls[0][1]["cwd"], tier_check.ROOT)


if __name__ == "__main__":
    unittest.main()
