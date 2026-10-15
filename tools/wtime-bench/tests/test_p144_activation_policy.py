import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[3]
POLICY_PATH = ROOT / "tools/wtime-bench/entry_sequence_policy.py"
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("p144_activation_policy", POLICY_PATH)
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


class ActivationPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p144-activation-policy-")
        self.root = pathlib.Path(self.temp.name).resolve()
        helpers = self.root / "helpers"
        self._inside(helpers).mkdir()
        self._inside(helpers / "entry_sequence_policy.py").write_bytes(POLICY_PATH.read_bytes())
        self._inside(helpers / "test_p144_activation_policy.py").write_bytes(pathlib.Path(__file__).read_bytes())
        self._inside(helpers / "command.json").write_text(json.dumps({"argv": [sys.executable, "-B", "-m", "unittest", *sys.argv[1:]]}))

    def tearDown(self):
        self.temp.cleanup()

    def _inside(self, path):
        resolved = pathlib.Path(path).resolve()
        self.assertTrue(resolved.is_relative_to(self.root), f"output escaped temp root: {resolved}")
        return resolved

    def _payload(self, n=3, strategy="adaptive"):
        threshold = n if strategy == "adaptive" else 0
        rows = []
        for position in policy.schedule(n):
            ready = strategy == "eager" or position["visit"] >= n
            dispatches = 4 if ready else 0
            row = dict(position)
            row.update({
                "result": position["body"],
                "register_dispatches": dispatches,
                "fallback_frames": 0 if ready else 1,
                "activation_peak_bytes": 64 if ready else 0,
                "activation_peak_frames": 1 if ready else 0,
                "activation_current_bytes": 0,
                "activation_current_frames": 0,
                "activation_active_scopes": 0,
                "activation_capacity_samples": 1,
                "activation_overflowed": False,
            })
            if strategy == "adaptive":
                builders = (position["body"] if position["phase"] == "initial" else 66) + int(ready)
                row.update({
                    "cache_state": "ready" if ready else "cold",
                    "entries": position["visit"],
                    "builder_starts": builders,
                    "published": builders,
                    "evictions": max(position["body"] - 63, 0) if position["phase"] == "initial" else 3,
                    "resident_entries": min(position["body"] + 1, 64) if position["phase"] == "initial" else 64,
                    "retained_bytes": 64,
                    "byte_limit": 1024,
                })
            rows.append(row)
        return {
            "schema": policy.ACTIVATION_SCHEMA,
            "schema_version": 1,
            "driver": policy.ACTIVATION_DRIVER,
            "case": "uniform66",
            "mode": "register",
            "feature_profile": "mvp",
            "source_fuel": None,
            "input": 0,
            "strategy": strategy,
            "threshold": None if strategy == "eager" else threshold,
            "schedule_threshold": n,
            "requested_calls": 67 * (n + 1),
            "actual_calls": 67 * (n + 1),
            "instrumented": True,
            "diagnostic": True,
            "activation_capacity_basis": policy.ACTIVATION_CAPACITY_BASIS,
            "per_call": rows,
        }, threshold

    def test_valid_eager_and_adaptive_activation_payloads(self):
        for strategy in ("eager", "adaptive"):
            with self.subTest(strategy=strategy):
                payload, threshold = self._payload(strategy=strategy)
                self.assertIs(policy.validate_activation_payload(payload, 3, strategy, threshold, 0), payload)
                self._inside(self.root / "helpers/command.json")

    def test_rejects_timing_stale_and_unexpected_wire_fields(self):
        payload, threshold = self._payload()
        for change in (
            lambda p: p.update({"prepare_ns": 0}),
            lambda p: p.update({"schema": policy.SCHEMA}),
            lambda p: p["per_call"][0].update({"elapsed_ns": 1}),
            lambda p: p.update({"unexpected": True}),
        ):
            changed = json.loads(json.dumps(payload))
            change(changed)
            with self.assertRaises(ValueError):
                policy.validate_activation_payload(changed, 3, "adaptive", threshold, 0)

        old = json.loads(json.dumps(payload))
        old["schema"] = policy.SCHEMA
        old["driver"] = "entry_sequence"
        old.pop("activation_capacity_basis")
        with self.assertRaises(ValueError):
            policy.validate_payload(old, 3, "adaptive", threshold, 0, diagnostic=True)

    def test_rejects_peak_reset_cold_sample_and_overflow_drift(self):
        mutations = (
            lambda row: row.update({"activation_peak_frames": 2}),
            lambda row: row.update({"activation_peak_bytes": 64}),
            lambda row: row.update({"activation_current_bytes": 1}),
            lambda row: row.update({"activation_active_scopes": 1}),
            lambda row: row.update({"activation_capacity_samples": 0}),
            lambda row: row.update({"activation_overflowed": True}),
            lambda row: row.update({"activation_peak_bytes": True}),
            lambda row: row.update({"activation_peak_bytes": 1 << 64}),
        )
        for mutation in mutations:
            payload, threshold = self._payload()
            mutation(payload["per_call"][0])
            with self.assertRaises(ValueError):
                policy.validate_activation_payload(payload, 3, "adaptive", threshold, 0)

        payload, threshold = self._payload()
        payload["per_call"][2]["activation_peak_bytes"] = 65
        with self.assertRaises(ValueError):
            policy.validate_activation_payload(payload, 3, "adaptive", threshold, 0)

        eager, threshold = self._payload(strategy="eager")
        eager["per_call"][0]["activation_peak_bytes"] = policy.MAX_ACTIVATION_BYTES + 1
        with self.assertRaises(ValueError):
            policy.validate_activation_payload(eager, 3, "eager", threshold, 0)


if __name__ == "__main__":
    unittest.main()
