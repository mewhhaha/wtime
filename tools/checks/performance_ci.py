#!/usr/bin/env python3
"""Portable correctness smoke and conservative reference-lane evidence helpers."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
from typing import Any, Callable

ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCHEMA = "wtime-p111-smoke-v1"
BUILD_SCHEMA = "wtime-p111-build-binding-v1"
BUILD_ARGV = ["cargo", "build", "--locked", "--offline", "-j", "1", "-p", "wtime-cli"]
REFERENCE_SCHEMA = "wtime-p111-reference-v1"
TREND_SCHEMA = "wtime-p111-trend-v1"
ORACLE_SOURCE = "tools/wtime-bench/run.py"
ORACLE_SHA256 = "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea"

# Fixed inputs from the existing P001 MVP workload set. Their WASM and WAT
# bytes are never rewritten by this tool.
FIXTURES = {
    "call_overhead": {
        "input": 100,
        "wat": "tools/wtime-bench/fixtures/call_overhead.wat",
        "wat_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
        "wasm": "tools/wtime-bench/fixtures/call_overhead.wasm",
        "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    },
    "integer_loop": {
        "input": 10000,
        "wat": "tools/wtime-bench/fixtures/integer_loop.wat",
        "wat_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
        "wasm": "tools/wtime-bench/fixtures/integer_loop.wasm",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    },
    "branch_calls": {
        "input": 5000,
        "wat": "tools/wtime-bench/fixtures/branch_calls.wat",
        "wat_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
        "wasm": "tools/wtime-bench/fixtures/branch_calls.wasm",
        "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    },
}
CORE_FIXED = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
RESULT_RE = re.compile(r"I32\((-?(?:0|[1-9][0-9]*))\)\n\Z")


class PerformanceCIError(ValueError):
    """Input, provenance, or smoke validation failure."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise PerformanceCIError(message)


def digest_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def file_sha(path: Path) -> str:
    return digest_bytes(path.read_bytes())


def normal(path: Path) -> dict[str, Any]:
    path = path.resolve(strict=True)
    need(path.is_file(), "expected a regular file: " + str(path))
    return {"path": str(path), "sha256": file_sha(path), "bytes": path.stat().st_size}


def verify_normal(value: Any) -> Path:
    need(isinstance(value, dict) and set(value) == {"path", "sha256", "bytes"},
         "file reference fields differ")
    need(isinstance(value["path"], str) and re.fullmatch(r"[0-9a-f]{64}", str(value["sha256"])),
         "file reference path/hash is malformed")
    need(type(value["bytes"]) is int and value["bytes"] > 0, "file reference length is invalid")
    path = Path(value["path"]).resolve(strict=True)
    need(path.is_file() and file_sha(path) == value["sha256"] and path.stat().st_size == value["bytes"],
         "file reference is stale: " + value["path"])
    return path


def current_core_sources(root: Path = ROOT) -> dict[str, str]:
    """Hash the same portable Rust source universe used by predecessor freezes."""
    root = root.resolve(strict=True)
    paths = {root / name for name in CORE_FIXED}
    crates = root / "crates"
    need(crates.is_dir(), "crates source tree is missing")
    for path in crates.rglob("*"):
        if path.is_file() and (path.suffix == ".rs" or path.name == "Cargo.toml"):
            paths.add(path)
    result = {}
    for path in sorted(paths):
        resolved = path.resolve(strict=True)
        need(resolved.is_relative_to(root) and resolved.is_file(), "core source escaped repository")
        result[resolved.relative_to(root).as_posix()] = file_sha(resolved)
    need(len(result) >= len(CORE_FIXED), "incomplete core source map")
    return result


def validate_build_binding(binding_path: Path, root: Path = ROOT,
                           binary_path: Path | None = None) -> dict[str, Any]:
    """Check an independently produced build receipt; never build implicitly."""
    binding_path = binding_path.resolve(strict=True)
    root = root.resolve(strict=True)
    need(binding_path.is_file(), "build binding is not a file")
    binding = json.loads(binding_path.read_text(encoding="utf-8"))
    need(isinstance(binding, dict) and set(binding) == {
        "schema", "build_argv", "build_exit_code", "binary", "core_sources"}, "build binding fields differ")
    need(binding["schema"] == BUILD_SCHEMA, "unknown build-binding schema")
    argv = binding["build_argv"]
    need(isinstance(argv, list) and argv and all(isinstance(arg, str) and arg for arg in argv),
         "build argv is absent or malformed")
    need(argv == BUILD_ARGV and type(binding["build_exit_code"]) is int and binding["build_exit_code"] == 0, "binding is not a successful default CLI build")
    sources = binding["core_sources"]
    need(isinstance(sources, dict) and sources and all(
        isinstance(name, str) and re.fullmatch(r"[0-9a-f]{64}", str(value))
        for name, value in sources.items()), "build source map is malformed")
    current = current_core_sources(root)
    need(sources == current, "build binding source map is stale or incomplete")
    binary = verify_normal(binding["binary"])
    need(binary.is_relative_to(root), "bound CLI binary escaped repository")
    if binary_path is not None:
        wanted = binary_path.resolve(strict=True)
        need(wanted == binary, "requested CLI binary differs from bound binary")
    return binding


def load_oracle(root: Path = ROOT) -> Callable[[str, int], int]:
    path = root / ORACLE_SOURCE
    need(file_sha(path) == ORACLE_SHA256, "independent P001 oracle source hash is stale")
    spec = importlib.util.spec_from_file_location("p111_bound_run_oracle", path)
    need(spec is not None and spec.loader is not None, "could not load bound run.py oracle")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    need(callable(getattr(module, "oracle", None)), "bound run.py has no oracle function")
    return module.oracle


def verify_fixtures(root: Path = ROOT) -> list[dict[str, Any]]:
    root = root.resolve(strict=True)
    oracle_path = root / ORACLE_SOURCE
    need(file_sha(oracle_path) == ORACLE_SHA256, "independent oracle source hash is stale")
    oracle = load_oracle(root)
    cases = []
    for name, spec in FIXTURES.items():
        wat_path = root / spec["wat"]
        wasm_path = root / spec["wasm"]
        need(file_sha(wat_path) == spec["wat_sha256"], name + " WAT hash mismatch")
        need(file_sha(wasm_path) == spec["wasm_sha256"], name + " Wasm hash mismatch")
        expected = oracle(name, spec["input"])
        need(type(expected) is int, name + " independent oracle returned a non-integer")
        cases.append({"case": name, "input": spec["input"], "expected": expected,
                      "wat": normal(wat_path), "wasm": normal(wasm_path),
                      "oracle_source": normal(oracle_path)})
    need(len(cases) == 3 and len({row["case"] for row in cases}) == 3,
         "P111 smoke must exercise exactly three distinct fixtures")
    return cases


def parse_value_i32(stdout: str) -> int:
    need(isinstance(stdout, str), "CLI stdout is not text")
    match = RESULT_RE.fullmatch(stdout)
    need(match is not None, "CLI stdout is not exactly one Value::I32 debug result")
    value = int(match.group(1))
    need(-(1 << 31) <= value < (1 << 31), "CLI result is outside i32")
    return value


def validate_smoke_report(report: Any, root: Path = ROOT) -> dict[str, Any]:
    need(isinstance(report, dict) and set(report) == {
        "schema", "status", "timing_evidence", "profile", "binary", "build_binding",
        "core_sources", "cases", "nonzero_invocations"}, "smoke report fields differ")
    need(report["schema"] == SMOKE_SCHEMA and report["status"] == "passed"
         and report["timing_evidence"] is False and report["profile"] == "mvp",
         "smoke report claims an unsupported status or setting")
    current = current_core_sources(root)
    need(report["core_sources"] == current, "smoke report core source identity is stale")
    binding_path = verify_normal(report["build_binding"])
    binding = validate_build_binding(binding_path, root, verify_normal(report["binary"]))
    need(report["binary"] == binding["binary"], "smoke report binary differs from build binding")
    records = report["cases"]
    need(isinstance(records, list) and len(records) == 3
         and [row.get("case") for row in records] == list(FIXTURES),
         "smoke case set/order differs")
    need(type(report["nonzero_invocations"]) is int and report["nonzero_invocations"] == 3,
         "smoke invocation count is zero or incomplete")
    expected_rows = {row["case"]: row for row in verify_fixtures(root)}
    for row in records:
        need(set(row) == {"case", "input", "expected", "actual", "profile", "argv",
                          "returncode", "stdout", "stderr", "fixture", "source",
                          "oracle_source", "invocations"}, "smoke case record fields differ")
        expected = expected_rows[row["case"]]
        need(row["input"] == expected["input"] and row["expected"] == expected["expected"]
             and row["actual"] == row["expected"] and row["profile"] == "mvp"
             and type(row["returncode"]) is int and row["returncode"] == 0
             and type(row["invocations"]) is int and row["invocations"] == 1
             and all(type(row[key]) is int for key in ("input", "expected", "actual")), "smoke result differs from independent oracle")
        need(row["fixture"] == expected["wasm"] and row["source"] == expected["wat"]
             and row["oracle_source"] == expected["oracle_source"], "smoke source binding differs")
        need(isinstance(row["argv"], list) and row["argv"] == [
            binding["binary"]["path"], "--profile", "mvp", "run", expected["wasm"]["path"],
            "run", "i32:" + str(expected["input"])], "smoke command differs")
        need(row["stdout"] == "I32(" + str(row["expected"]) + ")\n" and row["stderr"] == "",
             "smoke output is not the exact Value::I32 result")
    return {"cases": 3, "invocations": 3, "profile": "mvp", "timing_evidence": False}


def run_smoke(binary_path: Path, build_binding_path: Path, output_path: Path,
              root: Path = ROOT,
              run: Callable[..., Any] = subprocess.run) -> dict[str, Any]:
    root = root.resolve(strict=True)
    binary_path = binary_path.resolve(strict=True)
    output_path = output_path.resolve(strict=False)
    need(output_path.is_relative_to(root), "smoke output must stay inside repository")
    need(not output_path.exists(), "smoke output already exists")
    binding_path = build_binding_path.resolve(strict=True)
    binding = validate_build_binding(binding_path, root, binary_path)
    fixtures = verify_fixtures(root)
    core = current_core_sources(root)
    cases = []
    for fixture in fixtures:
        argv = [str(binary_path), "--profile", "mvp", "run", fixture["wasm"]["path"],
                "run", "i32:" + str(fixture["input"])]
        result = run(argv, cwd=root, text=True, capture_output=True, timeout=120, check=False)
        need(type(result.returncode) is int and result.returncode == 0,
             fixture["case"] + " CLI invocation failed")
        actual = parse_value_i32(result.stdout)
        need(actual == fixture["expected"], fixture["case"] + " disagrees with independent oracle")
        need(result.stderr == "", fixture["case"] + " emitted unexpected stderr")
        cases.append({"case": fixture["case"], "input": fixture["input"],
                      "expected": fixture["expected"], "actual": actual, "profile": "mvp",
                      "argv": argv, "returncode": result.returncode, "stdout": result.stdout,
                      "stderr": result.stderr, "fixture": fixture["wasm"],
                      "source": fixture["wat"], "oracle_source": fixture["oracle_source"],
                      "invocations": 1})
    report = {"schema": SMOKE_SCHEMA, "status": "passed", "timing_evidence": False,
              "profile": "mvp", "binary": binding["binary"],
              "build_binding": normal(binding_path), "core_sources": core,
              "cases": cases, "nonzero_invocations": len(cases)}
    validate_smoke_report(report, root)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
    return report


def finalized_statistics(completion_path: Path, statistics_path: Path,
                         root: Path = ROOT) -> dict[str, Any]:
    """Revalidate immutable full-lane evidence without rerunning its statistics."""
    import sys
    sys.path.insert(0, str(ROOT / "tools"))
    from verification.artifacts import ArtifactStore, digest, read_json
    from verification.contracts import identity, load_spec
    from verification.runner import Workflow

    completion = read_json(completion_path)
    need(completion.get("status") == "done" and len(completion.get("evidence", [])) == 1,
         "statistics require one finalized completion record")
    evidence = read_json(verify_normal(completion["evidence"][0]))
    need(evidence.get("schema") == "wtime-verification-evidence-v1"
         and evidence.get("passed") is True and evidence.get("synthetic") is False
         and evidence.get("task") == completion.get("task"), "invalid full-lane evidence")
    spec = load_spec(verify_normal(evidence["contract"]))
    need(spec["task"] == evidence["task"] and not spec.get("synthetic", False),
         "statistics contract is synthetic or belongs to another task")
    required = spec["completion"]["required_phases"]
    need(set(evidence["phases"]) == set(required)
         and {"freeze", "verify", "measure", "analyze"} <= set(required),
         "statistics need full frozen, verified, measured and analyzed evidence")
    store = ArtifactStore(Path(evidence["store"]))
    workflow = Workflow(root, evidence["store"])
    receipts = {phase: store.json(ref) for phase, ref in evidence["phases"].items()}
    for phase, receipt in receipts.items():
        need(receipt.get("schema") == "wtime-verification-phase-v1"
             and receipt.get("task") == evidence["task"] and receipt.get("phase") == phase
             and receipt.get("status") == "passed" and receipt.get("lane") == "full"
             and receipt.get("completion_eligible") is True and receipt.get("synthetic") is False,
             "statistics contain ineligible phase evidence")
        need(receipt["input_key"] == digest(receipt["inputs"]), "phase input checksum differs")
        need(receipt["inputs"]["definition"] == spec["stages"][phase], "phase declaration changed")
        for name, recorded in receipt["inputs"]["components"].items():
            need(recorded == identity(root, spec["identities"][name]), "statistics inputs changed: " + name)
        for dependency, ref in receipt["inputs"]["dependencies"].items():
            need(ref == evidence["phases"][dependency], "statistics dependency receipt differs")
        for ref in receipt["artifacts"].values():
            store.verify(ref)
        current_ref, _ = workflow._receipt(spec["task"], phase)
        need(current_ref == evidence["phases"][phase], "a newer phase supersedes the supplied completion")
        workflow._revalidate(spec, phase, receipt)
    bound = normal(statistics_path)
    need(bound in receipts["analyze"]["original_outputs"].values(),
         "statistics are not an output of the finalized analysis")
    statistics = read_json(statistics_path)
    need(statistics.get("schema") == "wtime-verification-statistics-v1"
         and statistics.get("status") == "passed" and statistics.get("timing_evidence") is True
         and statistics.get("statistics_computations") == 1
         and statistics.get("task") == evidence["task"], "invalid shared statistics")
    for ref in statistics["inputs"].values():
        verify_normal(ref)
    analysis = statistics.get("analysis", {})
    need(isinstance(analysis.get("per_run"), list) and len(analysis["per_run"]) >= 2
         and analysis.get("analysis", {}).get("minimum_pairs_per_group_per_run", 0) >= 30
         and analysis.get("analysis", {}).get("bootstrap_resamples", 0) >= 10000,
         "statistics do not satisfy the full paired protocol")
    groups: dict[tuple[Any, ...], set[int]] = {}
    seeds: dict[int, int] = {}
    for row in analysis["per_run"]:
        need(type(row.get("pairs")) is int and row["pairs"] >= 30
             and type(row.get("run")) is int and type(row.get("seed")) is int,
             "statistics contain an incomplete paired group")
        group = tuple(row.get(key) for key in ("workload", "stage", "input", "fixture"))
        need(all(value is not None for value in group), "statistics group identity is incomplete")
        runs = groups.setdefault(group, set())
        need(row["run"] not in runs, "duplicate statistics group/run")
        runs.add(row["run"])
        need(row["run"] not in seeds or seeds[row["run"]] == row["seed"],
             "statistics seed changed within a run")
        seeds[row["run"]] = row["seed"]
    need(len(seeds) == 2 and len(set(seeds.values())) == 2
         and all(runs == set(seeds) for runs in groups.values()),
         "statistics require two independent complete runs")
    decision = statistics.get("decision", {}).get("decision")
    need(decision in {"adopted", "rejected", "inconclusive"}
         and decision == evidence.get("outcome") == completion.get("outcome"),
         "statistics decision differs from completion")
    return {"completion": normal(completion_path), "evidence": normal(verify_normal(completion["evidence"][0])),
            "statistics": bound, "decision": decision, "receipts": receipts, "value": statistics}


def trend_lane(shared_statistics: Path | None = None, completion: Path | None = None,
               root: Path = ROOT) -> dict[str, Any]:
    if shared_statistics is None or completion is None:
        return {"schema": TREND_SCHEMA, "status": "unrun", "decision": "unrun",
                "reason": "finalized_statistics_absent", "timing_evidence": False}
    bound = finalized_statistics(completion, shared_statistics, root)
    return {"schema": TREND_SCHEMA, "status": "passed", "decision": "inconclusive",
            "reason": "shared_worker_trend_only", "timing_evidence": True,
            "completion": bound["completion"], "statistics": bound["statistics"],
            "recorded_decision": bound["decision"], "statistics_recomputed": False,
            "promotion_allowed": False}


def reference_lane(reference_config: Path | None = None,
                   shared_statistics: Path | None = None, completion: Path | None = None,
                   root: Path = ROOT) -> dict[str, Any]:
    """Accept a dedicated runner declaration only when frozen before measurement."""
    if reference_config is None:
        return {"status": "unrun", "decision": "unrun", "reason": "reference_configuration_absent",
                "timing_evidence": False}
    if shared_statistics is None or completion is None:
        return {"status": "unrun", "decision": "unrun", "reason": "shared_statistics_absent",
                "timing_evidence": False}
    config_path = reference_config.resolve(strict=True)
    config = json.loads(config_path.read_text(encoding="utf-8"))
    need(isinstance(config, dict) and set(config) == {
        "schema", "runner_id", "architecture", "exclusive_runner", "hardware"}
         and config.get("schema") == REFERENCE_SCHEMA,
         "reference configuration schema differs")
    need(config["exclusive_runner"] is True and isinstance(config["runner_id"], str)
         and bool(config["runner_id"]) and config["architecture"] in {"x86_64", "aarch64"},
         "reference runner must have an explicit exclusive identity and supported architecture")
    bound = finalized_statistics(completion, shared_statistics, root)
    measure = bound["receipts"]["measure"]
    config_ref = normal(config_path)
    need(config_path.is_relative_to(root.resolve()), "reference config escaped repository")
    relative = config_path.relative_to(root.resolve()).as_posix()
    need(any(component["files"].get(relative) == config_ref["sha256"]
             for component in measure["inputs"]["components"].values()),
         "reference declaration was not frozen into the measured inputs")
    variables = measure["inputs"]["environment"]["variables"]
    need(variables.get("WTIME_REFERENCE_RUNNER") == digest_bytes(config["runner_id"].encode()),
         "measured runner identity differs from dedicated reference declaration")
    report = json.loads(verify_normal(bound["value"]["inputs"]["report"]).read_text())
    hardware = report.get("metadata", {}).get("hardware", {})
    need(isinstance(config["hardware"], dict) and set(config["hardware"]) == {
        "cpu_model", "selected_cpu", "cpu_quota"}
         and type(config["hardware"]["selected_cpu"]) is int
         and all(hardware.get(key) == value for key, value in config["hardware"].items()),
         "measured hardware differs from dedicated reference declaration")
    return {"schema": REFERENCE_SCHEMA, "status": "passed", "decision": bound["decision"],
            "configuration": config_ref, "completion": bound["completion"],
            "statistics": bound["statistics"], "timing_evidence": True,
            "statistics_recomputed": False, "promotion_allowed": bound["decision"] == "adopted"}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lane", choices=("smoke", "trend", "reference"), required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--build-binding", type=Path, default=ROOT / "target/performance/P111/build.json")
    parser.add_argument("--reference-config", type=Path)
    statistics_input = parser.add_mutually_exclusive_group()
    statistics_input.add_argument("--shared-statistics", type=Path)
    statistics_input.add_argument("--statistics-artifact", help="analysis output name from the completion record")
    parser.add_argument("--completion", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.statistics_artifact:
        need(args.completion is not None and args.lane != "smoke", "artifact lookup requires a completion record")
        import sys
        sys.path.insert(0, str(ROOT / "tools"))
        from verification.artifacts import ArtifactStore, read_json
        completed = read_json(args.completion)
        evidence = read_json(verify_normal(completed["evidence"][0]))
        analysis = ArtifactStore(Path(evidence["store"])).json(evidence["phases"]["analyze"])
        need(args.statistics_artifact in analysis["original_outputs"], "analysis statistics output is missing")
        args.shared_statistics = verify_normal(analysis["original_outputs"][args.statistics_artifact])
    if args.lane == "smoke":
        binding = validate_build_binding(args.build_binding)
        binary = args.binary or Path(binding["binary"]["path"])
        report = run_smoke(binary, args.build_binding, args.output)
    else:
        report = (reference_lane(args.reference_config, args.shared_statistics, args.completion)
                  if args.lane == "reference" else trend_lane(args.shared_statistics, args.completion))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            json.dump(report, stream, indent=2, sort_keys=True, allow_nan=False)
            stream.write("\n")
    print(json.dumps({"status": report["status"], "lane": args.lane,
                      "output": str(args.output.resolve()), "timing_evidence": report["timing_evidence"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
