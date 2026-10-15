import importlib.util
import json
import pathlib
import shutil
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[3]
POLICY_PATH = ROOT / "tools/wtime-bench/entry_sequence_policy.py"
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("p141_sequence_policy", POLICY_PATH)
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


class EntrySequencePolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p141-policy-")
        self.root = pathlib.Path(self.temp.name).resolve()
        helpers = self.root / "helpers"
        self._inside(helpers)
        helpers.mkdir()
        # Preserve the exact executed helper source and command log before test work.
        source_copy = helpers / "entry_sequence_policy.py"
        self._inside(source_copy).write_bytes(POLICY_PATH.read_bytes())
        test_copy = helpers / "test_p141_sequence_policy.py"
        self._inside(test_copy).write_bytes(pathlib.Path(__file__).read_bytes())
        command_log = helpers / "command.json"
        self._inside(command_log).write_text(json.dumps({"argv": [sys.executable, "-B", "-m", "unittest", *sys.argv[1:]]}))

    def tearDown(self):
        self.temp.cleanup()

    def _inside(self, path):
        resolved = pathlib.Path(path).resolve()
        self.assertTrue(resolved.is_relative_to(self.root), f"output escaped temp root: {resolved}")
        return resolved

    def _timing(self, n, input_value=2147483647, strategy="adaptive"):
        threshold = n if strategy == "adaptive" else 0
        rows = []
        for position in policy.schedule(n):
            row = dict(position)
            row["result"] = ((input_value + row["body"] + (1 << 31)) % (1 << 32)) - (1 << 31)
            row["elapsed_ns"] = 0
            rows.append(row)
        return {
            "schema": policy.SCHEMA,
            "schema_version": 1,
            "driver": "entry_sequence",
            "case": "uniform66",
            "mode": "register",
            "feature_profile": "mvp",
            "source_fuel": None,
            "input": input_value,
            "strategy": strategy,
            "threshold": None if strategy == "eager" else threshold,
            "schedule_threshold": n,
            "requested_calls": 67 * (n + 1),
            "actual_calls": 67 * (n + 1),
            "instrumented": False,
            "diagnostic": False,
            "prepare_ns": 0,
            "instantiate_ns": 0,
            "per_call": rows,
        }, threshold

    def _diagnostic(self, n, strategy="adaptive"):
        threshold = n if strategy == "adaptive" else 0
        payload, _ = self._timing(n, 0, strategy)
        payload.pop("prepare_ns")
        payload.pop("instantiate_ns")
        payload["instrumented"] = True
        payload["diagnostic"] = True
        for row in payload["per_call"]:
            pos = row.copy()
            row.pop("elapsed_ns")
            ready = pos["visit"] >= n
            row["register_dispatches"] = 4 if (strategy == "eager" or ready) else 0
            row["fallback_frames"] = 0 if (strategy == "eager" or ready) else 1
            if strategy == "adaptive":
                row.update({
                    "cache_state": "ready" if ready else "cold",
                    "entries": pos["visit"],
                    "builder_starts": (pos["body"] if pos["phase"] == "initial" else 66) + int(ready),
                    "published": (pos["body"] if pos["phase"] == "initial" else 66) + int(ready),
                    "evictions": max(pos["body"] - 63, 0) if pos["phase"] == "initial" else 3,
                    "resident_entries": min(pos["body"] + 1, 64) if pos["phase"] == "initial" else 64,
                    "retained_bytes": 64,
                    "byte_limit": 1024,
                })
        return payload, threshold

    def test_exact_schedule_and_wraparound_oracle_for_all_thresholds(self):
        for n in (1, 3, 10, 64):
            with self.subTest(n=n):
                payload, threshold = self._timing(n)
                result = policy.validate_payload(payload, n, "adaptive", threshold, payload["input"])
                self.assertIs(result, payload)
                self.assertEqual(len(policy.schedule(n)), 67 * (n + 1))
                self.assertEqual(policy.schedule(n)[-1], {"body": 0, "visit": n + 1, "phase": "revisit"})
                self.assertEqual(policy.event_class({"body": 0, "visit": n, "phase": "revisit"}, n), "recompile")
                self._inside(self.root / "helpers/command.json")

    def test_rejects_exact_type_count_order_oracle_and_extra_field_drift(self):
        valid, threshold = self._timing(3)
        mutations = []
        p = json.loads(json.dumps(valid)); p["input"] = True; mutations.append(p)
        p = json.loads(json.dumps(valid)); p["prepare_ns"] = 1.0; mutations.append(p)
        p = json.loads(json.dumps(valid)); p["actual_calls"] -= 1; mutations.append(p)
        p = json.loads(json.dumps(valid)); p["per_call"][0], p["per_call"][1] = p["per_call"][1], p["per_call"][0]; mutations.append(p)
        p = json.loads(json.dumps(valid)); p["per_call"][-1]["result"] += 1; mutations.append(p)
        p = json.loads(json.dumps(valid)); p["unexpected"] = 1; mutations.append(p)
        for payload in mutations:
            with self.subTest(payload=payload.get("actual_calls")):
                with self.assertRaises(ValueError):
                    policy.validate_payload(payload, 3, "adaptive", threshold, valid["input"])

    def test_rejects_instrumented_or_mixed_timing_and_diagnostic_wires(self):
        timed, threshold = self._timing(1)
        p = json.loads(json.dumps(timed)); p["instrumented"] = True
        with self.assertRaises(ValueError):
            policy.validate_payload(p, 1, "adaptive", threshold, timed["input"])
        p = json.loads(json.dumps(timed)); p["per_call"][0]["register_dispatches"] = 0
        with self.assertRaises(ValueError):
            policy.validate_payload(p, 1, "adaptive", threshold, timed["input"])
        diagnostic, threshold = self._diagnostic(1)
        policy.validate_payload(diagnostic, 1, "adaptive", threshold, 0, diagnostic=True)
        diagnostic["prepare_ns"] = 0
        with self.assertRaises(ValueError):
            policy.validate_payload(diagnostic, 1, "adaptive", threshold, 0, diagnostic=True)

    def test_adaptive_transition_drift_and_eager_diagnostic_contract(self):
        payload, threshold = self._diagnostic(3)
        policy.validate_payload(payload, 3, "adaptive", threshold, 0, diagnostic=True)
        for field, wrong in (("cache_state", "ready"), ("builder_starts", 999), ("evictions", 99), ("fallback_frames", 0), ("resident_entries", 0)):
            changed, threshold = self._diagnostic(3)
            changed["per_call"][0][field] = wrong
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    policy.validate_payload(changed, 3, "adaptive", threshold, 0, diagnostic=True)
        eager, threshold = self._diagnostic(1, "eager")
        policy.validate_payload(eager, 1, "eager", threshold, 0, diagnostic=True)
        eager["per_call"][0]["cache_state"] = "cold"
        with self.assertRaises(ValueError):
            policy.validate_payload(eager, 1, "eager", threshold, 0, diagnostic=True)


if __name__ == "__main__":
    unittest.main()
