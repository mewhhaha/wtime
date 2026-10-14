#!/usr/bin/env python3
"""Compatibility facade for the phase-specific P046 producers."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
PRODUCERS = ROOT / "tools/verification/producers"
sys.dont_write_bytecode = True
sys.path.insert(0, str(PRODUCERS))

import p046_verify as _verify
from p046_verify import (BENCH, CORE_CRATES, FAMILIES, P001_BINARY, P001_MANIFEST,
                         ROOT, _runtime_sources, _seal_files,
                         _seal_source_snapshot, _validate_frozen_archive,
                         _write_deterministic_tar_gzip, build_baselines,
                         build_production, freeze, read, ref, retained, seal,
                         sha, write_new)
from p046_measure import _comparison, measure
from p046_analysis import portfolio

_core_source_map = _verify._core_source_map
_production_sources = _verify._production_sources
_product_rows = _verify._product_rows
_run_logged = _verify._run_logged


def main() -> int:
    if len(sys.argv) < 2:
        raise SystemExit("usage: p046.py {freeze|build-production|build-baselines|retained|seal|measure|portfolio} ...")
    phase = sys.argv[1]
    if phase in {"freeze", "build-production", "build-baselines", "retained", "seal"}:
        return _verify.main()
    if phase == "measure":
        from p046_measure import main as phase_main
        return phase_main()
    if phase == "portfolio":
        from p046_analysis import main as phase_main
        return phase_main()
    raise SystemExit(f"unsupported P046 phase: {phase}")


if __name__ == "__main__":
    raise SystemExit(main())
