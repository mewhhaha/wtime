"""P056 orchestration around existing builders, timers and portfolio analysis."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
sys.path.insert(0, str(ROOT / 'tools/verification/producers'))

from p046_verify import (P001_BINARY, P001_MANIFEST, _production_sources,
                         read, ref, sha, write_new)
import baseline
import p123_adapter

CRATES = ('wtime-runtime', 'wtime-types', 'wtime-decode', 'wtime-validate')


def need(ok, message):
    if not ok:
        raise ValueError(message)


def core_map(manifest):
    return dict(sorted((row['source_path'], row['sha256'])
                       for row in manifest['files'].values()
                       if row.get('role') == 'source'
                       and (row.get('source_path') in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
                            or (any(row.get('source_path', '').startswith('crates/' + crate + '/')
                                    for crate in CRATES)
                                and (row['source_path'].endswith('.rs')
                                     or Path(row['source_path']).name == 'Cargo.toml')))))


def verify_current_core(source_map):
    import p046_gate
    current = p046_gate._p056_current_core_source_map()
    need(current == source_map, 'P056 immediate predecessor does not match the complete current core')


def checked_ref(reference):
    path = Path(reference['path']).resolve(strict=True)
    need(ref(path) == reference, 'P056 immutable input changed: ' + str(path))
    return path


def predecessor(path):
    record = read(path)
    need(record.get('schema') == 'wtime-p056-predecessor-v1'
         and record.get('task') == 'P056' and record.get('status') == 'passed'
         and record.get('source_stable') is True, 'invalid P056 predecessor declaration')
    archive = checked_ref(record['archive']).parent
    need(core_map(baseline.verify_archive(archive)) == record['core_source_map'],
         'P056 frozen core source membership changed')
    verify_current_core(record['core_source_map'])
    return record, archive


def freeze(output):
    # The shared runner has already allocated this exclusive phase directory.
    output.mkdir(parents=True, exist_ok=True)
    need(not any((output / name).exists() for name in ('report.json', 'predeclared-inputs.json', 'helper.executed.py')),
         'P056 freeze output already contains producer evidence')
    shutil.copy2(__file__, output / 'helper.executed.py')
    before = _production_sources()
    archive = ROOT / 'target/performance/P055-before'
    manifest = baseline.verify_archive(archive)
    sources = core_map(manifest)
    verify_current_core(sources)
    original = ROOT / 'target/performance/baseline'
    baseline.verify_archive(original)
    need(sha(original / 'manifest.json') == P001_MANIFEST
         and sha(original / 'binary/wtime-bench') == P001_BINARY,
         'P056 immutable P001 archive was substituted')
    record = {'schema': 'wtime-p056-predecessor-v1', 'task': 'P056',
              'status': 'passed', 'source_stable': True, 'source_current': True,
              'archive': ref(archive / 'manifest.json'), 'core_source_map': sources,
              'runtime_source_map': {name: value for name, value in sources.items()
                                     if name.startswith('crates/wtime-runtime/src/')},
              }
    inputs = {'p001': ref(original / 'manifest.json'),
              'variants': {task: ref(ROOT / f'target/performance/{task}-before/manifest.json')
                           for task in ('P047', 'P050')},
              'selections': {task: ref(ROOT / f'docs/benchmarks/performance/{task}-selection.json')
                             for task in ('P047', 'P050')},
              'refresh_selections': {
                  'P047': ref(ROOT / 'docs/benchmarks/performance/P056-P047-refresh-selection.json'),
                  'P050': ref(ROOT / 'docs/benchmarks/performance/P050-selection.json')},
              'structural_comparison': 'current Compact against Decoded built from its exact immediate predecessor core',
              'timing_evidence': False}
    for reference in inputs['variants'].values():
        baseline.verify_archive(checked_ref(reference).parent)
    need(before == _production_sources(), 'sources changed during P056 freeze')
    write_new(output / 'report.json', record)
    write_new(output / 'predeclared-inputs.json', inputs)
    return record


def build_baselines(output, frozen, reuse_baselines=None):
    declaration, archive = predecessor(frozen)
    inputs = read(frozen.parent / 'predeclared-inputs.json')
    directory = output / 'baselines'
    directory.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, directory / 'helper.executed.py')
    if reuse_baselines is not None:
        previous = Path(reuse_baselines).resolve(strict=True)
        previous_result_path = previous / 'baseline-builds.result.json'
        previous_report_path = previous / 'baselines/report.json'
        previous_result_ref = ref(previous_result_path)
        previous_result = read(previous_result_path)
        old_argv = previous_result.get('argv')
        need(previous_result.get('name') == 'baseline-builds'
             and previous_result.get('exit_code') == 0
             and isinstance(old_argv, list) and len(old_argv) >= 8
             and old_argv[1] == '-B'
             and Path(old_argv[2]).resolve() == Path(__file__).resolve()
             and old_argv[3] == 'build-baselines'
             and old_argv[4] == '--output'
             and Path(old_argv[5]).resolve() == previous
             and old_argv[6] == '--frozen'
             and previous_result.get('cwd') == str(ROOT),
             'P056 baseline reuse requires a passed baseline-builds phase result')
        old_frozen_path = Path(old_argv[7]).resolve(strict=True)
        old_declaration, old_archive = predecessor(old_frozen_path)
        old_predecessor_ref = ref(old_frozen_path)
        need(old_declaration == declaration and old_archive == archive,
             'P056 reused baseline predecessor differs from the current freeze declaration')
        previous_report_ref = ref(previous_report_path)
        previous_report = read(previous_report_path)
        need(previous_report.get('schema') == 'p046-baselines-v1'
             and previous_report.get('task') == 'P056'
             and previous_report.get('status') == 'passed'
             and previous_report.get('timing_evidence') is False,
             'P056 reuse source is not a passed immutable baseline report')
        routes = {'p001': (checked_ref(inputs['p001']).parent,
                           previous_report['p001_adapter'], 'decoded', True),
                  'structural': (archive, previous_report['structural_adapter'], 'decoded', False),
                  'p047-base': (checked_ref(inputs['variants']['P047']).parent,
                                previous_report['individual_adapters']['p047-base'], 'compact', False),
                  'p047-only': (checked_ref(inputs['variants']['P050']).parent,
                                previous_report['individual_adapters']['p047-only'], 'compact', False)}
        adapters = {}
        for name, (expected_archive, reference, mode, legacy) in routes.items():
            adapter_path = checked_ref(reference)
            record = p123_adapter.verify(adapter_path.parent, expected_archive)
            need(record.get('mode') == mode and record.get('legacy_api') is legacy,
                 'P056 reused adapter route differs: ' + name)
            adapters[name] = reference
        capability_ref = previous_report['p123_capabilities']
        capability_path = checked_ref(capability_ref)
        import p046_gate
        capability, _support = p046_gate._load_p123_support(capability_ref, check_sources=True)
        import p123
        import workloads
        current_counts = {family: len(workloads.selection(family)) for family in p123.COUNTS}
        need(current_counts.get('memory') == 53
             and capability.get('actual_stage_probes') == sum(
                 len(row['stages']) for family in p123.COUNTS
                 for row in workloads.selection(family)),
             'P056 reused P001 capability receipt has stale current workload coverage')
        need(all(capability['families'][family].get('supported') == count
                 and capability['families'][family].get('passed') == count
                 and capability['families'][family].get('unsupported') == 0
                 for family, count in current_counts.items()),
             'P056 reused P001 capability receipt is incomplete')
        need(checked_ref(capability_ref) == capability_path
             and ref(previous_report_path) == previous_report_ref
             and ref(previous_result_path) == previous_result_ref
             and ref(old_frozen_path) == old_predecessor_ref,
             'P056 reused baseline evidence changed during validation')
        # Retain byte-identical capability output in the new phase; keep the
        # original observations and build references in the reuse record.
        retained_capability = directory / 'p123probe/capabilities/report.json'
        retained_capability.parent.mkdir(parents=True, exist_ok=False)
        shutil.copy2(capability_path, retained_capability)
        need(sha(retained_capability) == capability_ref['sha256'],
             'P056 retained capability report differs from its verified source')
        report = {'schema': 'p046-baselines-v1', 'task': 'P056', 'status': 'passed',
                  'timing_evidence': False, 'p001_adapter': adapters['p001'],
                  'structural_adapter': adapters['structural'],
                  'individual_adapters': {name: adapters[name]
                                          for name in ('p047-base', 'p047-only')},
                  'p001_archive': inputs['p001'], 'decoded_predecessor': declaration['archive'],
                  'structural_predecessor': ref(frozen), 'p123_capabilities': ref(retained_capability),
                  'report_reused': True,
                  'reuse_evidence': {'baseline_report': previous_report_ref,
                                     'baseline_phase_result': previous_result_ref,
                                     'old_predecessor': old_predecessor_ref,
                                     'adapters': adapters,
                                     'p123_capabilities': capability_ref}}
        write_new(directory / 'report.json', report)
        return report
    routes = {'p001': (ROOT / 'target/performance/baseline', 'decoded', True),
              'structural': (archive, 'decoded', False),
              'p047-base': (checked_ref(inputs['variants']['P047']).parent, 'compact', False),
              'p047-only': (checked_ref(inputs['variants']['P050']).parent, 'compact', False)}
    adapters = {}
    for name, (source, mode, legacy) in routes.items():
        destination = directory / 'adapters' / name
        p123_adapter.create(source, destination, mode=mode, legacy_api=legacy)
        adapters[name] = ref(destination / 'manifest.json')
    # One immutable fuse-only executable supplies both the P047 candidate and
    # the immediate P050 predecessor. This shares a build, never timing samples.
    import p123
    import workloads
    probe_path = directory / 'p123probe/capabilities'
    capability = p123.probe(Path(adapters['p001']['path']).parent, probe_path)
    expected_probes = sum(len(row['stages']) for family in p123.COUNTS
                          for row in workloads.selection(family))
    need(capability.get('actual_stage_probes') == expected_probes
         and capability.get('timing_evidence') is False, 'P001 capability coverage is incomplete')
    report = {'schema': 'p046-baselines-v1', 'task': 'P056', 'status': 'passed',
              'timing_evidence': False, 'p001_adapter': adapters['p001'],
              'structural_adapter': adapters['structural'],
              'individual_adapters': {name: adapters[name] for name in ('p047-base', 'p047-only')},
              'p001_archive': inputs['p001'], 'decoded_predecessor': declaration['archive'],
              'structural_predecessor': ref(frozen),
              'p123_capabilities': ref(probe_path / 'report.json')}
    write_new(directory / 'report.json', report)
    return report


def measure(verify_dir, output, frozen):
    predecessor(frozen)
    import p046_measure
    return p046_measure.measure(verify_dir, output, cold=True, seed=20261014,
                               structural_predecessor=ref(frozen),
                               cache=ROOT / 'target/verification/datasets/P056')


def portfolio(verify_dir, measure_dir, output, frozen):
    predecessor(frozen)
    baselines = read(verify_dir / 'baselines/report.json')
    measurements = read(measure_dir / 'report.json')
    output.mkdir(parents=True, exist_ok=False)
    for families in measurements['reports'].values():
        for reference in families.values():
            checked_ref(reference)
    value = {'schema': 'p046-portfolio-v1', 'structural_predecessor': ref(frozen),
             'reports': measurements['reports'], 'cold_controls': measurements['cold_controls'],
             'p121': ref(verify_dir / 'coverage.json'), 'p123': baselines['p123_capabilities'],
             'retained': ref(verify_dir / 'retained/report.json'),
             'structural_build_reports': {'default': ref(verify_dir / 'build.json'),
                                          'compact': ref(verify_dir / 'optional-build.json')},
             'frozen_build_report': str(ROOT / 'docs/verification/build-final.json'),
             'build_growth_explanation': 'P056 retains only the bounded P047 and P050 implementations inside the opt-in Compact feature. Decoded stays default; removed candidates add no live code or dependency.',
             'production_build_record': ref(verify_dir / 'production/build-record.json'),
             'baseline_report': ref(verify_dir / 'baselines/report.json'),
             'measurement_report': ref(measure_dir / 'report.json')}
    write_new(output / 'input.json', value)
    write_new(output / 'report.json', {'status': 'passed', 'task': 'P056',
                                      'statistics_input': ref(output / 'input.json')})
    return value


def evidence(verify_dir, output, statistics, variants):
    plan = read(ROOT / 'task-plans/performance/TASKS.json')
    tasks = {row['id']: row for row in plan['tasks']}
    history = []
    for task in ('P047', 'P048', 'P049', 'P050', 'P053', 'P054', 'P055'):
        row = tasks[task]
        proof = row['evidence'][0]
        checked_ref(proof)
        history.append({'task': task, 'decision': row['outcome'],
                        'disposition': 'retained_opt_in' if task in ('P047', 'P050') else 'removed',
                        'record': proof})
    value = {'schema': 'wtime-p056-specialization-input-v1', 'task': 'P056', 'status': 'passed',
             'statistics': ref(statistics), 'tier': ref(verify_dir / 'tier/report.json'),
             'verification': ref(verify_dir / 'receipt.json'), 'historical_variants': history,
             'individual_variants': ref(variants), 'optional_unimplemented': ['P051', 'P052']}
    write_new(output, value)
    return value


def variant_statistics(measure_dir, output):
    import analyze
    import p056_variants
    before = _production_sources()
    refresh_set = read(measure_dir / 'report.json')
    need(refresh_set.get('schema') == 'wtime-p056-variant-refresh-set-v1'
         and refresh_set.get('status') == 'passed'
         and set(refresh_set.get('reports', {})) == {'P047', 'P050'},
         'P056 requires both complete independent refresh datasets')
    reports = {}
    for task, reference in refresh_set['reports'].items():
        path = checked_ref(reference)
        raw = read(path)
        checked = p056_variants.validate_raw_report(raw, path, check_sources=True)
        need(checked['task'] == task and checked['report_sha256'] == reference['sha256'],
             'P056 refresh validator returned a different immutable comparison')
        reports[task] = {'raw': reference, 'report_sha256': reference['sha256'],
                         'statistics': analyze.analyze(raw)}
    need(before == _production_sources(), 'sources changed during independent refresh analysis')
    value = {'schema': 'wtime-p056-variant-statistics-v1', 'task': 'P056', 'status': 'passed',
             'reports': reports, 'dataset_statistics_computations': 2,
             'additional_bootstrap_computations': 2,
             'promotion_scope': 'diagnostic refresh; historical individual outcomes unchanged',
             'inputs': {'measurements': ref(measure_dir / 'report.json'),
                        'analyzer': ref(ROOT / 'tools/wtime-bench/analyze.py'),
                        'bootstrap': ref(ROOT / 'tools/wtime-bench/bootstrap.py'),
                        'validator': ref(Path(p056_variants.__file__)), 'producer': ref(Path(__file__))}}
    write_new(output, value)
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('freeze', 'build-baselines', 'measure', 'portfolio', 'evidence',
                                        'variant-statistics'))
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--frozen', type=Path)
    parser.add_argument('--reuse-baselines', type=Path)
    parser.add_argument('--verify-dir', type=Path)
    parser.add_argument('--measure-dir', type=Path)
    parser.add_argument('--statistics', type=Path)
    parser.add_argument('--variants', type=Path)
    args = parser.parse_args()
    required = {'freeze': (), 'build-baselines': ('frozen',),
                'measure': ('frozen', 'verify_dir'), 'portfolio': ('frozen', 'verify_dir', 'measure_dir'),
                'evidence': ('verify_dir', 'statistics', 'variants'),
                'variant-statistics': ('measure_dir',)}[args.phase]
    for name in required:
        if getattr(args, name) is None:
            parser.error('--' + name.replace('_', '-') + ' is required for ' + args.phase)
    if args.reuse_baselines is not None and args.phase != 'build-baselines':
        parser.error('--reuse-baselines is only valid for build-baselines')
    if args.phase == 'freeze':
        value = freeze(args.output)
    elif args.phase == 'build-baselines':
        value = build_baselines(args.output, args.frozen, args.reuse_baselines)
    elif args.phase == 'measure':
        value = measure(args.verify_dir, args.output, args.frozen)
    elif args.phase == 'portfolio':
        value = portfolio(args.verify_dir, args.measure_dir, args.output, args.frozen)
    elif args.phase == 'evidence':
        value = evidence(args.verify_dir, args.output, args.statistics, args.variants)
    else:
        value = variant_statistics(args.measure_dir, args.output)
    print(json.dumps({'task': 'P056', 'phase': args.phase, 'status': value.get('status', 'passed')}))


if __name__ == '__main__':
    main()
