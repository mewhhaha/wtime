"""Verify the rejected P073 candidate was restored before a fresh build."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE = ROOT / "target/performance/P073-before"
ARCHIVE_SHA256 = "f485386c860bb37fd77cc379880c009777902f48f7ef5101d795bdff6ee10d1f"
ARCHIVE_COMMIT = "5fd0439a5705e21a627f8d7a02ee7643cdb43fbc"
COMPLETION = ROOT / "target/performance/P073/shared-finalization/attempt-0003/completion.json"
COMPLETION_SHA256 = "6888f9a75e90e7daece1aad4f07078e64569468b36d619861033c66a13e6529f"
EVIDENCE = ROOT / "target/performance/P073/shared-finalization/attempt-0003/evidence.json"
EVIDENCE_SHA256 = "d279f9c24261fe9587b3a312a252bd83defe68df77f3fba386bc608c3f45c51f"
RESTORE_OPERATION = ROOT / "target/recovery/P073-restoration-0001/result.json"
RESTORE_OPERATION_SHA256 = "f22db6dc7fbf2fc652ff3a437e6f5bfd2e58a4c790e85a5af678ab8716259148"
SELECTION_FREEZE = ROOT / "target/performance/P073/selection-freeze.json"
RESTORED_PATHS = {
    "crates/wtime-runtime/src/lib.rs",
    "crates/wtime-runtime/src/linker.rs",
    "crates/wtime-runtime/src/execution.rs",
    "crates/wtime-runtime/src/reference.rs",
    "crates/wtime-runtime/src/execution/cursor.rs",
    "crates/wtime-runtime/src/execution/register/execute.rs",
    "crates/wtime-runtime/src/execution/compact/lower_control.rs",
    "crates/wtime-runtime/src/continuation.rs",
}
REMOVED_PATHS = {
    "crates/wtime-runtime/tests/performance/p073.rs",
}
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
    assert completion.get("task") == "P073"
    assert completion.get("status") == "done" and completion.get("outcome") == "rejected"
    evidence_refs = completion.get("evidence")
    assert isinstance(evidence_refs, list) and len(evidence_refs) == 1
    assert evidence_refs[0].get("path") == str(EVIDENCE)
    assert evidence_refs[0].get("sha256") == EVIDENCE_SHA256
    assert sha(EVIDENCE) == EVIDENCE_SHA256
    evidence = json.loads(EVIDENCE.read_text(encoding="utf-8"))
    assert evidence.get("task") == "P073"
    assert evidence.get("status") == "passed" and evidence.get("outcome") == "rejected"
    assert evidence.get("decision") == "rejected"
    assert evidence.get("candidate_promoted") is False and evidence.get("passed") is True
    assert set(evidence.get("phases", {})) == {"freeze", "verify", "measure", "analyze"}

    restore = json.loads(RESTORE_OPERATION.read_text(encoding="utf-8"))
    assert sha(RESTORE_OPERATION) == RESTORE_OPERATION_SHA256
    assert restore.get("status") == "passed" and restore.get("runtime_change_retained") is False
    files = restore.get("files")
    assert isinstance(files, dict) and set(files) == RESTORED_PATHS | REMOVED_PATHS

    # Check every archived crate input and root build input against both the
    # archive and current source tree, including the deleted P073 test input.
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

    for logical in RESTORED_PATHS:
        record = files[logical]
        current = (ROOT / logical).resolve(strict=True)
        assert logical in restored_sources
        assert record.get("restored_sha256") == restored_sources[logical]
        assert isinstance(record.get("candidate_sha256"), str)
        assert record["candidate_sha256"] != record["restored_sha256"]
        assert type(record.get("fresh_mtime_ns")) is int and record["fresh_mtime_ns"] > 0
        assert current.stat().st_mtime_ns == record["fresh_mtime_ns"]
    for logical in REMOVED_PATHS:
        record = files[logical]
        assert logical not in restored_sources and not (ROOT / logical).exists()
        assert record.get("removed") is True
        preserved = ROOT / "target/recovery/P073-restoration-0001/candidate" / logical
        assert record.get("candidate_sha256") == sha(preserved.resolve(strict=True))

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
    identities = production.get("source_identities")
    assert isinstance(identities, dict)
    for logical, digest in restored_sources.items():
        if logical in identities:
            assert identities[logical] == digest, logical
    for logical in PRODUCTION_ROOT_INPUTS:
        assert identities.get(logical) == restored_sources[logical], logical

    result = {
        "schema": "wtime-p073-restoration-v1",
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
