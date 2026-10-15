#!/usr/bin/env python3
"""Checkpoint four newly completed P067/P080 timing reports without rerunning work."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "tools/verification/producers"))
from measurement_checkpoint import copy_gzip, sha256_file, verify_gzip  # noqa: E402

SOURCE_COMMIT = "2df0267"
DOCS = ROOT / "docs/benchmarks/performance"
MANIFEST = DOCS / "measurement-checkpoint-0002.json"
DATASETS = {
    "p067_eager_compact": (
        ROOT / "target/verification/runs/P067/measure/attempt-0001/measure/eager-vs-compact/report.json",
        "P067-eager-compact-measurements.json.gz",
    ),
    "p067_adaptive_decoded": (
        ROOT / "target/verification/runs/P067/measure/attempt-0001/measure/adaptive-vs-decoded/report.json",
        "P067-adaptive-decoded-measurements.json.gz",
    ),
    "p080_threads": (
        ROOT / "target/verification/runs/P080/measure/attempt-0002/comparisons/threads/report.json",
        "P080-threads-measurements.json.gz",
    ),
    "p080_continuations": (
        ROOT / "target/verification/runs/P080/measure/attempt-0002/comparisons/continuations/report.json",
        "P080-continuations-measurements.json.gz",
    ),
}


def sha(path: Path) -> str:
    return sha256_file(path)[0]


def run() -> dict[str, Any]:
    docs = DOCS.resolve(strict=True)
    if not docs.is_relative_to(ROOT.resolve()):
        raise ValueError("documentation output directory escaped repository")
    if MANIFEST.exists():
        raise FileExistsError("refusing to overwrite follow-up manifest")
    if any((docs / filename).exists() for _, filename in DATASETS.values()):
        raise FileExistsError("refusing to overwrite follow-up gzip output")

    producer = Path(__file__).resolve()
    helper = ROOT / "tools/verification/producers/measurement_checkpoint.py"
    entries: list[dict[str, Any]] = []
    for name, (source, filename) in DATASETS.items():
        source = source.resolve(strict=True)
        if not source.is_relative_to(ROOT.resolve()):
            raise ValueError("measurement source escaped repository: " + str(source))
        target = docs / filename
        raw_sha, raw_bytes, stored_sha, stored_bytes = copy_gzip(source, target)
        verify_gzip(source, target, raw_sha, raw_bytes)
        entries.append({
            "dataset": name,
            "original_path": str(source),
            "original_sha256": raw_sha,
            "original_bytes": raw_bytes,
            "gzip_ref": {
                "path": target.relative_to(ROOT).as_posix(),
                "sha256": stored_sha,
                "bytes": stored_bytes,
                "encoding": "gzip-json-identity",
            },
        })

    manifest = {
        "schema": "wtime-measurement-checkpoint-v1",
        "checkpoint": "0002",
        "source_commit": SOURCE_COMMIT,
        "task_completion": False,
        "decision": "pending_final_gate",
        "claim": "Exact completed raw measurement datasets only; this checkpoint is not task completion or a supported speed claim.",
        "original_reports_modified": False,
        "producer": {"path": producer.relative_to(ROOT).as_posix(),
                     "sha256": sha(producer), "bytes": producer.stat().st_size},
        "reused_helper": {"path": helper.relative_to(ROOT).as_posix(),
                          "sha256": sha(helper), "bytes": helper.stat().st_size},
        "datasets": entries,
    }
    payload = (json.dumps(manifest, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    with MANIFEST.open("xb") as stream:
        stream.write(payload)
    return {"status": "checkpointed", "manifest": str(MANIFEST),
            "manifest_sha256": hashlib.sha256(payload).hexdigest(),
            "datasets": len(entries), "producer_sha256": sha(producer),
            "reused_helper_sha256": sha(helper)}


if __name__ == "__main__":
    print(json.dumps(run(), sort_keys=True))
