#!/usr/bin/env python3
"""Update only the source P128 plan metadata and its exact task-plans snapshot."""
import ast
import collections
import hashlib
import json
import os
import re
import stat
import subprocess
from pathlib import Path

ROOT = Path('/workspace/wtime')
SRC = Path('/workspace/tasks')
DST = ROOT / 'task-plans'
ARCHIVE = DST / 'recovery/P128-metadata-2026-10-14'
TASK_REL = Path('performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md')
JSON_REL = Path('performance/TASKS.json')
MANIFEST = ROOT / 'docs/recovery/task-plan-snapshot.json'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def names_from(path):
    tree = ast.parse(path.read_text(encoding='utf-8'))
    return sorted(n.name for n in ast.walk(tree)
                  if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.name.startswith('test_'))

def update_manifest_entry(manifest, rel):
    src, dst = SRC / rel, DST / rel
    if src.read_bytes() != dst.read_bytes():
        raise RuntimeError(f'snapshot/source mismatch after synchronization: {rel}')
    mode = stat.S_IMODE(src.stat().st_mode)
    os.chmod(dst, mode)
    rec = next((x for x in manifest['files'] if x['path'] == rel.as_posix()), None)
    if rec is None:
        raise RuntimeError(f'missing manifest file record: {rel}')
    rec.update(size_bytes=src.stat().st_size, sha256=sha(src), snapshot_sha256=sha(dst), byte_equal=True,
               mode_octal=format(mode, '04o'))

def main():
    archive = ARCHIVE / 'pre-edit'
    current_inputs = [
        (SRC / JSON_REL, archive / 'original/TASKS.json'),
        (SRC / TASK_REL, archive / 'original/P128-minimal-scalar-workloads.md'),
        (DST / JSON_REL, archive / 'snapshot/TASKS.json'),
        (DST / TASK_REL, archive / 'snapshot/P128-minimal-scalar-workloads.md'),
        (MANIFEST, archive / 'docs/task-plan-snapshot.json'),
    ]
    for current, saved in current_inputs:
        if not current.is_file() or not saved.is_file() or current.read_bytes() != saved.read_bytes():
            raise RuntimeError(f'pre-edit archive mismatch: {current}')

    model_tests = names_from(ROOT / 'tools/wtime-bench/tests/test_p128_scalar_workloads.py')
    mock_tests = names_from(ROOT / 'tools/verification/tests/test_p128_scalar.py')
    identity_tests = names_from(ROOT / 'tools/wtime-bench/tests/test_fast_execution_identity.py')
    if (len(model_tests), len(mock_tests), len(identity_tests)) != (9, 5, 7):
        raise RuntimeError(f'unexpected test counts: {len(model_tests)}, {len(mock_tests)}, {len(identity_tests)}')
    model_meanings = {
        'test_fixed_alias_grid_is_exact_and_registry_stays_459_rows': 'Only the 18 fixed model rows are admitted and the 459-row shared registry and selections remain unchanged.',
        'test_identity_copy_fixture_has_an_independent_n_oracle': 'The neutral copy shape retains its separate observable increment and returns n.',
        'test_literal_add_wraps_as_i32_without_float_or_trapping_ops': 'Literal wrapping addition matches independent signed-i32 arithmetic.',
        'test_literal_bitwise_fixture_matches_independent_mask_oracle': 'Literal AND results match the independent masked oracle.',
        'test_fixture_work_count_is_exactly_n_and_invalid_aliases_fail': 'Each fixed alias has exactly n work units and malformed aliases fail.',
        'test_wat_opcode_shape_rejects_multiply_divide_float_memory_or_calls': 'The fixture opcode shape excludes unrelated and disallowed operations.',
        'test_admitted_wat_and_wasm_bytes_match_frozen_hashes': 'Each admitted WAT and Wasm input matches its frozen hash.',
        'test_rows_bind_profile_modes_stages_oracle_and_mvp_work_units': 'Every row binds its MVP profile, both modes, all five stages, and independent oracle.',
        'test_frozen_registry_rows_and_selections_survive_model_generation': 'Model generation preserves the frozen registry rows and selection membership.',
    }
    mock_meanings = {
        'test_complete_matrix_and_all_outputs_are_contained': 'The mock proof covers all 180 stage/mode observations and keeps outputs inside its fresh root.',
        'test_oracle_mismatch_preserves_raw_output_before_failure': 'Raw output is retained before an oracle mismatch fails the proof.',
        'test_stale_record_rejected_before_guest_commands': 'Stale records or source drift fail before guest commands are run.',
        'test_nonzero_command_output_and_status_are_retained': 'Failed command output and status remain archived.',
        'test_output_escape_is_rejected_before_any_write': 'An output path outside the fresh root is rejected before writing.',
    }
    identity_meanings = {
        'test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs': 'Analysis-only changes remain visible without invalidating legacy execution pairs.',
        'test_runtime_timer_fixture_and_oracle_edits_stale_legacy_pairs': 'Runtime, timer, fixture, and oracle edits invalidate old execution identities.',
        'test_membership_changes_and_unknown_python_modules_fail_closed': 'Unclassified module additions and missing identity inputs fail closed.',
        'test_focused_producer_changes_stale_focused_pairs': 'Focused producer changes invalidate focused execution pairs.',
        'test_verify_inputs_uses_live_sources_without_running_a_comparison': 'Identity verification reads live inputs without launching a comparison.',
        'test_focused_identity_uses_task_declaration_without_registry_selection_lookup': 'Focused identity follows task declaration without registry selection lookup.',
        'test_adapter_archive_corruption_is_still_rejected': 'Tampered adapter archives are rejected.',
    }
    if set(model_tests) != set(model_meanings) or set(mock_tests) != set(mock_meanings) or set(identity_tests) != set(identity_meanings):
        raise RuntimeError('test AST names differ from reviewed names/meanings')

    plan_path = SRC / JSON_REL
    plan = json.loads(plan_path.read_text(encoding='utf-8'))
    task = next(x for x in plan['tasks'] if x['id'] == 'P128')
    p050_before = next(x for x in plan['tasks'] if x['id'] == 'P050')
    p050_bytes = json.dumps(p050_before, sort_keys=True)
    new_owned = [
        'tools/wtime-bench/execution_identity.py',
        'tools/wtime-bench/tests/test_fast_execution_identity.py',
        'tools/verification/producers/p128_evidence.py',
        'tools/verification/tests/data/admitted_wasm/p128_identity_copy.wasm',
        'tools/verification/tests/data/admitted_wasm/p128_literal_wrap_add.wasm',
        'tools/verification/tests/data/admitted_wasm/p128_literal_bitwise.wasm',
    ]
    owned = task['owned_paths']
    task['owned_paths'] = owned + [p for p in new_owned if p not in owned]
    task['fixture_cases'] = model_tests + mock_tests + identity_tests
    task['minimum_focused_cases'] = 21
    task['input_output_contract'] = task['input_output_contract'].replace(
        'The loop count aliases are fixed at `n=0,1,17,32,1024,10000`, producing 18 model rows, never arbitrary ranges.',
        'The loop count aliases are fixed at `n=0,1,17,32,1024,10000`, producing 18 model rows, never arbitrary ranges. Exact dynamic source events, including the implicit function end, follow `9 + 14*n`; the source proof requires 18 rows and 126 source-event/fuel/reference commands (one event command plus six fuel probes per row).')
    task['evidence_contract'] = task['evidence_contract'].replace(
        'all 180 observations by stage/mode;',
        'all 180 observations by stage/mode; the 18-row `9 + 14*n` source-event proof and 126 source-event/fuel/reference commands including implicit function end;')
    if task['status'] != 'todo' or task['outcome'] != 'pending':
        raise RuntimeError('P128 status/outcome changed unexpectedly')
    if json.dumps(p050_before, sort_keys=True) != p050_bytes:
        raise RuntimeError('P050 changed unexpectedly')
    plan_path.write_text(json.dumps(plan, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')

    task_path = SRC / TASK_REL
    doc = task_path.read_text(encoding='utf-8')
    old_cases = re.search(r'## Meaningful cases\n\n([\s\S]*?)\n## Verification', doc)
    if not old_cases:
        raise RuntimeError('Meaningful cases section not found')
    groups = [
        ('Model and fixture tests', model_tests, model_meanings),
        ('Proof mock tests', mock_tests, mock_meanings),
        ('Execution identity tests', identity_tests, identity_meanings),
    ]
    case_lines = []
    for title, names, meanings in groups:
        case_lines.append(f'### {title}\n')
        case_lines.extend(f'- `{name}`: {meanings[name]}' for name in names)
        case_lines.append('')
    replacement = '## Meaningful cases\n\n' + '\n'.join(case_lines).rstrip() + '\n\n'
    doc = doc[:old_cases.start()] + replacement + doc[old_cases.end():]
    doc = doc.replace(
        'Normalize every workload at all five stages (`prepare`, `instantiate`, `first_call`, `warm_call`, `load_and_call`) in `decoded` and `compact` modes using source-current uninstrumented runtime-only binaries. This is exactly **180 real observations/driver commands** (18 shape/input rows × 5 stages × 2 modes).',
        'The independent source-event proof uses `9 + 14*n` dynamic events, counting the implicit function-end event, across 18 rows. It records 126 source-event/fuel/reference commands: one source-event observation and six fuel probes (decoded, compact, and execution-reference at event-count minus one and exact event-count) per row. Separately, normalize every workload at all five stages (`prepare`, `instantiate`, `first_call`, `warm_call`, `load_and_call`) in `decoded` and `compact` modes using source-current uninstrumented runtime-only binaries. This remains exactly **180 real observations/driver commands** (18 shape/input rows × 5 stages × 2 modes).')
    doc = doc.replace(
        'The proof producer archives raw output and command records before oracle checks, independently checks result and modulo arithmetic, and requires every one of the 180 observations.',
        'The proof producer archives raw output and command records before oracle checks, independently checks result and modulo arithmetic, validates the 18-row source-event/fuel proof and its 126 source-event/fuel/reference commands, and requires every one of the 180 staged observations.')
    doc = doc.replace(
        'The named checks must all execute with nonzero assertions.',
        'All 21 named checks (nine model, five proof-mock, and seven execution-identity tests) must execute with nonzero assertions.')
    doc = doc.replace(
        'Close validates fixture identities, full 180-observation coverage, independent oracles, source-current runtime identity, warm batch, and containment.',
        'Close validates fixture identities, the 18-row `9 + 14*n` source-event proof with 126 source-event/fuel/reference commands, full 180-observation staged coverage, independent oracles, source-current runtime identity, warm batch, and containment.')
    doc = doc.replace(
        'all 18 fixed aliases and work counts, independent oracle results, 180 observations by stage/mode,',
        'all 18 fixed aliases and work counts, the `9 + 14*n` source-event formula including implicit function end and all 126 source-event/fuel/reference commands, independent oracle results, 180 observations by stage/mode,')
    task_path.write_text(doc, encoding='utf-8')

    # Exact source-to-snapshot sync for only the two authorized files.
    for rel in (JSON_REL, TASK_REL):
        source, dest = SRC / rel, DST / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(source.read_bytes())
        os.chmod(dest, stat.S_IMODE(source.stat().st_mode))

    # Run validators in original source locations, durably logging command and current source SHA first.
    commands = json.loads((ARCHIVE / 'argv.json').read_text())['validator_commands']
    runs = []
    log = ARCHIVE / 'validator-runs.jsonl'
    for command in commands:
        script = Path(command[-1])
        cwd = SRC if script.parent == SRC else script.parent
        record = {'command': command, 'cwd': str(cwd), 'validator_source': str(script),
                  'validator_source_sha256': sha(script), 'plan_manifest_sha256': sha(script.parent / 'TASKS.json')}
        with log.open('a', encoding='utf-8') as f:
            f.write(json.dumps({'pre_run': record}, sort_keys=True) + '\n'); f.flush(); os.fsync(f.fileno())
        proc = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
        result = {**record, 'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}
        runs.append(result)
        with log.open('a', encoding='utf-8') as f:
            f.write(json.dumps({'result': result}, sort_keys=True) + '\n'); f.flush(); os.fsync(f.fileno())

    manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
    manifest.setdefault('validator_history', []).append(manifest.get('validators', []))
    manifest['validators'] = [{'command': r['command'], 'exit_code': r['exit_code'], 'stdout': r['stdout'],
                               'stderr': r['stderr'], 'validator_source_sha256': r['validator_source_sha256'],
                               'plan_manifest_sha256': r['plan_manifest_sha256']} for r in runs]
    for rel in (JSON_REL, TASK_REL):
        update_manifest_entry(manifest, rel)
    # Recompute all listed-file equality to make the current snapshot claim explicit.
    manifest['snapshot_source_byte_equality'] = all(
        (SRC / row['path']).is_file() and (SRC / row['path']).read_bytes() == (DST / row['path']).read_bytes()
        and sha(SRC / row['path']) == row['sha256'] for row in manifest['files'])
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    if any(r['exit_code'] for r in runs):
        raise RuntimeError('one or more original task validators failed')
    if not manifest['snapshot_source_byte_equality']:
        raise RuntimeError('snapshot/source byte equality check failed')
    print(json.dumps({'suite_counts': [len(model_tests), len(mock_tests), len(identity_tests)],
                      'source_event_formula': '9 + 14*n', 'source_event_rows': 18,
                      'source_event_fuel_reference_commands': 126,
                      'validators': [{'command': r['command'], 'exit_code': r['exit_code'],
                                      'stdout': r['stdout'].strip(), 'stderr': r['stderr'].strip()} for r in runs],
                      'snapshot_byte_equality': manifest['snapshot_source_byte_equality'],
                      'status': task['status'], 'outcome': task['outcome']}, indent=2))

if __name__ == '__main__':
    main()
