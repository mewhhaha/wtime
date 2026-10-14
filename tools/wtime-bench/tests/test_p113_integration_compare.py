"""Mocked contract tests for P113 comparison; no encoder or native binary runs."""
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))
import compare
import integration_workloads as model


def _load_adapter():
    spec = importlib.util.spec_from_file_location("p113_test_integration_compare",
                                                  BENCH / "integration_compare.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


adapter = _load_adapter()


class P113IntegrationCompareTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p113-integration-compare-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.fixture = self.root / "tools" / "wtime-bench" / "fixtures" / "case.wasm"
        self.fixture.parent.mkdir(parents=True)
        self.fixture.resolve().relative_to(self.root)
        self.fixture.write_bytes(b"temporary mocked fixture")
        self.executables = {side: self.root / f"{side}-mock"
                            for side in ("baseline", "candidate")}

    def _wire(self, case, stage, batch=1, *, preflight=False, measured=None):
        state = model.expected(case)
        lifecycle = dict(state["lifecycle"])
        if "capture_bytes_minimum" in lifecycle:
            lifecycle["capture_bytes"] = lifecycle.pop("capture_bytes_minimum")
            lifecycle["after_resume_bytes"] = lifecycle.pop("after_resume_bytes_minimum")
            collection = dict(lifecycle["collection_after_root_release"])
            collection["reclaimed"] = collection.pop("reclaimed_minimum")
            lifecycle["collection_after_root_release"] = collection
        if measured is None:
            measured = max(1.0, batch * 10.0)
        result = {
            "schema_version": 2, **state, "lifecycle": lifecycle,
            "stage": stage, "mode": "compact", "profile": model.PROFILE,
            "instrumented": False, "diagnostic": False,
            "memory_sha256": state["memory_sha256"],
            "initial_memory_hex": state["initial_memory_hex"],
            "report_complete": True, "batch": batch, "calls_checked": batch,
        }
        if preflight:
            result["timing_excluded"] = True
            return result
        result.update(ns_per_operation=measured / batch,
                      measured_guest_ns=measured)
        return result

    def test_bad_full_image_preflight_stops_before_timed_path(self):
        fake_compare = SimpleNamespace()
        calls = []

        def process(executable, args, cpu):
            calls.append((Path(executable), list(args), cpu))
            row = self._wire("integration_gc_prefix", "first_call", preflight=True)
            row["memory_hex"] = "00"
            return row

        fake_compare.process = mock.Mock(side_effect=process)
        timed = mock.Mock()
        with self.assertRaisesRegex(ValueError, "memory image"):
            adapter._preflight_before_sampling(
                lambda: adapter._preflight_all(
                    [{"id": "integration_gc_prefix"}],
                    {"integration_gc_prefix": {"path": self.fixture}},
                    self.executables, 0, fake_compare, model), timed)
        timed.assert_not_called()
        fake_compare.process.assert_called_once()
        self.assertEqual(calls[0][1], ["--integration-driver", self.fixture,
                                       "integration_gc_prefix", "first_call", "32", "1",
                                       "compact", "--probe"])

    def test_mixed_mode_profile_or_instrumentation_is_rejected(self):
        good = {"schema_version": 2, "engine": "wtime", "instrumented": False,
                "modes": ["compact"], "features": [model.PROFILE]}
        adapter._validate_capabilities({"baseline": good, "candidate": good}, model.PROFILE)
        for bad in (dict(good, instrumented=True),
                    dict(good, modes=["decoded"]),
                    dict(good, features=["mvp"])):
            with self.subTest(bad=bad), self.assertRaisesRegex(
                    adapter.IntegrationComparisonError, "protocol"):
                adapter._validate_capabilities({"baseline": good, "candidate": bad}, model.PROFILE)
        wire = self._wire("integration_gc_prefix", "first_call", preflight=True)
        wire["instrumented"] = True
        with self.assertRaisesRegex(adapter.IntegrationComparisonError, "settings/instrumentation"):
            adapter._compact_wire(wire, model, "integration_gc_prefix", "first_call", 32,
                                  preflight=True)

    def test_balanced_random_orders_and_confirmed_shared_calibration(self):
        orders = adapter._balanced_orders(compare, 20261012, 30)
        self.assertEqual(len(orders), 30)
        self.assertEqual(sum(order[0] == "baseline" for order in orders), 15)
        self.assertTrue(all(set(order) == {"baseline", "candidate"} for order in orders))

        fake_compare = SimpleNamespace()
        calls = []

        def process(executable, args, cpu):
            requested = int(args[5])
            side = "baseline" if Path(executable) == self.executables["baseline"] else "candidate"
            actual = (4 if side == "baseline" else 5) if requested == 0 else requested
            measured = actual * 5_000_000.0
            calls.append((side, requested, actual))
            return self._wire("integration_numeric_control", "warm_call", actual,
                              measured=measured)

        fake_compare.process = mock.Mock(side_effect=process)
        def observe(side, _fixture, case, stage, n, requested):
            return fake_compare.process(self.executables[side],
                ["--integration-driver", self.fixture, case, stage, str(n), str(requested), "compact"], 0)

        result = adapter._calibrate("integration_numeric_control", 32, self.fixture,
                                    observe, model)
        self.assertTrue(result["passed"])
        self.assertEqual(result["paired_batch"], 5)
        self.assertEqual([item[1] for item in calls], [0, 0, 5, 5])
        self.assertEqual(calls[2][2], calls[3][2])

    def test_batch_zero_calibration_below_target_fails_hard(self):
        def observe(_side, _fixture, _case, _stage, _n, _requested):
            return self._wire("integration_numeric_control", "warm_call", 1,
                              measured=adapter.TARGET_NS - 1)

        with self.assertRaisesRegex(adapter.IntegrationComparisonError,
                                    "batch-0 calibration did not reach 20 ms"):
            adapter._calibrate("integration_numeric_control", 32, self.fixture,
                               observe, model)


if __name__ == "__main__":
    unittest.main()
