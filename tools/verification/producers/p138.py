#!/usr/bin/env python3
"""Freeze the P138 Register predecessor and validate cache diagnostic traces."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
PREDECESSOR = "target/performance/P138-before"
PREDECESSOR_COMMIT = "78c581a72433fb44b8d135eb0b9971a0eab6bd73"
PREDECESSOR_MANIFEST_SHA256 = "a70e79157fe54adc7d4a8413f73223fb4eea6c6532a245810fae59d235f2eba1"
P137_RECEIPT = ROOT / "target/performance/P137/shared-finalization/attempt-0002/evidence.json"
P137_PROOF_REPORT = ROOT / "target/verification/runs/P137/verify/attempt-0002/proof/report.json"
FIXTURE_DIR = ROOT / "crates/wtime-runtime/tests/fixtures/performance/p138"
PROVENANCE = FIXTURE_DIR / "provenance.json"
CASE_NAMES = (
    "p138_cold_threshold_and_single_builder_are_observed",
    "p138_guest_fifo_eviction_and_recompilation_stay_bounded",
    "p138_snapshot_preserves_all_fuel_prefixes_and_wraparound_results",
    "p138_private_snapshots_do_not_pin_evicted_artifacts",
    "p138_private_failures_cooldown_abort_poison_and_foreign_are_observed",
    "p138_private_active_builder_snapshot_and_saturating_counters",
)
LIFECYCLE_FIELDS = {
    "preparation_starts", "builder_starts", "completed", "published",
    "publication_failures", "declined", "exhausted", "invalid", "aborted", "evictions",
}
TRACE_FIELDS = {
    "case", "event", "instrumented", "timing_evidence", "engine", "threshold",
    "entry_limit", "active_body", "remaining_attempted_work", "remaining_retained_work",
    "retained_bytes", "byte_limit", "lifecycle", "rows",
}
ROW_FIELDS = {"body", "entries", "attempts", "state", "cooldown", "ready_bytes"}
FORBIDDEN_TIME_PARTS = ("time", "duration", "elapsed", "nanosecond", "compile_ns", "promotion")
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
P133_CASES = (
    "p133_cold_threshold_and_single_publish",
    "p133_concurrent_duplicate_uses_one_unlocked_builder",
    "p133_decline_poison_foreign_and_aborted_lease_fail_closed",
    "p133_fifo_eviction_pinned_lifetime_and_budget_churn",
)
P134_CASES = (
    "p134_cold_hot_entry_threshold_and_diagnostics",
    "p134_shared_module_concurrency_and_store_ownership",
    "p134_host_effect_fuel_and_foreign_refusal",
    "p134_eviction_pin_and_continuation_resume_without_reentry",
    "p134_activation_pin_survives_eviction",
)
P065_CASES = (
    "p065_outer_reference_survives_reentrant_collection",
    "p065_capture_retains_numeric_state_and_source_cursor",
    "p065_cross_module_exact_suspend_and_resource_controls",
    "p065_reference_and_vector_frames_decline",
    "p065_exception_unwind_preserves_numeric_payload",
    "p065_root_barrier_recipes_are_complete",
)
FULL_CASES = P133_CASES + P134_CASES + P065_CASES


class P138Error(ValueError):
    """P138 inputs or diagnostic evidence are invalid or stale."""


def need(ok: bool, message: str) -> None:
    if not ok:
        raise P138Error(message)


def integer(value: Any, label: str, *, minimum: int = 0) -> int:
    need(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    return value


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode("utf-8")).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def under_verification(path: Path, label: str) -> Path:
    path = Path(path).resolve(strict=True)
    store = (ROOT / "target/verification").resolve(strict=True)
    need(path.is_relative_to(store), f"{label} must stay under target/verification")
    return path


def read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P138Error(f"{label} is invalid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path, phase: str) -> Path:
    path = Path(path).resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P138 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    (path / "helpers").mkdir()
    for helper in (Path(__file__), ROOT / "tools/wtime-bench/baseline.py",
                   ROOT / "tools/checks/build_budget.py", ROOT / "tools/checks/dependency_budget.py"):
        shutil.copyfile(helper, path / "helpers" / helper.name)
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve()),
                                          "phase": phase})
    return path


def runtime_source_map(manifest: dict[str, Any]) -> dict[str, str]:
    prefixes = tuple(f"crates/{crate}/" for crate in CORE_CRATES)
    result: dict[str, str] = {}
    for row in manifest["files"].values():
        name = row.get("source_path", "")
        if row.get("role") == "source" and (
            name in ROOT_SOURCES
            or any(name == f"crates/{crate}/Cargo.toml" for crate in CORE_CRATES)
            or (name.endswith(".rs") and name.startswith(prefixes))
        ):
            result[name] = row["sha256"]
    return dict(sorted(result.items()))


def current_runtime_source_map() -> dict[str, str]:
    names = set(ROOT_SOURCES)
    for crate in CORE_CRATES:
        base = ROOT / "crates" / crate
        names.add(f"crates/{crate}/Cargo.toml")
        names.update(path.relative_to(ROOT).as_posix() for path in base.rglob("*.rs") if path.is_file())
    return {name: sha(ROOT / name) for name in sorted(names) if (ROOT / name).is_file()}


def fixture_identity() -> dict[str, Any]:
    provenance = read_json(PROVENANCE, "P138 fixture provenance")
    fixture = provenance.get("fixture", {})
    need(provenance.get("schema") == "wtime-p138-fixture-provenance-v1"
         and provenance.get("license") == "MIT"
         and provenance.get("validation", {}).get("status") == "passed",
         "P138 fixture provenance or MVP validation is invalid")
    need(fixture.get("name") == "uniform66" and fixture.get("function_count") == 66
         and fixture.get("features") == "MVP" and fixture.get("imports") == 0
         and fixture.get("signature") == "(i32)->i32"
         and fixture.get("source_events_per_call_including_end") == 4
         and fixture.get("body") == ["local.get 0", "i32.const INDEX", "i32.add", "end (implicit)"]
         and fixture.get("memories") == 0 and fixture.get("globals") == 0
         and fixture.get("tables") == 0 and fixture.get("bulk_operations") == 0
         and len(fixture.get("exports", [])) == 66
         and fixture["exports"] == [f"f{i:02}" for i in range(66)],
         "P138 fixture shape differs from the uniform66 oracle")
    archive = (ROOT / provenance.get("archive", "")).resolve(strict=True)
    need(archive.is_relative_to((ROOT / "target/recovery").resolve()),
         "P138 fixture archive must remain under target/recovery")
    generator = (ROOT / provenance.get("generator", "")).resolve(strict=True)
    need(generator.is_relative_to(archive)
         and sha(generator) == provenance.get("generator_sha256"),
         "P138 archived fixture generator is missing or stale")
    paths = {
        "provenance": PROVENANCE,
        "license": FIXTURE_DIR / "LICENSE.txt",
        "wat": FIXTURE_DIR / "uniform66.wat",
        "wasm": FIXTURE_DIR / "uniform66.wasm",
    }
    for key, name in (("license", "license_sha256"), ("wat", "wat_sha256"), ("wasm", "wasm_sha256")):
        need(sha(paths[key]) == (provenance.get(name) if key == "license" else fixture.get(name)),
             f"P138 {key} fixture hash differs from provenance")
    validation_refs: dict[str, Any] = {}
    validation = provenance.get("validation", {})
    for validator in ("official_validator", "wtime_validator"):
        row = validation.get(validator, {})
        need(row.get("exit_code") == 0, f"P138 {validator} did not pass")
        for stream in ("stdout", "stderr"):
            relative = row.get(stream)
            output = (ROOT / relative).resolve(strict=True)
            need(output.is_relative_to(archive)
                 and sha(output) == row.get(f"{stream}_sha256"),
                 f"P138 archived {validator} {stream} does not match its provenance")
            validation_refs[f"{validator}_{stream}"] = ref(output)
    return {key: ref(path) for key, path in paths.items()} | {
        "generator": ref(generator), "archive": str(archive),
        "validation_outputs": validation_refs,
        "fixture_schema": provenance["schema"], "profile": "MVP", "function_count": 66,
        "exports": fixture["exports"], "source_events_per_call_including_end": 4,
        "oracle": fixture["oracle"],
    }


def validate_predecessor() -> dict[str, Any]:
    import importlib.util

    baseline_path = ROOT / "tools/wtime-bench/baseline.py"
    spec = importlib.util.spec_from_file_location("p138_baseline", baseline_path)
    need(spec is not None and spec.loader is not None, "cannot load immutable archive validator")
    baseline = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(baseline)
    archive = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = archive / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P138 predecessor manifest identity changed")
    manifest = baseline.verify_archive(archive)
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT
         and manifest.get("mode") == "register"
         and manifest.get("execution_policy", {}).get("resolved_mode") == "register",
         "P138 predecessor is not the pinned immediate Register archive")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P138 predecessor omits Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map
             and any(x.startswith(f"crates/{crate}/") and x.endswith(".rs") for x in source_map),
             f"P138 predecessor omits {crate} sources")

    completion_path = ROOT / "docs/benchmarks/performance/P137-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P137-evidence.json"
    proof_path = ROOT / "docs/benchmarks/performance/P137-proof.json"
    completion = read_json(completion_path, "P137 completion")
    evidence = read_json(evidence_path, "P137 evidence")
    proof = read_json(proof_path, "P137 proof")
    need(completion.get("task") == "P137" and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         "P137 is not complete as a contract-only prerequisite")
    need(evidence.get("task") == "P137" and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("candidate_promoted") is False
         and evidence.get("synthetic") is False and evidence.get("outcome") == "contract_only",
         "P137 durable evidence is missing, stale, or promoting")
    verification_contract = ROOT / "docs/benchmarks/performance/P137-verification.json"
    need(evidence.get("contract") == ref(verification_contract),
         "P137 evidence does not bind the exported verification contract")
    need(proof.get("schema") == "wtime-p137-verification-v1" and proof.get("status") == "passed"
         and proof.get("timing_evidence") is False and proof.get("gate_eligible") is False,
         "P137 source-bound proof is invalid")
    final_evidence = read_json(P137_RECEIPT, "P137 retained finalization receipt")
    retained_proof = read_json(P137_PROOF_REPORT, "P137 retained proof report")
    need(final_evidence == evidence and retained_proof == proof
         and completion.get("evidence") == [ref(P137_RECEIPT)]
         and completion.get("artifact_hashes") == {str(P137_RECEIPT.resolve()): sha(P137_RECEIPT)},
         "P137 exported completion does not bind the retained final proof/evidence")
    return {
        "archive": ref(manifest_path), "manifest_sha256_pinned": PREDECESSOR_MANIFEST_SHA256,
        "commit": PREDECESSOR_COMMIT, "mode": "register", "runtime_source_map": source_map,
        "runtime_source_map_sha256": canonical_sha(source_map),
        "p137": {"completion": ref(completion_path), "evidence": ref(evidence_path),
                 "proof": ref(proof_path), "final_receipt": ref(P137_RECEIPT),
                 "retained_proof": ref(P137_PROOF_REPORT),
                 "outcome": "contract_only", "passed": True},
    }


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output, "freeze")
    predecessor = validate_predecessor()
    fixture = fixture_identity()
    current = current_runtime_source_map()
    write_new(output / "runtime-source-map.json", {"source_map": current,
                                                     "source_map_sha256": canonical_sha(current)})
    report = {
        "schema": "wtime-p138-freeze-v1", "status": "passed", "outcome": "contract_only",
        "timing_evidence": False, "instrumented": True, "gate_eligible": False,
        "predecessor": predecessor, "fixture": fixture,
        "current_source_map_sha256": canonical_sha(current),
        "current_source_file_count": len(current),
        "runtime_source_map": ref(output / "runtime-source-map.json"),
        "no_build_or_guest_launched": True,
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$", text, re.MULTILINE),
             f"{label} log lacks passing named test {name}")


def _paths(paths: Path | list[Path]) -> list[Path]:
    return [Path(paths)] if isinstance(paths, (str, Path)) else [Path(path) for path in paths]


def _json_markers(paths: Path | list[Path], marker: str) -> list[dict[str, Any]]:
    result = []
    for path in _paths(paths):
        for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith(marker + " "):
                try:
                    item = json.loads(line[len(marker) + 1:])
                except json.JSONDecodeError as exc:
                    raise P138Error(f"malformed {marker} record: {exc}") from exc
                need(isinstance(item, dict), f"{marker} record must be an object")
                result.append(item)
    return result


def _reject_timing_fields(value: Any, where: str = "trace") -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            lowered = key.lower()
            need(key == "timing_evidence" or not any(part in lowered for part in FORBIDDEN_TIME_PARTS),
                 f"{where} contains forbidden timing/promotion field {key}")
            _reject_timing_fields(item, f"{where}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            _reject_timing_fields(item, f"{where}[{index}]")


def validate_trace(record: dict[str, Any]) -> None:
    need(set(record) == TRACE_FIELDS, "P138 trace schema fields differ from the declared snapshot schema")
    need(record.get("case") in CASE_NAMES and isinstance(record.get("event"), str)
         and record.get("event"), "P138 trace case or event identity is invalid")
    need(record.get("instrumented") is True and record.get("timing_evidence") is False,
         "P138 trace is not explicitly instrumented and non-timing")
    integer(record.get("engine"), "trace engine", minimum=1)
    integer(record.get("threshold"), "trace threshold")
    limit = integer(record.get("entry_limit"), "trace entry limit", minimum=1)
    need(limit <= 64, "P138 trace entry limit exceeds 64")
    active = record.get("active_body")
    need(active is None or (type(active) is int and 0 <= active < 66), "trace active body is invalid")
    remaining = integer(record.get("remaining_attempted_work"), "remaining attempted work")
    retained_work = integer(record.get("remaining_retained_work"), "remaining retained work")
    retained = integer(record.get("retained_bytes"), "retained bytes")
    byte_limit = integer(record.get("byte_limit"), "byte limit", minimum=1)
    need(retained <= byte_limit, "trace retained bytes exceed byte limit")
    need(remaining >= 0 and retained_work >= 0, "trace work budget is negative")
    lifecycle = record.get("lifecycle")
    need(isinstance(lifecycle, dict) and set(lifecycle) == LIFECYCLE_FIELDS,
         "P138 trace lifecycle fields differ from the declared schema")
    for name, count in lifecycle.items():
        integer(count, f"lifecycle.{name}")
    rows = record.get("rows")
    need(isinstance(rows, list) and len(rows) <= limit, "P138 trace rows exceed entry limit")
    bodies = []
    compiling = []
    for row in rows:
        need(isinstance(row, dict) and set(row) == ROW_FIELDS, "P138 trace row fields are malformed")
        body = integer(row.get("body"), "row body")
        need(body < 66, "P138 trace body index exceeds fixture")
        bodies.append(body)
        entries = integer(row.get("entries"), "row entry count", minimum=1)
        attempts = integer(row.get("attempts"), "row attempts")
        need(attempts <= entries, "P138 trace attempts exceed entry count")
        need(row.get("state") in ("cold", "compiling", "ready", "cooldown", "negative"),
             "P138 trace row state is unknown")
        cooldown = integer(row.get("cooldown"), "row cooldown")
        ready_bytes = integer(row.get("ready_bytes"), "row ready bytes")
        if row["state"] == "compiling":
            compiling.append(body)
        if row["state"] == "ready":
            need(ready_bytes > 0 and cooldown == 0,
                 "ready cache row must carry a positive artifact size and no cooldown")
        elif row["state"] == "cooldown":
            need(cooldown > 0 and ready_bytes == 0,
                 "cooldown cache row must expose remaining retries without artifact bytes")
        else:
            need(cooldown == 0 and ready_bytes == 0,
                 "non-ready cache row must not expose cooldown or artifact bytes")
    need(len(set(bodies)) == len(bodies), "P138 trace contains duplicate resident body identities")
    need((compiling == [active]) if active is not None else not compiling,
         "P138 active body does not match the single compiling cache row")
    _reject_timing_fields(record)


def validate_case_records(path: Path | list[Path]) -> dict[str, dict[str, Any]]:
    cases: dict[str, dict[str, Any]] = {}
    for record in _json_markers(path, "P138_CASE"):
        name = record.get("name")
        need(name in CASE_NAMES and name not in cases, f"unexpected or duplicate P138_CASE {name!r}")
        need(record.get("passed") is True and record.get("instrumented") is True
             and record.get("timing_evidence") is False,
             f"P138 case {name} failed or claimed timing evidence")
        _reject_timing_fields(record, f"case {name}")
        cases[name] = record
    need(set(cases) == set(CASE_NAMES), f"P138 case markers incomplete: {sorted(set(CASE_NAMES)-set(cases))}")
    expected = {
        CASE_NAMES[0]: {"calls": 5, "cold_calls": 2, "builders": 1},
        CASE_NAMES[1]: {"calls": 67, "distinct_bodies": 66, "builders": 67, "evictions": 3},
        CASE_NAMES[2]: {"fuel_prefixes": 5, "policies": 4},
        CASE_NAMES[3]: {"snapshot_clones_artifact": False, "work_refunded": False},
        CASE_NAMES[4]: {"cooldown_entries": 128, "declines": 2,
                        "abort_work_charged": True, "poisoned_snapshots_refused": 2,
                        "foreign_snapshot_refused": True},
        CASE_NAMES[5]: {"active_observed": True, "builder_unlocked": True,
                        "counters_saturate": True},
    }
    for name, fields in expected.items():
        for key, value in fields.items():
            need(cases[name].get(key) == value, f"P138 case {name} has wrong {key}")
    return cases


def validate_traces(path: Path | list[Path]) -> dict[str, Any]:
    records = _json_markers(path, "P138_TRACE")
    for record in records:
        validate_trace(record)
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in records:
        grouped.setdefault(row["case"], []).append(row)
    cold = CASE_NAMES[0]
    churn = CASE_NAMES[1]
    need(cold in grouped and churn in grouped, "P138 cold or FIFO trace sequence is absent")
    cold_rows = grouped[cold]
    events = [row["event"] for row in cold_rows]
    need(events == ["initial", "call-1", "call-2", "call-3", "call-4", "call-5"],
         "P138 cold trace sequence differs from the exercised call order")
    initial = cold_rows[0]
    need(initial["threshold"] == 3 and initial["rows"] == []
         and initial["lifecycle"]["builder_starts"] == 0,
         "P138 cold initial snapshot is not empty at threshold three")
    for index, row in enumerate(cold_rows[1:], start=1):
        need(row["active_body"] is None and len(row["rows"]) == 1,
             "P138 cold trace must identify one settled resident body")
        body = row["rows"][0]
        need(body["body"] == 0 and body["entries"] == index
             and body["attempts"] == int(index >= 3),
             "P138 cold trace body identity, entries, or attempts differ")
        expected_state = "cold" if index < 3 else "ready"
        need(body["state"] == expected_state, "P138 cold trace state changed unexpectedly")
        need(row["lifecycle"]["preparation_starts"] == int(index >= 3)
             and row["lifecycle"]["builder_starts"] == int(index >= 3)
             and row["lifecycle"]["completed"] == int(index >= 3)
             and row["lifecycle"]["published"] == int(index >= 3),
             "P138 cold trace builder/publication transition differs")

    fifo = grouped[churn]
    need(len(fifo) == 68 and fifo[0]["event"] == "initial",
         "P138 FIFO trace must include initial state and 67 body calls")
    prev_remaining = fifo[0]["remaining_attempted_work"]
    prev_retained_work = fifo[0]["remaining_retained_work"]
    for index, row in enumerate(fifo[1:], start=1):
        body_index = 0 if index == 67 else index - 1
        need(row["event"] == f"call-{index}-body-{body_index}" and row["threshold"] == 1,
             "P138 FIFO trace event order or threshold changed")
        need(row["active_body"] is None and row["lifecycle"]["preparation_starts"] == index
             and row["lifecycle"]["builder_starts"] == index
             and row["lifecycle"]["completed"] == index
             and row["lifecycle"]["published"] == index
             and row["lifecycle"]["evictions"] == max(0, index - 64),
             "P138 FIFO lifecycle counts differ from bounded body insertion")
        bodies = [item["body"] for item in row["rows"]]
        expected_bodies = list(range(max(0, index - 64), min(index, 66)))
        if index == 67:
            expected_bodies.append(0)
        need(bodies == expected_bodies and len(bodies) <= 64,
             "P138 FIFO resident body order differs from insertion-order eviction")
        need(all(item["state"] == "ready" for item in row["rows"]),
             "P138 threshold-one body was not ready after invocation")
        need(row["remaining_attempted_work"] <= prev_remaining
             and row["remaining_retained_work"] <= prev_retained_work,
             "P138 FIFO budgets increased after preparation")
        prev_remaining, prev_retained_work = row["remaining_attempted_work"], row["remaining_retained_work"]

    pinned_case = CASE_NAMES[3]
    need(pinned_case in grouped, "P138 pinned artifact trace is absent")
    pinned = grouped[pinned_case]
    need([row["event"] for row in pinned] == ["pinned", "evicted-pinned", "released"],
         "P138 pinned artifact trace sequence differs")
    first, evicted, released = pinned
    need(len(first["rows"]) == 1 and first["rows"][0]["body"] == 0
         and first["rows"][0]["state"] == "ready" and first["rows"][0]["ready_bytes"] > 0,
         "P138 pinned artifact trace lacks its ready artifact identity")
    need([row["body"] for row in evicted["rows"]] == [1, 2]
         and evicted["lifecycle"]["evictions"] == 1
         and evicted["retained_bytes"] >= first["retained_bytes"],
         "P138 eviction did not retain the externally pinned artifact charge")
    need([row["body"] for row in released["rows"]] == [1, 2]
         and released["retained_bytes"] + first["rows"][0]["ready_bytes"] == evicted["retained_bytes"]
         and released["lifecycle"] == evicted["lifecycle"]
         and released["remaining_attempted_work"] == evicted["remaining_attempted_work"]
         and released["remaining_retained_work"] == evicted["remaining_retained_work"],
         "P138 artifact release refunded work or changed lifecycle state")

    failure_case = CASE_NAMES[4]
    need(failure_case in grouped, "P138 decline/poison/foreign trace is absent")
    failures = grouped[failure_case]
    need([row["event"] for row in failures] == [
        "decline-first", "decline-cooldown", "decline-negative", "aborted",
        "foreign-publication-refused", "invalid", "exhausted-work", "exhausted-wrapper",
    ], "P138 failure trace sequence differs")
    def only_row(event: str) -> tuple[dict[str, Any], dict[str, Any]]:
        record = next(item for item in failures if item["event"] == event)
        need(len(record["rows"]) == 1, f"P138 {event} must have one cache row")
        return record, record["rows"][0]
    record, row = only_row("decline-first")
    need(row["state"] == "cooldown" and row["cooldown"] == 128
         and record["lifecycle"]["declined"] == 1 and record["lifecycle"]["builder_starts"] == 1,
         "P138 initial decline/cooldown transition differs")
    record, row = only_row("decline-cooldown")
    need(row["state"] == "cooldown" and row["cooldown"] == 1
         and record["lifecycle"]["declined"] == 1,
         "P138 cooldown countdown or retry suppression differs")
    record, row = only_row("decline-negative")
    need(row["state"] == "negative" and row["attempts"] == 2
         and record["lifecycle"]["declined"] == 2,
         "P138 terminal decline state or retry count differs")
    record, row = only_row("aborted")
    need(record["active_body"] is None and row["state"] == "negative"
         and record["lifecycle"]["aborted"] == 1,
         "P138 aborted preparation was not settled and charged")
    record, row = only_row("foreign-publication-refused")
    need(row["state"] == "negative" and record["lifecycle"]["completed"] == 1
         and record["lifecycle"]["published"] == 0
         and record["lifecycle"]["publication_failures"] == 1,
         "P138 foreign artifact publication was not refused")
    record, row = only_row("invalid")
    need(row["state"] == "negative" and record["lifecycle"]["invalid"] == 1,
         "P138 invalid preparation classification differs")
    record, row = only_row("exhausted-work")
    need(row["state"] == "negative" and record["lifecycle"]["exhausted"] == 1
         and record["lifecycle"]["builder_starts"] == 1,
         "P138 exhausted builder classification differs")
    record, row = only_row("exhausted-wrapper")
    need(record["lifecycle"]["preparation_starts"] == 1
         and record["lifecycle"]["builder_starts"] == 0
         and record["lifecycle"]["exhausted"] == 1,
         "P138 wrapper exhaustion must not invoke the builder")

    active_case = CASE_NAMES[5]
    need(active_case in grouped, "P138 active builder trace is absent")
    active_rows = grouped[active_case]
    need([row["event"] for row in active_rows] == ["active", "published", "saturated"],
         "P138 active builder trace sequence differs")
    active, compiling = active_rows[0], active_rows[0]["rows"]
    need(active["active_body"] == 0 and len(compiling) == 1 and compiling[0]["body"] == 0
         and compiling[0]["state"] == "compiling"
         and active["lifecycle"]["preparation_starts"] == 1
         and active["lifecycle"]["builder_starts"] == 1
         and active["lifecycle"]["completed"] == 0,
         "P138 active builder snapshot does not expose the compiling transition")
    published = active_rows[1]
    need(published["active_body"] is None and published["lifecycle"]["completed"] == 1
         and published["lifecycle"]["published"] == 1
         and len(published["rows"]) == 1 and published["rows"][0]["state"] == "ready",
         "P138 post-builder snapshot does not expose publication")
    saturated = active_rows[2]
    need(all(value == (1 << 64) - 1 for value in saturated["lifecycle"].values()),
         "P138 lifecycle aggregates did not saturate at u64::MAX")

    expected_cases = {CASE_NAMES[0], CASE_NAMES[1], CASE_NAMES[3], CASE_NAMES[4], CASE_NAMES[5]}
    need(set(grouped) == expected_cases, "P138 trace cases differ from the expected semantic cases")
    return {"trace_count": len(records), "case_count": len(grouped),
            "per_case": {name: len(rows) for name, rows in sorted(grouped.items())},
            "trace_sha256": canonical_sha([sha(item) for item in _paths(path)])}


def _current_fixture_refs() -> dict[str, Any]:
    return fixture_identity()


def _build_budget_module():
    import importlib.util

    helper_path = ROOT / "tools/checks/build_budget.py"
    spec = importlib.util.spec_from_file_location("p138_build_budget", helper_path)
    need(spec is not None and spec.loader is not None, "cannot load build-budget verifier")
    helper = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(helper)
    return helper


def check_build(path: Path, mode: str, features: list[str]) -> dict[str, Any]:
    record = read_json(path, f"{mode} build budget")
    need(record.get("schema_version") == 2 and record.get("mode") == mode
         and record.get("features") == features,
         f"{mode} build budget feature configuration differs")
    need(record.get("source_stable") is True and record.get("budget_pass") is True
         and record.get("default_dependency_closure_pass") is True
         and record.get("default_dependency_violations") == [],
         f"{mode} build budget failed source stability or dependency policy")
    source_inputs = record.get("source_inputs")
    need(isinstance(source_inputs, dict) and source_inputs == record.get("source_inputs_after")
         == record.get("isolated_source_inputs_after"),
         f"{mode} build budget source input snapshots differ")
    helper = _build_budget_module()
    feature_arg = "wtime-runtime/register-executor" if features else None
    metadata = helper.cargo_metadata(features=feature_arg)
    current_inputs = helper.source_inputs(metadata)
    source_hash = helper.hash_inputs(current_inputs)
    need(current_inputs == source_inputs and record.get("source_sha256") == source_hash
         and record.get("source_sha256_after") == source_hash
         and record.get("isolated_workspace_source_sha256") == source_hash,
         f"{mode} build budget does not bind current workspace compilation inputs")
    need(helper.check_budget(record) is True,
         f"{mode} build budget fails the shared hard-budget checker")
    size = integer(record.get("stripped_bytes"), f"{mode} stripped build size", minimum=1)
    return {"artifact": ref(path), "mode": mode, "features": features, "stripped_bytes": size,
            "source_sha256": source_hash, "source_file_count": len(source_inputs),
            "source_stable": True, "budget_pass": True,
            "default_dependency_closure_pass": True}


def verify(output: Path, freeze_report: Path, p138_log: Path, p138_stderr: Path,
           register_log: Path, combined_log: Path, default_log: Path,
           default_build: Path, register_build: Path) -> dict[str, Any]:
    output = start_output(output, "verify")
    frozen = read_json(freeze_report, "P138 freeze report")
    need(frozen.get("schema") == "wtime-p138-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("outcome") == "contract_only" and frozen.get("timing_evidence") is False
         and frozen.get("instrumented") is True and frozen.get("gate_eligible") is False,
         "P138 freeze report is invalid or promoting")
    predecessor = validate_predecessor()
    fixture = _current_fixture_refs()
    need(frozen.get("predecessor") == predecessor and frozen.get("fixture") == fixture,
         "P138 predecessor or fixture identity changed after freeze")
    current = current_runtime_source_map()
    need(frozen.get("current_source_map_sha256") == canonical_sha(current)
         and frozen.get("current_source_file_count") == len(current),
         "P138 runtime source map changed after freeze")
    map_ref = frozen.get("runtime_source_map")
    map_path = Path(map_ref.get("path", "")).resolve(strict=True)
    need(map_path.is_relative_to(ROOT / "target/verification") and map_ref == ref(map_path)
         and read_json(map_path, "P138 frozen runtime source map").get("source_map") == current,
         "P138 frozen source-map artifact is invalid")

    p138_log, p138_stderr = Path(p138_log).resolve(strict=True), Path(p138_stderr).resolve(strict=True)
    register_log, combined_log, default_log = map(lambda p: Path(p).resolve(strict=True),
                                                  (register_log, combined_log, default_log))
    for label, path in (("P138 stdout", p138_log), ("P138 trace stderr", p138_stderr),
                        ("Register stdout", register_log), ("combined stdout", combined_log),
                        ("default stdout", default_log), ("default build report", Path(default_build)),
                        ("Register build report", Path(register_build))):
        under_verification(path, label)
    require_tests(p138_log, CASE_NAMES, "P138")
    require_tests(register_log, FULL_CASES + CASE_NAMES, "Register/reference")
    require_tests(combined_log, FULL_CASES + CASE_NAMES, "combined feature")
    require_tests(default_log, ("p132_module_preparation_remains_policy_neutral",), "default")
    cases = validate_case_records(p138_stderr)
    traces = validate_traces(p138_stderr)
    builds = {"default": check_build(default_build, "default", []),
              "register": check_build(register_build, "optional-feature", ["wtime-runtime/register-executor"])}
    helper_sources = {
        "p138.py": Path(__file__), "baseline.py": ROOT / "tools/wtime-bench/baseline.py",
        "build_budget.py": ROOT / "tools/checks/build_budget.py",
        "dependency_budget.py": ROOT / "tools/checks/dependency_budget.py",
    }
    helper_refs = {name: ref(output / "helpers" / name) for name in helper_sources}
    for name, source in helper_sources.items():
        need(sha(output / "helpers" / name) == sha(source),
             f"retained P138 helper {name} differs from its source")
    after = current_runtime_source_map()
    need(after == current, "P138 runtime sources changed during validation")
    report = {
        "schema": "wtime-p138-verification-v1", "status": "passed", "outcome": "contract_only",
        "decision": None, "instrumented": True, "timing_evidence": False, "gate_eligible": False,
        "candidate_promoted": False, "benchmark_samples": 0, "threshold_selection": None,
        "fixture": fixture, "predecessor": predecessor,
        "freeze_report": ref(Path(freeze_report)),
        "producer_helpers": helper_refs,
        "runtime_source_map_sha256": canonical_sha(current), "runtime_source_file_count": len(current),
        "case_records": cases, "trace_summary": traces,
        "counts": {"p138_named_cases": len(cases), "p138_trace_records": traces["trace_count"],
                   "p133_regression_named_cases": len(P133_CASES),
                   "p134_regression_named_cases": len(P134_CASES),
                   "p065_regression_named_cases": len(P065_CASES)},
        "test_logs": {"p138": ref(p138_log), "p138_stderr": ref(p138_stderr),
                      "register": ref(register_log), "combined": ref(combined_log),
                      "default": ref(default_log)},
        "build_budgets": builds,
        "nonpromotion": {"instrumented": True, "timing_evidence": False,
                         "gate_eligible": False, "candidate_promoted": False},
    }
    write_new(output / "report.json", report)
    return report


def verify_evidence(path: Path) -> dict[str, Any]:
    """Revalidate the retained report, source map, fixture, test logs and build reports."""
    report_path = Path(path).resolve(strict=True)
    report = read_json(report_path, "P138 evidence report")
    need(report.get("schema") == "wtime-p138-verification-v1" and report.get("status") == "passed"
         and report.get("outcome") == "contract_only" and report.get("decision") is None
         and report.get("instrumented") is True and report.get("timing_evidence") is False
         and report.get("gate_eligible") is False and report.get("candidate_promoted") is False
         and report.get("benchmark_samples") == 0 and report.get("threshold_selection") is None,
         "P138 evidence envelope is invalid or promoting")
    freeze_ref = report.get("freeze_report")
    need(isinstance(freeze_ref, dict) and freeze_ref == ref(Path(freeze_ref.get("path", "")))
         and Path(freeze_ref["path"]).is_relative_to(ROOT / "target/verification"),
         "P138 freeze report reference is stale or outside verification storage")
    frozen = read_json(Path(freeze_ref["path"]), "P138 freeze report")
    frozen_map_ref = frozen.get("runtime_source_map")
    need(isinstance(frozen_map_ref, dict)
         and frozen_map_ref == ref(Path(frozen_map_ref.get("path", "")))
         and Path(frozen_map_ref["path"]).is_relative_to(ROOT / "target/verification"),
         "P138 frozen source map reference is stale or outside verification storage")
    frozen_map = read_json(Path(frozen_map_ref["path"]), "P138 frozen runtime source map")
    need(frozen.get("schema") == "wtime-p138-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("outcome") == "contract_only" and frozen.get("timing_evidence") is False
         and frozen.get("instrumented") is True and frozen.get("gate_eligible") is False
         and frozen.get("predecessor") == report.get("predecessor")
         and frozen.get("fixture") == report.get("fixture")
         and frozen_map.get("source_map_sha256") == frozen.get("current_source_map_sha256")
         and canonical_sha(frozen_map.get("source_map")) == frozen.get("current_source_map_sha256")
         and frozen.get("current_source_map_sha256") == report.get("runtime_source_map_sha256"),
         "P138 freeze report does not bind the verification source and fixture")
    for label, names in (("p138", CASE_NAMES),
                         ("register", FULL_CASES + CASE_NAMES),
                         ("combined", FULL_CASES + CASE_NAMES),
                         ("default", ("p132_module_preparation_remains_policy_neutral",))):
        reference = report.get("test_logs", {}).get(label)
        need(isinstance(reference, dict) and reference == ref(Path(reference.get("path", ""))),
             f"P138 {label} test log reference is stale")
        under_verification(Path(reference["path"]), f"P138 {label} test log")
        require_tests(Path(reference["path"]), names, label)
    stderr_ref = report.get("test_logs", {}).get("p138_stderr")
    need(isinstance(stderr_ref, dict) and stderr_ref == ref(Path(stderr_ref.get("path", ""))),
         "P138 trace log reference is stale")
    marker_path = Path(stderr_ref["path"])
    need(validate_case_records(marker_path) == report.get("case_records"),
         "P138 case records differ from retained markers")
    need(validate_traces(marker_path) == report.get("trace_summary"),
         "P138 trace summary differs from retained markers")
    need(report.get("fixture") == _current_fixture_refs(), "P138 fixture identity is stale")
    expected_helpers = {
        "p138.py": Path(__file__), "baseline.py": ROOT / "tools/wtime-bench/baseline.py",
        "build_budget.py": ROOT / "tools/checks/build_budget.py",
        "dependency_budget.py": ROOT / "tools/checks/dependency_budget.py",
    }
    helper_refs = report.get("producer_helpers")
    need(isinstance(helper_refs, dict) and set(helper_refs) == set(expected_helpers),
         "P138 retained helper identity set is incomplete")
    helper_dir = (report_path.parent / "helpers").resolve(strict=True)
    for name, source in expected_helpers.items():
        retained = Path(helper_refs[name].get("path", "")).resolve(strict=True)
        need(retained.is_relative_to(helper_dir) and helper_refs[name] == ref(retained)
             and sha(retained) == sha(source),
             f"P138 retained helper {name} is stale or outside proof output")
    for key in ("default", "register"):
        build_ref = report.get("build_budgets", {}).get(key, {}).get("artifact")
        need(isinstance(build_ref, dict) and build_ref == ref(Path(build_ref.get("path", ""))),
             f"P138 {key} build artifact reference is stale")
        under_verification(Path(build_ref["path"]), f"P138 {key} build report")
        mode, features = (("default", []) if key == "default" else
                          ("optional-feature", ["wtime-runtime/register-executor"]))
        need(check_build(Path(build_ref["path"]), mode, features) == {
            "artifact": build_ref,
            **{field: report["build_budgets"][key][field] for field in
               ("mode", "features", "stripped_bytes", "source_sha256", "source_file_count",
                "source_stable", "budget_pass", "default_dependency_closure_pass")},
        }, f"P138 {key} build budget summary differs from its report")
    current = current_runtime_source_map()
    need(report.get("runtime_source_map_sha256") == canonical_sha(current)
         and report.get("runtime_source_file_count") == len(current),
         "P138 runtime sources changed after evidence capture")
    predecessor = validate_predecessor()
    need(report.get("predecessor") == predecessor,
         "P138 immutable predecessor archive or P137 prerequisite is stale")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    freeze_parser = commands.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--freeze", type=Path, required=True)
    verify_parser.add_argument("--p138-log", type=Path, required=True)
    verify_parser.add_argument("--p138-stderr", type=Path, required=True)
    verify_parser.add_argument("--register-log", type=Path, required=True)
    verify_parser.add_argument("--combined-log", type=Path, required=True)
    verify_parser.add_argument("--default-log", type=Path, required=True)
    verify_parser.add_argument("--default-build", type=Path, required=True)
    verify_parser.add_argument("--register-build", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path, required=True)
    evidence_parser = commands.add_parser("check-evidence")
    evidence_parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze":
            result = freeze(args.output)
        elif args.command == "verify":
            result = verify(args.output, args.freeze, args.p138_log, args.p138_stderr,
                            args.register_log, args.combined_log, args.default_log,
                            args.default_build, args.register_build)
        else:
            result = verify_evidence(args.report)
        print(json.dumps({"schema": result["schema"], "status": result["status"]}, sort_keys=True))
        return 0
    except (P138Error, OSError, KeyError, TypeError, ValueError, RuntimeError) as exc:
        print(f"P138 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
