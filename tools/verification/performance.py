"""Small reproducible tooling workloads; never guest performance evidence."""
from pathlib import Path
import random
import shutil
import statistics
import sys
import tempfile
import time
import tracemalloc
from unittest import mock
from .artifacts import ArtifactStore, need, normal, read_json, write_new
from .context import operation
from .development import run as develop
from .runner import Workflow

PRODUCER = '''import json,pathlib,sys
pathlib.Path(sys.argv[2]).write_text(json.dumps({'passed':True,'synthetic':True,'work':1}))
print('test p999_'+sys.argv[1]+' ... ok')
print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;')
'''


def _spec():
    value = {'schema_version': 1, 'task': 'P999', 'synthetic': True,
             'identities': {'runtime': {'paths': ['src/*'], 'settings': {'synthetic': True}}},
             'stages': {}, 'completion': {'outcome': 'contract_only',
             'required_phases': ['freeze', 'verify', 'measure', 'analyze'],
             'limitations': ['Synthetic tooling performance; never Wasm performance.']}}
    for name, dependencies in [('freeze', []), ('verify', ['freeze']),
                               ('measure', ['freeze', 'verify']), ('analyze', ['freeze', 'verify', 'measure'])]:
        value['stages'][name] = {'uses': ['runtime'], 'dependencies': dependencies,
            'commands': [{'name': 'produce', 'argv': ['{python}', '-B', '{repo}/produce.py', name, '{stage}/result.json']}],
            'outputs': {'result': '{stage}/result.json'},
            'tests': [{'command': 'produce', 'required_names': ['p999_' + name], 'minimum_passed': 1, 'features': []}],
            'checks': [{'artifact': 'result', 'pointer': '/passed', 'op': 'equal', 'value': True}]}
    value['development'] = {'groups': [{'name': 'focused', **value['stages']['freeze']}]}
    return value


def _timed(call):
    start = time.perf_counter()
    result = call()
    return time.perf_counter() - start, result


def _no_processes():
    return mock.patch.multiple('subprocess', run=mock.Mock(side_effect=AssertionError('cached operation launched a process')),
        Popen=mock.Mock(side_effect=AssertionError('cached operation launched a process')))


def run(repo, output, budget_path=None):
    repo, output = Path(repo).resolve(strict=True), Path(output).resolve()
    budget_path = Path(budget_path or Path(__file__).with_name('performance-budget.json'))
    budget = read_json(budget_path)
    need(budget.get('schema_version') == 1 and 3 <= budget.get('repetitions', 0) <= 10,
         'invalid tooling performance budget')
    need(type(budget.get('payload_bytes')) is int and 8 * 1024 * 1024 <= budget['payload_bytes'] <= 16 * 1024 * 1024,
         'tooling payload must exercise bounded multi-MiB streaming')
    need(not output.exists(), 'tooling performance output already exists')
    output.mkdir(parents=True, exist_ok=False)
    # Preserve executed sources before running any fixture commands.
    source_root = Path(__file__).parent
    for path in sorted(source_root.glob('*.py')):
        dest = output / 'sources' / path.name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)
    shutil.copy2(budget_path, output / 'budget.executed.json')
    (output / 'producer.executed.py').write_text(PRODUCER)
    write_new(output / 'spec.executed.json', _spec())
    samples = []
    for repetition in range(budget['repetitions']):
        with tempfile.TemporaryDirectory(prefix='wtime-tooling-perf-') as directory:
            base = Path(directory).resolve()
            fixture = base / 'repo'
            def path(relative):
                target = (fixture / relative).resolve()
                need(target.is_relative_to(base), 'synthetic output escaped fresh root')
                target.parent.mkdir(parents=True, exist_ok=True)
                return target
            path('src/runtime.rs').write_text('// synthetic runtime identity only\n')
            path('src/payload.bin').write_bytes(b'\x7fELF' + random.Random(budget['payload_seed']).randbytes(budget['payload_bytes'] - 4))
            path('produce.py').write_text(PRODUCER)
            contract = path('contract.json')
            write_new(contract, _spec())
            flow = Workflow(fixture, base / 'store')
            phases = ['freeze', 'verify', 'measure', 'analyze']
            cold, cold_result = _timed(lambda: flow.run_phases(contract, phases))
            cold_metrics = dict(flow.last_metrics)
            with _no_processes():
                warm, warm_result = _timed(lambda: flow.run_phases(contract, phases))
                need(all(value['reused'] for value in warm_result.values()), 'warm operation did not reuse every phase')
                warm_metrics = dict(flow.last_metrics)
                final, _ = _timed(lambda: flow.finalize(contract, base / 'final'))
                final_metrics = dict(flow.last_metrics)
            _, dev_cold = _timed(lambda: develop(fixture, base / 'dev-store', contract))
            with _no_processes():
                dev, dev_result = _timed(lambda: develop(fixture, base / 'dev-store', contract))
                need(all(group['reused'] for group in dev_result['groups'].values()), 'development no-op executed tests')
            refs = {ref['path']: ref for value in cold_result.values()
                    for ref in [value['receipt'], *value['artifacts'].values()]}
            unique_bytes = sum(ref['bytes'] + ref['stored_bytes'] for ref in refs.values())
            payload = flow.store.add(fixture / 'src/payload.bin')
            tracemalloc.start()
            try:
                with operation() as ctx:
                    for _ in range(3):
                        flow.store.verify(payload)
                _, peak = tracemalloc.get_traced_memory()
            finally:
                tracemalloc.stop()
            need(ctx.summary().get('artifact_verifications') == 1, 'stream checks repeated full verification')
            sample = {'cold_seconds': cold, 'warm_seconds': warm, 'finalize_seconds': final,
                      'development_noop_seconds': dev, 'stream_peak_bytes': peak,
                      'unique_artifact_bytes': unique_bytes, 'cold': cold_metrics, 'warm': warm_metrics,
                      'finalize': final_metrics, 'development_noop': dev_result['metrics'], 'stream': ctx.summary()}
            samples.append(sample)
            write_new(output / f'sample-{repetition + 1:02d}.json', sample)
    def median(name):
        return statistics.median(sample[name] for sample in samples)
    ordered_warm = sorted(sample['warm_seconds'] for sample in samples)
    limits = budget['limits']
    checks = {
        'cold_median': median('cold_seconds') <= limits['cold_median_seconds'],
        'warm_median': median('warm_seconds') <= limits['warm_median_seconds'],
        'warm_p95': ordered_warm[-1] <= limits['warm_p95_seconds'],
        'finalize_median': median('finalize_seconds') <= limits['finalize_median_seconds'],
        'development_noop_median': median('development_noop_seconds') <= limits['development_noop_median_seconds'],
        'stream_peak': max(sample['stream_peak_bytes'] for sample in samples) <= limits['stream_peak_bytes'],
        'warm_processes': all(sample['warm'].get('subprocesses', 0) == 0 for sample in samples),
        'finalize_processes': all(sample['finalize'].get('subprocesses', 0) == 0 for sample in samples),
        'development_noop_processes': all(sample['development_noop'].get('subprocesses', 0) == 0 for sample in samples),
        'warm_dag_once': all(sample['warm'].get('phase_inputs_computations') == 4 for sample in samples),
        'warm_artifact_bytes': all(sample['warm'].get('artifact_raw_bytes_verified', 0)
             + sample['warm'].get('artifact_stored_bytes_read', 0)
             <= sample['unique_artifact_bytes'] * limits['warm_cas_byte_multiplier'] for sample in samples)
    }
    report = {'schema': 'wtime-tooling-performance-v1', 'status': 'passed' if all(checks.values()) else 'failed',
              'synthetic': True, 'guest_timing_evidence': False, 'budget': normal(budget_path), 'checks': checks,
              'medians': {name: median(name) for name in ('cold_seconds', 'warm_seconds', 'finalize_seconds', 'development_noop_seconds')},
              'samples': samples, 'python': sys.version, 'limitations': ['Shared-cloud wall-time noise; stable work counters are also gated.']}
    write_new(output / 'report.json', report)
    need(report['status'] == 'passed', 'tooling performance budget failed: ' + ', '.join(name for name, passed in checks.items() if not passed))
    return {'status': 'passed', 'report': str(output / 'report.json'), 'medians': report['medians']}
