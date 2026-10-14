import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import types
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import compare
import baseline


def _sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _record(path, role, source_path=None):
    path = Path(path)
    return {
        "sha256": _sha(path),
        "size_bytes": path.stat().st_size,
        "role": role,
        **({"source_path": source_path} if source_path else {}),
    }


def _archive(root, *, mode="reference", protocol_bytes=None, missing_mode=False,
             include_binary=True):
    archive = Path(root) / "baseline"
    files = {}
    archived_protocol = archive / "source/tools/wtime-bench/src/protocol.rs"
    archived_protocol.parent.mkdir(parents=True, exist_ok=True)
    protocol_bytes = protocol_bytes or (HERE / "src/protocol.rs").read_bytes()
    archived_protocol.write_bytes(protocol_bytes)
    files["source/tools/wtime-bench/src/protocol.rs"] = _record(
        archived_protocol, "source", "tools/wtime-bench/src/protocol.rs"
    )

    binary_hash = None
    if include_binary:
        binary = archive / "binary/wtime-bench"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"mock current-source benchmark executable")
        binary.chmod(0o755)
        binary_record = _record(binary, "release_benchmark_executable", "tools/wtime-bench/target/release/wtime-bench")
        binary_record["mode_bits"] = stat.S_IMODE(binary.stat().st_mode)
        files["binary/wtime-bench"] = binary_record
        binary_hash = binary_record["sha256"]

    fixtures = archive / "fixtures"
    fixtures.mkdir(parents=True, exist_ok=True)
    for case in ("call_overhead", "integer_loop", "memory_fill_scan", "branch_calls", "prepare_large"):
        source = HERE / "fixtures" / f"{case}.wasm"
        destination = fixtures / f"{case}.wasm"
        destination.write_bytes(source.read_bytes())
        files[f"fixtures/{case}.wasm"] = _record(destination, "deterministic_fixture")

    manifest = {
        "schema": 1,
        "mode": mode,
        "execution_policy": {
            "requested_mode": mode,
            "resolved_mode": mode,
            "cargo_features": ["execution-reference"] if mode == "reference" else [],
            "protocol_modes": ["decoded", "portable", "reference"],
            "protocol_sha256": _sha(archived_protocol),
        },
        "mode_smoke": {"mode": mode, "binary_sha256": binary_hash},
        "files": files,
    }
    if missing_mode:
        del manifest["execution_policy"]["resolved_mode"]
    (archive / "manifest.json").write_text(json.dumps(manifest))
    return archive


def _mock_process_factory(baseline_path, *, baseline_modes=("decoded", "portable", "reference")):
    calls = []

    def process(executable, args, cpu, timeout=180):
        executable = Path(executable).resolve()
        side = "baseline" if executable == baseline_path.resolve() else "candidate"
        if args == ["--protocol"]:
            return {"schema_version": 2, "engine": "wtime", "instrumented": False,
                    "modes": list(baseline_modes if side == "baseline" else ("decoded", "portable", "reference"))}
        calls.append((side, args[-1]))
        _, fixture, case, input_value, expected, stage, requested_batch, mode = args
        input_value = int(input_value)
        expected_value = int(expected)
        return {
            "schema_version": 2, "case": case, "stage": stage,
            "input": input_value, "expected": expected_value,
            "result": None if case == "prepare_large" else expected_value,
            "batch": 4 if int(requested_batch) == 0 else int(requested_batch),
            "ns_per_operation": 120.0, "mode": mode,
        }

    return process, calls


class CurrentSourceComparisonTests(unittest.TestCase):
    def test_p017_current_archive_runs_with_recorded_policy_and_mock_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            candidate = root / "candidate"
            candidate.write_bytes(b"mock candidate executable")
            candidate.chmod(0o755)
            process, calls = _mock_process_factory(archive / "binary/wtime-bench")
            args = types.SimpleNamespace(
                baseline=archive, candidate=candidate, selection="original-mvp",
                output=root / "comparison", pairs=1, seed=20261011,
                cpu=min(os.sched_getaffinity(0)), candidate_mode="decoded",
            )
            with mock.patch.object(compare, "process", side_effect=process), \
                    mock.patch.object(compare.subprocess, "check_output", return_value="mock rustc -Vv"), \
                    contextlib.redirect_stdout(io.StringIO()):
                report = compare.run(args)
            self.assertTrue(report["passed"])
            self.assertEqual(report["metadata"]["settings"]["baseline_mode"], "reference")
            self.assertIn(("baseline", "reference"), calls)
            self.assertIn(("candidate", "decoded"), calls)
            self.assertEqual(report["metadata"]["baseline_manifest_sha256"],
                             _sha(archive / "manifest.json"))
            self.assertEqual(report["metadata"]["frozen_manifest_sha256"],
                             _sha(archive / "manifest.json"))

    def test_p017_current_archive_protocol_must_match_live_protocol(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = _archive(directory, protocol_bytes=b"different archived protocol")
            manifest = baseline.verify_archive(archive)
            with self.assertRaisesRegex(compare.ComparisonError, "different protocol"):
                compare._baseline_identity(archive, manifest, HERE / "src/protocol.rs", baseline)

    def test_p017_current_archive_requires_explicit_mode_and_binary_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = _archive(directory, missing_mode=True)
            manifest = baseline.verify_archive(archive)
            with self.assertRaisesRegex(compare.ComparisonError, "resolved execution mode"):
                compare._baseline_identity(archive, manifest, HERE / "src/protocol.rs", baseline)

        with tempfile.TemporaryDirectory() as directory:
            archive = _archive(directory, include_binary=False)
            manifest = baseline.verify_archive(archive)
            with self.assertRaisesRegex(compare.ComparisonError, "benchmark binary identity"):
                compare._baseline_identity(archive, manifest, HERE / "src/protocol.rs", baseline)

    def test_p017_recorded_baseline_mode_must_be_supported_by_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            candidate = root / "candidate"
            candidate.write_bytes(b"mock candidate executable")
            candidate.chmod(0o755)
            process, _ = _mock_process_factory(
                archive / "binary/wtime-bench", baseline_modes=("decoded",)
            )
            args = types.SimpleNamespace(
                baseline=archive, candidate=candidate, selection="original-mvp",
                output=root / "comparison", pairs=1, seed=20261011,
                cpu=min(os.sched_getaffinity(0)), candidate_mode="decoded",
            )
            with mock.patch.object(compare, "process", side_effect=process), \
                    mock.patch.object(compare.subprocess, "check_output", return_value="mock rustc -Vv"):
                with self.assertRaisesRegex(compare.ComparisonError, "reference unavailable"):
                    compare.run(args)


if __name__ == "__main__":
    unittest.main()
