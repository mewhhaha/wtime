#!/usr/bin/env python3
"""Create the P027 registry proposal while preserving the current ordered prefix."""
from __future__ import annotations
import copy
import importlib.util
import json
from pathlib import Path
import sys

TOOL = Path(__file__).resolve().parents[1] / "tools" / "wtime-bench"
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import memory_workloads
import workloads


def merge_registry(source: Path, destination: Path) -> dict:
    original = workloads.load_registry(source)
    result = copy.deepcopy(original)
    old_rows = result["workloads"]
    additions = [row for row in memory_workloads.entries()
                 if row["family"] in {"memory_copy_distinct_seeded", "memory_fill_seeded"}]
    if len(old_rows) != 304 or len(additions) != 10:
        raise ValueError("P027 registry bridge requires the current 304-row prefix and 10 additions")
    old_ids = {row["id"] for row in old_rows}
    if old_ids & {row["id"] for row in additions}:
        raise ValueError("P027 addition duplicates an existing workload ID")
    if any(row["oracle"]["module"] != "memory_workloads.py" for row in additions):
        raise ValueError("P027 rows must route through the merged memory oracle module")
    result["workloads"] = copy.deepcopy(old_rows) + copy.deepcopy(additions)
    memory_ids = result["selections"]["memory"]["workloads"]
    memory_ids.extend(row["id"] for row in additions)
    if len(result["workloads"]) != 314 or len(memory_ids) != 35:
        raise AssertionError("P027 registry must contain 314 rows and 35 memory-selected IDs")
    workloads.validate_registry(result)
    destination.write_text(json.dumps(result, indent=0, sort_keys=True) + "\n", encoding="utf-8")
    return result


if __name__ == "__main__":
    base = TOOL / "workloads.json"
    output = base
    if len(sys.argv) == 3:
        base, output = Path(sys.argv[1]), Path(sys.argv[2])
    elif len(sys.argv) != 1:
        raise SystemExit("usage: merge_registry.py [BASE_REGISTRY OUTPUT_REGISTRY]")
    registry = merge_registry(base, output)
    print(f"merged {len(registry['workloads'])} workloads; memory selection has "
          f"{len(registry['selections']['memory']['workloads'])} IDs")
