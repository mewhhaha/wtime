"""Content-addressed receipts for immutable, already validated datasets.

The cache deliberately keeps datasets in place.  A receipt binds the caller's
selection identity to the report and to every filesystem member below its
directory, so a hit is only usable while the original dataset remains intact.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
from typing import Any, Callable


_SCHEMA = "wtime-immutable-dataset-cache-v1"


class DatasetCacheError(RuntimeError):
    """A cache receipt or the dataset it describes is invalid."""


class CacheConflictError(DatasetCacheError):
    """A receipt already exists for this identity but describes other data."""


def _canonical_identity(identity: dict[str, Any]) -> tuple[bytes, str]:
    if not isinstance(identity, dict) or not identity:
        raise ValueError("identity must be a nonempty JSON object")
    try:
        encoded = json.dumps(identity, sort_keys=True, separators=(",", ":"),
                             ensure_ascii=False, allow_nan=False).encode("utf-8")
    except (TypeError, ValueError) as exc:
        raise ValueError("identity must contain only finite JSON values") from exc
    return encoded, hashlib.sha256(encoded).hexdigest()


def _read_report(report_path: Path) -> dict[str, Any]:
    try:
        with report_path.open("r", encoding="utf-8") as stream:
            report = json.load(stream)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise DatasetCacheError(f"cannot read report JSON: {report_path}") from exc
    if not isinstance(report, dict):
        raise DatasetCacheError("report must be a JSON object")
    if report.get("passed") is not True or type(report.get("passed")) is not bool:
        raise DatasetCacheError("report passed must be true")
    if report.get("source_stable") is not True or type(report.get("source_stable")) is not bool:
        raise DatasetCacheError("report source_stable must be true")
    runs = report.get("runs")
    if not isinstance(runs, list) or len(runs) != 2:
        raise DatasetCacheError("report must contain exactly two runs")
    pair_total = 0
    for index, run in enumerate(runs):
        if not isinstance(run, dict):
            raise DatasetCacheError(f"run {index + 1} must be an object")
        pairs = run.get("pairs")
        if not isinstance(pairs, list) or not pairs:
            raise DatasetCacheError(f"run {index + 1} must contain nonempty pairs")
        if any(not isinstance(pair, dict) or not pair for pair in pairs):
            raise DatasetCacheError(f"run {index + 1} contains an empty or invalid pair")
        pair_total += len(pairs)
    return {"runs": 2, "pairs": pair_total}


def _within(child: Path, parent: Path) -> bool:
    try:
        child.relative_to(parent)
        return True
    except ValueError:
        return False


def _inventory(report_path: Path, cache_root: Path) -> tuple[Path, dict[str, dict[str, Any]]]:
    """Hash all regular files and record all directory members under report.parent."""
    try:
        report = report_path.resolve(strict=True)
        root = report.parent.resolve(strict=True)
        cache = cache_root.resolve(strict=False)
    except OSError as exc:
        raise DatasetCacheError("report or dataset directory is unavailable") from exc
    if not report.is_file():
        raise DatasetCacheError("report path is not a regular file")
    if _within(cache, root):
        raise DatasetCacheError("cache root must be outside the dataset directory")

    members: dict[str, dict[str, Any]] = {}
    for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
        current_path = Path(current)
        for name in list(dirs):
            item = current_path / name
            if item.is_symlink():
                resolved = item.resolve(strict=False)
                if not _within(resolved, root):
                    raise DatasetCacheError(f"directory symlink escapes dataset: {item}")
                raise DatasetCacheError(f"directory symlinks are not supported: {item}")
            rel = item.relative_to(root).as_posix()
            members[rel] = {"kind": "directory"}
        for name in files:
            item = current_path / name
            if item.is_symlink():
                resolved = item.resolve(strict=False)
                if not _within(resolved, root):
                    raise DatasetCacheError(f"file symlink escapes dataset: {item}")
                raise DatasetCacheError(f"file symlinks are not supported: {item}")
            try:
                mode = item.lstat().st_mode
                if not stat.S_ISREG(mode):
                    raise DatasetCacheError(f"dataset member is not a regular file: {item}")
                digest = hashlib.sha256()
                size = 0
                with item.open("rb") as stream:
                    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                        digest.update(chunk)
                        size += len(chunk)
            except OSError as exc:
                raise DatasetCacheError(f"cannot hash dataset file: {item}") from exc
            rel = item.relative_to(root).as_posix()
            members[rel] = {"kind": "file", "sha256": digest.hexdigest(), "bytes": size}
    return report, dict(sorted(members.items()))


def _require_validator(validator: Callable[[Path], Any]) -> None:
    if not callable(validator):
        raise TypeError("validator must be callable and must raise for invalid reports")


def _validate_stable(report_path: Path, cache_root: Path,
                     expected: dict[str, dict[str, Any]] | None = None) -> tuple[Path, dict[str, dict[str, Any]], dict[str, int]]:
    stats = _read_report(report_path)
    resolved, inventory = _inventory(report_path, cache_root)
    if expected is not None and inventory != expected:
        raise DatasetCacheError("dataset membership or file content changed")
    return resolved, inventory, stats


def _receipt_path(cache_root: Path, identity_digest: str) -> Path:
    root = cache_root.resolve(strict=False)
    root.mkdir(parents=True, exist_ok=True)
    return root / f"{identity_digest}.json"


def _write_exclusive(path: Path, payload: bytes) -> None:
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    temp = Path(temp_name)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(temp, path)
        except FileExistsError:
            raise
    finally:
        try:
            temp.unlink()
        except FileNotFoundError:
            pass


def admit(report_path: str | os.PathLike[str], cache_root: str | os.PathLike[str],
          identity: dict[str, Any], *, validator: Callable[[Path], Any]) -> Path:
    """Validate and record an immutable report directory, returning its receipt."""
    _require_validator(validator)
    identity_bytes, identity_digest = _canonical_identity(identity)
    source = Path(report_path).resolve(strict=True)
    cache = Path(cache_root).resolve(strict=False)
    before_report, before, stats = _validate_stable(source, cache)
    validator(before_report)
    after_report, after, after_stats = _validate_stable(source, cache)
    if before_report != after_report or before != after or stats != after_stats:
        raise DatasetCacheError("dataset changed during validation")
    payload_object = {
        "schema": _SCHEMA,
        "identity_sha256": identity_digest,
        "identity": json.loads(identity_bytes.decode("utf-8")),
        "report_path": str(after_report),
        "dataset_root": str(after_report.parent),
        "runs": after_stats["runs"],
        "pairs": after_stats["pairs"],
        "members": after,
    }
    payload = (json.dumps(payload_object, sort_keys=True, separators=(",", ":"),
                          ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")
    receipt = _receipt_path(cache, identity_digest)
    try:
        _write_exclusive(receipt, payload)
    except FileExistsError:
        if receipt.is_symlink():
            raise DatasetCacheError("existing cache receipt must not be a symlink")
        try:
            existing = json.loads(receipt.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            raise DatasetCacheError("existing cache receipt is corrupt") from exc
        if existing != payload_object:
            raise CacheConflictError("identity already has a different immutable dataset receipt")
    return receipt


def lookup(cache_root: str | os.PathLike[str], identity: dict[str, Any], *,
           validator: Callable[[Path], Any]) -> Path | None:
    """Return a fully revalidated original report path, or None if no receipt exists."""
    _require_validator(validator)
    identity_bytes, identity_digest = _canonical_identity(identity)
    del identity_bytes
    receipt = Path(cache_root).resolve(strict=False) / f"{identity_digest}.json"
    if not os.path.lexists(receipt):
        return None
    if receipt.is_symlink():
        raise DatasetCacheError("cache receipt must not be a symlink")
    try:
        record = json.loads(receipt.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise DatasetCacheError("cache receipt is corrupt") from exc
    if not isinstance(record, dict) or record.get("schema") != _SCHEMA:
        raise DatasetCacheError("cache receipt has an unsupported schema")
    expected_fields = {"schema", "identity_sha256", "identity", "report_path",
                       "dataset_root", "runs", "pairs", "members"}
    if set(record) != expected_fields:
        raise DatasetCacheError("cache receipt has unexpected or missing fields")
    if record.get("identity_sha256") != identity_digest:
        raise DatasetCacheError("cache receipt identity digest is invalid")
    try:
        recorded_digest = _canonical_identity(record.get("identity"))[1]
    except (TypeError, ValueError) as exc:
        raise DatasetCacheError("cache receipt identity is invalid") from exc
    if recorded_digest != identity_digest or record.get("identity") != identity:
        raise DatasetCacheError("cache receipt identity does not match lookup")
    if not isinstance(record.get("report_path"), str) or not isinstance(record.get("dataset_root"), str):
        raise DatasetCacheError("cache receipt paths are invalid")
    report = Path(record["report_path"])
    if not report.is_absolute() or str(report) != record.get("report_path"):
        raise DatasetCacheError("cache receipt report path is invalid")
    root = Path(record["dataset_root"])
    if not root.is_absolute() or str(root) != record["dataset_root"] or report.parent != root:
        raise DatasetCacheError("cache receipt dataset root is invalid")
    members = record.get("members")
    if not isinstance(members, dict) or not members:
        raise DatasetCacheError("cache receipt membership is invalid")
    resolved, current, stats = _validate_stable(report, receipt.parent, members)
    if resolved != report or stats != {"runs": record.get("runs"), "pairs": record.get("pairs")}:
        raise DatasetCacheError("cache receipt report metadata is invalid")
    validator(resolved)
    _, after, final_stats = _validate_stable(report, receipt.parent, members)
    if after != current or final_stats != stats:
        raise DatasetCacheError("dataset changed during cache validation")
    return resolved


__all__ = ["DatasetCacheError", "CacheConflictError", "admit", "lookup"]
