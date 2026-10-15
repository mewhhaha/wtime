"""Pure source-size and optional-tier footprint contract tests for P110."""
from __future__ import annotations

import unittest

from tools.verification.producers import p110


MODES = ("decoded", "compact", "register-eager", "register-adaptive-3")


def observation(mode: str, length: int) -> dict:
    value = {
        "schema": "wtime-module-footprint-v1",
        "status": "passed",
        "timing_evidence": False,
        "instrumented": True,
        "complete_heap_census": False,
        "mode": mode,
        "profile": "mvp",
        "input": "fixture.wasm",
        "wasm_bytes": length,
        "source_bytes": length,
        "lowered_control_bytes": 64,
        "function_plan_bytes": 96,
        "guest_invocations": 0,
        "compact_preparation_counts": None,
        "register_cache_state": "not_requested",
        "register_cache_snapshot": None,
    }
    if mode == "compact":
        value["compact_preparation_counts"] = {
            "accepted_functions": 2,
            "declined_functions": 0,
            "attempted_work": 32,
            "retained_bytes": 256,
        }
    if mode == "register-adaptive-3":
        # prepare_large is a real compile-time fallback: it declines cache
        # construction, so the inspection tool reports this explicitly.
        value["register_cache_state"] = "unavailable" if length == 128_000 else "empty"
        if value["register_cache_state"] == "empty":
            value["register_cache_snapshot"] = {
                "state_after_compile": "empty",
                "threshold": 3,
                "entry_limit": 64,
                "entry_count": 0,
                "active_body": None,
                "retained_bytes": 128,
                "byte_limit": 1_048_576,
                "remaining_attempted_work": 65_536,
                "remaining_retained_work": 65_536,
                "lifecycle": {
                    "preparation_starts": 0,
                    "builder_starts": 0,
                    "completed": 0,
                    "published": 0,
                    "publication_failures": 0,
                    "declined": 0,
                    "exhausted": 0,
                    "invalid": 0,
                    "aborted": 0,
                    "evictions": 0,
                },
            }
    return value


class P110FootprintTests(unittest.TestCase):
    def test_accepts_all_modes_for_small_large_and_representative_source_lengths(self):
        fixture_lengths = (122, 128_000, 4096)
        for length in fixture_lengths:
            for mode in MODES:
                with self.subTest(length=length, mode=mode):
                    value = observation(mode, length)
                    self.assertIsNone(p110.validate_observation(value, mode, length))

    def test_counters_are_nonnegative_typed_and_source_lengths_exact_without_guest_runs(self):
        bad_rows = (
            ("wasm_bytes", True),
            ("source_bytes", -1),
            ("lowered_control_bytes", False),
            ("function_plan_bytes", -1),
            ("guest_invocations", 1),
            ("guest_invocations", True),
            ("source_bytes", 100),
        )
        for field, invalid in bad_rows:
            with self.subTest(field=field, invalid=invalid):
                value = observation("decoded", 122)
                value[field] = invalid
                with self.assertRaises(ValueError):
                    p110.validate_observation(value, "decoded", 122)

    def test_adaptive_compile_only_cache_must_be_empty_and_within_byte_limits(self):
        mutations = (
            ("entry_count", 1),
            ("active_body", 0),
            ("retained_bytes", 1_048_577),
            ("remaining_attempted_work", -1),
        )
        for field, invalid in mutations:
            with self.subTest(field=field):
                value = observation("register-adaptive-3", 4096)
                value["register_cache_snapshot"][field] = invalid
                with self.assertRaises(ValueError):
                    p110.validate_observation(value, "register-adaptive-3", 4096)
        value = observation("register-adaptive-3", 4096)
        value["register_cache_snapshot"]["lifecycle"]["builder_starts"] = 1
        with self.assertRaises(ValueError):
            p110.validate_observation(value, "register-adaptive-3", 4096)
        value = observation("register-adaptive-3", 4096)
        del value["register_cache_snapshot"]["lifecycle"]["aborted"]
        with self.assertRaises(ValueError):
            p110.validate_observation(value, "register-adaptive-3", 4096)

    def test_adaptive_unavailable_cache_is_explicit_and_never_fabricates_a_snapshot(self):
        value = observation("register-adaptive-3", 128_000)
        self.assertIsNone(p110.validate_observation(value, "register-adaptive-3", 128_000))

        fake_snapshot = observation("register-adaptive-3", 4096)["register_cache_snapshot"]
        value["register_cache_snapshot"] = fake_snapshot
        with self.assertRaises(ValueError):
            p110.validate_observation(value, "register-adaptive-3", 128_000)

        value["register_cache_state"] = "assumed-empty"
        value["register_cache_snapshot"] = None
        with self.assertRaises(ValueError):
            p110.validate_observation(value, "register-adaptive-3", 128_000)

    def test_compact_counters_are_owned_by_compact_mode_only(self):
        decoded_with_compact_counts = observation("decoded", 122)
        decoded_with_compact_counts["compact_preparation_counts"] = observation("compact", 122)[
            "compact_preparation_counts"]
        with self.assertRaises(ValueError):
            p110.validate_observation(decoded_with_compact_counts, "decoded", 122)

        compact = observation("compact", 122)
        compact["compact_preparation_counts"]["retained_bytes"] = -1
        with self.assertRaises(ValueError):
            p110.validate_observation(compact, "compact", 122)

        eager_with_cache = observation("register-eager", 122)
        eager_with_cache["register_cache_snapshot"] = observation("register-adaptive-3", 122)[
            "register_cache_snapshot"]
        with self.assertRaises(ValueError):
            p110.validate_observation(eager_with_cache, "register-eager", 122)


if __name__ == "__main__":
    unittest.main()
