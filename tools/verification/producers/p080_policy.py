"""Small typed policy checks shared by the P080 specialist gate tests."""
from __future__ import annotations

import math
import re
from typing import Any


COMPARATOR_FAMILIES = {"gc-exceptions", "simd", "threads", "continuations"}
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")


def validate_comparators(rows: Any) -> list[dict[str, Any]]:
    """Validate one typed comparator result per specialist family.

    Unsupported and unrun external comparators carry no ratio. A ratio is only
    meaningful when a comparison was actually performed and its evidence refs
    are present.
    """
    if type(rows) is not list or len(rows) != len(COMPARATOR_FAMILIES):
        raise ValueError("comparator summary must contain exactly four family rows")
    seen: set[str] = set()
    normalized: list[dict[str, Any]] = []
    for row in rows:
        if type(row) is not dict:
            raise ValueError("comparator row must be an object")
        family = row.get("family")
        if type(family) is not str or family not in COMPARATOR_FAMILIES or family in seen:
            raise ValueError("comparator family is unknown or duplicated")
        seen.add(family)
        if row.get("engine") != "wasmtime":
            raise ValueError("specialist comparator engine must be Wasmtime")
        status = row.get("status")
        reason = row.get("reason")
        if type(reason) is not str or not reason.strip():
            raise ValueError("comparator reason must be nonempty")
        if status in ("unsupported", "unrun"):
            if set(row) != {"family", "engine", "status", "reason", "ratio"}:
                raise ValueError("unavailable comparator has unexpected comparison fields")
            if row["ratio"] is not None:
                raise ValueError("unavailable comparator cannot carry a ratio")
        elif status == "compared":
            if set(row) != {"family", "engine", "status", "reason", "ratio", "artifact_refs"}:
                raise ValueError("compared comparator fields differ")
            ratio = row["ratio"]
            if (type(ratio) not in (int, float) or not math.isfinite(ratio) or ratio <= 0):
                raise ValueError("compared ratio must be a positive finite number")
            refs = row["artifact_refs"]
            if type(refs) is not list or not refs:
                raise ValueError("compared comparator requires evidence references")
            for ref in refs:
                if (type(ref) is not dict or type(ref.get("path")) is not str or not ref["path"]
                        or type(ref.get("sha256")) is not str or not _SHA256.fullmatch(ref["sha256"])):
                    raise ValueError("comparator evidence reference is malformed")
        else:
            raise ValueError("comparator status must be unsupported, unrun, or compared")
        normalized.append(row)
    if seen != COMPARATOR_FAMILIES:
        raise ValueError("comparator family set is incomplete")
    return normalized
