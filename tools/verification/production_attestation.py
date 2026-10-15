"""Strict, filesystem-only validation for runtime-only production receipts.

This module intentionally imports only the Python standard library and never
executes Cargo, a probe binary, or any other subprocess.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import re
import tomllib
from typing import Any


SCHEMA = "wtime-runtime-only-production-v1"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
MODES = {
    "decoded": {"harness_features": ["runtime-only"], "runtime_features": [],
                "cargo_features": ["runtime-only"]},
    "compact": {"harness_features": ["runtime-only"],
                "runtime_features": ["compact-executor"],
                "cargo_features": ["runtime-only", "compact-executor"]},
    "register": {"harness_features": ["runtime-only"],
                 "runtime_features": ["register-executor"],
                 "cargo_features": ["runtime-only", "register-executor"]},
}
ENTRYPOINT = (
    "#![forbid(unsafe_code)]\n"
    "mod protocol;\n"
    "fn main() {\n"
    " let args = std::env::args().skip(1).collect::<Vec<_>>();\n"
    " if !protocol::try_main(&args) { std::process::exit(2); }\n"
    "}\n"
).encode()
_SHA = re.compile(r"^[0-9a-f]{64}$")


class AttestationError(ValueError):
    """The retained runtime production receipt is incomplete or inconsistent."""


def _fail(message: str) -> None:
    raise AttestationError(message)


def _sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _sha(path: Path) -> str:
    try:
        return _sha_bytes(path.read_bytes())
    except OSError as exc:
        _fail(f"cannot read attested file {path}: {exc}")


def _canonical_hash(value: Any) -> str:
    try:
        data = json.dumps(value, sort_keys=True, separators=(",", ":"),
                          allow_nan=False).encode()
    except (TypeError, ValueError) as exc:
        _fail(f"cannot canonically hash attested data: {exc}")
    return _sha_bytes(data)


def _json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        _fail(f"cannot read {label}: {exc}")


def _digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or not _SHA.fullmatch(value):
        _fail(f"{label} must be a lowercase SHA-256 digest")
    return value


def _inside(path_value: Any, boundary: Path, label: str, *, must_exist: bool = True) -> Path:
    if not isinstance(path_value, (str, Path)) or not str(path_value):
        _fail(f"{label} path is missing")
    path = Path(path_value)
    try:
        resolved_boundary = boundary.resolve(strict=True)
        resolved = path.resolve(strict=must_exist)
    except OSError as exc:
        _fail(f"cannot resolve {label} path: {exc}")
    if not resolved.is_relative_to(resolved_boundary):
        _fail(f"{label} escapes its authorized directory")
    return resolved


def _file_ref(ref: Any, output: Path, label: str) -> tuple[Path, str]:
    if not isinstance(ref, dict):
        _fail(f"{label} reference must be an object")
    path = _inside(ref.get("path"), output, label)
    expected = _digest(ref.get("sha256"), f"{label} SHA")
    if not path.is_file() or _sha(path) != expected:
        _fail(f"{label} path/SHA does not match")
    return path, expected


def _source_map(value: Any, label: str) -> dict[str, str]:
    if not isinstance(value, dict) or not value:
        _fail(f"{label} is missing or empty")
    for name, digest in value.items():
        if not isinstance(name, str) or not name or Path(name).is_absolute() or ".." in Path(name).parts:
            _fail(f"{label} contains an unsafe source name")
        _digest(digest, f"{label}[{name}]")
    return value


def _manifest_bytes(root: Path, mode: str) -> bytes:
    features = MODES[mode]["cargo_features"]
    lines = ["default = []", "runtime-only = []"]
    if "compact-executor" in features:
        lines.append('compact-executor = ["wtime-runtime/compact-executor"]')
    if "register-executor" in features:
        lines.append('register-executor = ["wtime-runtime/register-executor"]')
    return (
        '[package]\nname = "wtime-production-protocol"\nversion = "0.1.0"\n'
        'edition = "2024"\npublish = false\n\n[workspace]\n\n[features]\n'
        + "\n".join(lines) + "\n\n[dependencies]\n"
        + "wtime-runtime = { path = " + json.dumps(str(root / "crates/wtime-runtime"))
        + ", default-features = false }\n"
        + "wtime-types = { path = " + json.dumps(str(root / "crates/wtime-types"))
        + ", default-features = false }\n"
    ).encode()


def _expected_cargo_argv(mode: str, manifest: Path) -> list[str]:
    return ["cargo", "build", "--locked", "--offline", "--release",
            "--no-default-features", "-j4", "--features",
            ",".join(MODES[mode]["cargo_features"]), "--manifest-path", str(manifest),
            "--bin", "wtime-production-protocol"]


def _check_command_logs(record: dict[str, Any], output: Path, root: Path,
                        mode: str, workspace_manifest: Path, binary: Path) -> dict[str, Any]:
    logs = record.get("logs")
    if not isinstance(logs, dict):
        _fail("command logs inventory is missing")
    checked: dict[str, Path] = {}
    for name, ref in logs.items():
        if not isinstance(name, str) or Path(name).name != name:
            _fail("command log inventory contains an unsafe name")
        checked[name] = _file_ref(ref, output, f"command log {name}")[0]

    commands: dict[str, dict[str, Any]] = {}
    statuses: dict[str, dict[str, Any]] = {}
    for name, path in checked.items():
        if name.endswith(".command.json"):
            value = _json(path, name)
            if not isinstance(value, dict) or not isinstance(value.get("argv"), list):
                _fail(f"{name} is malformed")
            if value.get("cwd") != str(root):
                _fail(f"{name} has an unexpected working directory")
            commands[name[:-len(".command.json")]] = value
        elif name.endswith(".status.json"):
            value = _json(path, name)
            if not isinstance(value, dict) or value.get("exit_code") != 0:
                _fail(f"{name} does not record a successful command")
            statuses[name[:-len(".status.json")]] = value
        elif not name.endswith((".stdout", ".stderr")):
            _fail(f"unexpected command log artifact {name}")
    if set(commands) != set(statuses):
        _fail("command and successful status inventories differ")
    required = {"rustc-version-before", "cargo-version-before", "root-metadata-before",
                "metadata-before", "production-build", "protocol", "metadata-after",
                "root-metadata-after", "rustc-version-after", "cargo-version-after"}
    if not required <= set(commands):
        _fail("required production command evidence is missing")
    expected = {
        "metadata-before": ["cargo", "metadata", "--locked", "--offline", "--no-default-features",
                            "--features", ",".join(MODES[mode]["cargo_features"]),
                            "--format-version", "1", "--manifest-path", str(workspace_manifest)],
        "production-build": _expected_cargo_argv(mode, workspace_manifest),
        "protocol": [str(binary), "--protocol"],
        "metadata-after": ["cargo", "metadata", "--locked", "--offline", "--no-default-features",
                           "--features", ",".join(MODES[mode]["cargo_features"]),
                           "--format-version", "1", "--manifest-path", str(workspace_manifest)],
        "root-metadata-before": ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
                                 "--manifest-path", str(root / "Cargo.toml")],
        "root-metadata-after": ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
                                "--manifest-path", str(root / "Cargo.toml")],
        "rustc-version-before": ["rustc", "-Vv"], "rustc-version-after": ["rustc", "-Vv"],
        "cargo-version-before": ["cargo", "-Vv"], "cargo-version-after": ["cargo", "-Vv"],
    }
    for label, argv in expected.items():
        if commands[label].get("argv") != argv:
            _fail(f"{label} command arguments do not match the production contract")
    if "lock" in commands and commands["lock"].get("argv") != [
            "cargo", "generate-lockfile", "--offline", "--manifest-path", str(workspace_manifest)]:
        _fail("lock generation command arguments do not match the offline wrapper contract")
    if record.get("command") != expected["production-build"]:
        _fail("recorded Cargo build command differs from its retained command log")
    protocol_command = record.get("protocol_command")
    if protocol_command != expected["protocol"] or record.get("protocol_exit") != 0:
        _fail("protocol probe command is not the exact successful --protocol invocation")
    for label in commands:
        if f"{label}.stdout" not in checked or f"{label}.stderr" not in checked:
            _fail(f"{label} stdout/stderr logs are incomplete")
    return {"commands": commands, "statuses": statuses, "files": checked}


def _root_metadata_summary(value: Any, root: Path) -> dict[str, Any]:
    if not isinstance(value, dict) or not isinstance(value.get("packages"), list):
        _fail("root Cargo metadata is malformed")
    by_name = {p.get("name"): p for p in value["packages"] if isinstance(p, dict)}
    resolution = {}
    for crate in CORE_CRATES:
        package = by_name.get(crate)
        expected = (root / "crates" / crate / "Cargo.toml").resolve(strict=True)
        if not isinstance(package, dict) or package.get("source") is not None:
            _fail(f"root Cargo metadata does not resolve local core crate {crate}")
        actual = Path(package.get("manifest_path", "")).resolve(strict=True)
        if actual != expected:
            _fail(f"root Cargo metadata resolves {crate} to an unexpected path")
        resolution[crate] = str(actual)
    return {"workspace_root": str(Path(value.get("workspace_root", "")).resolve(strict=True)),
            "core_resolution": resolution, "metadata_sha256": _canonical_hash(value)}


def _metadata_summary(value: Any, root: Path, workspace_manifest: Path,
                      mode: str) -> dict[str, Any]:
    if not isinstance(value, dict) or not isinstance(value.get("packages"), list):
        _fail("focused Cargo metadata is malformed")
    packages = value["packages"]
    names = set(CORE_CRATES) | {"wtime-production-protocol"}
    by_name = {p.get("name"): p for p in packages if isinstance(p, dict)}
    if len(packages) != 5 or set(by_name) != names:
        _fail("runtime-only wrapper must resolve exactly the five local packages")
    if any(p.get("source") is not None for p in packages):
        _fail("runtime-only wrapper resolved an external package")
    resolution = {}
    for crate in CORE_CRATES:
        expected = (root / "crates" / crate / "Cargo.toml").resolve(strict=True)
        actual = Path(by_name[crate].get("manifest_path", "")).resolve(strict=True)
        if actual != expected:
            _fail(f"Cargo resolved {crate} outside the current core")
        resolution[crate] = str(actual)
    wrapper = workspace_manifest.resolve()
    actual_wrapper = Path(by_name["wtime-production-protocol"].get("manifest_path", "")).resolve()
    if actual_wrapper != wrapper:
        _fail("Cargo metadata wrapper manifest path is unexpected")
    nodes = (value.get("resolve") or {}).get("nodes")
    if not isinstance(nodes, list) or len(nodes) != 5:
        _fail("Cargo resolution graph must contain exactly five package nodes")
    ids = {p.get("id") for p in packages}
    node_by_id = {node.get("id"): node for node in nodes if isinstance(node, dict)}
    if set(node_by_id) != ids:
        _fail("Cargo resolution graph package membership differs from package inventory")
    by_id = {p.get("id"): p for p in packages}
    node_by_name = {by_id[node_id]["name"]: node for node_id, node in node_by_id.items()}
    runtime_features = MODES[mode]["runtime_features"]
    wrapper_features = MODES[mode]["cargo_features"]
    if sorted(node_by_name["wtime-runtime"].get("features", [])) != runtime_features:
        _fail("resolved runtime feature set differs from the selected mode")
    if sorted(node_by_name["wtime-production-protocol"].get("features", [])) != sorted(wrapper_features):
        _fail("resolved wrapper feature set differs from the selected mode")
    return {"packages": sorted(names), "external_dependencies": 0,
            "resolution": resolution, "wrapper_manifest": str(wrapper),
            "resolve_sha256": _canonical_hash(nodes), "runtime_features": runtime_features,
            "wrapper_features": wrapper_features}


def validate_runtime_record(record_path: str | Path, metadata: dict[str, Any],
                            report: dict[str, Any], root: str | Path) -> dict[str, Any]:
    """Validate one retained runtime-only production receipt without running tools.

    ``metadata`` is the comparison report's metadata object. Returned fields match
    ``gates._validate_candidate_build_record`` and add ``lane``.
    """
    try:
        root = Path(root).resolve(strict=True)
        record_file = Path(record_path).resolve(strict=True)
        if not record_file.is_file() or not record_file.is_relative_to(root):
            _fail("production record is outside the selected repository")
        output = record_file.parent.resolve(strict=True)
        record = _json(record_file, "runtime production record")
        if not isinstance(record, dict):
            _fail("runtime production record must be a JSON object")
        if (record.get("schema") != SCHEMA or record.get("phase") != "build"
                or record.get("exit_code") != 0 or record.get("source_stable") is not True
                or record.get("cwd") != str(root)):
            _fail("record is not a successful, source-stable runtime-only production build")
        if report.get("passed") is not True or report.get("source_stable") is not True:
            _fail("comparison report must have passed=true and source_stable=true")
        if not isinstance(metadata, dict):
            _fail("comparison metadata is missing")
        settings = metadata.get("settings")
        mode = settings.get("candidate_mode") if isinstance(settings, dict) else None
        if mode not in MODES or settings.get("instrumented") is not False:
            _fail("comparison does not declare a supported uninstrumented candidate mode")
        if (record.get("mode") != mode or record.get("instrumented") is not False
                or record.get("harness_features") != MODES[mode]["harness_features"]
                or record.get("runtime_features") != MODES[mode]["runtime_features"]
                or record.get("feature_flags") != MODES[mode]["runtime_features"]
                or record.get("cargo_features") != MODES[mode]["cargo_features"]):
            _fail("record feature declarations differ from its exact production mode")

        source_identities = _source_map(record.get("source_identities"), "source_identities")
        candidate_sources = _source_map(metadata.get("candidate_sources"), "comparison candidate_sources")
        if source_identities != candidate_sources:
            _fail("record source_identities differ from the paired comparison source map")
        core_sources = _source_map(record.get("core_source_map"), "core_source_map")
        expected_core = {name: sha for name, sha in candidate_sources.items()
                         if any(name.startswith(f"crates/{crate}/src/") for crate in CORE_CRATES)}
        if (not expected_core or any(not any(name.startswith(f"crates/{crate}/src/")
                                              for name in expected_core) for crate in CORE_CRATES)
                or core_sources != expected_core):
            _fail("core_source_map differs from the four core source subsets in the comparison")
        runtime_sources = {name: sha for name, sha in candidate_sources.items()
                           if name.startswith("crates/wtime-runtime/src/")}
        if not runtime_sources or record.get("runtime_source_map") != runtime_sources:
            _fail("runtime_source_map differs from the comparison runtime source subset")

        archived = record.get("archived_sources")
        if not isinstance(archived, dict) or set(archived) != set(source_identities):
            _fail("archived source inventory differs from the complete source identity map")
        for name, expected_sha in source_identities.items():
            item = archived[name]
            if not isinstance(item, dict):
                _fail(f"archive entry {name} is malformed")
            source = _inside(item.get("source_path"), root, f"archive source {name}")
            if source.relative_to(root).as_posix() != name:
                _fail(f"archive source path does not match source identity {name}")
            archive = _inside(item.get("path"), output, f"archive output {name}")
            size = item.get("bytes")
            if (not archive.is_file() or item.get("sha256") != expected_sha
                    or _sha(archive) != expected_sha or isinstance(size, bool)
                    or not isinstance(size, int) or archive.stat().st_size != size):
                _fail(f"archived source {name} does not match its immutable identity")

        workspace_manifest_ref = record.get("workspace_manifest")
        workspace_lock_ref = record.get("workspace_lock")
        manifest_output, manifest_sha = _file_ref(workspace_manifest_ref, output, "workspace manifest")
        lock_output, lock_sha = _file_ref(workspace_lock_ref, output, "workspace lock")
        if manifest_output != (output / "compiled/Cargo.toml").resolve(strict=True):
            _fail("workspace manifest archive must be compiled/Cargo.toml")
        if lock_output != (output / "compiled/Cargo.lock").resolve(strict=True):
            _fail("workspace lock archive must be compiled/Cargo.lock")
        workspace_manifest = _inside(workspace_manifest_ref.get("workspace_path"), root,
                                     "workspace manifest source", must_exist=False)
        workspace_lock = _inside(workspace_lock_ref.get("workspace_path"), root,
                                  "workspace lock source", must_exist=False)
        if workspace_manifest_ref.get("workspace_path") != str(workspace_manifest.parent / "Cargo.toml"):
            _fail("workspace manifest source reference is malformed")
        if workspace_lock != workspace_manifest.parent / "Cargo.lock":
            _fail("workspace lock and manifest do not refer to the same wrapper workspace")
        if _sha(manifest_output) != manifest_sha or _sha(lock_output) != lock_sha:
            _fail("archived Cargo inputs changed")
        expected_manifest = _manifest_bytes(root, mode)
        if manifest_output.read_bytes() != expected_manifest:
            _fail("generated wrapper manifest does not match the safe protocol wrapper contract")
        try:
            parsed_manifest = tomllib.loads(manifest_output.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
            _fail(f"generated wrapper manifest is invalid TOML: {exc}")
        if parsed_manifest.get("package", {}).get("name") != "wtime-production-protocol":
            _fail("generated wrapper package name is unexpected")

        compiled = record.get("compiled_inputs")
        if not isinstance(compiled, dict):
            _fail("compiled_inputs inventory is missing")
        expected_compiled = {name for name in source_identities
                             if name.startswith("tools/wtime-bench/src/") and name.endswith(".rs")
                             and name != "tools/wtime-bench/src/main.rs"}
        expected_compiled |= {"generated:Cargo.toml", "generated:Cargo.lock", "generated:src/main.rs"}
        if set(compiled) != expected_compiled:
            _fail("compiled_inputs membership differs from the source-bound protocol harness")
        for name, item in compiled.items():
            if not isinstance(item, dict):
                _fail(f"compiled input {name} is malformed")
            expected_sha = _digest(item.get("sha256"), f"compiled input {name} SHA")
            archive = _inside(item.get("archive_path"), output, f"compiled archive {name}")
            if not archive.is_file() or _sha(archive) != expected_sha:
                _fail(f"compiled archive {name} does not match its recorded SHA")
            workspace_path = _inside(item.get("workspace_path"), root,
                                     f"compiled workspace input {name}", must_exist=False)
            if name == "generated:Cargo.toml":
                if archive != manifest_output or workspace_path != workspace_manifest or archive.read_bytes() != expected_manifest:
                    _fail("compiled wrapper manifest archive is inconsistent")
            elif name == "generated:Cargo.lock":
                if archive != lock_output or workspace_path != workspace_lock:
                    _fail("compiled wrapper lock archive is inconsistent")
            elif name == "generated:src/main.rs":
                if (archive != (output / "compiled/src/main.rs").resolve(strict=True)
                        or workspace_path != workspace_manifest.parent / "src/main.rs"
                        or archive.read_bytes() != ENTRYPOINT or expected_sha != _sha_bytes(ENTRYPOINT)):
                    _fail("generated safe protocol entrypoint does not match the required bytes")
            else:
                if (name not in source_identities or expected_sha != source_identities[name]
                        or archive.read_bytes() != _inside(str(output / "source" / name), output,
                                                           f"compiled source archive {name}").read_bytes()
                        or workspace_path != workspace_manifest.parent / "src" / Path(name).relative_to("tools/wtime-bench/src")):
                    _fail(f"compiled harness source {name} is inconsistent with its archived identity")
        compiled_manifest = record.get("compiled_manifest")
        cm_path, cm_sha = _file_ref(compiled_manifest, output, "compiled manifest")
        if cm_path != manifest_output or cm_sha != manifest_sha:
            _fail("compiled_manifest alias differs from the archived wrapper manifest")

        source_manifest_path, source_manifest_sha = _file_ref(record.get("source_manifest"), output, "source manifest")
        source_manifest = _json(source_manifest_path, "source manifest")
        if (source_manifest.get("source_identities") != source_identities
                or source_manifest.get("archived_sources") != archived
                or source_manifest.get("core_source_map") != core_sources
                or source_manifest.get("compiled_inputs") != compiled):
            _fail("final source_manifest contents differ from the production record")
        compilation_map = source_manifest.get("core_compilation_map")
        if not isinstance(compilation_map, dict):
            _fail("core compilation map is missing")
        expected_compilation = dict(core_sources)
        for crate in CORE_CRATES:
            manifest_name = f"crates/{crate}/Cargo.toml"
            digest = compilation_map.get(manifest_name)
            _digest(digest, f"core compilation map {manifest_name}")
            expected_compilation[manifest_name] = digest
        if compilation_map != expected_compilation:
            _fail("core compilation map must contain exactly the core sources and four manifests")

        binary = _inside(record.get("binary_path"), output, "production binary")
        binary_sha = _digest(record.get("binary_sha256"), "production binary SHA")
        if not binary.is_file() or _sha(binary) != binary_sha:
            _fail("production binary does not match its recorded SHA")
        if metadata.get("candidate_binary_sha256") != binary_sha:
            _fail("production binary SHA differs from the paired comparison")
        helper = _inside(record.get("helper_path"), output, "executed helper")
        helper_sha = _digest(record.get("helper_sha256"), "executed helper SHA")
        if not helper.is_file() or _sha(helper) != helper_sha:
            _fail("executed helper source does not match its recorded SHA")

        protocol_path = _inside(output / "protocol.stdout.json", output, "protocol JSON output")
        protocol_sha = _digest(record.get("protocol_sha256"), "protocol output SHA")
        if not protocol_path.is_file() or _sha(protocol_path) != protocol_sha:
            _fail("retained protocol JSON output SHA does not match")
        protocol = _json(protocol_path, "retained protocol output")
        caps = metadata.get("capabilities")
        reported_protocol = caps.get("candidate") if isinstance(caps, dict) else None
        if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
                or mode not in protocol.get("modes", []) or protocol.get("instrumented") is not False
                or protocol != record.get("protocol") or protocol != reported_protocol):
            _fail("retained protocol does not exactly match uninstrumented comparison capability")
        if ("register_entry_threshold" in settings
                or "execution_settings" in report.get("declaration", {})):
            helper = root / "tools/wtime-bench/p140_entry_policy.py"
            spec = importlib.util.spec_from_file_location("production_entry_policy", helper)
            if spec is None or spec.loader is None:
                _fail("adaptive entry policy helper is missing")
            entry_policy = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(entry_policy)
            frozen = entry_policy.declaration(report.get("declaration"))
            if frozen is None:
                _fail("adaptive entry production setting differs from its frozen declaration")
            entry_policy.validate_metadata(settings.get("register_entry_threshold"), frozen)
            entry_policy.request(report["declaration"], mode, frozen["candidate"])
            entry_policy.capability(protocol.get("register_entry_threshold_contract"))
        protocol_stdout = _inside(record.get("protocol_stdout"), output, "protocol stdout")
        protocol_stderr = _inside(record.get("protocol_stderr"), output, "protocol stderr")
        if not protocol_stdout.is_file() or protocol_stdout.read_bytes() != protocol_path.read_bytes():
            _fail("protocol stdout log differs from the retained protocol JSON")
        if not protocol_stderr.is_file() or protocol_stderr.read_bytes():
            _fail("successful protocol probe must have an empty stderr log")

        logs = _check_command_logs(record, output, root, mode, workspace_manifest, binary)
        build_stdout = logs["files"].get("production-build.stdout")
        build_stderr = logs["files"].get("production-build.stderr")
        stdout_copy = _inside(record.get("stdout_log", {}).get("path") if isinstance(record.get("stdout_log"), dict) else None,
                              output, "build stdout copy")
        stderr_copy = _inside(record.get("stderr_log", {}).get("path") if isinstance(record.get("stderr_log"), dict) else None,
                              output, "build stderr copy")
        if (not build_stdout or not build_stderr or not stdout_copy.is_file() or not stderr_copy.is_file()
                or stdout_copy.read_bytes() != build_stdout.read_bytes()
                or stderr_copy.read_bytes() != build_stderr.read_bytes()
                or _sha(stdout_copy) != record.get("stdout_sha256")
                or _sha(stderr_copy) != record.get("stderr_sha256")
                or record["stdout_log"].get("sha256") != _sha(stdout_copy)
                or record["stderr_log"].get("sha256") != _sha(stderr_copy)):
            _fail("retained build stdout/stderr copies do not match their command logs")

        metadata_refs = record.get("metadata")
        if not isinstance(metadata_refs, dict):
            _fail("focused Cargo metadata references are missing")
        before_path = _inside(metadata_refs.get("path"), output, "metadata before stdout")
        after_path = _inside(metadata_refs.get("after_path"), output, "metadata after stdout")
        if (_sha(before_path) != _digest(metadata_refs.get("sha256"), "metadata before SHA")
                or _sha(after_path) != _digest(metadata_refs.get("after_sha256"), "metadata after SHA")):
            _fail("focused Cargo metadata output hash is stale")
        before_meta, after_meta = _json(before_path, "focused Cargo metadata before"), _json(after_path, "focused Cargo metadata after")
        if before_meta != after_meta:
            _fail("focused Cargo metadata changed during the production build")
        summary = _metadata_summary(before_meta, root, workspace_manifest, mode)
        if metadata_refs.get("summary") != summary or record.get("core_resolution") != summary["resolution"]:
            _fail("focused Cargo metadata summary or core resolution differs from its JSON")

        toolchain = record.get("toolchain")
        if not isinstance(toolchain, dict) or toolchain.get("before") != toolchain.get("after"):
            _fail("toolchain identity changed during the production build")
        for suffix in ("before", "after"):
            tools = toolchain.get(suffix)
            if not isinstance(tools, dict) or set(tools) != {"cargo", "rustc"}:
                _fail("toolchain identity must contain Cargo and rustc")
            for name, row in tools.items():
                if not isinstance(row, dict):
                    _fail(f"{name} tool identity is malformed")
                exe = Path(row.get("path", "")).resolve(strict=True)
                if not exe.is_file() or _sha(exe) != _digest(row.get("sha256"), f"{name} executable SHA"):
                    _fail(f"{name} executable identity is invalid")
                version_log = logs["files"].get(f"{name}-version-{suffix}.stdout")
                stderr_log = logs["files"].get(f"{name}-version-{suffix}.stderr")
                if (version_log is None or stderr_log is None or version_log.read_text(encoding="utf-8") != row.get("version")
                        or _sha(version_log) != row.get("version_sha256")
                        or _sha(stderr_log) != row.get("stderr_sha256")):
                    _fail(f"{name} version evidence does not match its retained output")

        for suffix in ("before", "after"):
            ref = toolchain.get(f"root_metadata_{suffix}")
            path = _inside(ref.get("path") if isinstance(ref, dict) else None, output,
                           f"root Cargo metadata {suffix}")
            if not isinstance(ref, dict) or _sha(path) != _digest(ref.get("sha256"), f"root metadata {suffix} SHA"):
                _fail(f"root Cargo metadata {suffix} output SHA does not match")
            value = _json(path, f"root Cargo metadata {suffix}")
            if ref.get("summary") != _root_metadata_summary(value, root):
                _fail(f"root Cargo metadata {suffix} summary is inconsistent")
            if suffix == "before":
                root_metadata_before = value
            else:
                root_metadata_after = value
        if root_metadata_before != root_metadata_after:
            _fail("root Cargo metadata changed during the production build")

        cache_path, cache_sha = _file_ref(record.get("cache_manifest"), output, "cache manifest")
        cache = _json(cache_path, "cache manifest")
        compiled_cache = {name: {key: value for key, value in item.items() if key != "archive_path"}
                          for name, item in compiled.items()}
        tool_identity = {name: {key: row[key] for key in ("path", "sha256", "version_sha256")}
                         for name, row in toolchain["before"].items()}
        build_log = _json(logs["files"]["production-build.command.json"], "production build command")
        environment_sha = _digest(build_log.get("environment_sha256"), "build environment SHA")
        input_key_inputs = {"mode": mode, "sources": source_identities,
                            "core": source_manifest.get("core_compilation_map"),
                            "compiled": compiled_cache, "features": MODES[mode],
                            "tools": tool_identity,
                            "environment_sha256": environment_sha,
                            "target": cache.get("target"), "profile": "release", "jobs": 4}
        if (cache.get("schema") != "wtime-runtime-protocol-cache-v1"
                or cache.get("input_key") != record.get("input_key")
                or cache.get("mode") != mode or cache.get("workspace") != str(workspace_manifest.parent)
                or cache.get("target") != str(workspace_manifest.parent / "target")
                or cache.get("source_identities") != source_identities
                or cache.get("core_source_map") != core_sources
                or cache.get("core_compilation_map") != source_manifest.get("core_compilation_map")
                or cache.get("compiled_inputs") != compiled_cache or cache.get("features") != MODES[mode]
                or cache.get("cargo_metadata_sha256") != _canonical_hash(before_meta)
                or cache.get("cargo_lock_sha256") != lock_sha
                or cache.get("tools") != toolchain["before"]
                or _canonical_hash(input_key_inputs) != record.get("input_key")):
            _fail("cache manifest fields or reproducible input key differ from immutable build inputs")

        return {"record": {"path": str(record_file), "sha256": _sha(record_file),
                           "bytes": record_file.stat().st_size},
                "command_features": MODES[mode]["runtime_features"],
                "runtime_features": [f"wtime-runtime/{feature}"
                                     for feature in MODES[mode]["runtime_features"]],
                "binary_sha256": binary_sha,
                "runtime_source_sha256": _sha_bytes(json.dumps(
                    runtime_sources, sort_keys=True, separators=(",", ":")).encode()),
                "protocol": protocol, "protocol_sha256": protocol_sha,
                "lane": "runtime-only-production"}
    except AttestationError:
        raise
    except (OSError, KeyError, TypeError, ValueError, IndexError) as exc:
        raise AttestationError(f"invalid runtime production record: {exc}") from exc
