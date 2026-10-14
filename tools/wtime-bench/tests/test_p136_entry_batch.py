"""Contained synthetic end-to-end checks for the P136 Register pair collector."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
import sys
sys.path.insert(0, str(BENCH))
import entry_batch_compare as compare
import checkpoint
import entry_batch_adapter as adapter


COUNTS = (1, 10, 100, 10000)
SEEDS = [7103, 9917]
FAKE_PROGRAM = '''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ["P136_FAKE_ARGV_LOG"], "a", encoding="utf-8") as stream:
    stream.write(json.dumps(args) + "\\n")
if os.environ.get("P136_FAIL_ONCE"):
    state = pathlib.Path(os.environ["P136_FAIL_ONCE"])
    if not state.exists():
        state.write_text("failed once")
        print("synthetic interruption", file=sys.stderr)
        raise SystemExit(17)
assert len(args) == 9 and args[0] == "--entry-batch"
_, fixture, case, raw_input, raw_expected, raw_calls, mode, strategy, *tail = args
threshold = int(tail[0])
calls = int(raw_calls)
expected = int(raw_expected)
payload = {
    "schema": "wtime-entry-batch-v1", "schema_version": 1, "driver": "entry_batch",
    "case": case, "input": int(raw_input), "expected": expected, "result": expected,
    "mode": mode, "strategy": strategy, "threshold": None if strategy == "eager" else threshold,
    "requested_calls": calls, "actual_calls": calls, "instrumented": False,
    "source_fuel": None, "feature_profile": "mvp",
    "fixture_calls": calls if case == "p135_counter" else None,
    "total_ns": 4, "prepare_ns": 1, "instantiate_ns": 1, "invoke_ns": 2,
}
print(json.dumps(payload, separators=(",", ":")))
'''


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _within(path: Path, root: Path) -> Path:
    resolved = path.resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise AssertionError(f"synthetic path escapes temp root: {resolved}")
    return resolved


class CollectorFixture:
    def __init__(self, test: unittest.TestCase, *, fail_once: bool = False):
        self.tmp = tempfile.TemporaryDirectory(prefix="p136-entry-collector-")
        self.root = Path(self.tmp.name).resolve()
        self.test = test
        self.archive = self.root / "archive"
        self.adapter_root = self.root / "adapter"
        self.candidate_root = self.root / "candidate"
        self.repo = self.root / "repo"
        self.harness = self.root / "harness"
        self.output = self.root / "collection"
        self.fixture = self.root / "fixtures/counter.wasm"
        self.argv_log = self.root / "fake-argv.jsonl"
        self.fail_state = self.root / "fail-once-state"
        self.fail_once = fail_once
        self._mkdir(self.archive)
        self._mkdir(self.repo / "crates")
        self._mkdir(self.repo / "tools/wtime-conformance")
        self._mkdir(self.harness / "src")
        self._mkdir(self.fixture.parent)
        self.fixture.write_bytes(b"synthetic wasm bytes, not a real fixture")
        (self.archive / "manifest.json").write_text(json.dumps({"schema": 1, "mode": "register"}))
        self._write(self.harness / "src/protocol.rs", b"protocol-current\n")
        self._write(self.harness / "src/entry_batch_driver.rs", b"driver-current\n")
        self._write(self.harness / "src/counter_driver.rs", b"leaf-current\n")
        self.baseline = self._record(self.adapter_root, "source-bound-register-entry-adapter",
                                     "baseline-core-v1")
        self.candidate = self._record(self.candidate_root, "source-bound-register-entry-candidate",
                                      "candidate-core-v1")
        self.baseline_record_path = self.adapter_root / "adapter.json"
        self.candidate_record_path = self.candidate_root / "adapter.json"
        self.rows = [
            {"fixture": str(self.fixture), "case": "p135_counter", "input": 7, "expected": 8,
             "calls": count, "strategy": "eager" if count in (1, 100) else "adaptive",
             "threshold": 0 if count in (1, 100) else 3}
            for count in COUNTS
        ]

    def _mkdir(self, path: Path) -> Path:
        resolved = _within(path, self.root)
        resolved.mkdir(parents=True, exist_ok=True)
        return resolved

    def _write(self, path: Path, data: bytes) -> None:
        path = _within(path, self.root)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def _record(self, root: Path, kind: str, core_tag: str) -> dict:
        self._mkdir(root / "binary")
        executable = root / "binary/fake-entry.py"
        executable.write_text(FAKE_PROGRAM, encoding="utf-8")
        executable.chmod(executable.stat().st_mode | stat.S_IXUSR)
        overlay = {
            "src/protocol.rs": {"sha256": _sha((self.harness / "src/protocol.rs").read_bytes())},
            "src/entry_batch_driver.rs": {"sha256": _sha((self.harness / "src/entry_batch_driver.rs").read_bytes())},
            "src/counter_driver.rs": {"sha256": _sha((self.harness / "src/counter_driver.rs").read_bytes())},
        }
        record = {
            "schema": "synthetic-record",
            "kind": kind,
            "mode": "register",
            "executable": "binary/fake-entry.py",
            "executable_sha256": _sha(executable.read_bytes()),
            "copied_core_sources": {f"crates/synthetic/{core_tag}.rs": {"sha256": _sha(core_tag.encode())}},
            "core_source_map": {f"crates/synthetic/{core_tag}.rs": _sha(core_tag.encode())},
            "overlay_sources": overlay,
            "protocol_sha256": overlay["src/protocol.rs"]["sha256"],
            "entry_batch_driver_sha256": overlay["src/entry_batch_driver.rs"]["sha256"],
            "shared_leaf_map": {"src/counter_driver.rs": overlay["src/counter_driver.rs"]["sha256"]},
            "features": list(adapter.FEATURES), "profile": "release", "instrumented": False,
            "wasmtime_dependency": False, "feature_profile": "mvp", "source_fuel": None,
            "generated_manifest_sha256": "a" * 64, "isolated_lock_sha256": "b" * 64,
            "toolchain": {"rustc_vv": "rustc synthetic 1.99", "cargo_v": "cargo synthetic 1.99"},
            "binary_path": str(executable.resolve()),
            "source_roots": {"core": str(root / "source"), "harness": str(root / "workspace/src"),
                             "workspace": str(root / "workspace")},
        }
        (root / "adapter.json").write_text(json.dumps(record, sort_keys=True))
        return record

    def patch_verifiers(self):
        return mock.patch.multiple(
            compare.adapter,
            verify=mock.Mock(return_value=self.baseline),
            verify_current=mock.Mock(return_value=self.candidate),
        )

    def rows_for(self, counts=COUNTS):
        return [row for row in self.rows if row["calls"] in counts]

    def invoke(self, *, rows=None, output=None, pairs=2, seeds=SEEDS, smoke=True,
               same_source_smoke=False, resume=False, candidate_root=None):
        output = output or self.output
        _within(output, self.root)
        # Preserve the exact synthetic harness and arguments before any guest is launched.
        audit = self.root / "test-audit"
        self._mkdir(audit)
        source_copy = audit / "test_p136_entry_batch.py"
        source_copy.write_bytes(Path(__file__).read_bytes())
        argv_copy = audit / "collect-arguments.json"
        argv_copy.write_text(json.dumps({"rows": rows or self.rows, "pairs": pairs,
                                         "seeds": list(seeds), "smoke": smoke,
                                         "same_source_smoke": same_source_smoke,
                                         "output": str(output)}, sort_keys=True))
        os.environ["P136_FAKE_ARGV_LOG"] = str(self.argv_log)
        if self.fail_once:
            os.environ["P136_FAIL_ONCE"] = str(self.fail_state)
        else:
            os.environ.pop("P136_FAIL_ONCE", None)
        try:
            with self.patch_verifiers():
                return compare.collect(
                    self.archive, self.adapter_root,
                    candidate_root or self.candidate_root, self.repo, self.harness,
                    rows or self.rows, pairs, seeds, output,
                    smoke=smoke, same_source_smoke=same_source_smoke, resume=resume,
                )
        finally:
            os.environ.pop("P136_FAKE_ARGV_LOG", None)
            os.environ.pop("P136_FAIL_ONCE", None)

    def assert_contained_outputs(self):
        for path in self.root.rglob("*"):
            _within(path, self.root)

    def close(self):
        self.tmp.cleanup()


class P136EntryBatchTests(unittest.TestCase):
    def setUp(self):
        self.fx = CollectorFixture(self)

    def tearDown(self):
        self.fx.close()

    def test_p136_fixed_fresh_batch_rows_keep_exact_calls_without_calibration(self):
        report = self.fx.invoke()
        self.assertTrue(report["passed"])
        self.assertFalse(report["timing_evidence"])
        self.assertEqual(report["metric"], "entry_total")
        self.assertEqual(report["pair_count"], 16)
        launched = [json.loads(line) for line in self.fx.argv_log.read_text().splitlines()]
        self.assertEqual(len(launched), 2 * 2 * 2 * len(COUNTS))
        for args in launched:
            self.assertEqual(args[0], "--entry-batch")
            self.assertEqual(args[2], "p135_counter")
            self.assertEqual(args[6], "register")
            self.assertIn(int(args[5]), COUNTS)
            self.assertEqual(int(args[3]), 7)
            self.assertEqual(int(args[4]), 8)
        self.assertEqual({int(args[5]) for args in launched}, set(COUNTS))
        self.assertEqual(len(list(self.fx.output.glob("commands/**/result.json"))), 32)
        for run in report["runs"]:
            for pair in run["pairs"]:
                for obs in pair["observations"].values():
                    self.assertEqual(obs["payload"]["actual_calls"], int(obs["argv"][6]))
                    self.assertEqual(obs["payload"]["fixture_calls"], int(obs["argv"][6]))
                    self.assertNotIn("ns_per_operation", obs["payload"])
        self.fx.assert_contained_outputs()

    def test_p136_candidate_and_adapter_share_protocol_driver_config_and_toolchain(self):
        # Distinct core source maps are preserved while all compiled harness/build identities match.
        with self.fx.patch_verifiers():
            identity = compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                       self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)
        self.assertNotEqual(identity["sides"]["baseline"]["core_sources"],
                            identity["sides"]["candidate"]["core_sources"])
        cases = (
            ("features", ["register-executor"]),
            ("instrumented", True),
            ("feature_profile", "simd"),
            ("source_fuel", 50000000),
            ("generated_manifest_sha256", "c" * 64),
            ("isolated_lock_sha256", "d" * 64),
        )
        original = dict(self.fx.candidate)
        for field, value in cases:
            bad = dict(original)
            bad[field] = value
            with mock.patch.object(compare.adapter, "verify", return_value=self.fx.baseline), \
                 mock.patch.object(compare.adapter, "verify_current", return_value=bad):
                with self.subTest(field=field), self.assertRaises(ValueError):
                    compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                    self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)
        for field, overlay_key in (("protocol_sha256", "src/protocol.rs"),
                                   ("entry_batch_driver_sha256", "src/entry_batch_driver.rs")):
            bad = dict(original)
            bad[field] = "f" * 64
            bad["overlay_sources"] = dict(original["overlay_sources"])
            bad["overlay_sources"][overlay_key] = {"sha256": "f" * 64}
            with mock.patch.object(compare.adapter, "verify", return_value=self.fx.baseline), \
                 mock.patch.object(compare.adapter, "verify_current", return_value=bad):
                with self.subTest(field=field), self.assertRaisesRegex(ValueError, "identities differ"):
                    compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                    self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)
        bad = dict(original)
        bad["toolchain"] = {"rustc_vv": "foreign", "cargo_v": "foreign"}
        with mock.patch.object(compare.adapter, "verify", return_value=self.fx.baseline), \
             mock.patch.object(compare.adapter, "verify_current", return_value=bad):
            with self.assertRaisesRegex(ValueError, "identities differ"):
                compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)

    def test_p136_source_bound_adapter_rejects_foreign_or_stale_inputs_before_guest_launch(self):
        launched_before = self.fx.argv_log.exists()
        with mock.patch.object(compare.adapter, "verify", side_effect=ValueError("stale archive source")):
            with self.assertRaisesRegex(ValueError, "stale archive source"):
                compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)
        self.assertEqual(self.fx.argv_log.exists(), launched_before)
        self.assertFalse(self.fx.output.exists())
        with mock.patch.object(compare.adapter, "verify", side_effect=ValueError("foreign legacy adapter")):
            with self.assertRaisesRegex(ValueError, "foreign legacy adapter"):
                compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.candidate_root,
                                self.fx.repo, self.fx.harness, self.fx.rows, 2, SEEDS, True, False)

    def test_p136_register_settings_forward_and_reject_invalid_rows(self):
        valid = compare.normalize_rows(self.fx.rows)
        self.assertEqual([row["calls"] for row in valid], list(COUNTS))
        self.assertEqual(valid[0]["strategy"], "eager")
        self.assertEqual(valid[1]["strategy"], "adaptive")
        bad_rows = (
            {**self.fx.rows[0], "calls": True},
            {**self.fx.rows[0], "calls": 2},
            {**self.fx.rows[0], "strategy": "decoded"},
            {**self.fx.rows[0], "threshold": 1},
            {**self.fx.rows[1], "threshold": 0},
        )
        for row in bad_rows:
            with self.subTest(row=row), self.assertRaises(ValueError):
                compare.normalize_rows([row])
        report = self.fx.invoke(rows=self.fx.rows_for((10,)), pairs=2)
        for run in report["runs"]:
            for pair in run["pairs"]:
                for obs in pair["observations"].values():
                    args = obs["argv"]
                    self.assertEqual(args[1:], ["--entry-batch", str(self.fx.fixture), "p135_counter",
                                                "7", "8", "10", "register", "adaptive", "3"])
        payload = report["runs"][0]["pairs"][0]["observations"]["baseline"]["payload"]
        corruptions = []
        bool_timing = dict(payload)
        bool_timing["total_ns"] = True
        corruptions.append(bool_timing)
        bool_calls = dict(payload)
        bool_calls["actual_calls"] = True
        corruptions.append(bool_calls)
        divided = dict(payload)
        divided["total_ns"] = 1
        corruptions.append(divided)
        extra_divided_field = dict(payload)
        extra_divided_field["ns_per_operation"] = 0.1
        corruptions.append(extra_divided_field)
        for bad_payload in corruptions:
            with self.subTest(payload=bad_payload), self.assertRaises(ValueError):
                compare.validate_payload(bad_payload, report["identity"]["rows"][0])

    def test_p136_pairs_join_only_identical_fixture_settings_and_run_identity(self):
        report = self.fx.invoke(rows=self.fx.rows_for((1,)), pairs=2)
        output = self.fx.output
        spec = compare.pair_specs(report["identity"])[0]
        pair = report["runs"][0]["pairs"][0]
        corruptions = []
        bad_argv = json.loads(json.dumps(report))
        bad_argv["runs"][0]["pairs"][0]["observations"]["candidate"]["argv"][4] = "999"
        corruptions.append(bad_argv)
        bad_fixture = json.loads(json.dumps(report))
        bad_fixture["runs"][0]["pairs"][0]["observations"]["candidate"]["fixture_sha256"] = "0" * 64
        corruptions.append(bad_fixture)
        bad_seed = json.loads(json.dumps(report))
        bad_seed["runs"][0]["seed"] += 1
        corruptions.append(bad_seed)
        bad_counter = json.loads(json.dumps(report))
        bad_counter["runs"][0]["pairs"][0]["observations"]["baseline"]["payload"]["fixture_calls"] = 0
        corruptions.append(bad_counter)
        missing = json.loads(json.dumps(report))
        missing["runs"][0]["pairs"].pop()
        missing["pair_count"] -= 1
        corruptions.append(missing)
        duplicate = json.loads(json.dumps(report))
        duplicate["runs"][0]["pairs"].append(duplicate["runs"][0]["pairs"][0])
        duplicate["pair_count"] += 1
        corruptions.append(duplicate)
        for candidate in corruptions:
            with self.subTest(corruption=candidate["pair_count"]), self.assertRaises(ValueError):
                compare.validate_report(candidate, output)
        with self.assertRaises(ValueError):
            compare.validate_pair(pair, {**spec, "seed": spec["seed"] + 1}, report["identity"], output)

    def test_p136_receipts_resume_without_overwriting_raw_and_reject_duplicate_or_conflicting_seed(self):
        fx = CollectorFixture(self, fail_once=True)
        try:
            one_row = fx.rows_for((1,))
            with self.assertRaisesRegex(ValueError, "command failed"):
                fx.invoke(rows=one_row, pairs=2)
            failed = list(fx.output.glob("commands/**/attempt-0001/result.json"))
            self.assertEqual(len(failed), 1)
            first_bytes = failed[0].read_bytes()
            report = fx.invoke(rows=one_row, pairs=2, resume=True)
            self.assertEqual(report["pair_count"], 4)
            self.assertEqual(failed[0].read_bytes(), first_bytes)
            self.assertTrue(list(fx.output.glob("commands/**/attempt-0002/result.json")))
            identity = report["identity"]
            journal_root = fx.root / "bad-journal"
            _within(journal_root, fx.root)
            journal = checkpoint.Journal(journal_root, identity, dict(enumerate(SEEDS)))
            pair = {"pair_id": "0:duplicate:0"}
            journal.append(0, SEEDS[0], pair)
            with self.assertRaisesRegex(checkpoint.CheckpointError, "duplicate pair ID"):
                journal.append(0, SEEDS[0], pair)
            journal.close()
            with (journal_root / "checkpoint.jsonl").open("ab") as stream:
                stream.write(json.dumps({"type": "pair", "run_id": 0, "seed": SEEDS[0] + 1,
                                         "pair": {"pair_id": "0:conflict:0"}}).encode() + b"\n")
            with self.assertRaisesRegex(checkpoint.CheckpointError, "conflicting seed"):
                checkpoint.Journal(journal_root, identity, dict(enumerate(SEEDS)), resume=True)
            fx.assert_contained_outputs()
        finally:
            fx.close()

    def test_p136_same_source_alias_is_smoke_only(self):
        with self.fx.patch_verifiers():
            alias = compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.adapter_root,
                                    self.fx.repo, self.fx.harness, self.fx.rows_for((1,)), 2,
                                    SEEDS, True, True)
        self.assertTrue(alias["same_source_alias"])
        self.assertFalse(alias["timing_evidence"])
        report = self.fx.invoke(rows=self.fx.rows_for((1,)), pairs=2, same_source_smoke=True,
                                candidate_root=self.fx.adapter_root)
        self.assertTrue(report["same_source_alias"])
        self.assertFalse(report["timing_evidence"])
        self.fx.assert_contained_outputs()
        with self.fx.patch_verifiers():
            with self.assertRaisesRegex(ValueError, "same-source alias cannot be candidate evidence"):
                compare.prepare(self.fx.archive, self.fx.adapter_root, self.fx.adapter_root,
                                self.fx.repo, self.fx.harness, self.fx.rows_for((1,)), 30,
                                SEEDS, False, True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
