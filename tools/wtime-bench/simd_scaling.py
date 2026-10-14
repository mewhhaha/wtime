#!/usr/bin/env python3
"""Source-bound P010 SIMD measurement runner draft.

This runner records current candidate timings and per-case Wasmtime capability
results. It does not produce a frozen-baseline comparison or a speedup claim.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import random
import subprocess
import sys
from typing import Any


STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
SAMPLES = 5
MODE = "decoded"
MAX_MEMORY_SNAPSHOT_BYTES = 1 << 20


def batch_limit(entry):
    """Match the generic numeric and snapshot-memory Rust typed endpoints."""
    return 100_000 if isinstance(entry.get('memory_expectation'), dict) else 1_000_000


class MeasurementError(ValueError):
    pass


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise MeasurementError(f"cannot hash {path}: {exc}") from exc


def load_model(path: Path):
    spec = importlib.util.spec_from_file_location("p010_simd_workloads", path)
    if spec is None or spec.loader is None:
        raise MeasurementError(f"cannot load workload model {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    if not callable(getattr(module, "entries", None)) or not callable(getattr(module, "allowed_results", None)):
        raise MeasurementError("workload model must expose entries() and allowed_results()")
    return module


def _wire_value(kind: str, value: Any) -> str:
    if kind == "v128_bits":
        if not isinstance(value, str) or len(value) != 32:
            raise MeasurementError("v128_bits input must be 32 hexadecimal digits")
        try:
            bytes.fromhex(value)
        except ValueError as exc:
            raise MeasurementError("v128_bits input is not hexadecimal") from exc
        return f"v128_bits:{value.lower()}"
    if kind in ("i32", "i64", "f32_bits", "f64_bits"):
        if isinstance(value, bool) or not isinstance(value, int):
            raise MeasurementError(f"{kind} input must be an integer")
        return f"{kind}:{value}"
    raise MeasurementError(f"unsupported SIMD typed input kind {kind!r}")


def typed_input(entry: dict[str, Any]) -> str:
    params = entry.get("params")
    if not isinstance(params, list) or not params:
        raise MeasurementError(f"{entry.get('id')}: missing typed params")
    values = []
    for param in params:
        if not isinstance(param, dict) or not isinstance(param.get("type"), str):
            raise MeasurementError(f"{entry.get('id')}: malformed typed param")
        value = param.get("value", param.get("baseline"))
        if value is None:
            raise MeasurementError(f"{entry.get('id')}: param {param.get('name')} has no value")
        values.append(_wire_value(param["type"], value))
    return ";".join(values)


def allowed_wire_results(entry: dict[str, Any], model) -> list[str]:
    workload = entry.get("id")
    if not isinstance(workload, str):
        raise MeasurementError("workload entry has no string id")
    raw = entry.get("expected_results")
    if raw is None:
        if "expected_result" in entry:
            raw = [entry["expected_result"]]
        else:
            oracle_args = entry.get("oracle_arguments", {})
            value = next((oracle_args[key] for key in ("n", "case", "index") if key in oracle_args), None)
            raw = sorted(model.allowed_results(workload, value))
    if not isinstance(raw, (list, tuple, set)) or not raw:
        raise MeasurementError(f"{workload}: no expected result set")
    if any(isinstance(item, bool) or not isinstance(item, int) for item in raw):
        raise MeasurementError(f"{workload}: this runner expects i32 checksum results")
    return sorted({f"i32:{value}" for value in raw}, key=lambda item: int(item.split(":")[1]))


def execution_profile(entry: dict[str, Any]) -> str:
    features = set(entry.get("required_features", []))
    if "relaxed_simd" in features:
        return "simd-relaxed"
    # The simd profile intentionally enables the non-trapping float conversion
    # extension too; combined float_edges fixtures need both feature bits.
    if "simd" in features:
        return "simd"
    if "saturating_float_to_int" in features:
        return "mvp-saturating"
    return "mvp"


def fixture_for(entry: dict[str, Any], model_path: Path, encoded_dir: Path) -> tuple[Path, Path]:
    source = entry.get("source")
    if not isinstance(source, dict) or source.get("kind") != "wat_file":
        raise MeasurementError(f"{entry.get('id')}: source must be a WAT fixture")
    wat = model_path.parent / source.get("path", "")
    wasm = encoded_dir / (Path(source.get("path", "")).stem + ".wasm")
    if not wat.is_file() or not wasm.is_file():
        raise MeasurementError(f"{entry.get('id')}: WAT or encoded fixture is missing")
    if source.get("encoding_status") != "encoded":
        raise MeasurementError(f"{entry['id']}: fixture encoding is not marked current/encoded")
    if source.get("sha256") != sha256(wat):
        raise MeasurementError(f"{entry['id']}: WAT source hash differs from workload model")
    if source.get("wasm_sha256") != sha256(wasm):
        raise MeasurementError(f"{entry['id']}: encoded fixture hash differs from workload model")
    return wat, wasm


def _read_protocol_json(result: subprocess.CompletedProcess[str], label: str) -> dict[str, Any]:
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit status {result.returncode}"
        raise MeasurementError(f"{label} failed: {detail}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise MeasurementError(f"{label} did not return one JSON object: {exc}") from exc
    if not isinstance(value, dict):
        raise MeasurementError(f"{label} output must be a JSON object")
    return value


def _memory_proof(entry: dict[str, Any], output: dict[str, Any], stage: str) -> dict[str, Any]:
    expected = entry.get("memory_expectation")
    if not isinstance(expected, dict):
        raise MeasurementError(f"{entry.get('id')}: memory case has no independent memory oracle")
    hex_image = output.get("memory_hex")
    pages = output.get("memory_pages")
    page_log2 = output.get("memory_page_size_log2")
    wanted_pages = expected.get("page_count", 1)
    if isinstance(pages, bool) or not isinstance(pages, int) or pages != wanted_pages:
        raise MeasurementError(f"{entry['id']}: memory page count differs from oracle")
    if not isinstance(hex_image, str) or len(hex_image) % 2:
        raise MeasurementError(f"{entry['id']}: memory protocol omitted full bounded hex image")
    if isinstance(page_log2, bool) or not isinstance(page_log2, int) or page_log2 <= 0:
        raise MeasurementError(f"{entry['id']}: invalid memory_page_size_log2")
    if pages <= 0 or pages * (1 << page_log2) > MAX_MEMORY_SNAPSHOT_BYTES:
        raise MeasurementError(f"{entry['id']}: memory snapshot exceeds its byte bound")
    try:
        image = bytes.fromhex(hex_image)
    except ValueError as exc:
        raise MeasurementError(f"{entry['id']}: memory image is not hexadecimal") from exc
    if len(image) != pages * (1 << page_log2):
        raise MeasurementError(f"{entry['id']}: memory image length does not match its page count")
    if expected.get("page_size_bytes", 1 << page_log2) != 1 << page_log2:
        raise MeasurementError(f"{entry['id']}: memory page size differs from oracle")
    full_hash = hashlib.sha256(image).hexdigest()
    if full_hash != expected.get("full_memory_sha256"):
        raise MeasurementError(f"{entry['id']}: full memory state differs from oracle")
    region = expected.get("range")
    expected_range_sha256 = expected.get("sha256")
    if (not isinstance(region, list) or len(region) != 2 or
            any(isinstance(v, bool) or not isinstance(v, int) for v in region) or
            region[0] < 0 or region[1] < region[0] or not isinstance(expected_range_sha256, str)):
        raise MeasurementError(f"{entry['id']}: malformed memory range oracle")
    touched = image[region[0]:region[1]]
    if hashlib.sha256(touched).hexdigest() != expected_range_sha256:
        raise MeasurementError(f"{entry['id']}: memory range differs from oracle")
    # Keep raw 64 KiB images out of the report while retaining a verifiable proof.
    return {"memory_pages": pages, "memory_page_size_log2": page_log2,
            "memory_full_sha256": full_hash, "memory_range": region,
            "memory_range_sha256": hashlib.sha256(touched).hexdigest(),
            "memory_snapshot_phase": "preflight" if stage == "prepare" else "post_invoke"}


def _memory_oracle_record(entry: dict[str, Any]) -> dict[str, Any] | None:
    expected = entry.get("memory_expectation")
    if not isinstance(expected, dict):
        return None
    return {key: expected[key] for key in (
        "memory_index", "export", "range", "sha256", "full_memory_sha256",
        "page_count", "page_size_bytes", "setup_policy", "result"
    ) if key in expected}


def _assert_stable_sources(before: dict[str, str], after: dict[str, str]) -> None:
    if before != after:
        changed = sorted(path for path in before.keys() & after.keys() if before[path] != after[path])
        missing = sorted(before.keys() - after.keys())
        added = sorted(after.keys() - before.keys())
        raise MeasurementError(f"source/binary identity changed during P010 measurement (changed={changed[:3]}, missing={missing[:3]}, added={added[:3]})")


def _call_candidate(executable: Path, wasm: Path, entry: dict[str, Any], args: str,
                   allowed: list[str], stage: str, profile: str,
                   timeout: float) -> dict[str, Any]:
    is_memory = isinstance(entry.get("memory_expectation"), dict)
    multi = len(allowed) > 1
    mode = "--simd-memory-case" if is_memory else "--typed-allowed-case" if multi else "--typed-case"
    expected = "|".join(allowed) if multi else allowed[0]
    batch = 0 if stage == "warm_call" else 1
    command = [str(executable), mode, str(wasm), entry["id"], args, expected,
               stage, str(batch), MODE, profile]
    try:
        proc = subprocess.run(command, capture_output=True, text=True, check=False,
                              timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise MeasurementError(f"candidate invocation failed for {entry['id']}/{stage}: {exc}") from exc
    row = _read_protocol_json(proc, f"candidate {entry['id']}/{stage}")
    if row.get("schema_version") != 2 or row.get("case") != entry["id"] or row.get("stage") != stage:
        raise MeasurementError(f"{entry['id']}/{stage}: protocol identity mismatch")
    if row.get("input") != args or row.get("expected") != expected or row.get("mode") != MODE:
        raise MeasurementError(f"{entry['id']}/{stage}: protocol input/settings mismatch")
    if row.get("profile") != profile:
        raise MeasurementError(f"{entry['id']}/{stage}: profile mismatch")
    result = row.get("result")
    if result not in allowed:
        raise MeasurementError(f"{entry['id']}/{stage}: result {result!r} is outside independent oracle set {allowed}")
    ns = row.get("ns_per_operation")
    actual_batch = row.get("batch")
    if isinstance(ns, bool) or not isinstance(ns, (int, float)) or not math.isfinite(ns) or ns <= 0:
        raise MeasurementError(f"{entry['id']}/{stage}: timing must be finite and positive")
    if (isinstance(actual_batch, bool) or not isinstance(actual_batch, int)
            or not 1 <= actual_batch <= batch_limit(entry) or (stage != "warm_call" and actual_batch != 1)):
        raise MeasurementError(f"{entry['id']}/{stage}: batch must be a positive integer")
    result_row = {"ns_per_operation": float(ns), "batch": actual_batch, "result": result}
    if is_memory:
        result_row.update(_memory_proof(entry, row, stage))
    return result_row


def _capability(wasmtime: Path, wasm: Path, entry: dict[str, Any], args: str,
                allowed: list[str], timeout: float) -> dict[str, Any]:
    features = entry.get("required_features", [])
    if not isinstance(features, list) or any(not isinstance(f, str) for f in features):
        raise MeasurementError(f"{entry['id']}: required_features must be strings")
    prefix = "--capability-allowed" if len(allowed) > 1 else "--capability"
    command = [str(wasmtime), prefix, str(wasm), args, "|".join(allowed),
               ",".join(features)]
    try:
        proc = subprocess.run(command, capture_output=True, text=True, check=False,
                              timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise MeasurementError(f"Wasmtime capability invocation failed for {entry['id']}: {exc}") from exc
    row = _read_protocol_json(proc, f"Wasmtime capability {entry['id']}")
    if row.get("schema_version") != 2 or row.get("comparator") != "wasmtime-49.0.2":
        raise MeasurementError(f"{entry['id']}: Wasmtime capability identity mismatch")
    if row.get("status") not in ("supported", "unsupported"):
        raise MeasurementError(f"{entry['id']}: invalid Wasmtime capability status")
    if row.get("status") == "supported" and row.get("compile_execute_checked") is not True:
        raise MeasurementError(f"{entry['id']}: Wasmtime support lacks execution evidence")
    return row


def _selected_entries(model, registry_path: Path, selection: str) -> list[dict[str, Any]]:
    import workloads as schema
    try:
        registry = schema.load_registry(registry_path)
        schema.selection(selection, registry)
    except schema.WorkloadError as exc:
        raise MeasurementError(f"cannot select registered workloads: {exc}") from exc
    workloads = registry.get("selections", {}).get(selection, {}).get("workloads")
    if not isinstance(workloads, list) or not workloads or any(not isinstance(x, str) for x in workloads):
        raise MeasurementError(f"registry has no valid {selection!r} selection")
    if len(workloads) != len(set(workloads)):
        raise MeasurementError("registry selection contains duplicate ids")
    entries = model.entries()
    by_id = {item.get("id"): item for item in entries if isinstance(item, dict)}
    if len(by_id) != len(entries):
        raise MeasurementError("workload model contains missing or duplicate ids")
    missing = set(workloads) - set(by_id)
    extra = set(by_id) - set(workloads)
    if missing or extra:
        raise MeasurementError(f"registry/model workload mismatch (missing={sorted(missing)[:4]}, extra={sorted(extra)[:4]})")
    records = registry.get("workloads", [])
    registered = {row.get("id"): row for row in records if isinstance(row, dict)}
    if len(registered) != len(records) or any(registered.get(key) != value for key, value in by_id.items()):
        raise MeasurementError("registry records differ from the independent workload model")
    return [by_id[workload] for workload in workloads]


def _source_snapshot(paths: list[Path]) -> dict[str, str]:
    return {str(path.resolve()): sha256(path) for path in sorted(set(paths), key=lambda p: str(p.resolve()))}


def _repository_source_inputs(source_root: Path) -> list[Path]:
    """Return tracked build inputs that can change benchmark/runtime behavior."""
    root = source_root.resolve()
    fixed = [root / "Cargo.toml", root / "Cargo.lock",
             root / "tools" / "wtime-bench" / "Cargo.toml",
             root / "tools" / "wtime-bench" / "Cargo.lock"]
    for optional in (root / "rust-toolchain", root / "rust-toolchain.toml",
                     root / ".cargo" / "config.toml", root / ".cargo" / "config"):
        if optional.is_file():
            fixed.append(optional)
    required = [path for path in fixed[:4] if path.is_file()]
    if len(required) != 4:
        missing = [str(path) for path in fixed[:4] if not path.is_file()]
        raise MeasurementError(f"incomplete Rust build identity inputs: {missing}")
    rust_files = []
    for subtree in (root / "crates", root / "tools" / "wtime-bench"):
        if not subtree.is_dir():
            raise MeasurementError(f"missing Rust source tree: {subtree}")
        rust_files.extend(path for path in subtree.rglob("*.rs")
                          if "target" not in path.parts and ".git" not in path.parts)
    if not rust_files:
        raise MeasurementError("no runtime or benchmark Rust sources found for source binding")
    return sorted(set(required + [p for p in fixed[4:] if p.is_file()] + rust_files))


def _metadata(binary: Path, wasmtime: Path, encoder: Path, model_path: Path,
              registry: Path, wasm_paths: list[Path], wat_paths: list[Path],
              source_inputs: list[Path], identity: dict[str, str], seed: int,
              samples: int) -> dict[str, Any]:
    digest = lambda path: identity[str(path.resolve())]
    return {
        "candidate_binary": str(binary.resolve()),
        "candidate_binary_sha256": digest(binary),
        "fixture_encoder_binary": str(encoder.resolve()),
        "fixture_encoder_binary_sha256": digest(encoder),
        "wasmtime_capability_binary": str(wasmtime.resolve()),
        "wasmtime_capability_binary_sha256": digest(wasmtime),
        "candidate_sources": {
            str(model_path.resolve()): digest(model_path),
            str(registry.resolve()): digest(registry),
        },
        "fixture_wat_sha256": {str(path.resolve()): digest(path) for path in sorted(set(wat_paths))},
        "fixture_wasm_sha256": {str(path.resolve()): digest(path) for path in sorted(set(wasm_paths))},
        "runtime_and_benchmark_sources": {
            str(path.resolve()): digest(path) for path in source_inputs
        },
        "source_snapshot": identity,
        "seed": seed,
        "samples_per_stage": samples,
        "stages": list(STAGES),
        "execution_mode": MODE,
        "instrumented": False,
        "host": {"platform": platform.platform(), "python": platform.python_version(),
                 "cpu_count": os.cpu_count(), "affinity": sorted(os.sched_getaffinity(0))
                 if hasattr(os, "sched_getaffinity") else None},
    }


def measure(*, candidate: Path, wasmtime: Path, model_path: Path, registry_path: Path,
            encoded_dir: Path, selection: str, source_root: Path | None = None,
            encoder: Path | None = None, samples: int = SAMPLES,
            seed: int = 0x5010, timeout: float = 120.0) -> dict[str, Any]:
    if samples < 5:
        raise MeasurementError("P010 integrity run requires at least five samples per stage")
    if not candidate.is_file() or not wasmtime.is_file():
        raise MeasurementError("candidate and Wasmtime executables must exist")
    if encoder is None:
        raise MeasurementError("the actual fixture encoder identity must be supplied")
    if not encoder.is_file():
        raise MeasurementError("fixture encoder executable must exist")
    source_root = source_root or registry_path.resolve().parents[2]
    source_inputs = _repository_source_inputs(source_root)
    model = load_model(model_path)
    entries = _selected_entries(model, registry_path, selection)
    fixtures = {entry["id"]: fixture_for(entry, model_path, encoded_dir) for entry in entries}
    wat_paths = [pair[0] for pair in fixtures.values()]
    wasm_paths = [pair[1] for pair in fixtures.values()]
    source_paths = [candidate, wasmtime, encoder, model_path, registry_path, Path(__file__).resolve(),
                    Path(__file__).resolve().with_name("workloads.py"),
                    *source_inputs, *wat_paths, *wasm_paths]
    before = _source_snapshot(source_paths)
    metadata = _metadata(candidate, wasmtime, encoder, model_path, registry_path,
                         wasm_paths, wat_paths, source_inputs, before, seed, samples)
    protocol = _read_protocol_json(subprocess.run(
        [str(candidate), "--protocol"], capture_output=True, text=True, timeout=timeout),
        "candidate capability handshake")
    profiles = {execution_profile(entry) for entry in entries}
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
            or protocol.get("instrumented") is not False
            or MODE not in protocol.get("modes", [])
            or not profiles.issubset(set(protocol.get("features", [])))):
        raise MeasurementError("candidate protocol does not prove uninstrumented mode/profile support")
    metadata["candidate_protocol"] = protocol
    rng = random.Random(seed)
    rng.shuffle(entries)
    cases = []
    for entry in entries:
        workload = entry["id"]
        wat, wasm = fixtures[workload]
        args = typed_input(entry)
        allowed = allowed_wire_results(entry, model)
        profile = execution_profile(entry)
        capability = _capability(wasmtime, wasm, entry, args, allowed, timeout)
        case = {
            "workload": workload,
            "input": args,
            "fixture": {"wat_sha256": sha256(wat), "wasm_sha256": sha256(wasm)},
            "required_features": entry.get("required_features", []),
            "profile": profile,
            "allowed_results": allowed,
            "memory_oracle": _memory_oracle_record(entry),
            "wasmtime_capability": capability,
            "stages": {},
        }
        for stage in STAGES:
            pairs = []
            for sample in range(samples):
                result = _call_candidate(candidate, wasm, entry, args, allowed, stage, profile, timeout)
                pairs.append({"sample": sample, "order": ["candidate"], "candidate": result})
            case["stages"][stage] = pairs
        cases.append(case)
    after = _source_snapshot(source_paths)
    _assert_stable_sources(before, after)
    return {"schema_version": 1, "task": "P010", "selection": selection,
            "passed": True, "source_stable": True, "metadata": metadata,
            "case_execution_order": [case["workload"] for case in cases], "cases": cases}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--wasmtime", type=Path, required=True,
                        help="typed Wasmtime capability-probe executable")
    parser.add_argument("--model", type=Path, required=True,
                        help="independent P010 Python workload model")
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--encoded-dir", type=Path, required=True)
    parser.add_argument("--source-root", type=Path,
                        help="repository root containing runtime and benchmark Rust build inputs")
    parser.add_argument("--encoder", type=Path, required=True,
                        help="actual fixture encoder binary identity")
    parser.add_argument("--selection", default="simd")
    parser.add_argument("--samples", type=int, default=SAMPLES)
    parser.add_argument("--seed", type=int, default=0x5010)
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = measure(candidate=args.candidate, wasmtime=args.wasmtime, model_path=args.model,
                         registry_path=args.registry, encoded_dir=args.encoded_dir,
                         selection=args.selection, source_root=args.source_root,
                         encoder=args.encoder, samples=args.samples, seed=args.seed,
                         timeout=args.timeout)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix + ".tmp")
        temporary.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n")
        temporary.replace(args.output)
    except (OSError, MeasurementError, TypeError, ValueError, KeyError) as exc:
        print(f"p010_scaling.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "cases": len(report["cases"]),
                      "source_stable": report["source_stable"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
