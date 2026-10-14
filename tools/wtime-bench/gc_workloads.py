"""GC, reference and exception workload metadata with normalized oracles.

The normal invocation oracle returns a signed scalar checksum. Cases whose
observable outcome is an uncaught exception or a collector lifecycle expose a
separate normalized outcome/state expectation; no raw Store-local handle index
is part of the comparison contract.
"""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
GC_FIXTURES = HERE / "fixtures" / "gc"
EXCEPTION_FIXTURES = HERE / "fixtures" / "exceptions"
INPUTS = (1, 32, 1024)
RESET_POLICY = "fresh_instance_per_invocation"
STAGES = ["load_and_call"]

WASM_SHA256 = {
    "caught_throw.wat": "9ae413316b376fffe5b1755f7fb842b8b9d6f3a6deefef9c0e72fff84f080eb9",
    "cross_call_catch.wat": "58971ea6345c2693f4fc0cf173b2b13a61a108d5da8bd395de5b4aa70e119f44",
    "continuation_root.wat": "f3a432da4b984b715e74e26016d593a61e5dd4b24e421d1434bf63bf83c39f31",
    "descriptor_consumer.wat": "991fabf92a687b0557bd6e7b0959c500dca3bd4549fc3ccce1ad517cf44438ca",
    "descriptor_graph.wat": "474d6c1d97ac8548a942098293451ac53e66561e199e95dcfa91bfd72ed9d443",
    "descriptor_provider.wat": "a3f7000c6231b8acf83b56a22e08b6f4fceff4dc7930712f16c7b734d6d9cbdb",
    "host_reentry_collect.wat": "63852bfe7dd4659db0cabbd8a01fe4249734ea4a719c8a0dcb5396e1c8a7c8e4",
    "host_throw_catch.wat": "16fc8471e8b82acabbb7a1b1c5501ea0b9df884c539f452e49cc4f39c834eec2",
    "nominal_exact_casts.wat": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "normal_path.wat": "c4b8a9ad3a665906d9ee6528c32f784520cf1322934645bdb16c5c171ba49721",
    "packed_array.wat": "fbfb37283a67bee4e8a6a769a8491e68115758c321885cd776cbb4ce9bbceec1",
    "root_lifecycle.wat": "7210514252b38d64d46fb0d7452e91aa838ae87077922e9277aa81b65bd9fa5f",
    "struct_graph.wat": "641b2835a88599370dfb09c90dca596caded5d22deb98f4300435b4900aced0d",
    "throw_ref_unwind.wat": "2567e8fbc746714d9e316044665a2501670b77946f55e66b63938b99b17c8936",
    "typed_function_refs.wat": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "uncaught_throw.wat": "365142a10cf7d4a6da89ca87f404d5c21e4bb442c5daf0d5053a5ce81232837e",
}

FAMILIES = {
    "gc_packed_array": ("packed_array.wat", "array_elements", "run"),
    "gc_struct_graph": ("struct_graph.wat", "objects", "run"),
    "gc_nominal_exact_casts": ("nominal_exact_casts.wat", "cast_checks", "run"),
    "gc_typed_function_refs": ("typed_function_refs.wat", "typed_calls", "run"),
    "gc_descriptor_graph": ("descriptor_graph.wat", "objects", "run"),
    "gc_descriptor_cross_module": ("descriptor_consumer.wat", "objects", "run"),
    "gc_root_lifecycle": ("root_lifecycle.wat", "objects", "checksum"),
    "gc_host_reentry_collect": ("host_reentry_collect.wat", "host_safepoints", "run"),
    "gc_continuation_root": ("continuation_root.wat", "continuation_resumptions", "run"),
    "exception_caught_throw": ("caught_throw.wat", "throws_caught", "run"),
    "exception_normal_path": ("normal_path.wat", "normal_exception_region_calls", "run"),
    "exception_cross_call_catch": ("cross_call_catch.wat", "cross_call_throws", "run"),
    "exception_host_throw_catch": ("host_throw_catch.wat", "host_throws_caught", "run"),
    "exception_throw_ref_unwind": ("throw_ref_unwind.wat", "reference_rethrows", "run"),
    "exception_uncaught_throw": ("uncaught_throw.wat", "uncaught_exceptions", "run"),
}


def _signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _count(n: int) -> int:
    if type(n) is not int or not 0 <= n <= 0x7FFF_FFFF:
        raise ValueError("n must be a nonnegative signed i32")
    return n


def _family(workload_id: str) -> str:
    for family in FAMILIES:
        if workload_id == family or workload_id.startswith(family + "_n"):
            return family
    raise ValueError(f"unknown GC/exception workload id: {workload_id}")


def oracle(workload_id: str, n: int) -> int | dict[str, Any]:
    """Return independent scalar checksum or normalized exceptional outcome."""
    n = _count(n)
    family = _family(workload_id)
    if family in {"gc_packed_array"}:
        return _signed32(-3 * n)
    if family in {"gc_struct_graph", "gc_root_lifecycle"}:
        return _signed32(n * (n - 1) // 2)
    if family == "gc_nominal_exact_casts":
        # Each subtype passes the nominal base and child tests, but fails the
        # exact-base test: 1 + 0 + 1 successful tests per allocated object.
        return _signed32(2 * n)
    if family in {"gc_descriptor_graph", "gc_descriptor_cross_module"}:
        return n
    if family == "gc_typed_function_refs":
        return _signed32(n * (n - 1) // 2 + n)
    if family in {"gc_host_reentry_collect", "gc_continuation_root"}:
        return n
    if family in {"exception_caught_throw", "exception_normal_path", "exception_cross_call_catch",
                  "exception_host_throw_catch"}:
        return n
    if family == "exception_throw_ref_unwind":
        return n
    if family == "exception_uncaught_throw":
        return {"kind": "exception", "tag": "number", "payload": [n]}
    raise AssertionError(f"missing oracle for {family}")


def _path_for(filename: str) -> Path:
    if filename in {"caught_throw.wat", "cross_call_catch.wat", "host_throw_catch.wat",
                    "normal_path.wat", "throw_ref_unwind.wat", "uncaught_throw.wat"}:
        return EXCEPTION_FIXTURES / filename
    return GC_FIXTURES / filename


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _setup_modules(family: str) -> list[dict[str, str]]:
    if family != "gc_descriptor_cross_module":
        return []
    provider = GC_FIXTURES / "descriptor_provider.wat"
    return [{"module_name": "provider", "path": "fixtures/gc/descriptor_provider.wat",
             "sha256": _sha(provider), "wasm_sha256": WASM_SHA256[provider.name]}]


def _required_features(family: str) -> list[str]:
    if family.startswith("exception_"):
        return ["exception_handling", "gc"]
    if family == "gc_continuation_root":
        return ["gc", "custom_descriptors", "exception_handling", "stack_switching"]
    if family == "gc_typed_function_refs":
        return ["gc", "function_references"]
    if family in {"gc_descriptor_graph", "gc_descriptor_cross_module", "gc_host_reentry_collect"}:
        return ["gc", "custom_descriptors"]
    return ["gc"]


def _collector_stage(family: str, n: int) -> dict[str, Any] | None:
    if family == "gc_host_reentry_collect":
        return {
            "name": "collector_stage",
            "kind": "host_reentrant_store_collect",
            "status": "host_driver_required",
            "checkpoint": "during descriptor construction, the child is the sole temporary guest operand root",
            "expected": {"live": 1, "reclaimed": 0, "survivor": "temporary_operand_child"},
            "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                        "Store::gc_payload_bytes"],
        }
    if family == "gc_continuation_root":
        return {
            "name": "collector_stage",
            "kind": "suspended_continuation_root",
            "status": "host_driver_required",
            "checkpoint": "collect is called while the worker segment holds its sole object and descriptor references",
            "expected": {"live_min": 3, "reclaimed": 1,
                         "survivor": "continuation_segment_object_and_descriptor"},
            "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                        "Store::gc_payload_bytes", "Store::continuation_bytes"],
        }
    if family == "gc_root_lifecycle":
        return {
            "name": "collector_stage",
            "kind": "host_root_release_and_reclaim",
            "status": "host_driver_required",
            "sequence": [
                {"action": "collect after checksum invocation",
                 "expected": {"live": 0, "reclaimed": "n"}},
                {"action": "call make and register root_value", "root_category": "host_value_root"},
                {"action": "collect", "expected": {"live": 1, "reclaimed": 0}},
                {"action": "drop RootToken and collect", "expected": {"live": 0, "reclaimed": 1}},
            ],
            "isolation": "run as a separately named collector stage after load_and_call",
            "identity_comparison": "normalized_root_category; never compare raw GC handle indices",
            "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                        "Store::gc_payload_bytes"],
        }
    allocation_count = {
        "gc_packed_array": 1,
        "gc_struct_graph": n,
        "gc_nominal_exact_casts": n,
        "gc_typed_function_refs": 0,
        "gc_descriptor_graph": 3,
        "gc_descriptor_cross_module": 3,
    }.get(family)
    if allocation_count is not None:
        return {
            "name": "collector_stage",
            "kind": "post_invocation_store_collect",
            "status": "host_driver_required",
            "expected": {"live": 0, "reclaimed": allocation_count,
                         "normalized_roots": []},
            "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                        "Store::gc_payload_bytes"],
        }
    return None


def entries() -> list[dict[str, Any]]:
    """Return schema-v2 workload entries for GC, descriptor and exception cases."""
    result = []
    for family, (filename, unit, entrypoint) in FAMILIES.items():
        source_path = _path_for(filename)
        source = {
            "kind": "wat_file",
            "path": source_path.relative_to(HERE).as_posix(),
            "sha256": _sha(source_path),
            "wasm_sha256": WASM_SHA256[filename],
            "encoding_status": "encoded",
        }
        for n in INPUTS:
            wid = f"{family}_n{n}"
            is_uncaught = family == "exception_uncaught_throw"
            features = _required_features(family)
            item: dict[str, Any] = {
                "id": wid,
                "family": family,
                "source": source.copy(),
                "params": [
                    {"name": "n", "type": "i32", "min": n, "max": n, "baseline": n}
                ],
                "result": {"type": "none" if is_uncaught else "i32"},
                "oracle": {"kind": "python", "module": "gc_workloads.py",
                           "function": "oracle", "independent": True},
                "required_features": features,
                "work_units": {
                    "count": n if family in {"gc_packed_array", "gc_struct_graph",
                                             "gc_nominal_exact_casts", "gc_root_lifecycle",
                                             "gc_typed_function_refs"} else 1,
                    "unit": unit if family in {"gc_packed_array", "gc_struct_graph",
                                                "gc_nominal_exact_casts", "gc_root_lifecycle",
                                                "gc_typed_function_refs"} else
                             "descriptor_graphs" if family in {"gc_descriptor_graph",
                                                                "gc_descriptor_cross_module"} else
                             "host_collection_safepoints" if family == "gc_host_reentry_collect" else
                             "continuation_resumptions" if family == "gc_continuation_root" else
                             "exception_paths",
                },
                "reset_policy": RESET_POLICY,
                "stages": STAGES.copy(),
                "comparators": {"wasmtime": {
                    "status": "unsupported",
                    "reason": "The pinned wtime-bench Wasmtime 49.0.2 build omits its gc Cargo feature; wasm_exceptions also requires GC compiled into the build.",
                }},
                "entrypoint": entrypoint,
                "setup_modules": _setup_modules(family),
                "expected_outcome": "exception" if is_uncaught else "return",
                "reference_normalization": "structural/type category and payload; never raw engine handle indices",
            }
            if family.startswith("gc_"):
                item["gc_expectation"] = {
                    "allocation_count": 0 if family == "gc_typed_function_refs" else
                                        1 if family == "gc_packed_array" else
                                        2 if family == "gc_continuation_root" else
                                        n if family in {"gc_struct_graph", "gc_nominal_exact_casts",
                                                        "gc_root_lifecycle"} else 3,
                    "oracle_result": oracle(wid, n),
                    "live_bytes": "record Store::gc_payload_bytes; byte values are engine-specific and measured, not predicted",
                }
                collector = _collector_stage(family, n)
                if collector is not None:
                    item["collector_stage"] = collector
                    item["requires_host_driver"] = True
            else:
                item["exception_expectation"] = {
                    "outcome": oracle(wid, n),
                    "caught": family in {"exception_caught_throw", "exception_cross_call_catch",
                                          "exception_host_throw_catch"},
                    "path": "normal" if family == "exception_normal_path" else
                            "uncaught" if is_uncaught else "caught_throw",
                    "unwind": family in {"exception_cross_call_catch", "exception_throw_ref_unwind"},
                }
            if family == "gc_descriptor_cross_module":
                item["requires_host_driver"] = True
                item["module_setup"] = "instantiate provider, bind provider.make, then instantiate consumer"
                item["state_expectation"] = {"result": n,
                                              "object": "provider-owned nominal descriptor instance"}
            if family == "gc_host_reentry_collect":
                item["host_imports"] = [{"module": "env", "name": "collect",
                                         "signature": "() -> i32", "returns": 9}]
                item["state_expectation"] = {
                    "during_callback": {"live": 1, "reclaimed": 0,
                                         "retained_root": "temporary_operand_child"},
                    "after_guest_return_then_collect": {"live": 0, "reclaimed": 3},
                }
                item["post_invocation_collection"] = {
                    "name": "post_invocation_collector_stage",
                    "kind": "store_collect",
                    "status": "host_driver_required",
                    "expected": {"live": 0, "reclaimed": 3},
                    "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                                "Store::gc_payload_bytes"],
                }
            if family == "gc_continuation_root":
                item["requires_host_driver"] = True
                item["host_imports"] = [{"module": "env", "name": "collect",
                                         "signature": "() -> none", "action": "Store::collect"}]
                item["state_expectation"] = {
                    "during_collection": {"live_min": 3, "reclaimed": 1,
                                          "root_category": "suspended_guest_segment"},
                    "after_resume": {"continuation_consumed": True, "result": n,
                                     "continuation_bytes": "record; expected zero for consumed segment"},
                }
                item["post_invocation_collection"] = {
                    "name": "post_invocation_collector_stage",
                    "kind": "store_collect_after_resume",
                    "status": "host_driver_required",
                    "expected": {"live": 0, "reclaimed": "all fixture GC objects"},
                    "metrics": ["CollectionStats.live", "CollectionStats.reclaimed",
                                "Store::gc_payload_bytes", "Store::continuation_bytes"],
                }
            if is_uncaught:
                item["exception_tag"] = "number"
            if family == "exception_host_throw_catch":
                item["requires_host_driver"] = True
                item["host_imports"] = [
                    {"module": "env", "name": "number", "kind": "tag", "payload": ["i32"]},
                    {"module": "env", "name": "raise", "kind": "function",
                     "outcome": {"kind": "exception", "tag": "number", "payload": [n]}},
                ]
            result.append(item)
    return result


def encode_fixtures(encoder: str | Path, targetdir: str | Path) -> dict[str, str]:
    """Encode all GC/exception WAT with an existing encoder; never build it."""
    target = Path(targetdir)
    target.mkdir(parents=True, exist_ok=True)
    hashes: dict[str, str] = {}
    for filename, expected in WASM_SHA256.items():
        source = _path_for(filename)
        output = target / f"{source.stem}.wasm"
        subprocess.run([str(encoder), "encode", str(source), "-o", str(output)],
                       check=True, cwd=HERE.parent.parent)
        digest = _sha(output)
        if digest != expected:
            raise ValueError(f"encoded fixture identity mismatch for {filename}: {digest}")
        hashes[filename] = digest
    return hashes
