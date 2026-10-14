#!/usr/bin/env python3
"""Select between the two separately verified P055 Compact dispatch shapes."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

TASK = 'P055'
SOURCE = 'crates/wtime-runtime/src/execution/compact/dispatch_experiment.rs'
REQUIRED_TESTS = {
    'p055_dispatch_shapes_match_original_scalar_operations_and_selected_path',
    'p055_numeric_locals_and_iterative_loops_preserve_source_counts',
    'p055_every_fuel_prefix_barrier_and_materialization_remains_canonical',
}
VARIANTS = {'split': 'Split', 'handlers': 'Handlers'}


def need(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                    allow_nan=False).encode()).hexdigest()


def verify_ref(reference, label):
    need(isinstance(reference, dict) and isinstance(reference.get('path'), str),
         f'{label} file reference is missing')
    path = Path(reference['path']).resolve(strict=True)
    need(path.is_file() and sha(path) == reference.get('sha256'), f'{label} input changed')
    if 'bytes' in reference:
        need(type(reference['bytes']) is int and path.stat().st_size == reference['bytes'],
             f'{label} byte count changed')
    return path


def _required_ref(mapping, name, label):
    need(isinstance(mapping, dict) and name in mapping, f'{label} is missing')
    verify_ref(mapping[name], label)
    return mapping[name]


def _artifact_store_root(receipt_path, label):
    store_root = next((parent for parent in receipt_path.parents
                       if parent.name == 'verification' and parent.parent.name == 'target'), None)
    need(store_root is not None, f'{label} receipt has no verification artifact store')
    return store_root


def _artifact_store(receipt_path, label):
    store_root = _artifact_store_root(receipt_path, label)
    workspace = store_root.parent.parent
    tools_path = str(workspace / 'tools')
    if tools_path not in sys.path:
        sys.path.insert(0, tools_path)
    from verification.artifacts import ArtifactStore
    return ArtifactStore(store_root)


def _validate_semantic_logs(receipt, receipt_path, label):
    store = _artifact_store(receipt_path, label)
    artifacts = receipt.get('artifacts')
    need(isinstance(artifacts, dict) and artifacts, f'{label} semantic artifact archive is empty')
    for name, reference in artifacts.items():
        try:
            store.verify(reference)
        except ValueError as exc:
            raise ValueError(f'{label} receipt artifact {name} is invalid: {exc}') from exc
    commands = {row.get('name'): row for row in receipt.get('commands', [])}
    cargo_names = set()
    for group in receipt['inputs']['definition'].get('tests', []):
        command = commands.get(group.get('command'))
        need(isinstance(command, dict) and command.get('exit_code') == 0,
             f'{label} semantic test command did not pass')
        stdout = store.read(command['stdout'])
        stderr = store.read(command['stderr'])
        output = stdout.decode(errors='replace') + '\n' + stderr.decode(errors='replace')
        from verification.runner import test_results
        result = receipt['tests'].get(group['command'], {})
        checked = test_results(output, group)
        need(result == checked,
             f'{label} semantic test record differs from its immutable log')
        if group.get('kind') != 'unittest':
            cargo_names.update(name.rsplit('::', 1)[-1].rsplit('.', 1)[-1]
                               for name in checked.get('names', []))
    need(REQUIRED_TESTS <= cargo_names, f'{label} semantic Cargo logs omit a required P055 case')


def _read_variant(statistics_path, verify_receipt_path, variant):
    need(variant in VARIANTS, 'unknown P055 dispatch variant')
    stats_path = Path(statistics_path).resolve(strict=True)
    stats = json.loads(stats_path.read_text())
    need(stats.get('schema') == 'wtime-verification-statistics-v1'
         and stats.get('task') == TASK and stats.get('status') == 'passed'
         and stats.get('statistics_computations') == 1 and stats.get('timing_evidence') is True,
         f'{variant} needs one completed P055 statistics result')
    need(isinstance(stats.get('source_binding'), dict), f'{variant} source binding is missing')
    source_binding = stats['source_binding']
    need(source_binding.get('scope') == 'frozen candidate declaration',
         f'{variant} statistics source binding is not candidate-bound')

    inputs = stats.get('inputs')
    need(isinstance(inputs, dict), f'{variant} statistics inputs are missing')
    for name, reference in inputs.items():
        verify_ref(reference, f'{variant} statistics input {name}')
    for name in ('report', 'build', 'optional_build', 'candidate_build', 'contract'):
        need(name in inputs, f'{variant} statistics input {name} is missing')

    report_path = Path(inputs['report']['path']).resolve(strict=True)
    report = json.loads(report_path.read_text())
    need(report.get('schema') == 'wtime-focused-comparison-v1'
         and report.get('lane') == 'candidate' and report.get('passed') is True
         and report.get('source_stable') is True and report.get('selection') == TASK,
         f'{variant} comparison is incomplete or not source stable')
    need(report.get('metadata', {}).get('settings', {}).get('candidate_mode') == 'compact',
         f'{variant} comparison is not Compact')
    decision = stats.get('decision', {}).get('decision')
    need(decision in ('adopted', 'inconclusive', 'rejected'), f'{variant} gate verdict is unknown')

    identities = report.get('identities', {})
    for name in ('selection_manifest', 'selection_freeze', 'baseline_manifest', 'semantic_receipt'):
        _required_ref(identities, name, f'{variant} comparison {name}')
    declaration = report.get('declaration')
    need(isinstance(declaration, dict) and declaration.get('task') == TASK,
         f'{variant} frozen declaration is missing')
    need(report.get('metadata', {}).get('candidate_binary_sha256')
         == identities.get('candidate', {}).get('sha256'),
         f'{variant} candidate binary identity differs from report metadata')

    candidate_build_path = Path(inputs['candidate_build']['path']).resolve(strict=True)
    build_record = json.loads(candidate_build_path.read_text())
    need(build_record.get('schema') == 'wtime-runtime-only-production-v1'
         and build_record.get('mode') == 'compact' and build_record.get('source_stable') is True
         and build_record.get('instrumented') is False,
         f'{variant} Compact production build record is incomplete')
    binary_path = Path(build_record['binary_path']).resolve(strict=True)
    binary_sha = sha(binary_path)
    need(binary_sha == build_record.get('binary_sha256')
         == report['metadata']['candidate_binary_sha256'],
         f'{variant} comparison and production binary identities differ')
    need(build_record.get('source_identities', {}).get(SOURCE),
         f'{variant} production source identity lacks selected dispatch source')
    source_identity = build_record['source_identities'][SOURCE]
    archived_sources = build_record.get('archived_sources', {})
    source_copy = archived_sources.get(SOURCE)
    need(isinstance(source_copy, dict) and source_copy.get('sha256') == source_identity,
         f'{variant} copied dispatch source identity is missing')
    source_path = Path(source_copy.get('path', '')).resolve(strict=True)
    need(sha(source_path) == source_identity, f'{variant} copied dispatch source is stale')
    source_text = source_path.read_text()
    selected_lines = [line.strip() for line in source_text.splitlines()
                      if line.strip().startswith('const SELECTED: Shape =')]
    expected_shape = VARIANTS[variant]
    need(selected_lines == [f'const SELECTED: Shape = Shape::{expected_shape};'],
         f'{variant} copied Rust source does not select Shape::{expected_shape}')
    report_sources = report.get('metadata', {}).get('candidate_sources', {})
    need(report_sources.get(SOURCE) == source_identity,
         f'{variant} comparison source map differs from copied candidate source')
    report_source_dir = report_path.parent / 'sources'
    for relative, expected_sha in report_sources.items():
        archive_path = (report_source_dir / relative).resolve(strict=True)
        need(archive_path.is_relative_to(report_source_dir.resolve()) and sha(archive_path) == expected_sha,
             f'{variant} archived comparison source is stale: {relative}')
    need(build_record.get('source_manifest', {}).get('sha256'),
         f'{variant} production source manifest identity is missing')
    source_manifest_path = Path(build_record['source_manifest']['path']).resolve(strict=True)
    need(sha(source_manifest_path) == build_record['source_manifest']['sha256'],
         f'{variant} production source manifest changed')
    source_manifest = json.loads(source_manifest_path.read_text())
    need(source_manifest.get('source_identities', {}).get(SOURCE) == source_identity
         and source_manifest.get('archived_sources', {}).get(SOURCE, {}).get('sha256') == source_identity,
         f'{variant} production source manifest differs from copied dispatch source')

    semantic_path = Path(identities['semantic_receipt']['path']).resolve(strict=True)
    semantic = json.loads(semantic_path.read_text())
    need(semantic.get('schema') == 'wtime-verification-phase-v1'
         and semantic.get('task') == TASK and semantic.get('phase') == 'verify'
         and semantic.get('status') == 'passed' and semantic.get('lane', 'full') == 'full'
         and semantic.get('synthetic') is False,
         f'{variant} immutable semantic receipt is not a completed full verification')
    semantic_inputs = semantic.get('inputs', {})
    need(canonical_digest(semantic_inputs) == semantic.get('input_key'),
         f'{variant} semantic receipt input key is stale')
    recorded_sources = {}
    for component in semantic_inputs.get('components', {}).values():
        recorded_sources.update(component.get('files', {}))
    core_sources = {name: value for name, value in report_sources.items()
                    if name.startswith('crates/') or name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')}
    need(core_sources and all(recorded_sources.get(name) == value for name, value in core_sources.items()),
         f'{variant} semantic tests ran against different or incomplete candidate sources')
    semantic_test_names = set()
    for group in semantic_inputs.get('definition', {}).get('tests', []):
        result = semantic.get('tests', {}).get(group.get('command'), {})
        need(result.get('failed') == 0 and result.get('ignored') == 0,
             f'{variant} semantic receipt has failed or skipped tests')
        semantic_test_names.update(name.rsplit('::', 1)[-1].rsplit('.', 1)[-1]
                                   for name in result.get('names', []))
    need(REQUIRED_TESTS <= semantic_test_names,
         f'{variant} immutable semantic receipt omits a required P055 case')
    _validate_semantic_logs(semantic, semantic_path, variant)

    receipt_path = Path(verify_receipt_path).resolve(strict=True)
    receipt = json.loads(receipt_path.read_text())
    need(receipt.get('schema') == 'wtime-verification-phase-v1'
         and receipt.get('task') == TASK and receipt.get('phase') == 'verify'
         and receipt.get('status') == 'passed' and receipt.get('lane') == 'full'
         and receipt.get('completion_eligible') is True and receipt.get('synthetic') is False,
         f'{variant} source-current full verification receipt is missing')
    need(canonical_digest(receipt.get('inputs')) == receipt.get('input_key'),
         f'{variant} verification receipt input key is stale')
    source_delta = receipt.get('inputs', {}).get('source_delta')
    need(isinstance(source_delta, dict) and isinstance(source_delta.get('delta'), dict)
         and isinstance(source_delta.get('baseline_sha256'), str),
         f'{variant} source-current stage source delta is missing')
    need(source_delta['baseline_sha256'] == identities['selection_freeze']['sha256'],
         f'{variant} verify receipt is not bound to the comparison freeze')
    need(semantic_path == receipt_path and sha(semantic_path) == sha(receipt_path),
         f'{variant} comparison did not consume its retained verification receipt')
    tests = receipt.get('tests', {}).get('compact-semantics')
    need(isinstance(tests, dict) and tests.get('failed') == 0 and tests.get('ignored') == 0
         and tests.get('passed', 0) >= len(REQUIRED_TESTS)
         and REQUIRED_TESTS <= {name.rsplit('::', 1)[-1] for name in tests.get('names', [])},
         f'{variant} verification skipped a required semantic test')
    outputs = receipt.get('original_outputs', {})
    expected_outputs = {
        'default_build': inputs['build'],
        'compact_build': inputs['optional_build'],
        'production_record': inputs['candidate_build'],
    }
    stage_store = _artifact_store(receipt_path, variant)
    for name, reference in expected_outputs.items():
        output_ref = outputs.get(name)
        need(isinstance(output_ref, dict), f'{variant} verification output {name} is missing')
        verify_ref(output_ref, f'{variant} verification output {name}')
        need(output_ref['sha256'] == reference['sha256'],
             f'{variant} statistics input differs from its verification output {name}')
        archived_output = receipt.get('artifacts', {}).get(name)
        try:
            archived_bytes = stage_store.read(archived_output)
        except ValueError as exc:
            raise ValueError(f'{variant} archived verification output {name} is invalid: {exc}') from exc
        need(archived_bytes == Path(output_ref['path']).read_bytes(),
             f'{variant} verification output archive differs for {name}')
        output_doc = json.loads(Path(output_ref['path']).read_text())
        if name in ('default_build', 'compact_build'):
            need(output_doc.get('budget_pass') is True and output_doc.get('source_stable') is True,
                 f'{variant} {name} build gate did not pass')
        else:
            need(output_doc.get('source_stable') is True and output_doc.get('exit_code') == 0,
                 f'{variant} production build gate did not pass')

    report_ref = inputs['report']
    return {
        'variant': variant,
        'gate_decision': decision,
        'statistics': {'path': str(stats_path), 'sha256': sha(stats_path)},
        'source_binding': source_binding,
        'verification_receipt': {'path': str(receipt_path), 'sha256': sha(receipt_path)},
        'verification_input_key': receipt.get('input_key'),
        'verification_source_delta': source_delta,
        'verification_output_sha256': {name: ref['sha256'] for name, ref in expected_outputs.items()},
        'comparison': {'path': report_ref['path'], 'sha256': report_ref['sha256']},
        'candidate_binary': {'path': str(binary_path), 'sha256': binary_sha},
        'candidate_source': {'path': str(source_path), 'sha256': source_identity,
                             'selected_shape': expected_shape},
        'selection_manifest': identities['selection_manifest'],
        'selection_freeze': identities['selection_freeze'],
        'baseline_manifest': identities['baseline_manifest'],
        'declaration': declaration,
    }


def combine(split_statistics, split_verify_receipt, handlers_statistics, handlers_verify_receipt):
    split = _read_variant(split_statistics, split_verify_receipt, 'split')
    handlers = _read_variant(handlers_statistics, handlers_verify_receipt, 'handlers')
    for name in ('selection_manifest', 'selection_freeze', 'baseline_manifest', 'declaration'):
        need(split[name] == handlers[name], f'P055 variants used different frozen {name}')
    need(split['verification_input_key'] != handlers['verification_input_key'],
         'P055 variant verification stage identities must be distinct')
    need(split['statistics']['sha256'] != handlers['statistics']['sha256'],
         'P055 variant statistics records must be distinct')
    need(split['comparison']['sha256'] != handlers['comparison']['sha256']
         and split['comparison']['path'] != handlers['comparison']['path'],
         'P055 variants must have distinct comparison reports')
    need(split['candidate_binary']['sha256'] != handlers['candidate_binary']['sha256']
         and split['candidate_binary']['path'] != handlers['candidate_binary']['path'],
         'P055 variants must have distinct candidate binaries')
    need(split['candidate_source']['sha256'] != handlers['candidate_source']['sha256'],
         'P055 variants must compile distinct source-selected dispatch shapes')
    need(split['source_binding'] == handlers['source_binding'],
         'P055 statistics source bindings differ')
    need(split['verification_output_sha256'].keys() == handlers['verification_output_sha256'].keys(),
         'P055 verification output sets differ')
    split_sources = split['candidate_source']
    handlers_sources = handlers['candidate_source']
    # The two candidate source maps must describe the same source tree except
    # for the one predeclared compile-time dispatch selector.
    split_report = json.loads(Path(split['comparison']['path']).read_text())
    handlers_report = json.loads(Path(handlers['comparison']['path']).read_text())
    alpha = split_report['metadata']['candidate_sources']
    beta = handlers_report['metadata']['candidate_sources']
    need(alpha.keys() == beta.keys() and set(name for name in alpha if alpha[name] != beta[name]) == {SOURCE},
         'P055 variants changed source identities outside the selected dispatch source')
    split_bytes = Path(split_sources['path']).read_bytes()
    handlers_bytes = Path(handlers_sources['path']).read_bytes()
    need(split_bytes.replace(b'const SELECTED: Shape = Shape::Split;',
                             b'const SELECTED: Shape = Shape::Handlers;') == handlers_bytes,
         'P055 variant sources differ beyond the compile-time SELECTED shape')

    split_stats = json.loads(Path(split['statistics']['path']).read_text())
    handlers_stats = json.loads(Path(handlers['statistics']['path']).read_text())
    allowed_variant_inputs = {'report', 'build', 'optional_build', 'candidate_build'}
    need(split_stats['inputs'].keys() == handlers_stats['inputs'].keys(),
         'P055 statistics input sets differ')
    for name in split_stats['inputs']:
        if name not in allowed_variant_inputs:
            need(split_stats['inputs'][name] == handlers_stats['inputs'][name],
                 f'P055 variants consumed different shared input {name}')

    if split['gate_decision'] == 'adopted':
        decision, selected = 'adopted', 'split'
    elif handlers['gate_decision'] == 'adopted':
        decision, selected = 'adopted', 'handlers'
    elif 'inconclusive' in (split['gate_decision'], handlers['gate_decision']):
        decision, selected = 'inconclusive', None
    else:
        decision, selected = 'rejected', None
    return {
        'schema': 'wtime-p055-dispatch-decision-v1',
        'task': TASK,
        'status': 'passed',
        'source_stable': True,
        'selected_variant': selected,
        'variants': [split, handlers],
        'statistics_computations': 2,
        'aggregate_statistics_computations': 0,
        'additional_statistics_computations': 0,
        'decision': {
            'decision': decision,
            'selected_variant': selected,
            'policy': 'select split if accepted; otherwise select handlers if accepted; otherwise propagate inconclusive; prefer split when both are accepted',
        },
        'timing_evidence': True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--split-statistics', type=Path, required=True)
    parser.add_argument('--split-verify-receipt', type=Path, required=True)
    parser.add_argument('--handlers-statistics', type=Path, required=True)
    parser.add_argument('--handlers-verify-receipt', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = combine(args.split_statistics, args.split_verify_receipt,
                     args.handlers_statistics, args.handlers_verify_receipt)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    print(json.dumps({'status': 'passed', 'decision': result['decision']['decision'],
                      'selected_variant': result['selected_variant']}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
