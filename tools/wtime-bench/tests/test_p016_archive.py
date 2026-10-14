import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "baseline.py"
SPEC = importlib.util.spec_from_file_location("p016_baseline", SCRIPT)
baseline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(baseline)


class ExecutionModeArchiveTests(unittest.TestCase):
    def root_with_features(self, root: Path, features: dict[str, list[str]],
                           runtime_features: dict[str, list[str]] | None = None) -> Path:
        bench = root / "tools/wtime-bench"
        runtime = root / "crates/wtime-runtime"
        bench.mkdir(parents=True)
        runtime.mkdir(parents=True)
        (bench / "Cargo.toml").write_text(
            "[features]\ndefault = []\n" + "".join(
                f'{name} = {json.dumps(values)}\n' for name, values in features.items()
            ), encoding="utf-8")
        declared_runtime = runtime_features or {
            v.removeprefix("wtime-runtime/"): []
            for xs in features.values() for v in xs if v.startswith("wtime-runtime/")
        }
        (runtime / "Cargo.toml").write_text(
            "[features]\ndefault = []\n" + "".join(
                f'{name} = {json.dumps(values)}\n' for name, values in declared_runtime.items()
            ), encoding="utf-8")
        return root

    def test_p016_stale_release_binary_is_rebuilt_before_archive_use(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bench = root / "tools/wtime-bench/target/release/wtime-bench"
            encoder = root / "target/debug/wtime-conformance"
            bench.parent.mkdir(parents=True)
            encoder.parent.mkdir(parents=True)
            bench.write_bytes(b"stale release")
            encoder.write_bytes(b"existing encoder")

            def fake_cargo(command, **kwargs):
                self.assertIn("--release", command)
                bench.write_bytes(b"fresh release")
                return subprocess.CompletedProcess(command, 0)

            with mock.patch.object(baseline.subprocess, "run", side_effect=fake_cargo) as run:
                result = baseline._ensure_builds(root, "decoded", [])
            self.assertEqual(run.call_count, 1)
            self.assertEqual(result[0].read_bytes(), b"fresh release")
            self.assertIn("cargo build", result[2])

    def test_p016_unsupported_or_unforwarded_mode_fails_before_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(root, {"execution-reference": ["wtime-runtime/execution-reference"]})
            self.assertEqual(baseline._mode_config(root, "reference"), ("reference", ["execution-reference"]))
            with self.assertRaisesRegex(ValueError, "unavailable"):
                baseline._mode_config(root, "compact")
            with self.assertRaisesRegex(ValueError, "P095"):
                baseline._mode_config(root, "native")

    def test_p016_experimental_accepts_transitive_bench_feature_aggregation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(
                root,
                {"experimental-executors": ["compact-executor", "register-executor"],
                 "compact-executor": ["wtime-runtime/compact-executor"],
                 "register-executor": ["wtime-runtime/register-executor"]},
                {"compact-executor": [], "register-executor": []},
            )
            self.assertEqual(
                baseline._mode_config(root, "experimental"),
                ("experimental", ["experimental-executors"]),
            )

    def test_p016_experimental_rejects_cycles_and_missing_runtime_forward(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(
                root,
                {"experimental-executors": ["compact-executor", "register-executor"],
                 "compact-executor": ["experimental-executors"],
                 "register-executor": ["wtime-runtime/register-executor"]},
                {"register-executor": []},
            )
            with self.assertRaisesRegex(ValueError, "cyclic benchmark Cargo feature graph"):
                baseline._mode_config(root, "experimental")

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(
                root,
                {"experimental-executors": ["compact-executor", "register-executor"],
                 "compact-executor": ["wtime-runtime/compact-executor"],
                 "register-executor": []},
                {"compact-executor": []},
            )
            with self.assertRaisesRegex(ValueError, "does not forward required runtime"):
                baseline._mode_config(root, "experimental")

    def test_p016_experimental_accepts_runtime_aggregator_feature(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(
                root,
                {"experimental-executors": ["wtime-runtime/experimental-executors"]},
                {"experimental-executors": ["compact-executor", "register-executor"],
                 "compact-executor": [], "register-executor": []},
            )
            self.assertEqual(
                baseline._mode_config(root, "experimental"),
                ("experimental", ["experimental-executors"]),
            )

    def test_p016_portable_alias_resolves_to_decoded_and_protocol_must_advertise_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.root_with_features(root, {})
            self.assertEqual(baseline._mode_config(root, "portable"), ("decoded", []))
            self.assertEqual(baseline._require_protocol_mode({"modes": ["decoded"]}, "decoded"), ["decoded"])
            with self.assertRaisesRegex(RuntimeError, "absent from benchmark --protocol"):
                baseline._require_protocol_mode({"modes": ["decoded"]}, "reference")

    def test_p016_mode_smoke_records_requested_policy_and_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "bench"
            fixture = root / "call_overhead.wasm"
            report = root / "runs/mode-smoke.json"
            binary.write_bytes(b"selected binary")
            fixture.write_bytes(b"fixture")
            payload = {"case": "call_overhead", "input": 7, "expected": 8, "result": 8,
                       "mode": "reference", "stage": "warm_call"}
            completed = subprocess.CompletedProcess([], 0, json.dumps(payload), "")
            with mock.patch.object(baseline.subprocess, "run", return_value=completed) as run:
                smoke = baseline._mode_smoke(binary, fixture, "reference", report)
            self.assertEqual(run.call_args.args[0][-1], "reference")
            self.assertEqual(smoke["mode"], "reference")
            self.assertEqual(smoke["fixture_sha256"], baseline.sha(fixture))
            self.assertEqual(smoke["binary_sha256"], baseline.sha(binary))
            self.assertEqual(smoke["report_sha256"], baseline.sha(report))


if __name__ == "__main__":
    unittest.main()
