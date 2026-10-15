"""Contained synthetic tests for the P142 individual-entry collector."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
DRAFT = ROOT / "tools/wtime-bench"
BENCH = ROOT / "tools/wtime-bench"
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(ROOT))
import entry_sequence_policy as policy

spec = importlib.util.spec_from_file_location("p142_draft_collector", DRAFT / "entry_sequence_compare.py")
collector = importlib.util.module_from_spec(spec)
assert spec.loader
spec.loader.exec_module(collector)


def inside(root: Path, path: Path) -> Path:
    root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise AssertionError(f"synthetic write escapes fresh temporary root: {resolved}")
    return resolved


class SyntheticInputs:
    def __init__(self, test):
        self.test = test
        self.root = test.root
        self.fixture = inside(self.root, self.root / "fixtures/uniform66.wasm")
        inside(self.root, self.fixture.parent).mkdir(parents=True)
        inside(self.root, self.fixture).write_bytes(b"synthetic uniform66 fixture bytes")
        self.archive = inside(self.root, self.root / "archive")
        inside(self.root, self.archive).mkdir()
        inside(self.root, self.archive / "manifest.json").write_text("{}\n")
        self.harness = inside(self.root, self.root / "harness")
        inside(self.root, self.harness).mkdir()
        self.repo = inside(self.root, self.root / "repo")
        inside(self.root, self.repo / "crates").mkdir(parents=True)
        inside(self.root, self.repo / "tools/wtime-conformance").mkdir(parents=True)
        self.baseline = self._build("baseline", collector.adapter.KIND)
        self.candidate = self._build("candidate", collector.adapter.CURRENT_KIND)
        self.row = {"fixture": str(self.fixture), "input": 2147483647, "schedule_threshold": 3}

    def _build(self, name, kind):
        root = inside(self.root, self.root / name)
        inside(self.root, root).mkdir()
        binary = inside(self.root, root / "binary/sequence")
        inside(self.root, binary.parent).mkdir()
        inside(self.root, binary).write_bytes((name + " binary").encode())
        record = {
            "schema": "synthetic-build-v1", "kind": kind, "mode": "register",
            "executable": "binary/sequence", "copied_core_sources": {"runtime/src/lib.rs": {"sha256": "a" * 64}},
            "overlay_sources": {"src/protocol.rs": {"sha256": "b" * 64},
                                "src/entry_sequence_driver.rs": {"sha256": "c" * 64}},
            "features": list(collector.adapter.FEATURES), "profile": "release",
            "instrumented": False, "wasmtime_dependency": False, "feature_profile": "mvp",
            "source_fuel": None, "generated_manifest_sha256": "d" * 64,
            "isolated_lock_sha256": "e" * 64, "toolchain": {"rustc": "test", "cargo": "test"},
        }
        if kind == collector.adapter.CURRENT_KIND:
            record.update({"source_origin": "current-workspace", "current_source_map": {"src/lib.rs": "a" * 64}})
        inside(self.root, root / "adapter.json").write_text(json.dumps(record, sort_keys=True) + "\n")
        return root

    def records(self):
        baseline = json.loads((self.baseline / "adapter.json").read_text())
        candidate = json.loads((self.candidate / "adapter.json").read_text())
        return baseline, candidate

    def patch_adapters(self):
        baseline, candidate = self.records()
        return (mock.patch.object(collector.adapter, "verify", return_value=baseline),
                mock.patch.object(collector.adapter, "verify_current", return_value=candidate))

    def payload(self, argv):
        exe = Path(argv[3])
        side = "baseline" if exe.is_relative_to(self.baseline) else "candidate"
        normalized = collector.normalize_rows([self.row])[0]
        row = normalized
        strategy = "eager" if side == "baseline" else "adaptive"
        threshold = 0 if side == "baseline" else row["schedule_threshold"]
        per_call = []
        for item in policy.schedule(row["schedule_threshold"]):
            per_call.append({**item, "result": ((row["input"] + item["body"] + (1 << 31)) % (1 << 32)) - (1 << 31),
                             "elapsed_ns": 100 + item["body"] + item["visit"]})
        return {
            "schema": policy.SCHEMA, "schema_version": 1, "driver": "entry_sequence",
            "case": "uniform66", "mode": "register", "feature_profile": "mvp", "source_fuel": None,
            "input": row["input"], "strategy": strategy,
            "threshold": None if strategy == "eager" else threshold,
            "schedule_threshold": row["schedule_threshold"], "requested_calls": len(per_call),
            "actual_calls": len(per_call), "instrumented": False, "diagnostic": False,
            "prepare_ns": 1000, "instantiate_ns": 500, "per_call": per_call,
        }


class P142SequenceCollectionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="p142-sequence-collection-")
        self.root = Path(self.tmp.name).resolve()
        audit = inside(self.root, self.root / "audit")
        audit.mkdir()
        # Preserve the executed draft and actual helper sources/argv before fixture writes.
        for target, source in (("test_source.py", Path(__file__)),
                               ("entry_sequence_compare.py", DRAFT / "entry_sequence_compare.py"),
                               ("entry_sequence_policy.py", BENCH / "entry_sequence_policy.py"),
                               ("entry_batch_adapter.py", BENCH / "entry_batch_adapter.py"),
                               ("entry_batch_compare.py", BENCH / "entry_batch_compare.py"),
                               ("checkpoint.py", BENCH / "checkpoint.py")):
            inside(self.root, audit / target).write_bytes(source.read_bytes())
        inside(self.root, audit / "arguments.json").write_text(json.dumps({
            "test": self.id(), "argv": list(sys.argv),
        }, sort_keys=True, indent=2) + "\n")
        self.inputs = SyntheticInputs(self)
        self.output = inside(self.root, self.root / "collection")
        self.seeds = [14201, 14202]

    def tearDown(self):
        self.tmp.cleanup()

    def _run(self, *, smoke=True, resume=False, runner=None):
        p1, p2 = self.inputs.patch_adapters()
        runner = runner or (lambda argv, **kwargs: types.SimpleNamespace(
            returncode=0, stdout=json.dumps(self.inputs.payload(argv), separators=(",", ":")), stderr=""))
        with p1, p2, mock.patch.object(collector.subprocess, "run", side_effect=runner):
            return collector.collect(self.inputs.archive, self.inputs.baseline, self.inputs.candidate,
                self.inputs.repo, self.inputs.harness, [self.inputs.row], 2 if smoke else 30,
                self.seeds, self.output, smoke=smoke, resume=resume, cpu=min(__import__("os").sched_getaffinity(0)))

    def test_smoke_collects_contained_source_bound_pairs_and_replays(self):
        report = self._run()
        self.assertEqual(report["schema"], collector.SCHEMA)
        self.assertEqual(report["metric"], "individual_entry_latency")
        self.assertFalse(report["timing_evidence"])
        self.assertFalse(report["gate_eligible"])
        self.assertEqual(report["pair_count"], 4)
        p1, p2 = self.inputs.patch_adapters()
        with p1, p2:
            replay = collector.verify_collection(self.inputs.archive, self.inputs.baseline,
                self.inputs.candidate, self.inputs.repo, self.inputs.harness, [self.inputs.row],
                2, self.seeds, self.output, smoke=True, cpu=report["identity"]["environment"]["selected_cpu"])
        self.assertEqual(replay, report)
        p1, p2 = self.inputs.patch_adapters()
        with p1, p2:
            root, current = collector.verify_current_collection(self.output)
        self.assertEqual(root, self.output)
        self.assertEqual(current, report)
        journal = self.output / "checkpoint.jsonl"
        events = [json.loads(line) for line in journal.read_text().splitlines()]
        events[0]["run_seeds"]["0"] = True
        inside(self.root, journal).write_text(
            "".join(json.dumps(event, separators=(",", ":")) + "\n" for event in events))
        with self.assertRaisesRegex(ValueError, "checkpoint header"):
            p1, p2 = self.inputs.patch_adapters()
            with p1, p2:
                collector.verify_collection(self.inputs.archive, self.inputs.baseline,
                    self.inputs.candidate, self.inputs.repo, self.inputs.harness, [self.inputs.row],
                    2, self.seeds, self.output, smoke=True,
                    cpu=report["identity"]["environment"]["selected_cpu"])

    def test_full_collection_requires_thirty_pairs_and_distinct_seeds(self):
        p1, p2 = self.inputs.patch_adapters()
        with p1, p2:
            with self.assertRaisesRegex(ValueError, "thirty"):
                collector.collect(self.inputs.archive, self.inputs.baseline, self.inputs.candidate,
                    self.inputs.repo, self.inputs.harness, [self.inputs.row], 2, self.seeds,
                    self.output, smoke=False)
            with self.assertRaisesRegex(ValueError, "distinct"):
                collector.collect(self.inputs.archive, self.inputs.baseline, self.inputs.candidate,
                    self.inputs.repo, self.inputs.harness, [self.inputs.row], 30, [7, 7],
                    self.output, smoke=False)

    def test_failure_preserves_first_side_and_resume_uses_new_attempts(self):
        count = 0

        def fail_second(argv, **kwargs):
            nonlocal count
            count += 1
            if count == 2:
                return types.SimpleNamespace(returncode=9, stdout="", stderr="synthetic failure")
            return types.SimpleNamespace(returncode=0,
                stdout=json.dumps(self.inputs.payload(argv), separators=(",", ":")), stderr="")

        with self.assertRaisesRegex(ValueError, "command failed"):
            self._run(runner=fail_second)
        attempt_files = [inside(self.root, path) for path in
                         (self.output / "commands").rglob("attempt-0001/result.json")]
        self.assertEqual(len(attempt_files), 2)
        retained = [json.loads(path.read_text()) for path in attempt_files]
        self.assertEqual({record["returncode"] for record in retained}, {0, 9})
        report = self._run(resume=True)
        self.assertEqual(report["pair_count"], 4)
        self.assertTrue(list((self.output / "commands").rglob("attempt-0002/result.json")))

    def test_report_rejects_wire_order_and_raw_record_corruption(self):
        report = self._run()
        bad = json.loads(json.dumps(report))
        bad["runs"][0]["pairs"][0]["observations"]["candidate"]["payload"]["per_call"][0]["result"] += 1
        with self.assertRaisesRegex(ValueError, "raw stdout"):
            collector.validate_report(bad, self.output)
        # Python considers these numerically equal; canonical typed comparison must not.
        for key, replacement in (("visit", True), ("elapsed_ns", 101.0)):
            bad = json.loads(json.dumps(report))
            payload = bad["runs"][0]["pairs"][0]["observations"]["candidate"]["payload"]
            payload["per_call"][0][key] = replacement
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "raw stdout"):
                collector.validate_report(bad, self.output)

        bad = json.loads(json.dumps(report))
        bad["identity"]["pairs_per_row"] = 2.0
        bad["identity_sha256"] = collector.digest(bad["identity"])
        with self.assertRaisesRegex(ValueError, "pair count or smoke"):
            collector.validate_report(bad, self.output)
        bad = json.loads(json.dumps(report))
        bad["identity"]["seeds"][0] = True
        bad["identity_sha256"] = collector.digest(bad["identity"])
        with self.assertRaisesRegex(ValueError, "run seeds"):
            collector.validate_report(bad, self.output)

        bad = json.loads(json.dumps(report))
        raw = inside(self.root, self.output / bad["runs"][0]["pairs"][0]["observations"]["baseline"]["raw_record"])
        raw.write_text("{}\n")
        with self.assertRaisesRegex(ValueError, "raw command record"):
            collector.validate_report(bad, self.output)


if __name__ == "__main__":
    unittest.main()
