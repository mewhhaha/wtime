#!/usr/bin/env python3
"""Collect source-bound P009 memory observations from an existing driver.

The runner does not build. Every invocation uses a fresh guest instance and
is accepted only after its scalar outcome and full host-memory image match the
independent Python workload model.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
from typing import Any


DEFAULT_TOOL_DIR = Path(__file__).resolve().parent
PAGE = 65536
MODE = "decoded"
ALL_PHASE3 = "all-phase3-core"
MVP = "mvp"


class BaselineError(RuntimeError):
    pass


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _repo_root(tool_dir: Path) -> Path:
    for candidate in tool_dir.resolve().parents:
        if (candidate / "crates/wtime-runtime/Cargo.toml").is_file():
            return candidate
    raise BaselineError(f"cannot locate repository root above tool directory {tool_dir}")


def _load_registry(tool_dir: Path):
    path = tool_dir / "memory_workloads.py"
    spec = importlib.util.spec_from_file_location("p009_memory_registry", path)
    if spec is None or spec.loader is None:
        raise BaselineError(f"cannot load memory workload registry: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _source_paths(root: Path, tool_dir: Path) -> list[Path]:
    paths = [root / "Cargo.toml", root / "Cargo.lock",
             tool_dir / "Cargo.toml", tool_dir / "Cargo.lock",
             tool_dir / "memory_workloads.py", tool_dir / "workloads.py",
             tool_dir / "workloads.json"]
    for area in (root / "crates", tool_dir / "src",
                 root / "tools/wtime-conformance/src"):
        if area.exists():
            paths.extend(area.rglob("*.rs"))
    for manifest in (root / "tools/wtime-conformance/Cargo.toml",
                     root / "tools/wtime-conformance/build.rs"):
        paths.append(manifest)
    paths.extend((tool_dir / "fixtures/memory").glob("*.wat"))
    paths.append(Path(__file__).resolve())
    return sorted({p.resolve() for p in paths if p.is_file()})


def source_hashes(root: Path, tool_dir: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for path in _source_paths(root, tool_dir):
        try:
            key = path.relative_to(root).as_posix()
        except ValueError:
            key = f"runner:{path}"
        result[key] = sha(path)
    return result


def source_set_sha(records: dict[str, str]) -> str:
    payload = json.dumps(records, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def _cpu() -> int | None:
    try:
        cpus = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return None
    return cpus[0] if cpus else None


def _run_process(executable: Path, args: list[str], cpu: int | None,
                 timeout: int = 180) -> subprocess.CompletedProcess[str]:
    command = [str(executable), *args]
    kwargs: dict[str, Any] = {"text": True, "capture_output": True, "timeout": timeout}
    if cpu is not None and hasattr(os, "sched_setaffinity"):
        kwargs["preexec_fn"] = lambda: os.sched_setaffinity(0, {cpu})
    try:
        return subprocess.run(command, **kwargs)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise BaselineError(f"command could not complete: {command!r}: {exc}") from exc


def _json_result(result: subprocess.CompletedProcess[str], command: list[str]) -> dict[str, Any]:
    if result.returncode != 0:
        raise BaselineError(f"command failed ({result.returncode}): {command!r}\n{result.stderr[-4000:]}")
    try:
        data = json.loads(result.stdout)
    except (json.JSONDecodeError, TypeError) as exc:
        raise BaselineError(f"command returned invalid JSON: {command!r}: {result.stdout[:1000]!r}") from exc
    if not isinstance(data, dict):
        raise BaselineError(f"command JSON must be an object: {command!r}")
    return data


def _protocol(binary: Path, cpu: int | None) -> dict[str, Any]:
    args = ["--protocol"]
    data = _json_result(_run_process(binary, args, cpu, timeout=30), [str(binary), *args])
    if data.get("schema_version") != 2:
        raise BaselineError("benchmark --protocol must report schema_version 2")
    if data.get("instrumented") is not False:
        raise BaselineError("baseline requires protocol instrumented:false")
    if MODE not in data.get("modes", []):
        raise BaselineError("benchmark protocol does not advertise decoded mode")
    return data


def _profile(entry: dict[str, Any]) -> str:
    features = set(entry.get("required_features", []))
    if features - {"mvp"}:
        return ALL_PHASE3
    return MVP


def _driver(entry: dict[str, Any]) -> str:
    if entry["family"] == "memory_copy_distinct_seeded":
        return "seeded-copy"
    if entry["family"] == "memory_fill_seeded":
        return "seeded-fill"
    if entry["family"] == "memory_copy_aliased":
        return "alias"
    if entry["family"] == "memory_multi_memory_second_export":
        return "multi"
    return "plain"


def _validate_seeded_observation(data: dict[str, Any], entry: dict[str, Any],
                                 input_value: int, expected: dict[str, Any],
                                 mode: str, profile: str, driver: str, *,
                                 stage: str = "load_and_call",
                                 expected_batch: int | None = 1) -> dict[str, Any]:
    """Strict P027-only validation for seeded operations and every exported image."""
    family = entry.get("family")
    expected_driver = {"memory_copy_distinct_seeded": "seeded-copy",
                       "memory_fill_seeded": "seeded-fill"}.get(family)
    if expected_driver is None or driver != expected_driver:
        raise BaselineError(f"{entry.get('id')}: seeded workload/driver identity mismatch")
    outcome = _expected_outcome(expected)
    identity = {"schema_version": 2, "case": entry["id"], "stage": stage,
                "input": f"i32:{input_value}", "expected": outcome, "result": outcome,
                "mode": mode, "profile": profile, "driver": expected_driver}
    if any(data.get(key) != value for key, value in identity.items()):
        raise BaselineError(f"{entry['id']}: seeded identity/result mismatch")
    batch = data.get("batch")
    if (type(batch) is not int or not 1 <= batch <= 100_000 or
            (expected_batch is not None and batch != expected_batch)):
        wanted = "1..100000" if expected_batch is None else str(expected_batch)
        raise BaselineError(f"{entry['id']}/{stage}: batch mismatch, expected {wanted}, got {batch!r}")
    if stage != "warm_call" and batch != 1:
        raise BaselineError(f"{entry['id']}/{stage}: cold stage requires batch=1")
    elapsed = data.get("ns_per_operation")
    if (isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or
            not math.isfinite(elapsed) or elapsed <= 0):
        raise BaselineError(f"{entry['id']}/{stage}: invalid timing {elapsed!r}")
    if data.get("host_setup_excluded") is not True:
        raise BaselineError(f"{entry['id']}: host seed writes must be excluded from guest timing")
    if data.get("measurement_policy") != "prepare_plus_instantiate_plus_first_call_excluding_host_seed":
        raise BaselineError(f"{entry['id']}: seeded timing policy mismatch")
    setup_ns = data.get("host_setup_ns")
    if (isinstance(setup_ns, bool) or not isinstance(setup_ns, (int, float)) or
            not math.isfinite(setup_ns) or setup_ns <= 0):
        raise BaselineError(f"{entry['id']}: invalid actual host setup duration")
    if data.get("trap_kind") is not None or data.get("trap_message") is not None:
        raise BaselineError(f"{entry['id']}: seeded workload unexpectedly trapped")
    if type(expected.get("result")) is not int or expected["result"] != input_value:
        raise BaselineError(f"{entry['id']}: scalar and parameter oracle disagree")

    memories = expected.get("memories")
    wanted_count = 2 if family == "memory_copy_distinct_seeded" else 1
    if not isinstance(memories, list) or len(memories) != wanted_count:
        raise BaselineError(f"{entry['id']}: malformed nested image oracle")
    images = data.get("all_memory_hex")
    if (not isinstance(images, list) or len(images) != wanted_count or
            data.get("memory_count") != wanted_count):
        raise BaselineError(f"{entry['id']}: all exported memory images are required")
    selected_index = wanted_count - 1
    if (data.get("exported_memory_index") != selected_index or
            data.get("memory_hex") != images[selected_index]):
        raise BaselineError(f"{entry['id']}: selected memory/export index mismatch")
    selected = memories[selected_index]
    if data.get("memory_pages") != selected.get("pages") or data.get("memory_page_size_log2") != 16:
        raise BaselineError(f"{entry['id']}: selected memory page shape mismatch")

    verified_images = []
    for index, (image_hex, oracle) in enumerate(zip(images, memories)):
        if oracle.get("memory_index") != index:
            raise BaselineError(f"{entry['id']}: model memory indices must be contiguous")
        pages = oracle.get("pages")
        if type(pages) is not int or pages < 1:
            raise BaselineError(f"{entry['id']}: malformed page count for memory {index}")
        try:
            image = bytes.fromhex(image_hex) if isinstance(image_hex, str) else None
        except ValueError as exc:
            raise BaselineError(f"{entry['id']}: memory {index} image is invalid hex") from exc
        if image is None or len(image) != pages * PAGE:
            raise BaselineError(f"{entry['id']}: memory {index} full image has wrong size")
        full_digest = hashlib.sha256(image).hexdigest()
        if full_digest != oracle.get("full_memory_sha256"):
            raise BaselineError(f"{entry['id']}: memory {index} full image digest mismatch")
        byte_range = oracle.get("range")
        if (not isinstance(byte_range, list) or len(byte_range) != 2 or
                any(type(value) is not int for value in byte_range)):
            raise BaselineError(f"{entry['id']}: memory {index} range contract is malformed")
        begin, end = byte_range
        if not 0 <= begin <= end <= len(image):
            raise BaselineError(f"{entry['id']}: memory {index} range is out of bounds")
        range_digest = hashlib.sha256(image[begin:end]).hexdigest()
        if range_digest != oracle.get("range_sha256"):
            raise BaselineError(f"{entry['id']}: memory {index} range digest mismatch")
        initial = oracle.get("initial_full_memory_sha256")
        if family == "memory_copy_distinct_seeded" and index == 0 and initial != full_digest:
            raise BaselineError(f"{entry['id']}: copy source image was unexpectedly modified")
        verified_images.append({"memory_index": index, "pages": pages,
                                "full_memory_sha256": full_digest,
                                "range": byte_range, "range_sha256": range_digest})
    return {"result": outcome, "batch": batch, "driver": expected_driver,
            "host_setup_excluded": True, "host_setup_ns": float(setup_ns),
            "memory_count": wanted_count, "memory_index": selected_index,
            "memory_page_size_log2": 16, "memory_pages": selected["pages"],
            "memories": verified_images}


def _expected_outcome(memory: dict[str, Any]) -> str:
    if memory.get("trap") in {"trap", "out_of_bounds"}:
        return "trap"
    if type(memory.get("result")) is not int:
        raise BaselineError(f"memory oracle lacks a scalar result or expected trap: {memory!r}")
    return f"i32:{memory['result']}"


def _expected_trap_kind(memory: dict[str, Any]) -> str | None:
    if memory.get("trap") in {"trap", "out_of_bounds"}:
        return "Some(MemoryOutOfBounds)"
    return None


def _validate_observation(data: dict[str, Any], entry: dict[str, Any], input_value: int,
                          expected: dict[str, Any], mode: str, profile: str,
                          driver: str, *, require_timing: bool = True,
                          stage: str = "load_and_call", expected_batch: int | None = 1) -> dict[str, Any]:
    if entry.get("family") in {"memory_copy_distinct_seeded", "memory_fill_seeded"}:
        # Seeded cases always require real timing and explicit host-setup proof;
        # `require_timing=False` is only a legacy validator option.
        return _validate_seeded_observation(
            data, entry, input_value, expected, mode, profile, driver,
            stage=stage, expected_batch=expected_batch)
    case = entry["id"]
    outcome = _expected_outcome(expected)
    input_text = f"i32:{input_value}"
    identity = {
        "schema_version": 2,
        "case": case,
        "stage": stage,
        "input": input_text,
        "expected": outcome,
        "result": outcome,
        "mode": mode,
        "profile": profile,
        "driver": driver,
    }
    mismatches = {key: {'expected': value, 'actual': data.get(key)}
                  for key, value in identity.items() if data.get(key) != value}
    if mismatches:
        raise BaselineError(f"outcome/identity mismatch for {case}: {mismatches!r}")
    actual_batch = data.get("batch")
    if (type(actual_batch) is not int or actual_batch < 1 or actual_batch > 100_000 or
            (expected_batch is not None and actual_batch != expected_batch)):
        wanted = "1..100000" if expected_batch is None else str(expected_batch)
        raise BaselineError(f"{stage} batch mismatch for {case}: expected {wanted}, got {actual_batch!r}")
    if stage != "warm_call" and actual_batch != 1:
        raise BaselineError(f"cold stage {stage} must use batch=1 for {case}")
    elapsed = data.get("ns_per_operation")
    if require_timing and (isinstance(elapsed, bool) or not isinstance(elapsed, (int, float))
                           or not math.isfinite(elapsed) or elapsed <= 0):
        raise BaselineError(f"invalid actual timing for {case}: {elapsed!r}")

    trap_kind = _expected_trap_kind(expected)
    if trap_kind is None:
        if data.get("trap_kind") is not None or data.get("trap_message") is not None:
            raise BaselineError(f"unexpected trap metadata for successful case {case}")
    else:
        if data.get("trap_kind") != trap_kind:
            raise BaselineError(f"wrong trap category for {case}: expected {trap_kind}, got {data.get('trap_kind')!r}")
        if not isinstance(data.get("trap_message"), str) or not data["trap_message"]:
            raise BaselineError(f"out-of-bounds trap omitted its actual message for {case}")

    pages = expected.get("pages_after", expected.get("initial_pages"))
    if (type(pages) is not int or type(data.get("memory_pages")) is not int or
            data.get("memory_pages") != pages):
        raise BaselineError(f"memory page count mismatch for {case}: expected {pages}, got {data.get('memory_pages')!r}")
    page_bytes_declared = expected.get("page_size_bytes")
    if page_bytes_declared is not None:
        if type(page_bytes_declared) is not int or page_bytes_declared <= 0 or page_bytes_declared & (page_bytes_declared - 1):
            raise BaselineError(f"invalid expected page size in bytes for {case}: {page_bytes_declared!r}")
        bytes_log2 = page_bytes_declared.bit_length() - 1
    else:
        bytes_log2 = 16
    page_log2 = expected.get("page_size_log2", bytes_log2)
    if page_bytes_declared is not None and page_log2 != bytes_log2:
        raise BaselineError(f"conflicting expected page-size encodings for {case}")
    if type(page_log2) is not int or not 0 <= page_log2 <= 30:
        raise BaselineError(f"invalid expected memory page size for {case}: {page_log2!r}")
    actual_page_log2 = data.get("memory_page_size_log2")
    if actual_page_log2 is None:
        if page_log2 != 16:
            raise BaselineError(f"driver omitted nondefault memory_page_size_log2 for {case}")
    elif type(actual_page_log2) is not int or actual_page_log2 != page_log2:
        raise BaselineError(f"memory page-size mismatch for {case}: expected {page_log2}, got {actual_page_log2!r}")
    expected_index = expected.get("memory_index", 0)
    actual_index = data.get("exported_memory_index", data.get("memory_index"))
    if actual_index is None:
        if expected_index != 0:
            raise BaselineError(f"driver omitted nonzero memory index for {case}")
    elif type(actual_index) is not int or actual_index != expected_index:
        raise BaselineError(f"memory index mismatch for {case}: expected {expected_index}, got {actual_index!r}")
    if "memory_count" in expected:
        actual_count = data.get("memory_count")
        if type(actual_count) is not int or actual_count != expected["memory_count"]:
            raise BaselineError(f"memory count mismatch for {case}: expected {expected['memory_count']}, got {actual_count!r}")
    if "all_memory_sha256" in expected:
        all_memory_hex = data.get("all_memory_hex")
        if not isinstance(all_memory_hex, list) or len(all_memory_hex) != expected.get("memory_count"):
            raise BaselineError(f"all-memory images missing or malformed for {case}")
        actual_all_hashes = []
        try:
            for image_hex in all_memory_hex:
                if not isinstance(image_hex, str):
                    raise ValueError("image is not hex text")
                actual_all_hashes.append(hashlib.sha256(bytes.fromhex(image_hex)).hexdigest())
        except ValueError as exc:
            raise BaselineError(f"all-memory image is not valid hex for {case}") from exc
        if actual_all_hashes != expected["all_memory_sha256"]:
            raise BaselineError(f"all-memory digest mismatch for {case}")
        if expected_index >= len(all_memory_hex) or all_memory_hex[expected_index] != data.get("memory_hex"):
            raise BaselineError(f"selected memory image/index mismatch for {case}")

    page_bytes = 1 << page_log2
    hex_memory = data.get("memory_hex")
    if not isinstance(hex_memory, str) or len(hex_memory) != pages * page_bytes * 2:
        raise BaselineError(f"full memory image has invalid size for {case}")
    try:
        actual_memory = bytes.fromhex(hex_memory)
    except ValueError as exc:
        raise BaselineError(f"full memory image is not valid hex for {case}") from exc
    actual_full_sha = hashlib.sha256(actual_memory).hexdigest()
    if actual_full_sha != expected.get("full_memory_sha256"):
        raise BaselineError(f"full-memory hash mismatch for {case}: {actual_full_sha} != {expected.get('full_memory_sha256')}")

    byte_range = expected.get("range")
    if byte_range is not None:
        if (not isinstance(byte_range, list) or len(byte_range) != 2 or
                any(type(v) is not int for v in byte_range)):
            raise BaselineError(f"invalid oracle byte range for {case}")
        begin, end = byte_range
        if not 0 <= begin <= end <= len(actual_memory):
            raise BaselineError(f"oracle byte range is outside memory for {case}")
        expected_hex = expected.get("bytes_hex")
        if not isinstance(expected_hex, str):
            raise BaselineError(f"oracle range lacks expected bytes for {case}")
        try:
            expected_range = bytes.fromhex(expected_hex)
        except ValueError as exc:
            raise BaselineError(f"oracle range bytes are not valid hex for {case}") from exc
        actual_range = actual_memory[begin:end]
        if len(expected_range) != end - begin or actual_range != expected_range:
            raise BaselineError(f"expected range bytes mismatch for {case} [{begin}, {end})")
        if "sha256" in expected and hashlib.sha256(actual_range).hexdigest() != expected["sha256"]:
            raise BaselineError(f"expected range digest mismatch for {case}")
    elif "bytes_hex" in expected:
        raise BaselineError(f"oracle range bytes lack a range for {case}")

    return {"result": outcome, "memory_index": expected_index,
            "batch": actual_batch,
            "memory_count": expected.get("memory_count", 1),
            "memory_page_size_log2": page_log2,
            "memory_page_size_bytes": page_bytes,
            "memory_pages": pages,
            "full_memory_sha256": actual_full_sha,
            "all_memory_sha256": expected.get("all_memory_sha256"),
            "range": byte_range,
            "range_sha256": expected.get("sha256"),
            "trap_kind": trap_kind}


def _encode(encoder: Path, tool_dir: Path, output: Path, registry: Any,
            cpu: int | None) -> dict[str, dict[str, str]]:
    target = output / "fixtures" / "memory"
    target.mkdir(parents=True, exist_ok=True)
    fixture_records: dict[str, dict[str, str]] = {}
    for filename, expected_wasm_hash in registry.WASM_SHA256.items():
        source = tool_dir / "fixtures/memory" / filename
        dest = target / Path(filename).with_suffix(".wasm")
        args = ["encode", str(source), "-o", str(dest)]
        result = _run_process(encoder, args, cpu)
        if result.returncode != 0:
            raise BaselineError(f"fixture encoder failed ({result.returncode}): {[str(encoder), *args]!r}\n{result.stderr[-2000:]}")
        if not dest.is_file():
            raise BaselineError(f"encoder did not create fixture: {dest}")
        digest = sha(dest)
        if digest != expected_wasm_hash:
            raise BaselineError(f"encoded fixture hash mismatch for {filename}: {digest} != {expected_wasm_hash}")
        fixture_records[filename] = {"wat_sha256": sha(source), "wasm_sha256": digest,
                                     "path": dest.relative_to(output).as_posix()}
    declared = {Path(entry["source"]["path"]).name for entry in registry.entries()}
    if set(fixture_records) != declared:
        raise BaselineError("encoded fixture identities do not exactly match the memory workload registry")
    return fixture_records


def _memory_driver_args(fixture: Path, entry: dict[str, Any], input_value: int,
                        expected: str, profile: str, driver: str, *, mode: str = MODE) -> list[str]:
    return ["--memory-driver", str(fixture), entry["id"], f"i32:{input_value}",
            expected, mode, profile, driver]


def _wasmtime_capabilities(probe_binary: Path | None, output: Path,
                           entries: list[dict[str, Any]], registry: Any,
                           fixtures: dict[str, dict[str, str]], cpu: int | None
                           ) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for entry in entries:
        wid = entry["id"]
        input_value = entry["params"][0]["baseline"]
        expected_memory = registry.expected_memory(wid, input_value)
        driver = _driver(entry)
        if probe_binary is None:
            rows.append({"case": wid, "status": "not_requested",
                         "reason": "--probe-binary was not supplied"})
            continue
        if _expected_outcome(expected_memory) == "trap":
            rows.append({"case": wid, "status": "unsupported",
                         "reason": "trap-category oracle needs the explicit memory driver; generic capability output is not an equivalent trap check"})
            continue
        if driver == "alias" or expected_memory.get("alias_memories") or entry.get("driver_requirements", {}).get("imports"):
            rows.append({"case": wid, "status": "unsupported",
                         "reason": "generic Wasmtime capability API cannot install the required imported memory/alias setup"})
            continue
        fixture = output / "fixtures/memory" / Path(entry["source"]["path"]).with_suffix(".wasm").name
        expected = _expected_outcome(expected_memory)
        args = ["--capability", str(fixture), f"i32:{input_value}", expected,
                ",".join(entry["required_features"])]
        result = _run_process(probe_binary, args, cpu)
        data = _json_result(result, [str(probe_binary), *args])
        if data.get("schema_version") != 2 or data.get("status") not in {"supported", "unsupported"}:
            raise BaselineError(f"invalid Wasmtime capability response for {wid}: {data!r}")
        if data["status"] == "supported" and data.get("compile_execute_checked") is not True:
            raise BaselineError(f"Wasmtime capability was not actually checked for {wid}")
        if data["status"] == "unsupported" and (not isinstance(data.get("reason"), str) or not data["reason"]):
            raise BaselineError(f"Wasmtime unsupported capability lacks a reason for {wid}")
        rows.append({"case": wid, "status": data["status"],
                     "probe_input": f"i32:{input_value}", "expected": expected,
                     "feature_profile": entry["required_features"],
                     "fixture_sha256": fixtures[Path(entry["source"]["path"]).name]["wasm_sha256"],
                     "response": data})
    return rows


def _ensure_output_empty(output: Path) -> None:
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        raise BaselineError(f"refusing to overwrite nonempty output directory: {output}")
    output.mkdir(parents=True, exist_ok=True)


def _fixture_hashes_on_disk(output: Path,
                            records: dict[str, dict[str, str]]) -> dict[str, str]:
    return {name: sha(output / record["path"]) for name, record in records.items()}


def run(args: argparse.Namespace, *, tool_dir: Path | None = None) -> tuple[int, dict[str, Any]]:
    tool_dir = Path(tool_dir or getattr(args, "tool_dir", None) or DEFAULT_TOOL_DIR).resolve()
    root = _repo_root(tool_dir)
    output = Path(args.output).resolve()
    _ensure_output_empty(output)
    binary = Path(args.binary).resolve()
    encoder = Path(args.encoder).resolve()
    probe_binary = Path(args.probe_binary).resolve() if getattr(args, "probe_binary", None) else None
    cpu = _cpu()
    report: dict[str, Any] = {
        "schema_version": 1,
        "kind": "p009-memory-scaling-baseline",
        "started_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "status": "running", "passed": False,
        "settings": {"mode": MODE, "samples_per_entry": args.samples,
                      "stage": "load_and_call", "reset_policy": "fresh_instance_per_invocation",
                      "instrumented": False, "memory_hex_persisted": False},
        "observations": [], "correctness_probes": [], "failures": [],
        "executables": {"binary": str(binary), "encoder": str(encoder),
                        "probe_binary": str(probe_binary) if probe_binary else None},
        "environment": {"platform": platform.platform(), "python": sys.version,
                        "cpu": platform.processor() or platform.machine(), "selected_cpu": cpu},
    }
    before_sources: dict[str, str] = {}
    before_binary = before_encoder = before_probe = None
    fixture_hashes: dict[str, dict[str, str]] = {}
    try:
        before_sources = source_hashes(root, tool_dir)
        before_binary, before_encoder = sha(binary), sha(encoder)
        before_probe = sha(probe_binary) if probe_binary else None
        report["source_hashes_before"] = before_sources
        report["source_set_sha256_before"] = source_set_sha(before_sources)
        report["binary_sha256_before"] = before_binary
        report["encoder_sha256_before"] = before_encoder
        if before_probe:
            report["probe_binary_sha256_before"] = before_probe
        registry = _load_registry(tool_dir)
        entries = registry.entries()
        ids = {entry["id"] for entry in entries}
        fixture_names = {Path(entry["source"]["path"]).name for entry in entries}
        if len(ids) != len(entries):
            raise BaselineError("memory registry contains duplicate workload IDs")
        if fixture_names != set(registry.WASM_SHA256):
            raise BaselineError("memory workload fixture set does not match declared encoder fixture set")
        manifest = json.loads((tool_dir / "workloads.json").read_text(encoding="utf-8"))
        selections = manifest.get("selections", {})
        memory_selection = selections.get("memory", {})
        selected_ids = memory_selection.get("workloads", [])
        expected_selected = ids | {"memory_fill_scan"}
        if memory_selection.get("status") != "ready" or set(selected_ids) != expected_selected or len(selected_ids) != len(expected_selected):
            raise BaselineError("workloads.json memory selection must contain each memory workload and historical memory_fill_scan exactly once")
        fixture_hashes = _encode(encoder, tool_dir, output, registry, cpu)
        report["fixture_hashes"] = fixture_hashes
        report["entry_count"] = len(entries)
        report["protocol"] = _protocol(binary, cpu)

        for entry in entries:
            workload_id = entry["id"]
            input_value = entry["params"][0]["baseline"]
            profile = _profile(entry)
            driver = _driver(entry)
            expected_memory = registry.expected_memory(workload_id, input_value)
            oracle_outcome = registry.oracle(workload_id, input_value)
            if expected_memory.get("trap") in {"trap", "out_of_bounds"}:
                if oracle_outcome != "trap":
                    raise BaselineError(f"scalar and memory oracle disagree for expected trap {workload_id}")
            elif expected_memory.get("result") != oracle_outcome:
                raise BaselineError(f"scalar and memory oracles disagree for {workload_id}")
            fixture_path = output / "fixtures/memory" / Path(entry["source"]["path"]).with_suffix(".wasm").name
            record = fixture_hashes[Path(entry["source"]["path"]).name]
            if record["wat_sha256"] != entry["source"]["sha256"]:
                raise BaselineError(f"WAT source identity mismatch after encode for {workload_id}")
            if sha(fixture_path) != record["wasm_sha256"] or record["wasm_sha256"] != entry["source"]["wasm_sha256"]:
                raise BaselineError(f"fixture identity mismatch after encode for {workload_id}")
            expected = _expected_outcome(expected_memory)
            args_driver = _memory_driver_args(fixture_path, entry, input_value,
                                              expected, profile, driver)
            for sample in range(args.samples):
                raw = _json_result(_run_process(binary, args_driver, cpu),
                                   [str(binary), *args_driver])
                verified = _validate_observation(raw, entry, input_value, expected_memory,
                                                  MODE, profile, driver)
                report["observations"].append({
                    "case": workload_id, "family": entry["family"], "input": input_value,
                    "expected": expected, "stage": "load_and_call", "sample": sample,
                    "batch": 1, "ns_per_operation": raw["ns_per_operation"],
                    "mode": MODE, "profile": profile, "driver": driver,
                    "fixture_sha256": record["wasm_sha256"], "oracle_checked": True,
                    **verified,
                })

        # Boundary-only memory64 probe: zero-extend address 65536, trap before
        # the fixture's first store, and leave the original page all zero.
        memory64 = next(entry for entry in entries if entry["family"] == "memory64_i32_extended")
        fixture_path = output / "fixtures/memory/memory64.wasm"
        boundary_expected = {
            "trap": "out_of_bounds", "initial_pages": 1,
            "range": [0, PAGE], "bytes_hex": bytes(PAGE).hex(),
            "sha256": hashlib.sha256(bytes(PAGE)).hexdigest(),
            "full_memory_sha256": hashlib.sha256(bytes(PAGE)).hexdigest(),
        }
        args_driver = _memory_driver_args(fixture_path, memory64, PAGE,
                                          "trap", ALL_PHASE3, "plain")
        raw = _json_result(_run_process(binary, args_driver, cpu),
                           [str(binary), *args_driver])
        verified = _validate_observation(raw, memory64, PAGE, boundary_expected,
                                         MODE, ALL_PHASE3, "plain")
        report["correctness_probes"].append({
            "case": memory64["id"], "variant": "i32-extended-address-at-page-boundary",
            "input": PAGE, "expected": "trap", "status": "passed",
            "diagnostic": True, "timing_excluded": True,
            "address_extension": "i32.extend_i32_u", **verified,
        })

        report["wasmtime_capability_matrix"] = _wasmtime_capabilities(
            probe_binary, output, entries, registry, fixture_hashes, cpu)

        expected_rows = len(entries) * args.samples
        if len(report["observations"]) != expected_rows:
            raise BaselineError(f"incomplete timing coverage: {len(report['observations'])}/{expected_rows}")
        report["fixture_hashes_after"] = _fixture_hashes_on_disk(output, fixture_hashes)
        report["fixtures_stable"] = all(
            report["fixture_hashes_after"].get(name) == record["wasm_sha256"]
            for name, record in fixture_hashes.items())
        if not report["fixtures_stable"]:
            raise BaselineError("an encoded fixture changed during the run")
        report["source_hashes_after"] = source_hashes(root, tool_dir)
        report["source_set_sha256_after"] = source_set_sha(report["source_hashes_after"])
        report["source_stable"] = report["source_hashes_before"] == report["source_hashes_after"]
        report["binary_sha256_after"] = sha(binary)
        report["encoder_sha256_after"] = sha(encoder)
        if probe_binary:
            report["probe_binary_sha256_after"] = sha(probe_binary)
            report["probe_binary_stable"] = before_probe == report["probe_binary_sha256_after"]
        report["binary_stable"] = report["binary_sha256_before"] == report["binary_sha256_after"]
        report["encoder_stable"] = report["encoder_sha256_before"] == report["encoder_sha256_after"]
        if not report["source_stable"]:
            raise BaselineError("source, workload registry, or fixtures changed during run")
        if (not report["binary_stable"] or not report["encoder_stable"] or
                report.get("probe_binary_stable") is False):
            raise BaselineError("binary or encoder changed during run")
        report["status"] = "passed"
        report["passed"] = True
        report["performance_claim"] = "none; integrity baseline only"
    except Exception as exc:
        report["failures"].append({"type": type(exc).__name__, "message": str(exc)})
        report["status"] = "failed"
        report["passed"] = False
        try:
            after = source_hashes(root, tool_dir)
            report["source_hashes_after"] = after
            report["source_set_sha256_after"] = source_set_sha(after)
            report["source_stable"] = bool(before_sources) and before_sources == after
        except Exception as hash_error:
            report["source_stable"] = False
            report["failures"].append({"type": type(hash_error).__name__,
                                       "message": f"final source hashing failed: {hash_error}"})
        if fixture_hashes:
            try:
                report["fixture_hashes_after"] = _fixture_hashes_on_disk(output, fixture_hashes)
                report["fixtures_stable"] = all(
                    report["fixture_hashes_after"].get(name) == record["wasm_sha256"]
                    for name, record in fixture_hashes.items())
            except OSError as hash_error:
                report["fixtures_stable"] = False
                report["failures"].append({"type": type(hash_error).__name__, "message": str(hash_error)})
        for label, path, before in (("binary", binary, before_binary),
                                    ("encoder", encoder, before_encoder),
                                    ("probe_binary", probe_binary, before_probe)):
            if before is None:
                continue
            try:
                after_digest = sha(path)
                report[f"{label}_sha256_after"] = after_digest
                report[f"{label}_stable"] = before == after_digest
            except OSError as hash_error:
                report[f"{label}_stable"] = False
                report["failures"].append({"type": type(hash_error).__name__, "message": str(hash_error)})
    report["completed_at_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    report["actual_observation_count"] = len(report["observations"])
    report["actual_probe_count"] = len(report["correctness_probes"])
    report["entry_count"] = report.get("entry_count", 0)
    report["samples_per_entry"] = args.samples
    report["performance_claim"] = report.get("performance_claim", "none; integrity baseline only")
    (output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return (0 if report["passed"] else 1), report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="existing wtime memory-driver executable")
    parser.add_argument("--encoder", required=True, type=Path, help="existing conformance WAT encoder")
    parser.add_argument("--output", required=True, type=Path, help="new output directory; must be empty or absent")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--probe-binary", type=Path,
                        help="optional Wasmtime --capability harness executable")
    parser.add_argument("--tool-dir", type=Path, default=DEFAULT_TOOL_DIR,
                        help="directory containing memory_workloads.py and fixtures")
    args = parser.parse_args(argv)
    if args.samples <= 0 or args.samples > 1000:
        parser.error("--samples must be between 1 and 1000")
    code, report = run(args)
    print(json.dumps({"status": report["status"], "passed": report["passed"],
                      "report": str((Path(args.output).resolve() / "report.json")),
                      "entries": report["entry_count"],
                      "observations": report["actual_observation_count"],
                      "failures": len(report["failures"])}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
