#!/usr/bin/env python3
"""Freeze P135's immediate predecessor and validate fresh-entry CLI observations."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
PREDECESSOR = "target/performance/P135-before"
PREDECESSOR_COMMIT = "4249c6d29def33b9ae40de8d8c524fb687513594"
PREDECESSOR_MANIFEST_SHA256 = "4f7a9c969196dac91cb9053fbf098b63c7b4d37489ad2ab10424b3befcab884e"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P134_EVIDENCE_SHA256 = "e03bc0fa5d417a131158858f56739749b3fdf4eb4e4857db52f7b02c2bcd30c5"
COUNTER_WASM_SHA256 = "6b365cbdbe8c189ad1b19a8cbdc6304012207dbf0f83c0de6824f8c8b8f07776"
CALL_COUNTS = (1, 10, 100, 10000)
CASES = (
    "p135_entry_batch_parser_rejects_unsupported_counts_and_strategy_pairs",
    "p135_fresh_batch_times_compile_instantiation_and_exact_calls",
    "p135_oracle_checks_every_call_and_stops_on_mismatch",
    "p135_diagnostic_reports_dispatch_deltas_without_timing_or_promotion_claims",
    "p135_cli_matrix_proves_fresh_eager_and_adaptive_lifetimes",
)
TIMING_FIELDS = {"total_ns", "prepare_ns", "instantiate_ns", "invoke_ns"}
FORBIDDEN_DIAGNOSTIC_FIELDS = TIMING_FIELDS | {"compile_count", "promotion", "promotions"}


class P135Error(ValueError):
    """P135 evidence is absent, stale, malformed, or inconsistent."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P135Error(message)


def integer(value: Any, label: str, *, minimum: int | None = None) -> int:
    need(type(value) is int, f"{label} must be a JSON integer")
    if minimum is not None:
        need(value >= minimum, f"{label} must be at least {minimum}")
    return value


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                   allow_nan=False).encode("utf-8")).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P135Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path) -> Path:
    path = path.resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P135 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    with (path / "helper.executed.py").open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve())})
    return path


def runtime_source_map(manifest: dict[str, Any]) -> dict[str, str]:
    crates = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
    root_sources = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
    prefixes = tuple(f"crates/{crate}/" for crate in crates)
    sources: dict[str, str] = {}
    for row in manifest["files"].values():
        source_path = row.get("source_path", "")
        if row.get("role") == "source" and (
            source_path in root_sources
            or any(source_path == f"crates/{crate}/Cargo.toml" for crate in crates)
            or (source_path.endswith(".rs") and source_path.startswith(prefixes))
        ):
            sources[source_path] = row["sha256"]
    return dict(sorted(sources.items()))


def validate_foundations() -> dict[str, Any]:
    import importlib.util

    baseline_path = ROOT / "tools/wtime-bench/baseline.py"
    spec = importlib.util.spec_from_file_location("p135_baseline", baseline_path)
    need(spec is not None and spec.loader is not None, "cannot load P001 archive validator")
    baseline = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(baseline)

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    predecessor_manifest = predecessor / "manifest.json"
    need(sha(predecessor_manifest) == PREDECESSOR_MANIFEST_SHA256,
         "P135 immediate predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT
         and manifest.get("mode") == "register"
         and manifest.get("execution_policy", {}).get("resolved_mode") == "register",
         "P135 immediate predecessor is not the pinned Register archive")
    runtime_map = runtime_source_map(manifest)
    need({"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"} <= set(runtime_map),
         "P135 archive omits root Cargo/toolchain inputs")
    for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
        need(f"crates/{crate}/Cargo.toml" in runtime_map
             and any(name.startswith(f"crates/{crate}/") and name.endswith(".rs") for name in runtime_map),
             f"P135 archive omits {crate} source")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True
         and p001_manifest.get("smoke", {}).get("returncode") == 0,
         "P001 archive lacks a successful correctness smoke")

    completion_path = ROOT / "docs/benchmarks/performance/P134-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P134-evidence.json"
    archive_evidence_path = ROOT / "target/performance/P134/shared-finalization/attempt-0001/evidence.json"
    completion = load_json(completion_path, "P134 completion")
    evidence = load_json(evidence_path, "P134 evidence")
    archive_evidence = load_json(archive_evidence_path, "archived P134 evidence")
    need(completion.get("task") == "P134" and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         "P134 is not complete as a contract-only foundation")
    need(sha(evidence_path) == P134_EVIDENCE_SHA256
         and sha(archive_evidence_path) == P134_EVIDENCE_SHA256
         and evidence == archive_evidence,
         "P134 durable evidence identity or archive copy changed")
    need(evidence.get("schema") == "wtime-verification-evidence-v1"
         and evidence.get("task") == "P134" and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("synthetic") is False
         and evidence.get("decision") == "contract_only"
         and evidence.get("outcome") == "contract_only"
         and set(evidence.get("phases", {})) == {"freeze", "verify"}
         and completion.get("artifact_hashes", {}).get(str(archive_evidence_path.resolve())) == P134_EVIDENCE_SHA256,
         "P134 evidence is not a passed real contract-only record")
    return {
        "predecessor": ref(predecessor_manifest),
        "predecessor_commit": PREDECESSOR_COMMIT,
        "predecessor_mode": "register",
        "runtime_source_map": runtime_map,
        "runtime_source_map_sha256": canonical_sha(runtime_map),
        "p001": {"manifest": ref(p001 / "manifest.json"), "binary": ref(p001 / "binary/wtime-bench"),
                 "correctness_checked": True},
        "p134": {"completion": ref(completion_path), "evidence": ref(evidence_path),
                 "archived_evidence": ref(archive_evidence_path), "outcome": "contract_only",
                 "passed": True, "timing_evidence": False},
    }


def validate_p134_build_budget_reuse() -> tuple[dict[str, Any], dict[str, str]]:
    """Reuse P134 build checks only when their full compilation snapshot is still exact."""
    import importlib.util

    helper_path = ROOT / "tools/checks/build_budget.py"
    spec = importlib.util.spec_from_file_location("p135_build_budget", helper_path)
    need(spec is not None and spec.loader is not None, "cannot load shared build-budget checker")
    helper = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(helper)

    # Metadata is read-only. It lets the shared helper reproduce the exact source
    # membership rules used by the historical P134 build records without building.
    metadata = helper.cargo_metadata()
    current_inputs = helper.source_inputs(metadata)
    current_hash = helper.hash_inputs(current_inputs)
    proof = load_json(ROOT / "docs/benchmarks/performance/P134-proof.json", "P134 proof")
    inherited: dict[str, Any] = {}
    configurations = (
        ("default", "default", []),
        ("register", "optional-feature", ["wtime-runtime/register-executor"]),
    )
    for doc_name, report_mode, features in configurations:
        report_path = ROOT / f"docs/benchmarks/performance/P134-{doc_name}-build.json"
        report = load_json(report_path, f"P134 {doc_name} build report")
        need(report.get("mode") == report_mode and report.get("features") == features,
             f"P134 {doc_name} build report has the wrong feature configuration")
        before = report.get("source_inputs")
        after = report.get("source_inputs_after")
        isolated = report.get("isolated_source_inputs_after")
        need(report.get("source_stable") is True and isinstance(before, dict)
             and before == after == isolated == current_inputs
             and report.get("source_sha256") == current_hash
             and report.get("source_sha256_after") == current_hash,
             f"P134 {doc_name} compilation source membership or hashes are not current")
        need(helper.check_budget(report) is True
             and report.get("budget_pass") is True
             and report.get("default_dependency_closure_pass") is True
             and report.get("default_dependency_violations") == [],
             f"P134 {doc_name} build budget or dependency closure did not pass")
        need(type(report.get("stripped_bytes")) is int and report["stripped_bytes"] > 0,
             f"P134 {doc_name} stripped build size is invalid")

        declared_ref = proof.get("build_budgets", {}).get(doc_name, {}).get("artifact", {})
        archived_path = _inside(ROOT, declared_ref.get("path", ""),
                                f"P134 {doc_name} build report artifact")
        artifact_ref = ref(archived_path)
        need(declared_ref == artifact_ref and archived_path.read_bytes() == report_path.read_bytes(),
             f"P134 proof does not bind the exact {doc_name} build report bytes")
        proof_budget = proof.get("build_budgets", {}).get(doc_name, {})
        need(proof_budget.get("mode") == report_mode and proof_budget.get("features") == features
             and proof_budget.get("stripped_bytes") == report["stripped_bytes"]
             and proof_budget.get("budget_pass") is True
             and proof_budget.get("default_dependency_closure_pass") is True,
             f"P134 proof does not bind the passing {doc_name} build-budget result")
        inherited[doc_name] = {
            "report": artifact_ref,
            "source_sha256": current_hash,
            "source_file_count": len(current_inputs),
            "features": features,
            "stripped_bytes": report["stripped_bytes"],
            "budget_pass": True,
            "default_dependency_closure_pass": True,
            "historical_build_reused": True,
        }
    return inherited, current_inputs


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    foundations = validate_foundations()
    runtime_path = output / "runtime.json"
    write_new(runtime_path, {"runtime_source_map": foundations["runtime_source_map"],
                             "runtime_source_map_sha256": foundations["runtime_source_map_sha256"]})
    report = {
        "schema": "wtime-p135-freeze-v1", "status": "passed", "timing_evidence": False,
        **foundations, "runtime_map": ref(runtime_path),
        "runtime_source_file_count": len(foundations["runtime_source_map"]),
    }
    write_new(output / "report.json", report)
    return report


def _inside(root: Path, value: str, label: str) -> Path:
    root = Path(root).resolve(strict=True)
    path = Path(value).resolve(strict=True)
    need(path.is_relative_to(root), f"{label} escapes the evidence root")
    return path


def validate_timing_row(row: dict[str, Any], *, root: Path, binary: Path, fixture: Path,
                        strategy: str, calls: int) -> None:
    need(row.get("kind") == "timing" and integer(row.get("returncode"), "timing returncode") == 0,
         "timing child did not succeed")
    need(type(row.get("calls")) is int and row.get("strategy") == strategy and row.get("calls") == calls,
         "timing command settings differ from requested matrix row")
    binary_path = _inside(root, row.get("binary", ""), "binary")
    fixture_path = _inside(root, row.get("fixture", ""), "fixture")
    need(binary_path == binary.resolve(strict=True) and fixture_path == fixture.resolve(strict=True),
         "timing child executable or fixture path is not the expected source")
    need(row.get("binary_sha256") == sha(binary_path) and row.get("fixture_sha256") == sha(fixture_path),
         "timing child executable or fixture hash is stale")
    argv = row.get("argv")
    need(isinstance(argv, list) and len(argv) == 10 and argv[0] == str(binary_path)
         and argv[1] == "--entry-batch" and argv[2] == str(fixture_path),
         "timing child argv is malformed or not bound to its executable and fixture")
    expected_args = ["p135_counter", "7", "8", str(calls), "register", strategy,
                     "0" if strategy == "eager" else "3"]
    need(argv[3:] == expected_args and argv[3] == "p135_counter",
         "timing child argv does not match the registered settings")
    need(row.get("stderr") == "", "successful timing child wrote stderr")
    lines = row.get("stdout", "").splitlines()
    need(len(lines) == 1, "timing child must emit exactly one JSON line")
    try:
        value = json.loads(lines[0])
    except json.JSONDecodeError as exc:
        raise P135Error(f"timing child emitted invalid JSON: {exc}") from exc
    need(isinstance(value, dict), "timing output must be a JSON object")
    need(value.get("schema") == "wtime-entry-batch-v1"
         and integer(value.get("schema_version"), "timing schema_version") == 1
         and value.get("driver") == "entry_batch" and value.get("case") == "p135_counter",
         "timing output schema or driver identity is wrong")
    threshold = value.get("threshold")
    need(value.get("mode") == "register" and value.get("strategy") == strategy
         and ((threshold is None) if strategy == "eager"
              else integer(threshold, "timing threshold", minimum=1) == 3),
         "timing output strategy settings do not match argv")
    need(value.get("source_fuel") is None and value.get("feature_profile") == "mvp",
         "timing output changed the omitted-fuel or MVP settings")
    need(integer(value.get("input"), "timing input") == 7
         and integer(value.get("expected"), "timing expected") == 8
         and integer(value.get("result"), "timing result") == 8,
         "timing output oracle result does not match the admitted fixture")
    need(integer(value.get("fixture_calls"), "timing fixture_calls", minimum=0) == calls,
         "timing output counter does not prove exactly the requested guest entries")
    need(integer(value.get("requested_calls"), "timing requested_calls", minimum=0) == calls
         and integer(value.get("actual_calls"), "timing actual_calls", minimum=0) == calls,
         "timing output did not report exactly the requested calls")
    need(value.get("instrumented") is False, "timing executable is instrumented")
    need(not any("per_operation" in key or "per_op" in key for key in value),
         "timing output reports a divided per-operation field")
    need(TIMING_FIELDS <= value.keys(), "timing output omits a required duration")
    timings = [value[name] for name in ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")]
    need(all(type(item) is int and item >= 0 for item in timings)
         and value["total_ns"] > 0
         and value["total_ns"] == value["prepare_ns"] + value["instantiate_ns"] + value["invoke_ns"],
         "timing output phase durations are not additive integer nanoseconds")


def validate_diagnostic_row(row: dict[str, Any], *, root: Path, binary: Path, fixture: Path,
                            strategy: str, calls: int) -> None:
    need(row.get("kind") == "diagnostic" and integer(row.get("returncode"), "diagnostic returncode") == 0,
         "diagnostic child did not succeed")
    need(type(row.get("calls")) is int and row.get("strategy") == strategy and row.get("calls") == calls,
         "diagnostic command settings differ from requested row")
    binary_path = _inside(root, row.get("binary", ""), "diagnostic binary")
    fixture_path = _inside(root, row.get("fixture", ""), "diagnostic fixture")
    need(binary_path == binary.resolve(strict=True) and fixture_path == fixture.resolve(strict=True),
         "diagnostic child executable or fixture path is not the expected source")
    need(row.get("binary_sha256") == sha(binary_path) and row.get("fixture_sha256") == sha(fixture_path),
         "diagnostic child executable or fixture hash is stale")
    argv = row.get("argv")
    expected_args = ["--entry-diagnostic", str(fixture_path), "p135_counter", "7", "8", str(calls),
                     "register", strategy, "0" if strategy == "eager" else "3"]
    need(argv == [str(binary_path), *expected_args], "diagnostic argv is malformed")
    need(row.get("stderr") == "", "successful diagnostic child wrote stderr")
    lines = row.get("stdout", "").splitlines()
    need(len(lines) == 1, "diagnostic child must emit exactly one JSON line")
    try:
        value = json.loads(lines[0])
    except json.JSONDecodeError as exc:
        raise P135Error(f"diagnostic child emitted invalid JSON: {exc}") from exc
    need(isinstance(value, dict), "diagnostic output must be a JSON object")
    need(value.get("schema") == "wtime-entry-diagnostic-v1"
         and integer(value.get("schema_version"), "diagnostic schema_version") == 1
         and value.get("driver") == "entry_diagnostic" and value.get("case") == "p135_counter"
         and value.get("instrumented") is True,
         "diagnostic schema or instrumented flag is invalid")
    need(not (FORBIDDEN_DIAGNOSTIC_FIELDS & value.keys()), "diagnostic output contains timing or promotion fields")
    threshold = value.get("threshold")
    need(value.get("mode") == "register" and value.get("strategy") == strategy
         and ((threshold is None) if strategy == "eager"
              else integer(threshold, "diagnostic threshold", minimum=1) == 3),
         "diagnostic output strategy settings do not match argv")
    need(value.get("source_fuel") is None and value.get("feature_profile") == "mvp",
         "diagnostic output changed the omitted-fuel or MVP settings")
    need(integer(value.get("input"), "diagnostic input") == 7
         and integer(value.get("expected"), "diagnostic expected") == 8
         and integer(value.get("result"), "diagnostic result") == 8
         and integer(value.get("requested_calls"), "diagnostic requested_calls", minimum=0) == calls
         and integer(value.get("actual_calls"), "diagnostic actual_calls", minimum=0) == calls,
         "diagnostic output result or call count is invalid")
    rows = value.get("per_call")
    need(isinstance(rows, list) and len(rows) == calls, "diagnostic output does not contain one row per call")
    for index, call in enumerate(rows, 1):
        need(isinstance(call, dict), f"diagnostic call row {index} must be an object")
        for field in ("call", "register_dispatches", "register_exits", "fallback_frames"):
            integer(call.get(field), f"diagnostic call {index} {field}", minimum=0)
        need(call == {"call": index, "register_dispatches": (10 if strategy == "eager" or index >= 3 else 0),
                      "register_exits": 0, "fallback_frames": (0 if strategy == "eager" or index >= 3 else 1)},
             f"diagnostic counter delta is wrong at call {index}")
    first = 1 if strategy == "eager" else (3 if calls >= 3 else None)
    if first is not None:
        integer(value.get("first_register_call"), "diagnostic first_register_call", minimum=1)
    need(value.get("first_register_call") == first, "diagnostic first dispatch call is incorrect")


def _run(argv: list[str], *, kind: str, binary: Path, fixture: Path,
         strategy: str | None = None, calls: int | None = None) -> dict[str, Any]:
    result = subprocess.run(argv, text=True, capture_output=True, check=False)
    row: dict[str, Any] = {
        "kind": kind, "argv": argv, "returncode": result.returncode,
        "stdout": result.stdout, "stderr": result.stderr,
        "binary": str(binary.resolve(strict=True)), "fixture": str(fixture.resolve(strict=True)),
        "binary_sha256": sha(binary), "fixture_sha256": sha(fixture),
    }
    if strategy is not None:
        row.update(strategy=strategy, calls=calls)
    return row


def collect_observations(output: Path, production: Path, diagnostic: Path) -> dict[str, Any]:
    fixture = ROOT / "tools/wtime-bench/fixtures/p135/counter.wasm"
    bad_fixture = ROOT / "tools/wtime-bench/fixtures/p135/late_bad_result.wasm"
    provenance_path = ROOT / "tools/wtime-bench/fixtures/p135/provenance.json"
    provenance = load_json(provenance_path, "P135 fixture provenance")
    need(provenance.get("profile") == "strict MVP; both fixtures use only MVP memory and integer instructions"
         and provenance.get("official_validator") == "wasm-tools validate BINARY --features mvp"
         and provenance.get("errors_count") == 0,
         "P135 fixture profile or validation provenance changed")
    need(sha(ROOT / "tools/wtime-bench/fixtures/p135/LICENSE.txt") ==
         provenance.get("license_sha256"), "P135 fixture license identity changed")
    for fixture_name, expected_hash in (("counter", COUNTER_WASM_SHA256),
                                        ("late_bad_result", None)):
        record = provenance.get("fixtures", {}).get(fixture_name, {})
        wasm_path = ROOT / "tools/wtime-bench/fixtures/p135" / record.get("wasm", "")
        wat_path = ROOT / "tools/wtime-bench/fixtures/p135" / record.get("wat", "")
        recorded_hash = record.get("wasm_sha256")
        need((expected_hash is None or recorded_hash == expected_hash)
             and record.get("source_opcode_count", 0) > 0
             and record.get("exports", {}).get("run") == "(i32)->i32"
             and record.get("exports", {}).get("count", "").startswith("memory ")
             and wasm_path.is_file() and wat_path.is_file()
             and sha(wasm_path) == recorded_hash and sha(wat_path) == record.get("wat_sha256"),
             f"P135 {fixture_name} fixture identity or source binding changed")
    need(fixture.is_file() and bad_fixture.is_file(), "P135 CLI fixtures are missing")
    production = production.resolve(strict=True)
    diagnostic = diagnostic.resolve(strict=True)
    rows: list[dict[str, Any]] = []
    for strategy in ("eager", "adaptive"):
        for calls in CALL_COUNTS:
            argv = [str(production), "--entry-batch", str(fixture), "p135_counter", "7", "8", str(calls),
                    "register", strategy, "0" if strategy == "eager" else "3"]
            row = _run(argv, kind="timing", binary=production, fixture=fixture,
                       strategy=strategy, calls=calls)
            validate_timing_row(row, root=ROOT, binary=production, fixture=fixture,
                                strategy=strategy, calls=calls)
            rows.append(row)
    diagnostics: list[dict[str, Any]] = []
    for strategy in ("eager", "adaptive"):
        for calls in CALL_COUNTS:
            argv = [str(diagnostic), "--entry-diagnostic", str(fixture), "p135_counter", "7", "8", str(calls),
                    "register", strategy, "0" if strategy == "eager" else "3"]
            row = _run(argv, kind="diagnostic", binary=diagnostic, fixture=fixture,
                       strategy=strategy, calls=calls)
            validate_diagnostic_row(row, root=ROOT, binary=diagnostic, fixture=fixture,
                                    strategy=strategy, calls=calls)
            diagnostics.append(row)

    negative: list[dict[str, Any]] = []
    malformed = [
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "2", "register", "eager", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "10", "portable", "eager", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "10", "register", "eager", "3"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "10", "register", "adaptive", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "oops", "register", "eager", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "0", "register", "eager", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "-1", "register", "eager", "0"], "production"),
        (["--entry-batch", str(ROOT / "missing.wasm"), "p135_counter", "7", "8", "10001", "register", "eager", "0"], "production"),
        (["--entry-batch", str(bad_fixture), "p135_counter", "7", "8", "10", "register", "eager", "0"], "production"),
        (["--entry-diagnostic", str(fixture), "p135_counter", "7", "8", "10", "register", "eager", "0"], "production"),
        (["--entry-batch", str(fixture), "p135_counter", "7", "8", "10", "register", "eager", "0"], "diagnostic"),
    ]
    for suffix, (args, which) in enumerate(malformed):
        binary = production if which == "production" else diagnostic
        given_fixture = Path(args[1])
        argv = [str(binary), *args]
        row = _run(argv, kind=f"negative-{suffix}", binary=binary, fixture=fixture)
        need(row["returncode"] == 2 and row["stdout"] == "" and row["stderr"],
             f"negative CLI probe {suffix} did not fail before emitting JSON")
        if suffix < 8:
            need("entry protocol failed" in row["stderr"], "malformed CLI input failed for an unexpected reason")
        if suffix == 8:
            need("oracle mismatch at call 2" in row["stderr"],
                 "late bad result was not rejected at its first mismatching call")
        negative.append(row)

    observation = {
        "schema": "wtime-p135-observations-v1", "timing_evidence": False,
        "settings": {"mode": "register", "call_counts": list(CALL_COUNTS), "strategies": ["eager", "adaptive"],
                     "adaptive_threshold": 3, "source_fuel": None, "feature_profile": "mvp",
                     "fixture": str(fixture.resolve()),
                     "fixture_sha256": sha(fixture)},
        "production_binary": ref(production), "diagnostic_binary": ref(diagnostic),
        "timing_rows": rows, "diagnostic_rows": diagnostics, "negative_rows": negative,
    }
    validate_observations(observation, root=ROOT)
    write_new(output / "observations.json", observation)
    return observation


def validate_observations(observation: dict[str, Any], *, root: Path) -> None:
    root = Path(root).resolve(strict=True)
    need(observation.get("schema") == "wtime-p135-observations-v1"
         and observation.get("timing_evidence") is False,
         "P135 observation envelope is malformed or marked as timing evidence")
    settings = observation.get("settings", {})
    need(settings.get("mode") == "register"
         and isinstance(settings.get("call_counts"), list)
         and all(type(count) is int for count in settings["call_counts"])
         and settings.get("call_counts") == list(CALL_COUNTS)
         and settings.get("strategies") == ["eager", "adaptive"]
         and type(settings.get("adaptive_threshold")) is int
         and settings.get("adaptive_threshold") == 3
         and settings.get("source_fuel") is None
         and settings.get("feature_profile") == "mvp",
         "P135 observation settings are incomplete")
    fixture = _inside(root, settings.get("fixture", ""), "fixture")
    need(settings.get("fixture_sha256") == sha(fixture), "P135 observation fixture hash is stale")
    binaries: dict[str, Path] = {}
    for name, key in (("production", "production_binary"), ("diagnostic", "diagnostic_binary")):
        record = observation.get(key, {})
        path = _inside(root, record.get("path", ""), f"{name} binary")
        need(record.get("sha256") == sha(path)
             and integer(record.get("bytes"), f"{name} binary byte length", minimum=0) == path.stat().st_size,
             f"P135 {name} executable identity is stale")
        binaries[name] = path
    timing_rows = observation.get("timing_rows")
    diagnostic_rows = observation.get("diagnostic_rows")
    need(isinstance(timing_rows, list) and len(timing_rows) == 8,
         "P135 timing observation matrix must contain eight rows")
    need(isinstance(diagnostic_rows, list) and len(diagnostic_rows) == 8,
         "P135 diagnostic observation matrix must contain eight rows")
    seen: set[tuple[str, int]] = set()
    for row in timing_rows:
        need(isinstance(row, dict), "P135 timing matrix rows must be objects")
        strategy, calls = row.get("strategy"), row.get("calls")
        integer(calls, "timing matrix calls", minimum=1)
        key = (strategy, calls)
        need(key not in seen, "duplicate P135 timing matrix row")
        seen.add(key)
        validate_timing_row(row, root=root, binary=binaries["production"], fixture=fixture,
                            strategy=strategy, calls=calls)
    need(seen == {(strategy, count) for strategy in ("eager", "adaptive") for count in CALL_COUNTS},
         "P135 timing matrix does not cover every strategy and call count")
    seen.clear()
    for row in diagnostic_rows:
        need(isinstance(row, dict), "P135 diagnostic matrix rows must be objects")
        strategy, calls = row.get("strategy"), row.get("calls")
        integer(calls, "diagnostic matrix calls", minimum=1)
        key = (strategy, calls)
        need(key not in seen, "duplicate P135 diagnostic matrix row")
        seen.add(key)
        validate_diagnostic_row(row, root=root, binary=binaries["diagnostic"], fixture=fixture,
                                strategy=strategy, calls=calls)
    need(seen == {(strategy, count) for strategy in ("eager", "adaptive") for count in CALL_COUNTS},
         "P135 diagnostic matrix does not cover every strategy and call count")
    need(isinstance(observation.get("negative_rows"), list) and len(observation["negative_rows"]) == 11,
         "P135 negative CLI probes are incomplete")
    for index, row in enumerate(observation["negative_rows"]):
        need(isinstance(row, dict) and row.get("kind") == f"negative-{index}",
             "P135 negative rows are missing, reordered, or malformed")
        binary_path = _inside(root, row.get("binary", ""), "negative-probe binary")
        fixture_path = _inside(root, row.get("fixture", ""), "negative-probe fixture")
        need(row.get("binary_sha256") == sha(binary_path)
             and row.get("fixture_sha256") == sha(fixture_path),
             "negative CLI probe executable or fixture hash is stale")
        argv = row.get("argv")
        need(isinstance(argv, list) and len(argv) == 10 and argv[0] == str(binary_path),
             "negative CLI probe omitted its complete argv")
        option = argv[1]
        need(option in ("--entry-batch", "--entry-diagnostic"), "negative CLI probe has an unknown command")
        # Some parser probes deliberately name a missing fixture to prove parsing precedes file access.
        input_path = Path(argv[2]).resolve()
        need(input_path.is_relative_to(root), "negative CLI argv names a fixture outside the evidence root")
        need(integer(row.get("returncode"), f"negative-{index} returncode") == 2
             and row.get("stdout") == "" and isinstance(row.get("stderr"), str) and row.get("stderr"),
             "negative CLI probe unexpectedly succeeded or emitted JSON")


def verify(output: Path, freeze_report: Path, production: Path, diagnostic: Path) -> dict[str, Any]:
    output = start_output(output)
    frozen = load_json(freeze_report, "P135 freeze report")
    need(frozen.get("schema") == "wtime-p135-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P135 freeze report is invalid")
    foundations = validate_foundations()
    need(frozen.get("runtime_source_map") == foundations["runtime_source_map"]
         and frozen.get("runtime_source_map_sha256") == foundations["runtime_source_map_sha256"],
         "P135 runtime source map changed after freeze")
    inherited_builds, build_source_inputs = validate_p134_build_budget_reuse()

    observation = collect_observations(output, production, diagnostic)
    report = {
        "schema": "wtime-p135-verification-v1", "status": "passed", "timing_evidence": False,
        "decision": "integrity", "runtime_source_map_sha256": foundations["runtime_source_map_sha256"],
        "core_build_budgets_reused": True, "inherited_core_builds": inherited_builds,
        "build_source_inputs": build_source_inputs,
        "build_source_inputs_sha256": canonical_sha(build_source_inputs),
        "build_budget_note": "P134 budget checks are reused only; P135 makes no new compilation-time observation.",
        "production_instrumented": False,
        "counts": {"cli_matrix_rows": len(observation["timing_rows"]),
                   "diagnostic_rows": len(observation["diagnostic_rows"]),
                   "negative_rows": len(observation["negative_rows"]), "named_cases": len(CASES)},
        "production_binary": ref(production), "diagnostic_binary": ref(diagnostic),
        "observations": ref(output / "observations.json"),
    }
    write_new(output / "report.json", report)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze_parser = sub.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = sub.add_parser("verify")
    verify_parser.add_argument("--output", type=Path, required=True)
    verify_parser.add_argument("--freeze-report", type=Path, required=True)
    verify_parser.add_argument("--production-binary", type=Path, required=True)
    verify_parser.add_argument("--diagnostic-binary", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze":
            result = freeze(args.output)
        else:
            result = verify(args.output, args.freeze_report, args.production_binary, args.diagnostic_binary)
    except (P135Error, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"P135 verification failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True, allow_nan=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
