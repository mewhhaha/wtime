#!/usr/bin/env python3
"""Assemble the P046 portfolio input for the shared statistics phase."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
PRODUCERS = ROOT / "tools/verification/producers"
sys.dont_write_bytecode = True
sys.path.insert(0, str(PRODUCERS))

from p046_verify import FAMILIES, read, ref, write_new

def portfolio(verify_dir: Path, measure_dir: Path, output: Path, *,
              default_build: Path, optional_build: Path,
              frozen_build: Path = ROOT / "docs/verification/build-final.json",
              p121: Path = ROOT / "target/verification/runs/P121/verify/attempt-0001/coverage.json",
              retained_report: Path | None = None,
              build_growth_explanation: str = "P046 compares the optimized Compact implementation with the immediately preceding source and records Compact's optional build growth for review.") -> dict[str, Any]:
    """Assemble hashed report references for shared P046 statistics; no statistics run here."""
    verify_dir, measure_dir, output = (Path(verify_dir).resolve(strict=True),
                                       Path(measure_dir).resolve(strict=True), Path(output).resolve())
    output.mkdir(parents=True, exist_ok=False)
    verification = read(verify_dir / "baselines/report.json")
    measurements = read(measure_dir / "report.json")
    refs = measurements['reports']
    for families in refs.values():
        for reference in families.values():
            if ref(Path(reference['path'])) != reference:
                raise ValueError('reused measurement reference changed')
    if retained_report is None:
        retained_report = verify_dir / "retained/report.json"
    capabilities = verification.get("p123_capabilities")
    if not capabilities:
        raise ValueError("fresh P123 capability evidence is required to assemble the portfolio")
    value = {"schema": "p046-portfolio-v1", "reports": refs,
             "cold_controls": measurements.get("cold_controls", {}),
             "p121": ref(p121), "p123": capabilities,
             "retained": ref(retained_report),
             "structural_build_reports": {"default": ref(default_build), "compact": ref(optional_build)},
             "frozen_build_report": str(Path(frozen_build).resolve(strict=True)),
             "build_growth_explanation": build_growth_explanation,
             "production_build_record": ref(verify_dir / "production/build-record.json"),
             "baseline_report": ref(verify_dir / "baselines/report.json"),
             "measurement_report": ref(measure_dir / "report.json")}
    write_new(output / "input.json", value)
    write_new(output / "report.json", {"schema": "p046-portfolio-input-v1", "status": "passed",
                                       "statistics_input": ref(output / "input.json"),
                                       "timing_evidence": False})
    return value

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("portfolio",))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-dir", type=Path, required=True)
    parser.add_argument("--measure-dir", type=Path, required=True)
    parser.add_argument("--default-build", type=Path, required=True)
    parser.add_argument("--optional-build", type=Path, required=True)
    parser.add_argument("--frozen-build", type=Path,
                        default=ROOT / "docs/verification/build-final.json")
    parser.add_argument("--p121", type=Path,
                        default=ROOT / "target/verification/runs/P121/verify/attempt-0001/coverage.json")
    parser.add_argument("--retained-report", type=Path)
    args = parser.parse_args()
    value = portfolio(args.verify_dir, args.measure_dir, args.output,
                      default_build=args.default_build, optional_build=args.optional_build,
                      frozen_build=args.frozen_build, p121=args.p121,
                      retained_report=args.retained_report)
    print(json.dumps({"phase": args.phase, "status": "passed"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
