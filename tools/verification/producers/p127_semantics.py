"""Run private diagnostic cases beside byte-verified current core copies.

Only an appended cfg(test) module differs in the diagnostic execution.rs.
Production sources remain untouched. The tests execute the real accounting
helper, dispatchers and continuations, without a fake Store or opcode model.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
import p127_fuel_policy as model
import selection_manifest

FEATURES = ['compact-executor', 'execution-reference']
NAMES = [
    'p127_actual_accounting_merges_saved_fuel_once_and_preserves_error_priority',
    'p127_scalar_policies_match_reference_and_refill_each_invocation',
    'p127_low_fuel_prefixes_bind_memory_cursor_and_remaining_budget',
    'p127_real_continuations_preserve_none_some_fuel_transitions',
]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(proof_path, output):
    output = Path(output).resolve()
    if ROOT.resolve() not in output.parents:
        raise ValueError('diagnostic output must be contained in the repository')
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, output / 'helper.executed.py')
    cases = ROOT / 'tools/verification/producers/p127_fuel_cases.rs'
    shutil.copy2(cases, output / 'cases.executed.rs')
    write(output / 'argv.json', {'argv': sys.argv, 'cwd': str(Path.cwd())})
    core = {name: sha(ROOT / name) for name in selection_manifest._current_core_paths(ROOT / 'tools/wtime-bench/workloads.json')}
    proof_path = Path(proof_path).resolve(strict=True)
    if ROOT.resolve() not in proof_path.parents:
        raise ValueError('staged proof is outside the repository')
    proof_sha = sha(proof_path)
    proof = json.loads(proof_path.read_text())
    if proof.get('status') != 'passed' or proof.get('counts', {}).get('observations') != 184:
        raise ValueError('the complete staged metering proof is required')
    rows = model.entries(ROOT / 'tools/wtime-bench/workloads.json')
    scalar = [row for row in rows if row['id'] != 'prepare_large']
    plan = []
    fixture_refs = {}
    for row in scalar:
        path = (proof_path.parent / proof['fixtures'][row['id']]['wasm']).resolve(strict=True)
        if proof_path.parent not in path.parents or sha(path) != row['source']['wasm_sha256']:
            raise ValueError('diagnostic fixture identity differs: ' + row['id'])
        fixture_refs[row['id']] = {'path': str(path), 'sha256': sha(path)}
        plan.append('\t'.join([row['id'], str(path), str(row['input']), str(row['expected'])]))
    plan_path = output / 'scalar-plan.tsv'
    plan_path.write_text('\n'.join(plan) + '\n')
    prefix = ROOT / 'crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm'
    workspace = output / 'workspace'
    workspace.mkdir()
    copied = {}
    for crate in ['wtime-types', 'wtime-decode', 'wtime-validate', 'wtime-runtime']:
        source = ROOT / 'crates' / crate
        dest = workspace / 'crates' / crate
        shutil.copytree(source, dest, ignore=shutil.ignore_patterns('target', '__pycache__'))
        for path in sorted(source.rglob('*')):
            if path.is_file() and 'target' not in path.relative_to(source).parts:
                relative = path.relative_to(ROOT).as_posix()
                actual = dest / path.relative_to(source)
                if sha(actual) != sha(path):
                    raise ValueError('copied core differs: ' + relative)
                copied[relative] = sha(path)
    # Existing cfg(test) core modules also include shared benchmark fixtures.
    # Preserve that compilation closure rather than disabling those modules.
    fixture_root = ROOT / 'tools/wtime-bench/fixtures'
    for path in sorted(fixture_root.rglob('*')):
        if path.is_file() and path.suffix in {'.wasm', '.wat'}:
            relative = path.relative_to(ROOT).as_posix()
            dest = workspace / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dest)
            if sha(dest) != sha(path):
                raise ValueError('copied compilation fixture differs: ' + relative)
            copied[relative] = sha(path)
    execution = workspace / 'crates/wtime-runtime/src/execution.rs'
    original = execution.read_bytes()
    addon = ('\n#[cfg(all(test, feature = "execution-reference", feature = "compact-executor"))]\n'
             '#[path = ' + json.dumps(str(output / 'cases.executed.rs')) + ']\n'
             'mod p127_fuel_cases;\n').encode()
    execution.write_bytes(original + addon)
    assert execution.read_bytes() == original + addon
    root_manifest = (ROOT / 'Cargo.toml').read_text()
    root_manifest = root_manifest.replace('members = ["crates/*", "tools/wtime-conformance"]', 'members = ["crates/*"]')
    root_manifest = re.sub(r'^default-members = .*\n', '', root_manifest, flags=re.M)
    (workspace / 'Cargo.toml').write_text(root_manifest)
    write(output / 'source-binding.json', {'current_core': core, 'copied_original_files': copied,
          'test_only_addition': {'path': 'crates/wtime-runtime/src/execution.rs', 'original_sha256': hashlib.sha256(original).hexdigest(),
                                 'append_sha256': hashlib.sha256(addon).hexdigest(), 'diagnostic_sha256': sha(execution)},
          'cases_sha256': sha(output / 'cases.executed.rs'), 'fixtures': fixture_refs,
          'scalar_plan_sha256': sha(plan_path), 'prefix_sha256': sha(prefix),
          'workspace_manifest_sha256': sha(workspace / 'Cargo.toml')})
    env = dict(os.environ)
    env['CARGO_TARGET_DIR'] = str(ROOT / 'target/verification/build-caches/p127-private-diagnostics')
    env['P127_SCALAR_PLAN'] = str(plan_path)
    env['P127_PREFIX_WASM'] = str(prefix)
    command_results = []
    def command(name, argv):
        write(output / (name + '.argv.json'), {'argv': argv, 'cwd': str(workspace),
              'cargo_target_dir': env['CARGO_TARGET_DIR'], 'scalar_plan': str(plan_path), 'prefix': str(prefix)})
        proc = subprocess.run(argv, cwd=workspace, env=env, capture_output=True, text=True)
        (output / (name + '.stdout')).write_text(proc.stdout)
        (output / (name + '.stderr')).write_text(proc.stderr)
        row = {'name': name, 'exit_code': proc.returncode, 'stdout_sha256': sha(output / (name + '.stdout')), 'stderr_sha256': sha(output / (name + '.stderr'))}
        write(output / (name + '.result.json'), row)
        command_results.append(row)
        if proc.returncode:
            raise ValueError(name + ' failed; logs retained in ' + str(output))
        return proc
    command('generate-lock', ['cargo', 'generate-lockfile', '--offline'])
    lock_sha = sha(workspace / 'Cargo.lock')
    proc = command('diagnostic-tests', ['cargo', 'test', '--locked', '--offline', '-p', 'wtime-runtime', '--lib', '--features', ','.join(FEATURES), 'p127_', '--', '--show-output', '--test-threads=1'])
    sys.path.insert(0, str(ROOT / 'tools'))
    from verification.runner import test_results
    tests = test_results(proc.stdout + '\n' + proc.stderr, {'required_names': NAMES, 'minimum_passed': 4, 'features': FEATURES})
    markers = re.findall(r'P127_ROW\t([^\n]+)', proc.stdout)
    tuples = [row.split('\t') for row in markers]
    wanted = {(row['id'], mode, setting) for row in scalar for mode in ['Decoded', 'Compact', 'Reference'] for setting in ['unmetered', 'metered']}
    actual = {(row[0], row[1], row[2]) for row in tuples}
    if len(tuples) != 54 or actual != wanted or any(len(row) != 5 or int(row[3]) <= 0 or row[4] != '3' for row in tuples):
        raise ValueError('independent scalar proof matrix differs')
    if len(re.findall(r'P127_RESUME\t', proc.stdout)) != 27 or 'P127_PREFIXES\t24\t12' not in proc.stdout or 'P127_ACCOUNTING\t5\t3' not in proc.stdout:
        raise ValueError('missing independent source/fuel proof markers')
    if (core != {name: sha(ROOT / name) for name in core}
            or any(sha(ROOT / name) != digest for name, digest in copied.items())
            or sha(proof_path) != proof_sha or sha(prefix) != json.loads((output / 'source-binding.json').read_text())['prefix_sha256']):
        raise ValueError('current sources or proof inputs changed during diagnostic execution')
    record = {'schema': 'wtime-p127-independent-fuel-proof-v1', 'status': 'passed', 'runtime_core_unchanged': True,
              'current_core': core, 'features': FEATURES, 'instrumented': True, 'timing_evidence': False,
              'tests': tests, 'commands': command_results, 'workspace_lock_sha256': lock_sha,
              'scalar_rows': 9, 'scalar_mode_policy_tuples': 54, 'repeats_per_tuple': 3, 'scalar_guest_calls': 162,
              'independent_reference_probes': 9, 'low_fuel_prefixes': 24, 'cursor_and_remaining_budget_windows': 12,
              'continuation_transition_tuples': 27, 'continuation_guest_calls': 54,
              'accounting_merge_tuples': 5, 'error_priority_tuples': 3,
              'source_fuel_includes_implicit_end': True, 'scalar_source_events': [{'workload': row[0], 'mode': row[1], 'fuel_policy': row[2], 'events': int(row[3])} for row in tuples],
              'staged_proof': {'path': str(proof_path), 'sha256': proof_sha},
              'diagnostic_test_only_append': True}
    write(output / 'report.json', record)
    print(proc.stdout, end='')
    print(json.dumps({'status': 'passed', 'scalar_tuples': 54, 'low_fuel_prefixes': 24, 'resume_tuples': 27}))
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--proof', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.proof, args.output)
    except (OSError, ValueError, KeyError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
