"""Synthetic source-binding checks for P135; all files stay in fresh temp roots."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.verification.producers import p135


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class P135ObservationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p135-source-binding-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bin_prod = self.root / "bin" / "production"
        self.bin_diag = self.root / "bin" / "diagnostic"
        self.fixture = self.root / "fixtures" / "counter.wasm"
        self.bin_prod.parent.mkdir(parents=True)
        self.fixture.parent.mkdir(parents=True)
        self.bin_prod.write_bytes(b"synthetic uninstrumented executable")
        self.bin_diag.write_bytes(b"synthetic diagnostic executable")
        self.fixture.write_bytes(b"synthetic fixture")
        self.observation = self.make_observation()

    def timing_row(self, binary: Path, strategy: str, calls: int) -> dict:
        argv = [str(binary), "--entry-batch", str(self.fixture), "p135_counter", "7", "8", str(calls),
                "register", strategy, "0" if strategy == "eager" else "3"]
        payload = {
            "schema": "wtime-entry-batch-v1", "schema_version": 1, "driver": "entry_batch",
            "case": "p135_counter", "input": 7, "expected": 8, "mode": "register",
            "strategy": strategy, "threshold": None if strategy == "eager" else 3,
            "requested_calls": calls, "actual_calls": calls, "fixture_calls": calls, "result": 8,
            "source_fuel": None, "feature_profile": "mvp",
            "instrumented": False, "total_ns": 17, "prepare_ns": 5,
            "instantiate_ns": 4, "invoke_ns": 8,
        }
        return {
            "kind": "timing", "argv": argv, "returncode": 0, "stdout": json.dumps(payload) + "\n",
            "stderr": "", "binary": str(binary), "fixture": str(self.fixture),
            "binary_sha256": digest(binary), "fixture_sha256": digest(self.fixture),
            "strategy": strategy, "calls": calls,
        }

    def diagnostic_row(self, strategy: str, calls: int) -> dict:
        argv = [str(self.bin_diag), "--entry-diagnostic", str(self.fixture), "p135_counter", "7", "8",
                str(calls), "register", strategy, "0" if strategy == "eager" else "3"]
        per_call = [{"call": index,
                     "register_dispatches": 10 if strategy == "eager" or index >= 3 else 0,
                     "register_exits": 0,
                     "fallback_frames": 0 if strategy == "eager" or index >= 3 else 1}
                    for index in range(1, calls + 1)]
        payload = {
            "schema": "wtime-entry-diagnostic-v1", "schema_version": 1,
            "driver": "entry_diagnostic", "case": "p135_counter", "input": 7,
            "expected": 8, "mode": "register", "strategy": strategy,
            "threshold": None if strategy == "eager" else 3, "requested_calls": calls,
            "actual_calls": calls, "result": 8, "instrumented": True,
            "source_fuel": None, "feature_profile": "mvp",
            "first_register_call": 1 if strategy == "eager" else (3 if calls >= 3 else None),
            "per_call": per_call,
        }
        return {
            "kind": "diagnostic", "argv": argv, "returncode": 0, "stdout": json.dumps(payload) + "\n",
            "stderr": "", "binary": str(self.bin_diag), "fixture": str(self.fixture),
            "binary_sha256": digest(self.bin_diag), "fixture_sha256": digest(self.fixture),
            "strategy": strategy, "calls": calls,
        }

    def make_observation(self) -> dict:
        timing = [self.timing_row(self.bin_prod, strategy, count)
                  for strategy in ("eager", "adaptive") for count in p135.CALL_COUNTS]
        diagnostics = [self.diagnostic_row(strategy, count)
                       for strategy in ("eager", "adaptive") for count in p135.CALL_COUNTS]
        probe_args = [
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "2", "register", "eager", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "10", "portable", "eager", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "10", "register", "eager", "3"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "10", "register", "adaptive", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "oops", "register", "eager", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "0", "register", "eager", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "-1", "register", "eager", "0"],
            ["--entry-batch", str(self.root / "missing.wasm"), "p135_counter", "7", "8", "10001", "register", "eager", "0"],
            ["--entry-batch", str(self.root / "late.wasm"), "p135_counter", "7", "8", "10", "register", "eager", "0"],
            ["--entry-diagnostic", str(self.fixture), "p135_counter", "7", "8", "10", "register", "eager", "0"],
            ["--entry-batch", str(self.fixture), "p135_counter", "7", "8", "10", "register", "eager", "0"],
        ]
        negative = []
        for index, args in enumerate(probe_args):
            binary = self.bin_prod if index != 10 else self.bin_diag
            negative.append({
                "kind": f"negative-{index}", "argv": [str(binary), *args], "returncode": 2,
                "stdout": "", "stderr": "usage or oracle error",
                "binary": str(binary), "fixture": str(self.fixture),
                "binary_sha256": digest(binary), "fixture_sha256": digest(self.fixture),
            })
        return {
            "schema": "wtime-p135-observations-v1", "timing_evidence": False,
            "settings": {"mode": "register", "call_counts": list(p135.CALL_COUNTS),
                         "strategies": ["eager", "adaptive"], "adaptive_threshold": 3,
                         "source_fuel": None, "feature_profile": "mvp",
                         "fixture": str(self.fixture), "fixture_sha256": digest(self.fixture)},
            "production_binary": {"path": str(self.bin_prod), "sha256": digest(self.bin_prod),
                                  "bytes": self.bin_prod.stat().st_size},
            "diagnostic_binary": {"path": str(self.bin_diag), "sha256": digest(self.bin_diag),
                                  "bytes": self.bin_diag.stat().st_size},
            "timing_rows": timing, "diagnostic_rows": diagnostics, "negative_rows": negative,
        }

    def test_accepts_complete_source_bound_probe_matrix(self) -> None:
        p135.validate_observations(self.observation, root=self.root)

    def test_rejects_wrong_binary_or_fixture_binding(self) -> None:
        wrong_binary = self.root / "bin" / "other"
        wrong_binary.write_bytes(b"different executable")
        changed = json.loads(json.dumps(self.observation))
        changed["timing_rows"][0]["binary"] = str(wrong_binary)
        changed["timing_rows"][0]["binary_sha256"] = digest(wrong_binary)
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        changed["diagnostic_rows"][0]["fixture_sha256"] = "0" * 64
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)

    def test_rejects_instrumented_or_malformed_timing_json(self) -> None:
        changed = json.loads(json.dumps(self.observation))
        payload = json.loads(changed["timing_rows"][0]["stdout"])
        payload["instrumented"] = True
        changed["timing_rows"][0]["stdout"] = json.dumps(payload) + "\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        changed["timing_rows"][0]["stdout"] = "not json\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        payload = json.loads(changed["timing_rows"][0]["stdout"])
        payload["schema_version"] = True
        changed["timing_rows"][0]["stdout"] = json.dumps(payload) + "\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        changed["settings"]["feature_profile"] = "all-phase3-core"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)

    def test_rejects_bad_diagnostics_and_stale_observation(self) -> None:
        changed = json.loads(json.dumps(self.observation))
        payload = json.loads(changed["diagnostic_rows"][0]["stdout"])
        payload["total_ns"] = 12
        changed["diagnostic_rows"][0]["stdout"] = json.dumps(payload) + "\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        payload = json.loads(changed["diagnostic_rows"][0]["stdout"])
        payload["per_call"][0]["register_dispatches"] = True
        changed["diagnostic_rows"][0]["stdout"] = json.dumps(payload) + "\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        payload = json.loads(changed["timing_rows"][0]["stdout"])
        payload["requested_calls"] = True
        changed["timing_rows"][0]["stdout"] = json.dumps(payload) + "\n"
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)
        changed = json.loads(json.dumps(self.observation))
        changed["timing_rows"][0]["binary_sha256"] = "f" * 64
        with self.assertRaises(p135.P135Error):
            p135.validate_observations(changed, root=self.root)


if __name__ == "__main__":
    unittest.main()
