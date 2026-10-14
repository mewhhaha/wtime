#!/usr/bin/env python3
"""Fail-closed composite evidence checker for the P046 Compact portfolio gate.

The checker consumes immutable comparison reports. It does not launch engines,
compile candidates, or collect timing samples. Input paths are carried in the
normalized P046 input record; the verification workflow binds those files by
hash before invoking this producer.
"""
from __future__ import annotations

import argparse
import hashlib
import gzip
import json
import math
import random
import sys
import tempfile
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / 'tools'))

import analyze  # noqa: E402
import compare  # noqa: E402
import workloads  # noqa: E402
import execution_identity
from bootstrap import draw_median

SCHEMA = "p046-portfolio-v1"
FAMILIES = ("scalar", "memory", "simd", "representative")
BASELINES = ("decoded", "p001")
RUN_IDS = ((0, 1), (1, 2))
MIN_PAIRS = 30
P121_SELECTIONS = {"mvp", "core3", "phase3", "interactions"}
P121_EXPECTED_DIRECTIVES = 145_760
HEX = set("0123456789abcdef")


class PortfolioError(ValueError):
    """The evidence matrix or a hard gate violates the P046 contract."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise PortfolioError(message)


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False)


def _is_sha(value: Any) -> bool:
    return (isinstance(value, str) and len(value) == 64
            and all(char in HEX for char in value))


def _load_json(value: Any, label: str) -> tuple[dict[str, Any], str | None]:
    """Load an inline object or a {path, sha256} reference and return identity."""
    if isinstance(value, (str, Path)):
        path = Path(value).resolve(strict=True)
        value = {"path":str(path), "sha256":_sha(path)}
    if isinstance(value, dict) and isinstance(value.get("path"), str):
        path = Path(value["path"]).resolve(strict=True)
        need(path.is_file(), f"{label} path is not a file")
        digest = _sha(path)
        need(value.get("sha256") == digest, f"{label} hash is stale")
        if "bytes" in value:
            need(type(value["bytes"]) is int and value["bytes"] == path.stat().st_size,
                 f"{label} byte count changed")
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise PortfolioError(f"{label} JSON is invalid: {exc}") from exc
        need(isinstance(data, dict), f"{label} must contain a JSON object")
        return data, digest
    need(isinstance(value, dict), f"{label} must be an object or hashed JSON reference")
    return value, hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def _selection_rows(family: str) -> list[dict[str, Any]]:
    try:
        rows = workloads.selection(family)
    except (workloads.WorkloadError, workloads.EmptySelectionError) as exc:
        raise PortfolioError(f"cannot resolve full {family} workload selection: {exc}") from exc
    need(bool(rows), f"{family} workload selection is empty")
    return rows


def _expected_groups(family: str) -> set[tuple[str, str]]:
    return {(row["id"], stage) for row in _selection_rows(family)
            for stage in row["stages"]}


def _expected_source_map(family: str) -> dict[str, str]:
    values = compare.sources(include_memory=family == "memory",
                             include_simd=family == "simd",
                             include_representative=family == "representative")
    if family == 'representative':
        sys.path.insert(0,str(ROOT/'tools/verification/producers'))
        import portfolio_compare
        return portfolio_compare.measurement_sources(values)
    return values


def _representative_images(report, reference):
    """Recheck retained full images against the independent P015 oracle."""
    import representative_compare as timer
    import representative_scaling as scaling
    import representative_workloads as model
    need(isinstance(reference, dict) and isinstance(reference.get('path'), str),
         'representative images require a file-bound comparison report')
    root = Path(reference['path']).resolve(strict=True).parent
    entries = {row['id']: row for row in model.entries()}
    cache = {}
    observations = [(pair['workload'], side, pair[side])
                    for run in report['runs'] for pair in run['pairs']
                    for side in ('baseline', 'candidate')]
    observations += [(row['case'], row['side'], row['observation'])
                     for row in report.get('correctness_probes', [])]
    for calibration in report.get('warm_calibrations', []):
        for attempt in calibration['attempts']:
            observations += [(calibration['workload'], row['side'], {
                'raw_wire_observation':row['wire'], 'image_ref':row['wire']['image_ref'],
                'batch':row['wire']['batch']})
                             for row in attempt['sides']]
    for wid, side, observation in observations:
        image_ref = observation.get('image_ref')
        need(isinstance(image_ref, dict), 'representative full image was not retained')
        relative = Path(image_ref.get('path', ''))
        need(not relative.is_absolute() and '..' not in relative.parts
             and relative.parts[:1] == ('memory-images',),
             'representative image path escapes its comparison')
        try:
            path = (root / relative).resolve(strict=True)
        except OSError as error:
            raise PortfolioError('retained representative image is missing') from error
        need(path.is_relative_to(root), 'representative image resolves outside its comparison')
        identity = _canonical(image_ref)
        if identity not in cache:
            compressed = path.read_bytes()
            need(hashlib.sha256(compressed).hexdigest() == image_ref.get('compressed_sha256'),
                 'retained representative compressed image is stale')
            try:
                raw = gzip.decompress(compressed)
            except (OSError, EOFError) as error:
                raise PortfolioError('retained representative image is corrupt') from error
            need(hashlib.sha256(raw).hexdigest() == image_ref.get('raw_sha256')
                 and len(raw) == image_ref.get('raw_bytes'),
                 'retained representative full image is stale')
            cache[identity] = raw.hex()
        wire = dict(observation['raw_wire_observation'], memory_hex=cache[identity])
        mode = 'decoded' if side == 'baseline' else 'compact'
        try:
            validated = timer.validate_observation(entries[wid], wire, model, scaling,
                                                   observation['batch'], expected_mode=mode)
        except (timer.RepresentativeComparisonError, scaling.RepresentativeError) as error:
            raise PortfolioError('retained representative oracle failed: '+str(error)) from error
        if 'validated_memory_proof' in observation:
            need(validated['validated_memory_proof'] == observation['validated_memory_proof'],
                 'representative proof differs from the independent full image oracle')
    return len(cache)


def _verify_baseline_archive(report: dict[str, Any], baseline_name: str,
                             *, check_sources: bool) -> dict[str, Any] | None:
    metadata = report["metadata"]
    archive_path = metadata.get("baseline_archive_path")
    if baseline_name == "decoded":
        need(isinstance(archive_path, str) and archive_path,
             "comparison metadata must bind baseline_archive_path")
        if not check_sources:
            return None
        path = Path(archive_path).resolve(strict=True)
        import baseline as baseline_tool
        try:
            manifest = baseline_tool.verify_archive(path)
        except Exception as exc:
            raise PortfolioError(f"decoded baseline archive failed verification: {exc}") from exc
        if manifest.get('kind') == 'frozen-typed-adapter':
            import p123_adapter
            adapted = p123_adapter.verify(path)
            need(adapted['legacy_api'] is False and adapted['mode'] == 'decoded',
                 'structural adapter must use the frozen decoded current API')
            frozen_path = Path(adapted['baseline']).resolve(strict=True)
            need(frozen_path == ROOT/'target/performance/P046-decoded-before',
                 'structural adapter substituted the predeclared predecessor')
            core_manifest = baseline_tool.verify_archive(frozen_path)
            runtime_sources = {name: row['sha256'] for name,row in core_manifest['files'].items()
                               if row.get('source_path','').startswith('crates/wtime-runtime/src/')}
            need(len(runtime_sources) == 32, 'structural adapter has incomplete predecessor sources')
            predeclared,_ = _load_json(ROOT/'target/performance/P046/predeclaration/attempt-0002/receipt.json',
                                      'structural predecessor declaration')
            need(predeclared['baseline_manifest']['sha256'] == _sha(frozen_path/'manifest.json'),
                 'structural predecessor differs from its immutable predeclaration')
            need(_sha(path/'manifest.json') == metadata.get('baseline_manifest_sha256')
                 and adapted['binary_sha256'] == metadata.get('baseline_binary_sha256'),
                 'structural adapter report identity differs')
            return manifest
        need(manifest.get("mode") == "decoded",
             "structural baseline archive must resolve to decoded mode")
        runtime_sources = {name: row.get("sha256") for name, row in manifest.get("files", {}).items()
                           if isinstance(row, dict) and row.get("role") == "source"
                           and (row.get("source_path") or name.removeprefix("source/")).startswith(
                               "crates/wtime-runtime/src/")}
        need(len(runtime_sources) == 32 and all(_is_sha(value) for value in runtime_sources.values()),
             "structural baseline must bind the full 32-file P046 predecessor runtime")
        try:
            info = compare._baseline_identity(path, manifest, HERE / "src/protocol.rs", baseline_tool)
        except Exception as exc:
            raise PortfolioError(f"decoded structural baseline identity failed: {exc}") from exc
        need(info.get("kind") == "current-source-archive" and info.get("mode") == "decoded",
             "structural baseline must be a verified current-source decoded archive")
        need(_sha(path / "manifest.json") == metadata.get("baseline_manifest_sha256"),
             "decoded baseline manifest hash differs from comparison metadata")
        return manifest

    adapter_path = metadata.get("p001_adapter_path") or archive_path
    need(isinstance(adapter_path, str) and adapter_path,
         "P001 report must identify its verified typed adapter path")
    if not check_sources:
        return None
    adapter_path = Path(adapter_path).resolve(strict=True)
    try:
        import p123_adapter
        record = p123_adapter.verify(adapter_path)
    except Exception as exc:
        raise PortfolioError(f"P001 typed adapter failed verification: {exc}") from exc
    need(record.get("mode") == "decoded" and record.get("legacy_api") is True,
         "P001 route must use the decoded adapter against the immutable legacy API")
    need(record.get('baseline_manifest_sha256') == '7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472',
         'P001 comparison substituted the immutable original core')
    need(record.get("binary_sha256") == metadata.get("baseline_binary_sha256"),
         "P001 adapter binary differs from comparison metadata")
    need(_sha(adapter_path / "manifest.json") == metadata.get("baseline_manifest_sha256"),
         "P001 adapter manifest hash differs from comparison metadata")
    return record


def _report_groups(report: dict[str, Any], family: str, baseline_name: str,
                   *, check_sources: bool) -> dict[str, Any]:
    label = f"{baseline_name}/{family}"
    need(type(report.get("schema_version")) is int and report["schema_version"] == 2,
         f"{label} report schema must be version 2")
    need(report.get("passed") is True and type(report.get("passed")) is bool,
         f"{label} report passed must be strict true")
    need(report.get("source_stable") is True and type(report.get("source_stable")) is bool,
         f"{label} report source_stable must be strict true")
    need(report.get("selection") == family,
         f"{label} report selection differs")
    need(report.get("workload_subset") in (None, []),
         f"{label} subsetted reports are forbidden")
    metadata = report.get("metadata")
    need(isinstance(metadata, dict), f"{label} metadata is missing")
    settings = metadata.get("settings")
    need(isinstance(settings, dict), f"{label} metadata.settings is missing")
    need(settings.get("instrumented") is False and type(settings.get("instrumented")) is bool,
         f"{label} instrumented binaries cannot supply production timing")
    need(settings.get("profile") == "release",
         f"{label} candidate must use the release profile")
    need(settings.get("candidate_mode") == "compact",
         f"{label} candidate mode must be Compact")
    expected_baseline_mode = "decoded"
    need(settings.get("baseline_mode") == expected_baseline_mode,
         f"{label} baseline mode must be decoded")

    if family == "representative":
        need(settings.get("representative_stage") == "run_after_setup",
             "representative report must bind the run_after_setup timer")
        need(settings.get("representative_reset") is not None
             and settings.get("representative_measurement_policy") is not None,
             "representative report lacks source-bound setup/timing contract")

    source_map = metadata.get("candidate_sources")
    need(isinstance(source_map, dict) and source_map,
         f"{label} candidate source map is missing")
    need(all(isinstance(path, str) and _is_sha(digest)
             for path, digest in source_map.items()),
         f"{label} candidate source map contains malformed identities")
    if check_sources:
        current_map = _expected_source_map(family)
        try:
            execution_identity.verify(source_map, current_map)
        except ValueError as error:
            raise PortfolioError(f"{label} measured source map is stale: {error}") from error

    candidate_binary = metadata.get("candidate_binary_sha256")
    baseline_binary = metadata.get("baseline_binary_sha256")
    need(_is_sha(candidate_binary), f"{label} candidate binary identity is malformed")
    need(_is_sha(baseline_binary), f"{label} baseline binary identity is malformed")
    archive_record = _verify_baseline_archive(report, baseline_name,
                                               check_sources=check_sources)
    if baseline_name == "decoded":
        need(metadata.get("baseline_archive_kind") in ("current-source-archive", "frozen-typed-adapter"),
             f"{label} decoded baseline archive kind is invalid")
    else:
        need(metadata.get("baseline_archive_kind") in ("frozen-typed-adapter", "p123-typed-adapter"),
             f"{label} P001 archive kind is not a verified typed adapter")

    hardware = metadata.get("hardware")
    need(isinstance(hardware, dict), f"{label} hardware record is missing")
    selected_cpu = hardware.get("selected_cpu")
    need(type(selected_cpu) is int and selected_cpu >= 0,
         f"{label} selected CPU identity is missing")

    runs = report.get("runs")
    need(isinstance(runs, list) and len(runs) == 2,
         f"{label} must contain exactly two internal runs")
    run_ids = [run.get("run_id") if isinstance(run, dict) else None for run in runs]
    need(tuple(run_ids) in RUN_IDS,
         f"{label} internal run IDs must be ordered 0/1 or 1/2")
    seeds: set[int] = set()
    expected_groups = _expected_groups(family)
    normalized_runs: dict[int, dict[tuple[str, str], list[dict[str, Any]]]] = {}
    for run in runs:
        run_id = run["run_id"]
        seed = run.get("seed")
        need(type(seed) is int, f"{label} run {run_id} seed must be an integer")
        need(seed not in seeds, f"{label} run seeds must be distinct")
        seeds.add(seed)
        pairs = run.get("pairs")
        need(isinstance(pairs, list), f"{label} run {run_id} pairs must be an array")
        groups: dict[tuple[str, str], list[dict[str, Any]]] = {}
        pair_ids: set[str] = set()
        workload_inputs: dict[str, tuple[str, str, str | None, str | None]] = {}
        for pair in pairs:
            need(isinstance(pair, dict), f"{label} run {run_id} has a non-object pair")
            wid, stage = pair.get("workload"), pair.get("stage")
            group = (wid, stage)
            need(group in expected_groups,
                 f"{label} run {run_id} has unexpected workload-stage {group}")
            pair_id = pair.get("pair_id")
            need(isinstance(pair_id, str) and pair_id and pair_id not in pair_ids,
                 f"{label} run {run_id} has missing/duplicate pair id")
            pair_ids.add(pair_id)
            order = pair.get("order")
            need(isinstance(order, list) and len(order) == 2
                 and set(order) == {"baseline", "candidate"},
                 f"{label} pair {pair_id} order must be a side permutation")
            expected = pair.get("expected")
            need("expected" in pair, f"{label} pair {pair_id} is missing oracle result")
            try:
                expected_norm = _canonical(expected)
            except (TypeError, ValueError) as exc:
                raise PortfolioError(f"{label} pair {pair_id} has invalid oracle result") from exc
            for side_name, expected_mode in (("baseline", expected_baseline_mode),
                                             ("candidate", "compact")):
                side = pair.get(side_name)
                need(isinstance(side, dict),
                     f"{label} pair {pair_id} is missing {side_name} observation")
                mode = side.get("mode")
                need(mode == expected_mode,
                     f"{label} pair {pair_id} {side_name} mode differs")
                timing = side.get("ns_per_operation")
                need(type(timing) in (int, float) and math.isfinite(timing) and timing > 0,
                     f"{label} pair {pair_id} has invalid {side_name} timing")
                batch = side.get("batch")
                need(type(batch) is int and batch > 0,
                     f"{label} pair {pair_id} has invalid {side_name} batch")
                try:
                    result_norm = _canonical(side.get("result"))
                except (TypeError, ValueError) as exc:
                    raise PortfolioError(f"{label} pair {pair_id} has invalid {side_name} result") from exc
                need(result_norm == expected_norm,
                     f"{label} pair {pair_id} {side_name} result differs from normalized oracle")
            input_value = pair.get("input")
            input_norm = _canonical(input_value)
            wat_hash = pair.get("wat_sha256") or pair.get("wasm_sha256")
            wasm_hash = pair.get("wasm_sha256") or pair.get("sha256")
            if wat_hash is not None:
                need(_is_sha(wat_hash), f"{label} pair {pair_id} WAT identity is malformed")
            if wasm_hash is not None:
                need(_is_sha(wasm_hash), f"{label} pair {pair_id} Wasm identity is malformed")
            fixture_identity = (input_norm, expected_norm, wat_hash, wasm_hash)
            if wid in workload_inputs:
                need(workload_inputs[wid] == fixture_identity,
                     f"{label} workload {wid} changes its input/oracle/fixture identity")
            else:
                workload_inputs[wid] = fixture_identity
            groups.setdefault(group, []).append(pair)
        need(set(groups) == expected_groups,
             f"{label} run {run_id} is missing a workload or stage")
        need(len(pair_ids) == len(expected_groups) * MIN_PAIRS,
             f"{label} run {run_id} must contain exactly 30 pairs per group")
        for group, rows in groups.items():
            need(len(rows) == MIN_PAIRS,
                 f"{label} run {run_id} {group} has {len(rows)} pairs; expected 30")
        normalized_runs[run_id] = groups

    # The shared analyzer validates its full wire contract but keeps bootstrap
    # statistics out of this validation pass. The composite statistics call
    # computes uncertainty once per unique raw report later.
    try:
        analyzer_metadata, analyzer_runs = analyze._validated_runs(report)
    except (analyze.ReportError, TypeError, KeyError, ValueError) as exc:
        raise PortfolioError(f"{label} paired report validation failed: {exc}") from exc
    need(analyzer_metadata is metadata and len(analyzer_runs) == 2,
         f"{label} analyzer validation differs from raw report")
    need(archive_record is None or isinstance(archive_record, dict),
         f"{label} baseline source receipt is invalid")
    return {"report": report, "candidate_binary_sha256": candidate_binary,
            "baseline_binary_sha256": baseline_binary,
            "candidate_sources": execution_identity.project(source_map),
            "hardware": hardware, "groups_per_run": len(expected_groups),
            "pairs_per_run": len(expected_groups) * MIN_PAIRS,
            "pairs_total": len(expected_groups) * MIN_PAIRS * 2,
            "expected_groups": expected_groups}


def _p001_support_map(value: Any) -> tuple[dict[str, bool], dict[str, Any]]:
    if type(value) is bool:
        return ({family: value for family in FAMILIES}, {})
    need(isinstance(value, dict), "p001_supported must be a bool or per-family map")
    supported: dict[str, bool] = {}
    limitations: dict[str, Any] = {}
    for family in FAMILIES:
        row = value.get(family)
        if type(row) is bool:
            supported[family] = row
        elif isinstance(row, dict) and type(row.get("supported")) is bool:
            supported[family] = row["supported"]
            if not row["supported"]:
                need(row.get("kind") in ("compile", "profile", "api"),
                     f"P001 {family} limitation must cite a concrete capability failure")
                need(isinstance(row.get("reason"), str) and row["reason"].strip(),
                     f"P001 {family} limitation needs a reason")
                need(isinstance(row.get("evidence"), dict)
                     and _is_sha(row["evidence"].get("sha256")),
                     f"P001 {family} limitation needs a hashed capability receipt")
                limitations[family] = row
        else:
            raise PortfolioError(f"P001 support status is missing for {family}")
    need(set(value) == set(FAMILIES), "P001 support map must cover all four families")
    return supported, limitations


def validate_matrix(reports: Any, *, p001_supported: Any = True,
                    check_sources: bool = True) -> dict[str, Any]:
    """Validate the full decoded matrix and every P001 route declared supported.

    `reports` is `{decoded: {family: report}, p001: {family: report}}`. P001
    can be a bool (uniform support) or a complete per-family support map whose
    unsupported rows include a concrete hashed capability receipt.
    """
    need(type(check_sources) is bool, "check_sources must be a strict bool")
    need(isinstance(reports, dict), "portfolio reports must be an object")
    # The public P046 portfolio schema calls the current structural baseline
    # `structural`; keep `decoded` as a direct unit-level alias.
    if "structural" in reports:
        need("decoded" not in reports, "portfolio has duplicate structural baseline keys")
        reports = {**reports, "decoded": reports["structural"]}
        reports.pop("structural", None)
    need(set(reports) <= set(BASELINES), "portfolio contains an unknown baseline matrix")
    decoded = reports.get("decoded")
    need(isinstance(decoded, dict) and set(decoded) == set(FAMILIES),
         "complete four-family decoded structural matrix is required")
    p001_map, limitations = _p001_support_map(p001_supported)
    need(all(p001_map.values()),
         "P046 requires all four P001 families; capability limitations cannot reduce the matrix")
    p001 = reports.get("p001", {})
    need(isinstance(p001, dict), "P001 report matrix must be an object")
    required_p001 = {family for family, supported in p001_map.items() if supported}
    need(set(p001) == set(FAMILIES) == required_p001,
         "complete four-family P001 matrix is required; subset reports are forbidden")

    identities: dict[tuple[str, str], dict[str, Any]] = {}
    for baseline_name, family_reports in (("decoded", decoded), ("p001", p001)):
        for family, source in family_reports.items():
            report, _digest = _load_json(source, f"{baseline_name}/{family} report")
            identities[(baseline_name, family)] = _report_groups(
                report, family, baseline_name, check_sources=check_sources)
            if check_sources and family == 'representative':
                _representative_images(report, source)

    candidate_hashes = {row["candidate_binary_sha256"] for row in identities.values()}
    need(len(candidate_hashes) == 1,
         "all portfolio reports must use the same Compact candidate binary")
    common_source_paths = set.intersection(*(set(row["candidate_sources"])
                                             for row in identities.values()))
    for path in common_source_paths:
        values = {row["candidate_sources"][path] for row in identities.values()}
        need(len(values) == 1, f"candidate source identity differs across matrix: {path}")
    cpu_ids = {row["hardware"].get("selected_cpu") for row in identities.values()}
    need(len(cpu_ids) == 1, "portfolio reports must use the same selected CPU")
    for field in ("cpu_model", "cpu_quota"):
        values = {row["hardware"].get(field) for row in identities.values()}
        need(len(values) == 1 and all(isinstance(value, str) and value.strip()
                                      for value in values),
             f"portfolio reports must bind the same nonempty hardware {field}")
    for field in ("os", "physical_core_idle"):
        values = {row["hardware"].get(field) for row in identities.values()
                  if row["hardware"].get(field) is not None}
        need(len(values) <= 1, f"portfolio reports disagree on hardware {field}")

    counts = {baseline_name: {
        family: {"groups_per_run": identity["groups_per_run"],
                 "pairs_per_run": identity["pairs_per_run"],
                 "pairs_total": identity["pairs_total"]}
        for (matrix, family), identity in identities.items() if matrix == baseline_name
    } for baseline_name in BASELINES}
    need(sum(row["pairs_total"] for row in counts["decoded"].values()) == 65_700,
         "decoded full matrix must total 65,700 paired observations")
    if required_p001:
        p001_pair_total = sum(row["pairs_total"] for row in counts["p001"].values())
    else:
        p001_pair_total = 0
    return {"status": "passed", "candidate_binary_sha256": next(iter(candidate_hashes)),
            "selected_cpu": next(iter(cpu_ids)), "counts": counts,
            "decoded_pairs_total": 65_700, "p001_pairs_total": p001_pair_total,
            "p001_supported": p001_map, "p001_limitations": limitations,
            "reports": identities}


def validate_p121(record: Any, *, check_sources: bool = True) -> dict[str, Any]:
    """Validate source-bound four-suite P121 evidence and aggregate invariants."""
    value, _digest = _load_json(record, "P121 corpus record")
    need(value.get("status") == "passed" and value.get("task") == "P121",
         "P121 corpus record is missing or failed")
    need(value.get("timing_evidence") is False,
         "P121 corpus record must remain diagnostic, without timing evidence")
    reports = value.get("reports")
    need(isinstance(reports, list) and len(reports) == 4,
         "P121 must bind all four full-suite reports")
    selection_names = {row.get("suite") for row in reports if isinstance(row, dict)}
    need(selection_names == P121_SELECTIONS,
         "P121 must cover mvp/core3/phase3/interactions exactly once")
    totals = value.get("totals")
    need(isinstance(totals, dict), "P121 aggregate totals are missing")
    for field in ("selected", "executed"):
        need(type(totals.get(field)) is int and totals[field] == P121_EXPECTED_DIRECTIVES,
             f"P121 {field} directive count must equal {P121_EXPECTED_DIRECTIVES}")
    need(totals["selected"] == totals["executed"],
         "P121 selected and executed counts differ")
    # The P121 producer rejects any per-suite failed, skipped, unsupported,
    # or malformed report before emitting this aggregate. Also honor the
    # published P121-result.json shape when it is supplied as the input.
    for field in ("failed", "skipped", "unsupported"):
        if field in totals:
            need(type(totals[field]) is int and totals[field] == 0,
                 f"P121 has nonzero {field} corpus outcomes")
    for row in reports:
        need(isinstance(row, dict) and type(row.get("selected")) is int
             and row.get("selected") == row.get("executed")
             and row.get("selected", 0) > 0,
             "P121 suite has incomplete selected/executed coverage")
        for field in ("failed", "skipped", "unsupported"):
            if field in row:
                need(type(row[field]) is int and row[field] == 0,
                     f"P121 suite has nonzero {field} outcomes")
        if "preparation_summary" in row:
            basis_text = row["preparation_summary"].get("artifact_retained_budget_bytes_basis")
            need(isinstance(basis_text, str) and "conservative" in basis_text.lower(),
                 "P121 suite does not state conservative retained-byte accounting")
    total = totals.get("total_dispatched_source_events")
    optimized = totals.get("optimized_compact_source_events")
    fallback = totals.get("decoded_fallback_source_events")
    need(all(type(item) is int and item >= 0 for item in (total, optimized, fallback)),
         "P121 source-event counts are malformed")
    need(total > 0 and optimized > 0 and fallback == total - optimized,
         "P121 has zero optimized Compact work or inconsistent fallback accounting")
    prep = totals.get("preparation")
    need(isinstance(prep, dict), "P121 preparation totals are missing")
    # P121's producer names these fields precisely; published summary files
    # use a shorter retained-budget alias.
    if "retained_budget_bytes" not in prep and "artifact_retained_budget_bytes" in prep:
        prep = dict(prep, retained_budget_bytes=prep["artifact_retained_budget_bytes"])
    if "unpublished_artifacts" not in prep and "unpublished_artifacts_unknown_work" in prep:
        prep = dict(prep, unpublished_artifacts=prep["unpublished_artifacts_unknown_work"],
                    attempted_work_unknown_artifacts=prep["unpublished_artifacts_unknown_work"])
    required_counts = ("accepted", "declined", "published_artifacts",
                       "unpublished_artifacts", "attempted_work_known_sum",
                       "attempted_work_unknown_artifacts", "retained_budget_bytes")
    for field in required_counts:
        need(type(prep.get(field)) is int and prep[field] >= 0,
             f"P121 preparation field {field} is malformed")
    need(prep["published_artifacts"] > 0 and prep["accepted"] + prep["declined"] > 0,
         "P121 preparation has zero accepted/prepared work")
    need(prep["unpublished_artifacts"] > 0
         and prep["attempted_work_unknown_artifacts"] == prep["unpublished_artifacts"],
         "P121 unknown unpublished work is not explicitly accounted")
    need(prep["retained_budget_bytes"] > 0,
         "P121 retained code budget must be nonzero")
    basis = (prep.get("retained_budget_basis") or prep.get("artifact_retained_budget_bytes_basis")
             or value.get("retained_budget_basis"))
    if basis is None:
        # The real P121 aggregate sums numeric counters; its accounting
        # explanation belongs to every source-bound per-suite summary.
        summaries = [row.get('preparation_summary', {}) for row in reports]
        bases = [summary.get('artifact_retained_budget_bytes_basis') for summary in summaries]
        byte_counts = [summary.get('artifact_retained_budget_bytes') for summary in summaries]
        need(all(isinstance(text, str) and 'conservative' in text.lower() for text in bases)
             and len(set(bases)) == 1,
             'P121 suites must agree on conservative retained-byte accounting')
        need(all(type(count) is int and count >= 0 for count in byte_counts)
             and sum(byte_counts) == prep['retained_budget_bytes'],
             'P121 aggregate retained bytes differ from the four suite summaries')
        basis = bases[0]
    need(isinstance(basis, str) and "conservative" in basis.lower(),
         "P121 retained budget must state its conservative accounting basis")

    if check_sources:
        # Re-run the producer's real source/corpus verifier in a fresh temp
        # directory. It checks locked corpus membership, binary identity,
        # per-suite reports, execution counters, preparation arithmetic, and
        # its generated aggregate. Paths must identify actual immutable files.
        import tempfile
        from verification.producers import p121 as producer
        need(isinstance(value.get("current_binary"), dict),
             "P121 aggregate lacks the instrumented source-bound binary ref")
        binary_ref = value["current_binary"]
        binary = Path(binary_ref.get("path", "")).resolve(strict=True)
        need(_sha(binary) == binary_ref.get("sha256"),
             "P121 current binary hash is stale")
        report_paths = []
        for row in reports:
            ref = row.get("report")
            need(isinstance(ref, dict), "P121 per-suite report reference is missing")
            path = Path(ref.get("path", "")).resolve(strict=True)
            need(_sha(path) == ref.get("sha256"), "P121 per-suite report hash is stale")
            report_paths.append(path)
        with tempfile.TemporaryDirectory(prefix="p046-p121-") as temp:
            output = Path(temp) / "p121-verified.json"
            try:
                producer.validate(report_paths, binary, output, ROOT)
            except Exception as exc:
                raise PortfolioError(f"P121 producer source/corpus validation failed: {exc}") from exc
            verified = json.loads(output.read_text(encoding="utf-8"))
        verified_totals = verified.get("totals", {})
        for field in ("selected", "executed", "total_dispatched_source_events",
                      "optimized_compact_source_events", "decoded_fallback_source_events"):
            need(verified_totals.get(field) == totals.get(field),
                 f"P121 producer-recomputed {field} differs from saved aggregate")
        need(verified_totals.get("preparation", {}).get("artifact_retained_budget_bytes")
             == prep["retained_budget_bytes"],
             "P121 saved aggregate differs from producer-recomputed evidence")
    return {"status": "passed", "selected": totals["selected"],
            "executed": totals["executed"], "optimized_source_events": optimized,
            "fallback_source_events": fallback,
            "retained_budget_bytes": prep["retained_budget_bytes"],
            "unknown_unpublished_artifacts": prep["attempted_work_unknown_artifacts"]}


def _load_p123_support(value: Any, *, check_sources: bool) -> tuple[dict[str, Any], dict[str, bool]]:
    record, _digest = _load_json(value, "P123 capability receipt")
    need(record.get("schema") == "p123-capabilities-v1" and record.get("status") == "passed",
         "P123 typed adapter capability receipt is missing or failed")
    try:
        from verification.producers import p123 as producer
        producer.validate_report(record)
    except Exception as exc:
        raise PortfolioError(f"P123 capability report failed producer validation: {exc}") from exc
    rows = record.get("families")
    need(isinstance(rows, dict) and set(rows) == set(FAMILIES),
         "P123 receipt must state actual capability for all four families")
    support: dict[str, bool] = {}
    for family, row in rows.items():
        need(isinstance(row, dict) and type(row.get("supported")) is int
             and type(row.get("passed")) is int and type(row.get("unsupported")) is int,
             f"P123 {family} capability counts are malformed")
        support[family] = row["passed"] == row["supported"] and row["unsupported"] == 0
        need(support[family] or row["unsupported"] > 0,
             f"P123 {family} capability rows are neither fully supported nor explicitly unsupported")
    if check_sources:
        # The P123 producer's report binds the adapter, original P001 binary,
        # source-current harness, and every family/stage fixture observation.
        need(record.get("source_stable") is True and record.get("compile_source_stable") is True,
             "P123 adapter or compile source changed during capability probes")
        adapter_ref = record.get('adapter', {})
        path = Path(adapter_ref.get('path','')).resolve(strict=True)
        need(_sha(path/'manifest.json') == adapter_ref.get('manifest_sha256') == record['adapter_manifest_sha256'],
             'P123 original adapter manifest changed')
        import p123_adapter
        verified = p123_adapter.verify(path, check_current=False)
        need(verified['legacy_api'] is True and verified['mode'] == 'decoded'
             and verified['binary_sha256'] == record['binary_sha256'],
             'P123 original adapter binary/API differs')
        measured = record.get('source_identities', {})
        required = [str(source.relative_to(ROOT)) for source in (HERE/'src').glob('*.rs')]
        required += ['tools/wtime-bench/'+name for name in (
            'scalar_workloads.py','memory_workloads.py','simd_workloads.py','representative_workloads.py',
            'compare.py','representative_compare.py','memory_scaling.py','simd_scaling.py',
            'representative_scaling.py','run.py','workloads.py','workloads.json')]
        need(all(measured.get(name) == _sha(ROOT/name) for name in required),
             'P123 measured compilation/oracle inputs changed; fresh probes are required')
    return record, support


def _build_decision(build_path: Path, optional_build_path: Path | None,
                    portfolio: dict[str, Any], candidate_build_record_path: Path | None,
                    *, check_sources: bool) -> dict[str, Any]:
    """Apply the shared P005 absolute, relative, dependency and feature checks."""
    import gates
    report_refs = portfolio.get("structural_build_reports")
    need(isinstance(report_refs, dict) and isinstance(report_refs.get("default"), dict)
         and isinstance(report_refs.get("compact"), dict),
         "portfolio must bind source-stable default and Compact build reports")
    default_path = Path(report_refs["default"].get("path", "")).resolve(strict=True)
    compact_path = Path(report_refs["compact"].get("path", "")).resolve(strict=True)
    for label, ref, path in (("default build", report_refs["default"], default_path),
                             ("Compact build", report_refs["compact"], compact_path)):
        need(_is_sha(ref.get("sha256")) and _sha(path) == ref["sha256"],
             f"{label} report hash is stale")
    failures = []
    try:
        default = gates._validate_build(default_path, expected_mode="default",
                                        label="P046 default build")
        default_budget_pass = True
    except Exception as exc:
        message = str(exc)
        if "exceeds clean_seconds" in message or "exceeds leaf_seconds" in message \
                or "exceeds stripped_bytes" in message or "budget_pass=false" in message:
            default_budget_pass = False
            failures.append(f"default build hard budget: {message}")
        else:
            raise PortfolioError(f"default build evidence failed integrity validation: {message}") from exc
        default = gates._read_json(default_path, "P046 default build")
    try:
        compact = gates._validate_build(compact_path, expected_mode="optional-feature",
                                        label="P046 Compact feature build")
        compact_budget_pass = True
    except Exception as exc:
        message = str(exc)
        if "exceeds clean_seconds" in message or "exceeds leaf_seconds" in message \
                or "exceeds stripped_bytes" in message or "budget_pass=false" in message:
            compact_budget_pass = False
            failures.append(f"Compact build hard budget: {message}")
        else:
            raise PortfolioError(f"Compact build evidence failed integrity validation: {message}") from exc
        compact = gates._read_json(compact_path, "P046 Compact build")
    try:
        dependency_violations = gates._check_default_dependencies()
    except Exception as exc:
        dependency_violations = [str(exc)]
        failures.append(f"default dependency closure: {exc}")
    need(default_path.resolve() == Path(build_path).resolve(strict=True),
         "statistics build argument differs from portfolio's default build evidence")
    if optional_build_path is not None:
        need(compact_path.resolve() == Path(optional_build_path).resolve(strict=True),
             "statistics optional-build argument differs from portfolio's Compact build evidence")
    else:
        raise PortfolioError("P046 requires the Compact optional-feature build report")
    baseline_path = Path(portfolio.get("frozen_build_report", "")).resolve(strict=True)
    frozen = gates._read_json(baseline_path, "frozen original build report")
    relative = gates._relative_build_growth(default, frozen)
    compact_relative = gates._relative_build_growth(compact, default)
    explanation = portfolio.get("build_growth_explanation")
    growth_review = bool(relative or compact_relative)
    explanation_review = growth_review and not (isinstance(explanation, str) and explanation.strip())
    candidate_attestation = None
    if candidate_build_record_path is not None:
        # The first full report is enough to bind the shared candidate binary;
        # validate all report source maps separately in validate_matrix.
        report_ref = portfolio["reports"]["structural"]["scalar"]
        raw, _ = _load_json(report_ref, "structural/scalar report")
        candidate_attestation = gates._validate_candidate_build_record(
            candidate_build_record_path, raw["metadata"], raw)
    return {"default_budget_pass": default_budget_pass,
            "compact_budget_pass": compact_budget_pass,
            "dependency_violations": dependency_violations,
            "relative_growth_to_frozen_original": relative,
            "compact_relative_growth_to_default": compact_relative,
            "relative_growth_review": growth_review,
            "relative_growth_explanation_missing": explanation_review,
            "hard_failures": failures,
            "candidate_build": candidate_attestation}


def _retained_gate(value: Any, *, check_sources: bool) -> dict[str, Any]:
    record, _digest = _load_json(value, "retained code evidence")
    need(record.get("task") == "P046" and record.get("source_stable") is True,
         "retained code evidence is missing or stale")
    metrics = record.get("bytes")
    need(isinstance(metrics, dict), "retained code evidence lacks measured byte counts")
    ratios = {}
    labels=('module_code_graph_requested_bytes',) if 'decoded_module_code_graph_requested_bytes' in metrics else ('module','code')
    for label in labels:
        before = metrics.get(f"decoded_{label}")
        after = metrics.get(f"compact_{label}")
        need(type(before) is int and before > 0 and type(after) is int and after > 0,
             f"retained {label} bytes are missing or zero")
        ratio = after / before
        ratios[label] = ratio
    growth = [label for label, ratio in ratios.items() if ratio > 1.25]
    explanation = record.get("simplification_packaging_decision")
    relative_status = "within_budget" if not growth else "requires_review"
    explanation_missing = bool(growth and not (isinstance(explanation, str) and explanation.strip()))
    if check_sources:
        refs = record.get("artifacts")
        need(isinstance(refs, dict) and refs,
             "retained code evidence must bind measured artifact files")
        for name, ref in refs.items():
            need(isinstance(ref, dict) and isinstance(ref.get("path"), str)
                 and _is_sha(ref.get("sha256")),
                 f"retained artifact {name} is not hash-bound")
            need(_sha(Path(ref["path"]).resolve(strict=True)) == ref["sha256"],
                 f"retained artifact {name} is stale")
        need(record.get('schema') == 'p046-retained-v1'
             and record.get('status') == 'passed' and record.get('timing_evidence') is False,
             'retained graph evidence must be the actual diagnostic producer report')
        sys.path.insert(0, str(ROOT/'tools/verification/producers'))
        import p046_verify as producer
        execution_identity.verify(record.get('source_identities'), producer._production_sources())
        with tempfile.TemporaryDirectory(prefix='p046-retained-validation-') as directory:
            products = producer._product_rows(Path(directory)/'generated')
        expected = {(row['source_sha256'], row['profile']): row for row in products}
        rows = record.get('rows')
        need(isinstance(rows, list) and len(rows) == len(expected),
             'retained graph portfolio is incomplete')
        actual = {(row['source_sha256'], row['profile']): row for row in rows}
        need(set(actual) == set(expected), 'retained graph portfolio substituted a product')
        for key, row in actual.items():
            need(row['wasm_sha256'] == expected[key]['wasm_sha256']
                 and row['members'] == expected[key]['members']
                 and row['fixture']['sha256'] == row['wasm_sha256'],
                 'retained graph fixture differs from the full independent registry')
            for mode in ('decoded', 'compact'):
                diagnostic = row[mode]
                need(diagnostic.get('mode') == mode and diagnostic.get('profile') == key[1]
                     and diagnostic.get('instrumented') is True
                     and diagnostic.get('timing_evidence') is False
                     and diagnostic.get('graph_released') is True,
                     'retained graph observation is not the released diagnostic product')
        for mode in ('decoded', 'compact'):
            need(metrics[mode+'_module_code_graph_requested_bytes'] ==
                 sum(row[mode]['compiled_graph_requested_bytes'] for row in rows),
                 'retained graph aggregate differs from measured rows')
    return {"status": "passed", "ratios": ratios, "relative_status": relative_status,
            "growth_fields": growth, "explanation_missing": explanation_missing,
            "simplification_packaging_decision": explanation}


def _family_statistics(report, identity):
    """Fixed, equal workload weights with within-group paired resampling."""
    statistics=[]
    for number,run in enumerate(report['runs'],1):
        groups={}
        for pair in run['pairs']:
            if pair['stage'] == 'warm_call':
                groups.setdefault(pair['workload'],[]).append(math.log(
                    pair['candidate']['ns_per_operation']/pair['baseline']['ns_per_operation']))
        if not groups:continue
        ordered=[sorted(values) for _,values in sorted(groups.items())]
        point=math.exp(math.fsum(analyze._percentile(values,.5) for values in ordered)/len(ordered))
        seed=int.from_bytes(hashlib.sha256(f'P046-family-v1:{identity}:{number}:{run["seed"]}'.encode()).digest()[:8],'big')
        rng=random.Random(seed)
        draws=[math.exp(math.fsum(draw_median(values,rng) for values in ordered)/len(ordered))
               for _ in range(analyze.BOOTSTRAP_RESAMPLES)]
        statistics.append({'run':number,'median_ratio':point,
                           'ci95':{'lower':analyze._percentile(draws,.025),'upper':analyze._percentile(draws,.975)},
                           'workloads':len(ordered),'bootstrap_resamples':analyze.BOOTSTRAP_RESAMPLES,
                           'bootstrap_seed':seed,'weighting':'equal fixed workload weights; within-workload paired resampling'})
    return statistics


def _cold_controls(portfolio, matrix, *, check_sources=True):
    """Full original-MVP controls include large preparation and one-shot use."""
    references=portfolio.get('cold_controls')
    need(isinstance(references,dict) and set(references)=={'structural','p001'},
         'both full original-MVP cold controls are required')
    entries={row['id']:row for row in workloads.selection('original-mvp')}
    expected={(name,stage) for name,row in entries.items() for stage in row['stages']}
    raw={}
    for baseline_name,reference in references.items():
        report,digest=_load_json(reference,'cold '+baseline_name)
        need(report.get('selection')=='original-mvp' and report.get('passed') is True
             and report.get('source_stable') is True and not report.get('workload_subset'),
             'cold controls must use the complete successful original-MVP selection')
        metadata=report['metadata'];settings=metadata['settings']
        need(settings.get('candidate_mode')=='compact' and settings.get('baseline_mode')=='decoded'
             and settings.get('instrumented') is False and settings.get('profile')=='release',
             'cold controls require production Compact/Decoded products')
        need(metadata['candidate_binary_sha256']==matrix['candidate_binary_sha256']
             and metadata['hardware']['selected_cpu']==matrix['selected_cpu'],
             'cold controls changed candidate or selected CPU')
        if check_sources:
            execution_identity.verify(metadata.get('candidate_sources'), compare.sources())
            _verify_baseline_archive(report,'decoded' if baseline_name=='structural' else 'p001',check_sources=True)
        try:
            _,runs=analyze._validated_runs(report)
        except analyze.ReportError as error:
            raise PortfolioError('cold control wire contract failed: '+str(error)) from error
        for run in runs:
            groups={(key[0],key[1]):rows for key,rows in run['groups'].items()}
            need(set(groups)==expected and len(run['groups'])==len(expected)
                 and all(len(rows)==30 for rows in groups.values()),
                 'cold controls are missing a workload, stage or paired observation')
            for (name,stage),rows in groups.items():
                need(all(row['wasm_sha256']==entries[name]['source']['wasm_sha256'] for row in rows),
                     'cold fixture identity differs from the independent registry')
        raw[baseline_name]=(report,digest)
    return raw


def _cold_policy(summaries, references):
    reasons=[];failed=False;uncertain=False
    for baseline,digest in references.items():
        for row in summaries[digest]['per_run']:
            if (row['workload'],row['stage']) not in (
                ('call_overhead','prepare'),('prepare_large','prepare'),
                ('call_overhead','first_call'),('call_overhead','load_and_call')):continue
            limit=1.25 if row['stage']=='prepare' else 1.05
            interval=row['median_ratio_95ci']
            if interval['upper']>limit:
                reasons.append(f'{baseline} {row["workload"]}/{row["stage"]} run{row["run"]} cold control crosses {limit}')
                if interval['lower']>limit:failed=True
                else:uncertain=True
    return ('rejected' if failed else 'inconclusive' if uncertain else 'adopted'),reasons


def _analysis_policy(summaries: dict[str, Any], summary_refs: dict[str, str],
                     matrix: dict[str, Any], policy: Any) -> tuple[str, list[str], dict[str, Any]]:
    """Apply predeclared family promotion and protected-control rules."""
    need(isinstance(policy, dict), "P046 decision policy is missing")
    primary = policy.get("primary_families")
    protected = policy.get("protected_families")
    need(isinstance(primary, list) and primary
         and all(item in FAMILIES for item in primary),
         "P046 primary family declaration is malformed")
    need(isinstance(protected, list) and all(item in FAMILIES for item in protected)
         and not set(primary) & set(protected),
         "P046 protected family declaration is malformed")
    need(set(primary) | set(protected) == set(FAMILIES),
         "P046 policy must assign all four families to primary or protected controls")
    minimum_gain = policy.get("minimum_warm_improvement")
    protected_limit = policy.get("protected_upper_ratio_limit")
    preparation_limit = policy.get("preparation_upper_ratio_limit")
    cold_limit = policy.get("cold_upper_ratio_limit")
    for label, value in (("minimum_warm_improvement", minimum_gain),
                         ("protected_upper_ratio_limit", protected_limit),
                         ("preparation_upper_ratio_limit", preparation_limit),
                         ("cold_upper_ratio_limit", cold_limit)):
        need(type(value) in (int, float) and math.isfinite(value) and value > 0,
             f"P046 policy {label} is malformed")
    need(minimum_gain < 1.0, "P046 minimum warm ratio must represent an improvement")

    rows_by_report: dict[str, list[dict[str, Any]]] = {}
    for report_key, digest in summary_refs.items():
        summary = summaries[digest]
        rows = summary.get("per_run")
        need(isinstance(rows, list) and rows,
             f"{report_key} paired analysis has no per-run intervals")
        rows_by_report[report_key] = rows
    intervals: dict[str, Any] = {}
    reasons: list[str] = []
    hard_failure = False
    inconclusive = False
    for family in FAMILIES:
        report_key = f"decoded/{family}"
        rows = rows_by_report.get(report_key)
        need(rows is not None, f"current decoded analysis is missing {family}")
        intervals[report_key] = rows
        for run in (1, 2):
            run_rows = [row for row in rows if row.get("run") == run]
            need(run_rows, f"{family} analysis is missing run {run}")
            if family in primary:
                warm = [row for row in run_rows if row.get("stage") == "warm_call"]
                if not warm:
                    inconclusive = True
                    reasons.append(f"{family} has no declared warm groups in run {run}")
                else:
                    aggregates=summaries[summary_refs[report_key]].get('portfolio_family',[])
                    matches=[row for row in aggregates if row['run']==run]
                    need(len(matches)==1 and matches[0]['workloads']==len(warm),
                         f'{family} equally weighted paired family interval is missing')
                    aggregate=matches[0]
                    intervals.setdefault('primary_family',{}).setdefault(family,[]).append(aggregate)
                    if aggregate['median_ratio']>minimum_gain or aggregate['ci95']['upper']>=1.0:
                        inconclusive=True
                        reasons.append(f'{family} family run {run} does not establish a5% improvement')
                    for row in warm:
                        interval = row.get("median_ratio_95ci", {})
                        ratio = row.get("candidate_over_baseline", {}).get("median")
                        if (type(ratio) not in (int, float)
                                or type(interval.get("upper")) not in (int, float)
                                or interval["upper"] > protected_limit):
                            inconclusive = True
                            reasons.append(f"{family}/{row.get('workload')} warm run {run} exceeds protected interval")
                            if type(interval.get('lower')) in (int,float) and interval['lower']>protected_limit:
                                hard_failure=True
            else:
                controls = [row for row in run_rows
                            if row.get("stage") in ("warm_call", "run_after_setup")]
                if not controls:
                    inconclusive = True
                    reasons.append(f"{family} has no protected timing controls in run {run}")
                for row in controls:
                    upper = row.get("median_ratio_95ci", {}).get("upper")
                    if type(upper) not in (int, float) or upper > protected_limit:
                        inconclusive = True
                        reasons.append(f"{family}/{row.get('workload')} protected run {run} exceeds its interval")

            for row in run_rows:
                stage = row.get("stage")
                limit = preparation_limit if stage == "prepare" else cold_limit
                if stage in ("prepare", "instantiate", "first_call", "load_and_call", "run_after_setup"):
                    upper = row.get("median_ratio_95ci", {}).get("upper")
                    if type(upper) not in (int, float):
                        inconclusive = True
                        reasons.append(f"{family}/{row.get('workload')} {stage} run {run} lacks an interval")
                    elif upper > limit:
                        if row['median_ratio_95ci']['lower']>limit:hard_failure=True
                        else:inconclusive=True
                        reasons.append(f"{family}/{row.get('workload')} {stage} run {run} exceeds its cold interval")

    # Historical P001 rows are explanatory; they do not replace current
    # decoded controls or hide a current-source regression.
    for report_key, rows in rows_by_report.items():
        if not report_key.startswith("p001/"):
            continue
        intervals[report_key] = rows
        for row in rows:
            if row.get("stage") == "warm_call" and row.get("decision") == "regression":
                inconclusive = True
                reasons.append(f"{report_key}/{row.get('workload')} regresses against P001")
    if hard_failure:
        return "rejected", reasons, intervals
    if inconclusive or matrix["p001_pairs_total"] == 0:
        return "inconclusive", reasons or ["P001 has no capability-proven timed family rows"], intervals
    return "adopted", reasons, intervals


def evaluate_portfolio(portfolio_path: Path, build_path: Path,
                       optional_build_path: Path | None = None,
                       contract_path: Path | None = None,
                       candidate_build_record_path: Path | None = None,
                       *, analyzer_fn=None, check_sources: bool = True) -> dict[str, Any]:
    """Shared P046 entry point. Validates all inputs and computes eight analyses."""
    portfolio, portfolio_sha = _load_json(portfolio_path, "P046 portfolio input")
    need(portfolio.get("schema") == "p046-portfolio-v1",
         "portfolio schema must be p046-portfolio-v1")
    reports = portfolio.get("reports")
    need(isinstance(reports, dict) and set(reports) == {"structural", "p001"},
         "portfolio must bind structural and P001 report matrices")
    p123, support = _load_p123_support(portfolio.get("p123"), check_sources=check_sources)
    matrix = validate_matrix(reports, p001_supported=support,
                             check_sources=check_sources)
    p121 = validate_p121(portfolio.get("p121"), check_sources=check_sources)
    build = _build_decision(Path(build_path), optional_build_path, portfolio,
                            candidate_build_record_path, check_sources=check_sources)
    retained = _retained_gate(portfolio.get("retained"), check_sources=check_sources)
    cold = _cold_controls(portfolio,matrix,check_sources=check_sources) if check_sources or portfolio.get('cold_controls') else {}

    analyzer_fn = analyzer_fn or analyze.analyze
    summaries: dict[str, Any] = {}
    summary_refs: dict[str, str] = {}
    for baseline_name, name in (("structural", "decoded"), ("p001", "p001")):
        for family in FAMILIES:
            if family not in reports[baseline_name]:
                continue
            report, digest = _load_json(reports[baseline_name][family],
                                        f"{baseline_name}/{family} report")
            if digest not in summaries:
                summaries[digest] = analyzer_fn(report)
                if analyzer_fn is analyze.analyze:
                    summaries[digest]['portfolio_family']=_family_statistics(report,digest)
            summary_refs[f"{name}/{family}"] = digest
    need(len(summary_refs) == 8,
         "portfolio analysis requires eight complete structural and P001 reports")
    need(len(summaries) == 8,
         "each of the eight portfolio reports must have a distinct raw SHA-256 identity")
    need(len(summaries) == len(set(summary_refs.values())),
         "unique raw comparison reports were not analyzed exactly once")
    cold_refs={}
    for baseline_name,(report,digest) in cold.items():
        need(digest not in summaries, 'cold controls duplicate a family dataset')
        summaries[digest]=analyzer_fn(report)
        cold_refs[baseline_name]=digest
    contract = None
    if contract_path is not None:
        contract, _contract_sha = _load_json(contract_path, "P124 portfolio policy contract")
        need(contract.get("schema") == "p046-portfolio-policy-v1",
             "P124 contract must declare p046-portfolio-policy-v1")
    policy = portfolio.get("decision_policy") or (contract or {}).get("decision_policy")
    decision, reasons, policy_analysis = _analysis_policy(summaries, summary_refs, matrix, policy)
    if cold_refs:
        cold_decision,cold_reasons=_cold_policy(summaries,cold_refs)
        reasons.extend(cold_reasons)
        if cold_decision=='rejected':decision='rejected'
        elif cold_decision=='inconclusive' and decision=='adopted':decision='inconclusive'
    if not build["default_budget_pass"] or not build["compact_budget_pass"] \
            or build["dependency_violations"] or not p121["optimized_source_events"] \
            or not retained["status"] == "passed":
        decision = "rejected"
        reasons.append("a hard build, dependency, corpus, or retained-code gate failed")
    elif (retained["relative_status"] == "requires_review" or build["relative_growth_review"]) \
            and decision == "adopted":
        decision = "inconclusive"
        reasons.append("relative size/build growth requires documented packaging review")
    if build["relative_growth_explanation_missing"] or retained["explanation_missing"]:
        reasons.append("relative growth lacks the required simplification/packaging decision")
    return {"schema": "wtime-p046-portfolio-statistics-v1", "task": "P046",
            "status": "passed", "decision": decision, "reasons": reasons,
            "source_binding": {"portfolio_sha256": portfolio_sha,
                               "candidate_binary_sha256": matrix["candidate_binary_sha256"],
                               "candidate_sources_sha256": hashlib.sha256(
                                   _canonical(next(iter(matrix["reports"].values()))["candidate_sources"]).encode()).hexdigest(),
                               "p123_receipt_sha256": _sha(Path(portfolio["p123"]["path"]))
                                   if isinstance(portfolio.get("p123"), dict) and "path" in portfolio["p123"] else None},
            "matrix": {key: value for key, value in matrix.items() if key != "reports"},
            "p121": p121, "build": build, "retained_code": retained,
            "statistics_by_report_sha256": summary_refs,
            "cold_statistics_by_report_sha256": cold_refs,
            "dataset_statistics_computations": len(summaries),
            "portfolio_decision_computations": 1,
            "analysis": summaries, "policy_analysis": policy_analysis,
            "decision_note": "P046 gate computes evidence once; parent retains final promotion review."}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--portfolio", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--optional-build", type=Path, required=True)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--candidate-build-record", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args(argv)
    if not args.execute:
        print("inert: pass --execute")
        return 0
    try:
        result = evaluate_portfolio(args.portfolio, args.build, args.optional_build,
                                    args.contract, args.candidate_build_record)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            json.dump(result, stream, sort_keys=True, indent=2)
            stream.write("\n")
    except (OSError, json.JSONDecodeError, PortfolioError) as exc:
        print(f"P046 portfolio validation failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"status": result["status"], "decision": result["decision"],
                      "statistics_computations": result["statistics_computations"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
