#!/usr/bin/env python3
"""Freeze P132's Register predecessor and verify recorded preparation evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

PREDECESSOR = "target/performance/P132-before"
PREDECESSOR_COMMIT = "4e333ac7890b5325ca80d8940e0d5a89ee0d4938"
PREDECESSOR_MANIFEST_SHA256 = "8a4aebc05209ec9fa9a77c6fcb2b229e17820271779661473698c523aa672374"
PROTOCOL_SHA256 = "ac7b9ce0d12a0a17b38acbfb964f8704c8f85c1927d89385a055b612f77b848c"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
INVENTORY_DIR = "target/performance/P132-inventory-before"
INVENTORY_GOLDEN_SHA256 = "78f2c13525bcd4f4989307589321436badad713f53e4561cbd367c78df965f07"
INVENTORY_REFERENCE_SHA256 = "0df1410bf4f057788ed24e8a65e67177ac61785c7cf449251ceb4951d737e86c"
INVENTORY_FIXTURE_MAP_SHA256 = "36c54de2f75e62e1fb492715ea0cc9a7a8cb4e4255e3f4ea378d7b12dc29d394"
INVENTORY_LINE_COUNT = 1133

COMPLETED = {
    "P065": {
        "completion_sha256": "b83fa7c6dce5dfd0f37eacf872fcb5163553ff6de860b48bf1d04fb7b9cd46cb",
        "evidence_sha256": "5bf93a86a64a5a6a902b1ddb2944bd95c59b4c527705254c1eadbaeafdee6118",
        "final_evidence": "target/performance/P065/shared-finalization/attempt-0004/evidence.json",
    },
    "P131": {
        "completion_sha256": "ec2d44410b640798665329668508edf260cc50ea2126cbc7e5da3ad9873f3041",
        "evidence_sha256": "b1d0e3e58c7cd0891f9772fcff71f16e26d314029c27d98db1705484b2a2a1fc",
        "final_evidence": "target/performance/P131/shared-finalization/attempt-0001/evidence.json",
    },
}

P132_TESTS = (
    "p132_eligible_functions_preserve_prepared_artifacts",
    "p132_unsupported_and_overbudget_functions_decline_without_publication",
    "p132_module_attempts_and_aggregate_budget_match_predecessor",
    "p132_module_preparation_remains_policy_neutral",
)
DEFAULT_TEST = P132_TESTS[-1]
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
MAX_BUILD_BYTES = 30_000_000
EXPECTED_COUNTS = {
    "fixture_count": 20,
    "prepared_fixture_count": 19,
    "total_body_count": 196,
    "total_prepared": 181,
    "total_declined": 15,
    "pc_line_count": 805,
    "constant_line_count": 68,
    "memory_argument_line_count": 33,
    "edge_line_count": 10,
}
RECORD_KINDS = ("fixture", "body", "pc", "constant", "memory_argument", "edge", "summary")


class P132Error(ValueError):
    """P132 evidence is absent, stale, malformed, or inconsistent."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P132Error(message)


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


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path) -> Path:
    path = path.resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P132 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    with (path / "helper.executed.py").open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve())})
    return path


def runtime_source_map(manifest: dict[str, Any]) -> dict[str, str]:
    prefixes = tuple(f"crates/{crate}/" for crate in CORE_CRATES)
    sources: dict[str, str] = {}
    for row in manifest["files"].values():
        source_path = row.get("source_path", "")
        if (row.get("role") == "source" and
                (source_path in ROOT_SOURCES or
                 any(source_path == f"crates/{crate}/Cargo.toml" for crate in CORE_CRATES) or
                 (source_path.endswith(".rs") and source_path.startswith(prefixes)))):
            sources[source_path] = row["sha256"]
    return dict(sorted(sources.items()))


def load_json(path: Path, label: str) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P132Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def completed(task: str) -> dict[str, Any]:
    expected = COMPLETED[task]
    completion_path = ROOT / f"docs/benchmarks/performance/{task}-completion.json"
    evidence_path = ROOT / f"docs/benchmarks/performance/{task}-evidence.json"
    final_path = ROOT / expected["final_evidence"]
    need(sha(completion_path) == expected["completion_sha256"],
         f"{task} completion identity changed")
    need(sha(evidence_path) == expected["evidence_sha256"], f"{task} evidence identity changed")
    completion = load_json(completion_path, f"{task} completion")
    evidence = load_json(evidence_path, f"{task} evidence")
    need(completion.get("task") == task and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         f"{task} is not a durable completed correctness foundation")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == expected["evidence_sha256"]
         and sha(final_path) == expected["evidence_sha256"],
         f"{task} final evidence differs from its completion record")
    need(evidence.get("task") == task and evidence.get("status") == "passed"
         and evidence.get("decision") == "contract_only",
         f"{task} final evidence did not pass as contract_only")
    return {"completion": ref(completion_path), "evidence": ref(evidence_path),
            "final_evidence": ref(final_path), "outcome": completion["outcome"]}


def validate_inventory_inputs(manifest: dict[str, Any]) -> dict[str, Any]:
    inventory = ROOT / INVENTORY_DIR
    golden_path = inventory / "golden-inventory.txt"
    reference_path = inventory / "reference-corrected.json"
    fixture_map_path = inventory / "fixture-identities-before.json"
    need(sha(golden_path) == INVENTORY_GOLDEN_SHA256, "P132 immutable golden inventory changed")
    need(sha(reference_path) == INVENTORY_REFERENCE_SHA256, "P132 corrected inventory reference changed")
    need(sha(fixture_map_path) == INVENTORY_FIXTURE_MAP_SHA256,
         "P132 prior fixture identity map changed")
    reference = load_json(reference_path, "corrected golden inventory reference")
    fixture_pins = load_json(fixture_map_path, "prior fixture source pins")
    need(reference.get("schema") == 1 and reference.get("kind") == "corrected-golden-reader-reference",
         "P132 corrected inventory reference schema changed")
    need(reference.get("golden_path") == str(golden_path.resolve())
         and reference.get("golden_sha256") == INVENTORY_GOLDEN_SHA256
         and reference.get("golden_line_count") == INVENTORY_LINE_COUNT,
         "P132 corrected reference does not pin the golden inventory")
    need(reference.get("archive_commit") == PREDECESSOR_COMMIT
         and reference.get("archive_manifest_sha256") == PREDECESSOR_MANIFEST_SHA256,
         "P132 inventory reference does not name the immutable immediate predecessor")
    need(reference.get("fixture_identity_path") == str(fixture_map_path.resolve())
         and reference.get("fixture_identity_sha256") == INVENTORY_FIXTURE_MAP_SHA256
         and reference.get("fixture_source_pins") == fixture_pins,
         "P132 prior inventory fixture identities differ")
    need(reference.get("golden_line_count") == INVENTORY_LINE_COUNT
         and reference.get("fixture_count") == EXPECTED_COUNTS["fixture_count"]
         and reference.get("total_body_count") == EXPECTED_COUNTS["total_body_count"]
         and reference.get("prepared_body_count") == EXPECTED_COUNTS["total_prepared"]
         and reference.get("declined_body_count") == EXPECTED_COUNTS["total_declined"]
         and reference.get("pc_line_count") == EXPECTED_COUNTS["pc_line_count"]
         and reference.get("constant_line_count") == EXPECTED_COUNTS["constant_line_count"]
         and reference.get("memory_argument_line_count") == EXPECTED_COUNTS["memory_argument_line_count"]
         and reference.get("edge_line_count") == EXPECTED_COUNTS["edge_line_count"]
         and reference.get("fixture_summary_expected") ==
         "fixture_count=20; prepared_fixture_count=19; total_prepared=181; total_declined=15",
         "P132 corrected inventory field counts changed")
    need(reference.get("validation") == {
        "typed_record_kinds_counted_by_exact_prefix": True,
        "summary_consistent": True,
        "strict_membership": True,
        "per_prepared_body_pc_count": True,
        "memory_coverage_positive": True,
        "golden_unchanged": True,
    }, "P132 corrected inventory validation record changed")
    ordered_membership = reference.get("ordered_fixture_membership", [])
    need(len(ordered_membership) == len(set(ordered_membership))
         and set(ordered_membership) == {record.get("fixture") for record in fixture_pins.values()},
         "P132 golden fixture membership differs from the pinned fixture set")
    for absolute, record in fixture_pins.items():
        fixture_path = Path(absolute).resolve(strict=True)
        need(fixture_path.is_relative_to(ROOT), f"P132 fixture pin escapes repository: {absolute}")
        need(sha(fixture_path) == record.get("sha256")
             and fixture_path.stat().st_size == record.get("size_bytes"),
             f"P132 source fixture differs from frozen inventory pin: {absolute}")
    archived_execute = manifest["files"].get("source/crates/wtime-runtime/src/execution/register/execute.rs")
    need(isinstance(archived_execute, dict)
         and archived_execute.get("sha256") == reference.get("archived_execute_original_sha256"),
         "P132 inventory reference is not bound to archived Register preparation source")
    reader = ROOT / "target/recovery/P132-inventory-0001/validate_golden_reader.py"
    need(sha(reader) == reference.get("reader_source_sha256"),
         "P132 corrected golden reader source changed")
    prior_run = load_json(inventory / "reference.json", "prior inventory run record")
    cargo_result = load_json(inventory / "cargo-result.json", "prior inventory Cargo result")
    cargo_command = load_json(inventory / "cargo-command.json", "prior inventory Cargo command")
    expected_command = ["cargo", "test", "--locked", "-p", "wtime-runtime", "--features",
                        "register-executor,execution-reference", "--lib",
                        "p132_inventory_prepared_functions", "--", "--nocapture"]
    need(prior_run.get("archive_commit") == PREDECESSOR_COMMIT
         and prior_run.get("archive_manifest_sha256") == PREDECESSOR_MANIFEST_SHA256
         and prior_run.get("cargo_command") == expected_command
         and prior_run.get("features") == ["register-executor", "execution-reference"]
         and prior_run.get("cargo_returncode") == 0
         and prior_run.get("cargo_stdout_sha256") == sha(inventory / "cargo.stdout")
         and prior_run.get("cargo_stderr_sha256") == sha(inventory / "cargo.stderr"),
         "P132 prior inventory log does not match its source/archive record")
    need(cargo_command.get("argv") == expected_command
         and cargo_command.get("features") == ["register-executor", "execution-reference"],
         "P132 prior inventory command record changed")
    launcher = cargo_command.get("launcher_argv")
    need(isinstance(launcher, list) and len(launcher) == 3
         and launcher[0:2] == ["bash", "-lc"]
         and launcher[2].endswith("exec " + " ".join(expected_command))
         and cargo_command.get("command_sha256") == cargo_result.get("command_sha256")
         and cargo_command.get("command_sha256") == hashlib.sha256(
             json.dumps(launcher, separators=(",", ":")).encode()).hexdigest(),
         "P132 prior inventory launcher identity changed")
    need(cargo_result.get("returncode") == 0
         and cargo_result.get("argv") == expected_command
         and cargo_result.get("stdout_sha256") == sha(inventory / "cargo.stdout")
         and cargo_result.get("stderr_sha256") == sha(inventory / "cargo.stderr"),
         "P132 prior inventory Cargo result does not match captured logs")
    helper_record_path = Path(prior_run.get("executed_helper_argv_path", "")).resolve(strict=True)
    need(helper_record_path.is_relative_to(ROOT)
         and sha(helper_record_path) == prior_run.get("executed_helper_argv_sha256"),
         "P132 prior executed-helper record identity changed")
    executed_helpers = prior_run.get("executed_helper_sources")
    need(isinstance(executed_helpers, list) and executed_helpers,
         "P132 prior run does not bind its executed helper sources")
    helper_refs = []
    for row in executed_helpers:
        helper_path = Path(row["path"]).resolve(strict=True)
        need(helper_path.is_relative_to(ROOT) and sha(helper_path) == row.get("sha256"),
             f"P132 prior executed helper source changed: {row.get('path')}")
        helper_refs.append(ref(helper_path))
    observer_test = ROOT / "target/recovery/P132-inventory-0001/p132_private.rs"
    archived_original = ROOT / "target/recovery/P132-inventory-0001/execute.rs.archived-original"
    injected_source = ROOT / "target/recovery/P132-inventory-0001/workspace/crates/wtime-runtime/src/execution/register/execute.rs"
    need(sha(observer_test) == reference.get("reusable_candidate_private_test_sha256")
         and sha(archived_original) == reference.get("archived_execute_original_sha256")
         and sha(injected_source) == reference.get("archived_execute_injected_sha256"),
         "P132 inventory observer inputs differ from the corrected source record")
    return {
        "golden": ref(golden_path),
        "reference": ref(reference_path),
        "fixture_identity_map": ref(fixture_map_path),
        "reader": ref(reader),
        "prior_run": ref(inventory / "reference.json"),
        "prior_cargo_result": ref(inventory / "cargo-result.json"),
        "prior_cargo_stdout": ref(inventory / "cargo.stdout"),
        "prior_cargo_stderr": ref(inventory / "cargo.stderr"),
        "prior_helper_execution_record": ref(helper_record_path),
        "prior_executed_helpers": helper_refs,
        "observer_test_source": ref(observer_test),
        "archived_original_execute_source": ref(archived_original),
        "instrumented_execute_source": ref(injected_source),
        "fixture_count": len(fixture_pins),
        "fixture_source_pins": dict(sorted(
            (Path(name).resolve().relative_to(ROOT).as_posix(), row["sha256"])
            for name, row in fixture_pins.items())),
        "reference_counts": {key: reference[key] for key in (
            "fixture_count", "total_body_count", "prepared_body_count", "declined_body_count",
            "pc_line_count", "constant_line_count", "memory_argument_line_count", "edge_line_count",
            "golden_line_count")},
    }


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P132 immediate predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT
         and manifest.get("mode") == "register",
         "P132 immediate predecessor is not the pinned Register archive")
    policy = manifest.get("execution_policy", {})
    need(policy.get("requested_mode") == "register" and policy.get("resolved_mode") == "register"
         and policy.get("protocol_modes") == ["decoded", "portable", "register"]
         and policy.get("protocol_sha256") == PROTOCOL_SHA256,
         "P132 predecessor protocol is not exactly Register")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P132 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P132 predecessor omits {crate} source files")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True
         and p001_manifest.get("smoke", {}).get("returncode") == 0,
         "immutable P001 archive has no successful correctness smoke")

    inventory = validate_inventory_inputs(manifest)
    prerequisites = {task.lower(): completed(task) for task in ("P065", "P131")}
    runtime_doc = {
        "runtime_source_map": source_map,
        "runtime_source_map_sha256": canonical_sha(source_map),
        "golden_inventory": inventory,
    }
    runtime_path = output / "runtime.json"
    write_new(runtime_path, runtime_doc)
    report = {
        "schema": "wtime-p132-freeze-v1",
        "status": "passed",
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_manifest_sha256_pinned": PREDECESSOR_MANIFEST_SHA256,
        "predecessor_mode": "register",
        "predecessor_commit": PREDECESSOR_COMMIT,
        "protocol_sha256": PROTOCOL_SHA256,
        "runtime_source_map": source_map,
        "runtime_source_map_sha256": canonical_sha(source_map),
        "runtime_source_file_count": len(source_map),
        "runtime_map": ref(runtime_path),
        "p001": {"manifest": ref(p001 / "manifest.json"),
                 "binary": ref(p001 / "binary/wtime-bench"),
                 "manifest_sha256_pinned": P001_MANIFEST_SHA256,
                 "binary_sha256_pinned": P001_BINARY_SHA256,
                 "correctness_checked": True},
        "prerequisites": prerequisites,
        "golden_inventory": inventory,
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = path.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def inventory_lines(path: Path) -> tuple[bytes, dict[str, int]]:
    text = path.read_bytes()
    lines = text.splitlines(keepends=True)
    selected = [line for line in lines if line.startswith(b"P132_INVENTORY|")]
    joined = b"".join(selected)
    decoded = [line.decode("utf-8").rstrip("\r\n") for line in selected]
    need(all(line.endswith(b"\n") for line in selected), "P132 inventory row lacks a line terminator")
    counts = {kind: sum(line.startswith(f"P132_INVENTORY|{kind}|".encode())
                        for line in selected) for kind in RECORD_KINDS}
    fixture_rows = [line.split("|", 2)[2] for line in decoded
                    if line.startswith("P132_INVENTORY|fixture|")]
    fixture_prepared = []
    fixture_total_prepared = 0
    fixture_total_declined = 0
    for row in fixture_rows:
        values = dict(field.split("=", 1) for field in row.split("|") if "=" in field)
        prepared_count = int(values["prepared_count"])
        fixture_prepared.append(prepared_count > 0)
        fixture_total_prepared += prepared_count
        fixture_total_declined += int(values["declined_count"])
    body_rows = [line for line in decoded if line.startswith("P132_INVENTORY|body|")]
    prepared = sum("|status=Prepared|" in line for line in body_rows)
    declined = sum(line.endswith("|status=None") for line in body_rows)
    counts.update({
        "pc_line_count": counts["pc"],
        "constant_line_count": counts["constant"],
        "memory_argument_line_count": counts["memory_argument"],
        "edge_line_count": counts["edge"],
        "fixture_count": len(fixture_rows),
        "prepared_fixture_count": sum(fixture_prepared),
        "total_body_count": len(body_rows),
        "total_prepared": prepared,
        "total_declined": declined,
        "fixture_total_prepared": fixture_total_prepared,
        "fixture_total_declined": fixture_total_declined,
    })
    return joined, counts


def validate_inventory(inventory_log: Path, frozen: dict[str, Any]) -> dict[str, Any]:
    inventory = frozen.get("golden_inventory", {})
    golden_ref = inventory.get("golden", {})
    golden_path = Path(golden_ref.get("path", "")).resolve(strict=True)
    need(golden_path.is_relative_to(ROOT), "P132 golden inventory is outside repository")
    golden_bytes = golden_path.read_bytes()
    actual_bytes, counts = inventory_lines(inventory_log)
    need(sha(golden_path) == INVENTORY_GOLDEN_SHA256 and len(golden_bytes.splitlines()) == INVENTORY_LINE_COUNT,
         "P132 frozen golden inventory identity changed")
    need(actual_bytes == golden_bytes,
         "P132 Register inventory rows differ byte-for-byte from the immutable predecessor inventory")
    expected_record_counts = {
        **EXPECTED_COUNTS,
        "fixture": 20,
        "body": 196,
        "pc": 805,
        "constant": 68,
        "memory_argument": 33,
        "edge": 10,
        "summary": 1,
        "fixture_total_prepared": 181,
        "fixture_total_declined": 15,
    }
    need(counts == expected_record_counts,
         f"P132 Register inventory record counts changed: {counts!r}")
    expected_fixtures = inventory.get("fixture_source_pins", {})
    need(len(expected_fixtures) == EXPECTED_COUNTS["fixture_count"],
         "P132 frozen fixture source pin count changed")
    for relative, digest in expected_fixtures.items():
        fixture = (ROOT / relative).resolve(strict=True)
        need(fixture.is_relative_to(ROOT) and sha(fixture) == digest,
             f"P132 source fixture no longer matches frozen pin: {relative}")
    return {
        "byte_for_byte_match": True,
        "golden": golden_ref,
        "source_log": ref(inventory_log),
        "matched_line_count": len(actual_bytes.splitlines()),
        "matched_sha256": hashlib.sha256(actual_bytes).hexdigest(),
        "counts": counts,
        "fixture_source_pin_count": len(expected_fixtures),
        "fixture_source_pins_sha256": canonical_sha(expected_fixtures),
    }


def check_build(path: Path, label: str, mode: str, features: list[str]) -> dict[str, Any]:
    record = load_json(path, f"{label} build budget")
    need(record.get("mode") == mode and record.get("features") == features,
         f"{label} build budget has the wrong feature configuration")
    need(record.get("source_stable") is True and record.get("budget_pass") is True
         and record.get("default_dependency_closure_pass") is True,
         f"{label} build budget failed source, hard-budget, or default-dependency policy")
    byte_count = record.get("stripped_bytes")
    need(type(byte_count) is int and 0 < byte_count <= MAX_BUILD_BYTES,
         f"{label} build budget has an invalid or over-limit stripped binary size")
    return {"artifact": ref(Path(path)), "mode": mode, "features": features,
            "stripped_bytes": byte_count, "budget_pass": True,
            "default_dependency_closure_pass": True}


def verify(output: Path, register_log: Path, inventory_log: Path, combined_log: Path,
           default_log: Path, default_build: Path, register_build: Path,
           freeze_report: Path) -> dict[str, Any]:
    output = start_output(output)
    frozen = load_json(freeze_report, "P132 freeze report")
    need(frozen.get("schema") == "wtime-p132-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P132 freeze report is invalid")
    need(frozen.get("predecessor_commit") == PREDECESSOR_COMMIT
         and frozen.get("predecessor_manifest_sha256_pinned") == PREDECESSOR_MANIFEST_SHA256
         and frozen.get("predecessor_mode") == "register"
         and frozen.get("protocol_sha256") == PROTOCOL_SHA256,
         "P132 freeze does not bind its exact immediate predecessor")
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(frozen.get("runtime_source_map")),
         "P132 frozen source map hash is invalid")
    runtime_ref = frozen.get("runtime_map", {})
    runtime_path = Path(runtime_ref.get("path", "")).resolve(strict=True)
    need(runtime_path.is_relative_to(ROOT / "target/verification")
         and sha(runtime_path) == runtime_ref.get("sha256"),
         "P132 frozen runtime map artifact is invalid")
    runtime_doc = load_json(runtime_path, "P132 frozen runtime map")
    need(runtime_doc.get("runtime_source_map") == frozen.get("runtime_source_map")
         and runtime_doc.get("golden_inventory") == frozen.get("golden_inventory"),
         "P132 runtime map differs from freeze report")

    register_log = Path(register_log).resolve(strict=True)
    inventory_log = Path(inventory_log).resolve(strict=True)
    combined_log = Path(combined_log).resolve(strict=True)
    default_log = Path(default_log).resolve(strict=True)
    need(inventory_log.is_relative_to(ROOT / "target/verification"),
         "P132 inventory log is outside the verification artifact store")
    require_tests(register_log, P132_TESTS, "full Register/reference")
    require_tests(combined_log, P132_TESTS, "full combined Compact/Register/reference")
    require_tests(default_log, (DEFAULT_TEST,), "default public policy")
    inventory = validate_inventory(inventory_log, frozen)
    builds = {
        "default": check_build(default_build, "default", "default", []),
        "register": check_build(register_build, "register", "optional-feature",
                                 ["wtime-runtime/register-executor"]),
    }
    report = {
        "schema": "wtime-p132-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "decision": "contract_only",
        "limitations": ["Preparation semantics, exact archived inventory, policy neutrality, and build budgets only; no timing or performance promotion is claimed."],
        "settings": {
            "register_features": ["register-executor", "execution-reference"],
            "combined_features": ["compact-executor", "register-executor", "execution-reference"],
            "default_features": [],
            "immediate_predecessor": "register",
        },
        "counts": {
            "p132_named_cases": len(P132_TESTS),
            "default_named_cases": 1,
            "inventory_lines": inventory["matched_line_count"],
            **inventory["counts"],
        },
        "inventory": inventory,
        "freeze_report": ref(Path(freeze_report)),
        "runtime_source_map_sha256": frozen["runtime_source_map_sha256"],
        "test_logs": {
            "register": ref(register_log),
            "register_inventory": ref(inventory_log),
            "combined": ref(combined_log),
            "default": ref(default_log),
        },
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="phase", required=True)
    freeze_parser = subparsers.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--output", type=Path, required=True)
    verify_parser.add_argument("--register-log", type=Path, required=True)
    verify_parser.add_argument("--inventory-log", type=Path, required=True)
    verify_parser.add_argument("--combined-log", type=Path, required=True)
    verify_parser.add_argument("--default-log", type=Path, required=True)
    verify_parser.add_argument("--default-build", type=Path, required=True)
    verify_parser.add_argument("--register-build", type=Path, required=True)
    verify_parser.add_argument("--freeze-report", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.phase == "freeze":
            result = freeze(args.output)
        else:
            result = verify(args.output, args.register_log, args.inventory_log,
                            args.combined_log, args.default_log, args.default_build,
                            args.register_build, args.freeze_report)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"P132 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
