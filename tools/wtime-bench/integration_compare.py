"""P113 paired production comparison for compact-prefix integration cases."""
from __future__ import annotations

import hashlib
import math
import os
from pathlib import Path
import platform
import random

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TARGET_NS = 20_000_000.0
MAX_ATTEMPTS = 8
MAX_BATCH = 100_000
RUN_IDS = (0, 1)


class IntegrationComparisonError(ValueError):
    pass


def need(ok: bool, message: str) -> None:
    if not ok:
        raise IntegrationComparisonError(message)


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _balanced_orders(compare, seed: int, count: int) -> list[list[str]]:
    """Retain randomized order while balancing first-side counts exactly."""
    raw = compare.pair_order(seed, count)
    rng = random.Random(seed ^ 0x50313133)
    wanted = count // 2 + (1 if count % 2 and rng.getrandbits(1) else 0)
    baseline_first = [index for index, order in enumerate(raw) if order[0] == "baseline"]
    candidate_first = [index for index, order in enumerate(raw) if order[0] == "candidate"]
    if len(baseline_first) > wanted:
        flip = rng.sample(baseline_first, len(baseline_first) - wanted)
        for index in flip:
            raw[index] = ["candidate", "baseline"]
    elif len(baseline_first) < wanted:
        flip = rng.sample(candidate_first, wanted - len(baseline_first))
        for index in flip:
            raw[index] = ["baseline", "candidate"]
    return raw


def _settings(mode: str) -> dict:
    return {"features": ["all-phase3-core"], "fuel": False,
            "cancellation_quantum": 1, "mode": mode}


def _validate_capabilities(capabilities: dict, profile: str) -> None:
    for side, protocol in capabilities.items():
        need(protocol.get("schema_version") == 2 and protocol.get("engine") == "wtime"
             and protocol.get("instrumented") is False
             and "compact" in protocol.get("modes", [])
             and profile in protocol.get("features", []),
             f"{side} protocol does not prove uninstrumented Compact/{profile}")


def _preflight_all(selected: list[dict], fixtures: dict, executables: dict,
                   cpu: int, compare, model) -> list[dict]:
    probes = []
    for row in selected:
        case = row["id"]
        for side in ("baseline", "candidate"):
            args = ["--integration-driver", fixtures[case]["path"], case,
                    "first_call", str(model.INPUT), "1", "compact", "--probe"]
            wire = compare.process(executables[side], args, cpu)
            proof = _compact_wire(wire, model, case, "first_call", model.INPUT,
                                  preflight=True)
            probes.append({"case": case, "side": side, "status": "passed",
                           "timing_excluded": True,
                           "memory_sha256": proof["memory_sha256"],
                           "initial_memory_sha256": proof.get("initial_memory_sha256"),
                           "result": proof["result"], "outcome": proof["outcome"],
                           "payload": proof["payload"], "lifecycle": proof["lifecycle"]})
    return probes


def _preflight_before_sampling(preflight, sampling):
    """Keep correctness preflight as a hard gate before the first timed call."""
    probes = preflight()
    observations = sampling()
    return probes, observations


def _compact_wire(wire: dict, model, case: str, stage: str, n: int,
                  expected_batch: int | None = None, *, preflight: bool = False) -> dict:
    need(isinstance(wire, dict), f"{case}/{stage}: driver response is not an object")
    if preflight:
        need(wire.get("schema_version") == 2 and wire.get("case") == case
             and wire.get("stage") == stage and wire.get("input") == f"i32:{n}",
             f"{case}: preflight response identity mismatch")
        need(wire.get("mode") == "compact" and wire.get("profile") == "all-phase3-core"
             and wire.get("instrumented") is False and wire.get("diagnostic") is False,
             f"{case}: preflight execution settings/instrumentation mismatch")
        proof = model.validate_preflight(wire, case, n)
        need(wire.get("report_complete") is True and wire.get("timing_excluded") is True
             and wire.get("batch") == 1 and wire.get("calls_checked") == 1,
             f"{case}: preflight must complete one untimed checked invocation")
    else:
        proof = model.validate_observation(wire, case, stage, n,
                                           expected_batch=expected_batch)
        need(wire.get("report_complete") is True,
             f"{case}/{stage}: driver did not complete the requested report")
    batch = wire.get("batch")
    calls = wire.get("calls_checked")
    if not preflight:
        need(type(calls) is int and calls == batch,
             f"{case}/{stage}: calls_checked must equal the actual batch")
    memory = bytes.fromhex(wire["memory_hex"])
    compact = {key: value for key, value in wire.items()
               if key not in {"memory_hex", "initial_memory_hex"}}
    compact["memory_proof"] = {"memory_count": wire["memory_count"],
                               "memory_pages": wire["memory_pages"],
                               "memory_bytes": len(memory),
                               "memory_sha256": hashlib.sha256(memory).hexdigest()}
    if "initial_memory_hex" in wire:
        initial = bytes.fromhex(wire["initial_memory_hex"])
        compact["initial_memory_proof"] = {
            "memory_bytes": len(initial),
            "memory_sha256": hashlib.sha256(initial).hexdigest(),
        }
    if preflight:
        compact["settings"] = _settings("compact")
        compact["instrumented"] = False
        compact["diagnostic"] = False
        compact["profile"] = "all-phase3-core"
        compact["memory_sha256"] = proof["memory_sha256"]
        if "initial_memory_sha256" in proof:
            compact["initial_memory_sha256"] = proof["initial_memory_sha256"]
        return compact

    measured = wire.get("measured_guest_ns")
    ns_per = wire.get("ns_per_operation")
    need(type(measured) in (int, float) and math.isfinite(measured) and measured > 0,
         f"{case}/{stage}: measured_guest_ns must be finite and positive")
    need(type(ns_per) in (int, float) and math.isfinite(ns_per) and ns_per > 0,
         f"{case}/{stage}: ns_per_operation must be finite and positive")
    need(math.isclose(float(measured), float(ns_per) * batch,
                      rel_tol=1e-9, abs_tol=0.001),
         f"{case}/{stage}: summed guest interval does not equal per-call time times batch")
    compact["settings"] = _settings("compact")
    compact["actual_guest_calls"] = calls
    compact["measured_guest_ns"] = float(measured)
    compact["ns_per_operation"] = float(ns_per)
    compact["batch"] = batch
    compact["instrumented"] = False
    compact["diagnostic"] = False
    compact["profile"] = "all-phase3-core"
    return compact


def _calibrate(case: str, n: int, fixture: Path, observe, model) -> dict:
    first = {}
    for side in ("baseline", "candidate"):
        first[side] = observe(side, fixture, case, "warm_call", n, 0)
        actual = first[side].get("batch")
        _compact_wire(first[side], model, case, "warm_call", n, actual)
        need(first[side]["measured_guest_ns"] >= TARGET_NS,
             f"{case}: native batch-0 calibration did not reach 20 ms on {side}")
    batch = max(first[side]["batch"] for side in first)
    attempts = []
    for attempt in range(1, MAX_ATTEMPTS + 1):
        confirmations = {}
        for side in ("baseline", "candidate"):
            wire = observe(side, fixture, case, "warm_call", n, batch)
            confirmations[side] = _compact_wire(wire, model, case, "warm_call", n, batch)
        attempts.append({"attempt": attempt, "batch": batch,
                         "sides": {side: {"measured_guest_ns": row["measured_guest_ns"],
                                          "ns_per_operation": row["ns_per_operation"]}
                                   for side, row in confirmations.items()}})
        if all(row["measured_guest_ns"] >= TARGET_NS for row in confirmations.values()):
            return {"passed": True, "policy": "shared_batch_both_sides_confirmed",
                    "target_ns": TARGET_NS, "max_attempts": MAX_ATTEMPTS,
                    "max_batch": MAX_BATCH, "paired_batch": batch,
                    "batch0_calibration": {side: {"actual_batch": row["batch"],
                                                  "measured_guest_ns": row["measured_guest_ns"]}
                                           for side, row in first.items()},
                    "attempts": attempts}
        if attempt == MAX_ATTEMPTS or batch == MAX_BATCH:
            break
        fastest = min(row["ns_per_operation"] for row in confirmations.values())
        projected = math.ceil(TARGET_NS / fastest * 1.1)
        batch = min(MAX_BATCH, max(batch + 1, projected))
    return {"passed": False, "policy": "shared_batch_both_sides_confirmed",
            "target_ns": TARGET_NS, "max_attempts": MAX_ATTEMPTS,
            "max_batch": MAX_BATCH, "paired_batch": batch,
            "batch0_calibration": {side: {"actual_batch": row["batch"],
                                          "measured_guest_ns": row["measured_guest_ns"]}
                                   for side, row in first.items()},
            "attempts": attempts}


def _fixture_binding(entry: dict, compare, frozen: Path, manifest: dict,
                     source_map: dict[str, str]) -> dict:
    source = entry["source"]
    wat = HERE / source["path"]
    wasm = wat.with_suffix(".wasm")
    need(wat.is_file() and wasm.is_file(), f"{entry['id']}: encoded WAT/Wasm fixture is missing")
    wat_hash, wasm_hash = sha(wat), sha(wasm)
    need(wat_hash == source.get("sha256") and wasm_hash == source.get("wasm_sha256"),
         f"{entry['id']}: live fixture hashes do not match registry")
    for path, digest in ((wat, wat_hash), (wasm, wasm_hash)):
        logical = path.relative_to(ROOT).as_posix()
        need(source_map.get(logical) == digest, f"{entry['id']}: candidate source map omits fixture {logical}")
        key = "source/" + logical
        record = manifest.get("files", {}).get(key)
        archived = frozen / "source" / logical
        need(isinstance(record, dict) and record.get("sha256") == digest
             and archived.is_file() and sha(archived) == digest,
             f"{entry['id']}: immediate predecessor does not bind fixture {logical}")
    return {"path": wasm, "wat_sha256": wat_hash, "wasm_sha256": wasm_hash}


def run(args, growth_explanation=None):
    import baseline
    import checkpoint
    import compare
    import integration_workloads as model
    import workloads

    need(args.selection == "integration", "wrong integration comparison route")
    need(args.candidate_mode == "compact" and type(args.pairs) is int and args.pairs >= 30,
         "integration requires compact mode and at least 30 pairs per group")
    seed_base = getattr(args, "seed", 20261012)
    checkpoint.Journal.preflight(args.output)
    manifest = baseline.verify_archive(args.baseline)
    info = compare._baseline_identity(args.baseline, manifest, HERE / "src/protocol.rs", baseline)
    need(info["kind"] == "current-source-archive" and info["mode"] == "compact",
         "integration requires a fresh current-source Compact predecessor archive")
    frozen = Path(info["frozen"])
    compare._verify_scalar_harness_sources(frozen, manifest)

    selected = workloads.selection("integration")
    need([row["id"] for row in selected] == list(model.SELECTED),
         "integration registry selection differs from the six-row P113 model")
    for row in selected:
        need(row.get("stages") == model.STAGES and row.get("params", [{}])[0].get("baseline") == model.INPUT,
             f"{row.get('id')}: input or stage contract differs from integration model")
        need(row.get("result", {}).get("type") == "i32",
             f"{row['id']}: entrypoint signature must remain i32")
    candidate = Path(args.candidate).resolve(strict=True)
    baseline_exe = Path(info["executable"]).resolve(strict=True)
    before_binary = sha(candidate)
    before_baseline = sha(baseline_exe)
    before_sources = compare.sources(include_integration=True)
    available = sorted(os.sched_getaffinity(0))
    cpu = args.cpu if args.cpu is not None else available[0]
    need(cpu in available, "CPU outside current process affinity")

    files = manifest["files"]
    for logical, digest in before_sources.items():
        if not logical.startswith("tools/wtime-bench/"):
            continue
        record = files.get("source/" + logical)
        archived = frozen / "source" / logical
        need(isinstance(record, dict) and record.get("sha256") == digest
             and archived.is_file() and sha(archived) == digest,
             f"immediate predecessor does not bind identical source {logical}")
    fixtures = {row["id"]: _fixture_binding(row, compare, frozen, manifest, before_sources)
                for row in selected}

    capabilities = {side: compare.process(exe, ["--protocol"], cpu)
                    for side, exe in (("baseline", baseline_exe), ("candidate", candidate))}
    _validate_capabilities(capabilities, model.PROFILE)

    seeds = {run_id: seed_base + run_id * 1_000_003 for run_id in RUN_IDS}
    settings = {"features": [model.PROFILE], "feature_profiles": [model.PROFILE],
                "fuel": False, "cancellation_quantum": 1, "profile": "release",
                "instrumented": False, "baseline_mode": "compact", "candidate_mode": "compact",
                "integration_stages": model.STAGES,
                "integration_measurement_policy": "sum actual guest intervals; setup and full-state probes excluded",
                "calibration": {"target_ns": TARGET_NS, "max_attempts": MAX_ATTEMPTS,
                                "max_batch": MAX_BATCH, "policy": "shared_batch_both_sides_confirmed"}}
    metadata = {"settings": settings, "capabilities": capabilities,
                "hardware": {"selected_cpu": cpu, "available_cpus": available,
                             "platform": platform.platform(), "machine": platform.machine()},
                "baseline_archive_kind": info["kind"],
                "baseline_manifest_sha256": sha(Path(args.baseline) / "manifest.json"),
                "frozen_manifest_sha256": sha(frozen / "manifest.json"),
                "baseline_binary_sha256": before_baseline,
                "candidate_binary_sha256": before_binary,
                "candidate_sources": before_sources,
                "fixtures": [{"id": row["id"], "role": row.get("role"),
                              "wat_sha256": fixtures[row["id"]]["wat_sha256"],
                              "wasm_sha256": fixtures[row["id"]]["wasm_sha256"],
                              "input": model.INPUT, "stages": model.STAGES}
                             for row in selected]}
    if growth_explanation is not None:
        metadata.update(growth_explanation["metadata"])
    identity = {"selection": "integration", "metadata": metadata,
                "pairs": args.pairs, "seed": seed_base}
    journal = checkpoint.Journal(args.output, identity, seeds)
    report = {"schema_version": 2, "selection": "integration", "passed": False,
              "source_stable": False, "metadata": metadata, "runs": [],
              "warm_calibrations": [], "correctness_probes": []}

    def observe(side: str, case: str, stage: str, batch: int) -> dict:
        executable = baseline_exe if side == "baseline" else candidate
        args_list = ["--integration-driver", fixtures[case]["path"], case, stage,
                     str(model.INPUT), str(batch), "compact"]
        return compare.process(executable, args_list, cpu)

    try:
        def collect_pairs():
            for run_id in RUN_IDS:
                run_record = {"run_id": run_id, "seed": seeds[run_id], "pairs": []}
                report["runs"].append(run_record)
                for row in selected:
                    case = row["id"]
                    for stage in model.STAGES:
                        batch = 1
                        if stage == "warm_call":
                            calibration = _calibrate(
                                case, model.INPUT, fixtures[case]["path"],
                                lambda side, _fixture, _case, _stage, _n, requested:
                                    observe(side, _case, _stage, requested), model)
                            calibration["run_id"] = run_id
                            calibration["workload"] = case
                            calibration["stage"] = stage
                            report["warm_calibrations"].append(calibration)
                            need(calibration["passed"], f"{case}: shared warm calibration failed")
                            batch = calibration["paired_batch"]
                        orders = _balanced_orders(compare, seeds[run_id]
                                                  + int(fixtures[case]["wasm_sha256"][:8], 16)
                                                  + model.STAGES.index(stage), args.pairs)
                        for sample, order in enumerate(orders):
                            pair = {"pair_id": f"{run_id}:{case}:{stage}:{sample}",
                                    "workload": case, "stage": stage,
                                    "input": f"i32:{model.INPUT}",
                                    "expected": model.expected(case, model.INPUT)["expected"],
                                    "fixture": fixtures[case]["wasm_sha256"],
                                    "wasm_sha256": fixtures[case]["wasm_sha256"],
                                    "wat_sha256": fixtures[case]["wat_sha256"],
                                    "batch": batch, "order": order,
                                    "role": row.get("role"), "settings": _settings("compact")}
                            for side in order:
                                wire = observe(side, case, stage, batch)
                                pair[side] = _compact_wire(wire, model, case, stage,
                                                           model.INPUT, batch)
                            journal.append(run_id, seeds[run_id], pair)
                            run_record["pairs"].append(pair)

        report["correctness_probes"], _ = _preflight_before_sampling(
            lambda: _preflight_all(
                selected, fixtures, {"baseline": baseline_exe, "candidate": candidate},
                cpu, compare, model), collect_pairs)
        need(compare.sources(include_integration=True) == before_sources
             and sha(candidate) == before_binary and sha(baseline_exe) == before_baseline,
             "source or executable identity changed during paired measurement")
        compare._verify_build_growth_explanation(growth_explanation)
        expected_pairs = len(selected) * len(model.STAGES) * args.pairs
        need(len(report["correctness_probes"]) == len(selected) * 2
             and len(report["warm_calibrations"]) == len(RUN_IDS) * len(selected),
             "integration preflight/calibration matrix is incomplete")
        need(all(len(run["pairs"]) == expected_pairs for run in report["runs"]),
             "integration paired matrix is incomplete")
        report["counts"] = {"workloads": len(selected), "stages_per_workload": len(model.STAGES),
                            "groups_per_run": len(selected) * len(model.STAGES),
                            "groups_total": len(RUN_IDS) * len(selected) * len(model.STAGES),
                            "pairs_per_group": args.pairs,
                            "pair_events": len(RUN_IDS) * expected_pairs,
                            "side_observations": 2 * len(RUN_IDS) * expected_pairs,
                            "correctness_probes": len(report["correctness_probes"]),
                            "calibrations": len(report["warm_calibrations"])}
        report["passed"] = True
        report["source_stable"] = True
    except Exception as error:
        report["error"] = type(error).__name__ + ": " + str(error)
    finally:
        journal.close()
        journal.write_report_once(report)
    if not report["passed"]:
        raise IntegrationComparisonError(report["error"])
    return report
