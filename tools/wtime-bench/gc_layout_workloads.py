"""Staged P034 GC-layout model; no registry writes or runtime observations.

Rows use only the three previously encoded CC0 fixtures. Integration must extend
actual gc_scalar_stages dispatcher support for these family/export pairs.
"""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / 'fixtures/gc'
BASE_REGISTRY = Path('/workspace/wtime/tools/wtime-bench/workloads.json')
INPUTS = (0, 1, 17, 32, 1024, 10000)
PRIMARY_INPUTS = (1, 32, 1024, 10000)
HELD_OUT_INPUTS = (0, 17)
TYPE_COUNTS = (16, 256, 1024)
STAGES = ['prepare', 'instantiate', 'first_call', 'warm_call', 'load_and_call']
RESET_POLICY = 'fresh_instance_for_cold_stages_reused_instance_for_warm_call'
REGISTRY_BYTE_CAP = 524288
OLD_ROWS = 350
ADDED_ROWS = 36
OLD_CONTROLS = 81
NEW_SELECTION_COUNT = 117
RUNS = 2
PAIRS_PER_GROUP = 30
CALIBRATION_TARGET_NS = 20000000
CALIBRATION_MAX_ATTEMPTS = 8
EXPECTED_MATRIX = {'groups_per_run': 405, 'groups_total': 810,
                   'pairs_total': 24300, 'warm_calibrations_total': 144,
                   'workload_sides_per_run': 234}
WASMTIME_UNSUPPORTED = (
    'Pinned Wasmtime has no GC.'
)
FIXTURES = {
    16: {'file': 'packed_gc_types_16.wat',
         'wat_sha256': 'f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963',
         'wasm_sha256': '709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f',
         'wasm_size_bytes': 660},
    256: {'file': 'packed_gc_types_256.wat',
          'wat_sha256': '847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c',
          'wasm_sha256': 'e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471',
          'wasm_size_bytes': 6563},
    1024: {'file': 'packed_gc_types_1024.wat',
           'wat_sha256': '5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b',
           'wasm_sha256': '29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46',
           'wasm_size_bytes': 25767},
}
FAMILIES = {
    f'gc_layout_{kind}_{count}': {'kind': kind, 'type_count': count,
        'export': 'run' if kind == 'array' else 'run_struct',
        'unit': 'reads'}
    for kind in ('array', 'struct') for count in TYPE_COUNTS
}


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False,
                      allow_nan=False).encode('utf-8')


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def max_batch(n: int) -> int:
    if type(n) is not int or not 0 <= n <= 10000:
        raise ValueError('n must be a bounded integer in 0..=10000')
    return min(100000, 100000000 // max(n, 1))


def oracle(workload_id: str, n: int) -> int:
    max_batch(n)
    if not isinstance(workload_id, str) or n not in INPUTS:
        raise ValueError('unknown registered P034 request')
    if workload_id not in {f'{family}_n{n}' for family in FAMILIES}:
        raise ValueError('workload family/input mismatch or noncanonical ID')
    # Each packed byte starts at0xff; unsigned access returns255 per read.
    # Independent enumeration rather than invoking the guest/runtime result.
    total = sum(255 for _ in range(n)) & 0xffffffff
    return total - 0x100000000 if total & 0x80000000 else total


def fixture_identities() -> dict[str, dict[str, Any]]:
    found = {}
    for count, record in FIXTURES.items():
        wat = FIXTURE_DIR / record['file']
        wasm = wat.with_suffix('.wasm')
        for path, expected in ((wat, record['wat_sha256']), (wasm, record['wasm_sha256'])):
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
            if actual != expected:
                raise ValueError(f'{path.name}: frozen encoded/source identity mismatch')
        if wasm.stat().st_size != record['wasm_size_bytes']:
            raise ValueError(f'{wasm.name}: encoded size mismatch')
        found[str(count)] = dict(record)
    return found


def entries() -> list[dict[str, Any]]:
    fixture_identities()
    rows = []
    for family, details in FAMILIES.items():
        record = FIXTURES[details['type_count']]
        for n in INPUTS:
            workload_id = f'{family}_n{n}'
            rows.append({
                'id': workload_id, 'family': family,
                'source': {'kind': 'wat_file', 'path': 'fixtures/gc/' + record['file'],
                           'sha256': record['wat_sha256'], 'wasm_sha256': record['wasm_sha256'],
                           'encoding_status': 'encoded'},
                'params': [{'name': 'n', 'type': 'i32', 'min': n, 'max': n, 'baseline': n}],
                'result': {'type': 'i32'},
                'oracle': {'kind': 'python', 'module': 'gc_layout_workloads.py',
                           'function': 'oracle', 'independent': True},
                'required_features': ['gc'],
                'work_units': {'count': n, 'unit': details['unit']},
                'reset_policy': RESET_POLICY, 'stages': STAGES.copy(),
                'driver': {'kind': 'gc_scalar_stages', 'profile': 'all-phase3-core',
                           'export': details['export']},
                'comparators': {'wasmtime': {'status': 'unsupported', 'reason': WASMTIME_UNSUPPORTED}},
                'gc_expectation': {'allocation_count': 1,
                                   'live_after_reset': 0, 'payload_bytes_after_reset': 0,
                                   },
            })
    return rows


def append_to_registry(registry: dict[str, Any]) -> dict[str, Any]:
    """Append36 rows +NEW selection only; preserve all350 old rows/selections."""
    if not isinstance(registry, dict) or not isinstance(registry.get('workloads'), list) or not isinstance(registry.get('selections'), dict):
        raise ValueError('registry shape invalid')
    old_rows = registry['workloads']; old_selections = registry['selections']
    if len(old_rows) != 350:
        raise ValueError('P034 requires frozen350-row immediate predecessor')
    selection = old_selections.get('gc-exceptions')
    if not isinstance(selection, dict) or selection.get('status') != 'ready' or len(selection.get('workloads', [])) != 81:
        raise ValueError('requires unchanged ready81-row gc-exceptions control selection')
    if 'gc-layouts' in old_selections:
        raise ValueError('new gc-layouts selection already present')
    old_ids = [row['id'] for row in old_rows]
    if len(old_ids) != len(set(old_ids)) or not set(selection['workloads']).issubset(old_ids):
        raise ValueError('duplicate/unknown predecessor IDs')
    new = entries(); new_ids = [row['id'] for row in new]
    if set(old_ids).intersection(new_ids):
        raise ValueError('append would duplicate existing IDs')
    result = copy.deepcopy(registry)
    result['workloads'].extend(new)
    result['selections']['gc-layouts'] = {'status': 'ready', 'workloads': selection['workloads'] + new_ids}
    if canonical_bytes(result['workloads'][:350]) != canonical_bytes(old_rows):
        raise AssertionError('old logical workload prefix changed')
    if any(result['selections'][name] != old for name, old in old_selections.items()):
        raise AssertionError('old logical selection changed')
    if len(result['workloads']) != 386 or len(result['selections']['gc-layouts']['workloads']) != 117:
        raise AssertionError('P034 appendix counts incorrect')
    return result


def compact_registry_bytes(registry: dict[str, Any]) -> bytes:
    """Deterministic whole-JSON whitespace reduction; NEVER writes registry."""
    encoded = canonical_bytes(registry) + b'\n'
    if len(encoded) >= REGISTRY_BYTE_CAP:
        raise ValueError('whole registry exceeds strict512KiB cap')
    decoded = json.loads(encoded)
    if canonical_bytes(decoded) != canonical_bytes(registry):
        raise AssertionError('compact encoding changed logical records')
    return encoded


def contract(registry: dict[str, Any]) -> dict[str, Any]:
    """Exact gates._load_contract four-key schema; no measurement extensions."""
    combined = append_to_registry(registry)
    addition = combined['workloads'][OLD_ROWS:]
    primary = [row['id'] for row in addition if row['params'][0]['baseline'] in PRIMARY_INPUTS]
    controls = registry['selections']['gc-exceptions']['workloads'].copy()
    held_out = [row['id'] for row in addition if row['params'][0]['baseline'] in HELD_OUT_INPUTS]
    return {'schema_version': 1, 'selection': 'gc-layouts',
            'primary_workloads': primary, 'protected_workloads': controls + held_out}


def selected_matrix(combined: dict[str, Any]) -> dict[str, int]:
    """Derive stage/calibration counts from actual selected rows, not gate keys."""
    by_id = {row['id']: row for row in combined['workloads']}
    selection = combined['selections']['gc-layouts']
    ids = selection['workloads']
    if len(combined['workloads']) != OLD_ROWS + ADDED_ROWS or selection['status'] != 'ready' or len(ids) != NEW_SELECTION_COUNT or len(ids) != len(set(ids)):
        raise ValueError('P034 selected matrix shape/count mismatch')
    rows = [by_id[wid] for wid in ids]
    for row in rows:
        if row.get('driver', {}).get('kind') == 'gc_scalar_stages':
            if row['stages'] != STAGES:
                raise ValueError('scalar stage matrix changed')
        elif row['stages'] != ['load_and_call']:
            raise ValueError('old cold control stages changed')
    groups = sum(len(row['stages']) for row in rows)
    warm = sum('warm_call' in row['stages'] for row in rows)
    actual = {'groups_per_run': groups, 'groups_total': RUNS * groups,
              'pairs_total': RUNS * groups * PAIRS_PER_GROUP,
              'warm_calibrations_total': RUNS * warm,
              'workload_sides_per_run': 2 * len(rows)}
    if actual != EXPECTED_MATRIX:
        raise ValueError('derived P034 matrix differs from fixed predeclaration')
    return actual


def measurement_contract(registry: dict[str, Any]) -> dict[str, Any]:
    """Rich predeclaration/provenance artifact; NEVER passed as a gate contract."""
    combined = append_to_registry(registry)
    addition = combined['workloads'][OLD_ROWS:]
    gate = contract(registry)
    matrix = selected_matrix(combined)
    return {'schema_version': 1, 'task': 'P034', 'state': 'staged_not_measured',
            'selection': 'gc-layouts', 'candidate_mode': 'decoded', 'profile': 'all-phase3-core',
            'primary_count': len(gate['primary_workloads']),
            'protected_count': len(gate['protected_workloads']), 'selection_count': NEW_SELECTION_COUNT,
            'gate_contract_path': 'contract.draft.json',
            'gate_contract_canonical_sha256': canonical_sha(gate),
            'registry_rows': OLD_ROWS + ADDED_ROWS, 'old_prefix_rows': OLD_ROWS,
            'stages_new_rows': STAGES.copy(), 'runs': RUNS, 'pairs_per_group': PAIRS_PER_GROUP,
            **matrix,
            'group_derivation': {'old_cold': 45, 'old_warm': 36 * len(STAGES), 'new_layout': ADDED_ROWS * len(STAGES)},
            'calibration': {'target_ns': CALIBRATION_TARGET_NS, 'max_attempts': CALIBRATION_MAX_ATTEMPTS,
                            'shared_matched_batch': True,
                            'max_batch_rule': 'min(100000,100000000//max(n,1))'},
            'allocation_rule': 'one object per invocation even n0; collect/release outside each guest interval',
            'old_logical_workloads_sha256': canonical_sha(registry['workloads']),
            'old_logical_selections_sha256': canonical_sha(registry['selections']),
            'new_rows_sha256': canonical_sha(addition),
            'compact_proposed_bytes': len(compact_registry_bytes(combined)),
            'comparison_status': 'not_started', 'registry_installation': 'not_performed',
            'driver_integration': 'must extend current gc_scalar_stages actual family/export handling before execution'}
