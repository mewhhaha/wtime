"""Conservative planning for reusable cold preparation benchmark products.

This module plans aliases only for explicitly identical ``prepare`` products.
For preparation, ``setup`` identifies the canonical preflight chosen for that
product. It does not execute benchmarks or infer equivalence from workload
names. First-call, warm-call, load-and-call, and instantiate workloads retain
their workload identity and never alias.
"""
from __future__ import annotations

import hashlib
import json
import re
from typing import Any


SCHEMA = "wtime-cold-products-v1"
UNCERTAINTY = "aliases share raw pairs and uncertainty; not independent samples"
_SHA256 = re.compile(r"^[0-9a-fA-F]{64}$")
_PRODUCT_KEYS = ("wasm_sha256", "profile", "runtime_settings", "timer", "setup")


def _canonical_json(value: Any) -> str:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"),
                          ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise ValueError("product fields must contain canonical JSON values") from exc


def _nonempty_mapping(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict) or not value:
        raise ValueError(f"{field} must be a nonempty object")
    # Validate nested keys and values now, rather than failing during hashing.
    _canonical_json(value)
    return value


def _normalize_products(products: Any) -> list[dict[str, Any]]:
    if not isinstance(products, list) or not products:
        raise ValueError("products must be a nonempty list")
    normalized: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for index, row in enumerate(products):
        if not isinstance(row, dict):
            raise ValueError(f"product {index} must be an object")
        expected = {"workload", "stage", *_PRODUCT_KEYS}
        if set(row) != expected:
            raise ValueError(f"product {index} must have exactly {sorted(expected)}")
        workload, stage, profile = row["workload"], row["stage"], row["profile"]
        if not isinstance(workload, str) or not workload.strip():
            raise ValueError(f"product {index} workload must be a nonempty string")
        if not isinstance(stage, str) or not stage.strip():
            raise ValueError(f"product {index} stage must be a nonempty string")
        if not isinstance(profile, str) or not profile.strip():
            raise ValueError(f"product {index} profile must be a nonempty string")
        digest = row["wasm_sha256"]
        if not isinstance(digest, str) or not _SHA256.fullmatch(digest):
            raise ValueError(f"product {index} wasm_sha256 must be 64 hexadecimal characters")
        key = (workload, stage)
        if key in seen:
            raise ValueError(f"duplicate workload/stage product: {workload!r}/{stage!r}")
        seen.add(key)
        normalized.append({
            "workload": workload,
            "stage": stage,
            "wasm_sha256": digest.lower(),
            "profile": profile,
            "runtime_settings": _nonempty_mapping(row["runtime_settings"], "runtime_settings"),
            "timer": _nonempty_mapping(row["timer"], "timer"),
            "setup": _nonempty_mapping(row["setup"], "setup"),
        })
    return normalized


def plan(products: list[dict[str, Any]]) -> dict[str, Any]:
    """Return a deterministic plan, aliasing only equal ``prepare`` products."""
    rows = _normalize_products(products)
    buckets: dict[str, dict[str, Any]] = {}
    for row in rows:
        product: dict[str, Any] = {key: row[key] for key in _PRODUCT_KEYS}
        if row["stage"] != "prepare":
            # Input- and setup-sensitive stages retain workload identity.
            product["workload"] = row["workload"]
        identity = {"stage": row["stage"], "product": product}
        product_sha256 = hashlib.sha256(_canonical_json(identity).encode("utf-8")).hexdigest()
        group = buckets.get(product_sha256)
        if group is None:
            group = {
                "sample_workload": row["workload"],
                "stage": row["stage"],
                "members": [],
                "product": product,
                "product_sha256": product_sha256,
            }
            buckets[product_sha256] = group
        group["members"].append(row["workload"])

    groups = sorted(buckets.values(), key=lambda group: (group["stage"], group["product_sha256"]))
    for group in groups:
        group["members"].sort()
        group["sample_workload"] = group["members"][0]
    logical_groups = len(rows)
    unique_groups = len(groups)
    return {
        "schema": SCHEMA,
        "groups": groups,
        "logical_groups": logical_groups,
        "unique_groups": unique_groups,
        "aliases": logical_groups - unique_groups,
        "uncertainty": UNCERTAINTY,
    }


def verify(plan_record: Any, products: list[dict[str, Any]]) -> dict[str, Any]:
    """Recompute and compare the complete plan to reject altered aliases."""
    expected = plan(products)
    try:
        matches = _canonical_json(plan_record) == _canonical_json(expected)
    except ValueError:
        matches = False
    if not matches:
        raise ValueError("cold product plan differs from recomputed products")
    return expected


def summarize(plan_record: Any, raw_counts: dict[str, int]) -> dict[str, Any]:
    """Count physical pairs once and logical references with alias weighting.

    ``raw_counts`` uses only canonical ``sample_workload + "\\0" + stage``
    keys. Each positive count is one group's physical pair count; logical
    references weight that count by the number of aliased workloads.
    """
    if not isinstance(plan_record, dict) or plan_record.get("schema") != SCHEMA:
        raise ValueError("invalid cold product plan")
    groups = plan_record.get("groups")
    if not isinstance(groups, list):
        raise ValueError("invalid cold product groups")
    expected_keys: set[str] = set()
    for group in groups:
        if (not isinstance(group, dict) or not isinstance(group.get("members"), list)
                or not group["members"] or not isinstance(group.get("stage"), str)
                or not isinstance(group.get("sample_workload"), str)):
            raise ValueError("invalid cold product group")
        members = group["members"]
        if any(not isinstance(member, str) or not member for member in members):
            raise ValueError("invalid cold product group members")
        if len(set(members)) != len(members) or group["sample_workload"] != min(members):
            raise ValueError("invalid canonical sample workload")
        key = f"{group['sample_workload']}\0{group['stage']}"
        if key in expected_keys:
            raise ValueError("duplicate canonical workload/stage in cold product plan")
        expected_keys.add(key)
    if not isinstance(raw_counts, dict) or set(raw_counts) != expected_keys:
        raise ValueError("raw_counts keys must exactly match planned workload/stage products")

    unique_pair_count = 0
    logical_pair_references = 0
    for group in groups:
        members = group["members"]
        stage = group["stage"]
        count = raw_counts[f"{group['sample_workload']}\0{stage}"]
        if not isinstance(count, int) or isinstance(count, bool) or count <= 0:
            raise ValueError("raw pair counts must be positive integers")
        unique_pair_count += count
        logical_pair_references += count * len(members)
    return {
        "unique_pair_count": unique_pair_count,
        "logical_pair_references": logical_pair_references,
        "extra_samples": 0,
        "sampling_note": "aliases reuse the planned raw pairs; no extra samples are added",
        "uncertainty": UNCERTAINTY,
    }
