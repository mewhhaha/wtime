"""Separate benchmark execution identity from analysis and verification tools.

Legacy comparison reports recorded every benchmark Python module.  Keep their
timing inputs strict while allowing reviewed analysis/build-only tooling to
change without making the guest observations stale.
"""
from __future__ import annotations

import hashlib
import re
from pathlib import Path
from typing import Any


_BENCH = "tools/wtime-bench/"

# These modules inspect, build, gate, or summarize retained evidence. They do
# not select workloads, encode fixtures, issue timer commands, or collect the
# paired observations used by a comparison.
_ANALYSIS_ONLY = {
    "analyze.py", "baseline.py", "bootstrap.py", "compact_production.py",
    "gates.py", "p042_check.py", "p042_coverage.py",
    "p042_production.py", "p043_coverage.py",
    "p043_workflow.py", "p045_corpus.py", "p046_gate.py",
    "p123_adapter.py", "profile.py", "profile_routes.py",
    "structural_check.py", "tier_check.py",
}

# These are new fast-path planning/execution tools. They are not dependencies
# of legacy reports; focused reports bind them below.
_KNOWN_FAST_PATH = {
    "cold_products.py", "execution_identity.py", "focused_compare.py",
    "selection_manifest.py", "dataset_cache.py", "candidate_gate.py", "semantic_receipt.py",
    "root_workloads.py", "root_compare.py", "thread_compare.py",
}

# Explicit legacy execution inputs. The non-Python portion of compare.sources
# is always retained, so runtime, Cargo, fixtures, registry and conformance
# inputs remain bound independently of this list.
_LEGACY_TIMING = {
    "checkpoint.py", "compare.py", "continuation_scaling.py", "gc_cold_adapter.py",
    "continuation_workloads.py", "gc_compare.py", "gc_layout_workloads.py",
    "gc_scaling.py", "gc_warm_workloads.py", "gc_workloads.py",
    "integration_compare.py", "integration_workflow.py",
    "integration_workloads.py", "memory_scaling.py", "memory_workloads.py",
    "p027_memory_workloads.py", "p043_fixtures.py", "p127_fuel_policy.py", "p128_scalar_scaling.py",
    "representative_compare.py",
    "representative_scaling.py", "representative_workloads.py",
    "scalar_p128_workloads.py", "scalar_workloads.py", "scaling.py", "simd_scaling.py",
    "simd_workloads.py", "thread_scaling.py", "thread_workloads.py",
    "workloads.py", "run.py",
}

_FOCUSED_TIMING = {
    "cold_products.py", "execution_identity.py", "focused_compare.py",
    "selection_manifest.py", "semantic_receipt.py", "root_workloads.py", "root_compare.py",
    "thread_compare.py",
}
_SHA = re.compile(r"[0-9a-f]{64}\Z")


def _classify(path: str, *, focused: bool) -> str:
    if not path.startswith(_BENCH) or not path.endswith(".py"):
        return "protected"
    name = path[len(_BENCH):]
    if "/" in name:
        # Tests are not part of compare.sources; any other nested Python
        # module is unexpected and must receive an explicit classification.
        raise ValueError(f"unknown benchmark Python identity: {path}")
    if name in _ANALYSIS_ONLY:
        return "analysis"
    if name in _LEGACY_TIMING:
        return "protected"
    if name in _KNOWN_FAST_PATH:
        if focused and name in _FOCUSED_TIMING:
            return "protected"
        return "tooling"
    raise ValueError(f"unclassified benchmark Python identity: {path}")


def _validate_map(source_map: Any, label: str) -> dict[str, str]:
    if not isinstance(source_map, dict) or not source_map:
        raise ValueError(f"{label} source map must be a nonempty object")
    result: dict[str, str] = {}
    for path, digest in source_map.items():
        if (not isinstance(path, str) or not path or not isinstance(digest, str)
                or not _SHA.fullmatch(digest)):
            raise ValueError(f"{label} source map has a malformed path or SHA-256")
        result[path] = digest
    return result


def project(source_map: dict[str, str], *, focused: bool = False) -> dict[str, str]:
    """Return only reviewed execution inputs, retaining every non-Python key."""
    if type(focused) is not bool:
        raise ValueError("focused must be boolean")
    source_map = _validate_map(source_map, "candidate")
    result = {}
    for path, digest in source_map.items():
        if path.startswith(_BENCH) and path.endswith(".py"):
            if _classify(path, focused=focused) == "protected":
                result[path] = digest
        else:
            result[path] = digest
    if focused and not _FOCUSED_TIMING <= {
            path[len(_BENCH):] for path in result if path.startswith(_BENCH)}:
        missing = sorted(_BENCH + name for name in _FOCUSED_TIMING
                         if _BENCH + name not in result)
        raise ValueError(f"focused execution identity is missing required tools: {missing}")
    return dict(sorted(result.items()))


def verify(recorded: dict[str, str], current: dict[str, str], *,
           focused: bool = False) -> dict[str, Any]:
    """Require exact projected membership and digests; report excluded changes."""
    recorded = _validate_map(recorded, "recorded")
    current = _validate_map(current, "current")
    recorded_projection = project(recorded, focused=focused)
    current_projection = project(current, focused=focused)
    if recorded_projection != current_projection:
        old, new = set(recorded_projection), set(current_projection)
        changed = sorted(path for path in old & new
                         if recorded_projection[path] != current_projection[path])
        missing, added = sorted(old - new), sorted(new - old)
        raise ValueError(
            "execution source identity is stale "
            f"(changed={changed[:5]}, missing={missing[:5]}, added={added[:5]})")
    excluded = {}
    for path in sorted(set(recorded) | set(current)):
        if path in recorded_projection or path in current_projection:
            continue
        excluded[path] = {
            "recorded_sha256": recorded.get(path),
            "current_sha256": current.get(path),
            "changed": recorded.get(path) != current.get(path),
        }
    return {"execution_inputs": current_projection,
            "execution_inputs_sha256": _canonical_sha(current_projection),
            "excluded_analysis_inputs": excluded,
            "focused": focused}


def verify_inputs(report: dict[str, Any], *, focused: bool = False) -> dict[str, Any]:
    """Validate a comparison report against the live source map without timing."""
    metadata = report.get("metadata") if isinstance(report, dict) else None
    if not isinstance(metadata, dict) or not isinstance(metadata.get("candidate_sources"), dict):
        raise ValueError("comparison report lacks metadata.candidate_sources")
    selection = report.get("selection")
    if not isinstance(selection, str) or not selection:
        raise ValueError("comparison report lacks a workload selection")
    from pathlib import Path
    import sys

    bench = Path(__file__).resolve().parent
    sys.path.insert(0, str(bench))
    import compare

    current = (compare.sources(include_memory=True, include_simd=True,
                               include_representative=True, include_integration=True)
               if focused else compare.sources(
                   include_memory=compare.selection_uses_oracle_module(selection, "memory_workloads.py"),
                   include_simd=compare.selection_uses_oracle_module(selection, "simd_workloads.py"),
                   include_representative=selection == "representative",
                   include_integration=selection == "integration"))
    return verify(metadata["candidate_sources"], current, focused=focused)


def _canonical_sha(value: dict[str, str]) -> str:
    import json
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    ensure_ascii=False).encode()).hexdigest()
