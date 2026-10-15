"""Retain shared-bulk full-memory proofs as content-addressed raw images."""
from __future__ import annotations

import hashlib
import json
import os
import copy
from pathlib import Path
import re
from typing import Any


_SHA = re.compile(r"[0-9a-f]{64}\Z")


class SharedBulkComparisonError(ValueError):
    pass


def _need(ok: bool, message: str) -> None:
    if not ok:
        raise SharedBulkComparisonError(message)


def _digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _inside(root: Path, path: Path) -> Path:
    resolved = path.resolve(strict=False)
    _need(resolved.is_relative_to(root), "shared-bulk image path escapes comparison root")
    return resolved


def _canonical(value: Any) -> bytes:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"),
                          allow_nan=False).encode("utf-8")
    except (TypeError, ValueError) as exc:
        raise SharedBulkComparisonError("stored shared-bulk view is not canonical JSON") from exc


def _refs_for(wire: dict[str, Any], memory_proof: dict[str, Any]) -> dict[str, dict[str, Any]]:
    images = wire.get("backings_hex")
    proof_hashes = memory_proof.get("full_backing_sha256")
    _need(isinstance(images, dict) and images and isinstance(proof_hashes, dict)
          and set(images) == set(proof_hashes),
          "validated shared-bulk observation lacks matching full-image proof")
    _need(type(memory_proof.get("full_memory_bytes_checked")) is int
          and type(memory_proof.get("full_memory_pages_checked")) is int,
          "validated shared-bulk memory accounting is malformed")
    refs: dict[str, dict[str, Any]] = {}
    total_bytes = 0
    for name, encoded in images.items():
        _need(isinstance(name, str) and name, "shared-bulk backing name is malformed")
        _need(type(encoded) is str and len(encoded) % 2 == 0
              and all(char in "0123456789abcdef" for char in encoded),
              f"shared-bulk backing {name} is not canonical lowercase full-memory hex")
        raw = bytes.fromhex(encoded)
        digest = _digest(raw)
        _need(len(raw) > 0 and len(raw) % 65_536 == 0,
              f"shared-bulk backing {name} is not a full Wasm-memory image")
        _need(proof_hashes.get(name) == digest,
              f"shared-bulk backing {name} differs from its validated digest")
        refs[name] = {"path": f"memory-images/{digest}.bin",
                      "raw_sha256": digest, "raw_bytes": len(raw)}
        total_bytes += len(raw)
    _need(memory_proof["full_memory_bytes_checked"] == total_bytes
          and memory_proof["full_memory_pages_checked"] == total_bytes // 65_536,
          "shared-bulk full-memory accounting does not match retained images")
    return refs


def _write_once(root: Path, refs: dict[str, dict[str, Any]], wire: dict[str, Any]) -> None:
    image_dir = _inside(root, root / "memory-images")
    image_dir.mkdir(parents=True, exist_ok=True)
    _need(image_dir.resolve(strict=True).is_relative_to(root),
          "shared-bulk memory image directory escapes comparison root")
    for name, ref in refs.items():
        encoded = wire["backings_hex"][name]
        image = bytes.fromhex(encoded)
        digest = ref["raw_sha256"]
        artifact = _inside(root, root / ref["path"])
        _need(artifact.parent == image_dir, "shared-bulk image artifact has an invalid parent")
        if artifact.exists():
            _need(not artifact.is_symlink() and artifact.is_file(),
                  "existing shared-bulk image artifact is not a regular file")
            existing = artifact.read_bytes()
            _need(existing == image and _digest(existing) == digest,
                  "existing shared-bulk image artifact differs from its content identity")
            continue
        try:
            with artifact.open("xb") as stream:
                stream.write(image)
                stream.flush()
                os.fsync(stream.fileno())
        except FileExistsError:
            # A concurrent creator is reusable only if it wrote the exact image.
            _need(not artifact.is_symlink() and artifact.is_file(),
                  "racing shared-bulk image artifact is not a regular file")
            existing = artifact.read_bytes()
            _need(existing == image and _digest(existing) == digest,
                  "racing shared-bulk image artifact differs from its content identity")
        except OSError as exc:
            raise SharedBulkComparisonError(f"cannot retain shared-bulk image: {exc}") from exc


def _stored_view(validated: dict[str, Any], refs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    raw = dict(validated["raw_wire_observation"])
    raw.pop("backings_hex", None)
    raw["backings_image_refs"] = copy.deepcopy(refs)
    stored = dict(validated)
    stored["raw_wire_observation"] = raw
    stored["memory_image_refs"] = copy.deepcopy(refs)
    return stored


def retain_memory_images(validated: dict[str, Any], output_root: str | Path) -> dict[str, Any]:
    """Project a validated raw observation to compact references after preserving its images."""
    _need(isinstance(validated, dict), "validated shared-bulk observation must be an object")
    wire = validated.get("raw_wire_observation")
    proof = validated.get("memory_proof")
    _need(isinstance(wire, dict) and isinstance(proof, dict),
          "validated shared-bulk observation lacks raw wire or memory proof")
    root = Path(output_root).resolve(strict=True)
    _need(root.is_dir(), "shared-bulk comparison root must be an existing directory")
    refs = _refs_for(wire, proof)
    _write_once(root, refs, wire)
    return _stored_view(validated, refs)


def revalidate_observation(entry: dict[str, Any], stored: dict[str, Any], output_root: str | Path,
                           *, expected_batch: int | None = None,
                           expected_thread_cpus: list[int] | None = None) -> dict[str, Any]:
    """Rehydrate retained bytes, rerun the strict independent oracle, and compare the full view."""
    _need(isinstance(stored, dict), "stored shared-bulk observation must be an object")
    root = Path(output_root).resolve(strict=True)
    _need(root.is_dir(), "shared-bulk comparison root must be an existing directory")
    raw_stored = stored.get("raw_wire_observation")
    refs = stored.get("memory_image_refs")
    _need(isinstance(raw_stored, dict) and isinstance(refs, dict) and refs,
          "stored shared-bulk observation lacks memory image references")
    _need("backings_hex" not in raw_stored and raw_stored.get("backings_image_refs") == refs,
          "stored shared-bulk wire image references are inconsistent")
    raw = dict(raw_stored)
    raw.pop("backings_image_refs")
    raw["backings_hex"] = {}
    for name, ref in refs.items():
        _need(isinstance(name, str) and isinstance(ref, dict)
              and set(ref) == {"path", "raw_sha256", "raw_bytes"},
              "shared-bulk image reference fields are malformed")
        digest, size, relative = ref["raw_sha256"], ref["raw_bytes"], ref["path"]
        _need(type(digest) is str and _SHA.fullmatch(digest)
              and type(size) is int and size > 0
              and relative == f"memory-images/{digest}.bin",
              "shared-bulk image reference is not content addressed")
        artifact = _inside(root, root / relative)
        _need(artifact.is_file() and not artifact.is_symlink(),
              "referenced shared-bulk image is missing or not a regular file")
        image = artifact.read_bytes()
        _need(len(image) == size and _digest(image) == digest,
              "referenced shared-bulk image bytes or digest differ")
        raw["backings_hex"][name] = image.hex()

    import shared_bulk_workloads as model
    verified = model.validate_observation(entry, raw, expected_batch=expected_batch,
                                           expected_thread_cpus=expected_thread_cpus)
    projected = _stored_view(verified, refs)
    _need(_canonical(projected) == _canonical(stored),
          "stored shared-bulk normalized view differs from revalidated raw image bytes")
    return projected
