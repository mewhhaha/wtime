"""Contained mock tests for the P128 source-current proof producer.

These tests never start Rust binaries, encoder, validator, Cargo, or guest code.
The fake runner writes only beneath a TemporaryDirectory workspace.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parents[1]
DRAFT = HERE.parents[1]
BENCH = DRAFT / "tools/wtime-bench"
TEST_DIR = HERE / "tests"
ADMITTED = TEST_DIR / "data/admitted_wasm"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(module)
    return module


class FakeWorkspace:
    def __init__(self, root: Path):
        self.root = root
        self.bench = root / "tools/wtime-bench"
        self.producer = root / "tools/verification/producers/p128_scalar.py"
        self.bench.mkdir(parents=True)
        driver = self.bench / "src/scalar_driver.rs"
        driver.parent.mkdir(parents=True)
        driver.write_text("// synthetic scalar command source\n", encoding="utf-8")
        (root / "tools/verification/producers").mkdir(parents=True)
        (root / "docs/benchmarks/performance").mkdir(parents=True)
        for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
            path = root / f"crates/{crate}/src/lib.rs"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"// synthetic source for {crate}\n", encoding="utf-8")
        shutil.copyfile(BENCH / "scalar_p128_workloads.py", self.bench / "scalar_p128_workloads.py")
        shutil.copyfile(BENCH / "p128_scalar_scaling.py", self.bench / "p128_scalar_scaling.py")
        shutil.copyfile(BENCH / "p128_scalar_scaling.py", self.bench / "focused_compare.py")
        shutil.copyfile(self.producer_source, self.producer)
        if str(self.bench) not in sys.path:
            sys.path.insert(0, str(self.bench))
        fixture_dir = self.bench / "fixtures/scalar"
        fixture_dir.mkdir(parents=True)
        model_source = load_module("p128_test_fixture_model", BENCH / "scalar_p128_workloads.py")
        for metadata in model_source.FIXTURES.values():
            wasm_testdata = ADMITTED / metadata["file"].replace(".wat", ".wasm")
            if digest(wasm_testdata) != metadata["wasm_sha256"]:
                raise AssertionError(f"mock encoder input hash mismatch: {wasm_testdata.name}")
        for item in (BENCH / "fixtures/scalar").glob("*.wat"):
            shutil.copyfile(item, fixture_dir / item.name)
        registry = root / "tools/wtime-bench/workloads.json"
        shutil.copyfile(BENCH / "workloads.json", registry)
        self.registry = registry
        self.tools = root / "target/pinned-tools"
        self.tools.mkdir(parents=True)
        self.encoder = self.tools / "encoder"
        self.validator = self.tools / "validator"
        self.encoder.write_bytes(b"synthetic pinned encoder executable\n")
        self.validator.write_bytes(b"synthetic pinned validator executable\n")
        self.receipt = root / "target/pinned-tools/source-receipt.json"
        self.receipt.write_text('{"status":"retained-test-input"}\n', encoding="utf-8")
        self.lock = root / "docs/benchmarks/performance/P125-fixture-tools.json"
        lock_data = {
            "schema": 1,
            "source_receipt": {"path": str(self.receipt), "sha256": digest(self.receipt)},
            "tools": {
                "encoder": {"path": str(self.encoder), "sha256": digest(self.encoder)},
                "validator": {"path": str(self.validator), "sha256": digest(self.validator)},
            },
        }
        self.lock.write_text(json.dumps(lock_data, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        self.lock_sha = digest(self.lock)
        binaries = root / "target/production"
        binaries.mkdir(parents=True)
        self.records = {}
        compiled = self.compiled_sources()
        runtime = {k: v for k, v in compiled.items() if k.startswith("crates/wtime-runtime/src/")}
        core = {k: v for k, v in compiled.items()
                if k.startswith(("crates/wtime-runtime/src/", "crates/wtime-types/src/",
                                 "crates/wtime-decode/src/", "crates/wtime-validate/src/"))
                and k.endswith(".rs")}
        for mode, features in (("decoded", []), ("compact", ["compact-executor"])):
            protocol = {
                "schema_version": 2, "engine": "wtime", "modes": [mode],
                "features": ["mvp"], "instrumented": False,
            }
            binary = binaries / f"wtime-bench-{mode}"
            binary.write_bytes(f"synthetic {mode} binary\n".encode())
            protocol_path = root / "target/protocol" / f"{mode}.json"
            protocol_path.parent.mkdir(exist_ok=True)
            protocol_path.write_text(json.dumps(protocol, sort_keys=True) + "\n", encoding="utf-8")
            record = {
                "schema": "wtime-runtime-only-production-v1", "phase": "build", "mode": mode,
                "exit_code": 0, "source_stable": True, "instrumented": False,
                "runtime_features": features, "feature_flags": features,
                "harness_features": ["runtime-only"],
                "cargo_features": ["runtime-only", *features],
                "command": ["cargo", "build", "--locked", "--offline", "--no-default-features",
                            "--features", ",".join(["runtime-only", *features])],
                "source_identities": compiled, "runtime_source_map": runtime,
                "core_source_map": core,
                "binary_path": str(binary), "binary_sha256": digest(binary),
                "protocol": protocol, "protocol_exit": 0,
                "protocol_command": [str(binary), "--protocol"],
                "protocol_stdout": str(protocol_path), "protocol_sha256": digest(protocol_path),
            }
            record_path = root / "target/production-records" / f"{mode}.json"
            record_path.parent.mkdir(exist_ok=True)
            record_path.write_text(json.dumps(record, sort_keys=True) + "\n", encoding="utf-8")
            self.records[mode] = record_path
        self.compiled = compiled

    @property
    def producer_source(self) -> Path:
        return HERE / "producers/p128_scalar.py"

    def compiled_sources(self) -> dict[str, str]:
        result = {}
        for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
            rel = f"crates/{crate}/src/lib.rs"
            result[rel] = digest(self.root / rel)
        for rel in (
            "tools/wtime-bench/scalar_p128_workloads.py",
            "tools/wtime-bench/p128_scalar_scaling.py",
            "tools/wtime-bench/focused_compare.py",
            "tools/wtime-bench/src/scalar_driver.rs",
            "tools/wtime-bench/workloads.json",
        ):
            result[rel] = digest(self.root / rel)
        for fixture in sorted((self.bench / "fixtures/scalar").glob("*.wat")):
            result[fixture.relative_to(self.root).as_posix()] = digest(fixture)
        return dict(sorted(result.items()))

    def source_reader(self, root: Path):
        assert root == self.root
        compiled = self.compiled_sources()
        proof = dict(compiled)
        proof["tools/verification/producers/p128_scalar.py"] = digest(self.producer)
        proof["docs/benchmarks/performance/P125-fixture-tools.json"] = digest(self.lock)
        return compiled, dict(sorted(proof.items()))


class MockCommands:
    def __init__(self, *, mismatch_label: str | None = None, fail_label: str | None = None,
                 mutate_path: Path | None = None):
        self.calls: list[list[str]] = []
        self.mismatch_label = mismatch_label
        self.fail_label = fail_label
        self.mutate_path = mutate_path
        self.mutated = False

    def __call__(self, command, cwd, timeout):
        command = list(command)
        self.calls.append(command)
        cwd = Path(cwd)
        if command[-1:] == ["--protocol"]:
            if self.mutate_path is not None and not self.mutated:
                self.mutate_path.write_bytes(self.mutate_path.read_bytes() + b";; drift\n")
                self.mutated = True
            binary = Path(command[0])
            mode = "compact" if "compact" in binary.name else "decoded"
            out = json.dumps({"schema_version": 2, "engine": "wtime", "modes": [mode],
                              "features": ["mvp"], "instrumented": False}) + "\n"
            return subprocess.CompletedProcess(command, 0, out, "")
        if "encode" in command:
            wat = Path(command[2])
            wasm = Path(command[command.index("-o") + 1])
            shutil.copyfile(ADMITTED / f"{wat.stem}.wasm", wasm)
            return subprocess.CompletedProcess(command, 0, "encoded\n", "")
        if "check" in command:
            return subprocess.CompletedProcess(command, 0, "valid\n", "")
        if "--scalar-driver" in command:
            case = command[3]
            n = int(command[4])
            expected = int(command[5])
            stage, batch, mode, profile, driver = command[6:]
            if self.fail_label == f"{mode}.{case}.{stage}":
                return subprocess.CompletedProcess(command, 9, "untrusted partial\n", "mock failure\n")
            result = expected
            if self.mismatch_label == f"{mode}.{case}.{stage}":
                result += 1
            out = {
                "schema_version": 2, "case": case, "stage": stage,
                "input": f"i32:{n}", "expected": f"i32:{expected}", "result": f"i32:{result}",
                "mode": mode, "profile": profile, "driver": driver, "diagnostic": False,
                "batch": int(batch), "ns_per_operation": 42.0,
            }
            return subprocess.CompletedProcess(command, 0, json.dumps(out) + "\n", "")
        raise AssertionError(f"unexpected command: {command!r}")


class P128ProducerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p128-contained-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.ws = FakeWorkspace(self.root)
        sys.modules.pop("p128_scalar_scaling", None)
        self.producer = load_module("p128_producer_test", self.ws.producer)
        self.scaling = self.producer.p128_scalar_scaling

    def run_proof(self, output: Path, mock: MockCommands):
        with patch.object(self.scaling, "P125_LOCK_SHA256", self.ws.lock_sha):
            return self.producer.run(self.ws.records["decoded"], self.ws.records["compact"], output,
                                     root=self.root, source_reader=self.ws.source_reader,
                                     command_runner=mock)

    def test_complete_matrix_and_all_outputs_are_contained(self):
        output = self.root / "target/proofs/success"
        mock = MockCommands()
        report = self.run_proof(output, mock)
        self.assertEqual(report["counts"]["observations"], 180)
        self.assertEqual(report["counts"]["observation_commands"], 180)
        self.assertEqual(report["counts"]["by_mode"], {"decoded": 90, "compact": 90})
        self.assertEqual(report["counts"]["by_stage"], {stage: 36 for stage in self.scaling.STAGES})
        self.assertTrue(all(item["batch"] == (3 if item["stage"] == "warm_call" else 1)
                            for item in report["observations"]))
        self.assertEqual(report["counts"]["bad_exits"], 0)
        self.assertEqual(report["counts"]["skipped"], 0)
        self.assertEqual(len(mock.calls), 188)
        for path in output.rglob("*"):
            if path.is_file():
                self.assertIn(output, path.resolve().parents)
        self.assertEqual(json.loads((output / "report.json").read_text())["counts"]["skipped"], 0)

    def test_oracle_mismatch_preserves_raw_output_before_failure(self):
        output = self.root / "target/proofs/oracle-mismatch"
        label = "decoded.p128_identity_copy_n0.prepare"
        with self.assertRaises(self.scaling.P128Error):
            self.run_proof(output, MockCommands(mismatch_label=label))
        raw = output / "observations/raw" / f"{label}.stdout"
        self.assertTrue(raw.is_file())
        self.assertEqual(json.loads(raw.read_text())["result"], "i32:1")
        self.assertFalse((output / "report.json").exists())

    def test_stale_record_rejected_before_guest_commands(self):
        record_path = self.ws.records["compact"]
        original_record = record_path.read_bytes()
        record = json.loads(original_record)
        record["source_stable"] = False
        record_path.write_text(json.dumps(record))
        output = self.root / "target/proofs/stale-record"
        mock = MockCommands()
        with self.assertRaises(self.scaling.P128Error):
            self.run_proof(output, mock)
        self.assertEqual(mock.calls, [])

        record_path.write_bytes(original_record)
        drift_output = self.root / "target/proofs/source-drift"
        drift_mock = MockCommands(mutate_path=self.ws.bench / "fixtures/scalar/p128_identity_copy.wat")
        with self.assertRaises(self.scaling.P128Error):
            self.run_proof(drift_output, drift_mock)
        self.assertTrue(drift_mock.mutated)
        self.assertFalse(any("--scalar-driver" in call for call in drift_mock.calls))
        self.assertFalse((drift_output / "report.json").exists())

    def test_nonzero_command_output_and_status_are_retained(self):
        output = self.root / "target/proofs/command-failure"
        label = "decoded.p128_identity_copy_n0.prepare"
        with self.assertRaises(self.scaling.P128Error):
            self.run_proof(output, MockCommands(fail_label=label))
        self.assertEqual((output / "commands" / f"{label}.status.json").read_text(), '{\n  "exit_code": 9\n}\n')
        self.assertEqual((output / "commands" / f"{label}.stdout").read_text(), "untrusted partial\n")
        self.assertFalse((output / "report.json").exists())

    def test_output_escape_is_rejected_before_any_write(self):
        outside = self.root.parent / (self.root.name + "-escape")
        with self.assertRaises(self.scaling.P128Error):
            self.run_proof(outside, MockCommands())
        self.assertFalse(outside.exists())


if __name__ == "__main__":
    unittest.main()
