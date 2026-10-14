"""Run the legacy representative timer with its precise execution identity.

The timer, registry, oracle and paired observations are unchanged. Legacy
reports included every Python file; verification-only files are not runtime
harness inputs and need not have existed in the frozen adapter.
"""
from __future__ import annotations
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import compare
import execution_identity

SOURCE = 'tools/verification/producers/portfolio_compare.py'


def measurement_sources(raw):
    return {**execution_identity.project(raw), SOURCE:compare.sha(ROOT/SOURCE)}


def main():
    args = sys.argv[1:]
    if args.count('--selection') != 1 or args[args.index('--selection')+1:][:1] != ['representative']:
        raise ValueError('portfolio identity adapter is restricted to the full representative selection')
    original = compare.sources
    compare.sources = lambda **kwargs: measurement_sources(original(**kwargs))
    try:
        return compare.main()
    finally:
        compare.sources = original


if __name__ == '__main__':
    raise SystemExit(main())
