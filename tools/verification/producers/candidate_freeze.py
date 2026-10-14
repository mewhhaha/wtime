#!/usr/bin/env python3
"""Revalidate and retain an immutable pre-edit candidate selection receipt."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

import selection_manifest


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def revalidate(manifest: Path, predecessor: Path, frozen: Path, output: Path) -> dict:
    """Verify receipt bindings and copy the receipt byte-for-byte to output."""
    manifest = manifest.resolve(strict=True)
    predecessor = predecessor.resolve(strict=True)
    frozen = frozen.resolve(strict=True)
    output = output.resolve()

    # Resolve and validate every input before creating any output.
    receipt = json.loads(frozen.read_text(encoding="utf-8"))
    selection_manifest.verify_frozen(manifest, frozen)
    if Path(receipt.get("predecessor_path", "")).resolve(strict=True) != predecessor:
        raise ValueError("frozen receipt names a different predecessor path")
    if receipt.get("predecessor_sha256") != sha(predecessor):
        raise ValueError("frozen receipt predecessor hash differs")
    if output.exists():
        raise FileExistsError(f"refusing to overwrite freeze output: {output}")
    if not output.parent.is_dir():
        raise FileNotFoundError(f"freeze output directory does not exist: {output.parent}")

    payload = frozen.read_bytes()
    with output.open("xb") as stream:
        stream.write(payload)
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--predecessor", type=Path, required=True)
    parser.add_argument("--frozen", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    receipt = revalidate(args.manifest, args.predecessor, args.frozen, args.output)
    print(json.dumps({"status": "passed", "task": receipt.get("task"),
                      "predecessor_sha256": receipt["predecessor_sha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
