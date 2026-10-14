"""Synthetic checks for the narrow P056 analysis reuse gate."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools/performance"))
import p056_revalidate_analysis as reuse
from verification.artifacts import ArtifactStore, digest, write_new


def synthetic_case(root: Path):
    root = root.resolve()
    store_root = root / "target/verification"
    prior = store_root / "runs/P056/analyze/attempt-0001"
    output = store_root / "runs/P056/analyze/attempt-0002"
    for path in (prior, output):
        path.mkdir(parents=True)
        assert path.resolve().is_relative_to(root)
    contract = root / "contract.json"
    contract.write_text("{}")
    assert contract.resolve().is_relative_to(root)
    store = ArtifactStore(store_root)
    inputs = {
        "components": {"analysis": {"sha256": "a"}},
        "definition": {"commands": [{"name": "portfolio"}, {"name": "statistics"},
                                      {"name": "seal-analysis"}]},
        "dependencies": {"measure": {"sha256": "m"}},
        "environment": {"variables": {}, "toolchain": {}}, "lane": "full",
        "runner_sources": {"runner.py": "r"}, "source_delta": None, "tools": {},
    }
    write_new(prior / "inputs.json", inputs)
    commands = []
    seal_streams = {}
    for name in ("portfolio", "statistics", "seal-analysis"):
        stdout, stderr = prior / (name + ".stdout"), prior / (name + ".stderr")
        stdout.write_text("" if name == "seal-analysis" else name + " output\n")
        stderr.write_text("")
        if name == "seal-analysis":
            seal_streams = {"stdout": stdout, "stderr": stderr}
        else:
            commands.append({"name": name, "exit_code": 0,
                             "stdout": store.add(stdout), "stderr": store.add(stderr)})
    output_bytes = {}
    for name in reuse.OUTPUTS:
        if name == "inventory.json":
            continue
        path = prior / name
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = ("raw " + name + "\n").encode()
        path.write_bytes(payload)
        output_bytes[name] = payload
    files = {}
    for path in sorted(p for p in prior.rglob("*") if p.is_file() and p.name not in ("inventory.json", "artifacts.tar.gz", "failure.json")):
        relative = path.relative_to(prior).as_posix()
        files[relative] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                           "bytes": path.stat().st_size, "mode_bits": path.stat().st_mode & 0o777}
    bundle_path = prior / "artifacts.tar.gz"
    with tarfile.open(bundle_path, "w:gz") as archive:
        for name in files:
            archive.add(prior / name, arcname=name, recursive=False)
    inventory = {"schema": "p046-stage-seal-v1", "status": "passed", "phase_root": str(prior),
                 "source_stable": True, "source_before": {"inputs": "stable"},
                 "source_after": {"inputs": "stable"},
                 "files": files, "file_count": len(files),
                 "bundle": {"sha256": hashlib.sha256(bundle_path.read_bytes()).hexdigest(),
                            "bytes": bundle_path.stat().st_size}}
    write_new(prior / "inventory.json", inventory)
    output_bytes["inventory.json"] = (prior / "inventory.json").read_bytes()
    seal_streams["stdout"].write_text("seal-analysis completed successfully\n")
    commands.append({"name": "seal-analysis", "exit_code": 0,
                     "stdout": store.add(seal_streams["stdout"]),
                     "stderr": store.add(seal_streams["stderr"])})
    failure = {"status": "failed", "input_key": digest(inputs), "error": reuse.EXPECTED_ERROR,
               "commands": commands}
    write_new(prior / "failure.json", failure)
    current = copy.deepcopy(inputs)
    current["components"]["analysis_revalidation"] = {"sha256": "new-helper-and-tests"}
    return store_root, prior, output, contract, inputs, failure, current, output_bytes


class P056AnalysisReuseTests(unittest.TestCase):
    def test_complete_postcheck_failure_reuses_analysis(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            store, prior, output, contract, _inputs, _failure, current, expected = synthetic_case(root)
            with mock.patch.object(reuse, "load_spec", return_value={}), \
                 mock.patch.object(reuse.Workflow, "_inputs", return_value=current):
                result = reuse.revalidate(prior, output, contract, store, repo=root)
            self.assertTrue(result["source_current"])
            self.assertEqual((result["paired_sample_events"], result["statistics_computations"],
                              result["bootstrap_computations"]), (0, 0, 0))
            self.assertEqual(set(current["components"]), {"analysis", "analysis_revalidation"})
            for name, payload in expected.items():
                target = output / name
                self.assertTrue(target.resolve().is_relative_to(root))
                self.assertEqual(target.read_bytes(), payload)
            self.assertTrue((output / "portfolio/report.json").is_file())
            self.assertEqual((prior / "seal-analysis.stdout").read_bytes(),
                             b"seal-analysis completed successfully\n")

    def test_changed_consumed_inputs_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            store, prior, output, contract, _inputs, _failure, current, _expected = synthetic_case(root)
            current["components"]["analysis"]["sha256"] = "changed"
            with mock.patch.object(reuse, "load_spec", return_value={}), \
                 mock.patch.object(reuse.Workflow, "_inputs", return_value=current):
                with self.assertRaisesRegex(ValueError, "components changed"):
                    reuse.revalidate(prior, output, contract, store, repo=root)
            current = copy.deepcopy(_inputs)
            current["components"]["analysis_revalidation"] = {"sha256": "new-helper-and-tests"}
            current["dependencies"]["measure"]["sha256"] = "changed"
            with self.assertRaisesRegex(ValueError, "analysis input changed: dependencies"):
                reuse.validate_records(_inputs, _failure, current)

    def test_failed_command_or_wrong_failure_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            store, prior, output, contract, inputs, failure, current, _expected = synthetic_case(root)
            bad = copy.deepcopy(failure)
            bad["commands"][1]["exit_code"] = 1
            with self.assertRaisesRegex(ValueError, "incomplete or did not all succeed"):
                reuse.validate_records(inputs, bad, current)
            bad = copy.deepcopy(failure)
            bad["error"] = "VerificationError: a different failure"
            with self.assertRaisesRegex(ValueError, "prior failure differs"):
                reuse.validate_records(inputs, bad, current)
            payload = prior / "statistics.json"
            payload.write_text("tampered\n")
            with mock.patch.object(reuse, "load_spec", return_value={}), \
                 mock.patch.object(reuse.Workflow, "_inputs", return_value=current):
                with self.assertRaisesRegex(ValueError, "prior inventory artifact changed"):
                    reuse.revalidate(prior, output, contract, store, repo=root)
            self.assertTrue(output.resolve().is_relative_to(root))
            with tempfile.TemporaryDirectory() as seal_name:
                root = Path(seal_name).resolve()
                store, prior, output, contract, _inputs, _failure, current, _expected = synthetic_case(root)
                (prior / "seal-analysis.stdout").write_text("tampered seal stream\n")
                with mock.patch.object(reuse, "load_spec", return_value={}), \
                     mock.patch.object(reuse.Workflow, "_inputs", return_value=current):
                    with self.assertRaises(ValueError):
                        reuse.revalidate(prior, output, contract, store, repo=root)


if __name__ == "__main__":
    unittest.main()
