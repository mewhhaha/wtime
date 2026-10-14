"""Version 2 continuation fixtures and independent semantic oracle metadata.

This module describes work for the continuation-specific host driver. A
suspended invocation is modeled as a suspension payload plus a later resumed
result; it is never represented as a generic returned timing value.
"""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
SCHEMA_VERSION = 2
PROFILE_STATUS = "ready"
STAGE_LABELS = ("setup", "capture", "resume", "accounting")

_FILES = {
    "continuation_depth_0": ("depth_0.wat", "c4d1378e50d36ff9ec82b4f78a0f83783664c002c31e27ccfb751da5d97451ac",
                             "2e30292dab3ac305fd029a6b8ff92d5af14f96900b40d5053317c75e8ba589a3"),
    "continuation_depth_4": ("depth_4.wat", "05cd6aec67fe1eee5466a1bb39cdf7bc1d701f2fbe41b88620eb62ce06c7d554",
                             "880e23776c57398a7fa046424de5c8a9943d1af61935f69f85fc6b33ed7b5d17"),
    "continuation_depth_32": ("depth_32.wat", "8fc4cd5704aac0b9ca91a13ff8ac87957cd30406b43994fa5eadab23d00ba226",
                              "fcc256cb133113e2a21aebab749bd5e0e5a14ebcb514492f740facffca1d1189"),
    "continuation_captured_reference": ("captured_reference.wat", "f04c26de19fe09fce7bfb0182559bd2976de582b891712d304dc4bc760c6f7d1",
                                         "5b2e701d1eaede5060f35b08fbfec8fae7ad28851676f2d92535c55e4e5bbf1c"),
    "continuation_bound_nested": ("bound_nested.wat", "5bb9f6ec7c849fd71bb91e7e9006ad6b954ab723a724442539bdb956c1c66f70",
                                  "f7297ddc64454c2dc9ecdd0f58a3d51a05679922dd8b464f78fb0088eefe89c6"),
    "continuation_one_shot": ("one_shot.wat", "1d33035426adde6a81cb71f9fe3457e869bfd810c68c728e5868b3196daf9ae9",
                              "b17fb545ed18f933b6a19ca53876ef334a5d925b6511f7b51b84f0f55f9164c3"),
    "continuation_cross_module_provider": ("cross_module_provider.wat", "4e69dd91d7bc14d7e8ba630138ab2ae4c58accdf9de0f284bcfdfcda13b6e651",
                                           "d6d7298a08c1888fe15c44b430187f10b5b8734ca04dc25b627b2149860c2290"),
    "continuation_cross_module_consumer": ("cross_module_consumer.wat", "6c9a7c8e869e9f3c1d931b63251032bae765c20c78d06f11aa5ff9e1fb24256d",
                                           "85d6081efdbf66f3b825d6847fd0831e25c0be110e51feccca99b56595adb2ab"),
}


def _hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def depth_oracle(depth: int, *, resume_value: int = 9, payload: int = 7) -> dict[str, Any]:
    """Independent model for the numeric nested-frame fixture family."""
    if depth not in (0, 4, 32):
        raise ValueError("depth must be one of 0, 4, or 32")
    return {
        "expected_suspension": {
            "kind": "suspended", "payload": [{"type": "i32", "value": payload}],
        },
        "expected_resume": {
            "kind": "returned",
            "result": [{"type": "i32", "value": 1000 + resume_value + depth * (depth + 1) // 2}],
        },
        "captured_logical_frames": depth + 1,
        "captured_numeric_locals": depth + 1,
        "wrapper_local_values": list(range(1, depth + 1)),
    }


def accounting_metadata() -> dict[str, Any]:
    """Describe observable byte checkpoints without asserting a Rust layout."""
    return {
        "api": "Store.continuation_bytes",
        "checkpoints": ["before_capture", "after_capture", "after_root_transfer",
                        "after_resume", "after_drop_and_collect"],
        "capture_expectation": "positive for a retained suspended segment",
        "release_expectation": "record bytes after consumption/drop; require eventual zero",
        "asymptotic_driver": {
            "fixture_ids": ["continuation_depth_0", "continuation_depth_4", "continuation_depth_32"],
            "independent_variable": "captured logical guest frame count",
            "report": ["continuation_bytes", "bytes_per_captured_frame"],
            "rule": "report measured values; make no exact object-layout or linearity claim from three sizes",
        },
        "boundary_controls": [
            "zero max_continuation_bytes rejects the first capture",
            "one-less-than-required fuel exits at the guest source boundary",
        ],
    }


def _entry(workload_id: str, *, stages: tuple[str, ...] = STAGE_LABELS,
           features: tuple[str, ...] = ("mvp", "stack_switching"),
           oracle: dict[str, Any], host_setup: dict[str, Any] | None = None,
           accounting: dict[str, Any] | None = None,
           description: str) -> dict[str, Any]:
    filename, source_sha, wasm_sha = _FILES[workload_id]
    return {
        "schema_version": SCHEMA_VERSION,
        "id": workload_id,
        "source": {"kind": "wat_file", "path": f"fixtures/continuations/{filename}",
                   "sha256": source_sha, "wasm_sha256": wasm_sha},
        "stages": list(stages),
        "profile": {"status": PROFILE_STATUS, "reason": "public continuation driver provides separate capture/resume/accounting"},
        "required_features": list(features),
        "oracle": {"kind": "semantic", "function": "continuation_workloads.py",
                   "independent": True, **oracle},
        "host_setup": host_setup or {"setup": ["fresh engine, store, and instance per sample"]},
        "continuation_bytes": accounting or accounting_metadata(),
        "description": description,
    }


def workload_entries() -> list[dict[str, Any]]:
    """Return immutable-by-convention v2 metadata entries for P012's driver."""
    entries: list[dict[str, Any]] = []
    for depth in (0, 4, 32):
        workload_id = f"continuation_depth_{depth}"
        entries.append(_entry(workload_id, oracle=depth_oracle(depth),
                              host_setup={"setup": ["fresh engine, store, and instance per sample"],
                                          "capture": "invoke exported run and require RunOutcome::Suspended",
                                          "resume": "pass the returned continuation to exported resume",
                                          "outcome_contract": "check payload and resumed result separately"},
                              description=f"Guest call-chain capture with {depth} wrapper frames and saved numeric locals."))

    entries.append(_entry(
        "continuation_captured_reference", features=("mvp", "gc", "stack_switching"),
        oracle={"expected_suspension": {"kind": "suspended", "payload": [{"type": "i32", "value": 5}]},
                "expected_resume": {"kind": "returned", "result": [{"type": "i32", "value": 86}]},
                "captured_reference": {"type": "struct", "field_i32": 77}},
        host_setup={"setup": ["fresh engine, store, and instance per sample"],
                    "capture": "invoke exported run and require payload [i32 5]",
                    "root_transfer": ["root continuation", "release published outcome ownership"],
                    "collection": "call Store.collect before guest resume",
                    "resume": "invoke exported resume with the rooted continuation",
                    "expected_result": "i32 86; the captured struct field remains 77"},
        description="Suspended frame retains a GC struct local across collection and guest resume."))

    entries.append(_entry(
        "continuation_bound_nested", stages=("setup", "bind", "resume", "accounting"),
        oracle={"expected_resume": {"kind": "returned", "result": [{"type": "i32", "value": 0},
                                                                            {"type": "i32", "value": 1122}]},
                "expected_worker_calls": 1,
                "binding_order": ["bind c0 to c1 with 11", "bind c1 to c2 with 22", "resume c2"]},
        description="Nested guest cont.bind composes captured arguments and consumes the bound chain once."))

    entries.append(_entry(
        "continuation_one_shot", stages=("setup", "capture", "resume_once", "resume_reuse", "cleanup"),
        oracle={"expected_first_resume": {"kind": "returned", "result": [{"type": "i32", "value": 1}]},
                "expected_second_resume": {"kind": "trap", "category": "Trap"},
                "reason": "resume consumes the same guest continuation exactly once"},
        description="A guest driver attempts to resume one continuation twice; second use must trap."))

    entries.append(_entry(
        "continuation_cross_module_provider", stages=("setup", "nominal_context"),
        oracle={"role": "exported provider tag", "provided_nominal_shape": "struct { i32 }"},
        host_setup={"setup": ["compile provider and consumer in the same engine"],
                    "link": "define provider.yield in the consumer linker"},
        description="Provider module defines the exported tag and a same-shaped nominal GC type."))
    entries.append(_entry(
        "continuation_cross_module_consumer", stages=("setup", "capture", "resume", "nominal_context"),
        features=("mvp", "gc", "stack_switching"),
        oracle={"expected_suspension": {"kind": "suspended", "payload": []},
                "expected_resume": {"kind": "returned", "result": [{"type": "i32", "value": 77}]},
                "nominal_probe": {"same_shape_cross_module_tag": "accepted",
                                  "incompatible_exact_host_result": "rejected with Host error"}},
        host_setup={"setup": ["compile provider and consumer in one engine", "instantiate provider first",
                               "link exported provider.yield tag into consumer"],
                    "capture": "consumer run returns Suspended; inspect as suspended, never Returned",
                    "resume": "consumer resume constructs its local nominal object and returns field 77",
                    "negative_probe": "host binding with exact consumer result type must reject the provider suspension"},
        description="Cross-module continuation preserves imported tag context and nominal GC type exactness."))
    return entries


def verify_fixture_sources() -> list[str]:
    """Check source hashes and v2 metadata against the checked-in WAT files."""
    errors = []
    for entry in workload_entries():
        source = entry["source"]
        path = HERE / source["path"]
        if not path.is_file():
            errors.append(f"{entry['id']}: missing fixture {path}")
            continue
        if _hash(path.read_bytes()) != source["sha256"]:
            errors.append(f"{entry['id']}: stale WAT source hash")
        if len(source["wasm_sha256"]) != 64:
            errors.append(f"{entry['id']}: invalid encoded Wasm hash")
        if entry["schema_version"] != SCHEMA_VERSION or entry["profile"]["status"] != PROFILE_STATUS:
            errors.append(f"{entry['id']}: invalid schema/profile metadata")
    return errors


if __name__ == "__main__":
    failures = verify_fixture_sources()
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"validated {len(workload_entries())} continuation workload entries (schema v{SCHEMA_VERSION})")
