#!/usr/bin/env python3
"""Source-bound P015 integrity runner for baseline, candidate, and Wasmtime.

Five fresh-process observations are collected for each of nine frozen inputs.
The result is integrity evidence only: this runner makes no speedup claim.
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
import subprocess
import sys
import tempfile
from typing import Any

STAGE = "run_after_setup"
PROFILE = "mvp"
MAX_MEMORY_BYTES = 1 << 20
MIN_SAMPLES = 5


class RepresentativeError(ValueError):
    pass


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise RepresentativeError(f"cannot hash {path}: {exc}") from exc


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RepresentativeError(f"cannot load Python module {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def load_model(path: Path):
    model = load_module(path, "p015_representative_model")
    for attr in ("entries", "oracle", "expected_memory_image", "provenance", "encode_fixtures"):
        if not callable(getattr(model, attr, None)):
            raise RepresentativeError(f"model must expose {attr}()")
    return model


def source_snapshot(paths: list[Path]) -> dict[str, str]:
    unique = {path.resolve() for path in paths}
    missing = sorted(str(path) for path in unique if not path.is_file())
    if missing:
        raise RepresentativeError(f"required source/binary identity files are missing: {missing[:8]}")
    return {str(path): sha256(path) for path in sorted(unique, key=str)}


def assert_source_stable(before: dict[str, str], after: dict[str, str]) -> None:
    if before != after:
        changed = sorted(path for path in before.keys() & after.keys() if before[path] != after[path])
        missing = sorted(before.keys() - after.keys())
        added = sorted(after.keys() - before.keys())
        raise RepresentativeError(
            f"source/binary identity changed during run (changed={changed[:6]}, "
            f"missing={missing[:6]}, added={added[:6]})"
        )


def _selected_entries(schema_path: Path, registry_path: Path, model_entries: list[dict[str, Any]],
                      name: str = "representative") -> list[dict[str, Any]]:
    schema = load_module(schema_path, "p015_workload_schema")
    try:
        registry = schema.load_registry(registry_path)
        selected = schema.selection(name, registry)
    except Exception as exc:
        raise RepresentativeError(f"registry/schema validation failed: {exc}") from exc
    if len(model_entries) != 9 or len(selected) != 9:
        raise RepresentativeError("P015 integrity run requires all nine frozen inputs")
    model_by_id = {row.get("id"): row for row in model_entries}
    selected_by_id = {row.get("id"): row for row in selected}
    if len(model_by_id) != 9 or len(selected_by_id) != 9 or set(model_by_id) != set(selected_by_id):
        raise RepresentativeError("registry selection must match the complete nine-entry model")
    if registry["selections"][name]["status"] != "ready":
        raise RepresentativeError("representative selection is not ready")
    for workload, modeled in model_by_id.items():
        if selected_by_id[workload] != modeled:
            raise RepresentativeError(f"{workload}: complete registry row differs from model")
    return selected


def validate_fixtures(entries: list[dict[str, Any]], model_path: Path, encoded_dir: Path,
                      encoder: Path, model) -> tuple[dict[str, tuple[Path, Path]], dict[str, str]]:
    fixtures: dict[str, tuple[Path, Path]] = {}
    expected_encoded: dict[str, str] = {}
    for entry in entries:
        source = entry.get("source")
        if not isinstance(source, dict) or source.get("kind") != "wat_file":
            raise RepresentativeError(f"{entry.get('id')}: fixture must be a WAT file")
        relative = source.get("path")
        if (not isinstance(relative, str) or Path(relative).is_absolute() or
                ".." in Path(relative).parts):
            raise RepresentativeError(f"{entry.get('id')}: unsafe or missing fixture path")
        wat = model_path.parent / relative
        wasm = encoded_dir / (Path(relative).stem + ".wasm")
        if not wat.is_file() or not wasm.is_file():
            raise RepresentativeError(f"{entry.get('id')}: WAT or Wasm artifact missing")
        if sha256(wat) != source.get("sha256"):
            raise RepresentativeError(f"{entry['id']}: WAT source hash mismatch")
        expected_hash = source.get("wasm_sha256")
        if (not isinstance(expected_hash, str) or len(expected_hash) != 64 or
                source.get("encoding_status") != "encoded" or sha256(wasm) != expected_hash):
            raise RepresentativeError(f"{entry['id']}: Wasm hash/status mismatch")
        if expected_hash != expected_hash.lower() or any(c not in "0123456789abcdef" for c in expected_hash):
            raise RepresentativeError(f"{entry['id']}: invalid Wasm SHA-256 encoding")
        fixtures[entry["id"]] = (wat, wasm)
        expected_encoded[Path(relative).name] = expected_hash

    pinned = getattr(model, "ENCODER_SHA256", None)
    provenance = model.provenance()
    if (not isinstance(pinned, str) or len(pinned) != 64 or sha256(encoder) != pinned or
            any(record.get("encoder_sha256") != pinned
                for record in provenance.get("kernels", {}).values())):
        raise RepresentativeError("encoder binary does not match the model's pinned provenance")
    # Encoding is a setup/identity check; it occurs outside every guest timer.
    with tempfile.TemporaryDirectory(prefix="p015-representative-encode-") as temp:
        try:
            reencoded = model.encode_fixtures(encoder, temp)
        except Exception as exc:
            raise RepresentativeError(f"pinned fixture re-encoding failed: {exc}") from exc
    if reencoded != expected_encoded:
        raise RepresentativeError("re-encoded fixture hashes differ from all nine frozen registry entries")
    return fixtures, reencoded


def _run(executable: Path, args: list[str], timeout: float) -> subprocess.CompletedProcess[str]:
    try:
        affinity = None
        if hasattr(os, "sched_getaffinity"):
            allowed = os.sched_getaffinity(0)
            if allowed:
                affinity = min(allowed)
        return subprocess.run([str(executable), *args], text=True, capture_output=True,
                              timeout=timeout, check=False,
                              preexec_fn=(lambda: os.sched_setaffinity(0, {affinity})
                                          if affinity is not None else None))
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RepresentativeError(f"{executable.name} invocation failed: {exc}") from exc


def _parse_object(stdout: str, context: str) -> dict[str, Any]:
    try:
        row = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise RepresentativeError(f"{context}: stdout is not one JSON object") from exc
    if not isinstance(row, dict):
        raise RepresentativeError(f"{context}: JSON root must be an object")
    return row


def _protocol(executable: Path, label: str, timeout: float, *, require_decoded: bool) -> dict[str, Any]:
    proc = _run(executable, ["--protocol"], timeout)
    if proc.returncode != 0:
        raise RepresentativeError(f"{label} --protocol failed: {proc.stderr.strip()}")
    row = _parse_object(proc.stdout, f"{label} --protocol")
    if row.get("instrumented") is not False:
        raise RepresentativeError(f"{label} protocol does not prove counters are disabled")
    if row.get("schema_version") != 2:
        raise RepresentativeError(f"{label} protocol schema version is unsupported")
    if require_decoded:
        if row.get("engine") != "wtime" or not isinstance(row.get("modes"), list) or "decoded" not in row["modes"]:
            raise RepresentativeError(f"{label} protocol does not advertise decoded execution")
    return row


def _compile_unsupported(row: dict[str, Any]) -> bool:
    """Accept unsupported only when the driver explicitly identifies compile failure."""
    return (row.get("status") == "unsupported" and
            row.get("failure_stage") == "compile" and
            isinstance(row.get("reason"), str) and bool(row["reason"].strip()))


def _validate_observation(row: dict[str, Any], entry: dict[str, Any], model,
                          mode: str, profile: str) -> dict[str, Any]:
    size = entry["params"][0]["baseline"]
    expected = model.oracle(entry["id"], size)
    if (entry.get("expected_result") != expected["result"] or
            entry.get("expected_output_sha256") != expected["output_sha256"]):
        raise RepresentativeError(f"{entry['id']}: registry scalar/digest differs from independent oracle")
    setup = entry.get("setup_policy")
    if (not isinstance(setup, dict) or setup.get("timed") is not False or
            setup.get("frequency") != "before_each_timed_run"):
        raise RepresentativeError(f"{entry['id']}: setup policy must be reset before and excluded from timing")
    identity = {
        "schema_version": 2, "case": entry["id"], "stage": STAGE,
        "input": str(size), "expected": f"i32:{expected['result']}",
        "result": f"i32:{expected['result']}", "mode": mode, "profile": profile,
        "expected_output_sha256": expected["output_sha256"],
    }
    for key, value in identity.items():
        if row.get(key) != value:
            raise RepresentativeError(f"{entry['id']}: driver {key}={row.get(key)!r}, expected {value!r}")
    elapsed = row.get("ns_per_operation")
    if (isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or
            not math.isfinite(elapsed) or elapsed <= 0):
        raise RepresentativeError(f"{entry['id']}: timing must be finite and positive")
    if row.get("batch") != 1:
        raise RepresentativeError(f"{entry['id']}: setup-aware representative timing requires batch=1")

    pages, page_log2, image_hex = (row.get("memory_pages"), row.get("memory_page_size_log2"),
                                   row.get("memory_hex"))
    expected_pages = entry.get("memory_page_count")
    if (type(pages) is not int or pages != expected_pages or page_log2 != 16 or
            not isinstance(image_hex, str)):
        raise RepresentativeError(f"{entry['id']}: memory dimensions/readback differ from frozen contract")
    try:
        image = bytes.fromhex(image_hex)
    except ValueError as exc:
        raise RepresentativeError(f"{entry['id']}: malformed full memory hex") from exc
    if len(image) != pages * (1 << page_log2) or len(image) > MAX_MEMORY_BYTES:
        raise RepresentativeError(f"{entry['id']}: full-memory image has invalid bounded length")
    expected_image = model.expected_memory_image(entry["id"], size)
    if not isinstance(expected_image, bytes) or len(expected_image) != len(image):
        raise RepresentativeError(f"{entry['id']}: independent full-memory oracle has invalid length")
    expected_memory_hash = hashlib.sha256(expected_image).hexdigest()
    if entry.get("expected_full_memory_sha256") != expected_memory_hash:
        raise RepresentativeError(f"{entry['id']}: registry full-memory digest differs from model")
    if image != expected_image:
        raise RepresentativeError(f"{entry['id']}: full memory image differs from independent model")
    if hashlib.sha256(image).hexdigest() != entry["expected_full_memory_sha256"]:
        raise RepresentativeError(f"{entry['id']}: full-memory digest mismatch")

    output_range = entry.get("output_range")
    if (not isinstance(output_range, list) or len(output_range) != 2 or
            any(type(value) is not int for value in output_range)):
        raise RepresentativeError(f"{entry['id']}: output range is malformed")
    start, end = output_range
    if start < 0 or end < start or end > len(image) or end - start != entry.get("output_bytes"):
        raise RepresentativeError(f"{entry['id']}: output range is outside memory/incorrect length")
    output_hash = hashlib.sha256(image[start:end]).hexdigest()
    if output_hash != expected["output_sha256"]:
        raise RepresentativeError(f"{entry['id']}: output range digest differs from independent oracle")
    return {
        "stage": STAGE, "ns_per_operation": float(elapsed), "batch": 1,
        "result": expected["result"], "output_sha256": output_hash,
        "full_memory_sha256": expected_memory_hash,
        "memory_pages": pages, "memory_page_size_log2": page_log2,
        "setup_excluded_from_timing": True,
    }


def _invoke(executable: Path, label: str, wasm: Path, entry: dict[str, Any], model,
            timeout: float, *, wasmtime: bool) -> dict[str, Any]:
    size = entry["params"][0]["baseline"]
    oracle = model.oracle(entry["id"], size)
    mode = "wasmtime" if wasmtime else "decoded"
    args = (["--representative-wasmtime-case"] if wasmtime else ["--representative-case"]) + [
        str(wasm), entry["id"], str(size), f"i32:{oracle['result']}",
        oracle["output_sha256"], STAGE, mode, PROFILE,
    ]
    proc = _run(executable, args, timeout)
    row = _parse_object(proc.stdout, f"{label}/{entry['id']}")
    if wasmtime and _compile_unsupported(row):
        return {"status": "unsupported", "failure_stage": "compile", "reason": row["reason"],
                "returncode": proc.returncode, "stderr": proc.stderr.strip(), "raw": row}
    if proc.returncode != 0:
        raise RepresentativeError(f"{label}/{entry['id']} failed: {proc.stderr.strip()}")
    if row.get("status") == "unsupported":
        raise RepresentativeError(f"{label}/{entry['id']}: unsupported status is not a compilation failure")
    validated = _validate_observation(row, entry, model, mode, PROFILE)
    return {"status": "ok", **validated, "raw": row}


def _source_paths(source_root: Path, bench_root: Path, executables: list[Path], encoder: Path,
                  schema: Path, registry: Path, model_path: Path, entries: list[dict[str, Any]],
                  runner: Path) -> list[Path]:
    paths = [*executables, encoder, schema, registry, model_path, runner,
             source_root / "Cargo.toml", source_root / "Cargo.lock",
             source_root / "rust-toolchain.toml"]
    for base in (source_root / "crates", source_root / "tools"):
        for child in base.iterdir():
            if child.is_dir():
                for name in ("Cargo.toml", "Cargo.lock"):
                    candidate = child / name
                    if candidate.is_file(): paths.append(candidate)
    paths.extend((source_root / "crates").rglob("*.rs"))
    paths.extend((bench_root / "src").rglob("*.rs"))
    paths.extend((source_root / "tools" / "wtime-conformance" / "src").rglob("*.rs"))
    paths.extend(p for p in (source_root / ".cargo").rglob("*") if p.is_file())
    for entry in entries:
        wat = model_path.parent / entry["source"]["path"]
        paths.append(wat)
    return paths


def run(*, baseline: Path, candidate: Path, wasmtime: Path, encoder: Path, source_root: Path,
        registry: Path, model_path: Path, encoded_dir: Path, output: Path,
        samples: int = MIN_SAMPLES, timeout: float = 180.0) -> dict[str, Any]:
    if samples < MIN_SAMPLES:
        raise RepresentativeError("P015 integrity evidence requires at least five samples per engine/input")
    if output.exists():
        raise RepresentativeError(f"output already exists; refusing to overwrite: {output}")
    for path in (baseline, candidate, wasmtime, encoder, source_root, registry, model_path, encoded_dir):
        if not path.exists():
            raise RepresentativeError(f"required P015 input does not exist: {path}")
    model = load_model(model_path)
    entries = model.entries()
    bench_root = source_root / "tools" / "wtime-bench"
    schema_path = bench_root / "workloads.py"
    entries = _selected_entries(schema_path, registry, entries)
    fixtures, encoded_hashes = validate_fixtures(entries, model_path, encoded_dir, encoder, model)
    paths = _source_paths(source_root, bench_root, [baseline, candidate, wasmtime], encoder,
                          schema_path, registry, model_path, entries, Path(__file__))
    for wat, wasm in fixtures.values():
        paths.extend((wat, wasm))
    before = source_snapshot(paths)
    protocols = {
        "baseline": _protocol(baseline, "baseline", timeout, require_decoded=True),
        "candidate": _protocol(candidate, "candidate", timeout, require_decoded=True),
        "wasmtime": _protocol(wasmtime, "wasmtime", timeout, require_decoded=False),
    }

    results_by_workload: dict[str, dict[str, Any]] = {}
    try:
        # Interleave engine observations within each input/sample round to avoid
        # silently biasing the evidence by doing one whole engine last.
        for entry in entries:
            workload = entry["id"]
            wasm = fixtures[workload][1]
            observations: dict[str, list[dict[str, Any]]] = {"baseline": [], "candidate": [], "wasmtime": []}
            wasmtime_unsupported = None
            measurement_order: list[list[str]] = []
            for _sample in range(samples):
                round_order = []
                observations["baseline"].append(_invoke(baseline, "baseline", wasm, entry, model,
                                                        timeout, wasmtime=False))
                round_order.append("baseline")
                observations["candidate"].append(_invoke(candidate, "candidate", wasm, entry, model,
                                                         timeout, wasmtime=False))
                round_order.append("candidate")
                if wasmtime_unsupported is None:
                    outcome = _invoke(wasmtime, "wasmtime", wasm, entry, model, timeout, wasmtime=True)
                    round_order.append("wasmtime")
                    if outcome["status"] == "unsupported":
                        if observations["wasmtime"]:
                            raise RepresentativeError(
                                f"{workload}: Wasmtime compiled earlier samples but then reported compilation unsupported"
                            )
                        wasmtime_unsupported = outcome
                    else:
                        observations["wasmtime"].append(outcome)
                measurement_order.append(round_order)
            results_by_workload[workload] = {
                "workload": workload, "family": entry["family"],
                "size": entry["params"][0]["baseline"],
                "validation_role": entry["validation_role"],
                "fixture_wat_sha256": entry["source"]["sha256"],
                "fixture_wasm_sha256": entry["source"]["wasm_sha256"],
                "expected_result": entry["expected_result"],
                "expected_output_sha256": entry["expected_output_sha256"],
                "expected_full_memory_sha256": entry["expected_full_memory_sha256"],
                "observations": observations,
                "measurement_order": measurement_order,
                "wasmtime_capability": wasmtime_unsupported or {"status": "supported", "samples": samples},
            }
    finally:
        after = source_snapshot(paths)
        assert_source_stable(before, after)

    return {
        "schema_version": 1, "task": "P015", "selection": "representative",
        "passed": True, "source_stable": True,
        "evidence_kind": "integrity_only",
        "claim": "No speedup, ratio, or performance promotion claim; five samples are integrity observations, not calibrated comparison evidence.",
        "settings": {"stage": STAGE, "baseline_mode": "decoded", "candidate_mode": "decoded",
                     "wasmtime_mode": "wasmtime", "profile": PROFILE,
                     "samples_per_input_per_engine": samples, "instrumented": False,
                     "timing_scope": "run after setup; setup, compile, instantiate, and full-memory readback are outside timing"},
        "protocols": protocols,
        "engine_artifacts": {
            label: {"path": str(path.resolve()), "sha256": before[str(path.resolve())]}
            for label, path in (("baseline", baseline), ("candidate", candidate),
                                ("wasmtime", wasmtime), ("encoder", encoder))
        },
        "source_snapshot": before,
        "fixture_provenance": model.provenance(),
        "reencoded_fixture_wasm_sha256": encoded_hashes,
        "hardware": {"platform": platform.platform(), "python": platform.python_version(),
                     "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None},
        "cases": list(results_by_workload.values()),
    }


def _write_fresh(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n"
    try:
        with path.open("x", encoding="utf-8") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
    except FileExistsError as exc:
        raise RepresentativeError(f"output already exists; refusing to overwrite: {path}") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--wasmtime", type=Path, required=True)
    parser.add_argument("--encoder", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--encoded-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=MIN_SAMPLES)
    parser.add_argument("--timeout", type=float, default=180.0)
    args = parser.parse_args(argv)
    try:
        report = run(baseline=args.baseline, candidate=args.candidate, wasmtime=args.wasmtime,
                     encoder=args.encoder, source_root=args.source_root, registry=args.registry,
                     model_path=args.model, encoded_dir=args.encoded_dir, output=args.output,
                     samples=args.samples, timeout=args.timeout)
        _write_fresh(args.output, report)
    except (OSError, RepresentativeError, KeyError, TypeError, ValueError,
            subprocess.TimeoutExpired) as exc:
        print(f"representative_scaling.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "cases": len(report["cases"]),
                      "source_stable": report["source_stable"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
