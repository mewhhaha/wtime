"""One-time P042 adapter: validate frozen readers, then retain each payload once.

The historical closure reader runs with a reference-only copy hook. This keeps
its semantic/integrity checks while replacing its repeated artifact packaging.
No Cargo, encoder, guest, or bootstrap command is executed by this adapter.
"""
from __future__ import annotations
import argparse
from pathlib import Path
import sys
from unittest.mock import patch
from .artifacts import ArtifactStore, digest, need, normal, read_json, sha, verify_normal, write_new
from .finalize import completion
from .statistics import module

RECORDS = {
    'check_result': 'check/attempt-0002/result.json',
    'prepare_receipt': 'prepare/attempt-0003/receipt.json',
    'test_contract': 'integration/attempt-0001/test-contract.json',
    'comparison_report': 'comparison/report.json', 'decision': 'decision.json',
    'coverage': 'coverage/coverage.json',
    'extra_family_receipt': 'support-validation/attempt-0001/receipt.json',
    'production_build_record': 'production/build/attempt-0004/record.json',
    'diagnostic_focused_result': 'diagnostic-focused/attempt-0001/result.json',
    'selection_preflight_result': 'selection-preflight/attempt-0001/result.json',
    'profile_repair_record': 'benchmark-profile-repair/attempt-0002/record.json',
    'failed_comparison_preservation_receipt': 'production/compare/attempt-0001/failed-output-preservation.json',
}


def make_contract(repo, output):
    repo = Path(repo).resolve(strict=True)
    base = repo / 'target/performance/P042'
    records = {name: normal(base / path) for name, path in RECORDS.items()}
    records['p041_evidence'] = normal(repo / 'target/performance/P041/closure-attempt-0001/instance-call-evidence.json')
    records['diagnostic_contract'] = normal(repo / 'docs/benchmarks/performance/P042-diagnostic-contract.json')
    validator = repo / 'target/performance/paused/P042-verification-workflow-attempt-0002/drafts/P042-closure-stage/build.py'
    need(validator.is_file(), 'preserved historical P042 reader is missing')
    contract = {'schema_version': 1, 'task': 'P042', 'adapter': 'p042-v1',
                'legacy_validator': normal(validator), 'records': records,
                'limitations': ['Compact remains opt-in; the performance result is inconclusive.',
                                'This migration reuses completed Compact-versus-Compact measurements; it does not refresh Wasmtime comparisons.']}
    write_new(output, contract)
    return contract


class ReferencePlan:
    """The old copy API becomes validation plus a deduplicated destination plan."""
    def __init__(self, output):
        self.output = Path(output).resolve()
        self.bindings = {}

    def copy(self, source, destination, expected=None):
        source = Path(source).resolve(strict=True)
        destination = Path(destination).resolve()
        need(destination.is_relative_to(self.output), 'legacy reader destination escaped its exclusive attempt')
        ref = normal(source)
        need(expected is None or ref['sha256'] == expected, 'legacy payload identity differs')
        name = destination.relative_to(self.output).as_posix()
        prior = self.bindings.get(name)
        need(prior is None or (prior['sha256'], prior['bytes']) == (ref['sha256'], ref['bytes']),
             'conflicting logical fixture identity: ' + name)
        self.bindings[name] = {**ref, 'mode': oct(source.stat().st_mode & 0o7777)}
        # Path is a location hint only; identical content at copied paths shares
        # its identity. Retain the origin for transparent migration provenance.
        return {**ref, 'original_path': str(source), 'original_sha256': ref['sha256']}


def prepare_migration(repo, spec_path, store_root):
    repo = Path(repo).resolve(strict=True)
    spec_path = Path(spec_path).resolve(strict=True)
    spec = read_json(spec_path)
    need(spec.get('schema_version') == 1 and spec.get('task') == 'P042' and spec.get('adapter') == 'p042-v1',
         'unsupported legacy verification contract')
    paths = {name: verify_normal(ref) for name, ref in spec['records'].items()}
    need(set(paths) == set(RECORDS) | {'p041_evidence', 'diagnostic_contract'}, 'incomplete legacy record selection')
    validator_path = verify_normal(spec['legacy_validator'])
    base = repo / 'target/performance/P042'
    number = 1
    while (base / f'closure-attempt-{number:04d}').exists():
        number += 1
    validation_output = base / f'closure-attempt-{number:04d}'
    control = base / 'shared-migrations' / f'attempt-{number:04d}'
    control.mkdir(parents=True, exist_ok=False)
    # Capture the actual adapter and reader before executing validation.
    import shutil
    shutil.copy2(Path(__file__), control / 'adapter.executed.py')
    shutil.copy2(validator_path, control / 'reader.executed.py')
    write_new(control / 'inputs.json', {'contract': normal(spec_path), 'records': spec['records'],
              'reader': spec['legacy_validator'], 'mode': 'reference-only validation; no artifact copies or commands'})
    reader = module(validator_path, 'shared_p042_historical_reader')
    need(reader.ROOT.resolve() == repo, 'legacy reader belongs to another repository')
    args = argparse.Namespace(**paths, output_dir=validation_output)
    plan = ReferencePlan(validation_output)
    reader.copy = plan.copy
    original_ref = reader.ref
    preservation = read_json(paths['failed_comparison_preservation_receipt'])
    def compatible_ref(value, label):
        # The old record deliberately stores this plan location as a string.
        # Normalize that one documented legacy field at the adapter boundary.
        if isinstance(value, str):
            need(value == preservation['preservation_plan'], 'historical plan location differs')
            value = {'path': value, 'sha256': preservation['preservation_plan_sha256'],
                     'bytes': Path(value).stat().st_size}
        return original_ref(value, label)
    reader.ref = compatible_ref
    try:
        # Prove that the compatibility reader cannot launch commands or timing
        # work. Existing pure parsers validate source, fixtures, logs and pairs.
        with patch('subprocess.run', side_effect=AssertionError('migration cannot execute commands')), \
             patch('subprocess.Popen', side_effect=AssertionError('migration cannot execute commands')):
            result = reader.build(args)
        proof = read_json(result['evidence'])
        analysis = read_json(base / 'comparison/analysis.json')
        decision = read_json(paths['decision'])
        need(analysis == decision.get('analysis'), 'retained analysis differs from the completed gate analysis')
        need(decision['source_identity']['comparison_report_sha256'] == sha(paths['comparison_report']), 'gate report identity differs')
        analyzer = module(repo / 'tools/wtime-bench/analyze.py', 'shared_p042_sample_validator')
        report = read_json(paths['comparison_report'])
        analyzer._validate_provenance(report['metadata'])
        _, runs = analyzer._validated_runs(report)
        pairs = sum(len(group) for run in runs for group in run['groups'].values())
        need(len(runs) == 2 and pairs == 33000 and len(analysis['per_run']) == 1100,
             'historical paired samples are incomplete')
        proof.update(pairs=pairs, observations=pairs * 2, analysis_groups=len(analysis['per_run']))
        plan.copy(base / 'comparison/analysis.json', validation_output / 'artifacts/analysis.json')
        # The old reader exports nested refs. Its actual validation checks remain
        # authoritative; only this adapter's new storage/index format changes.
        write_new(control / 'validation.json', {'status': 'passed', 'outcome': proof['outcome'],
                  'validation_index': normal(result['evidence']), 'planned_payloads': plan.bindings,
                  'command_count': 0, 'bootstrap_computations': 0})
    except BaseException as error:
        write_new(control / 'failure.json', {'status': 'failed', 'error': type(error).__name__ + ': ' + str(error)})
        raise
    store = ArtifactStore(store_root)
    objects = store.plan(plan.bindings)
    by_identity = {(ref['sha256'], ref['bytes']): ref for ref in objects.values()}
    record_objects = {}
    for name, path in paths.items():
        key = (sha(path), path.stat().st_size)
        record_objects[name] = by_identity[key]
    record_objects['analysis'] = by_identity[(sha(base / 'comparison/analysis.json'), (base / 'comparison/analysis.json').stat().st_size)]
    write_new(control / 'objects.json', objects)
    receipts = {}
    phase_records = {
        'freeze': ['prepare_receipt', 'p041_evidence'],
        'verify': ['check_result', 'test_contract', 'coverage', 'diagnostic_focused_result',
                   'selection_preflight_result', 'profile_repair_record', 'failed_comparison_preservation_receipt'],
        'measure': ['production_build_record', 'comparison_report'],
        'analyze': ['analysis', 'decision'],
    }
    for phase, names in phase_records.items():
        record = {'schema': 'wtime-imported-verification-phase-v1', 'task': 'P042', 'phase': phase,
                  'status': 'passed', 'origin': 'completed historical producers; validated read-only',
                  'artifacts': {name: record_objects[name] for name in names},
                  'commands_executed_during_migration': 0, 'bootstrap_computations_during_migration': 0,
                  'runtime_source_sha256': proof['runtime_source_sha256'],
                  'benchmark_sources_sha256': proof['candidate_sources_sha256']}
        path = control / (phase + '.json')
        write_new(path, record)
        receipts[phase] = store.add(path)
    prepared = {'schema': 'wtime-prepared-migration-v1', 'task': 'P042', 'status': 'passed',
                'contract': normal(spec_path), 'store': str(store.root), 'phases': receipts,
                'repo': str(repo),
                'objects': objects, 'record_objects': record_objects,
                'summary': {k: v for k, v in proof.items() if k != 'artifact_refs'},
                'validation': normal(control / 'validation.json'), 'adapter': store.add(Path(__file__)),
                'limitations': spec.get('limitations', []), 'source_map': proof['runtime_source_map']}
    prepared_path = control / 'prepared.json'
    write_new(prepared_path, prepared)
    prepared_ref = control / 'prepared.ref.json'
    write_new(prepared_ref, {'store': str(store.root), 'artifact': store.add(prepared_path)})
    return {'prepared': str(prepared_ref), 'status': 'passed', 'objects': len(by_identity), 'logical_artifacts': len(objects)}


def finalize_migration(prepared_path, output):
    prepared_path = Path(prepared_path).resolve(strict=True)
    locator = read_json(prepared_path)
    store = ArtifactStore(locator['store'])
    prepared = store.json(locator['artifact'])
    need(prepared.get('schema') == 'wtime-prepared-migration-v1' and prepared.get('status') == 'passed', 'migration has not passed validation')
    need(str(store.root) == prepared['store'], 'prepared migration refers to a different store')
    checked = store.json(prepared['record_objects']['check_result'])
    need(checked['runtime_source_map'] == prepared['source_map'], 'prepared runtime identity differs')
    repo = Path(prepared['repo']).resolve(strict=True)
    for relative, expected in checked['runtime_source_map'].items():
        need(sha(repo / relative) == expected, 'runtime changed after migration preparation: ' + relative)
    report = store.json(prepared['record_objects']['comparison_report'])
    for relative, expected in report['metadata']['candidate_sources'].items():
        need(sha(repo / relative) == expected, 'measurement input changed after migration preparation: ' + relative)
    for ref in prepared['phases'].values():
        receipt = store.json(ref)
        need(receipt['status'] == 'passed' and receipt['task'] == prepared['task'], 'invalid imported phase receipt')
    seen = set()
    for ref in prepared['objects'].values():
        identity = (ref['path'], ref['stored_sha256'])
        if identity not in seen:
            store.verify(ref)
            seen.add(identity)
    # No legacy reader, command, guest, or statistic is invoked here.
    output = Path(output).resolve()
    need(not output.exists(), 'final migration index already exists')
    output.mkdir(parents=True, exist_ok=False)
    refs = prepared['record_objects']
    compatibility_refs = {name: store.normal_ref(refs[key]) for name, key in {
        'check_result': 'check_result', 'prepare_receipt': 'prepare_receipt',
        'comparison': 'comparison_report', 'decision': 'decision', 'coverage': 'coverage',
        'analysis': 'analysis', 'production_build_record': 'production_build_record',
        'diagnostic_focused_result': 'diagnostic_focused_result',
    }.items()}
    # Existing successor readers consume this thin view; all large payloads live
    # in the shared store and are never embedded or copied into this index.
    summary = prepared['summary']
    proof = {**summary, 'verification_schema': 'wtime-verification-evidence-v1',
             'verification_flow': 'shared manifest finalization', 'phases': prepared['phases'],
             'store': str(store.root), 'artifact_refs': compatibility_refs,
             'artifact_catalog': normal(prepared_path), 'limitations': prepared['limitations'],
             'migration_commands': 0, 'migration_bootstrap_computations': 0,
             'unique_payloads': len(seen), 'logical_artifacts': len(prepared['objects'])}
    path = output / 'indirect-cache-evidence.json'
    write_new(path, proof)
    proposal = output / 'completion.json'
    write_new(proposal, completion(path, prepared['task'], proof['outcome']))
    return {'evidence': str(path), 'completion_record': str(proposal), 'outcome': proof['outcome']}
