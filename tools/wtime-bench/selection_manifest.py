"""Validation and freezing for predeclared fast verification selections."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
from typing import Any

HERE = Path(__file__).resolve().parent
DEFAULT_REGISTRY = HERE / "workloads.json"
SCHEMA = "wtime-candidate-selection-v1"
FAMILIES = {"scalar", "memory", "simd", "representative", "original-mvp", "gc-exceptions", "gc-root-snapshots", "threads", "continuations", "shared-bulk"}
ROW_FIELDS = {"family", "id", "role", "scale", "stages", "row_sha256"}
MANIFEST_FIELDS = {"schema", "task", "lane", "rationale", "registry_sha256", "rows", "semantic_cases"}
OPTIONAL_MANIFEST_FIELDS = {"fixture_models", "execution_settings"}
_TASK = re.compile(r"P\d{3}\Z")
_SHA = re.compile(r"[0-9a-f]{64}\Z")
_CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _canonical_sha(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return _sha(encoded)


def _read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read JSON {path}: {exc}") from exc


def _registry(registry_path: str | Path) -> tuple[dict[str, Any], str, dict[str, dict[str, Any]]]:
    path = Path(registry_path).resolve()
    data = _read_json(path)
    if not isinstance(data, dict) or not isinstance(data.get("selections"), dict):
        raise ValueError("registry must contain a selections object")
    rows = data.get("workloads")
    if not isinstance(rows, list) or any(not isinstance(row, dict) or not isinstance(row.get("id"), str) for row in rows):
        raise ValueError("registry workloads must be objects with string ids")
    by_id: dict[str, dict[str, Any]] = {}
    for row in rows:
        if row["id"] in by_id:
            raise ValueError(f"duplicate registry workload id: {row['id']}")
        by_id[row["id"]] = row
    return data, _sha(path.read_bytes()), by_id


def load(path: str | Path, registry_path: str | Path = DEFAULT_REGISTRY) -> dict[str, Any]:
    """Load and validate a candidate manifest against the complete registry."""
    manifest_path = Path(path).resolve()
    return _validate_object(_read_json(manifest_path), registry_path)


def rows(manifest: dict[str, Any], family: str | None = None,
         registry_path: str | Path = DEFAULT_REGISTRY) -> list[dict[str, Any]]:
    """Return selected global or explicitly bound fixed-model rows."""
    validated = load_data(manifest, registry_path)
    _, _, by_id = _registry(registry_path)
    by_id.update(_fixture_model_rows(validated, by_id))
    return [by_id[item["id"]] for item in validated["rows"] if family is None or item["family"] == family]


def load_data(manifest: dict[str, Any], registry_path: str | Path = DEFAULT_REGISTRY) -> dict[str, Any]:
    """Validate an in-memory manifest using the same checks as :func:`load`."""
    return _validate_object(manifest, registry_path)


def _validate_object(manifest: dict[str, Any], registry_path: str | Path) -> dict[str, Any]:
    """Validate a decoded manifest object."""
    if (not isinstance(manifest, dict) or not MANIFEST_FIELDS.issubset(manifest)
            or set(manifest) - MANIFEST_FIELDS - OPTIONAL_MANIFEST_FIELDS):
        raise ValueError(f"manifest fields must be {sorted(MANIFEST_FIELDS)} with optional fixture_models")
    if manifest["schema"] != SCHEMA or not isinstance(manifest["task"], str) or not _TASK.fullmatch(manifest["task"]):
        raise ValueError("invalid schema or task")
    if manifest["lane"] != "candidate" or not isinstance(manifest["rationale"], str) or not manifest["rationale"].strip():
        raise ValueError("lane must be candidate and rationale nonempty")
    import p140_entry_policy
    p140_entry_policy.declaration(manifest)
    registry, registry_hash, by_id = _registry(registry_path)
    if manifest["registry_sha256"] != registry_hash:
        raise ValueError("manifest registry_sha256 does not match registry bytes")
    model_rows = _fixture_model_rows(manifest, by_id)
    by_id.update(model_rows)
    cases, declared = manifest["semantic_cases"], manifest["rows"]
    if (not isinstance(cases, list) or not cases or any(not isinstance(x, str) or not x.strip() for x in cases)
            or len(cases) != len(set(cases))):
        raise ValueError("semantic_cases must contain unique nonempty strings")
    if not isinstance(declared, list) or not declared:
        raise ValueError("rows must be a nonempty list")
    seen: set[str] = set()
    for item in declared:
        if not isinstance(item, dict) or set(item) != ROW_FIELDS:
            raise ValueError(f"each row must have exactly {sorted(ROW_FIELDS)}")
        fam, wid = item["family"], item["id"]
        if not isinstance(fam, str) or fam not in FAMILIES or not isinstance(wid, str) or wid not in by_id:
            raise ValueError(f"unknown family or workload: {fam}/{wid}")
        if wid in seen:
            raise ValueError(f"duplicate selected workload id: {wid}")
        seen.add(wid)
        if wid in model_rows:
            if fam != "scalar":
                raise ValueError(f"fixed model workload {wid} must use virtual scalar family")
        elif by_id[wid].get("oracle", {}).get("module") == "continuation_live_workloads.py":
            if (fam != "continuations"
                    or wid not in {f"continuation_live_{n}" for n in (1, 8, 73, 512)}):
                raise ValueError(f"invalid live-continuation family or fixed row: {wid}")
        elif by_id[wid].get("oracle", {}).get("module") == "shared_bulk_workloads.py":
            selection = registry["selections"].get("shared-bulk")
            if (fam != "shared-bulk" or not isinstance(selection, dict)
                    or wid not in selection.get("workloads", [])):
                raise ValueError(f"shared-bulk row {wid} must use the shared-bulk family")
        else:
            selection = registry["selections"].get(fam)
            if not isinstance(selection, dict) or wid not in selection.get("workloads", []):
                raise ValueError(f"{wid} is not in registry selection {fam}")
        if (not isinstance(item["role"], str) or item["role"] not in {"targeted", "held_out", "protected"}
                or not isinstance(item["scale"], str) or item["scale"] not in {"small", "large", "other"}):
            raise ValueError(f"invalid role or scale for {wid}")
        stages, full_stages = item["stages"], by_id[wid].get("stages")
        if not isinstance(stages, list) or not stages or not isinstance(full_stages, list):
            raise ValueError(f"stages must be nonempty for {wid}")
        if (any(not isinstance(stage, str) for stage in stages) or len(stages) != len(set(stages))
                or any(stage not in full_stages for stage in stages)):
            raise ValueError(f"stages must be a unique subset of registry stages for {wid}")
        if fam == "threads" and (full_stages != ["steady_state_group"] or stages != ["steady_state_group"]):
            raise ValueError(f"thread row {wid} must select its exact steady_state_group stage")
        if by_id[wid].get("oracle", {}).get("module") == "continuation_live_workloads.py":
            if (fam != "continuations" or wid not in {f"continuation_live_{n}" for n in (1, 8, 73, 512)}
                    or full_stages != ["setup", "capture"] or stages != ["setup", "capture"]):
                raise ValueError(f"live-continuation row {wid} must select its exact setup/capture stages")
        if by_id[wid].get("oracle", {}).get("module") == "shared_bulk_workloads.py":
            if (fam != "shared-bulk" or full_stages != ["steady_state_group"]
                    or stages != ["steady_state_group"]):
                raise ValueError(f"shared-bulk row {wid} must select its exact steady_state_group stage")
        digest = item["row_sha256"]
        if not isinstance(digest, str) or not _SHA.fullmatch(digest) or digest != _canonical_sha(by_id[wid]):
            raise ValueError(f"row_sha256 mismatch for {wid}")
    for role, scale in (("targeted", "small"), ("targeted", "large"), ("held_out", None), ("protected", None)):
        if not any(x["role"] == role and (scale is None or x["scale"] == scale) for x in declared):
            raise ValueError(f"selection requires {role}" + (f" {scale}" if scale else "") + " workload")
    protected = {x["id"]: set(x["stages"]) for x in declared if x["role"] == "protected"}
    if "prepare_large" not in protected or "prepare" not in protected["prepare_large"]:
        raise ValueError("prepare_large must be protected for its prepare stage")
    if "call_overhead" not in protected or not {"first_call", "load_and_call"}.issubset(protected["call_overhead"]):
        raise ValueError("call_overhead must be protected for first_call and load_and_call")
    return manifest


def groups(manifest: dict[str, Any], registry_path: str | Path = DEFAULT_REGISTRY) -> set[tuple[str, str, str]]:
    _, registry_hash, by_id = _registry(registry_path)
    if "fixture_models" in manifest or manifest.get("registry_sha256") == registry_hash:
        validated = load_data(manifest, registry_path)
        by_id.update(_fixture_model_rows(validated, by_id))
    else:
        # Historically groups() accepted already-decoded synthetic manifests
        # without consulting the global registry. Keep that API for legacy data.
        validated = manifest
    return {(item["family"], item["id"], stage)
            for item in validated["rows"] for stage in item["stages"]}


def _fixture_model_rows(manifest: dict[str, Any], global_rows: dict[str, dict[str, Any]]) -> dict[str, dict[str, Any]]:
    """Load only the allowlisted, source-bound fixed model named by a manifest."""
    if "fixture_models" not in manifest:
        return {}
    bindings = manifest["fixture_models"]
    if not isinstance(bindings, list) or not bindings:
        raise ValueError("fixture_models must be a nonempty list")
    seen: set[str] = set()
    combined: dict[str, dict[str, Any]] = {}
    for binding in bindings:
        if not isinstance(binding, dict) or set(binding) != {"name", "source_sha256"}:
            raise ValueError("fixture model binding must contain exactly name and source_sha256")
        name, digest = binding["name"], binding["source_sha256"]
        if not isinstance(name, str) or name != "p128_scalar" or name in seen:
            raise ValueError(f"unknown or duplicate fixture model: {name!r}")
        if not isinstance(digest, str) or not _SHA.fullmatch(digest):
            raise ValueError("fixture model source_sha256 must be lowercase SHA-256")
        seen.add(name)
        bench_root = HERE.resolve()
        source = (bench_root / "scalar_p128_workloads.py").resolve(strict=True)
        if not source.is_relative_to(bench_root) or not source.is_file():
            raise ValueError("fixed P128 model source escapes benchmark root")
        if _sha(source.read_bytes()) != digest:
            raise ValueError("fixed P128 model source hash mismatch")
        spec = importlib.util.spec_from_file_location("_wtime_selection_p128_scalar_model", source)
        if spec is None or spec.loader is None:
            raise ValueError("cannot load fixed P128 scalar model")
        model = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(model)
        model_rows = model.entries()
        ids = [row.get("id") for row in model_rows if isinstance(row, dict)]
        if len(model_rows) != 18 or len(ids) != 18 or len(set(ids)) != 18:
            raise ValueError("fixed P128 scalar model must yield 18 unique aliases")
        for row in model_rows:
            wid = row["id"]
            if wid in global_rows or wid in combined:
                raise ValueError(f"fixed model workload collides with global registry id: {wid}")
            combined[wid] = row
    return combined


def _repository_root(registry_path: str | Path) -> Path:
    path = Path(registry_path).resolve()
    if (path.name == "workloads.json" and path.parent.name == "wtime-bench"
            and path.parent.parent.name == "tools"):
        return path.parents[2]
    return path.parent


def _core_runtime_sources(predecessor_manifest_path: Path, registry_path: str | Path) -> dict[str, str]:
    """Read core source hashes from archive payloads, checking every recorded byte."""
    manifest = _read_json(predecessor_manifest_path)
    files = manifest.get("files") if isinstance(manifest, dict) else None
    if not isinstance(manifest, dict) or manifest.get("schema") != 1 or not isinstance(files, dict):
        raise ValueError("predecessor archive manifest must use schema 1 and contain a files object")
    archive_root = predecessor_manifest_path.parent.resolve()
    repository_root = _repository_root(registry_path)
    result: dict[str, str] = {}
    required_crates: set[str] = set()
    for archive_name, record in files.items():
        if not isinstance(archive_name, str) or not isinstance(record, dict) or record.get("role") != "source":
            continue
        source_path = record.get("source_path")
        if not isinstance(source_path, str):
            continue
        logical = Path(source_path)
        if logical.is_absolute() or ".." in logical.parts:
            raise ValueError(f"unsafe predecessor source_path: {source_path}")
        normalized = logical.as_posix()
        crate = next((name for name in _CORE_CRATES
                      if normalized.startswith(f"crates/{name}/")
                      and (normalized.endswith(".rs") or logical.name == "Cargo.toml")), None)
        root_manifest = normalized in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
        if crate is None and not root_manifest:
            continue
        digest = record.get("sha256")
        if not isinstance(digest, str) or not _SHA.fullmatch(digest):
            raise ValueError(f"invalid predecessor source hash for {normalized}")
        if normalized in result:
            raise ValueError(f"duplicate predecessor source identity: {normalized}")
        archive_file = (archive_root / archive_name).resolve()
        if not archive_file.is_relative_to(archive_root) or not archive_file.is_file():
            raise ValueError(f"predecessor archived source is missing or unsafe: {archive_name}")
        if _sha(archive_file.read_bytes()) != digest:
            raise ValueError(f"predecessor archived source hash mismatch: {normalized}")
        current_file = (repository_root / logical).resolve()
        if not current_file.is_relative_to(repository_root):
            raise ValueError(f"predecessor source escapes repository root: {normalized}")
        if not current_file.is_file():
            raise ValueError(f"current core source is missing: {normalized}")
        result[normalized] = digest
        if crate:
            required_crates.add(crate)
    missing_crates = sorted(set(_CORE_CRATES) - required_crates)
    if missing_crates:
        raise ValueError(f"predecessor archive lacks core crate sources: {', '.join(missing_crates)}")
    return result


def _current_core_paths(registry_path: str | Path) -> set[str]:
    """Enumerate every current Rust/Cargo source identity covered by a freeze."""
    root = _repository_root(registry_path)
    paths: set[str] = set()
    for crate in _CORE_CRATES:
        crate_root = root / "crates" / crate
        if crate_root.is_dir():
            paths.update(path.relative_to(root).as_posix() for path in crate_root.rglob("*.rs") if path.is_file())
            paths.update(path.relative_to(root).as_posix() for path in crate_root.rglob("Cargo.toml") if path.is_file())
    for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
        path = root / name
        if path.is_file():
            paths.add(name)
    return paths


def freeze(manifest_path: str | Path, predecessor_path: str | Path, output_path: str | Path,
           registry_path: str | Path = DEFAULT_REGISTRY,
           runtime_sources: dict[str, Any] | None = None) -> dict[str, Any]:
    manifest_path, predecessor_path, output_path = map(lambda p: Path(p).resolve(),
                                                        (manifest_path, predecessor_path, output_path))
    manifest = load(manifest_path, registry_path)
    if not predecessor_path.is_file():
        raise ValueError(f"predecessor manifest does not exist: {predecessor_path}")
    if output_path.exists():
        raise ValueError(f"freeze output already exists: {output_path}")
    predecessor = _sha(predecessor_path.read_bytes())
    archived_runtime = _core_runtime_sources(predecessor_path, registry_path)
    current_paths = _current_core_paths(registry_path)
    if set(archived_runtime) != current_paths:
        missing = sorted(current_paths - set(archived_runtime))
        extra = sorted(set(archived_runtime) - current_paths)
        raise ValueError(f"predecessor core source membership differs from current tree (missing={missing}, extra={extra})")
    if runtime_sources is not None and runtime_sources != archived_runtime:
        raise ValueError("supplied runtime_sources differ from predecessor archive core source identities")
    repository_root = _repository_root(registry_path)
    for logical, digest in archived_runtime.items():
        current = (repository_root / logical).resolve()
        if not current.is_relative_to(repository_root) or _sha(current.read_bytes()) != digest:
            raise ValueError(f"current core source differs from predecessor: {logical}")
    receipt = {
        "schema": "wtime-candidate-selection-freeze-v1",
        "manifest_path": str(manifest_path),
        "manifest_sha256": _sha(manifest_path.read_bytes()),
        "registry_path": str(Path(registry_path).resolve()),
        "registry_sha256": _registry(registry_path)[1],
        "predecessor_path": str(predecessor_path),
        "predecessor_sha256": predecessor,
        "runtime_sources": archived_runtime,
        "frozen_at_utc": datetime.now(timezone.utc).isoformat(),
        "selection_groups": [list(item) for item in sorted(groups(manifest, registry_path))],
        "selection_count": len(manifest["rows"]),
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation makes an accidental overwrite impossible even under races.
    with output_path.open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return receipt


def verify_frozen(manifest_path: str | Path, freeze_path: str | Path,
                  registry_path: str | Path = DEFAULT_REGISTRY) -> dict[str, Any]:
    manifest_path, freeze_path = Path(manifest_path).resolve(), Path(freeze_path).resolve()
    manifest = load(manifest_path, registry_path)
    receipt = _read_json(freeze_path)
    if not isinstance(receipt, dict) or receipt.get("schema") != "wtime-candidate-selection-freeze-v1":
        raise ValueError("invalid candidate selection freeze receipt")
    checks = {
        "manifest_path": str(manifest_path),
        "manifest_sha256": _sha(manifest_path.read_bytes()),
        "registry_path": str(Path(registry_path).resolve()),
        "registry_sha256": _registry(registry_path)[1],
        "selection_groups": [list(item) for item in sorted(groups(manifest, registry_path))],
        "selection_count": len(manifest["rows"]),
    }
    for key, expected in checks.items():
        if receipt.get(key) != expected:
            raise ValueError(f"frozen receipt {key} mismatch")
    predecessor = Path(str(receipt.get("predecessor_path", "")))
    if not predecessor.is_absolute() or not predecessor.is_file() or _sha(predecessor.read_bytes()) != receipt.get("predecessor_sha256"):
        raise ValueError("frozen predecessor manifest missing or changed")
    archived_runtime = _core_runtime_sources(predecessor, registry_path)
    if receipt.get("runtime_sources") != archived_runtime:
        raise ValueError("frozen receipt runtime_sources differ from predecessor archive")
    return manifest


def _main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("manifest")
    validate.add_argument("--registry", default=str(DEFAULT_REGISTRY))
    frozen = sub.add_parser("freeze")
    frozen.add_argument("manifest")
    frozen.add_argument("predecessor")
    frozen.add_argument("output")
    frozen.add_argument("--registry", default=str(DEFAULT_REGISTRY))
    args = parser.parse_args(argv)
    try:
        if args.command == "validate":
            manifest = load(args.manifest, args.registry)
            print(json.dumps({"valid": True, "task": manifest["task"], "groups": len(groups(manifest))}, sort_keys=True))
        else:
            print(json.dumps(freeze(args.manifest, args.predecessor, args.output, args.registry), sort_keys=True))
    except (ValueError, OSError) as exc:
        print(f"selection manifest error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
