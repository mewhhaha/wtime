#!/usr/bin/env python3
"""Validate and replay P045 compact fuzz records against Node's Wasm engine."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any

SCHEMA = "p045-corpus-v1"
PREFIX_CASE = "P045_CASE "
PREFIX_SUMMARY = "P045_SUMMARY "
FAMILIES = {"scalar", "local", "if", "branch", "memory", "trap"}
MAX_MODULE_BYTES = 1 << 20
NODE_BATCH = r'''
const fs = require('fs');
const crypto = require('crypto');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const results = [];
const instances = new Map();
for (const c of input.cases) {
  try {
    let item = instances.get(c.module_hex);
    if (!item) {
      const bytes = Buffer.from(c.module_hex, 'hex');
      const module = new WebAssembly.Module(bytes);
      const instance = new WebAssembly.Instance(module, {});
      const memoryName = c.memory_export || 'memory';
      const memory = instance.exports[memoryName];
      const initialBytes = memory instanceof WebAssembly.Memory ? memory.buffer.byteLength : null;
      item = { instance, memory, initialBytes };
      instances.set(c.module_hex, item);
    }
    if (item.memory instanceof WebAssembly.Memory && item.initialBytes !== null) {
      if (item.memory.buffer.byteLength !== item.initialBytes) {
        results.push({case_index:c.case_index, error:'memory size changed across calls'}); continue;
      }
      new Uint8Array(item.memory.buffer).fill(0);
    }
    const fn = item.instance.exports[c.export];
    if (typeof fn !== 'function') throw new Error('missing function export: ' + c.export);
    let expectedMemory = null;
    if (c.expected_memory) {
      expectedMemory = Buffer.alloc(c.expected_memory.bytes);
      for (const write of c.expected_memory.writes) {
        Buffer.from(write.data_hex, 'hex').copy(expectedMemory, write.offset);
      }
    }
    let outcome;
    try { outcome = {kind:'value', value:fn(c.input_i32)}; }
    catch (e) {
      if (e instanceof WebAssembly.RuntimeError) outcome = {kind:'trap', name:e.name, message:e.message};
      else throw e;
    }
    const memory = item.memory;
    let memoryEqual = null;
    let memorySha256 = null;
    if (expectedMemory) {
      if (!(memory instanceof WebAssembly.Memory)) throw new Error('expected memory export is missing');
      const image = Buffer.from(memory.buffer);
      memoryEqual = image.equals(expectedMemory);
      memorySha256 = crypto.createHash('sha256').update(image).digest('hex');
    }
    results.push({case_index:c.case_index, outcome, memory_equal:memoryEqual, memory_sha256:memorySha256});
  } catch (e) {
    results.push({case_index:c.case_index, error:String(e && e.message || e)});
  }
}
process.stdout.write(JSON.stringify({results}));
'''


class CorpusError(RuntimeError):
    pass


def need(ok: bool, message: str) -> None:
    if not ok:
        raise CorpusError(message)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_uleb(data: bytes, offset: int, limit: int) -> tuple[int, int]:
    """Read one bounded unsigned LEB128 value restricted to u32."""
    value = 0
    for index in range(5):
        need(offset < limit, "truncated Wasm unsigned LEB128")
        byte = data[offset]
        offset += 1
        if index == 4:
            need((byte & 0xF0) == 0, "Wasm unsigned LEB128 exceeds u32")
        value |= (byte & 0x7F) << (7 * index)
        if byte < 0x80:
            return value, offset
    raise CorpusError("Wasm unsigned LEB128 exceeds five bytes")


def _vector_count(data: bytes, offset: int, limit: int) -> tuple[int, int]:
    count, offset = read_uleb(data, offset, limit)
    return count, offset


def exported_body(module: bytes, export_name: str) -> bytes:
    """Return an exported function's exact Code-section body payload."""
    need(len(module) <= MAX_MODULE_BYTES, "module_hex exceeds the 1 MiB verifier bound")
    need(module.startswith(b"\0asm\x01\0\0\0"), "invalid Wasm header")
    sections: dict[int, bytes] = {}
    offset = 8
    while offset < len(module):
        section_id = module[offset]
        offset += 1
        section_size, offset = read_uleb(module, offset, len(module))
        end = offset + section_size
        need(end <= len(module), "Wasm section exceeds module bounds")
        if section_id != 0:
            need(section_id not in sections, f"duplicate Wasm section {section_id}")
            sections[section_id] = module[offset:end]
        offset = end

    imports = sections.get(2)
    if imports is not None:
        count, at = _vector_count(imports, 0, len(imports))
        need(count == 0 and at == len(imports), "imported functions are unsupported by P045 body binding")

    function_section = sections.get(3)
    code_section = sections.get(10)
    export_section = sections.get(7)
    need(function_section is not None and code_section is not None and export_section is not None,
         "Wasm module is missing function, code, or export section")

    function_count, at = _vector_count(function_section, 0, len(function_section))
    for _ in range(function_count):
        _, at = read_uleb(function_section, at, len(function_section))
    need(at == len(function_section), "malformed function section payload")

    code_count, at = _vector_count(code_section, 0, len(code_section))
    bodies: list[bytes] = []
    for _ in range(code_count):
        body_size, at = read_uleb(code_section, at, len(code_section))
        body_end = at + body_size
        need(body_end <= len(code_section), "function body exceeds code section bounds")
        bodies.append(code_section[at:body_end])
        at = body_end
    need(at == len(code_section) and len(bodies) == function_count,
         "function and code section counts differ or code payload is malformed")

    export_count, at = _vector_count(export_section, 0, len(export_section))
    function_index = None
    names: set[str] = set()
    for _ in range(export_count):
        name_size, at = read_uleb(export_section, at, len(export_section))
        name_end = at + name_size
        need(name_end <= len(export_section), "Wasm export name exceeds section bounds")
        try:
            name = export_section[at:name_end].decode("utf-8")
        except UnicodeDecodeError as exc:
            raise CorpusError("Wasm export name is not UTF-8") from exc
        at = name_end
        need(at < len(export_section), "truncated Wasm export kind")
        kind = export_section[at]
        at += 1
        index, at = read_uleb(export_section, at, len(export_section))
        need(name not in names, f"duplicate Wasm export name {name!r}")
        names.add(name)
        if name == export_name:
            need(kind == 0, f"export {export_name!r} is not a function")
            function_index = index
    need(at == len(export_section), "malformed export section payload")
    need(function_index is not None and function_index < len(bodies),
         f"exported function {export_name!r} has no code body")
    return bodies[function_index]


def parse_log(path: Path, min_cases: int = 1000) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    summaries: list[dict[str, Any]] = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.startswith(PREFIX_CASE):
            try:
                value = json.loads(line[len(PREFIX_CASE):])
            except json.JSONDecodeError as exc:
                raise CorpusError(f"line {line_no}: invalid case JSON: {exc}") from exc
            need(isinstance(value, dict), f"line {line_no}: case row must be an object")
            cases.append(value)
        elif line.startswith(PREFIX_SUMMARY):
            try:
                value = json.loads(line[len(PREFIX_SUMMARY):])
            except json.JSONDecodeError as exc:
                raise CorpusError(f"line {line_no}: invalid summary JSON: {exc}") from exc
            need(isinstance(value, dict), f"line {line_no}: summary row must be an object")
            summaries.append(value)
    need(len(summaries) == 1, f"expected one P045_SUMMARY row, found {len(summaries)}")
    summary = summaries[0]
    need(summary.get("schema") == SCHEMA, "summary schema must be p045-corpus-v1")
    need(type(summary.get("case_count")) is int and summary["case_count"] == len(cases),
         "summary case_count does not match P045_CASE rows")
    need(len(cases) >= min_cases, f"need at least {min_cases} cases, found {len(cases)}")
    seen: set[int] = set()
    module_bodies: dict[tuple[str, str], bytes] = {}
    for index, row in enumerate(cases):
        prefix = f"case row {index}"
        need(row.get("schema") == SCHEMA, f"{prefix}: schema must be p045-corpus-v1")
        for field in ("case_index", "seed", "body_hex", "module_hex", "input_i32", "family",
                      "events", "policy_count", "fuel_probes", "compact_steps", "expected_kind", "export"):
            need(field in row, f"{prefix}: missing {field}")
        ci = row["case_index"]
        need(type(ci) is int and ci >= 0, f"{prefix}: case_index must be a nonnegative integer")
        need(ci not in seen, f"duplicate case_index {ci}")
        seen.add(ci)
        need(type(row["seed"]) is int and row["seed"] == 20445, f"{prefix}: seed must be exactly 20445")
        for field in ("body_hex", "module_hex"):
            value = row[field]
            need(isinstance(value, str) and len(value) > 0 and len(value) % 2 == 0,
                 f"{prefix}: {field} must be nonempty even-length hex")
            need(all(ch in "0123456789abcdefABCDEF" for ch in value),
                 f"{prefix}: {field} contains non-hex characters")
            try:
                raw = bytes.fromhex(value)
            except ValueError as exc:
                raise CorpusError(f"{prefix}: {field} is invalid hex") from exc
            if field == "module_hex":
                need(raw.startswith(b"\0asm\x01\0\0\0"), f"{prefix}: module_hex has invalid Wasm header")
                need(len(raw) <= MAX_MODULE_BYTES, f"{prefix}: module_hex exceeds the 1 MiB verifier bound")
                module_bytes = raw
        val = row["input_i32"]
        need(type(val) is int and -(1 << 31) <= val < (1 << 31), f"{prefix}: input_i32 is out of range")
        need(isinstance(row["family"], str) and row["family"] in FAMILIES,
             f"{prefix}: invalid family {row['family']!r}")
        need(isinstance(row["expected_kind"], str) and row["expected_kind"] in {"returned", "trap"},
             f"{prefix}: expected_kind must be returned or trap")
        need((row["family"] == "trap") == (row["expected_kind"] == "trap"),
             f"{prefix}: family and expected_kind disagree")
        need(isinstance(row["export"], str) and bool(row["export"]), f"{prefix}: export must be nonempty text")
        body_key = (row["module_hex"], row["export"])
        if body_key not in module_bodies:
            module_bodies[body_key] = exported_body(module_bytes, row["export"])
        need(module_bodies[body_key] == bytes.fromhex(row["body_hex"]),
             f"{prefix}: body_hex does not match exported function code body")
        for field in ("events", "policy_count", "fuel_probes", "compact_steps"):
            need(type(row[field]) is int and row[field] > 0, f"{prefix}: {field} must be positive")
        need(row["policy_count"] == 3, f"{prefix}: policy_count must be 3")
        need(row["fuel_probes"] >= row["events"] + 2, f"{prefix}: fuel_probes must cover events + 2")
        if row["expected_kind"] == "returned":
            need("expected_i32" in row, f"{prefix}: returned cases require expected_i32")
            expected = row["expected_i32"]
            need(type(expected) is int and -(1 << 31) <= expected < (1 << 31),
                 f"{prefix}: expected_i32 is out of range")
        else:
            need("expected_i32" not in row, f"{prefix}: trap cases cannot carry expected_i32")
        if row["family"] == "memory":
            need(row["expected_kind"] == "returned", f"{prefix}: memory family must return")
            expected_memory = row.get("expected_memory")
            need(isinstance(expected_memory, dict) and set(expected_memory) == {"bytes", "writes"},
                 f"{prefix}: memory family requires expected_memory bytes/writes")
            need(type(expected_memory["bytes"]) is int and expected_memory["bytes"] == 65536,
                 f"{prefix}: expected_memory.bytes must be 65536")
            need(isinstance(expected_memory["writes"], list), f"{prefix}: expected_memory.writes must be an array")
            for write in expected_memory["writes"]:
                need(isinstance(write, dict) and set(write) == {"offset", "data_hex"},
                     f"{prefix}: memory write must have offset/data_hex")
                need(type(write["offset"]) is int and 0 <= write["offset"] <= 65532,
                     f"{prefix}: memory write offset is out of bounds")
                need(isinstance(write["data_hex"], str) and len(write["data_hex"]) == 8,
                     f"{prefix}: memory write data_hex must encode four bytes")
                need(all(ch in "0123456789abcdefABCDEF" for ch in write["data_hex"]),
                     f"{prefix}: memory write data_hex contains non-hex characters")
                try:
                    bytes.fromhex(write["data_hex"])
                except ValueError as exc:
                    raise CorpusError(f"{prefix}: memory write data_hex is invalid hex") from exc
        else:
            need("expected_memory" not in row, f"{prefix}: non-memory case cannot carry expected_memory")
    need(len(seen) == len(cases), "case indices are not unique")
    return cases, summary


def check_node_results(cases: list[dict[str, Any]], results: list[dict[str, Any]]) -> dict[str, Any]:
    need(len(results) == len(cases), "Node oracle returned a different case count")
    by_index = {row["case_index"]: row for row in results}
    need(len(by_index) == len(results), "Node oracle returned duplicate case indices")
    values = traps = memory_checks = 0
    memory_hashes = []
    for case in cases:
        got = by_index.get(case["case_index"])
        need(got is not None, f"Node oracle omitted case {case['case_index']}")
        need("error" not in got, f"Node oracle failed case {case['case_index']}: {got.get('error')}")
        outcome = got.get("outcome")
        if case["expected_kind"] == "trap":
            need(isinstance(outcome, dict) and outcome.get("kind") == "trap",
                 f"Node oracle expected a trap in case {case['case_index']}")
            traps += 1
        else:
            need(isinstance(outcome, dict) and outcome.get("kind") == "value",
                 f"Node oracle expected a value in case {case['case_index']}")
            expected = case["expected_i32"]
            value = outcome.get("value")
            need(type(value) is int and ((value + (1 << 31)) % (1 << 32) - (1 << 31)) == expected,
                 f"Node result mismatch in case {case['case_index']}: expected {expected}, got {value}")
            values += 1
        if "expected_memory" in case:
            expected_memory = case["expected_memory"]
            image = bytearray(expected_memory["bytes"])
            for write in expected_memory["writes"]:
                offset = write["offset"]
                image[offset:offset + 4] = bytes.fromhex(write["data_hex"])
            expected_sha256 = sha256_bytes(image)
            need(got.get("memory_equal") is True,
                 f"Node memory mismatch in case {case['case_index']}")
            got_sha256 = got.get("memory_sha256")
            need(isinstance(got_sha256, str) and got_sha256 == expected_sha256,
                 f"Node memory SHA-256 mismatch in case {case['case_index']}")
            memory_hashes.append({"case_index": case["case_index"], "sha256": got_sha256})
            memory_checks += 1
    return {"value_cases": values, "trap_cases": traps, "memory_checks": memory_checks,
            "memory_sha256": memory_hashes}


def contained(path: Path, root: Path) -> Path:
    resolved = path.resolve()
    need(resolved.is_relative_to(root.resolve()), f"output path escapes report directory: {path}")
    return resolved


def execute(rust_log: Path, output: Path, node: str = "node", min_cases: int = 1000) -> dict[str, Any]:
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report_path = contained(output / "report.json", output)
    helper_path = Path(__file__).resolve()
    source_sha256 = sha256_file(helper_path)
    # Preserve executable helper and exact invocation before parsing or launching Node.
    archived_helper = contained(output / helper_path.name, output)
    shutil.copy2(helper_path, archived_helper)
    need(sha256_file(archived_helper) == source_sha256, "archived helper source changed during preservation")
    command = [sys.executable, "-B", str(helper_path), "--rust-log", str(rust_log.resolve()), "--output", str(output), "--node", node]
    command_record = {
        "argv": command,
        "node_version_argv": [node, "--version"],
        "node_batch_argv": [node, "-e", NODE_BATCH],
        "node_batch_sha256": sha256_bytes(NODE_BATCH.encode("utf-8")),
    }
    (contained(output / "command.json", output)).write_text(json.dumps(command_record, indent=2) + "\n")
    report: dict[str, Any] = {
        "schema": SCHEMA,
        "source": {"path": str(helper_path), "sha256": source_sha256},
        "rust_log": {"path": str(rust_log.resolve()), "sha256": sha256_file(rust_log), "bytes": rust_log.stat().st_size},
        "node": {"executable": node, "path": None, "sha256": None, "version": None},
        "tested": {"case_count": 0, "unique_cases": 0, "unique_modules": 0},
        "checks": {}, "errors": [],
    }
    try:
        cases, summary = parse_log(rust_log, min_cases=min_cases)
        family_counts = {family: sum(1 for row in cases if row["family"] == family)
                         for family in sorted(FAMILIES)}
        report["tested"].update(case_count=len(cases), unique_cases=len({x["case_index"] for x in cases}),
                                 unique_modules=len({x["module_hex"] for x in cases}))
        report["family_counts"] = family_counts
        report["families"] = sorted(family_counts)
        report["module_sha256"] = sorted({sha256_bytes(bytes.fromhex(x["module_hex"])) for x in cases})
        report["summary"] = summary
        binary_path = summary.get("binary_path")
        need(isinstance(binary_path, str) and bool(binary_path), "summary binary_path is required")
        binary_path = Path(binary_path).resolve(strict=True)
        need(binary_path.is_file(), "summary binary_path is not a file")
        report["rust_binary"] = {"path": str(binary_path), "sha256": sha256_file(binary_path),
                                 "bytes": binary_path.stat().st_size}
        need(report["tested"]["unique_cases"] == len(cases), "case indices are not unique")
        node_path = shutil.which(node)
        need(node_path is not None, f"Node executable not found: {node}")
        node_path = Path(node_path).resolve(strict=True)
        node_sha256 = sha256_file(node_path)
        report["node"].update(path=str(node_path), sha256=node_sha256,
                              bytes=node_path.stat().st_size)
        version = subprocess.run([node, "--version"], check=True, capture_output=True, text=True)
        report["node"]["version"] = version.stdout.strip()
        proc = subprocess.run([node, "-e", NODE_BATCH], input=json.dumps({"cases": cases}),
                              capture_output=True, text=True)
        report["node"]["exit_code"] = proc.returncode
        report["node"]["stderr"] = proc.stderr
        identities_stable = (sha256_file(helper_path) == source_sha256
                             and sha256_file(binary_path) == report["rust_binary"]["sha256"]
                             and sha256_file(node_path) == node_sha256)
        report["identity_stable"] = identities_stable
        need(identities_stable, "helper, Rust binary, or Node binary identity changed during oracle run")
        need(proc.returncode == 0, f"Node oracle failed with exit code {proc.returncode}")
        try:
            node_result = json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            raise CorpusError(f"Node oracle emitted invalid JSON: {exc}") from exc
        node_rows = node_result.get("results", [])
        report["node_result_summary"] = {
            "case_count": len(cases),
            "value_cases": sum(1 for row in node_rows
                                if isinstance(row.get("outcome"), dict)
                                and row["outcome"].get("kind") == "value"),
            "trap_cases": sum(1 for row in node_rows
                              if isinstance(row.get("outcome"), dict)
                              and row["outcome"].get("kind") == "trap"),
            "memory_checks": sum(1 for row in node_rows if row.get("memory_sha256") is not None),
            "memory_sha256": [
                {"case_index": row.get("case_index"), "sha256": row.get("memory_sha256")}
                for row in node_rows if row.get("memory_sha256") is not None
            ],
        }
        counts = check_node_results(cases, node_rows)
        report["checks"] = {"schema": True, "summary_count": True, "seed": True, "unique_indices": True,
                            "body_binding": True, "replay": True, "external_oracle": True,
                            "identity_stable": True, "family_counts": True,
                            "value_cases": counts["value_cases"], "trap_cases": counts["trap_cases"],
                            "memory_checks": counts["memory_checks"]}
        report["status"] = "passed"
    except (CorpusError, OSError, subprocess.SubprocessError) as exc:
        report["status"] = "failed"
        report["errors"].append(str(exc))
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-log", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new exclusive report directory")
    parser.add_argument("--node", default="node")
    args = parser.parse_args(argv)
    try:
        report = execute(args.rust_log, args.output, args.node)
    except (CorpusError, FileExistsError, OSError) as exc:
        print(f"p045-corpus: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"status": report["status"], "report": str(args.output.resolve() / "report.json")}))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
