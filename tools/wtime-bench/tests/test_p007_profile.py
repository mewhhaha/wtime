"""Mocked profiling cases: capability gap, crash, and artifact separation."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import profile as profile_tool


def completed(returncode=0, stdout="", stderr=""):
    return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)


class ProfileTests(unittest.TestCase):
    def make_runner(self, *, crash=False):
        def runner(command, **kwargs):
            if command[0] in ("rustc", "cargo") and any(arg.startswith("-V") for arg in command[1:]):
                return completed(stdout=f"{command[0]} mocked version")
            if command[0] == "cargo":
                target = Path(command[command.index("--target-dir") + 1])
                binary = target / "release" / "wtime-bench"
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(("counter" if "--features" in command else "sampling").encode())
                return completed()
            if "--diagnostic" in command:
                if crash:
                    return completed(19, stderr="controlled fixture trap")
                expected = json.loads(command[4])
                response = {"schema_version": 1, "diagnostic": True, "result": expected,
                            "counters": {name: 1 for name in profile_tool.COUNTERS}}
                return completed(stdout=json.dumps(response))
            raise AssertionError(f"unexpected mocked command: {command}")
        return runner

    @staticmethod
    def no_tools(_name):
        return None

    def test_profiler_unavailable_still_produces_counter_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "profile"
            report = profile_tool.run_profile("scalar", output, runner=self.make_runner(), which=self.no_tools)
            self.assertTrue(report["passed"])
            self.assertEqual(len(report["counters"]), 3)
            self.assertTrue(all(not item["available"] for item in report["cpu_profiles"]))
            self.assertTrue(all(item["capability_gap"] == "perf executable is unavailable"
                                for item in report["cpu_profiles"]))

    def test_crashing_counter_workload_cannot_write_success_report(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "profile"
            with self.assertRaisesRegex(profile_tool.ProfileError, "workload command failed"):
                profile_tool.run_profile("scalar", output, runner=self.make_runner(crash=True),
                                         which=self.no_tools)
            saved = json.loads((output / "profile.json").read_text())
            self.assertIs(saved["passed"], False)
            self.assertIn("controlled fixture trap", saved["error"])

    def test_sampling_and_counter_artifacts_are_distinct_and_hash_bound(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "profile"
            report = profile_tool.run_profile("scalar", output, runner=self.make_runner(), which=self.no_tools)
            artifacts = report["metadata"]["artifacts"]
            self.assertTrue(artifacts["distinct_binaries"])
            self.assertNotEqual(artifacts["sampling"]["binary"], artifacts["counters"]["binary"])
            self.assertNotEqual(artifacts["sampling"]["binary_sha256"], artifacts["counters"]["binary_sha256"])
            self.assertTrue(all(row["binary_sha256"] == artifacts["counters"]["binary_sha256"]
                                for row in report["counters"]))
            self.assertTrue(all(row["fixture_sha256"] for row in report["counters"]))


if __name__ == "__main__":
    unittest.main()
