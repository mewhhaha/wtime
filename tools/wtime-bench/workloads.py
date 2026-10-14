"""Versioned workload metadata and typed JSON value normalization."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
SCHEMA_VERSION = 2
STAGES = {"prepare", "instantiate", "first_call", "warm_call", "load_and_call",
          "setup", "capture", "resume", "accounting", "bind",
          "resume_once", "resume_reuse", "cleanup", "nominal_context", "steady_state_group", "run_after_setup"}
RESET_POLICIES = {
    "fresh_instance_for_cold_stages_reused_instance_for_warm_call",
    "fresh_instance_per_invocation",
    "fresh_engine_per_prepare_sample",
}
REFERENCE_CATEGORIES = {
    "null",
    "func",
    "extern",
    "struct",
    "array",
    "i31",
    "exception",
    "continuation",
}
ID_PATTERN = re.compile(r"^[a-z][a-z0-9_-]*$")
HASH_PATTERN = re.compile(r"^[0-9a-f]{64}$")


class WorkloadError(ValueError):
    """The registry or a requested workload value violates its contract."""


class EmptySelectionError(WorkloadError):
    """A declared but not-yet-populated selection was requested for launch."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise WorkloadError(message)


def validate_registry(registry: dict[str, Any]) -> dict[str, Any]:
    """Validate registry structure, workload contracts, and explicit selections."""
    _require(isinstance(registry, dict), "registry must be a JSON object")
    _require(registry.get("schema_version") == SCHEMA_VERSION, "schema_version must be 2")
    workloads = registry.get("workloads")
    selections = registry.get("selections")
    _require(isinstance(workloads, list), "workloads must be a list")
    _require(isinstance(selections, dict), "selections must be an object")

    by_id: dict[str, dict[str, Any]] = {}
    for entry in workloads:
        _require(isinstance(entry, dict), "each workload must be an object")
        wid = entry.get("id")
        _require(isinstance(wid, str) and ID_PATTERN.fullmatch(wid) is not None,
                 f"invalid workload id: {wid!r}")
        _require(wid not in by_id, f"duplicate workload id: {wid}")
        by_id[wid] = entry

        source = entry.get("source")
        _require(isinstance(source, dict), f"{wid}: missing deterministic source")
        _require(source.get("kind") in {"wat_file", "deterministic_generator"},
                 f"{wid}: unsupported source kind")
        _require(isinstance(source.get("path"), str) and source["path"],
                 f"{wid}: source path is required")
        for hash_key in ("sha256", "wasm_sha256"):
            _require(isinstance(source.get(hash_key), str)
                     and HASH_PATTERN.fullmatch(source[hash_key]) is not None,
                     f"{wid}: {hash_key} must be a lowercase SHA-256")

        params = entry.get("params")
        _require(isinstance(params, list), f"{wid}: params must be a list")
        names: set[str] = set()
        for param in params:
            _require(isinstance(param, dict), f"{wid}: each param must be an object")
            name = param.get("name")
            _require(isinstance(name, str) and name not in names,
                     f"{wid}: missing or duplicate parameter name")
            names.add(name)
            _require(param.get("type") in {"i32", "i64", "f32_bits", "f64_bits", "v128_bits", "reference"},
                     f"{wid}.{name}: unsupported parameter type")
            if "min" in param or "max" in param:
                low, high = param.get("min"), param.get("max")
                _require(type(low) is int and type(high) is int and low <= high,
                         f"{wid}.{name}: bounds must be ordered integers")
                if "baseline" in param:
                    _require(type(param["baseline"]) is int and low <= param["baseline"] <= high,
                             f"{wid}.{name}: baseline must fall within its bounds")

        result = entry.get("result")
        _require(isinstance(result, dict) and result.get("type") in
                 {"none", "i32", "i64", "f32_bits", "f64_bits", "v128_bits", "reference"},
                 f"{wid}: result type is missing or unsupported")
        oracle = entry.get("oracle")
        _require(isinstance(oracle, dict)
                 and oracle.get("kind") in {"python", "semantic"}
                 and isinstance(oracle.get("function"), str) and oracle["function"]
                 and oracle.get("independent") is True,
                 f"{wid}: an independent registered oracle is required")
        features = entry.get("required_features")
        _require(isinstance(features, list) and features
                 and all(isinstance(feature, str) and feature for feature in features),
                 f"{wid}: required_features must be a nonempty string list")
        units = entry.get("work_units")
        _require(isinstance(units, dict) and isinstance(units.get("unit"), str)
                 and units["unit"], f"{wid}: work_units must name a unit")
        if "count" in units:
            _require(type(units["count"]) is int and units["count"] >= 0,
                     f"{wid}: work-unit count must be a nonnegative integer")
        if "count_from_param" in units:
            _require(units["count_from_param"] in names,
                     f"{wid}: work units refer to an unknown parameter")
        _require("count" in units or "count_from_param" in units,
                 f"{wid}: work-unit count is required")

        reset = entry.get("reset_policy")
        _require(reset in RESET_POLICIES, f"{wid}: unknown reset policy")
        stages = entry.get("stages")
        _require(isinstance(stages, list) and stages and len(stages) == len(set(stages)),
                 f"{wid}: stages must be a nonempty unique list")
        _require(all(stage in STAGES for stage in stages), f"{wid}: unknown timing stage")
        if reset == "fresh_engine_per_prepare_sample":
            _require(stages == ["prepare"], f"{wid}: prepare-only reset policy requires prepare stage only")
        elif reset == "fresh_instance_for_cold_stages_reused_instance_for_warm_call":
            _require("warm_call" in stages, f"{wid}: reused warm instance requires warm_call stage")
        elif reset == "fresh_instance_per_invocation":
            _require("warm_call" not in stages,
                     f"{wid}: per-invocation reset conflicts with warm_call stage")

        comparators = entry.get("comparators")
        _require(isinstance(comparators, dict), f"{wid}: comparator capability metadata is required")
        for name, capability in comparators.items():
            _require(name in {"wasmtime"}, f"{wid}: unknown comparator {name!r}")
            _require(isinstance(capability, dict)
                     and capability.get("status") in {"probe_required", "supported", "unsupported"},
                     f"{wid}: invalid {name} capability status")
            status = capability["status"]
            if status == "unsupported":
                _require(isinstance(capability.get("reason"), str) and capability["reason"],
                         f"{wid}: unsupported comparator needs a reason")
            else:
                _require(isinstance(capability.get("feature_profile"), list),
                         f"{wid}: comparator feature_profile is required")
                _require(status != "supported" or capability.get("probe_id"),
                         f"{wid}: supported comparator requires a recorded probe_id")

    for name, selection in selections.items():
        _require(isinstance(name, str) and name, "selection names must be nonempty strings")
        _require(isinstance(selection, dict), f"selection {name}: must be an object")
        status = selection.get("status")
        ids = selection.get("workloads")
        _require(status in {"ready", "pending"}, f"selection {name}: invalid status")
        _require(isinstance(ids, list) and len(ids) == len(set(ids)),
                 f"selection {name}: workloads must be a unique list")
        _require(all(wid in by_id for wid in ids), f"selection {name}: unknown workload id")
        if status == "ready":
            _require(bool(ids), f"selection {name}: ready selection cannot be empty")
        else:
            _require(not ids, f"selection {name}: pending selection must not contain workloads")
            _require(isinstance(selection.get("owner_task"), str),
                     f"selection {name}: pending selection needs owner_task")
    return registry


def load_registry(path: str | Path = HERE / "workloads.json") -> dict[str, Any]:
    """Load and validate a version-2 JSON workload registry."""
    try:
        registry = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise WorkloadError(f"cannot load workload registry {path}: {exc}") from exc
    return validate_registry(registry)


def selection(name: str, registry: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    """Return explicitly selected entries; pending or empty selections cannot launch."""
    registry = load_registry() if registry is None else validate_registry(registry)
    try:
        selected = registry["selections"][name]
    except KeyError as exc:
        raise WorkloadError(f"unknown workload selection: {name}") from exc
    if selected["status"] == "pending" or not selected["workloads"]:
        owner = selected.get("owner_task", "a later workload task")
        raise EmptySelectionError(f"selection {name!r} is empty and pending {owner}")
    by_id = {entry["id"]: entry for entry in registry["workloads"]}
    return [by_id[wid] for wid in selected["workloads"]]


def generate_prepare_large() -> str:
    """Reproduce run.py's historical 1,000-function preparation fixture exactly."""
    functions = []
    for i in range(1000):
        export = '(export "run") ' if i == 0 else ""
        operations = " ".join(
            f"i32.const {1664525 + j + i} i32.mul i32.const {1013904223 + j} i32.xor"
            for j in range(16)
        )
        functions.append(f"(func {export}(param i32) (result i32) local.get 0 {operations})")
    return "(module\n" + "\n".join(functions) + "\n)"


def oracle_prepare_only() -> None:
    """The prepare-only case has no invocation result by definition."""
    return None


def encode_value(type_name: str, value: Any) -> dict[str, Any]:
    """Encode a typed value as JSON-safe data without losing float/vector bits."""
    if type_name in {"i32", "i64"}:
        _require(type(value) is int, f"{type_name} value must be an integer")
        bits = int(type_name[1:])
        _require(-(1 << (bits - 1)) <= value < (1 << (bits - 1)),
                 f"{type_name} value is out of range")
        return {"type": type_name, "value": value}
    if type_name in {"f32_bits", "f64_bits"}:
        _require(type(value) is int, f"{type_name} must be an unsigned bit-pattern integer")
        bits = int(type_name[1:3])
        _require(0 <= value < (1 << bits), f"{type_name} bit pattern is out of range")
        return {"type": type_name, "value": value}
    if type_name == "v128_bits":
        if isinstance(value, int) and type(value) is int:
            _require(0 <= value < (1 << 128), "v128_bits integer is out of range")
            raw = value.to_bytes(16, "little")
        elif isinstance(value, (bytes, bytearray, memoryview)):
            raw = bytes(value)
        elif isinstance(value, list):
            _require(len(value) == 16 and all(type(byte) is int and 0 <= byte <= 255 for byte in value),
                     "v128_bits byte list must contain exactly 16 bytes")
            raw = bytes(value)
        elif isinstance(value, str):
            _require(re.fullmatch(r"(?:0x)?[0-9a-fA-F]{32}", value) is not None,
                     "v128_bits must be 16 bytes of hexadecimal")
            raw = bytes.fromhex(value.removeprefix("0x"))
        else:
            raise WorkloadError("v128_bits must be bytes, a 16-byte list, hex, or a 128-bit integer")
        _require(len(raw) == 16, "v128_bits must contain exactly 16 bytes")
        return {"type": type_name, "value": raw.hex()}
    if type_name in {"reference", "ref"}:
        _require(isinstance(value, str) and value in REFERENCE_CATEGORIES,
                 "reference value must be a normalized reference category")
        return {"type": "reference", "value": value}
    raise WorkloadError(f"unsupported typed value: {type_name}")


def decode_value(encoded: dict[str, Any]) -> Any:
    """Decode an encoded typed value; v128 returns its little-endian bytes."""
    _require(isinstance(encoded, dict) and set(encoded) == {"type", "value"},
             "encoded value must contain exactly type and value")
    type_name = encoded["type"]
    value = encoded["value"]
    canonical = encode_value(type_name, value)
    if type_name == "v128_bits":
        return bytes.fromhex(canonical["value"])
    return canonical["value"]


def preflight(executable: Path, name: str, fixture_root: Path, output: Path) -> dict:
    """Bind actual same-byte comparator capability checks to a harness binary."""
    import hashlib
    import subprocess
    import run as original
    entries = selection(name)
    rows = []
    for entry in entries:
        wid = entry['id']
        path = fixture_root / (wid + '.wasm')
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        _require(digest == entry['source']['wasm_sha256'], f'{wid}: fixture identity mismatch')
        value = entry['params'][0]['baseline'] if entry['params'] else 0
        if wid == 'prepare_large':
            result = 0
            for j in range(16):
                result = ((result * (1664525 + j)) ^ (1013904223 + j)) & 0xffffffff
            expected = original.signed(result)
        else:
            expected = original.oracle(wid,value)
        command = [str(executable), '--capability', str(path), f'i32:{value}', f'i32:{expected}', ','.join(entry['required_features'])]
        process = subprocess.run(command, check=True, text=True, capture_output=True, timeout=180)
        capability = json.loads(process.stdout)
        _require(capability.get('status') in {'supported','unsupported'}, 'invalid capability response')
        _require(capability.get('status')!='supported' or capability.get('compile_execute_checked') is True,
                 'supported comparator requires executed compile/call oracle')
        _require(capability.get('status')!='unsupported' or capability.get('reason'), 'unsupported comparator needs reason')
        rows.append(dict(workload=wid,wasm_sha256=digest,command=command,capability=capability,
                         preparation_only=wid=='prepare_large',auxiliary_probe_expected=expected))
    report=dict(schema_version=1,selection=name,benchmark_binary_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                registry_sha256=hashlib.sha256((HERE/'workloads.json').read_bytes()).hexdigest(),probes=rows,
                supported=sum(r['capability']['status']=='supported' for r in rows),
                unsupported=sum(r['capability']['status']=='unsupported' for r in rows))
    output.parent.mkdir(parents=True,exist_ok=True)
    output.write_text(json.dumps(report,indent=2)+'\n')
    return report


if __name__ == '__main__':
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preflight',required=True,type=Path)
    parser.add_argument('--selection',required=True)
    parser.add_argument('--fixtures',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args()
    preflight(args.preflight.resolve(),args.selection,args.fixtures.resolve(),args.output)
