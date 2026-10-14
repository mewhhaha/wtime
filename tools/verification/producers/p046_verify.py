#!/usr/bin/env python3
"""P046 freeze, immutable baseline, retained graph, and evidence sealing phases."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
PRODUCERS = ROOT / "tools/verification/producers"
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(PRODUCERS))

P001_MANIFEST = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
FAMILIES = ("scalar", "memory", "simd", "representative")

def sha(path: Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def write_new(path: Path, value: Any) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def read(path: Path) -> Any:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def _runtime_sources(manifest: dict[str, Any]) -> dict[str, str]:
    return {row["source_path"]: row["sha256"]
            for row in manifest["files"].values()
            if row.get("role") == "source"
            and row.get("source_path", "").startswith("crates/wtime-runtime/src/")}


def _validate_frozen_archive(path: Path, mode: str, expected_manifest: str,
                             declared_map: dict[str, str] | None = None) -> dict[str, Any]:
    import baseline
    path = Path(path).resolve(strict=True)
    manifest = baseline.verify_archive(path)
    if sha(path / "manifest.json") != expected_manifest:
        raise ValueError(f"{path.name} manifest differs from its immutable predeclaration")
    if manifest.get("mode") != mode:
        raise ValueError(f"{path.name} mode is {manifest.get('mode')!r}, expected {mode!r}")
    runtime = _runtime_sources(manifest)
    if len(runtime) != 32:
        raise ValueError(f"{path.name} must archive exactly 32 runtime source files")
    if declared_map is not None and runtime != declared_map:
        raise ValueError(f"{path.name} runtime source map differs from the predeclaration")
    return manifest


def freeze(output: Path) -> dict[str, Any]:
    """Validate P046-before, its matched decoded archive, and immutable P001."""
    import baseline
    compact_predecl = ROOT / "target/performance/P046/predeclaration/attempt-0001/receipt.json"
    decoded_predecl = ROOT / "target/performance/P046/predeclaration/attempt-0002/receipt.json"
    compact_receipt, decoded_receipt = read(compact_predecl), read(decoded_predecl)
    for label, receipt in (("Compact", compact_receipt), ("decoded", decoded_receipt)):
        if receipt.get("status") != "passed" or receipt.get("source_stable") is not True:
            raise ValueError(f"P046 {label} predeclaration did not pass")
    compact_path = ROOT / "target/performance/P046-before"
    decoded_path = ROOT / "target/performance/P046-decoded-before"
    compact_manifest = _validate_frozen_archive(
        compact_path, "compact", compact_receipt["baseline_manifest"]["sha256"],
        compact_receipt["runtime_source_map"])
    decoded_manifest = _validate_frozen_archive(
        decoded_path, "decoded", decoded_receipt["baseline_manifest"]["sha256"],
        compact_receipt["runtime_source_map"])
    if decoded_receipt.get("matched_compact_archive_manifest", {}).get("sha256") != sha(compact_path / "manifest.json"):
        raise ValueError("decoded predecessor receipt does not bind the matched Compact archive")

    p001_path = ROOT / "target/performance/baseline"
    p001 = baseline.verify_archive(p001_path)
    if (sha(p001_path / "manifest.json") != P001_MANIFEST
            or sha(p001_path / "binary/wtime-bench") != P001_BINARY
            or len(p001.get("files", {})) != 85
            or not p001.get("smoke", {}).get("correctness_checked")):
        raise ValueError("immutable P001 archive identity or smoke evidence differs")
    # The frozen P001 archive owns exact copies of the two historical reports.
    for name in ("wasmtime-comparison.json", "wasmtime-comparison-run2.json"):
        archived = p001_path / "historical_reports" / name
        original = ROOT / "docs/benchmarks" / name
        if not archived.is_file() or sha(archived) != sha(original):
            raise ValueError(f"P001 historical report copy differs: {name}")

    report = {
        "schema": "p046-freeze-v1", "status": "passed", "timing_evidence": False,
        "archives": {"compact_predecessor": ref(compact_path / "manifest.json"),
                     "decoded_predecessor": ref(decoded_path / "manifest.json"),
                     "p001_original": ref(p001_path / "manifest.json"),
                     "p001_binary": ref(p001_path / "binary/wtime-bench")},
        "predeclarations": {"compact": ref(compact_predecl), "decoded": ref(decoded_predecl)},
        "runtime_source_map": _runtime_sources(decoded_manifest),
        "runtime_source_file_count": len(_runtime_sources(decoded_manifest)),
        "p001_file_count": len(p001["files"]),
    }
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    write_new(output / "report.json", report)
    return report


def _core_source_map() -> dict[str, str]:
    result = {}
    for crate in CORE_CRATES:
        directory = ROOT / "crates" / crate / "src"
        result.update({path.relative_to(ROOT).as_posix(): sha(path)
                       for path in directory.rglob("*.rs") if path.is_file()})
    if not all(any(name.startswith(f"crates/{crate}/src/") for name in result)
               for crate in CORE_CRATES):
        raise ValueError("production source identity does not include all four core crates")
    return dict(sorted(result.items()))


def _production_sources() -> dict[str, str]:
    import compare
    # Include the entire core and the timed benchmark/harness map used by compare.py.
    values = compare.sources(include_memory=True, include_simd=True, include_representative=True)
    return dict(sorted(values.items()))


def _run_logged(argv: list[str], cwd: Path, directory: Path, label: str,
                *, env: dict[str, str] | None = None) -> tuple[subprocess.CompletedProcess[str], Path, Path]:
    command_path = directory / f"{label}.command.json"
    stdout_path, stderr_path = directory / f"{label}.stdout", directory / f"{label}.stderr"
    write_new(command_path, {"argv": argv, "cwd": str(cwd.resolve())})
    with stdout_path.open("x", encoding="utf-8") as stdout, stderr_path.open("x", encoding="utf-8") as stderr:
        result = subprocess.run(argv, cwd=cwd, env=env, text=True, stdout=stdout,
                                stderr=stderr, check=False)
    write_new(directory / f"{label}.status.json", {"exit_code": result.returncode})
    return result, stdout_path, stderr_path


def build_production(output: Path) -> dict[str, Any]:
    """Build and attest the uninstrumented current Compact benchmark binary."""
    output = Path(output).resolve()
    directory = output / "production"
    directory.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, directory / "helper.executed.py")
    before = _production_sources()
    before_core = _core_source_map()
    command = ["cargo", "build", "--locked", "--release", "--no-default-features",
               "--manifest-path", "tools/wtime-bench/Cargo.toml",
               "--features", "compact-executor", "--bin", "wtime-bench"]
    proc, stdout, stderr = _run_logged(command, ROOT, directory, "build")
    source_binary = Path(os.environ.get("CARGO_TARGET_DIR", str(BENCH / "target"))) / "release/wtime-bench"
    if proc.returncode:
        raise RuntimeError(f"production build failed with {proc.returncode}; see {directory}")
    if not source_binary.is_file():
        raise ValueError(f"Cargo succeeded without producing {source_binary}")
    binary_dir = directory / "bin"
    binary_dir.mkdir()
    binary = binary_dir / "wtime-bench"
    shutil.copy2(source_binary, binary)
    protocol_command = [str(binary), "--protocol"]
    pproc, protocol_stdout, protocol_stderr = _run_logged(
        protocol_command, ROOT, directory, "protocol")
    if pproc.returncode:
        raise RuntimeError(f"production protocol probe failed with {pproc.returncode}; see {directory}")
    protocol = json.loads(protocol_stdout.read_text(encoding="utf-8"))
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
            or protocol.get("instrumented") is not False
            or "compact" not in protocol.get("modes", [])):
        raise ValueError("production protocol is not the expected uninstrumented Compact protocol")
    after = _production_sources()
    after_core = _core_source_map()
    stable = before == after and before_core == after_core
    if not stable:
        raise ValueError("candidate sources changed during the production build")
    record = {
        "schema": "wtime-production-command-v1", "phase": "build", "exit_code": 0,
        "source_stable": True, "cwd": str(ROOT), "command": command,
        "binary_path": str(binary), "binary_sha256": sha(binary),
        "runtime_source_map": {name: digest for name, digest in before_core.items()
                               if name.startswith("crates/wtime-runtime/src/")},
        "core_source_map": before_core, "source_identities": before,
        "feature_flags": ["compact-executor"],
        "protocol_command": protocol_command, "protocol_exit": 0,
        "helper_sha256": sha(directory / "helper.executed.py"),
        "stdout_sha256": sha(stdout), "stderr_sha256": sha(stderr),
        "protocol_stdout": str(protocol_stdout), "protocol_stderr": str(protocol_stderr),
        "protocol_sha256": sha(protocol_stdout),
    }
    shutil.copy2(stdout, directory / "stdout.log")
    shutil.copy2(stderr, directory / "stderr.log")
    write_new(directory / "protocol.stdout.json", protocol)
    write_new(directory / "build-record.json", record)
    return record


def build_baselines(output: Path, *, run_probe: bool = True) -> dict[str, Any]:
    """Create fresh stage-local P001 and decoded current-harness adapters."""
    import baseline
    import p123_adapter
    output = Path(output).resolve()
    base_dir = output / "baselines"
    base_dir.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, base_dir / "helper.executed.py")
    p001_archive = ROOT / "target/performance/baseline"
    frozen_decoded = ROOT / "target/performance/P046-decoded-before"
    p001_manifest = baseline.verify_archive(p001_archive)
    if (sha(p001_archive / "manifest.json") != P001_MANIFEST
            or sha(p001_archive / "binary/wtime-bench") != P001_BINARY):
        raise ValueError("P001 source archive changed before adapter creation")
    p001_path = base_dir / "adapters/p001"
    structural_path = base_dir / "adapters/structural"
    # create() preserves its executed helper, Cargo logs, source identity, and fresh binary.
    p001 = p123_adapter.create(p001_archive, p001_path, mode="decoded", legacy_api=True)
    structural = p123_adapter.create(frozen_decoded, structural_path,
                                     mode="decoded", legacy_api=False)
    for record, legacy, label in ((p001, True, "P001"), (structural, False, "structural")):
        if record.get("legacy_api") is not legacy or record.get("mode") != "decoded":
            raise ValueError(f"{label} adapter has the wrong frozen API/mode")
        if len(record.get("frozen_core_identities", {})) < 4:
            raise ValueError(f"{label} adapter is missing frozen core identities")
    probe_ref = None
    if run_probe:
        import p123
        probe_dir = base_dir / "p123probe" / "capabilities"
        capability = p123.probe(p001_path, probe_dir)
        if capability.get("timing_evidence") is not False or capability.get("actual_stage_probes") != 1095:
            raise ValueError("fresh P123 capability probe did not complete 1,095 actual stage probes")
        probe_ref = ref(probe_dir / "report.json")
    report = {"schema": "p046-baselines-v1", "status": "passed",
              "timing_evidence": False,
              "p001_adapter": ref(p001_path / "manifest.json"),
              "structural_adapter": ref(structural_path / "manifest.json"),
              "p001_binary": ref(p001_path / "binary/wtime-bench"),
              "structural_binary": ref(structural_path / "binary/wtime-bench"),
              "p001_archive": ref(p001_archive / "manifest.json"),
              "decoded_predecessor": ref(frozen_decoded / "manifest.json"),
              "p123_capabilities": probe_ref,
              "p001_file_count": len(p001_manifest["files"])}
    write_new(base_dir / "report.json", report)
    return report


def _product_rows(generated_dir: Path) -> list[dict[str, Any]]:
    import workloads
    entries = []
    all_rows = workloads.load_registry()["workloads"]
    selected = {item["id"]: item for family in FAMILIES for item in workloads.selection(family)}
    # Keep the original large preparation control even if a future family registry
    # moves it out of the scalar timed selection.
    prepare_large = next((item for item in all_rows if item.get("id") == "prepare_large"), None)
    if prepare_large is not None:
        selected.setdefault("prepare_large", prepare_large)
    for wid, item in selected.items():
        source_record = item["source"]
        if source_record.get("kind") == "deterministic_generator":
            import workloads as workload_models
            source = Path(generated_dir) / f"{wid}-generated.wat"
            generated = (workload_models.generate_prepare_large() + "\n").encode("utf-8")
            if hashlib.sha256(generated).hexdigest() != source_record["sha256"]:
                raise ValueError(f"generated workload source identity changed: {wid}")
            source.parent.mkdir(parents=True, exist_ok=True)
            if source.exists():
                if sha(source) != source_record["sha256"]:
                    raise ValueError(f"retained generated source path is stale: {source}")
            else:
                source.write_bytes(generated)
        else:
            source = BENCH / source_record["path"]
        if not source.is_file() or sha(source) != source_record["sha256"]:
            raise ValueError(f"workload source identity changed: {item.get('id')}")
        driver_profile = item.get("driver", {}).get("profile")
        features = set(item.get("required_features", ["mvp"]))
        if item.get('oracle',{}).get('module') == 'memory_workloads.py':
            import memory_scaling
            profile=memory_scaling._profile(item)
        elif item.get('oracle',{}).get('module') == 'simd_workloads.py':
            import simd_scaling
            profile=simd_scaling.execution_profile(item)
        elif isinstance(driver_profile, str):
            profile = driver_profile
        elif "relaxed_simd" in features:
            profile = "simd-relaxed"
        elif "simd" in features:
            profile = "simd"
        elif "saturating_float_to_int" in features:
            profile = "mvp-saturating"
        elif features <= {"mvp"}:
            profile = "mvp"
        elif features <= {"mvp", "bulk_memory", "reference_types", "multi_value", "sign_extension"}:
            profile = "stable-core"
        else:
            profile = "all-phase3-core"
        key = (source_record["sha256"], profile)
        wasm_path = (ROOT / "target/performance/P046-decoded-before/fixtures/prepare_large.wasm"
                     if wid == "prepare_large" else None)
        if wasm_path is not None and sha(wasm_path) != source_record["wasm_sha256"]:
            raise ValueError("frozen prepare_large fixture differs from the declared Wasm hash")
        entries.append({"family": item.get("family", "original-mvp"), "id": item["id"], "source": source,
                        "source_sha256": key[0], "profile": profile,
                        "wasm_sha256": source_record["wasm_sha256"], "wasm_path": wasm_path,
                        "key": key})
    # Keep a single product per WAT byte identity/profile while retaining full membership.
    unique: dict[tuple[str, str], dict[str, Any]] = {}
    for row in entries:
        unique.setdefault(row["key"], {**row, "members": []})["members"].append(
            {"family": row["family"], "id": row["id"]})
    return list(unique.values())


def retained(binary: Path, output: Path, *, encoder: Path | None = None) -> dict[str, Any]:
    """Measure decoded/Compact compiled graph requested heap for each unique product."""
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, output / "helper.executed.py")
    original_binary = Path(binary).resolve(strict=True)
    if not original_binary.is_file():
        raise ValueError("retained diagnostic binary is missing")
    binary_dir = output / "bin"
    binary_dir.mkdir()
    binary = binary_dir / "wtime-retained"
    shutil.copy2(original_binary, binary)
    original_binary_hash = sha(original_binary)
    if sha(binary) != original_binary_hash:
        raise ValueError("retained diagnostic binary copy differs from its build artifact")
    encoder = Path(encoder or ROOT / "target/performance/baseline/binary/wtime-conformance").resolve(strict=True)
    before_sources = _production_sources()
    before_binary, before_encoder = sha(binary), sha(encoder)
    fixture_dir = output / "fixtures"
    fixture_dir.mkdir()
    products = _product_rows(output / "generated")
    rows = []
    commands_path = output / "commands.jsonl"
    for index, product in enumerate(products):
        fixture = fixture_dir / f"{index:04d}-{product['source_sha256'][:12]}.wasm"
        # This pinned P001 encoder runs once for each unique WAT/profile product.
        archived_fixture = product.get("wasm_path")
        if archived_fixture is not None:
            shutil.copy2(archived_fixture, fixture)
        elif not fixture.exists():
            argv = [str(encoder), "encode", str(product["source"]), "-o", str(fixture)]
            with commands_path.open("a", encoding="utf-8") as log:
                log.write(json.dumps({"argv": argv, "cwd": str(ROOT)}) + "\n")
            result = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, check=False)
            (output / f"encode-{index:04d}.stdout").write_text(result.stdout, encoding="utf-8")
            (output / f"encode-{index:04d}.stderr").write_text(result.stderr, encoding="utf-8")
            if result.returncode:
                raise RuntimeError(f"P001 encoder failed for {product['source']}")
        if sha(fixture) != product["wasm_sha256"]:
            raise ValueError(f"encoded Wasm differs from pinned fixture: {product['source']}")
        modes = {}
        for mode in ("decoded", "compact"):
            argv = [str(binary), str(fixture), mode, product["profile"]]
            with commands_path.open("a", encoding="utf-8") as log:
                log.write(json.dumps({"argv": argv, "cwd": str(ROOT), "timing": False}) + "\n")
            result = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, check=False)
            (output / f"{index:04d}-{mode}.stdout").write_text(result.stdout, encoding="utf-8")
            (output / f"{index:04d}-{mode}.stderr").write_text(result.stderr, encoding="utf-8")
            if result.returncode:
                raise RuntimeError(f"retained heap probe failed for {product['source']} ({mode})")
            value = json.loads(result.stdout)
            if (value.get("schema") != "wtime-retained-v1" or value.get("instrumented") is not True
                    or value.get("timing_evidence") is not False or value.get("mode") != mode
                    or value.get("profile") != product["profile"] or value.get("graph_released") is not True):
                raise ValueError("retained heap probe returned invalid diagnostic evidence")
            graph = value.get("compiled_graph_requested_bytes")
            if type(graph) is not int or graph <= 0:
                raise ValueError("retained heap probe lacks positive compiled graph bytes")
            modes[mode] = value
        rows.append({"source_sha256": product["source_sha256"],
                     "wasm_sha256": product["wasm_sha256"], "profile": product["profile"],
                     "members": product["members"], "fixture": ref(fixture),
                     "decoded": modes["decoded"], "compact": modes["compact"]})
    # The metric is the sum of measured Engine+Module graph heap across the complete
    # unique WAT/profile portfolio; it includes code and plans as one observed graph.
    decoded_total = sum(row["decoded"]["compiled_graph_requested_bytes"] for row in rows)
    compact_total = sum(row["compact"]["compiled_graph_requested_bytes"] for row in rows)
    if sha(binary) != before_binary or sha(encoder) != before_encoder:
        raise ValueError("retained probe executable or pinned encoder changed during collection")
    report = {"task": "P046", "schema": "p046-retained-v1", "status": "passed",
              "source_stable": True, "timing_evidence": False,
              "binary": ref(binary), "binary_original": ref(original_binary),
              "encoder": ref(encoder),
              "bytes": {"decoded_module_code_graph_requested_bytes": decoded_total,
                        "compact_module_code_graph_requested_bytes": compact_total},
              "artifacts": {"binary": ref(binary), "encoder": ref(encoder),
                            **{f"fixture-{index:04d}": row["fixture"]
                               for index, row in enumerate(rows)}},
              "rows": rows,
              "simplification_packaging_decision": "The aggregate is requested live heap for each complete compiled Engine+Module graph; compiled code and module plan allocations are included together because this allocator probe cannot assign physical code pages separately.",
              "source_identities": before_sources}
    if _production_sources() != before_sources:
        raise ValueError("sources changed during retained graph collection")
    write_new(output / "report.json", report)
    return report


def _seal_source_snapshot() -> dict[str, Any]:
    return {"bench_and_runtime": _production_sources(),
            "core_sources": _core_source_map(),
            "producer_sha256": sha(Path(__file__).resolve())}


def _seal_files(root: Path, excluded: set[str]) -> list[tuple[str, Path, dict[str, Any]]]:
    rows = []
    root = root.resolve(strict=True)
    for path in sorted(root.rglob("*"), key=lambda value: value.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        if relative in excluded:
            continue
        if path.is_symlink():
            try:
                resolved = path.resolve(strict=True)
                resolved.relative_to(root)
            except (OSError, ValueError) as exc:
                raise ValueError(f"seal rejects symlink outside its phase root: {relative}") from exc
            if not resolved.is_file():
                raise ValueError(f"seal symlink must resolve to a regular file: {relative}")
            actual = resolved
        elif path.is_dir():
            continue
        else:
            actual = path
        try:
            info = actual.stat()
        except OSError as exc:
            raise ValueError(f"seal cannot stat phase artifact {relative}: {exc}") from exc
        if not stat.S_ISREG(info.st_mode):
            raise ValueError(f"seal accepts regular files only: {relative}")
        rows.append((relative, actual, {"sha256": sha(actual), "bytes": info.st_size,
                                        "mode_bits": stat.S_IMODE(info.st_mode)}))
    return rows


def _write_deterministic_tar_gzip(path: Path,
                                 files: list[tuple[str, Path, dict[str, Any]]]) -> None:
    """Write a deterministic gzip/PAX archive with normalized owner and time."""
    with path.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for relative, source, record in files:
                    data = source.read_bytes()
                    if len(data) != record["bytes"] or hashlib.sha256(data).hexdigest() != record["sha256"]:
                        raise ValueError(f"phase artifact changed during seal: {relative}")
                    info = tarfile.TarInfo(relative)
                    info.type = tarfile.REGTYPE
                    info.size = len(data)
                    info.mode = record["mode_bits"]
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    info.pax_headers = {}
                    archive.addfile(info, io.BytesIO(data))


def seal(output: Path) -> dict[str, Any]:
    """Inventory and package only the actual files beneath an existing phase root."""
    root = Path(output).resolve(strict=True)
    if not root.is_dir():
        raise ValueError("seal output must be an existing phase directory")
    inventory_path, bundle_path = root / "inventory.json", root / "artifacts.tar.gz"
    if inventory_path.exists() or bundle_path.exists():
        raise FileExistsError("seal refuses to replace an existing inventory or artifact bundle")
    helper_copy = root / "seal.helper.executed.py"
    with helper_copy.open("xb") as stream:
        stream.write(Path(__file__).resolve().read_bytes())
    source_before = _seal_source_snapshot()
    # The shared runner owns these two active streams and binds them after this
    # process exits. Its final receipt is written after all phase outputs exist.
    excluded = {"inventory.json", "artifacts.tar.gz", "seal-evidence.stdout",
                "seal-evidence.stderr"}
    files = _seal_files(root, excluded)
    _write_deterministic_tar_gzip(bundle_path, files)
    source_after = _seal_source_snapshot()
    if source_after != source_before:
        raise ValueError("P046 producer/core/harness sources changed during sealing")
    # Recheck every archived input after packaging and before recording the seal.
    for relative, source, record in files:
        logical = root / relative
        if (not logical.is_file() or logical.resolve(strict=True) != source.resolve(strict=True)
                or not source.is_file() or source.stat().st_size != record["bytes"]
                or sha(source) != record["sha256"]):
            raise ValueError(f"phase artifact changed before inventory write: {relative}")
    inventory = {"schema": "p046-stage-seal-v1", "status": "passed",
                 "phase_root": str(root), "source_stable": True,
                 "source_before": source_before, "source_after": source_after,
                 "files": {relative: record for relative, _source, record in files},
                 "excluded_runner_streams": ["seal-evidence.stdout", "seal-evidence.stderr"],
                 "file_count": len(files), "bundle": ref(bundle_path)}
    write_new(inventory_path, inventory)
    return inventory

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="phase", required=True)
    for name in ("freeze", "build-production", "build-baselines", "retained", "seal"):
        item = sub.add_parser(name)
        item.add_argument("--output", type=Path, required=True)
        if name == "retained":
            item.add_argument("--binary", type=Path, required=True)
            item.add_argument("--encoder", type=Path)
        elif name == "build-baselines":
            item.add_argument("--skip-probe", action="store_true")
    args = parser.parse_args()
    if args.phase == "freeze":
        value = freeze(args.output)
    elif args.phase == "build-production":
        value = build_production(args.output)
    elif args.phase == "build-baselines":
        value = build_baselines(args.output, run_probe=not args.skip_probe)
    elif args.phase == "retained":
        value = retained(args.binary, args.output, encoder=args.encoder)
    else:
        value = seal(args.output)
    print(json.dumps({"phase": args.phase, "status": value.get("status", "passed")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
