#!/usr/bin/env python3
"""Source-bound decoded/reference semantic differential checks.

This is a correctness runner, not a performance benchmark. It uses the public
typed single-case protocol and requires every selected case to run in both modes.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Callable


ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
DEFAULT_BINARY = HERE / "target/release/wtime-bench"
DEFAULT_COUNTERS_BINARY = ROOT / "target/performance/P014/counters-build/release/wtime-bench"
DEFAULT_CONFORMANCE = ROOT / "target/release/wtime-conformance"
REGISTRY = HERE / "workloads.json"
STATE_MANIFEST = HERE / "fixtures/integration/expected_state.json"
STATE_FIXTURE_DIR = HERE / "fixtures/integration"
STATE_WASM_DIR = ROOT / "target/performance/P014-state-fixtures"
STATE_CASES = ("state_normal", "state_trap", "state_graph")
STATE_FUEL = 100_000
INTEGRATION_CASES = ("call_overhead", "integer_loop", "branch_calls")
CONFORMANCE_SUITES = ("reference_types", "interactions")
SNAPSHOT_WASM = ROOT / "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm"
SNAPSHOT_WAT = ROOT / "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat"
SNAPSHOT = {
    "case": "store_successor",
    "args": "-",
    "expected": "i32:7",
    "source_pc": 3,
    "profile": "all-phase3-core",
}
MODES = ("reference", "decoded")
TIMEOUT_SECONDS = 45
MAX_REGISTER_ENTRY_THRESHOLD = (1 << 64) - 1
REFERENCE_BUILD = [
    "cargo", "build", "--locked", "--release", "--manifest-path",
    "tools/wtime-bench/Cargo.toml", "-j", "4", "--no-default-features",
    "--features", "execution-reference,runtime-only",
]
CONFORMANCE_BUILD = [
    "cargo", "build", "--locked", "--release", "-p", "wtime-conformance",
    "--no-default-features", "--features", "execution-reference,opcode-coverage",
]
COUNTERS_BUILD = [
    "cargo", "build", "--locked", "--release", "--manifest-path",
    "tools/wtime-bench/Cargo.toml", "-j", "4", "--no-default-features",
    "--features", "execution-reference,performance-counters,runtime-only", "--target-dir",
    "target/performance/P014/counters-build",
]


class TierCheckError(ValueError):
    """A tier run is missing, stale, unsupported, or semantically inconsistent."""


def _sha(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise TierCheckError(f"cannot hash {path}: {exc}") from exc


def _load_oracle():
    spec = importlib.util.spec_from_file_location("tier_check_run_oracle", HERE / "run.py")
    if spec is None or spec.loader is None:
        raise TierCheckError("cannot load the independent run.py oracle")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.oracle


def _source_paths() -> list[Path]:
    paths: set[Path] = {ROOT / "Cargo.toml", ROOT / "Cargo.lock", REGISTRY,
                         HERE / "Cargo.toml", HERE / "Cargo.lock", HERE / "workloads.py",
                         HERE / "run.py", Path(__file__).resolve()}
    for crate in ("wtime-types", "wtime-decode", "wtime-validate", "wtime-runtime"):
        base = ROOT / "crates" / crate
        paths.add(base / "Cargo.toml")
        paths.update(base.rglob("*.rs"))
    src = HERE / "src"
    paths.update(src.rglob("*.rs"))
    conformance = ROOT / "tools/wtime-conformance"
    paths.update(conformance.rglob("*.rs"))
    paths.update({conformance / "Cargo.toml", HERE / "fixtures/integration/expected_state.json"})
    return sorted(path for path in paths if path.is_file())


def _source_identity() -> dict[str, str]:
    return {path.relative_to(ROOT).as_posix(): _sha(path) for path in _source_paths()}


def _fixture_record(entry: dict[str, Any]) -> dict[str, str]:
    source = entry["source"]
    wat = HERE / source["path"]
    wasm = wat.with_suffix(".wasm")
    wat_hash, wasm_hash = _sha(wat), _sha(wasm)
    if wat_hash != source["sha256"]:
        raise TierCheckError(f"{entry['id']}: WAT fixture hash differs from workloads.json")
    if wasm_hash != source["wasm_sha256"]:
        raise TierCheckError(f"{entry['id']}: Wasm fixture hash differs from workloads.json")
    return {"wat": wat.relative_to(ROOT).as_posix(), "wat_sha256": wat_hash,
            "wasm": wasm.relative_to(ROOT).as_posix(), "wasm_sha256": wasm_hash}


def _integration_cases() -> list[dict[str, Any]]:
    try:
        registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise TierCheckError(f"cannot read workload registry: {exc}") from exc
    workloads = registry.get("workloads")
    if not isinstance(workloads, list):
        raise TierCheckError("workload registry has no workloads list")
    by_id = {item.get("id"): item for item in workloads if isinstance(item, dict)}
    oracle = _load_oracle()
    selected = []
    for case_id in INTEGRATION_CASES:
        entry = by_id.get(case_id)
        if entry is None:
            raise TierCheckError(f"integration semantic case is absent: {case_id}")
        oracle_info = entry.get("oracle", {})
        if (oracle_info.get("kind") != "python" or oracle_info.get("module") != "run.py"
                or oracle_info.get("function") != "oracle" or oracle_info.get("independent") is not True):
            raise TierCheckError(f"{case_id}: expected an independent run.py oracle")
        features = entry.get("required_features")
        if features != ["mvp"]:
            raise TierCheckError(f"{case_id}: tier smoke set is pinned to the MVP profile")
        params = entry.get("params")
        if not isinstance(params, list) or len(params) != 1 or params[0].get("type") != "i32":
            raise TierCheckError(f"{case_id}: typed integration case requires one i32 input")
        value = params[0].get("baseline")
        if type(value) is not int:
            raise TierCheckError(f"{case_id}: baseline i32 input is missing")
        if entry.get("result", {}).get("type") != "i32":
            raise TierCheckError(f"{case_id}: expected an i32 result")
        expected = oracle(case_id, value)
        selected.append({
            "id": case_id,
            "wasm": HERE / entry["source"]["path"].replace(".wat", ".wasm"),
            "input": f"i32:{value}",
            "expected": f"i32:{expected}",
            "oracle": {"module": "run.py", "function": "oracle", "independent": True},
            "fixture": _fixture_record(entry),
        })
    if len(selected) < 3 or len({case["id"] for case in selected}) != len(selected):
        raise TierCheckError("integration selection must contain at least three unique semantic cases")
    return selected


def _run_json(binary: Path, args: list[str], runner: Callable[..., Any]) -> dict[str, Any]:
    try:
        completed = runner(
            [str(binary), *args], cwd=ROOT, text=True, capture_output=True,
            timeout=TIMEOUT_SECONDS, check=False,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise TierCheckError(f"could not run {binary.name} {' '.join(args[:2])}: {exc}") from exc
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "no process output").strip()
        raise TierCheckError(f"process exited {completed.returncode}: {detail[-1000:]}")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise TierCheckError(f"process returned invalid JSON: {completed.stdout[:300]!r}") from exc
    if not isinstance(value, dict):
        raise TierCheckError("process JSON result must be an object")
    return value


def _protocol(binary: Path, runner: Callable[..., Any]) -> dict[str, Any]:
    value = _run_json(binary, ["--protocol"], runner)
    modes = value.get("modes")
    if value.get("schema_version") != 2 or value.get("engine") != "wtime":
        raise TierCheckError("benchmark --protocol schema/engine mismatch")
    if not isinstance(modes, list) or any(not isinstance(mode, str) for mode in modes):
        raise TierCheckError("benchmark --protocol modes are missing or malformed")
    required = {"decoded", "reference"}
    missing = sorted(required - set(modes))
    if missing:
        raise TierCheckError("required execution modes are not built/forwarded: " + ", ".join(missing))
    if value.get("instrumented") is not True:
        raise TierCheckError("tier differential binary must expose development-only reference diagnostics")
    return value


def _case_run(binary: Path, case: dict[str, Any], mode: str,
              runner: Callable[..., Any],
              register_entry_threshold: int | None = None) -> dict[str, Any]:
    args = [
        "--typed-case", str(case["wasm"]), case["id"], case["input"], case["expected"],
        "first_call", "1", mode, "mvp",
    ]
    if register_entry_threshold is not None and mode == "register":
        args.extend(["--register-entry-threshold", str(register_entry_threshold)])
    result = _run_json(binary, args, runner)
    _check_register_entry_threshold(
        result, register_entry_threshold if mode == "register" else None, f"{case['id']} ({mode})"
    )
    expected_fields = {
        "schema_version": 2, "case": case["id"], "stage": "first_call",
        "input": case["input"], "expected": case["expected"], "result": case["expected"],
        "batch": 1, "mode": mode, "profile": "mvp",
    }
    for key, expected in expected_fields.items():
        if result.get(key) != expected:
            raise TierCheckError(f"{case['id']} ({mode}): {key}={result.get(key)!r}, expected {expected!r}")
    elapsed = result.get("ns_per_operation")
    if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed <= 0:
        raise TierCheckError(f"{case['id']} ({mode}): invalid execution observation")
    return {"mode": mode, "stage": "first_call", "result": result["result"],
            "register_entry_threshold": register_entry_threshold if mode == "register" else None,
            "ns_per_operation": elapsed, "binary_sha256": None}


def _check_register_entry_threshold(value: dict[str, Any], requested: int | None,
                                    context: str) -> None:
    observed = value.get("register_entry_threshold")
    if requested is None:
        if observed is not None:
            raise TierCheckError(f"{context}: unexpected register entry threshold {observed!r}")
    elif type(observed) is not int or observed != requested:
        raise TierCheckError(
            f"{context}: register entry threshold {observed!r}, expected {requested}"
        )


def _snapshot_run(binary: Path, runner: Callable[..., Any]) -> dict[str, Any]:
    value = _run_json(binary, [
        "--snapshot", str(SNAPSHOT_WASM), SNAPSHOT["args"], SNAPSHOT["expected"],
        str(SNAPSHOT["source_pc"]), "decoded", SNAPSHOT["profile"], SNAPSHOT["case"],
    ], runner)
    roundtrips = value.get("materialization_roundtrips")
    if (value.get("schema_version") != 1 or value.get("diagnostic") is not True
            or value.get("case") != SNAPSHOT["case"]
            or value.get("source_pc") != SNAPSHOT["source_pc"]
            or value.get("pending_boundary_charged") is not True
            or type(roundtrips) is not int or roundtrips < 1
            or not isinstance(value.get("normalized_state"), str)
            or not value["normalized_state"]):
        raise TierCheckError("P018 decoded snapshot did not prove the requested source boundary/roundtrip")
    return value


def _dispatch_diagnostic(binary: Path, case: dict[str, Any],
                         runner: Callable[..., Any]) -> dict[str, Any]:
    input_value = int(case["input"].split(":", 1)[1])
    expected_value = int(case["expected"].split(":", 1)[1])
    value = _run_json(binary, [
        "--diagnostic", str(case["wasm"]), str(input_value), str(expected_value),
        "decoded", "mvp",
    ], runner)
    counters = value.get("counters")
    dispatches = counters.get("source_dispatches") if isinstance(counters, dict) else None
    if (value.get("schema_version") != 1 or value.get("diagnostic") is not True
            or value.get("result") != expected_value
            or type(dispatches) is not int or dispatches <= 0):
        raise TierCheckError(f"{case['id']}: diagnostic did not report a valid decoded source-op count")
    return {"case": case["id"], "mode": "decoded", "source_dispatches": dispatches,
            "counters": counters}


def _build_reference(binary: Path, builder: Callable[..., Any], tier: str = "reference") -> dict[str, Any]:
    # Use the established feature-graph validator before invoking Cargo so a
    # stale or incomplete forwarding edge cannot silently select decoded mode.
    try:
        sys.path.insert(0, str(HERE))
        import baseline
        resolved, reference_features = baseline._mode_config(ROOT, "reference")
        candidate_resolved, candidate_features = baseline._mode_config(ROOT, tier)
    except (ImportError, ValueError) as exc:
        raise TierCheckError(f"tier Cargo feature forwarding is invalid: {exc}") from exc
    if resolved != "reference" or reference_features != ["execution-reference"]:
        raise TierCheckError("reference build resolved to an unexpected Cargo feature set")
    if candidate_resolved != tier and not (tier == "portable" and candidate_resolved == "decoded"):
        raise TierCheckError(f"candidate tier {tier!r} resolved to unexpected mode {candidate_resolved!r}")
    if tier == "reference":
        candidate_features = []
    benchmark_features = [*reference_features, *candidate_features, "runtime-only"]
    conformance_features = [*reference_features, *candidate_features, "opcode-coverage"]
    benchmark_build = [*REFERENCE_BUILD]
    benchmark_build[benchmark_build.index("execution-reference,runtime-only")] = ",".join(benchmark_features)
    conformance_build = [*CONFORMANCE_BUILD]
    conformance_build[conformance_build.index("execution-reference,opcode-coverage")] = ",".join(conformance_features)
    builds = []
    for command, target, label in (
        (benchmark_build, binary, f"{tier} benchmark"),
        (conformance_build, DEFAULT_CONFORMANCE, f"{tier} conformance runner"),
        (COUNTERS_BUILD, DEFAULT_COUNTERS_BINARY, "instrumented diagnostic benchmark"),
    ):
        try:
            result = builder(command, cwd=ROOT, text=True, capture_output=True,
                             timeout=1800, check=False)
        except (OSError, subprocess.SubprocessError) as exc:
            raise TierCheckError(f"could not build {label}: {exc}") from exc
        builds.append({"command": command, "returncode": result.returncode})
        if result.returncode:
            detail = (result.stderr or result.stdout or "no build output").strip()
            raise TierCheckError(f"{label} build failed: {detail[-1500:]}")
        if not target.is_file() or not os.access(target, os.X_OK):
            raise TierCheckError(f"build did not produce executable: {target}")
    return {"feature_sets": {
                "policy": [*reference_features, *candidate_features],
                "benchmark_harness": [*candidate_features, "runtime-only"],
                "conformance_policy": [*reference_features, *candidate_features],
                "conformance_instrumentation": ["opcode-coverage"],
                "diagnostic_instrumentation": ["performance-counters"],
            },
            "commands": builds}


def _conformance_run(binary: Path, suite: str, mode: str, report_path: Path,
                     runner: Callable[..., Any],
                     register_entry_threshold: int | None = None) -> dict[str, Any]:
    report_path.parent.mkdir(parents=True, exist_ok=True)
    command = [str(binary), "run", "--mode", mode, "--profile", "all-phase3-core",
               "--suite", suite, "--filter", "all", "--report", str(report_path)]
    if register_entry_threshold is not None and mode == "register":
        command.extend(["--register-entry-threshold", str(register_entry_threshold)])
    try:
        completed = runner(command, cwd=ROOT, text=True, capture_output=True,
                           timeout=1800, check=False)
    except (OSError, subprocess.SubprocessError) as exc:
        raise TierCheckError(f"could not run {suite} conformance in {mode}: {exc}") from exc
    if not report_path.is_file():
        detail = (completed.stderr or completed.stdout or "no process output").strip()
        raise TierCheckError(f"{suite} conformance ({mode}) produced no report: {detail[-1000:]}")
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise TierCheckError(f"{suite} conformance ({mode}) report is invalid: {exc}") from exc
    if completed.returncode:
        detail = (completed.stderr or completed.stdout or "no process output").strip()
        raise TierCheckError(f"{suite} conformance ({mode}) failed: {detail[-1000:]}")
    if report.get("execution_mode") != mode or report.get("instrumented") is not True:
        raise TierCheckError(f"{suite} conformance report did not identify instrumented mode {mode}")
    _check_register_entry_threshold(
        report, register_entry_threshold if mode == "register" else None,
        f"{suite} conformance ({mode})",
    )
    for field in ("selected", "executed", "failed", "skipped", "unsupported"):
        if type(report.get(field)) is not int:
            raise TierCheckError(f"{suite} conformance report has malformed {field}")
    if (report["selected"] <= 0 or report["executed"] <= 0 or report["failed"] != 0
            or report["skipped"] != 0 or report["unsupported"] != 0):
        raise TierCheckError(f"{suite} conformance ({mode}) has zero work or nonpassing outcomes")
    opcodes = report.get("executed_opcodes")
    if not isinstance(opcodes, list) or not opcodes or any(type(op) is not int for op in opcodes):
        raise TierCheckError(f"{suite} conformance ({mode}) lacks actual opcode-coverage evidence")
    source_hash = report.get("source_hash")
    if not isinstance(source_hash, str):
        raise TierCheckError(f"{suite} conformance ({mode}) report has no source_hash")
    corpus_sources = _resolve_corpus_sources(source_hash)
    return {"suite": suite, "mode": mode,
            "register_entry_threshold": register_entry_threshold if mode == "register" else None,
            "report": report,
            "report_sha256": _sha(report_path), "command": command,
            "corpus_sources": corpus_sources}


def _resolve_corpus_sources(source_hash: str) -> dict[str, str]:
    lock = ROOT / "docs/architecture/corpus-lock.json"
    expected_lock_hash = _sha(lock)
    sources: dict[str, str] = {lock.relative_to(ROOT).as_posix(): expected_lock_hash}
    saw_lock = False
    for item in source_hash.split(";"):
        name, separator, digest = item.rpartition(":")
        if not separator or not name or len(digest) != 64:
            raise TierCheckError("conformance source_hash has a malformed component")
        if name == "corpus-lock":
            saw_lock = True
            if digest != expected_lock_hash:
                raise TierCheckError("conformance corpus-lock identity differs from repository lock")
            continue
        path = Path(name)
        if not path.is_absolute():
            path = ROOT / path
        path = path.resolve()
        if ROOT not in path.parents:
            raise TierCheckError(f"conformance source path escapes repository: {name}")
        actual = _sha(path)
        if actual != digest:
            raise TierCheckError(f"conformance source identity mismatch: {name}")
        sources[path.relative_to(ROOT).as_posix()] = actual
    if not saw_lock or len(sources) < 2:
        raise TierCheckError("conformance source_hash lacks pinned corpus lock or selected source files")
    return sources


def _state_expected() -> dict[str, Any]:
    try:
        manifest = json.loads(STATE_MANIFEST.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise TierCheckError(f"cannot read P014 state expectations: {exc}") from exc
    if manifest.get("schema_version") != 1 or not isinstance(manifest.get("cases"), dict):
        raise TierCheckError("P014 state expectation manifest has an unsupported schema")
    return manifest


def _state_fixture_hashes(manifest: dict[str, Any]) -> dict[str, dict[str, str]]:
    result = {}
    for case in STATE_CASES:
        entry = manifest["cases"].get(case)
        if not isinstance(entry, dict):
            raise TierCheckError(f"P014 state expectation missing {case}")
        hashes = entry.get("source")
        if not isinstance(hashes, dict):
            raise TierCheckError(f"P014 state expectation has no fixture identity for {case}")
        wat, wasm = STATE_FIXTURE_DIR / f"{case}.wat", STATE_WASM_DIR / f"{case}.wasm"
        wat_hash, wasm_hash = _sha(wat), _sha(wasm)
        if wat_hash != hashes.get("wat_sha256") or wasm_hash != hashes.get("wasm_sha256"):
            raise TierCheckError(f"{case}: state fixture bytes differ from expected_state.json")
        result[case] = {"wat": wat.relative_to(ROOT).as_posix(), "wat_sha256": wat_hash,
                        "wasm": wasm.relative_to(ROOT).as_posix(), "wasm_sha256": wasm_hash}
    return result


def _fixture_identities(cases: list[dict[str, Any]],
                        state_fixtures: dict[str, dict[str, str]]) -> dict[str, Any]:
    return {
        "semantic_cases": {case["id"]: case["fixture"] for case in cases},
        "state_cases": state_fixtures,
        "p018_cursor": {
            "wat": SNAPSHOT_WAT.relative_to(ROOT).as_posix(), "wat_sha256": _sha(SNAPSHOT_WAT),
            "wasm": SNAPSHOT_WASM.relative_to(ROOT).as_posix(), "wasm_sha256": _sha(SNAPSHOT_WASM),
        },
    }


def _state_run(binary: Path, case: str, mode: str, expected: dict[str, Any],
               runner: Callable[..., Any],
               register_entry_threshold: int | None = None) -> dict[str, Any]:
    wasm = STATE_WASM_DIR / f"{case}.wasm"
    args = ["--tier-state-driver", str(wasm), case, mode,
            "all-phase3-core", str(STATE_FUEL), "none"]
    if register_entry_threshold is not None and mode == "register":
        args.extend(["--register-entry-threshold", str(register_entry_threshold)])
    value = _run_json(binary, args, runner)
    _check_register_entry_threshold(
        value, register_entry_threshold if mode == "register" else None, f"{case} ({mode})"
    )
    want_outcome = expected["expected_outcome"].split(":", 1)
    if want_outcome[0] == "returned":
        outcome, trap, values = "returned", None, [want_outcome[1]]
    else:
        outcome, trap, values = "Trap", want_outcome[1], []
    if (value.get("schema_version") != 1 or value.get("diagnostic") is not True
            or value.get("instrumented") is not False or value.get("case") != case
            or value.get("mode") != mode or value.get("fuel_budget") != STATE_FUEL
            or value.get("outcome") != outcome or value.get("trap") != trap
            or value.get("values") != values):
        raise TierCheckError(f"{case} ({mode}): outcome/result protocol mismatch")
    expected_policy = {"reference": "Reference", "decoded": "Decoded",
                       "portable": "Portable", "compact": "Compact",
                       "register": "Register"}.get(mode)
    if expected_policy is None:
        raise TierCheckError(f"unsupported state-driver mode {mode!r}")
    if value.get("resolved_policy") != expected_policy:
        raise TierCheckError(f"{case} ({mode}): resolved policy differs from requested policy")
    memory_hex = value.get("memory_hex")
    if not isinstance(memory_hex, str) or len(memory_hex) != 2 * 65536:
        raise TierCheckError(f"{case} ({mode}): full 64KiB memory image is missing")
    try:
        memory = bytes.fromhex(memory_hex)
    except ValueError as exc:
        raise TierCheckError(f"{case} ({mode}): malformed memory image") from exc
    memory_expectation = expected["memory"]
    if (len(memory) != memory_expectation["byte_length"]
            or hashlib.sha256(memory).hexdigest() != memory_expectation["sha256"]
            or int.from_bytes(memory[:4], "little") != memory_expectation["prefix_u32"]
            or any(memory[4:])):
        raise TierCheckError(f"{case} ({mode}): full memory state differs from manifest")
    if value.get("global") != f"i32:{expected['global_state_i32']}":
        raise TierCheckError(f"{case} ({mode}): mutable global differs from manifest")
    expected_table = ["function", "null"]
    if value.get("table") != expected_table:
        raise TierCheckError(f"{case} ({mode}): table state differs from manifest")
    graph = expected["root_graph"]
    expected_root = None if graph is None else "gc"
    expected_projection = None if graph is None else [f"i32:{n}" for n in graph["normalized_guest_projection"]]
    if value.get("root") != expected_root or value.get("graph") != expected_projection:
        raise TierCheckError(f"{case} ({mode}): normalized reference graph differs from manifest")
    collection = value.get("collection")
    if graph is None:
        if not isinstance(collection, dict):
            raise TierCheckError(f"{case} ({mode}): collection result is missing")
    elif collection != {"live": graph["after_collection"]["live_gc_objects"],
                        "reclaimed": graph["after_collection"]["reclaimed_gc_objects"]}:
        raise TierCheckError(f"{case} ({mode}): normalized graph collection state differs from manifest")
    if value.get("cursor") is not None:
        raise TierCheckError(f"{case} ({mode}): unexpected cursor snapshot for PC=none")
    return {"case": case, "mode": mode,
            "register_entry_threshold": register_entry_threshold if mode == "register" else None,
            "outcome": outcome, "trap": trap,
            "values": values, "memory_sha256": hashlib.sha256(memory).hexdigest(),
            "memory_byte_length": len(memory), "memory_prefix_u32": int.from_bytes(memory[:4], "little"),
            "remaining_memory_zero": not any(memory[4:]), "global": value["global"],
            "table": value["table"], "root": value["root"], "graph": value["graph"],
            "collection": collection, "binary_sha256": _sha(binary)}


def _state_fuel_probe(binary: Path, case: str, mode: str, fuel: int,
                      runner: Callable[..., Any],
                      register_entry_threshold: int | None = None) -> dict[str, Any]:
    wasm = STATE_WASM_DIR / f"{case}.wasm"
    args = ["--tier-state-driver", str(wasm), case, mode,
            "all-phase3-core", str(fuel), "none"]
    if register_entry_threshold is not None and mode == "register":
        args.extend(["--register-entry-threshold", str(register_entry_threshold)])
    value = _run_json(binary, args, runner)
    _check_register_entry_threshold(
        value, register_entry_threshold if mode == "register" else None,
        f"{case} ({mode}, fuel={fuel})",
    )
    policy = {"reference": "Reference", "decoded": "Decoded", "portable": "Portable",
              "compact": "Compact", "register": "Register"}.get(mode)
    outcome = value.get("outcome")
    if (policy is None or value.get("schema_version") != 1 or value.get("diagnostic") is not True
            or value.get("instrumented") is not False or value.get("case") != case
            or value.get("mode") != mode or value.get("resolved_policy") != policy
            or value.get("fuel_budget") != fuel
            or outcome not in {"returned", "Trap", "ResourceLimit"}):
        raise TierCheckError(f"{case} ({mode}, fuel={fuel}): malformed fuel-boundary observation")
    memory_hex = value.get("memory_hex")
    if not isinstance(memory_hex, str) or len(memory_hex) != 131072:
        raise TierCheckError(f"{case} ({mode}, fuel={fuel}): missing full memory image")
    try:
        memory = bytes.fromhex(memory_hex)
    except ValueError as exc:
        raise TierCheckError(f"{case} ({mode}, fuel={fuel}): malformed memory image") from exc
    if value.get("cursor") is not None:
        raise TierCheckError(f"{case} ({mode}, fuel={fuel}): unexpected cursor for PC=none")
    return {"case": case, "mode": mode,
            "register_entry_threshold": register_entry_threshold if mode == "register" else None,
            "fuel": fuel, "outcome": outcome,
            "trap": value.get("trap"), "values": value.get("values"),
            "memory_prefix_hex": memory[:8].hex(),
            "memory_sha256": hashlib.sha256(memory).hexdigest(),
            "global": value.get("global"), "table": value.get("table"),
            "root": value.get("root")}


def _state_cursor(binary: Path, case: str, runner: Callable[..., Any]) -> dict[str, Any]:
    wasm = STATE_WASM_DIR / f"{case}.wasm"
    value = _run_json(binary, ["--tier-state-driver", str(wasm), case, "decoded",
                               "all-phase3-core", str(STATE_FUEL), "3"], runner)
    cursor = value.get("cursor")
    if (value.get("schema_version") != 1 or value.get("diagnostic") is not True
            or value.get("instrumented") is not False or value.get("case") != case
            or value.get("mode") != "decoded" or value.get("outcome") != "returned"
            or value.get("values") != ["i32:42"] or value.get("fuel_budget") != STATE_FUEL
            or not isinstance(cursor, dict) or cursor.get("source_pc") != 3
            or type(cursor.get("prepared_pc")) is not int
            or cursor.get("charged") is not True
            or not isinstance(cursor.get("state"), str) or not cursor["state"]
            or type(cursor.get("remaining_fuel")) is not int):
        raise TierCheckError(f"{case}: decoded PC=3 cursor snapshot is missing or malformed")
    return {"case": case, "mode": "decoded", "requested_source_pc": 3,
            "cursor": cursor}


def check_tier(*, tier: str = "decoded", selection: str = "integration",
               output: Path, binary: Path | None = None,
               conformance_binary: Path | None = None,
               diagnostic_binary: Path | None = None,
               runner: Callable[..., Any] = subprocess.run,
               builder: Callable[..., Any] = subprocess.run,
               register_entry_threshold: int | None = None) -> dict[str, Any]:
    if not tier or tier == "reference":
        raise TierCheckError("--tier must name a candidate mode; reference is the comparison baseline")
    if selection != "integration":
        raise TierCheckError("only the declared integration selection is supported")
    if register_entry_threshold is not None:
        if (type(register_entry_threshold) is not int
                or not 1 <= register_entry_threshold <= MAX_REGISTER_ENTRY_THRESHOLD):
            raise TierCheckError("register entry threshold must be a positive u64 integer")
        if tier != "register":
            raise TierCheckError("register entry threshold requires --tier register")
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise TierCheckError(f"refusing to overwrite existing output directory: {output}")
    output.mkdir(parents=True, exist_ok=False)
    build_info = None
    auto_build = binary is None
    binary = (binary or DEFAULT_BINARY).resolve()
    conformance_binary = (conformance_binary or DEFAULT_CONFORMANCE).resolve()
    diagnostic_binary = (diagnostic_binary or DEFAULT_COUNTERS_BINARY).resolve()
    if auto_build:
        build_info = _build_reference(binary, builder, tier=tier)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise TierCheckError(f"benchmark executable is missing or not executable: {binary}")
    if not conformance_binary.is_file() or not os.access(conformance_binary, os.X_OK):
        raise TierCheckError(f"instrumented conformance executable is missing: {conformance_binary}")
    if not diagnostic_binary.is_file() or not os.access(diagnostic_binary, os.X_OK):
        raise TierCheckError(f"performance-counter diagnostic executable is missing: {diagnostic_binary}")
    cases = _integration_cases()
    source_before = _source_identity()
    corpus_sources_before: dict[str, str] = {}
    binary_before = _sha(binary)
    conformance_binary_before = _sha(conformance_binary)
    diagnostic_binary_before = _sha(diagnostic_binary)
    fixture_hashes = {
        "workloads.json": _sha(REGISTRY),
        "state_expectations.json": _sha(STATE_MANIFEST),
        "p018/store_successor.wat": _sha(SNAPSHOT_WAT),
        "p018/store_successor.wasm": _sha(SNAPSHOT_WASM),
    }
    state_expectations = _state_expected()
    state_fixtures = _state_fixture_hashes(state_expectations)
    fixture_identities_before = _fixture_identities(cases, state_fixtures)
    fixture_hashes["state_fixtures_sha256"] = hashlib.sha256(
        json.dumps(state_fixtures, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    setup_errors: list[str] = []
    runs: list[dict[str, Any]] = []
    conformance_runs: list[dict[str, Any]] = []
    state_runs: list[dict[str, Any]] = []
    state_fuel_runs: list[dict[str, Any]] = []
    fuel_boundaries: dict[str, Any] = {}
    cursor_checks: list[dict[str, Any]] = []
    try:
        protocol = _protocol(binary, runner)
        if tier not in protocol["modes"]:
            raise TierCheckError(f"requested tier {tier!r} is absent from --protocol modes")
    except TierCheckError as exc:
        protocol = {}
        setup_errors.append(str(exc))

    for case in cases:
        item: dict[str, Any] = {"id": case["id"], "input": case["input"],
                                "expected": case["expected"], "oracle": case["oracle"],
                                "fixture": case["fixture"], "runs": {}, "equal": False}
        for mode in ("reference", tier):
            try:
                threshold = register_entry_threshold if mode == tier and tier == "register" else None
                mode_result = _case_run(binary, case, mode, runner, threshold)
                mode_result["binary_sha256"] = binary_before
                item["runs"][mode] = mode_result
            except TierCheckError as exc:
                setup_errors.append(f"{case['id']} ({mode}): {exc}")
        if "reference" in item["runs"] and tier in item["runs"]:
            item["equal"] = item["runs"]["reference"]["result"] == item["runs"][tier]["result"]
            if not item["equal"]:
                setup_errors.append(f"{case['id']}: reference and {tier} results differ")
        runs.append(item)

    for suite in CONFORMANCE_SUITES:
            suite_runs = []
            for mode in ("reference", tier):
                try:
                    item = _conformance_run(
                        conformance_binary, suite, mode,
                        output / "conformance" / f"{suite}-{mode}.json", runner,
                        register_entry_threshold if mode == tier and tier == "register" else None,
                    )
                    suite_runs.append(item)
                    conformance_runs.append(item)
                    for path, digest in item["corpus_sources"].items():
                        previous = corpus_sources_before.setdefault(path, digest)
                        if previous != digest:
                            setup_errors.append(f"conformance source hash drifted during run: {path}")
                except TierCheckError as exc:
                    setup_errors.append(str(exc))
            if len(suite_runs) == 2:
                left, right = (entry["report"] for entry in suite_runs)
                compare_fields = ("selected", "executed", "failed", "skipped", "unsupported",
                                  "compatibility_assertions", "errors")
                if any(left.get(field) != right.get(field) for field in compare_fields):
                    setup_errors.append(f"{suite}: reference and {tier} conformance outcomes differ")
                if left.get("trap_codes") != right.get("trap_codes"):
                    setup_errors.append(f"{suite}: reference and {tier} TrapCode outcomes differ")

    for case in STATE_CASES:
            case_runs = []
            for mode in ("reference", tier):
                try:
                    threshold = register_entry_threshold if mode == tier and tier == "register" else None
                    item = _state_run(binary, case, mode,
                                      state_expectations["cases"][case], runner, threshold)
                    case_runs.append(item)
                    state_runs.append(item)
                except TierCheckError as exc:
                    setup_errors.append(str(exc))
            if len(case_runs) == 2:
                comparable = [{key: value for key, value in result.items()
                               if key not in {"mode", "binary_sha256", "register_entry_threshold"}}
                              for result in case_runs]
                if comparable[0] != comparable[1]:
                    setup_errors.append(f"{case}: reference and {tier} full-state observations differ")

    for case in ("state_normal", "state_trap"):
            case_observations: dict[str, list[dict[str, Any]]] = {"reference": [], tier: []}
            for fuel in range(17):
                pair = []
                for mode in ("reference", tier):
                    try:
                        threshold = register_entry_threshold if mode == tier and tier == "register" else None
                        result = _state_fuel_probe(binary, case, mode, fuel, runner, threshold)
                        pair.append(result)
                        case_observations[mode].append(result)
                        state_fuel_runs.append(result)
                    except TierCheckError as exc:
                        setup_errors.append(str(exc))
                if len(pair) == 2:
                    fields = ("outcome", "trap", "values", "memory_prefix_hex", "global", "table", "root")
                    if any(pair[0].get(field) != pair[1].get(field) for field in fields):
                        setup_errors.append(f"{case} fuel={fuel}: reference and {tier} state prefixes differ")
            if case == "state_normal":
                successes = {mode: next((row["fuel"] for row in rows if row["outcome"] == "returned"), None)
                             for mode, rows in case_observations.items()}
                if any(value is None for value in successes.values()):
                    setup_errors.append("state_normal: no successful execution in fuel range 0..16")
                elif len(set(successes.values())) != 1:
                    setup_errors.append(f"state_normal: minimal success fuel differs by mode: {successes}")
                previous = {mode: (None if fuel == 0 else
                                   next((row["outcome"] for row in case_observations[mode]
                                         if row["fuel"] == fuel - 1), None))
                            for mode, fuel in successes.items() if fuel is not None}
                if any(outcome == "returned" for outcome in previous.values()):
                    setup_errors.append("state_normal: minimum-success boundary was not minimal")
                fuel_boundaries[case] = {"minimal_success_fuel": successes,
                                         "previous_fuel_outcome": previous,
                                         "boundary_verified": bool(successes) and all(
                                             fuel == 0 or previous.get(mode) != "returned"
                                             for mode, fuel in successes.items()),
                                         "max_probe_fuel": 16}
            else:
                expected_trap = state_expectations["cases"][case]["expected_outcome"].split(":", 1)[1]
                trap_fuels = {mode: next((row["fuel"] for row in rows
                                          if row["outcome"] == "Trap" and row["trap"] == expected_trap), None)
                              for mode, rows in case_observations.items()}
                if any(value is None for value in trap_fuels.values()):
                    setup_errors.append("state_trap: expected TrapCode not reached in fuel range 0..16")
                elif len(set(trap_fuels.values())) != 1:
                    setup_errors.append(f"state_trap: trap admission boundary differs by mode: {trap_fuels}")
                fuel_boundaries[case] = {"minimal_expected_trap_fuel": trap_fuels,
                                         "trap_code": expected_trap, "max_probe_fuel": 16}

    try:
        cursor_checks.append(_state_cursor(binary, "state_normal", runner))
    except TierCheckError as exc:
        setup_errors.append(str(exc))

    snapshot: dict[str, Any] | None = None
    dispatch_diagnostics: list[dict[str, Any]] = []
    try:
        snapshot = _snapshot_run(binary, runner)
        snapshot["binary_sha256"] = binary_before
    except TierCheckError as exc:
        setup_errors.append(str(exc))

    for case in cases:
            try:
                diagnostic = _dispatch_diagnostic(diagnostic_binary, case, runner)
                diagnostic["binary_sha256"] = _sha(diagnostic_binary)
                dispatch_diagnostics.append(diagnostic)
            except TierCheckError as exc:
                setup_errors.append(str(exc))

    source_after = _source_identity()
    binary_after = _sha(binary)
    if source_before != source_after:
        changed = sorted(path for path in set(source_before) | set(source_after)
                         if source_before.get(path) != source_after.get(path))
        setup_errors.append(f"implementation sources changed during run: {changed[:8]}")
    if binary_before != binary_after:
        setup_errors.append("benchmark binary changed during run")
    if conformance_binary_before != _sha(conformance_binary):
        setup_errors.append("conformance binary changed during run")
    if diagnostic_binary_before != _sha(diagnostic_binary):
        setup_errors.append("diagnostic benchmark binary changed during run")
    corpus_sources_after: dict[str, str] = {}
    for path in corpus_sources_before:
        try:
            corpus_sources_after[path] = _sha(ROOT / path)
        except TierCheckError as exc:
            setup_errors.append(str(exc))
    if corpus_sources_before != corpus_sources_after:
        setup_errors.append("pinned conformance corpus sources changed during run")
    fixture_hashes_after = {
        "workloads.json": _sha(REGISTRY),
        "state_expectations.json": _sha(STATE_MANIFEST),
        "p018/store_successor.wat": _sha(SNAPSHOT_WAT),
        "p018/store_successor.wasm": _sha(SNAPSHOT_WASM),
    }
    current_state_fixtures = _state_fixture_hashes(_state_expected())
    fixture_identities_after = _fixture_identities(_integration_cases(), current_state_fixtures)
    fixture_hashes_after["state_fixtures_sha256"] = hashlib.sha256(
        json.dumps(current_state_fixtures, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    if fixture_hashes != fixture_hashes_after:
        setup_errors.append("workload or P018 fixture changed during run")
    if fixture_identities_before != fixture_identities_after:
        setup_errors.append("semantic, state, or P018 Wasm fixtures changed during run")

    executed = sum(len(item["runs"]) for item in runs)
    expected_runs = len(cases) * 2
    expected_conformance_runs = len(CONFORMANCE_SUITES) * 2
    expected_state_runs = len(STATE_CASES) * 2
    expected_fuel_runs = 2 * 17 * 2
    expected_checks = (expected_runs + expected_conformance_runs + expected_state_runs
                       + expected_fuel_runs + 1 + 1 + len(cases))
    successful_checks = (executed + len(conformance_runs) + len(state_runs)
                         + len(state_fuel_runs) + len(cursor_checks)
                         + int(bool(snapshot)) + len(dispatch_diagnostics))
    complete_work = (executed == expected_runs
                     and len(conformance_runs) == expected_conformance_runs
                     and len(state_runs) == expected_state_runs
                     and len(state_fuel_runs) == 68
                     and len(cursor_checks) == 1
                     and len(snapshot or {}) > 0
                     and len(dispatch_diagnostics) == len(cases))
    report = {
        "schema_version": 1,
        "passed": not setup_errors and complete_work and all(item["equal"] for item in runs),
        "source_stable": source_before == source_after,
        "selection": selection,
        "requested_tier": tier,
        "register_strategy": ("disabled" if tier != "register" else
                              "adaptive" if register_entry_threshold is not None else "eager"),
        "register_entry_threshold": register_entry_threshold,
        "timing_evidence": False,
        "build": build_info,
        "diagnostic_instrumentation": {
            "enabled": len(dispatch_diagnostics) == len(cases),
            "feature": "performance-counters",
            "binary": str(diagnostic_binary.relative_to(ROOT) if ROOT in diagnostic_binary.parents else diagnostic_binary),
            "binary_sha256": _sha(diagnostic_binary),
        },
        "conformance_binary": {"path": str(conformance_binary.relative_to(ROOT) if ROOT in conformance_binary.parents else conformance_binary),
                               "sha256": conformance_binary_before},
        "baseline_tier": "reference",
        "modes": protocol.get("modes", []),
        "instrumented": protocol.get("instrumented"),
        "counts": {"selected_cases": len(cases), "semantic_cases": len(cases),
                   "state_cases": len(STATE_CASES), "conformance_suites": len(CONFORMANCE_SUITES),
                   "mode_runs_expected": expected_runs, "mode_runs_executed": executed,
                   "state_mode_runs_expected": expected_state_runs,
                   "state_mode_runs_executed": len(state_runs),
                   "state_fuel_probes_expected": 68,
                   "state_fuel_probes_executed": len(state_fuel_runs),
                   "cursor_checks_executed": len(cursor_checks),
                   "conformance_mode_runs_expected": expected_conformance_runs,
                   "conformance_mode_runs_executed": len(conformance_runs),
                   "all_checks_expected": expected_checks,
                   "all_checks_attempted": expected_checks,
                   "all_checks_executed": successful_checks,
                   "failed": len(setup_errors), "skipped": 0, "unsupported": 0},
        "binary": {"path": binary.relative_to(ROOT).as_posix() if ROOT in binary.parents else str(binary),
                   "sha256": binary_before},
        "implementation_sources": source_before,
        "implementation_sources_sha256": hashlib.sha256(
            json.dumps(source_before, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        "fixtures": fixture_hashes,
        "fixture_identities_before": fixture_identities_before,
        "fixture_identities_after": fixture_identities_after,
        "conformance_sources_before": corpus_sources_before,
        "conformance_sources_after": corpus_sources_after,
        "state_fixture_identities": state_fixtures,
        "cases": runs,
        "conformance": conformance_runs,
        "state_checks": state_runs,
        "state_fuel_checks": state_fuel_runs,
        "fuel_boundaries": fuel_boundaries,
        "cursor_checks": cursor_checks,
        "snapshot": snapshot,
        "decoded_source_op_diagnostics": dispatch_diagnostics,
        "errors": setup_errors,
        "limitations": [
            "The reference dispatcher shares admission, frames, numeric/memory/GC/continuation helpers; it is not an independent oracle for bugs in shared helpers.",
            "P018 snapshots are bounded diagnostic views and are checked only in decoded mode; P022 supplies broader interaction-state fixtures.",
            "This is a correctness check. Execution timings in typed protocol output are diagnostic only and are not performance evidence.",
        ],
    }
    report_path = output / "report.json"
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


class _RegisterThresholdAction(argparse.Action):
    def __call__(self, parser, namespace, value, option_string=None):
        if getattr(namespace, self.dest, None) is not None:
            raise argparse.ArgumentError(self, "may only be specified once")
        setattr(namespace, self.dest, value)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", default="decoded", help="requested candidate execution mode")
    parser.add_argument("--selection", default="integration", choices=["integration"])
    parser.add_argument("--register-entry-threshold", type=int, action=_RegisterThresholdAction,
                        help="use the optional adaptive Register policy at this positive u64 threshold")
    parser.add_argument("--output", type=Path, required=True, help="directory for report.json")
    parser.add_argument("--binary", type=Path,
                        help="use an existing mode-aware benchmark executable (default: build with execution-reference)")
    parser.add_argument("--conformance-binary", type=Path,
                        help=f"use an existing instrumented conformance runner (default: {DEFAULT_CONFORMANCE})")
    parser.add_argument("--diagnostic-binary", type=Path,
                        help=f"use a separate performance-counters benchmark (default: {DEFAULT_COUNTERS_BINARY})")
    args = parser.parse_args(argv)
    try:
        report = check_tier(tier=args.tier, selection=args.selection,
                            output=args.output, binary=args.binary,
                            conformance_binary=args.conformance_binary,
                            diagnostic_binary=args.diagnostic_binary,
                            register_entry_threshold=args.register_entry_threshold)
    except TierCheckError as exc:
        print(f"tier_check: error: {exc}", file=sys.stderr)
        return 2
    counts = report["counts"]
    print("passed={} selected={} executed={} failed={} skipped={} unsupported={}".format(
        str(report["passed"]).lower(), counts["selected_cases"], counts["mode_runs_executed"],
        counts["failed"], counts["skipped"], counts["unsupported"],
    ))
    print(f"report={args.output / 'report.json'}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
