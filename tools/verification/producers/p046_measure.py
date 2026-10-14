#!/usr/bin/env python3
"""P046 paired family and cold-control measurements."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
PRODUCERS = ROOT / "tools/verification/producers"
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(PRODUCERS))

from p046_verify import (FAMILIES, _core_source_map, _production_sources,
                         read, ref, sha, write_new)
import execution_identity
import dataset_cache

CACHE = ROOT/'target/verification/datasets/P046'


def dataset_identity(report):
    metadata = report['metadata']
    return {'schema':'p046-dataset-identity-v1','selection':report['selection'],
            'baseline_manifest_sha256':metadata['baseline_manifest_sha256'],
            'candidate_binary_sha256':metadata['candidate_binary_sha256'],
            'candidate_mode':metadata['settings']['candidate_mode'],
            'baseline_mode':metadata['settings']['baseline_mode'],
            'execution_sources':execution_identity.project(metadata['candidate_sources']),
            'hardware':{key:metadata['hardware'][key] for key in ('selected_cpu','cpu_model','cpu_quota')},
            'run_seeds':[run['seed'] for run in report['runs']], 'pairs_per_group':30}


def validate_dataset(path):
    import analyze
    import p046_gate
    report = read(path)
    selection = report['selection']
    name = 'p001' if read(Path(report['metadata']['baseline_archive_path'])/'manifest.json').get('legacy_api') else 'decoded'
    if selection in FAMILIES:
        p046_gate._report_groups(report, selection, name, check_sources=True)
        if selection == 'representative':
            p046_gate._representative_images(report, ref(path))
    else:
        if selection != 'original-mvp' or report.get('workload_subset'):
            raise ValueError('P046 cache accepts complete family or original-MVP datasets only')
        execution_identity.verify_inputs(report)
        p046_gate._verify_baseline_archive(report,name,check_sources=True)
        _, runs = analyze._validated_runs(report)
        expected = p046_gate._expected_groups(selection)
        for run in runs:
            if ({(key[0],key[1]) for key in run['groups']} != expected
                    or len(run['groups']) != len(expected)
                    or any(len(rows) != 30 for rows in run['groups'].values())):
                raise ValueError('incomplete P046 cold control dataset')
    return report


def admit_dataset(path, cache=CACHE):
    report = validate_dataset(path)
    return dataset_cache.admit(path, cache, dataset_identity(report), validator=validate_dataset)

def _comparison(argv: list[str], output: Path, label: str) -> Path:
    directory = output / label
    if directory.exists():
        raise FileExistsError(f"comparison output already exists: {directory}")
    values = {argv[i]:argv[i+1] for i in range(0,len(argv),2)}
    selection = values['--selection']
    import compare
    seed = int(values['--seed'])
    expected_sources = compare.sources(include_memory=selection=='memory',
        include_simd=selection=='simd',include_representative=selection=='representative')
    if selection == 'representative':
        import portfolio_compare
        expected_sources = portfolio_compare.measurement_sources(expected_sources)
    expected = {'schema':'p046-dataset-identity-v1','selection':selection,
        'baseline_manifest_sha256':sha(Path(values['--baseline'])/'manifest.json'),
        'candidate_binary_sha256':sha(Path(values['--candidate'])),
        'candidate_mode':values['--candidate-mode'],'baseline_mode':'decoded',
        'execution_sources':execution_identity.project(expected_sources),
        'hardware':{'selected_cpu':sorted(os.sched_getaffinity(0))[0],
            'cpu_model':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines()
                if line.startswith('model name')), 'unknown'),
            'cpu_quota':Path('/sys/fs/cgroup/cpu.max').read_text().strip()},
        'run_seeds':[seed,seed+1_000_003],'pairs_per_group':30}
    reused = dataset_cache.lookup(CACHE, expected, validator=validate_dataset)
    if reused is not None:
        # The phase bundle must retain the actual evidence, not just a pointer
        # into another attempt which could later disappear. Copying is outside
        # all timers and preserves every raw byte and relative image reference.
        shutil.copytree(reused.parent, directory)
        retained = directory/'report.json'
        if sha(retained) != sha(reused):
            raise ValueError('cached dataset changed while retaining its payload')
        with (output/'commands.jsonl').open('a',encoding='utf-8') as stream:
            stream.write(json.dumps({'label':label,'action':'reuse-immutable-dataset',
                                     'origin':ref(reused),'report':ref(retained)})+'\n')
        return retained
    script = PRODUCERS/'portfolio_compare.py' if selection == 'representative' else BENCH/'compare.py'
    args = [sys.executable, "-B", str(script), *argv,
            "--output", str(directory)]
    with (output / "commands.jsonl").open("a", encoding="utf-8") as stream:
        stream.write(json.dumps({"label": label, "argv": args, "cwd": str(ROOT),
                                 "seed": next((args[i + 1] for i, x in enumerate(args) if x == "--seed"), None),
                                 "cpu_affinity": sorted(os.sched_getaffinity(0))}) + "\n")
    # Snapshot source, binary, and fixture identities around the entire CLI call.
    before_sources = _production_sources()
    before_core = _core_source_map()
    proc = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=False)
    (output / f"{label}.stdout").write_text(proc.stdout, encoding="utf-8")
    (output / f"{label}.stderr").write_text(proc.stderr, encoding="utf-8")
    write_new(output / f"{label}.status.json", {"exit_code": proc.returncode})
    report_path = directory / "report.json"
    if proc.returncode:
        raise RuntimeError(f"comparison {label} failed with {proc.returncode}; partial output retained at {directory}")
    report = read(report_path)
    if (report.get("passed") is not True or report.get("source_stable") is not True
            or report.get("metadata", {}).get("settings", {}).get("instrumented") is not False):
        raise ValueError(f"comparison {label} failed production/source stability checks")
    if before_sources != _production_sources() or before_core != _core_source_map():
        raise ValueError(f"candidate source identities changed during comparison {label}")
    if dataset_identity(report) != expected:
        raise ValueError('completed dataset differs from requested measurement identity')
    admit_dataset(report_path)
    return report_path


def measure(verify_dir: Path, output: Path, *, cold: bool = True,
            seed: int = 20261013) -> dict[str, Any]:
    """Run eight full portfolio comparisons and optional predeclared cold controls."""
    verify_dir, output = Path(verify_dir).resolve(strict=True), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, output / "helper.executed.py")
    production = read(verify_dir / "production/build-record.json")
    baselines = read(verify_dir / "baselines/report.json")
    candidate = Path(production["binary_path"]).resolve(strict=True)
    if sha(candidate) != production["binary_sha256"]:
        raise ValueError("production binary changed before measurement")
    if production.get("source_stable") is not True:
        raise ValueError("production build did not preserve source identities")
    # Four predeclared selections, each measured against the P001 and decoded routes.
    reports = {'p001':{},'structural':{}}
    for i, family in enumerate(FAMILIES):
        for baseline_name, baseline_ref in (("p001", baselines["p001_adapter"]),
                                            ("structural", baselines["structural_adapter"])):
            adapter = Path(baseline_ref["path"]).parent
            if sha(adapter / "manifest.json") != baseline_ref["sha256"]:
                raise ValueError(f"{baseline_name} adapter identity changed before measurement")
            label = f"{baseline_name}-{family}"
            path = _comparison(["--baseline", str(adapter), "--candidate", str(candidate),
                         "--candidate-mode", "compact", "--selection", family,
                         "--pairs", "30", "--seed", str(seed + i * 2 + (baseline_name == "structural"))],
                        output, label)
            reports[baseline_name][family] = ref(path)
    cold_reports: dict[str, Any] = {}
    if cold:
        for baseline_name, baseline_ref in (("p001", baselines["p001_adapter"]),
                                            ("structural", baselines["structural_adapter"])):
            adapter = Path(baseline_ref["path"]).parent
            label = f"cold-{baseline_name}-original-mvp"
            path = _comparison(["--baseline", str(adapter), "--candidate", str(candidate),
                         "--candidate-mode", "compact", "--selection", "original-mvp",
                         "--pairs", "30", "--seed", str(seed + 20 + (baseline_name == "structural"))],
                        output, label)
            cold_reports[baseline_name] = ref(path)
    report = {"schema": "p046-measurements-v1", "status": "passed",
              "timing_evidence": True, "candidate": ref(candidate),
              "baseline_manifest_refs": {"p001": baselines["p001_adapter"],
                                          "structural": baselines["structural_adapter"]},
              "reports": reports,
              "cold_controls": cold_reports,
              "commands": ref(output / "commands.jsonl"),
              "pair_count_per_comparison": 30,
              "portfolio_pair_count": 131400 + (2520 if cold else 0),
              "cpu_affinity": sorted(os.sched_getaffinity(0))}
    write_new(output / "report.json", report)
    return report

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("measure", "admit"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-dir", type=Path)
    parser.add_argument("--report",type=Path)
    parser.add_argument("--skip-cold-controls", action="store_true")
    parser.add_argument("--seed", type=int, default=20261013)
    args = parser.parse_args()
    if args.phase == 'admit':
        if args.report is None:
            parser.error('admit requires --report')
        value = {'status':'passed','receipt':str(admit_dataset(args.report,args.output))}
    else:
        if args.verify_dir is None:
            parser.error('measure requires --verify-dir')
        value = measure(args.verify_dir, args.output, cold=not args.skip_cold_controls, seed=args.seed)
    print(json.dumps({"phase": args.phase, "status": value.get("status", "passed")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
