"""Source-bound fixture admission and two-sided GC root stage capability proof."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path.insert(0, str(BENCH))
import baseline
import compare
import execution_identity
import p123_adapter
import root_compare
import root_workloads
import selection_manifest


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def core_map():
    return {name: sha(ROOT/name) for name in sorted(selection_manifest._current_core_paths(BENCH/'workloads.json'))}


def freeze(predecessor, output):
    record = baseline.verify_archive(predecessor)
    sources = selection_manifest._core_runtime_sources(predecessor/'manifest.json', BENCH/'workloads.json')
    write(output, {'schema': 'wtime-root-foundation-freeze-v1', 'status': 'passed',
        'predecessor': {'path': str(predecessor/'manifest.json'), 'sha256': sha(predecessor/'manifest.json')},
        'runtime_sources': sources, 'core_crates': list(selection_manifest._CORE_CRATES),
        'archive_members': len(record['files'])})


def fixtures(output):
    """Re-encode/validate all six fixtures with the already pinned tools."""
    lock = json.loads((ROOT/'docs/benchmarks/performance/P125-fixture-tools.json').read_text())
    tools = {}
    for name in ('encoder', 'validator'):
        item = lock['tools'][name]
        path = Path(item['path']).resolve(strict=True)
        if sha(path) != item['sha256']:
            raise ValueError('pinned fixture tool changed: '+name)
        retained = output/'tools'/name
        retained.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, retained)
        tools[name] = retained
    rows = []
    for family, model in root_workloads.FAMILIES.items():
        wat = root_workloads.FIXTURE_DIR/model['file']
        if sha(wat) != model['wat_sha256']:
            raise ValueError('fixture WAT identity differs')
        retained = output/'fixtures'/wat.name
        retained.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(wat, retained)
        wasm = retained.with_suffix('.wasm')
        commands = ([str(tools['encoder']), 'encode', str(retained), '-o', str(wasm)],
                    [str(tools['validator']), '--profile', 'all-phase3-core', 'check', str(wasm)])
        for index, command in enumerate(commands):
            label = f'{family}.{index}'
            write(output/'fixtures'/(label+'.command.json'), {'argv': command, 'cwd': str(ROOT)})
            with (output/'fixtures'/(label+'.stdout')).open('xb') as stdout, (output/'fixtures'/(label+'.stderr')).open('xb') as stderr:
                proc = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=60)
            write(output/'fixtures'/(label+'.status.json'), {'exit_code': proc.returncode})
            if proc.returncode:
                raise RuntimeError('pinned fixture admission failed: '+label)
        if sha(wasm) != model['wasm_sha256']:
            raise ValueError('encoded fixture identity differs')
        rows.append({'family': family, 'wat_sha256': sha(retained), 'wasm_sha256': sha(wasm)})
    return rows, lock


def verify(frozen_path, output):
    """Run correctness probes, not a performance candidate or speed gate."""
    frozen = json.loads(frozen_path.read_text())
    predecessor = Path(frozen['predecessor']['path']).parent
    baseline.verify_archive(predecessor)
    if sha(predecessor/'manifest.json') != frozen['predecessor']['sha256'] or core_map() != frozen['runtime_sources']:
        raise ValueError('foundation must use the exact pre-P068 core on both sides')
    before = execution_identity.project(compare.sources(include_memory=True, include_simd=True,
        include_representative=True, include_integration=True), focused=True)
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, output/'helper.executed.py')
    for name, digest in before.items():
        dest = output/'sources'/name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT/name, dest)
        if sha(dest) != digest:
            raise ValueError('source changed while retaining foundation inputs')
    admitted, lock = fixtures(output)
    # This producer also preserves the actual Cargo command, logs and binary.
    from candidate_production import build_production
    current = build_production(output/'production', 'decoded')
    adapter = p123_adapter.create(predecessor, output/'adapter', mode='decoded', legacy_api=False)
    binaries = {'baseline': output/'adapter/binary/wtime-bench', 'candidate': Path(current['binary_path'])}
    cpu = sorted(os.sched_getaffinity(0))[1]
    capabilities = {side: compare.process(executable, ['--protocol'], cpu) for side, executable in binaries.items()}
    for value in capabilities.values():
        if value.get('instrumented') is not False or 'decoded' not in value.get('modes', []) or 'all-phase3-core' not in value.get('features', []):
            raise ValueError('foundation capability is unavailable')
    model_rows = root_workloads.entries()
    registry = json.loads((BENCH/'workloads.json').read_text())
    selected = registry['selections']['gc-root-snapshots']
    by_id = {row['id']: row for row in registry['workloads']}
    if [by_id[wid] for wid in selected['workloads']] != model_rows:
        raise ValueError('root registry selection differs from the admitted model')
    records = []
    try:
        for entry in model_rows:
            wasm = output/'fixtures'/Path(entry['source']['path']).with_suffix('.wasm').name
            for stage in root_workloads.STAGES:
                batch = 3 if stage == 'warm_call' else 1
                for side, executable in binaries.items():
                    command = ['--gc-root-driver', wasm, entry['id'], entry['family'], entry['params'][0]['baseline'],
                               'decoded', 'all-phase3-core', stage, batch]
                    wire = compare.process(executable, command, cpu)
                    observation = root_compare.validate_observation(entry, wire, batch, expected_stage=stage)
                    records.append({'workload': entry['id'], 'stage': stage, 'side': side, 'observation': observation})
        if core_map() != frozen['runtime_sources'] or execution_identity.project(compare.sources(
                include_memory=True, include_simd=True, include_representative=True, include_integration=True), focused=True) != before:
            raise ValueError('foundation sources changed during capability proof')
        for name, item in lock['tools'].items():
            if name in ('encoder', 'validator') and sha(item['path']) != item['sha256']:
                raise ValueError('fixture tool changed during capability proof')
        for side, executable in binaries.items():
            expected = adapter['binary_sha256'] if side == 'baseline' else current['binary_sha256']
            if sha(executable) != expected:
                raise ValueError('foundation executable changed')
    except BaseException:
        write(output/'partial-records.json', records)
        raise
    counts = {side: sum(record['side'] == side for record in records) for side in binaries}
    if counts != {'baseline': 120, 'candidate': 120}:
        raise ValueError('foundation row-stage count differs')
    write(output/'report.json', {'schema': 'wtime-root-foundation-proof-v1', 'status': 'passed',
        'source_stable': True, 'task': 'P125', 'performance_candidate': False, 'speed_decision': None,
        'counts': {'workloads': len(model_rows), 'fixtures': len(admitted), 'row_stage_records': counts,
                   'warm_repeated_records': sum(record['stage'] == 'warm_call' for record in records)},
        'settings': {'mode': 'decoded', 'profile': 'all-phase3-core', 'instrumented': False,
                     'selected_cpu': cpu, 'warm_batch': 3, 'hardware_isolation': 'shared cloud'},
        'predecessor': frozen['predecessor'], 'runtime_sources': frozen['runtime_sources'],
        'execution_inputs': before, 'fixtures': admitted, 'pinned_tools': lock,
        'binaries': {side: {'path': str(executable), 'sha256': sha(executable)} for side, executable in binaries.items()},
        'capabilities': capabilities, 'records': records})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('freeze', 'verify'))
    parser.add_argument('--predecessor', type=Path)
    parser.add_argument('--frozen', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.operation == 'freeze':
        freeze(args.predecessor.resolve(strict=True), args.output.resolve())
    else:
        verify(args.frozen.resolve(strict=True), args.output.resolve())
    print(json.dumps({'status': 'passed', 'operation': args.operation}))


if __name__ == '__main__':
    main()
