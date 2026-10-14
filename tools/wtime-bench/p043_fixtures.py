#!/usr/bin/env python3
"""Create fresh, validated P043 Wasm fixtures; inert unless --execute is set."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys

FIXTURES = ("access-boundaries", "custom-page-one-byte", "growth-caller",
            "memory64-overflow", "scalar-widths", "shared-multi-memory")
EXPECTED = {f"{n}.wat" for n in FIXTURES} | {
    "LICENSE.txt", "PROVENANCE.json", "manual-oracles.json"
}
IMAGE_PREFIX = "expected-memory/store-kinds/"


class FixtureError(RuntimeError):
    pass


def need(ok: bool, message: str) -> None:
    if not ok:
        raise FixtureError(message)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def ref(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": digest(path), "bytes": path.stat().st_size}


def safe_packet_file(root: Path, rel: str) -> Path:
    p = PurePosixPath(rel)
    need(not p.is_absolute() and bool(p.parts) and all(x not in ("", ".", "..") for x in p.parts),
         f"unsafe packet path: {rel!r}")
    cur = root
    for part in p.parts:
        cur = cur / part
        need(not cur.is_symlink(), f"packet symlink rejected: {rel}")
    resolved = cur.resolve(strict=True)
    need(resolved.is_relative_to(root.resolve()) and resolved.is_file(), f"packet path escapes/is not a file: {rel}")
    return resolved


def packet_snapshot(source: Path, dest: Path) -> tuple[dict, dict]:
    root = source.resolve(strict=True)
    need(root.is_dir(), "packet must be a directory")
    sums = safe_packet_file(root, "SHA256SUMS")
    entries = {}
    for line in sums.read_text(encoding="utf-8").splitlines():
        m = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        need(m is not None, "malformed SHA256SUMS row")
        expected, rel = m.groups()
        need(rel not in entries, f"duplicate packet member: {rel}")
        path = safe_packet_file(root, rel)
        need(digest(path) == expected, f"SHA256SUMS mismatch: {rel}")
        entries[rel] = expected
    images = {x for x in entries if x.startswith(IMAGE_PREFIX)}
    need(len(images) == 30 and set(entries) == EXPECTED | images,
         "packet must contain exactly six WATs, license/provenance/manual and 30 images")
    # Disallow unmanifested content, including nested symlinks and special files.
    actual = set()
    for base, dirs, files in os.walk(root, followlinks=False):
        basepath = Path(base)
        for name in list(dirs):
            item = basepath / name
            need(not item.is_symlink(), f"packet symlink rejected: {item.relative_to(root)}")
        for name in files:
            item = basepath / name
            rel = item.relative_to(root).as_posix()
            need(not item.is_symlink() and stat.S_ISREG(item.lstat().st_mode), f"packet special member rejected: {rel}")
            if rel != "SHA256SUMS":
                actual.add(rel)
    need(actual == set(entries), "packet has missing or unmanifested members")
    copy_root = dest / "packet"
    for rel in sorted(set(entries) | {"SHA256SUMS"}):
        src = safe_packet_file(root, rel)
        target = copy_root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, target)
        need(digest(target) == digest(src), f"packet snapshot mismatch: {rel}")
        if rel in entries:
            need(digest(target) == entries[rel], f"packet changed during snapshot: {rel}")
    for rel in images:
        path = safe_packet_file(root, rel)
        need(path.stat().st_size == 65536, f"image is not a complete 65536-byte memory: {rel}")
    oracle = json.loads((copy_root / "manual-oracles.json").read_text(encoding="utf-8"))
    need(oracle.get("schema") == "p043-memory-source-oracles-v1" and
         oracle.get("status") == "draft_source_only_unencoded_unvalidated_unexecuted",
         "manual oracle schema/status mismatch")
    rows = oracle.get("fixtures", {}).get("scalar-widths.wat", {}).get("store_kinds", {}).get("fuel_images")
    need(isinstance(rows, list) and len(rows) == 30, "manual oracle must bind 30 fuel images")
    image_refs = {}
    for row in rows:
        rel = row.get("expected_memory_file")
        need(rel in images and row.get("memory_length") == 65536, "manual image reference/length mismatch")
        path = safe_packet_file(copy_root, rel)
        need(path.stat().st_size == 65536 and digest(path) == row.get("memory_sha256"),
             f"manual image digest mismatch: {rel}")
        image_refs[rel] = ref(path)
    need(set(image_refs) == images, "manual oracle does not reference every packet image")
    prov = json.loads((copy_root / "PROVENANCE.json").read_text(encoding="utf-8"))
    need(prov.get("schema") == "p043-fixture-source-packet-v1" and
         prov.get("status") == "draft_source_only_unencoded_unvalidated_unexecuted" and
         prov.get("image_oracles", {}).get("count") == 30 and
         prov.get("image_oracles", {}).get("directory") == "expected-memory/store-kinds",
         "packet provenance schema/status/image declaration mismatch")
    license_text = (copy_root / "LICENSE.txt").read_text(encoding="utf-8")
    need("MIT License" in license_text and "Apache License" in license_text, "dual license text missing")
    return ({rel: ref(copy_root / rel) for rel in sorted(entries)}, image_refs)


def source_map(root: Path) -> dict:
    base = root / "crates/wtime-runtime/src"
    return {p.relative_to(root).as_posix(): digest(p) for p in sorted(base.rglob("*.rs"))}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--packet", type=Path, required=True)
    ap.add_argument("--encoder", type=Path, required=True)
    ap.add_argument("--validator", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True, help="new, exclusive evidence directory")
    ap.add_argument("--execute", action="store_true", help="copy inputs, encode, and validate fixtures")
    args = ap.parse_args()
    if not args.execute:
        print("inert: pass --execute to snapshot, encode, and validate")
        return 0
    output = args.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2]
    try:
        shutil.copy2(__file__, output / 'helper.executed.py')
        before = source_map(root)
        need(len(before) == 29, f"expected 29 runtime source files, found {len(before)}")
        inputs = output / "inputs"
        inputs.mkdir()
        packet_rows, images = packet_snapshot(args.packet, inputs)
        tools = {}
        for name, path in (("encoder", args.encoder), ("validator", args.validator)):
            original = path.resolve(strict=True)
            need(original.is_file() and os.access(original, os.X_OK), f"{name} is not executable")
            saved = inputs / "tools" / name
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(original, saved)
            need(digest(saved) == digest(original), f"{name} snapshot hash mismatch")
            need(stat.S_IMODE(saved.stat().st_mode) == stat.S_IMODE(original.stat().st_mode), f"{name} mode mismatch")
            tools[name] = {**ref(saved), "mode": oct(stat.S_IMODE(saved.stat().st_mode)),
                           "original_path": str(original), "original_sha256": digest(original),
                           "original_mode": oct(stat.S_IMODE(original.stat().st_mode))}
        encoded = output / "fixtures"
        encoded.mkdir()
        fixture_rows = {}
        logs = output / "commands"
        logs.mkdir()
        for name in FIXTURES:
            source = inputs / "packet" / (name + ".wat")
            wasm = encoded / (name + ".wasm")
            cmds = {
                "encode": [str((inputs / "tools/encoder").resolve()), "encode", str(source), "-o", str(wasm)],
                "validate": [str((inputs / "tools/validator").resolve()), "--profile", "all-phase3-core", "check", str(wasm)],
            }
            command_refs = {}
            for kind, argv in cmds.items():
                try:
                    proc = subprocess.run(argv, cwd=root, text=True, capture_output=True, check=False,
                                          env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
                    record = {"argv": argv, "cwd": str(root), "exit_code": proc.returncode,
                              "stdout": proc.stdout, "stderr": proc.stderr}
                except Exception as exc:
                    record = {"argv": argv, "cwd": str(root), "exit_code": None,
                              "stdout": "", "stderr": "", "error": f"{type(exc).__name__}: {exc}"}
                log = logs / f"{name}-{kind}.json"
                log.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
                command_refs[kind] = {"argv": argv, "exit_code": record["exit_code"], "log": ref(log)}
                need(record["exit_code"] == 0, f"{name} {kind} failed; see {log}")
                if kind == "validate":
                    need(record["stdout"].strip() == "valid", f"{name} validator did not emit exact 'valid'")
            need(wasm.is_file() and wasm.stat().st_size > 8, f"missing/empty encoded Wasm: {name}")
            fixture_rows[name] = {"source": ref(source), "wasm": ref(wasm), "commands": command_refs,
                                  "manual_oracles": ref(inputs / "packet/manual-oracles.json"),
                                  "provenance": ref(inputs / "packet/PROVENANCE.json"),
                                  "license": ref(inputs / "packet/LICENSE.txt")}
        after = source_map(root)
        need(after == before, "runtime source map changed during fixture preparation")
        need(all(digest(Path(row["path"])) == row["sha256"] for row in tools.values()),
             "tool snapshot changed during preparation")
        receipt = {"schema": "p043-fresh-fixture-validation-v1", "status": "passed", "phase": "prepare",
                   "runtime_predecessor": before, "current_runtime_source_map": after,
                   "runtime_source_count": 29,
                   "runtime_source_sha256": hashlib.sha256(json.dumps(before, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                   "packet_manifest": ref(inputs / "packet/SHA256SUMS"), "packet_members": packet_rows,
                   "fixtures": fixture_rows, "tools": tools,
                   "manual_oracles": ref(inputs / "packet/manual-oracles.json"),
                   "provenance": ref(inputs / "packet/PROVENANCE.json"),
                   "licenses": {"LICENSE.txt": ref(inputs / "packet/LICENSE.txt")},
                   "image_oracles": images, "source_stable": True}
        receipt['helper'] = ref(output / 'helper.executed.py')
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"passed: {output / 'receipt.json'}")
        return 0
    except Exception as exc:
        (output / "failure.json").write_text(json.dumps({"status": "failed", "error": f"{type(exc).__name__}: {exc}"},
                                                          indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"failed: {exc}; artifacts retained in {output}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
