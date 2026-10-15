"""Source-bound P080 freeze, prior-evidence checks, and specialist comparisons.

This workflow does not build executables. It reuses the sealed P080 predecessor,
the source-current P067 decoded adapter, and the independent P109/P067 receipts.
The focused comparator validates or encodes missing fixture Wasm into its fresh
attempt output; the committed/private fixture sources remain unchanged.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
PRIVATE_ROOT = ROOT / "target/performance/P080-focused-source"
PREDECESSOR = ROOT / "target/performance/P080-before-complete"
THREAD_MANIFEST = ROOT / "docs/benchmarks/performance/P080-threads-selection.json"
CONTINUATION_MANIFEST = ROOT / "docs/benchmarks/performance/P080-continuations-selection.json"
P067_REPORT = ROOT / "target/verification/runs/P067/verify/attempt-0001/decoded/report.json"
P067_RECEIPT = ROOT / "target/verification/runs/P067/verify/attempt-0001/receipt.json"
P067_PREDECESSOR = ROOT / "target/performance/P067-before"
P067_BUILD = ROOT / "target/performance/P067-builds/decoded"
P067_BINARY = ROOT / "target/performance/P067-builds/decoded/binary/wtime-bench"
P109_RECEIPT = ROOT / "target/verification/runs/P109/verify/attempt-0004/receipt.json"
P080_FAMILY_SUMMARY = ROOT / "target/performance/P080-families-0004/summary.json"
P080_FAMILY_DIR = ROOT / "target/performance/P080-families-0004"
THREAD_SEEDS = (80021, 80022)
CONTINUATION_SEEDS = (80031, 80032)
THREAD_CPUS = "2,3"
CPU = 2

sys.path[:0] = [str(ROOT / "tools"), str(ROOT / "tools/checks"),
                str(ROOT / "tools/wtime-bench"), str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore, need, normal, read_json, verify_normal, write_new


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _private_modules(root: Path):
    """Import the private harness so source enumeration is rooted there."""
    bench = (root / "tools/wtime-bench").resolve(strict=True)
    tools = (root / "tools").resolve(strict=True)
    need(bench.is_relative_to(root.resolve()) and tools.is_relative_to(root.resolve()),
         "private harness paths escape the declared source root")
    for item in (str(bench), str(tools)):
        while item in sys.path:
            sys.path.remove(item)
        sys.path.insert(0, item)
    # These module names are benchmark-local and are not imported by this file.
    import baseline
    import compare
    import focused_compare
    import selection_manifest
    for name, module in (("baseline", baseline), ("compare", compare),
                         ("focused_compare", focused_compare),
                         ("selection_manifest", selection_manifest)):
        origin = Path(module.__file__).resolve(strict=True)
        need(origin.is_relative_to(bench),
             name + " was imported from outside the private harness: " + str(origin))
    return baseline, compare, focused_compare, selection_manifest


def _core_sources(root: Path) -> dict[str, str]:
    names = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
    for crate in (root / "crates").glob("*"):
        if not crate.is_dir():
            continue
        names.update(path.relative_to(root).as_posix() for path in crate.rglob("Cargo.toml") if path.is_file())
        names.update(path.relative_to(root).as_posix() for path in crate.rglob("*.rs") if path.is_file())
    result = {}
    for name in sorted(names):
        path = (root / name).resolve(strict=True)
        need(path.is_relative_to(root.resolve()) and path.is_file(), "core input escapes root: " + name)
        result[name] = sha(path)
    return result


def _archive_core(manifest_path: Path) -> tuple[dict, dict[str, str]]:
    manifest = read_json(manifest_path)
    source_map = {}
    archive_root = manifest_path.parent
    for archive_name, record in manifest.get("files", {}).items():
        if not isinstance(record, dict) or record.get("role") != "source":
            continue
        name = record.get("source_path")
        if (not isinstance(name, str) or not (
                name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
                or (name.startswith("crates/") and
                    (name.endswith(".rs") or name.endswith("/Cargo.toml"))))):
            continue
        need(name not in source_map, "duplicate predecessor source path: " + name)
        source_map[name] = record.get("sha256")
        archived = (archive_root / archive_name).resolve(strict=True)
        need(archived.is_relative_to(archive_root.resolve()) and archived.is_file()
             and sha(archived) == record.get("sha256"),
             "predecessor source bytes differ: " + name)
    need(source_map, "predecessor manifest has no core source map")
    return manifest, source_map


def _private_focused_sources(root: Path) -> dict[str, str]:
    _, _, focused_compare, _ = _private_modules(root)
    result = focused_compare.sources()
    need(isinstance(result, dict) and result, "focused comparison source map is empty")
    for name, digest in result.items():
        path = (root / name).resolve(strict=True)
        need(path.is_relative_to(root.resolve()) and sha(path) == digest,
             "focused source identity is stale or escapes private root: " + name)
    return result


def freeze(output: Path, private_root: Path = PRIVATE_ROOT,
           predecessor: Path = PREDECESSOR) -> dict:
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    baseline, _, _, selection_manifest = _private_modules(private_root)
    archive = baseline.verify_archive(predecessor)
    need(archive.get("mode") == "decoded", "P080 predecessor must be decoded")
    manifest_path = predecessor / "manifest.json"
    _, archived_core = _archive_core(manifest_path)
    current_core = _core_sources(ROOT)
    private_core = _core_sources(private_root)
    need(current_core == archived_core, "current runtime core differs from the P080 predecessor")
    need(private_core == archived_core, "private focused runtime core differs from the P080 predecessor")
    candidate_sources = _private_focused_sources(private_root)
    # The full focused source projection must bind all core source paths.
    need(all(candidate_sources.get(name) == digest for name, digest in private_core.items()),
         "focused candidate source projection omits or changes core sources")
    output.parent.mkdir(parents=True, exist_ok=True)
    thread_freeze = output.parent / "threads-freeze.json"
    continuation_freeze = output.parent / "continuations-freeze.json"
    # selection_manifest owns its narrower four-crate core set. The workflow
    # above has already checked the broader Rust/Cargo map, including wtime-cli.
    selection_manifest.freeze(THREAD_MANIFEST, manifest_path, thread_freeze,
                              registry_path=private_root / "tools/wtime-bench/workloads.json")
    selection_manifest.freeze(CONTINUATION_MANIFEST, manifest_path, continuation_freeze,
                              registry_path=private_root / "tools/wtime-bench/workloads.json")
    candidate = _validate_p067_candidate(private_root)
    value = {
        "schema": "wtime-p080-freeze-v1", "task": "P080", "status": "passed",
        "predecessor": normal(manifest_path), "predecessor_mode": "decoded",
        "core_sources": archived_core, "current_core_sources": current_core,
        "private_core_sources": private_core, "candidate_sources": candidate_sources,
        "thread_selection": normal(THREAD_MANIFEST),
        "thread_freeze": normal(thread_freeze),
        "continuation_selection": normal(CONTINUATION_MANIFEST),
        "continuation_freeze": normal(continuation_freeze),
        "p067_decoded_report": normal(P067_REPORT), "p067_candidate_binary": candidate["binary"],
        "p067_candidate_adapter": candidate["adapter"],
        "timing_evidence": False,
    }
    write_new(output, value)
    return value


def _validate_p109_receipt(receipt_path: Path, current_core: dict[str, str]) -> dict:
    import p109
    import p109_coverage
    receipt = read_json(receipt_path)
    p109_coverage.validate_receipt_sources(receipt, p109.sources())
    store = ArtifactStore(ROOT / "target/verification")
    for reference in receipt.get("artifacts", {}).values():
        store.verify(reference)
    commands = {item["name"]: item for item in receipt.get("commands", [])}
    tests = receipt.get("inputs", {}).get("definition", {}).get("tests", [])
    need(tests, "P109 verification receipt has no named test groups")
    from verification.runner import test_results
    names = set()
    passed = failed = ignored = 0
    for group in tests:
        command = commands.get(group.get("command"))
        need(command is not None and command.get("exit_code") == 0,
             "P109 named test command failed or is absent")
        text = store.read(command["stdout"]).decode() + "\n" + store.read(command["stderr"]).decode()
        observed = test_results(text, group)
        need(observed == receipt.get("tests", {}).get(group["command"]),
             "P109 test results differ from content-addressed logs")
        names.update(observed.get("names", []))
        passed += observed.get("passed", 0)
        failed += observed.get("failed", 0)
        ignored += observed.get("ignored", 0)
    need(passed >= 781 and failed == 0 and ignored == 0,
         "P109 current-source full conformance/hardening tests are incomplete")
    return {"receipt": normal(receipt_path), "passed": passed, "failed": failed,
            "ignored": ignored, "names": sorted(names), "core_sources": current_core}


def _validate_p067_candidate(private_root: Path = PRIVATE_ROOT) -> dict:
    _private_modules(private_root)
    import p123_adapter
    module_path = Path(p123_adapter.__file__).resolve(strict=True)
    expected_root = (private_root / "tools/wtime-bench").resolve(strict=True)
    need(module_path.is_relative_to(expected_root),
         "P067 adapter helper was imported from outside the frozen private harness")
    adapter = p123_adapter.verify(P067_BUILD, P067_PREDECESSOR)
    need(adapter.get("mode") == "decoded" and adapter.get("legacy_api") is False,
         "P067 production comparison adapter is not the decoded protocol build")
    binary = normal(P067_BINARY)
    need(adapter.get("binary_sha256") == binary["sha256"],
         "P067 retained measured binary differs from its adapter attestation")
    need(adapter.get("baseline_manifest_sha256") == normal(P067_PREDECESSOR / "manifest.json")["sha256"],
         "P067 adapter does not bind its frozen predecessor")
    return {"binary": binary, "adapter": normal(P067_BUILD / "manifest.json"),
            "record": adapter}


def _validate_p067(current_core: dict[str, str], private_root: Path = PRIVATE_ROOT) -> dict:
    import p143
    from verification.runner import test_results

    report = read_json(P067_REPORT)
    counts = p143.validate_tier(report, None, "decoded")
    report_sources = report.get("implementation_sources", {})
    need(isinstance(report_sources, dict) and report_sources,
         "P067 decoded report lacks implementation source identities")
    for name, digest in report_sources.items():
        path = (ROOT / name).resolve(strict=True)
        need(path.is_relative_to(ROOT.resolve()) and path.is_file() and sha(path) == digest,
             "P067 decoded tier report source is stale or escapes the repository: " + name)
    report_runtime_sources = {
        name for name in current_core
        if name in ("Cargo.toml", "Cargo.lock") or any(
            name.startswith("crates/" + crate + "/")
            for crate in ("wtime-types", "wtime-decode", "wtime-validate", "wtime-runtime"))
    }
    need(report_runtime_sources <= set(report_sources),
         "P067 decoded report omits source files for one or more runtime crates")
    receipt = read_json(P067_RECEIPT)
    need(receipt.get("schema") == "wtime-verification-phase-v1"
         and receipt.get("task") == "P067" and receipt.get("phase") == "verify"
         and receipt.get("lane") == "full" and receipt.get("status") == "passed"
         and receipt.get("synthetic") is False,
         "P067 full verification receipt is missing or synthetic")
    recorded_runtime = receipt.get("inputs", {}).get("components", {}).get(
        "runtime_sources", {}).get("files")
    need(recorded_runtime == current_core,
         "P067 test receipt was compiled from a different runtime source map")
    outputs = receipt.get("original_outputs", {})
    need(outputs.get("decoded/report.json") == normal(P067_REPORT),
         "P067 receipt does not bind its decoded tier report")
    store = ArtifactStore(ROOT / "target/verification")
    for reference in receipt.get("artifacts", {}).values():
        store.verify(reference)
    commands = {item.get("name"): item for item in receipt.get("commands", [])}
    tests = receipt.get("inputs", {}).get("definition", {}).get("tests", [])
    observed_names = set()
    totals = {"passed": 0, "failed": 0, "ignored": 0}
    for group in tests:
        command = commands.get(group.get("command"))
        need(command is not None and command.get("exit_code") == 0,
             "P067 focused test command failed or is absent")
        text = store.read(command["stdout"]).decode() + "\n" + store.read(command["stderr"]).decode()
        observed = test_results(text, group)
        need(observed == receipt.get("tests", {}).get(group["command"]),
             "P067 named outcomes differ from retained command logs")
        observed_names.update(name.rsplit("::", 1)[-1].rsplit(".", 1)[-1]
                              for name in observed.get("names", []))
        for key in totals:
            totals[key] += observed.get(key, 0)
    required_names = {
        "test_register_route_passes_mode_guard_and_reaches_baseline_validation",
        "test_reference_and_unknown_modes_fail_before_any_preflight_io",
        "test_small_boolean_or_invalid_pair_counts_fail_before_io",
        "test_workload_subset_is_rejected_before_io",
        "p066_representative_fresh_store_and_hot_cache_equivalence",
        "p066_representative_source_fuel_prefix_equivalence",
        "p066_representative_cancellation_and_error_memory",
    }
    need(required_names <= observed_names and totals == {"passed": 7, "failed": 0, "ignored": 0},
         "P067 focused source-current tests are incomplete")
    candidate = _validate_p067_candidate(private_root)
    adapter_core = {name.removeprefix("source/"): digest
                    for name, digest in candidate["record"].get("frozen_core_identities", {}).items()
                    if name.endswith(".rs") or name.endswith("/Cargo.toml")}
    current_adapter_core = {name: digest for name, digest in current_core.items()
                            if name.startswith(("crates/wtime-types/", "crates/wtime-decode/",
                                                "crates/wtime-validate/", "crates/wtime-runtime/"))}
    need(adapter_core == current_adapter_core,
         "P067 measured adapter core differs from the source-current runtime")
    return {"report": normal(P067_REPORT), "receipt": normal(P067_RECEIPT),
            "binary": candidate["binary"], "adapter": candidate["adapter"],
            "focused_tests": {"names": sorted(observed_names), **totals},
            "counts": counts, "source_current": True}


def _validate_prior_family(summary_path: Path, family_dir: Path, candidate_sha: str) -> dict:
    summary = read_json(summary_path)
    need(summary.get("schema") == "wtime-p080-independent-families-v1"
         and summary.get("status") == "passed" and summary.get("source_stable") is True
         and summary.get("timing_evidence") is True and summary.get("mode") == "decoded",
         "prior P080 family summary is not a completed decoded measurement")
    root = Path(summary.get("private_source_root", "")).resolve(strict=True)
    need(root.is_dir(), "prior P080 private source root is missing")
    for name, digest in summary.get("source_inputs", {}).items():
        path = (root / name).resolve(strict=True)
        need(path.is_relative_to(root) and path.is_file() and sha(path) == digest,
             "prior P080 summary source map is stale: " + name)
    rows = summary.get("reports", [])
    need({row.get("family") for row in rows} == {"gc-exceptions", "simd"} and len(rows) == 2,
         "prior P080 family summary is incomplete")
    report_refs = {}
    for row in rows:
        family = row["family"]
        ref = verify_normal(row["report"])
        report = read_json(ref)
        meta = report.get("metadata", {})
        settings = meta.get("settings", {})
        need(report.get("passed") is True and report.get("source_stable") is True,
             "prior P080 family report failed: " + family)
        need(settings.get("candidate_mode") == "decoded" and settings.get("baseline_mode") == "decoded"
             and settings.get("instrumented") is False,
             "prior P080 family report used an incompatible mode")
        need(meta.get("candidate_binary_sha256") == candidate_sha,
             "prior P080 family report uses a different candidate binary")
        source_map = meta.get("candidate_sources")
        need(isinstance(source_map, dict) and bool(source_map),
             "prior P080 family report source map is missing")
        for name, digest in source_map.items():
            path = (root / name).resolve(strict=True)
            need(path.is_relative_to(root) and path.is_file() and sha(path) == digest,
                 "prior P080 family candidate source is stale: " + name)
            if name in summary["source_inputs"]:
                need(summary["source_inputs"][name] == digest,
                     "prior P080 summary/report source maps conflict: " + name)
        runs = report.get("runs", [])
        need(len(runs) == 2 and all(isinstance(item.get("pairs"), list) and item["pairs"] for item in runs),
             "prior P080 family report lacks two nonempty measured runs")
        expected_pairs_per_run = 6_750 if family == "gc-exceptions" else 17_100
        need(all(len(item["pairs"]) == expected_pairs_per_run for item in runs),
             "prior P080 family report has incomplete paired coverage: " + family)
        report_refs[family] = normal(ref)
    return {"summary": normal(summary_path), "reports": report_refs,
            "private_source_root": str(root), "source_inputs": summary["source_inputs"],
            "decision": summary.get("decision")}


def verify(freeze_path: Path, output: Path, private_root: Path = PRIVATE_ROOT) -> dict:
    import p109
    record = read_json(freeze_path)
    need(record.get("schema") == "wtime-p080-freeze-v1" and record.get("status") == "passed",
         "P080 frozen selection is missing")
    verify_normal(record["predecessor"])
    verify_normal(record["thread_selection"])
    verify_normal(record["thread_freeze"])
    verify_normal(record["continuation_selection"])
    verify_normal(record["continuation_freeze"])
    _, _, _, selection_manifest = _private_modules(private_root)
    selection_manifest.verify_frozen(THREAD_MANIFEST, Path(record["thread_freeze"]["path"]),
                                     registry_path=private_root / "tools/wtime-bench/workloads.json")
    selection_manifest.verify_frozen(CONTINUATION_MANIFEST, Path(record["continuation_freeze"]["path"]),
                                     registry_path=private_root / "tools/wtime-bench/workloads.json")
    current_core = _core_sources(private_root)
    need(current_core == record.get("core_sources"), "P080 frozen runtime core changed")
    candidate_sources = _private_focused_sources(private_root)
    need(candidate_sources == record.get("candidate_sources"), "P080 private focused harness changed after freeze")
    corpus = p109.validate_corpus()
    p109_checks = _validate_p109_receipt(P109_RECEIPT, p109.sources())
    p067_checks = _validate_p067(current_core, private_root)
    family = _validate_prior_family(P080_FAMILY_SUMMARY, P080_FAMILY_DIR,
                                    p067_checks["binary"]["sha256"])
    value = {"schema": "wtime-p080-prior-validation-v1", "task": "P080",
             "status": "passed", "source_stable": True, "timing_evidence": False,
             "freeze": normal(freeze_path), "p109_corpus": corpus,
             "p109_full_verification": p109_checks, "p067_decoded": p067_checks,
             "previous_family_reports": family,
             "limitations": ["P109 and P067 checks are reused source-current receipts.",
                 "GC/exception and SIMD reports are preserved prior timing evidence; this phase collected no samples.",
                 "No unsupported Wasmtime comparator is assigned a ratio."]}
    write_new(output, value)
    return value


def _ensure_semantic_store_link(private_root: Path) -> dict:
    target_dir = private_root / "target"
    target_dir.mkdir(exist_ok=True)
    link = target_dir / "verification"
    store = ROOT / "target/verification"
    if link.is_symlink():
        need(link.resolve(strict=True) == store.resolve(strict=True),
             "private semantic store symlink points elsewhere")
    elif link.exists():
        raise ValueError("private target/verification exists and is not the shared store symlink")
    else:
        link.symlink_to(store.resolve(strict=True), target_is_directory=True)
    return {"path": str(link), "target": str(store.resolve(strict=True)),
            "symlink": True}


def _validate_comparison(report_path: Path, family: str, seeds: tuple[int, int],
                         candidate_sources: dict[str, str], manifest_path: Path,
                         frozen_path: Path, candidate_binary: dict,
                         semantic_receipt: dict) -> dict:
    report = read_json(report_path)
    need(report.get("schema") == "wtime-focused-comparison-v1"
         and report.get("lane") == "candidate" and report.get("passed") is True
         and report.get("source_stable") is True,
         "focused " + family + " comparison did not pass")
    settings = report.get("metadata", {}).get("settings", {})
    need(settings.get("instrumented") is False and settings.get("baseline_mode") == "decoded"
         and settings.get("candidate_mode") == "decoded",
         "focused comparison mode/instrumentation differs")
    need(settings.get("profile") == "release" and settings.get("fuel_policy") is False,
         "focused comparison profile or fuel policy differs")
    recorded = report.get("metadata", {}).get("candidate_sources")
    need(recorded == candidate_sources, "focused comparison source map differs from frozen private harness")
    declaration = read_json(manifest_path)
    need(report.get("declaration") == declaration,
         "focused comparison does not bind the complete frozen selection")
    identities = report.get("identities", {})
    expected_refs = {
        "baseline_manifest": normal(PREDECESSOR / "manifest.json"),
        "candidate": candidate_binary,
        "selection_manifest": normal(manifest_path),
        "selection_freeze": normal(frozen_path),
        "semantic_receipt": semantic_receipt,
    }
    for name, expected in expected_refs.items():
        actual = identities.get(name)
        need(_matches_comparison_ref(actual, expected),
             "focused comparison identity differs for " + name)
    need(report.get("metadata", {}).get("candidate_binary_sha256") == candidate_binary["sha256"],
         "focused comparison candidate binary differs from P067 adapter")
    baseline_identity = identities.get("baseline")
    expected_baseline = normal(PREDECESSOR / "binary/wtime-bench")
    need(expected_baseline["sha256"] == baseline_expected_binary_sha(PREDECESSOR),
         "frozen predecessor executable differs from its manifest")
    need(_matches_comparison_ref(baseline_identity, expected_baseline),
         "focused comparison baseline binary differs from the frozen predecessor")
    need(report.get("metadata", {}).get("baseline_binary_sha256") == baseline_identity["sha256"],
         "focused comparison baseline metadata differs from its executable identity")
    hardware = report.get("metadata", {}).get("hardware", {})
    need(hardware.get("selected_cpu") == CPU,
         "focused comparison ran outside its declared single CPU")
    expected_thread_cpus = [int(value) for value in THREAD_CPUS.split(",")]
    need(settings.get("thread_cpu_affinity") ==
         (expected_thread_cpus if family == "threads" else None),
         "focused thread affinity differs from the predeclared policy")
    expected_groups = {(row["id"], stage) for row in declaration["rows"]
                       for stage in row["stages"]}
    need(expected_groups, "frozen selection has no workload stages")
    runs = report.get("runs", [])
    need(len(runs) == 2 and tuple(row.get("seed") for row in runs) == seeds,
         "focused comparison must contain both predeclared independent seeds")
    for run_id, run in enumerate(runs):
        need(run.get("run_id") == run_id,
             "focused comparison run identifiers are not the two declared independent runs")
        pairs = run.get("pairs")
        need(isinstance(pairs, list) and len(pairs) == 30 * len(expected_groups),
             "focused run does not contain 30 pairs for every selected workload stage")
        observed_groups = {(pair.get("workload"), pair.get("stage")) for pair in pairs}
        need(observed_groups == expected_groups,
             "focused run workload/stage products differ from the complete selection")
        products = report.get("cold_products", {}).get("groups", [])
        product_ids = {(group.get("sample_workload"), group.get("stage")): group.get("product_sha256")
                       for group in products}
        need(len(products) == len(expected_groups) and set(product_ids) == expected_groups
             and report["cold_products"].get("aliases") == 0,
             "focused cold-product plan omits, aliases, or substitutes selected products")
        need(len({pair.get("pair_id") for pair in pairs}) == len(pairs),
             "focused run contains duplicate pair identifiers")
        for workload, stage in expected_groups:
            product_pairs = [pair for pair in pairs
                             if pair.get("workload") == workload and pair.get("stage") == stage]
            need(len(product_pairs) == 30 and len({pair.get("pair_id") for pair in product_pairs}) == 30,
                 "focused workload stage lacks 30 unique paired observations")
            need({tuple(pair.get("order", [])) for pair in product_pairs}
                 == {("baseline", "candidate"), ("candidate", "baseline")},
                 "focused workload stage did not alternate both randomized side orders")
            for pair in product_pairs:
                need(pair.get("order") in (["baseline", "candidate"], ["candidate", "baseline"])
                     and pair.get("baseline") is not None and pair.get("candidate") is not None
                     and pair["baseline"].get("result") == pair["candidate"].get("result")
                     == pair.get("expected")
                     and pair.get("product_sha256") == product_ids[(workload, stage)],
                     "focused observation has a result or oracle mismatch")
    counts = report.get("counts", {})
    expected_pair_events = 2 * 30 * len(expected_groups)
    need(counts.get("unique_pair_count") == expected_pair_events
         and counts.get("logical_pair_references") == expected_pair_events
         and counts.get("extra_samples") == 0,
         "focused pair totals differ from the complete two-run contract")
    return {"report": normal(report_path), "family": family,
            "seeds": [run["seed"] for run in runs],
            "runs": len(runs), "groups_per_run": len(expected_groups),
            "pair_count_per_group_per_run": 30,
            "pair_count_per_run": len(runs[0]["pairs"]),
            "counts": counts, "settings": settings}


def baseline_expected_binary_sha(archive_path: Path) -> str:
    manifest = read_json(archive_path / "manifest.json")
    record = manifest.get("files", {}).get("binary/wtime-bench")
    need(isinstance(record, dict) and record.get("role") == "release_benchmark_executable",
         "frozen predecessor lacks its release benchmark executable")
    return record.get("sha256")


def _matches_comparison_ref(actual: object, expected: dict) -> bool:
    if not isinstance(actual, dict) or set(actual) != {"path", "sha256"}:
        return False
    try:
        path = Path(actual["path"]).resolve(strict=True)
        expected_path = Path(expected["path"]).resolve(strict=True)
        return (path == expected_path and path.is_file()
                and actual["sha256"] == expected["sha256"]
                and sha(path) == actual["sha256"])
    except (OSError, TypeError, ValueError):
        return False


def _run_one(name: str, family: str, manifest: Path, frozen: Path,
             seeds: tuple[int, int], output: Path, *, private_root: Path,
             baseline: Path, candidate: Path, semantic_receipt: Path) -> tuple[list[str], dict]:
    script = private_root / "tools/wtime-bench/focused_compare.py"
    command = [sys.executable, "-B", str(script), "--manifest", str(manifest),
               "--baseline", str(baseline), "--candidate", str(candidate),
               "--output", str(output), "--frozen", str(frozen),
               "--semantic-receipt", str(semantic_receipt),
               "--candidate-mode", "decoded", "--run-seeds", str(seeds[0]), str(seeds[1]),
               "--cpu", str(CPU)]
    if family == "threads":
        command.extend(["--thread-cpus", THREAD_CPUS])
    return command, {"name": name, "family": family, "seeds": list(seeds),
                     "cpu": CPU, "thread_cpus": THREAD_CPUS if family == "threads" else None}


def _retain_measurement_artifacts(output: Path, family_sources: dict[str, str]) -> dict:
    store = ArtifactStore(ROOT / "target/verification")
    inventory = {}

    def retain_tree(prefix: str, root: Path):
        root = root.resolve(strict=True)
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            resolved = path.resolve(strict=True)
            need(resolved.is_relative_to(root), "measurement artifact escaped its owned output root")
            key = prefix + "/" + path.relative_to(root).as_posix()
            inventory[key] = store.add(resolved)

    for name in ("threads", "continuations", "commands", "executed"):
        retain_tree("focused/" + name, output / name)
    retain_tree("prior/gc-exceptions", P080_FAMILY_DIR / "gc-exceptions")
    retain_tree("prior/simd", P080_FAMILY_DIR / "simd")
    inventory["prior/summary.json"] = store.add(P080_FAMILY_SUMMARY)
    prior_root = Path(read_json(P080_FAMILY_SUMMARY)["private_source_root"]).resolve(strict=True)
    need(prior_root.is_relative_to(ROOT), "prior family source root escapes the repository")
    for name, digest in sorted(family_sources.items()):
        path = (prior_root / name).resolve(strict=True)
        need(path.is_relative_to(prior_root) and path.is_file() and sha(path) == digest,
             "prior source changed before artifact retention: " + name)
        inventory["prior/source/" + name] = store.add(path, expected=digest)
    inventory["candidate/P067-decoded-binary"] = store.add(P067_BINARY)
    write_new(output / "artifacts.json", inventory)
    return {"inventory": normal(output / "artifacts.json"),
            "entry_count": len(inventory), "store": str(store.root)}


def measure(freeze_path: Path, verify_dir: Path, output: Path,
            private_root: Path = PRIVATE_ROOT) -> dict:
    freeze_record = read_json(freeze_path)
    need(freeze_record.get("schema") == "wtime-p080-freeze-v1"
         and freeze_record.get("status") == "passed",
         "P080 selection freeze is missing")
    for key in ("predecessor", "thread_selection", "thread_freeze",
                "continuation_selection", "continuation_freeze",
                "p067_decoded_report", "p067_candidate_binary", "p067_candidate_adapter"):
        verify_normal(freeze_record[key])
    preflight = read_json(verify_dir / "prior.json")
    need(preflight.get("schema") == "wtime-p080-prior-validation-v1"
         and preflight.get("status") == "passed" and preflight.get("source_stable") is True,
         "P080 prior-evidence verification did not pass")
    candidate_sources = _private_focused_sources(private_root)
    need(candidate_sources == freeze_record.get("candidate_sources"),
         "private focused harness differs from frozen source identity")
    candidate = verify_normal(freeze_record["p067_candidate_binary"])
    need(normal(P067_BINARY) == freeze_record["p067_candidate_binary"],
         "P067 production adapter binary changed after freeze")
    semantic_receipt = verify_dir / "receipt.json"
    need(semantic_receipt.is_file(), "full P080 verify receipt missing for semantic preflight")
    semantic_ref = normal(semantic_receipt)
    symlink = _ensure_semantic_store_link(private_root)
    baseline = PREDECESSOR
    need(normal(baseline / "manifest.json") == freeze_record["predecessor"],
         "P080 predecessor archive differs from frozen selection")
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "executed").mkdir()
    shutil.copyfile(Path(__file__).resolve(), output / "executed/p080_workflow.py")
    shutil.copyfile(private_root / "tools/wtime-bench/focused_compare.py",
                    output / "executed/focused_compare.py")
    links = {"semantic_store": symlink}
    commands = []
    comparisons = {}
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT / "tools")
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    cases = [
        ("threads", THREAD_MANIFEST, Path(freeze_record["thread_freeze"]["path"]),
         THREAD_SEEDS, output / "threads"),
        ("continuations", CONTINUATION_MANIFEST, Path(freeze_record["continuation_freeze"]["path"]),
         CONTINUATION_SEEDS, output / "continuations"),
    ]
    for family, manifest, frozen, seeds, report_dir in cases:
        command, settings = _run_one(family, family, manifest, frozen, seeds, report_dir,
                                     private_root=private_root, baseline=baseline,
                                     candidate=candidate, semantic_receipt=semantic_receipt)
        taskset = ["taskset", "-c", THREAD_CPUS if family == "threads" else str(CPU)]
        full_command = taskset + command
        command_record = {"argv": full_command, "cwd": str(private_root),
                          "environment": {"PYTHONPATH": env["PYTHONPATH"],
                                          "PYTHONDONTWRITEBYTECODE": "1"},
                          "settings": settings, "baseline": normal(baseline / "manifest.json"),
                          "candidate": normal(candidate), "manifest": normal(manifest),
                          "frozen": normal(frozen), "semantic_receipt": normal(semantic_receipt)}
        command_dir = output / "commands"
        command_dir.mkdir(exist_ok=True)
        command_path = command_dir / (family + ".command.json")
        write_new(command_path, command_record)
        proc = subprocess.run(full_command, cwd=private_root, env=env,
                              text=True, capture_output=True)
        stdout = command_dir / (family + ".stdout")
        stderr = command_dir / (family + ".stderr")
        stdout.write_text(proc.stdout, encoding="utf-8")
        stderr.write_text(proc.stderr, encoding="utf-8")
        result = {"name": family, "argv": full_command, "cwd": str(private_root),
                  "exit_code": proc.returncode, "stdout": normal(stdout), "stderr": normal(stderr),
                  "command": normal(command_path)}
        write_new(command_dir / (family + ".result.json"), result)
        commands.append(result)
        need(proc.returncode == 0, "focused " + family + " comparison failed; retained raw command/logs")
        report_path = report_dir / "report.json"
        comparisons[family] = _validate_comparison(
            report_path, family, seeds, candidate_sources, manifest, frozen,
            normal(candidate), semantic_ref)
    artifact_inventory = _retain_measurement_artifacts(
        output, preflight["previous_family_reports"]["source_inputs"])
    value = {"schema": "wtime-p080-focused-comparisons-v1", "task": "P080",
             "status": "passed", "source_stable": _private_focused_sources(private_root) == candidate_sources,
             "timing_evidence": True, "instrumented": False, "mode": "decoded",
             "freeze": normal(freeze_path), "prior_validation": normal(verify_dir / "prior.json"),
             "semantic_receipt": normal(semantic_receipt), "predecessor": normal(baseline / "manifest.json"),
             "candidate_binary": normal(candidate), "candidate_sources": candidate_sources,
             "semantic_store_link": links["semantic_store"], "commands": commands,
             "comparisons": comparisons,
             "previous_family_reports": preflight["previous_family_reports"],
             # Keep the inventory as a normal reference. The analyzer verifies
             # every content-addressed payload listed by this manifest.
             "artifacts": artifact_inventory["inventory"],
             "artifact_count": artifact_inventory["entry_count"],
             "artifact_store": artifact_inventory["store"],
             "executed_helpers": {"workflow": normal(output / "executed/p080_workflow.py"),
                                  "focused_compare": normal(output / "executed/focused_compare.py")},
             "pairing": {"pairs_per_run": 30, "runs_per_family": 2,
                         "threads_seeds": list(THREAD_SEEDS),
                         "continuation_seeds": list(CONTINUATION_SEEDS),
                         "thread_worker_cpus": THREAD_CPUS, "measurement_cpu": CPU}}
    need(value["source_stable"] is True, "private candidate source changed during comparisons")
    write_new(output / "summary.json", value)
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="phase", required=True)
    for name in ("freeze", "verify", "measure"):
        phase = sub.add_parser(name)
        phase.add_argument("--output", type=Path, required=True)
        phase.add_argument("--private-root", type=Path, default=PRIVATE_ROOT)
    v = sub.choices["verify"]
    v.add_argument("--freeze", type=Path, required=True)
    m = sub.choices["measure"]
    m.add_argument("--freeze", type=Path, required=True)
    m.add_argument("--verify-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.phase == "freeze":
            result = freeze(args.output, args.private_root)
        elif args.phase == "verify":
            result = verify(args.freeze, args.output, args.private_root)
        else:
            result = measure(args.freeze, args.verify_dir, args.output, args.private_root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        print(f"P080 {args.phase} failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"status": result.get("status"), "schema": result.get("schema")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
