"""Verify P077 rejection, one-file restoration, and a fresh source-bound build."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE = ROOT / "target/performance/P077-before"
ARCHIVE_SHA256 = "4e1df7c6e4a4f1dcd711ba707129cb513f7b247319d14143d17f5cfd7bd6ada5"
ARCHIVE_COMMIT = "be3558b1c315d509c5b9153a21dfd1007b96ec49"
COMPLETION = ROOT / "target/performance/P077/shared-finalization/attempt-0002/completion.json"
COMPLETION_SHA256 = "f6aca8dfad05d6fa6083f3877703ac03fc1375abbcfa27824ca99c6de426b173"
EVIDENCE = ROOT / "target/performance/P077/shared-finalization/attempt-0002/evidence.json"
EVIDENCE_SHA256 = "2eb4a89125609632ec81f37d3043a213e2bad745c74a0bd56c400a26148818f2"
RESTORE_OPERATION = ROOT / "target/recovery/P077-restoration-0001/result.json"
RESTORE_OPERATION_SHA256 = "25536501b0b5b4811601d144e398f634fdff9bb73c40891a24d01d9213eb73ea"
SELECTION_FREEZE = ROOT / "target/performance/P077/selection-freeze.json"
RESTORED_PATHS = {"crates/wtime-runtime/src/shared_memory.rs"}
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_INPUTS = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE-MIT", "LICENSE-APACHE"}
PRODUCTION_ROOT_INPUTS = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = path.resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def check(production_path: Path, output_path: Path) -> dict[str, Any]:
    sys.path.insert(0, str(ROOT / "tools/wtime-bench"))
    import baseline

    manifest_path = ARCHIVE / "manifest.json"
    assert sha(manifest_path) == ARCHIVE_SHA256
    manifest = baseline.verify_archive(ARCHIVE)
    assert manifest.get("repository_commit") == ARCHIVE_COMMIT
    assert manifest.get("mode") == "decoded"

    completion = json.loads(COMPLETION.read_text(encoding="utf-8"))
    assert sha(COMPLETION) == COMPLETION_SHA256
    assert completion.get("task") == "P077"
    assert completion.get("status") == "done" and completion.get("outcome") == "rejected"
    evidence_ref = completion.get("evidence")
    assert isinstance(evidence_ref, list) and len(evidence_ref) == 1
    assert evidence_ref[0].get("path") == str(EVIDENCE)
    assert evidence_ref[0].get("sha256") == EVIDENCE_SHA256
    assert sha(EVIDENCE) == EVIDENCE_SHA256
    evidence = json.loads(EVIDENCE.read_text(encoding="utf-8"))
    assert evidence.get("task") == "P077"
    assert evidence.get("status") == "passed" and evidence.get("outcome") == "rejected"
    assert evidence.get("decision") == "rejected" and evidence.get("candidate_promoted") is False
    assert evidence.get("passed") is True
    assert set(evidence.get("phases", {})) == {"freeze", "verify", "measure", "analyze"}

    restore = json.loads(RESTORE_OPERATION.read_text(encoding="utf-8"))
    assert sha(RESTORE_OPERATION) == RESTORE_OPERATION_SHA256
    assert restore.get("status") == "passed" and restore.get("runtime_change_retained") is False
    files = restore.get("files")
    assert isinstance(files, dict) and set(files) == RESTORED_PATHS

    # Check all archived core and root build inputs against both the archive
    # and the current tree. The candidate itself changed only shared_memory.rs.
    restored_sources: dict[str, str] = {}
    for row in manifest["files"].values():
        logical = row.get("source_path")
        if row.get("role") != "source" or not isinstance(logical, str):
            continue
        if not (logical.startswith("crates/") or logical in ROOT_INPUTS):
            continue
        archive_rel = row.get("archive_path") or ("source/" + logical)
        archived = (ARCHIVE / archive_rel).resolve(strict=True)
        current = (ROOT / logical).resolve(strict=True)
        assert archived.is_relative_to(ARCHIVE.resolve())
        assert current.is_relative_to(ROOT.resolve())
        digest = row["sha256"]
        assert sha(archived) == digest, logical
        assert sha(current) == digest, logical
        restored_sources[logical] = digest
    assert restored_sources

    assert set(files) == RESTORED_PATHS
    for logical in RESTORED_PATHS:
        assert logical in restored_sources
        record = files[logical]
        current = (ROOT / logical).resolve(strict=True)
        assert record.get("restored_sha256") == restored_sources[logical]
        assert isinstance(record.get("candidate_sha256"), str)
        assert record["candidate_sha256"] != record["restored_sha256"]
        assert type(record.get("fresh_mtime_ns")) is int and record["fresh_mtime_ns"] > 0
        assert current.stat().st_mtime_ns == record["fresh_mtime_ns"]

    selection_freeze = ref(SELECTION_FREEZE)
    production_path = production_path.resolve(strict=True)
    production = json.loads(production_path.read_text(encoding="utf-8"))
    assert production.get("source_stable") is True and production.get("exit_code") == 0
    binary = Path(production["binary_path"]).resolve(strict=True)
    assert binary.is_relative_to(production_path.parent.parent.resolve())
    assert sha(binary) == production.get("binary_sha256")

    expected_core = {
        logical: digest for logical, digest in restored_sources.items()
        if any(logical.startswith(f"crates/{crate}/src/") for crate in CORE_CRATES)
        and logical.endswith(".rs")
    }
    expected_runtime = {
        logical: digest for logical, digest in expected_core.items()
        if logical.startswith("crates/wtime-runtime/src/")
    }
    assert expected_core and expected_runtime
    assert production.get("core_source_map") == dict(sorted(expected_core.items()))
    assert production.get("runtime_source_map") == dict(sorted(expected_runtime.items()))
    production_identities = production.get("source_identities")
    assert isinstance(production_identities, dict)
    for logical, digest in restored_sources.items():
        if logical in production_identities:
            assert production_identities[logical] == digest, logical
    for logical in PRODUCTION_ROOT_INPUTS:
        assert production_identities.get(logical) == restored_sources[logical], logical

    result = {
        "schema": "wtime-p077-restoration-v1",
        "status": "passed",
        "candidate_outcome": "rejected",
        "restored_core_matches_predecessor": True,
        "runtime_change_retained": False,
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "selection_freeze": selection_freeze,
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
