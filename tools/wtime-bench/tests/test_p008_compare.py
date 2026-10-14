"""Mock-process tests for P008 scalar fixture construction and comparison."""
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
import baseline
import compare
import scalar_workloads
import workloads


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


def _archive(root, *, mode="reference"):
    archive = Path(root) / "baseline"
    files = {}
    for source in sorted((HERE / "src").glob("*.rs")):
        relative = source.relative_to(ROOT).as_posix()
        destination = archive / "source" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        key = "source/" + relative
        files[key] = _record(destination, "source", relative)

    binary = archive / "binary/wtime-bench"
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_bytes(b"mock current-source executable")
    binary.chmod(0o755)
    files["binary/wtime-bench"] = _record(binary, "release_benchmark_executable")
    files["binary/wtime-bench"]["mode_bits"] = stat.S_IMODE(binary.stat().st_mode)

    for case in ("call_overhead", "integer_loop", "memory_fill_scan", "branch_calls", "prepare_large"):
        source = HERE / "fixtures" / f"{case}.wasm"
        destination = archive / "fixtures" / f"{case}.wasm"
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        files[f"fixtures/{case}.wasm"] = _record(destination, "deterministic_fixture")

    protocol = archive / "source/tools/wtime-bench/src/protocol.rs"
    protocol_hash = _sha(protocol)
    manifest = {
        "schema": 1,
        "mode": mode,
        "execution_policy": {
            "requested_mode": mode,
            "resolved_mode": mode,
            "cargo_features": ["execution-reference"] if mode == "reference" else [],
            "protocol_modes": ["decoded", "portable", "reference"],
            "protocol_sha256": protocol_hash,
        },
        "mode_smoke": {"mode": mode, "binary_sha256": _sha(binary)},
        "files": files,
    }
    (archive / "manifest.json").write_text(json.dumps(manifest))
    return archive


def _scalar_entry(case="scalar_accumulator_n17"):
    return next(entry for entry in workloads.selection("scalar") if entry["id"] == case)


def _factory(baseline_path, *, bad_candidate_result=False):
    calls = []

    def process(executable, args, cpu, timeout=180):
        executable = Path(executable).resolve()
        side = "baseline" if executable == baseline_path.resolve() else "candidate"
        args = list(map(str, args))
        calls.append((side, args))
        if args == ["--protocol"]:
            return {
                "schema_version": 2,
                "engine": "wtime",
                "instrumented": False,
                "modes": ["decoded", "portable", "reference"],
                "features": ["mvp", "stable-core", "all-phase3-core"],
            }
        if args[0] == "--scalar-table-probe":
            _, _fixture, n, expected_a, expected_b, mode, profile = args
            return {
                "schema_version": 2,
                "case": "scalar_table_mutation",
                "driver": "table_mutation",
                "diagnostic": True,
                "status": "mutation_verified",
                "input": f"i32:{n}",
                "target_a": f"i32:{expected_a}",
                "target_b": f"i32:{expected_b}",
                "target_a_after_restore": f"i32:{expected_a}",
                "mode": mode,
                "profile": profile,
            }
        if args[0] != "--scalar-driver":
            raise AssertionError(args)
        _, _fixture, case, input_value, expected, stage, requested_batch, mode, profile, driver = args
        if driver in ("probe_null", "probe_mismatch", "depth_probe"):
            kind = {
                "probe_null": "NullReference",
                "probe_mismatch": "IndirectCallTypeMismatch",
                "depth_probe": "ResourceLimit",
            }[driver]
            return {
                "schema_version": 2, "case": case, "driver": driver,
                "diagnostic": True, "status": "trap_verified", "trap_kind": kind,
                "message": "call depth exhausted" if driver == "depth_probe" else kind,
                "mode": mode, "profile": profile,
            }
        if driver == "tail_probe":
            return {
                "schema_version": 2, "case": case, "driver": driver,
                "diagnostic": True, "status": "returned", "input": f"i32:{input_value}",
                "expected": f"i32:{expected}", "result": f"i32:{expected}",
                "mode": mode, "profile": profile,
            }
        result = f"i32:{expected}"
        if bad_candidate_result and side == "candidate":
            result = f"i32:{int(expected) + 1}"
        return {
            "schema_version": 2, "case": case, "stage": stage,
            "input": f"i32:{input_value}", "expected": f"i32:{expected}",
            "result": result,
            "batch": 4 if int(requested_batch) == 0 else int(requested_batch),
            # Synthetic timing keeps oracle/CLI mocks within the comparator's
            # 20 ms calibration target at its 100k cap; this is no speed evidence.
            "ns_per_operation": 500.0, "mode": mode, "profile": profile,
            "driver": driver, "diagnostic": False,
        }

    return process, calls


class P008CompareTests(unittest.TestCase):
    def _run(self, root, entry, *, process_override=None):
        entries = entry if isinstance(entry, list) else [entry]
        archive = _archive(root)
        candidate = root / "candidate"
        candidate.write_bytes(b"mock candidate")
        candidate.chmod(0o755)
        process, calls = _factory(archive / "binary/wtime-bench")
        if process_override is not None:
            process = process_override(process, calls)
        args = types.SimpleNamespace(
            baseline=archive,
            candidate=candidate,
            selection="scalar",
            output=root / "comparison",
            pairs=1,
            seed=20261011,
            cpu=min(os.sched_getaffinity(0)),
            candidate_mode="decoded",
        )
        with mock.patch.object(workloads, "selection", return_value=entries), \
                mock.patch.object(compare, "process", side_effect=process), \
                mock.patch.object(compare.subprocess, "check_output", return_value="mock rustc -Vv"), \
                contextlib.redirect_stdout(io.StringIO()):
            report = compare.run(args)
        return report, calls, archive

    def test_scalar_fixture_is_encoded_and_typed_wire_rows_are_normalized(self):
        with tempfile.TemporaryDirectory() as directory:
            report, calls, _ = self._run(Path(directory), _scalar_entry())
            self.assertTrue(report["passed"])
            fixture = report["metadata"]["fixtures"][0]
            self.assertEqual(fixture["source_sha256"], _scalar_entry()["source"]["sha256"])
            self.assertEqual(fixture["profile"], "mvp")
            self.assertEqual(fixture["driver"], "plain")
            self.assertEqual(fixture["sha256"], _scalar_entry()["source"]["wasm_sha256"])
            pair = report["runs"][0]["pairs"][0]
            self.assertEqual(pair["baseline"]["input"], 17)
            self.assertEqual(pair["baseline"]["expected"], scalar_workloads.oracle("scalar_accumulator_n17", 17))
            self.assertEqual(pair["baseline"]["result"], pair["baseline"]["expected"])
            self.assertEqual(pair["baseline"]["wire_observation"]["result"], "i32:136")
            self.assertEqual(report["metadata"]["settings"]["features"], "mvp")
            self.assertTrue(any(args and args[0] == "--scalar-driver" for _, args in calls))

    def test_mixed_scalar_profiles_are_reported_per_workload(self):
        entries = [_scalar_entry("scalar_accumulator_n17"),
                   _scalar_entry("scalar_tail_calls_n17")]
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self._run(Path(directory), entries)
            settings = report["metadata"]["settings"]
            self.assertEqual(settings["features"], "per-workload")
            self.assertEqual(settings["feature_profiles"], ["all-phase3-core", "mvp"])
            by_id = {fixture["id"]: fixture for fixture in report["metadata"]["fixtures"]}
            self.assertEqual(by_id["scalar_accumulator_n17"]["profile"], "mvp")
            self.assertEqual(by_id["scalar_tail_calls_n17"]["profile"], "all-phase3-core")

    def test_scalar_semantic_oracle_mismatch_fails_comparison(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            candidate = root / "candidate"
            candidate.write_bytes(b"mock candidate")
            candidate.chmod(0o755)
            process, _ = _factory(archive / "binary/wtime-bench", bad_candidate_result=True)
            args = types.SimpleNamespace(
                baseline=archive, candidate=candidate, selection="scalar", output=root / "comparison",
                pairs=1, seed=20261011, cpu=min(os.sched_getaffinity(0)), candidate_mode="decoded",
            )
            with mock.patch.object(workloads, "selection", return_value=[_scalar_entry()]), \
                    mock.patch.object(compare, "process", side_effect=process), \
                    mock.patch.object(compare.subprocess, "check_output", return_value="mock rustc -Vv"), \
                    contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(compare.ComparisonError, "scalar oracle mismatch"):
                    compare.run(args)

    def test_scalar_wat_hash_mismatch_is_rejected_before_runner_calls(self):
        entry = _scalar_entry()
        entry["source"]["sha256"] = "0" * 64
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            candidate = root / "candidate"
            candidate.write_bytes(b"mock candidate")
            candidate.chmod(0o755)
            args = types.SimpleNamespace(
                baseline=archive, candidate=candidate, selection="scalar", output=root / "comparison",
                pairs=1, seed=20261011, cpu=min(os.sched_getaffinity(0)), candidate_mode="decoded",
            )
            with mock.patch.object(workloads, "selection", return_value=[entry]), \
                    mock.patch.object(compare, "process") as runner, \
                    mock.patch.object(compare.subprocess, "check_output", return_value="mock rustc -Vv"):
                with self.assertRaisesRegex(compare.ComparisonError, "WAT source hash mismatch"):
                    compare.run(args)
            runner.assert_not_called()

    def test_current_source_archive_must_bind_each_harness_leaf(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            leaf = archive / "source/tools/wtime-bench/src/main.rs"
            leaf.write_bytes(b"different archived harness leaf")
            manifest_path = archive / "manifest.json"
            manifest = json.loads(manifest_path.read_text())
            manifest["files"]["source/tools/wtime-bench/src/main.rs"] = _record(
                leaf, "source", "tools/wtime-bench/src/main.rs"
            )
            manifest_path.write_text(json.dumps(manifest))
            verified = baseline.verify_archive(archive)
            with self.assertRaisesRegex(compare.ComparisonError, "different harness leaf: main.rs"):
                compare._verify_scalar_harness_sources(archive, verified)

    def test_legacy_adapter_rejects_scalar_selection_before_runner_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = _archive(root)
            candidate = root / "candidate"
            candidate.write_bytes(b"mock candidate")
            candidate.chmod(0o755)
            args = types.SimpleNamespace(
                baseline=archive, candidate=candidate, selection="scalar", output=root / "comparison",
                pairs=1, seed=20261011, cpu=min(os.sched_getaffinity(0)), candidate_mode="decoded",
            )
            identity = {
                "frozen": archive,
                "executable": archive / "binary/wtime-bench",
                "mode": "decoded",
                "protocol_sha256": _sha(HERE / "src/protocol.rs"),
                "kind": "legacy-api-adapter",
            }
            with mock.patch.object(workloads, "selection", return_value=[_scalar_entry()]), \
                    mock.patch.object(compare, "_baseline_identity", return_value=identity), \
                    mock.patch.object(compare, "process") as runner:
                with self.assertRaisesRegex(compare.ComparisonError, "unsupported by the legacy-api"):
                    compare.run(args)
            runner.assert_not_called()

    def test_table_mutation_null_and_mismatch_probes_run_for_both_processes(self):
        entry = _scalar_entry("scalar_table_mutation_n17")
        with tempfile.TemporaryDirectory() as directory:
            report, calls, _ = self._run(Path(directory), entry)
            self.assertTrue(report["passed"])
            for side in ("baseline", "candidate"):
                commands = [args[0] for call_side, args in calls if call_side == side]
                self.assertIn("--scalar-table-probe", commands)
                scalar = [args for call_side, args in calls if call_side == side and args[0] == "--scalar-driver"]
                self.assertTrue(any(args[-1] == "probe_null" for args in scalar))
                self.assertTrue(any(args[-1] == "probe_mismatch" for args in scalar))


if __name__ == "__main__":
    unittest.main()
