"""Revalidate P066's immutable Register adapters and build budgets without rebuilding."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
ATTEMPT = ROOT / 'target/verification/runs/P066/verify/attempt-0001'
ARCHIVE = ROOT / 'target/performance/P066-before'
PINS = {
    'entry_baseline_manifest': '17bcd5768c1f016c621f77c11c84b5a62d9888f4239935a7bba3176fd161e870',
    'entry_candidate_manifest': 'c35860ab64d0f3456852f3b559cda2a206933d827a4e25937a4fba5441dca6aa',
    'typed_adapter_manifest': 'bb162b93a4a7beebb8f0d78ead259c7ca659ca3cdd9b59085a54d188ffc8cd48',
    'default_build': '0817507c1b45f7665d043daded86b03d423bec5b5d7491bd7a5516d0bb53ec7b',
    'register_build': '6dec126a6d8e25631728a661d2a22559614899cd14c496fcf884f9b39ff0e634',
    'inputs': '6f50fbcb9a1fce0d70a963a21764fc99ad124c3cc301f47af0532fb2f8d34c55',
}
HELPERS = (
    'tools/verification/producers/p066_reuse_adapters.py',
    'tools/wtime-bench/entry_batch_adapter.py',
    'tools/wtime-bench/p123_adapter.py',
    'tools/wtime-bench/baseline.py',
    'tools/wtime-bench/compare.py',
    'tools/wtime-bench/analyze.py',
    'tools/wtime-bench/gates.py',
    'tools/checks/build_budget.py',
    'tools/checks/dependency_budget.py',
    'tools/verification/runner.py',
    'tools/verification/artifacts.py',
    'tools/verification/contracts.py',
    'tools/verification/context.py',
)
sys.dont_write_bytecode = True
sys.path[:0] = [str(ROOT), str(BENCH), str(ROOT / 'tools')]

import entry_batch_adapter
import p123_adapter
import gates
from verification.artifacts import ArtifactStore, need
from verification.runner import execution_environment


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def ref(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}


def inside(root: Path, path: Path) -> Path:
    resolved = Path(path).resolve()
    need(resolved.is_relative_to(Path(root).resolve()), f'path escapes authorized root: {resolved}')
    return resolved


def read_json(path: Path) -> dict:
    value = json.loads(Path(path).read_text(encoding='utf-8'))
    need(isinstance(value, dict), f'expected JSON object: {path}')
    return value


def start(output: Path) -> tuple[Path, dict]:
    output = Path(output).absolute()
    verification_root = (ROOT / 'target/verification').resolve(strict=True)
    need(output == output.resolve() and output.is_relative_to(verification_root),
         'P066 reuse output must stay under target/verification')
    inside(verification_root, output.parent).mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=False, exist_ok=False)
    helpers_root = inside(output, output / 'helpers')
    helpers_root.mkdir()
    helpers = {}
    for relative in HELPERS:
        source = ROOT / relative
        target = inside(output, helpers_root / relative)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        helpers[relative] = ref(target)
        need(helpers[relative]['sha256'] == sha(source), f'helper changed while preserved: {relative}')
    invocation = {'argv': list(sys.argv), 'cwd': str(Path.cwd())}
    invocation_path = inside(output, output / 'invocation.json')
    invocation_path.write_text(json.dumps(invocation, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    return output, {'helpers': helpers, 'invocation': ref(invocation_path)}


def _verify_original_result(store: ArtifactStore, side: str, adapter: Path, expected_schema: str,
                            manifest: dict) -> dict:
    result_path = ATTEMPT / f'p066-{side}-adapter.result.json'
    result = read_json(result_path)
    need(type(result.get('exit_code')) is int and result['exit_code'] == 0,
         f'original {side} adapter command did not exit successfully')
    need(result.get('cwd') == str(ROOT), f'original {side} adapter command cwd differs')
    argv = result.get('argv')
    helper_index = next((index for index, item in enumerate(argv)
                         if isinstance(item, str) and Path(item).name == 'entry_batch_adapter.py'), None)
    need(isinstance(argv, list) and len(argv) >= 3 and Path(argv[0]).name.startswith('python')
         and helper_index is not None and helper_index > 0
         and Path(argv[helper_index]).resolve() == (BENCH / 'entry_batch_adapter.py').resolve(),
         f'original {side} adapter command entry point differs')
    need('--output' in argv and Path(argv[argv.index('--output') + 1]).resolve() == adapter.resolve(),
         f'original {side} adapter command output differs')
    for key in ('stdout', 'stderr'):
        need(isinstance(result.get(key), dict), f'original {side} command lacks {key} object reference')
        store.verify(result[key])
    stdout = json.loads(store.read(result['stdout']))
    need(stdout.get('schema') == expected_schema and stdout.get('executable_sha256') == manifest.get('executable_sha256'),
         f'original {side} command result does not bind verified adapter')
    return {'command_result': ref(result_path), 'stdout': result['stdout'], 'stderr': result['stderr']}


def revalidate(output: Path) -> dict:
    output, started = start(output)
    store = ArtifactStore(ROOT / 'target/verification')

    # The immutable input pins are checked before invoking any source-current validators.
    pinned = {
        'entry_baseline_manifest': ATTEMPT / 'entry-baseline/adapter.json',
        'entry_candidate_manifest': ATTEMPT / 'entry-candidate/adapter.json',
        'typed_adapter_manifest': ATTEMPT / 'baseline-adapter/manifest.json',
        'default_build': ATTEMPT / 'default-build.json',
        'register_build': ATTEMPT / 'register-build.json',
        'inputs': ATTEMPT / 'inputs.json',
    }
    pinned_refs = {}
    for name, path in pinned.items():
        need(sha(path) == PINS[name], f'original pinned input changed: {name}')
        pinned_refs[name] = ref(path)

    prior_inputs = read_json(pinned['inputs'])
    need(execution_environment(ROOT, prior_inputs['definition']) == prior_inputs['environment'],
         'P066 verification execution environment differs from original inputs')

    frozen_archive, _ = entry_batch_adapter._load_archive(ARCHIVE)
    need(sha(ARCHIVE / 'manifest.json') == read_json(ATTEMPT / 'entry-baseline/adapter.json')['archive_manifest_sha256'],
         'entry baseline archive manifest changed')
    baseline_root = ATTEMPT / 'entry-baseline'
    candidate_root = ATTEMPT / 'entry-candidate'
    baseline = entry_batch_adapter.verify(baseline_root, ARCHIVE, BENCH)
    candidate = entry_batch_adapter.verify_current(candidate_root, ROOT, BENCH)
    typed_root = ATTEMPT / 'baseline-adapter'
    typed = p123_adapter.verify(typed_root, ARCHIVE, check_current=True)

    need(baseline.get('mode') == candidate.get('mode') == 'register', 'entry adapters are not Register mode')
    need(baseline.get('features') == candidate.get('features') == ['register-executor', 'runtime-only'],
         'entry adapter features differ')
    need(baseline.get('feature_profile') == candidate.get('feature_profile') == 'mvp',
         'entry adapter profile differs')
    need(baseline.get('toolchain') == candidate.get('toolchain'), 'entry adapter toolchains differ')
    need(baseline.get('core_source_map') != candidate.get('core_source_map'),
         'baseline and candidate unexpectedly bind the same core source map')
    need(typed.get('mode') == 'register' and typed.get('legacy_api') is False
         and typed.get('build', {}).get('features') == ['runtime-only', 'register-executor'],
         'frozen typed Register adapter features differ')
    need(typed.get('frozen_core_identities') == p123_adapter.core_identities(frozen_archive),
         'typed adapter frozen core identity map differs from the immutable archive')
    need(typed.get('build', {}).get('rustc') == baseline.get('toolchain', {}).get('rustc_vv')
         and typed.get('build', {}).get('rustc') == candidate.get('toolchain', {}).get('rustc_vv'),
         'typed and entry adapter Rust toolchains differ')

    adapter_results = {
        'entry_baseline': _verify_original_result(store, 'entry-baseline', baseline_root,
            'wtime.source-bound-register-entry-adapter.v1', baseline),
        'entry_candidate': _verify_original_result(store, 'entry-candidate', candidate_root,
            'wtime.source-bound-register-entry-candidate.v1', candidate),
    }

    snapshot = gates._make_build_snapshot()
    default_build = gates._validate_build(pinned['default_build'], expected_mode='default',
                                          label='P066 original default build', snapshot=snapshot)
    register_build = gates._validate_build(pinned['register_build'], expected_mode='optional-feature',
                                           label='P066 original Register build', snapshot=snapshot)
    gates._validate_optional_features(register_build, ['wtime-runtime/register-executor'], pinned['register_build'])
    need(not gates._check_default_dependencies(snapshot), 'current default dependency closure failed')
    need(default_build.get('features') == [], 'P066 original default budget enables features')
    need(register_build.get('features') == ['wtime-runtime/register-executor'],
         'P066 original optional budget feature differs')
    need(register_build.get('feature_package') == 'wtime-runtime'
         and register_build.get('feature_name') == 'register-executor',
         'P066 Register budget does not name the required runtime feature')
    need(default_build.get('compile_environment') == register_build.get('compile_environment') == {},
         'P066 build environment is not the recorded empty environment')
    need(default_build.get('rustc') == register_build.get('rustc')
         and default_build.get('cargo_version') == register_build.get('cargo_version'),
         'P066 build budget toolchains differ')
    need(default_build.get('source_inputs') == register_build.get('source_inputs') == snapshot.source_inputs,
         'P066 build budget source inputs differ from current source')
    need(default_build.get('default_dependency_closure_pass') is True
         and register_build.get('default_dependency_closure_pass') is True,
         'P066 build budget dependency closure failed')
    gates._finish_build_snapshot(snapshot, 'representative', gates._current_source_identity('representative'))

    original_commands = {}
    for name in ('p066-baseline-adapter', 'p066-default-build-budget', 'p066-register-build-budget'):
        path = ATTEMPT / (name + '.result.json')
        record = read_json(path)
        need(type(record.get('exit_code')) is int and record['exit_code'] == 0
             and record.get('cwd') == str(ROOT), 'original build command did not pass')
        for key in ('stdout', 'stderr'):
            store.verify(record[key])
        original_commands[name] = {'result': ref(path), 'stdout': record['stdout'], 'stderr': record['stderr']}

    # Re-run pure validators after build-input capture and guard every input against drift.
    entry_batch_adapter.verify(baseline_root, ARCHIVE, BENCH)
    entry_batch_adapter.verify_current(candidate_root, ROOT, BENCH)
    p123_adapter.verify(typed_root, ARCHIVE, check_current=True)
    for name, path in pinned.items():
        need(sha(path) == PINS[name], f'pinned input changed during revalidation: {name}')
    for relative, reference in started['helpers'].items():
        need(sha(ROOT / relative) == reference['sha256'], f'helper source changed during revalidation: {relative}')

    report = {
        'schema': 'wtime-p066-adapter-reuse-v1',
        'status': 'passed',
        'source_stable': True,
        'timing_evidence': False,
        'guest_or_build_launched': False,
        'adapters': {
            'entry_baseline': {'manifest': ref(baseline_root / 'adapter.json'),
                               'binary': ref(baseline_root / 'binary/wtime-register-entry-adapter'),
                               'original_command': adapter_results['entry_baseline']},
            'entry_candidate': {'manifest': ref(candidate_root / 'adapter.json'),
                                'binary': ref(candidate_root / 'binary/wtime-register-entry-adapter'),
                                'original_command': adapter_results['entry_candidate']},
            'typed_baseline': {'manifest': ref(typed_root / 'manifest.json'),
                               'binary': ref(typed_root / 'binary/wtime-bench')},
        },
        'build_budgets': {
            'default': ref(pinned['default_build']),
            'register': ref(pinned['register_build']),
            'source_sha256': snapshot.source_sha256,
            'source_input_count': len(snapshot.source_inputs),
            'features': {'default': default_build['features'], 'register': register_build['features']},
            'toolchain': {'rustc': default_build['rustc'], 'cargo': default_build['cargo_version']},
            'execution_environment': default_build['compile_environment'],
        },
        'original_commands': original_commands,
        'prior_inputs': ref(pinned['inputs']),
        'execution_environment': prior_inputs['environment'],
        'pinned_inputs': pinned_refs,
        'helpers': started['helpers'],
        'invocation': started['invocation'],
    }
    need(execution_environment(ROOT, prior_inputs['definition']) == prior_inputs['environment'], 'build environment changed during revalidation')
    report_path = inside(output, output / 'report.json')
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = revalidate(args.output)
    print(json.dumps({'status': result['status'], 'guest_or_build_launched': result['guest_or_build_launched']}))

