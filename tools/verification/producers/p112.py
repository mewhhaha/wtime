"""Read-only release evidence aggregator for the selected portable portfolio."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from verification.artifacts import ArtifactStore, VerificationError, need, normal, read_json, verify_normal, write_new
import p108

TASKS = ("P108", "P109", "P110", "P111")
LIMITATIONS = [
    "Native AArch64 release execution for T166 and T169 remains unrun; this report does not verify the original release on that target.",
    "Only the decoded tier is the default. Compact and Register remain experimental opt-ins and carry no default speed claim.",
    "Historical release proof remains history; this report is a separate current-source appendix.",
    "The specialist thread portfolio used a two-CPU mask. Four requested workers were quota-clamped to two effective workers; no four-worker scaling claim is made.",
]


def source_identity(root: Path) -> dict[str, str]:
    paths = [root / "Cargo.toml", root / "Cargo.lock", root / "rust-toolchain.toml",
             *root.glob("crates/*/Cargo.toml"), *root.glob("crates/**/*.rs")]
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(set(paths)) if path.is_file()}


def _ref(path: Path) -> dict:
    path = path.resolve(strict=True)
    need(path.is_file(), "release evidence reference is not a file")
    return {"path": str(path), "sha256": _sha(path), "bytes": path.stat().st_size}


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _check_ref(value: object, label: str) -> Path:
    need(isinstance(value, dict), f"{label} reference is missing")
    return verify_normal(value)


def _validate_completion(root: Path, task: str, sources: dict[str, str]) -> dict:
    folder = root / "docs/benchmarks/performance"
    completion_path = folder / f"{task}-completion.json"
    evidence_path = folder / f"{task}-evidence.json"
    proof_path = folder / f"{task}-proof.json"
    completion = read_json(completion_path)
    evidence = read_json(evidence_path)
    proof = read_json(proof_path)

    need(completion.get("task") == task and completion.get("status") == "done"
         and completion.get("outcome") in ({"adopted", "inconclusive"} if task == "P110" else {"contract_only"}),
         f"{task} completion task/status/outcome differs")
    refs = completion.get("evidence")
    need(isinstance(refs, list) and len(refs) == 1, f"{task} completion evidence is incomplete")
    retained = _check_ref(refs[0], f"{task} retained evidence")
    need(retained.is_relative_to(root) and retained.read_bytes() == evidence_path.read_bytes()
         and completion.get("artifact_hashes") == {
        str(retained): refs[0]["sha256"]}, f"{task} completion does not bind its exported evidence")
    expected_timing = task == "P110"
    outcome = completion["outcome"]
    need(evidence.get("schema") == "wtime-verification-evidence-v1"
         and evidence.get("task") == task and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("synthetic") is False
         and evidence.get("outcome") == outcome
         and (task == "P110" or evidence.get("candidate_promoted") is not True),
         f"{task} verification evidence is synthetic, incomplete, or promoting")

    # Validate immutable final phase payload and require it to be exactly the retained proof.
    phases = evidence.get("phases")
    need(isinstance(phases, dict) and "analyze" in phases, f"{task} final proof phase is missing")
    store_root = Path(evidence.get("store", ""))
    need(store_root.is_absolute() and store_root.is_dir(), f"{task} artifact store is missing")
    store = ArtifactStore(store_root)
    receipt = store.json(phases["analyze"])
    need(receipt.get("task") == task and receipt.get("status") == "passed"
         and receipt.get("lane") == "full" and receipt.get("synthetic") is False,
         f"{task} analyze phase is not a full nonsynthetic pass")
    original = receipt.get("original_outputs", {}).get("proof")
    need(isinstance(original, dict) and original.get("sha256") == _sha(proof_path)
         and original.get("bytes") == proof_path.stat().st_size,
         f"{task} retained proof differs from immutable analyze output")

    need(proof.get("task") == task and proof.get("status") == "passed"
         and proof.get("timing_evidence") is expected_timing,
         f"{task} proof status or timing evidence label differs")
    if task == "P110":
        need(proof.get("outcome") == outcome, "P110 proof outcome differs from completion")
    proof_sources = proof.get("core_sources", proof.get("sources"))
    need(proof_sources == sources, f"{task} proof is stale against current core sources")
    proof_ref = _ref(proof_path)
    return {"task": task, "completion": _ref(completion_path), "evidence": _ref(evidence_path),
            "proof": proof_ref, "proof_schema": proof.get("schema", "unspecified"),
            "outcome": outcome, "candidate_promoted": evidence.get("candidate_promoted", False)}


def _validate_portfolio(root: Path, proof_ref: dict, sources: dict[str, str]) -> dict:
    proof = read_json(verify_normal(proof_ref))
    need(proof.get("schema") == "wtime-p108-portfolio-v1" and proof.get("task") == "P108",
         "P108 portfolio proof schema or task differs")
    need(proof.get("sources") == sources, "P108 portfolio source identity is stale")
    policy_ref = proof.get("policy")
    need(isinstance(policy_ref, dict), "P108 policy artifact reference is missing")
    policy = read_json(verify_normal(policy_ref))
    tiers = p108.validate_policy(policy, sources=sources)
    selected = proof.get("selected_tiers")
    need(isinstance(selected, dict) and set(selected) == {row["name"] for row in tiers},
         "P108 selected tier proofs are incomplete")
    for reference in selected.values():
        verify_normal(reference)
    verify_normal(proof["verification_receipt"])
    return {"default": policy["default"], "native_enabled": policy["native_enabled"],
            "aot_enabled": policy["aot_enabled"], "tiers": tiers, "policy": policy_ref}


def _validate_final_gates(root: Path, refs: dict[str, dict], sources: dict[str, str]) -> dict:
    # P109's selected conformance proof is the conformance gate.
    p109 = read_json(verify_normal(refs["P109"]["proof"]))
    need(p109.get("schema") == "wtime-p109-selected-conformance-v1"
         and p109.get("status") == "passed" and p109.get("core_sources") == sources,
         "P109 selected conformance gate is not current and passed")
    verify_normal(p109["verification_receipt"])

    p110 = read_json(verify_normal(refs["P110"]["proof"]))
    need(p110.get("schema") == "wtime-p110-final-budgets-v1"
         and p110.get("hard_gate_passed") is True and p110.get("core_sources") == sources,
         "P110 final hard budget gate is not current and passed")
    verify_normal(p110["policy"])
    verify_normal(p110["footprints"])
    cold_collection_ref = p110.get("cold_collection")
    need(isinstance(cold_collection_ref, dict), "P110 cold collection reference is missing")
    cold_collection_path = verify_normal(cold_collection_ref)
    collection = read_json(cold_collection_path)
    need(collection.get("schema") == "wtime-p110-cold-collection-v1"
         and collection.get("task") == "P110" and collection.get("status") == "passed"
         and collection.get("source_stable") is True and collection.get("timing_evidence") is True,
         "P110 cold collection did not pass with stable timing inputs")
    need(set(collection.get("reports", {})) == {"default-vs-p001", "compact-vs-decoded"},
         "P110 retained cold report set differs")
    for reference in collection["reports"].values():
        report = read_json(verify_normal(reference))
        need(report.get("passed") is True and report.get("source_stable") is True
             and report.get("selection") == "original-mvp",
             "P110 retained cold comparison is incomplete or stale")
    artifacts_manifest = read_json(verify_normal(collection["artifacts"]))
    store = ArtifactStore(root / "target/verification")
    need(isinstance(artifacts_manifest, dict) and bool(artifacts_manifest),
         "P110 cold collection artifact manifest is empty")
    for reference in artifacts_manifest.values():
        store.verify(reference)
    need(isinstance(p110.get("builds"), dict) and isinstance(p110.get("cold_reviews"), dict)
         and bool(p110["builds"]) and bool(p110["cold_reviews"]),
         "P110 final build/cold budget review is incomplete")
    statistics = p110.get("statistics")
    expected_statistics = {"default-vs-p001", "compact-vs-decoded", "eager-vs-decoded", "adaptive-vs-decoded"}
    need(isinstance(statistics, dict) and set(statistics) == expected_statistics,
         "P110 final measured statistics set is incomplete")
    for reference in statistics.values():
        stats = read_json(verify_normal(reference))
        summary = stats.get("analysis") if "decision" in stats else stats
        need(isinstance(summary, dict) and isinstance(summary.get("analysis"), dict)
             and isinstance(summary.get("per_run"), list),
             "P110 measured statistics artifact is malformed")
    builds = p110["builds"]
    need(builds.get("schema") == "wtime-p110-build-budgets-v1" and builds.get("status") == "passed"
         and builds.get("source_inputs_current") is True
         and builds.get("dependency_budget", {}).get("status") == "passed",
         "P110 retained build budget proof is incomplete")
    need(set(builds.get("retained_reports", {})) == {"default", "register", "compact"}
         and set(builds.get("retained_commands", {})) == {"default", "register", "compact"},
         "P110 retained build reports or command records are incomplete")
    for name, reference in builds["retained_reports"].items():
        build = read_json(verify_normal(reference))
        need(build.get("mode") == ("default" if name == "default" else "optional-feature")
             and build.get("source_stable") is True and build.get("budget_pass") is True,
             f"P110 retained {name} build report is stale or failed")
    for name, reference in builds["retained_commands"].items():
        command = read_json(verify_normal(reference))
        need(command.get("exit_code") == 0, f"P110 retained {name} build command failed")
    verify_normal(builds["retained_dependency_report"])
    p111 = read_json(verify_normal(refs["P111"]["proof"]))
    need(p111.get("schema") == "wtime-p111-ci-v1" and p111.get("status") == "passed"
         and p111.get("timing_evidence") is False,
         "P111 CI lane proof is invalid")
    verify_normal(p111["verification_receipt"])
    need(p111.get("smoke") and p111.get("reference"), "P111 smoke/reference lane references are missing")
    verify_normal(p111["smoke"])
    reference = read_json(verify_normal(p111["reference"]))
    need(reference.get("status") == "unrun" and reference.get("decision") == "unrun",
         "P111 reference lane must remain explicitly unrun")
    return {"final_budget": refs["P110"]["proof"], "conformance": refs["P109"]["proof"],
            "p109_conformance": refs["P109"]["proof"],
            "measured_statistics": p110["statistics"], "build_budgets": p110["builds"],
            "cold_reviews": p110["cold_reviews"], "cold_collection": cold_collection_ref,
            "footprints": p110["footprints"]}


def _markdown(report: dict) -> str:
    lines = ["# Performance release appendix", "",
             "This appendix records a current-source proof of the selected portable performance portfolio.",
             "It preserves the earlier release proof as historical evidence and makes no new timing claims.", "",
             "## Current policy", "", "- Default execution: `decoded`.",
             "- Native and AOT: disabled.",
             "- Compact and Register: experimental opt-ins only.", "",
             "## Measured evidence", "",
             "The exact final build budgets, cold reviews, and retained statistics are referenced in the P110 proof:", "",
             f"- P110 final proof: `{report['final_gates']['final_budget']['path']}` "
             f"(`{report['final_gates']['final_budget']['sha256']}`).", "",
             ""]
    builds = report['final_gates']['build_budgets']
    lines += ['## Build and deployment cost', '',
              'These are retained source-current observations on the shared cloud host. Optional-tier ratios are unpaired cost reviews; no longitudinal P001 build comparison is available.', '',
              '| Configuration | Clean (s) | CLI leaf (s) | Runtime leaf (s) | Stripped CLI (bytes) | Peak build RSS (KiB) |',
              '| --- | ---: | ---: | ---: | ---: | ---: |']
    metrics = ('clean_seconds', 'leaf_seconds', 'runtime_leaf_seconds', 'stripped_bytes', 'clean_peak_rss_kib')
    for name, reference in builds['retained_reports'].items():
        build = read_json(verify_normal(reference))
        if all(key in build for key in metrics):
            lines.append(f"| {name} | {build['clean_seconds']:.3f} | {build['leaf_seconds']:.3f} | {build['runtime_leaf_seconds']:.3f} | {build['stripped_bytes']} | {build['clean_peak_rss_kib']} |")
    footprints = read_json(verify_normal(report['final_gates']['footprints']))
    lines += ['', 'Compile-only module counters below are instrumented diagnostics with zero guest invocations. The columns overlap; they are not additive or a complete heap census. An unavailable adaptive cache is reported explicitly.', '',
              '| Fixture | Mode | Wasm bytes | Lowered control bytes | Function-plan bytes | Compact retained bytes | Adaptive cache state |',
              '| --- | --- | ---: | ---: | ---: | ---: | --- |']
    for row in footprints.get('records', []):
        value = row['observation']
        compact = value.get('compact_preparation_counts')
        retained = compact['retained_bytes'] if compact is not None else 'not requested'
        lines.append(f"| {Path(row['fixture']['path']).stem} | {row['mode']} | {value['wasm_bytes']} | {value['lowered_control_bytes']} | {value['function_plan_bytes']} | {retained} | {value['register_cache_state']} |")
    lines += ['', '## Fixed warm examples', '',
              'Candidate/baseline ratios below 1 favor the candidate. Each row preserves its own run and 95% interval. These examples do not summarize other workloads or establish a universal tier ranking. All original products remain in the linked statistics.', '',
              '| Comparison | Workload | Run | Baseline (us) | Candidate (us) | Ratio | 95% interval |',
              '| --- | --- | ---: | ---: | ---: | ---: | --- |']
    summaries = {}
    for name, reference in report['final_gates']['measured_statistics'].items():
        value = read_json(verify_normal(reference))
        summary = value['analysis'] if 'decision' in value else value
        summaries[name] = summary
        for row in summary['per_run']:
            if row['stage'] != 'warm_call' or row['workload'] not in ('call_overhead', 'integer_loop', 'branch_calls'):
                continue
            interval = row['median_ratio_95ci']
            lines.append(f"| {name} | {row['workload']} | {row['run']} | {row['baseline_ns_per_operation']['median']/1000:.3f} | {row['candidate_ns_per_operation']['median']/1000:.3f} | {row['candidate_over_baseline']['median']:.3f} | {interval['lower']:.3f}–{interval['upper']:.3f} |")
    lines += ['', '## Cold controls and amortization', '',
              '| Comparison | Workload / stage | Run | Ratio | 95% interval | Budget review |',
              '| --- | --- | ---: | ---: | --- | --- |']
    for name, review in report['final_gates']['cold_reviews'].items():
        for row in review.get('rows', []):
            if not (row['workload'] == 'prepare_large' or
                    row['workload'] == 'call_overhead' and row['stage'] in ('first_call', 'load_and_call')):
                continue
            interval = row['median_ratio_95ci']
            lines.append(f"| {name} | {row['workload']} / {row['stage']} | {row['run']} | {row['candidate_over_baseline']:.3f} | {interval['lower']:.3f}–{interval['upper']:.3f} | {row['review']} |")
    lines += ['', 'All remaining cold rows, including arithmetic and mixed branch/call modules, stay in the P110 cold review. A budget overrun or uncertain interval remains visible and does not promote an optional tier.', '',
              'Break-even counts below are descriptive estimates already computed by the paired analyzer from preparation, instantiation and warm medians. They are not measured deployment totals or confidence bounds.', '']
    for name, summary in summaries.items():
        for row in summary.get('break_even', []):
            if row['workload'] == 'call_overhead':
                result = str(row['break_even_calls']) + ' calls' if row['break_even_calls'] is not None else row['break_even']
                lines.append(f"- {name}, call_overhead, run {row['run']}: {result}.")
    lines += ['', '## Wasmtime comparison scope', '',
              'The historical Wasmtime 49.0.2 comparisons in `docs/benchmarks/wasmtime-comparison*.json` retain their original source identities and settings. This current portfolio did not collect a new matched Wasmtime comparison. Its wtime/predecessor ratios cannot refresh those historical ratios. GC, thread and continuation comparator limitations describe the pinned harness interface; no unsupported or unrun comparator receives a ratio.', '',
              '## Verified milestones', '']
    for task, record in report["milestones"].items():
        lines.append(f"- {task} ({record['outcome']}): completion `{record['completion']['sha256']}`, "
                     f"proof `{record['proof']['sha256']}`.")
    if report["milestones"]["P110"]["candidate_promoted"] is True:
        lines += ["", "P110 records its budget-gate approval; P108 still keeps Compact and Register experimental."]
    lines += ["", "## Current core sources", "", "The accompanying `report.json` retains the complete current core source map and exact prerequisite proof hashes."]
    lines += ["", "## Limitations", ""]
    lines.extend(f"- {item}" for item in report["limitations"])
    lines.append("")
    return "\n".join(lines)


def finalize(root: Path, output_dir: Path) -> dict:
    root = root.resolve(strict=True)
    sources = source_identity(root)
    need(bool(sources), "current core source identity is empty")
    milestones = {task: _validate_completion(root, task, sources) for task in TASKS}
    portfolio = _validate_portfolio(root, milestones["P108"]["proof"], sources)
    gates = _validate_final_gates(root, milestones, sources)
    report = {"schema": "wtime-p112-release-report-v1", "task": "P112", "status": "passed",
              "timing_evidence": False, "current_core_sources": sources, "milestones": milestones,
              "policy": portfolio, "final_gates": gates, "limitations": LIMITATIONS}
    output_dir = output_dir.resolve()
    need(output_dir.is_relative_to(root), "release output must stay inside the supplied root")
    output_dir.mkdir(parents=True, exist_ok=False)
    write_new(output_dir / "report.json", report)
    with (output_dir / "release-appendix.md").open("x", encoding="utf-8") as stream:
        stream.write(_markdown(report))
    write_new(output_dir / "release-schema.json", {
        "$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "wtime-p112-release-report-v1",
        "type": "object", "required": ["schema", "task", "status", "timing_evidence", "current_core_sources", "milestones", "policy", "final_gates", "limitations"],
        "properties": {"schema": {"const": "wtime-p112-release-report-v1"}, "task": {"const": "P112"},
                       "status": {"const": "passed"}, "timing_evidence": {"const": False}},
        "additionalProperties": True})
    historical = root / 'docs/architecture/release-manifest.json'
    write_new(output_dir / 'performance-release-manifest.json', {
        'schema': 'wtime-performance-release-manifest-v1', 'task': 'P112',
        'status': 'passed', 'timing_evidence': False,
        'current_core_sources': sources, 'portfolio': portfolio,
        'prerequisite_proofs': {task: record['proof'] for task, record in milestones.items()},
        'performance_report': _ref(output_dir / 'report.json'),
        'historical_release_manifest': _ref(historical) if historical.is_file() else None,
        'original_release_reverified': False,
        'native_aarch64': {'status': 'unrun', 'original_tasks': ['T166', 'T169']},
        'default_external_dependencies': 0, 'default_unsafe': False,
        'native_enabled': False, 'aot_enabled': False,
        'limitations': LIMITATIONS})
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        finalize(args.root, args.output_dir)
    except (OSError, KeyError, TypeError, VerificationError, ValueError) as error:
        print(f"P112 release proof failed: {error}", file=sys.stderr)
        return 1
    print("P112 release proof passed: four current-source milestones; timing claims are not generated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
