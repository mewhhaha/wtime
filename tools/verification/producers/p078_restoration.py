"""Verify that the P078 runtime candidate was restored to its archived source."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE = ROOT / "target/performance/P078-before"
ARCHIVE_SHA256 = "e82fe66aaf16fa742b3daa0f95242132dbb5c0ee45af795acd83f4d6ec7f9507"
COMPLETION = ROOT / "target/performance/P078/shared-finalization/attempt-0002/completion.json"
COMPLETION_SHA256 = "2f5b05b8644397d0089471bdc6583ad4abe655afdaad77a8fecfab9dd1c4cdf9"
EVIDENCE = ROOT / "target/performance/P078/shared-finalization/attempt-0002/evidence.json"
EVIDENCE_SHA256 = "7b6597ce8aac9259987f3a4baa93a04db789c70931b7ca7ca24909a47fb5d8f4"
RESTORE_OPERATION = ROOT / "target/recovery/P078-restoration-0001/result.json"
RESTORE_OPERATION_SHA256 = "312e032db285f59524a5a72622237a1e1a7e3dbfc7df4ec876bc076b2e9e3bbc"
RESTORED_PATHS = {
    "crates/wtime-runtime/src/shared_memory.rs",
    "crates/wtime-runtime/src/execution.rs",
    "crates/wtime-runtime/tests/performance/p027.rs",
}
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_INPUTS = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE-MIT", "LICENSE-APACHE"}


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ref(path: Path) -> dict:
    path = path.resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def check(production_path: Path, output_path: Path) -> dict:
    sys.path.insert(0, str(ROOT / "tools/wtime-bench"))
    import baseline

    manifest_path = ARCHIVE / "manifest.json"
    assert sha(manifest_path) == ARCHIVE_SHA256
    manifest = baseline.verify_archive(ARCHIVE)
    assert manifest.get("repository_commit") == "27ce42e95a774203f1a08a4d934f208a6b076ed5"

    completion = json.loads(COMPLETION.read_text(encoding="utf-8"))
    assert sha(COMPLETION) == COMPLETION_SHA256
    assert completion.get("task") == "P078"
    assert completion.get("status") == "done" and completion.get("outcome") == "inconclusive"
    evidence_ref = completion.get("evidence")
    assert isinstance(evidence_ref, list) and len(evidence_ref) == 1
    assert evidence_ref[0].get("path") == str(EVIDENCE)
    assert evidence_ref[0].get("sha256") == EVIDENCE_SHA256
    assert sha(EVIDENCE) == EVIDENCE_SHA256
    evidence = json.loads(EVIDENCE.read_text(encoding="utf-8"))
    assert evidence.get("status") == "passed" and evidence.get("outcome") == "inconclusive"
    assert evidence.get("passed") is True
    assert set(evidence.get("phases", {})) == {"freeze", "verify", "measure", "analyze"}

    restore = json.loads(RESTORE_OPERATION.read_text(encoding="utf-8"))
    assert sha(RESTORE_OPERATION) == RESTORE_OPERATION_SHA256
    assert restore.get("status") == "passed" and restore.get("runtime_change_retained") is False
    files = restore.get("files")
    assert isinstance(files, dict) and set(files) == RESTORED_PATHS

    # Verify every archived Rust/core source and root build input against both
    # the current checkout and its archive copy, while leaving benchmark
    # harness changes outside the restored runtime boundary.
    restored_sources = {}
    for row in manifest["files"].values():
        logical = row.get("source_path")
        if row.get("role") != "source" or not isinstance(logical, str):
            continue
        if not (logical.startswith("crates/") or logical in ROOT_INPUTS):
            continue
        archive_rel = row.get("archive_path")
        if archive_rel is None:
            # The archive's source mirror is keyed by logical source path.
            archive_rel = "source/" + logical
        archived = (ARCHIVE / archive_rel).resolve(strict=True)
        current = (ROOT / logical).resolve(strict=True)
        assert archived.is_relative_to(ARCHIVE.resolve())
        assert current.is_relative_to(ROOT.resolve())
        digest = row["sha256"]
        assert sha(archived) == digest, logical
        assert sha(current) == digest, logical
        restored_sources[logical] = digest
    assert restored_sources
    for logical in RESTORED_PATHS:
        assert logical in restored_sources
        record = files[logical]
        assert record.get("restored_sha256") == restored_sources[logical]
        assert isinstance(record.get("candidate_sha256"), str)
        assert isinstance(record.get("fresh_mtime_ns"), int) and record["fresh_mtime_ns"] > 0

    production_path = production_path.resolve(strict=True)
    production = json.loads(production_path.read_text(encoding="utf-8"))
    assert production.get("source_stable") is True and production.get("exit_code") == 0
    binary = Path(production["binary_path"]).resolve(strict=True)
    assert binary.is_relative_to(production_path.parent.parent.resolve())
    assert sha(binary) == production.get("binary_sha256")

    expected_core = {}
    for logical, digest in restored_sources.items():
        if any(logical.startswith(f"crates/{crate}/src/") for crate in CORE_CRATES):
            expected_core[logical] = digest
    assert expected_core
    assert production.get("core_source_map") == dict(sorted(expected_core.items()))

    result = {
        "schema": "wtime-p078-restoration-v1",
        "status": "passed",
        "restored_core_matches_predecessor": True,
        "runtime_change_retained": False,
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "restored_sources": dict(sorted(restored_sources.items())),
        "production_record": ref(production_path),
        "production_binary": ref(binary),
        "candidate_completion": ref(COMPLETION),
        "candidate_evidence": ref(EVIDENCE),
        "restore_operation": ref(RESTORE_OPERATION),
    }
    output_path = output_path.resolve()
    assert output_path.is_relative_to((ROOT / "target/verification").resolve())
    assert not output_path.exists()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--production", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({"status": check(args.production, args.output)["status"]}))
