#!/usr/bin/env python3
"""Run the P128 fixed-alias correctness proof on supplied source-current binaries."""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))
import p128_scalar_scaling

P128Error = p128_scalar_scaling.P128Error


def run(decoded_record: Path, compact_record: Path, output: Path, *, root: Path = ROOT,
        source_reader=None, command_runner=None):
    return p128_scalar_scaling.run(decoded_record, compact_record, output, root=root,
                                   source_reader=source_reader, command_runner=command_runner)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--decoded-record", type=Path, required=True,
                        help="source-bound runtime-only decoded production build record")
    parser.add_argument("--compact-record", type=Path, required=True,
                        help="source-bound runtime-only compact production build record")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = run(args.decoded_record, args.compact_record, args.output)
    except (P128Error, OSError, ValueError, KeyError) as exc:
        print(f"P128 proof failed: {exc}", file=sys.stderr)
        return 1
    print(f"P128 proof passed: {report['counts']['observations']} observations; timing analysis disabled")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
