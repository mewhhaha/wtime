#!/usr/bin/env python3
"""Create compact immutable checkpoints for already-completed raw measurements.

This producer copies only three named report files. It does not interpret their
statistics, create a task completion record, or modify the original reports.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SOURCE_COMMIT = "c73fc0d"
OUTPUTS = {
    "p067_eager_decoded": "P067-eager-decoded-measurements.json.gz",
    "p080_gc": "P080-gc-measurements.json.gz",
    "p080_simd": "P080-simd-measurements.json.gz",
}
INPUTS = {
    "p067_eager_decoded": ROOT / "target/verification/runs/P067/measure/attempt-0001/measure/eager-vs-decoded/report.json",
    "p080_gc": ROOT / "target/performance/P080-families-0004/gc-exceptions/report.json",
    "p080_simd": ROOT / "target/performance/P080-families-0004/simd/report.json",
}
MANIFEST = ROOT / "docs/benchmarks/performance/measurement-checkpoint.json"


def sha256_file(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        while True:
            block = stream.read(1024 * 1024)
            if not block:
                break
            digest.update(block)
            count += len(block)
    return digest.hexdigest(), count


def copy_gzip(source: Path, destination: Path) -> tuple[str, int, str, int]:
    source_hash = hashlib.sha256()
    raw_bytes = 0
    with source.open("rb") as src, destination.open("xb") as raw_out:
        # filename="" and mtime=0 make the stream deterministic.
        with gzip.GzipFile(fileobj=raw_out, mode="wb", compresslevel=9,
                           filename="", mtime=0) as compressed:
            while True:
                block = src.read(1024 * 1024)
                if not block:
                    break
                source_hash.update(block)
                raw_bytes += len(block)
                compressed.write(block)
    digest = source_hash.hexdigest()
    return digest, raw_bytes, sha256_file(destination)[0], destination.stat().st_size


def verify_gzip(source: Path, compressed_path: Path, expected_hash: str,
                expected_bytes: int) -> None:
    digest = hashlib.sha256()
    raw_bytes = 0
    with gzip.open(compressed_path, "rb") as stream:
        while True:
            block = stream.read(1024 * 1024)
            if not block:
                break
            digest.update(block)
            raw_bytes += len(block)
    if digest.hexdigest() != expected_hash or raw_bytes != expected_bytes:
        raise ValueError(f"gzip round-trip mismatch: {compressed_path}")
    source_hash, source_bytes = sha256_file(source)
    if source_hash != expected_hash or source_bytes != expected_bytes:
        raise ValueError(f"original report changed during copy: {source}")


def run(output_dir: Path) -> dict[str, object]:
    output_dir = output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if not output_dir.is_relative_to(ROOT.resolve()):
        raise ValueError("output directory must stay inside the repository")
    manifest_path = output_dir / MANIFEST.name
    expected_paths = [output_dir / name for name in OUTPUTS.values()]
    if manifest_path.exists() or any(path.exists() for path in expected_paths):
        raise FileExistsError("refusing to overwrite a checkpoint output")

    helper_path = Path(__file__).resolve()
    helper_hash, helper_bytes = sha256_file(helper_path)
    datasets: list[dict[str, object]] = []
    for key, source in INPUTS.items():
        source = source.resolve(strict=True)
        if not source.is_relative_to(ROOT.resolve()):
            raise ValueError(f"source escaped repository: {source}")
        destination = output_dir / OUTPUTS[key]
        raw_hash, raw_bytes, compressed_hash, compressed_bytes = copy_gzip(source, destination)
        verify_gzip(source, destination, raw_hash, raw_bytes)
        datasets.append({
            "dataset": key,
            "original_path": str(source),
            "original_sha256": raw_hash,
            "original_bytes": raw_bytes,
            "gzip_ref": {
                "path": destination.relative_to(ROOT).as_posix(),
                "sha256": compressed_hash,
                "bytes": compressed_bytes,
                "encoding": "gzip-json-identity",
            },
        })

    manifest = {
        "schema": "wtime-measurement-checkpoint-v1",
        "source_commit": SOURCE_COMMIT,
        "task_completion": False,
        "decision": "pending_final_gate",
        "claim": "These are exact completed raw measurement datasets, not task completion or supported speed claims.",
        "original_reports_modified": False,
        "helper": {
            "path": helper_path.relative_to(ROOT).as_posix(),
            "sha256": helper_hash,
            "bytes": helper_bytes,
        },
        "datasets": datasets,
    }
    payload = (json.dumps(manifest, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode()
    with manifest_path.open("xb") as stream:
        stream.write(payload)
    return {"status": "checkpointed", "manifest": str(manifest_path),
            "manifest_sha256": hashlib.sha256(payload).hexdigest(),
            "datasets": len(datasets), "helper_sha256": helper_hash}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    result = run(args.output_dir)
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
