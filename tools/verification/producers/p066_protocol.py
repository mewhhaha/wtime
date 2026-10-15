"""Source-bound non-promoting representative Register protocol probes for P066."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
SELECTION = ROOT / 'docs/benchmarks/performance/P066-selection.json'
THRESHOLD = 3
BATCH = 4
HELPERS = (
    'tools/verification/producers/p066_protocol.py',
    'tools/verification/producers/p066_driver_tests.py',
    'tools/verification/production_attestation.py',
    'tools/verification/producers/runtime_production.py',
    'tools/wtime-bench/focused_compare.py',
    'tools/wtime-bench/selection_manifest.py',
    'tools/wtime-bench/p140_entry_policy.py',
    'tools/wtime-bench/representative_compare.py',
    'tools/wtime-bench/representative_scaling.py',
    'tools/wtime-bench/representative_workloads.py',
)
LANES = (('decoded', 'decoded', None),
         ('register-eager', 'register', None),
         ('register-adaptive-3', 'register', THRESHOLD))

sys.dont_write_bytecode = True
for path in (ROOT, BENCH):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import focused_compare
import p140_entry_policy
import representative_compare
import representative_scaling
import representative_workloads
import selection_manifest
from tools.verification import production_attestation


class P066ProtocolError(ValueError):
    pass


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P066ProtocolError(message)


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inside(root: Path, path: Path) -> Path:
    resolved = Path(path).resolve()
    if not resolved.is_relative_to(Path(root).resolve()):
        raise P066ProtocolError(f'output escapes its authorized root: {resolved}')
    return resolved


def write_new(root: Path, path: Path, payload: bytes) -> Path:
    path = inside(root, path)
    inside(root, path.parent).mkdir(parents=True, exist_ok=True)
    with path.open('xb') as stream:
        stream.write(payload)
    return path


def write_json(root: Path, path: Path, value) -> Path:
    return write_new(root, path,
                     (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n').encode())


def ref(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}


def start(output: Path) -> Path:
    output = Path(output).absolute()
    verification_root = (ROOT / 'target/verification').resolve()
    need(output == output.resolve() and output.is_relative_to(verification_root),
         'P066 protocol output must stay under target/verification')
    inside(verification_root, output.parent).mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=False, exist_ok=False)
    helpers_dir = inside(output, output / 'helpers')
    helpers_dir.mkdir()
    for name in HELPERS:
        source = ROOT / name
        destination = inside(output, helpers_dir / Path(name).name)
        shutil.copyfile(source, destination)
    write_json(output, output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output


def validate_production(record_path: Path) -> tuple[dict, dict, dict, Path]:
    record_path = Path(record_path).resolve(strict=True)
    record = json.loads(record_path.read_text(encoding='utf-8'))
    declaration = selection_manifest.load(SELECTION)
    setting = p140_entry_policy.request(declaration, 'register', THRESHOLD)
    protocol = record.get('protocol')
    p140_entry_policy.capability(protocol.get('register_entry_threshold_contract')
                                 if isinstance(protocol, dict) else None)
    metadata = {
        'settings': {'candidate_mode': 'register', 'instrumented': False,
                     'register_entry_threshold': setting},
        'candidate_sources': record.get('source_identities'),
        'candidate_binary_sha256': record.get('binary_sha256'),
        'capabilities': {'candidate': protocol},
    }
    validation_report = {'passed': True, 'source_stable': True, 'declaration': declaration}
    attested = production_attestation.validate_runtime_record(
        record_path, metadata, validation_report, ROOT)
    binary = Path(record['binary_path']).resolve(strict=True)
    need(binary.is_file(), 'production executable is missing')
    need(record.get('mode') == 'register' and record.get('runtime_features') == ['register-executor'],
         'production record is not the actual Register-only build')
    return record, declaration, metadata, binary


def _run(output: Path, binary: Path, name: str, argv: list[str], *, expected_success: bool) -> tuple[dict | None, str, str, int]:
    command_path = output / f'{name}.command.json'
    stdout_path, stderr_path = output / f'{name}.stdout', output / f'{name}.stderr'
    write_json(output, command_path, {'argv': [str(binary), *map(str, argv)], 'cwd': str(ROOT)})
    stdout_path, stderr_path = inside(output, stdout_path), inside(output, stderr_path)
    with stdout_path.open('xb') as stdout, stderr_path.open('xb') as stderr:
        completed = subprocess.run([str(binary), *map(str, argv)], cwd=ROOT,
                                   stdout=stdout, stderr=stderr, text=False, check=False)
    result = {'exit_code': completed.returncode}
    write_json(output, output / f'{name}.result.json', result)
    raw_stdout, raw_stderr = stdout_path.read_bytes(), stderr_path.read_bytes()
    if expected_success:
        need(completed.returncode == 0, f'{name} production probe failed')
        try:
            wire = json.loads(raw_stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise P066ProtocolError(f'{name} did not emit one JSON wire object') from exc
        need(isinstance(wire, dict) and raw_stderr == b'', f'{name} output shape differs')
        return wire, raw_stdout.decode(), raw_stderr.decode(), completed.returncode
    need(completed.returncode != 0, f'{name} malformed option unexpectedly succeeded')
    need(b'cannot read representative fixture' not in raw_stderr,
         f'{name} reached fixture IO before rejecting malformed options')
    return None, raw_stdout.decode(errors='replace'), raw_stderr.decode(errors='replace'), completed.returncode


def _fixture_paths(entries: list[dict]) -> dict[str, dict]:
    paths = {}
    for entry in entries:
        source = entry['source']
        wat = (BENCH / source['path']).resolve(strict=True)
        wasm = wat.with_suffix('.wasm').resolve(strict=True)
        need(sha(wat) == source['sha256'] and sha(wasm) == source['wasm_sha256'],
             'representative fixture source or encoded bytes changed: ' + entry['id'])
        for path, digest in ((wat, source['sha256']), (wasm, source['wasm_sha256'])):
            logical = path.relative_to(ROOT).as_posix()
            need(focused_compare.sources().get(logical) == digest,
                 'focused source map omits or changes representative fixture: ' + logical)
        paths[entry['id']] = {'wat': wat, 'wasm': wasm,
                              'wat_sha256': source['sha256'], 'wasm_sha256': source['wasm_sha256']}
    return paths


def _base_args(wasm: Path, entry: dict, mode: str) -> list[str]:
    size = entry['params'][0]['baseline']
    expected = representative_workloads.oracle(entry['id'], size)
    return ['--representative-staged', str(wasm), entry['id'], str(size),
            'i32:' + str(expected['result']), expected['output_sha256'],
            'run_after_setup', str(BATCH), mode, 'mvp']


def run(production_path: Path, output_path: Path) -> dict:
    output = start(output_path)
    before_sources = focused_compare.sources()
    before_helpers = {name: sha(ROOT / name) for name in HELPERS}
    record, declaration, metadata, binary = validate_production(production_path)
    need(record.get('source_identities') == before_sources,
         'production record does not bind the current focused source map')
    entries = representative_workloads.entries()
    selected = {entry['id']: entry for entry in selection_manifest.rows(declaration)
                if entry['oracle'].get('module') == 'representative_workloads.py'}
    need(set(selected) == {entry['id'] for entry in entries},
         'P066 frozen selection must bind all nine representative rows')
    need(all(entry['stages'] == ['run_after_setup'] for entry in selected.values()),
         'P066 representative rows must retain the setup-excluded stage')
    entries = [selected[entry['id']] for entry in entries]
    fixtures = _fixture_paths(entries)
    binary_ref = ref(binary)
    production_ref = ref(production_path)
    observations = []
    for entry in entries:
        fixture = fixtures[entry['id']]
        for lane, mode, threshold in LANES:
            argv = _base_args(fixture['wasm'], entry, mode)
            if threshold is not None:
                argv += ['--register-entry-threshold', str(threshold)]
            wire, _, _, _ = _run(output, binary, f'{lane}-{entry["id"]}', argv,
                                 expected_success=True)
            validated = representative_compare.validate_observation(
                entry, wire, representative_workloads, representative_scaling, BATCH,
                expected_mode=mode, register_entry_threshold=threshold)
            inside(output, output / 'memory-images')
            retained = representative_compare.retain_memory_image(validated, output)
            observations.append({'lane': lane, 'workload': entry['id'], 'batch': BATCH,
                                 'threshold': threshold, 'fixture': {
                                     'wat_sha256': fixture['wat_sha256'],
                                     'wasm_sha256': fixture['wasm_sha256']},
                                 'wire_stdout': ref(output / f'{lane}-{entry["id"]}.stdout'),
                                 'wire': retained})

    missing_fixture = inside(output, output / 'malformed-option-must-not-read.wasm')
    invalid = []
    sample = entries[0]
    base = _base_args(missing_fixture, sample, 'register')
    invalid_args = [
        ('zero', base + ['--register-entry-threshold', '0']),
        ('negative', base + ['--register-entry-threshold', '-1']),
        ('overflow', base + ['--register-entry-threshold', '18446744073709551616']),
        ('duplicate', base + ['--register-entry-threshold', '3', '--register-entry-threshold', '4']),
        ('mode-mismatch', _base_args(missing_fixture, sample, 'decoded') + ['--register-entry-threshold', '3']),
        ('missing-value', base + ['--register-entry-threshold']),
    ]
    for name, argv in invalid_args:
        _, stdout, stderr, exit_code = _run(output, binary, 'invalid-' + name, argv,
                                            expected_success=False)
        need('cannot read representative fixture' not in stderr,
             'invalid Register command reached fixture IO: ' + name)
        invalid.append({'name': name, 'exit_code': exit_code,
                        'stdout_sha256': hashlib.sha256(stdout.encode()).hexdigest(),
                        'stderr_sha256': hashlib.sha256(stderr.encode()).hexdigest()})

    need(focused_compare.sources() == before_sources, 'focused sources changed during P066 protocol probes')
    need({name: sha(ROOT / name) for name in HELPERS} == before_helpers,
         'P066 protocol helper changed during execution')
    need(all(sha(output / 'helpers' / Path(name).name) == before_helpers[name]
             for name in HELPERS), 'archived P066 protocol helper differs from executed source')
    need(ref(binary) == binary_ref and ref(production_path) == production_ref,
         'P066 production record or executable changed during probes')
    need(_fixture_paths(entries) == fixtures, 'P066 fixture identities changed during probes')
    helper_refs = {Path(name).name: ref(output / 'helpers' / Path(name).name) for name in HELPERS}
    report = {'schema': 'wtime-p066-representative-protocol-v1', 'status': 'passed',
              'source_stable': True, 'outcome': 'contract_only', 'timing_evidence': False,
              'gate_eligible': False, 'candidate_promoted': False,
              'production': production_ref, 'binary': binary_ref,
              'selection': ref(SELECTION),
              'threshold': THRESHOLD, 'batch': BATCH,
              'counts': {'representative_rows': len(entries), 'lanes': len(LANES),
                         'valid_observations': len(observations), 'invalid_probes': len(invalid)},
              'observations': observations, 'invalid_probes': invalid,
              'helpers': helper_refs,
              'fixtures': {name: {'wat': ref(value['wat']), 'wasm': ref(value['wasm'])}
                           for name, value in fixtures.items()},
              'production_protocol_sha256': sha(Path(record['protocol_stdout'])),
              'candidate_sources_sha256': hashlib.sha256(json.dumps(
                  metadata['candidate_sources'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()}
    write_json(output, output / 'report.json', report)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--production', type=Path, required=True,
                        help='source-bound runtime production build-record.json')
    parser.add_argument('--output', type=Path, required=True,
                        help='new output directory under target/verification')
    args = parser.parse_args(argv)
    try:
        report = run(args.production, args.output)
    except (P066ProtocolError, production_attestation.AttestationError, OSError,
            KeyError, TypeError, ValueError) as exc:
        print(f'P066 representative protocol failed: {exc}', file=sys.stderr)
        return 1
    print(json.dumps({'status': report['status'], 'counts': report['counts'],
                      'timing_evidence': False, 'gate_eligible': False}, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
