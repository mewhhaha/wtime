from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
PRODUCER_PATH = Path(__file__).resolve().parents[1] / "producers/p126_memory.py"
spec = importlib.util.spec_from_file_location("p126_memory_under_test", PRODUCER_PATH)
p126 = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(p126)


def within(root: Path, path: Path) -> Path:
    resolved_root, resolved = root.resolve(), path.resolve()
    if resolved == resolved_root or resolved_root not in resolved.parents:
        raise AssertionError(f"synthetic output escaped temporary root: {resolved}")
    return resolved


def observation(entry: dict, mode: str = "decoded", stage: str = "prepare"):
    row_id = entry["id"]
    value = entry["params"][0]["baseline"]
    expected = p126.memory_workloads.expected_memory(row_id, value)
    image = bytearray(expected["initial_pages"] * 65536)
    begin, end = expected["range"]
    image[begin:end] = bytes.fromhex(expected["bytes_hex"])
    profile = p126.memory_scaling._profile(entry)
    driver = p126.memory_scaling._driver(entry)
    outcome = p126.memory_scaling._expected_outcome(expected)
    return expected, {
        "schema_version": 2, "case": row_id, "stage": stage,
        "input": f"i32:{value}", "expected": outcome, "result": outcome,
        "mode": mode, "profile": profile, "driver": driver,
        "batch": 3 if stage == "warm_call" else 1, "ns_per_operation": 1.0,
        "trap_kind": None, "trap_message": None, "memory_pages": expected["initial_pages"],
        "memory_page_size_log2": 16, "memory_hex": bytes(image).hex(),
    }


class P126ProducerTests(unittest.TestCase):
    def test_exact_alias_rows_and_stages_are_required(self):
        registry, rows = p126._load_rows()
        self.assertEqual(len(rows), 18)
        self.assertEqual({row["params"][0]["baseline"] for row in rows}, set(p126.COUNTS))
        retained = [row for row in registry.entries()
                    if row["id"] != "memory_widths_predictable_0"]
        with mock.patch.object(p126.memory_workloads, "entries", return_value=retained):
            with self.assertRaisesRegex(p126.P126Error, "missing/duplicated"):
                p126._load_rows()

    def test_full_memory_oracle_rejects_altered_model_image(self):
        entry = next(row for row in p126.memory_workloads.entries()
                     if row["id"] == "memory_widths_predictable_0")
        expected, wire = observation(entry)
        proof = p126.memory_scaling._validate_observation(
            wire, entry, 0, expected, "decoded", "mvp", "plain", stage="prepare", expected_batch=1)
        self.assertEqual(proof["full_memory_sha256"], expected["full_memory_sha256"])
        changed = copy.deepcopy(expected)
        changed["full_memory_sha256"] = "0" * 64
        with self.assertRaisesRegex(p126.memory_scaling.BaselineError, "full-memory hash mismatch"):
            p126.memory_scaling._validate_observation(
                wire, entry, 0, changed, "decoded", "mvp", "plain", stage="prepare", expected_batch=1)

    def test_protocol_requires_requested_mode_and_uninstrumented_binary(self):
        with tempfile.TemporaryDirectory(prefix="p126-protocol-") as temp:
            root = Path(temp)
            binary = root / "fake-binary"
            binary.write_bytes(b"mock executable")
            for index, (response, message) in enumerate([
                ({"schema_version": 2, "instrumented": False, "modes": ["compact"],
                  "features": [p126.PROFILE]}, "mode/instrumentation/profile"),
                ({"schema_version": 2, "instrumented": True, "modes": ["decoded"],
                  "features": [p126.PROFILE]}, "mode/instrumentation/profile"),
            ]):
                completed = mock.Mock(returncode=0, stdout=json.dumps(response), stderr="")
                output = root / f"out-{index}"
                output.mkdir()
                with mock.patch.object(p126, "_run", return_value=completed):
                    with self.assertRaisesRegex(p126.P126Error, message):
                        p126._protocol(binary, "decoded", output)

    def test_staged_driver_argv_preserves_stage_and_batch_fields(self):
        _, rows = p126._load_rows()
        entry = next(row for row in rows if row["id"] == "memory_widths_predictable_0")
        with tempfile.TemporaryDirectory(prefix="p126-argv-") as temp:
            root = Path(temp)
            output = root / "out"
            output.mkdir()
            binary, fixture = root / "fake-binary", root / "fake-fixture.wasm"
            completed = mock.Mock(returncode=0, stdout="{}", stderr="")
            with mock.patch.object(p126, "_run", return_value=completed) as run:
                command, data = p126._raw_memory(binary, fixture, entry, 0, "i32:0",
                                                  "decoded", "mvp", "plain", "warm_call", 3,
                                                  output, "decoded.test.warm")
            self.assertEqual(command, [str(binary), "--memory-staged", str(fixture), entry["id"],
                                       "i32:0", "i32:0", "warm_call", "3", "decoded", "mvp", "plain"])
            self.assertEqual(run.call_args.args[0], command)
            self.assertEqual(data, {})

    def test_wrong_memory_response_is_archived_before_oracle_failure(self):
        entry = next(row for row in p126.memory_workloads.entries()
                     if row["id"] == "memory_widths_predictable_0")
        expected, wire = observation(entry)
        wire["memory_hex"] = wire["memory_hex"][:-2] + "01"
        with tempfile.TemporaryDirectory(prefix="p126-raw-response-") as temp:
            root = Path(temp)
            output = root / "out"
            output.mkdir()
            binary, fixture = root / "binary", root / "fixture.wasm"
            label = "decoded.memory_widths_predictable_0.prepare"
            completed = mock.Mock(returncode=0, stdout=json.dumps(wire), stderr="mock stderr")
            with mock.patch.object(p126, "_run", return_value=completed):
                command, retained = p126._raw_memory(
                    binary, fixture, entry, 0, "i32:0", "decoded", "mvp", "plain",
                    "prepare", 1, output, label)
                with self.assertRaisesRegex(p126.memory_scaling.BaselineError, "full-memory hash mismatch"):
                    p126.memory_scaling._validate_observation(
                        retained, entry, 0, expected, "decoded", "mvp", "plain",
                        stage="prepare", expected_batch=1)
            argv_path = within(root, output / "commands" / f"{label}.argv.json")
            status_path = within(root, output / "commands" / f"{label}.status.json")
            stdout_path = within(root, output / "commands" / f"{label}.stdout")
            raw_path = within(root, output / "observations/raw" / f"{label}.json")
            self.assertTrue(argv_path.is_file())
            self.assertEqual(json.loads(status_path.read_text())["exit_code"], 0)
            self.assertEqual(stdout_path.read_text(), json.dumps(wire))
            self.assertEqual(json.loads(raw_path.read_text()), wire)
            self.assertIn("--memory-staged", json.loads(argv_path.read_text())["argv"])

    def test_containment_refuses_escape_before_any_write(self):
        with tempfile.TemporaryDirectory(prefix="p126-contained-") as temp:
            root = Path(temp) / "out"
            root.mkdir()
            target = within(root, root / "a/b.json")
            self.assertTrue(target.is_relative_to(root.resolve()))
            with self.assertRaisesRegex(p126.P126Error, "escapes or aliases"):
                p126._contained(root, root.parent / "outside.json")

    def test_source_drift_comparison_is_exact(self):
        before = {"tools/wtime-bench/memory_workloads.py": "a" * 64}
        after = dict(before)
        p126._require_stable(before, after)
        after["tools/wtime-bench/memory_workloads.py"] = "b" * 64
        with self.assertRaisesRegex(p126.P126Error, "source identity changed"):
            p126._require_stable(before, after)

    def test_warm_call_uses_batch_three_and_full_state(self):
        entry = next(row for row in p126.memory_workloads.entries()
                     if row["id"] == "memory_widths_predictable_0")
        expected, wire = observation(entry, stage="warm_call")
        proof = p126.memory_scaling._validate_observation(
            wire, entry, 0, expected, "decoded", "mvp", "plain", stage="warm_call", expected_batch=3)
        self.assertEqual(proof["batch"], 3)
        with self.assertRaises(p126.memory_scaling.BaselineError):
            p126.memory_scaling._validate_observation(
                {**wire, "batch": 1}, entry, 0, expected, "decoded", "mvp", "plain",
                stage="warm_call", expected_batch=3)


if __name__ == "__main__":
    unittest.main()
