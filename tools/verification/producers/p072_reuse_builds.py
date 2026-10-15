"""Revalidate and stage immutable successful P072 build products without building."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
ATTEMPT = ROOT / 'target/verification/runs/P072/verify/attempt-0001'
ARCHIVE = ROOT / 'target/performance/P072-before'
VERIFICATION_ROOT = ROOT / 'target/verification'
PINS = {
    'inputs': '9398a4c8a3bbad22170648c8d4143d51fd7ddc3eb8efc8858376f18df1917219',
    'baseline_adapter_manifest': '42d7ea44f1ef12ce5beaacb9304da9a0d39d67f81ff25f2cb0f2fd12ca21c65a',
    'production_record': '7cdc7ae6d0f6d5820da522801d5b28d7f7407f81fedf15318a8ad381c3d1b6b6',
    'production_binary': 'f5112aad50edbfceb84876af8ca07b95a222841fc3082901e6bfce17cb1c91cc',
    'production_protocol': '16b22ce0ce0b84ae8d0b33cdd976eb06e678f0520b75bf5344651a4b1ca436df',
    'build_budget': '6bf21720129e0242a5effe0e582019d18c5775a8d39087cbc29e17ad121080bc',
    'baseline_adapter_command': 'd207ee0ec7901ed60a7cc52930ab23127df37eedd3d1713f3133f0721906901f',
    'production_command': '63aad3107425d76c4e718f86b0cf1d387c96993f9db97424dc4267ccb3b9adb7',
    'build_budget_command': '61d28dd69bdc244c6a05ae74b6c110fd05f87d6a81a9806b62967f1e8a37924e',
    'dependency_budget_command': '8433fcdc58179a1075317db2c931c95a44a0f541d5d65b22d04f6980e2372515',
    'archive_manifest': '995a4cb27dfd9fe78c59f6127848ddf6e0988ce3a3f8c7ba5a03a898d657c912',
}
HELPERS = (
    'tools/verification/producers/p072_reuse_builds.py',
    'tools/verification/producers/candidate_production.py',
    'tools/wtime-bench/p123_adapter.py',
    'tools/wtime-bench/baseline.py',
    'tools/wtime-bench/compare.py',
    'tools/wtime-bench/execution_identity.py',
    'tools/wtime-bench/gates.py',
    'tools/checks/build_budget.py',
    'tools/checks/dependency_budget.py',
    'tools/verification/runner.py',
    'tools/verification/artifacts.py',
    'tools/verification/contracts.py',
    'tools/verification/context.py',
)
sys.dont_write_bytecode = True


class ReuseError(ValueError):
    pass


def need(condition: bool, message: str) -> None:
    if not condition:
        raise ReuseError(message)


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inside(root: Path, path: Path) -> Path:
    resolved = Path(path).resolve()
    need(resolved.is_relative_to(Path(root).resolve()), f'path escapes contained output: {resolved}')
    return resolved


def ref(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}


def read_json(path: Path) -> dict:
    value = json.loads(Path(path).read_text(encoding='utf-8'))
    need(isinstance(value, dict), f'expected JSON object: {path}')
    return value


def _write_json(root: Path, path: Path, value: object) -> None:
    path = inside(root, path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def _copy_file(root: Path, source: Path, destination: Path) -> None:
    source = Path(source).resolve(strict=True)
    destination = inside(root, destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    need(sha(source) == sha(destination), f'copied artifact differs: {source}')


def _copy_tree(root: Path, source_root: Path, destination_root: Path) -> dict[str, dict]:
    source_root = Path(source_root).resolve(strict=True)
    need(source_root.is_dir() and source_root.is_relative_to(ATTEMPT.resolve()),
         'source product directory escapes the immutable original attempt')
    destination_root = inside(root, destination_root)
    destination_root.mkdir(parents=True, exist_ok=False)
    copied = {}
    for source in sorted(source_root.rglob('*')):
        relative = source.relative_to(source_root)
        destination = inside(root, destination_root / relative)
        if source.is_dir():
            destination.mkdir(parents=True, exist_ok=True)
        elif source.is_file():
            _copy_file(root, source, destination)
            copied[relative.as_posix()] = ref(destination)
        else:
            raise ReuseError(f'unsupported source artifact type: {source}')
    return copied


def _start(output: Path) -> tuple[Path, dict]:
    output = Path(output).absolute()
    verification_root = VERIFICATION_ROOT.resolve(strict=True)
    need(output == output.resolve() and output.is_relative_to(verification_root)
         and output != verification_root and not output.exists(),
         'P072 reuse output must be a fresh target/verification child')
    inside(verification_root, output.parent).mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=False, exist_ok=False)

    preserved = {}
    helpers_root = inside(output, output / 'helpers')
    helpers_root.mkdir()
    for relative in HELPERS:
        source = (ROOT / relative).resolve(strict=True)
        need(source.is_relative_to(ROOT), f'helper source escapes repository: {relative}')
        destination = inside(output, helpers_root / relative)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        preserved[relative] = ref(destination)
        need(preserved[relative]['sha256'] == sha(source), f'helper changed while preserving: {relative}')
    invocation = {'argv': list(sys.argv), 'cwd': str(Path.cwd())}
    invocation_path = inside(output, output / 'invocation.json')
    _write_json(output, invocation_path, invocation)
    return output, {'helpers': preserved, 'invocation': ref(invocation_path)}


def _verify_current_inputs(build: dict) -> dict[str, str]:
    """Check the recorded Cargo package input set and every source byte without Cargo."""
    source_inputs = build.get('source_inputs')
    need(isinstance(source_inputs, dict) and source_inputs
         and source_inputs == build.get('source_inputs_after')
         and source_inputs == build.get('isolated_source_inputs_after'),
         'P072 build budget source input snapshots differ')
    need(build.get('source_sha256') == build.get('source_sha256_after'),
         'P072 build budget source digest changed during its original build')

    # Recover the package-root inventory from the successful build's recorded
    # Cargo graph. This checks for both changed files and unrecorded additions,
    # without invoking Cargo or another command.
    from urllib.parse import unquote, urlparse
    import os
    package_roots = set()
    for row in build.get('resolved_graph', []):
        package_id = row.get('package') if isinstance(row, dict) else None
        if isinstance(package_id, str) and package_id.startswith('path+file://'):
            package = Path(unquote(urlparse(package_id[len('path+'):]).path)).resolve()
            if package.is_relative_to(ROOT):
                package_roots.add(package)
    need(package_roots, 'P072 build budget has no local Cargo package roots')

    current_paths = {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock'}
    for path in (ROOT / 'rust-toolchain.toml', ROOT / 'rust-toolchain', ROOT / '.cargo'):
        if path.is_file():
            current_paths.add(path)
        elif path.is_dir():
            current_paths.update(child for child in path.rglob('*') if child.is_file())
    skip = {'.git', 'target', 'node_modules', '__pycache__'}
    for package_root in package_roots:
        for current, directories, filenames in os.walk(package_root):
            directories[:] = sorted(name for name in directories if name not in skip)
            for filename in filenames:
                path = Path(current) / filename
                if path.is_file():
                    current_paths.add(path)
    current = {path.relative_to(ROOT).as_posix() for path in current_paths if path.is_file()}
    need(current == set(source_inputs),
         'current Cargo source input inventory differs from the retained P072 budget')
    for relative, expected in source_inputs.items():
        path = (ROOT / relative).resolve(strict=True)
        need(path.is_relative_to(ROOT) and sha(path) == expected,
             f'P072 build input is stale: {relative}')

    from tools.checks import build_budget
    need(build_budget.hash_inputs(source_inputs) == build.get('source_sha256'),
         'P072 build budget source digest does not match its inputs')
    return source_inputs


def _verify_normal_command(name: str, expected_script: Path, expected_args: list[str], store,
                           *, expected_output: Path | None = None) -> dict:
    result_path = ATTEMPT / f'{name}.result.json'
    command_path = ATTEMPT / f'{name}.command.json'
    result = read_json(result_path)
    command = read_json(command_path)
    need(result.get('exit_code') == 0 and result.get('cwd') == str(ROOT),
         f'original {name} command did not succeed in the repository root')
    need(command.get('argv') == result.get('argv') and command.get('cwd') == result.get('cwd'),
         f'original {name} command/result invocation differs')
    argv = result.get('argv')
    need(isinstance(argv, list) and len(argv) >= 3 and Path(argv[2]).resolve() == expected_script.resolve(),
         f'original {name} command entry point differs')
    for token in expected_args:
        need(token in argv, f'original {name} command omitted {token!r}')
    if expected_output is not None:
        need('--output' in argv and Path(argv[argv.index('--output') + 1]).resolve() == expected_output.resolve(),
             f'original {name} command output path differs')
    stream_refs = {}
    for stream in ('stdout', 'stderr'):
        reference = result.get(stream)
        need(isinstance(reference, dict), f'original {name} command lacks {stream} artifact reference')
        data = store.read(reference)
        log_path = ATTEMPT / f'{name}.{stream}'
        need(log_path.is_file() and log_path.read_bytes() == data,
             f'original {name} {stream} log differs from its immutable artifact reference')
        stream_refs[stream] = reference
    return {'result': ref(result_path), 'command': ref(command_path),
            'stdout_log': ref(ATTEMPT / f'{name}.stdout'),
            'stderr_log': ref(ATTEMPT / f'{name}.stderr'), 'artifact_streams': stream_refs}


def _verify_failure_record() -> dict:
    path = ATTEMPT / 'failure.json'
    failure = read_json(path)
    need(failure.get('status') == 'failed'
         and failure.get('error') == 'VerificationError: required full suite was filtered',
         'original P072 verification did not fail only at the expected required-suite filter')
    commands = failure.get('commands')
    need(isinstance(commands, list) and commands
         and all(type(row.get('exit_code')) is int and row['exit_code'] == 0 for row in commands),
         'an original P072 verification command failed before the required-suite filter')
    need({row.get('name') for row in commands} >= {
        'live-policy-tests', 'live-gate-tests', 'prepare-protocol', 'protocol-tests',
        'runtime-unit-tests', 'ledger-tier-tests', 'baseline-adapter', 'production-build',
        'build-budget', 'dependency-budget'},
        'original P072 attempt omitted a successful required verification command')
    return {'reference': ref(path), 'reason': failure['error'],
            'successful_commands': len(commands)}


def revalidate(output: Path) -> dict:
    output, started = _start(output)

    # Imports happen after their exact helper sources and invocation were copied.
    sys.path[:0] = [str(ROOT), str(ROOT / 'tools/wtime-bench'), str(ROOT / 'tools')]
    from tools.verification.artifacts import ArtifactStore
    from tools.verification.runner import execution_environment
    from tools.verification.producers import candidate_production
    import p123_adapter
    import gates
    from tools.checks import build_budget

    pinned_paths = {
        'inputs': ATTEMPT / 'inputs.json',
        'baseline_adapter_manifest': ATTEMPT / 'baseline-adapter/manifest.json',
        'production_record': ATTEMPT / 'production/build-record.json',
        'production_binary': ATTEMPT / 'production/bin/wtime-bench',
        'production_protocol': ATTEMPT / 'production/protocol.stdout.json',
        'build_budget': ATTEMPT / 'build.json',
        'baseline_adapter_command': ATTEMPT / 'baseline-adapter.result.json',
        'production_command': ATTEMPT / 'production-build.result.json',
        'build_budget_command': ATTEMPT / 'build-budget.result.json',
        'dependency_budget_command': ATTEMPT / 'dependency-budget.result.json',
        'archive_manifest': ARCHIVE / 'manifest.json',
    }
    for name, path in pinned_paths.items():
        need(sha(path) == PINS[name], f'original pinned P072 input changed: {name}')

    original_inputs = read_json(pinned_paths['inputs'])
    need(execution_environment(ROOT, original_inputs['definition']) == original_inputs['environment'],
         'P072 original verification execution environment changed')
    need(sha(pinned_paths['archive_manifest']) == PINS['archive_manifest'],
         'P072 immutable predecessor archive changed')

    store = ArtifactStore(VERIFICATION_ROOT)
    original_commands = {
        'baseline-adapter': _verify_normal_command(
            'baseline-adapter', ROOT/'tools/wtime-bench/p123_adapter.py',
            ['build', '--current-api', '--mode', 'decoded', '--baseline', str(ARCHIVE), '--output'],
            store, expected_output=ATTEMPT/'baseline-adapter'),
        'production-build': _verify_normal_command(
            'production-build', ROOT/'tools/verification/producers/candidate_production.py',
            ['--mode', 'decoded', '--output'], store, expected_output=ATTEMPT/'production'),
        'build-budget': _verify_normal_command(
            'build-budget', ROOT/'tools/checks/build_budget.py', ['--output'], store,
            expected_output=ATTEMPT/'build.json'),
        'dependency-budget': _verify_normal_command(
            'dependency-budget', ROOT/'tools/checks/dependency_budget.py', [], store),
    }
    _verify_failure_record()

    baseline_root = ATTEMPT / 'baseline-adapter'
    adapter = p123_adapter.verify(baseline_root, ARCHIVE, check_current=True)
    need(adapter.get('mode') == 'decoded' and adapter.get('legacy_api') is False
         and adapter.get('build', {}).get('features') == ['runtime-only'],
         'original P072 baseline adapter is not the source-current decoded configuration')
    need(adapter.get('protocol', {}).get('instrumented') is False
         and 'decoded' in adapter.get('protocol', {}).get('modes', []),
         'original P072 baseline adapter protocol differs')

    production_root = ATTEMPT / 'production'
    record_path = production_root / 'build-record.json'
    production = read_json(record_path)
    production_protocol_path = production_root / 'protocol.stdout.json'
    protocol_wire_path = production_root / 'protocol.stdout'
    protocol = read_json(production_protocol_path)
    need(production.get('schema') == 'wtime-production-command-v1'
         and production.get('phase') == 'build' and production.get('exit_code') == 0
         and production.get('source_stable') is True and production.get('cwd') == str(ROOT),
         'original P072 production build record is not successful and source-stable')
    need(production.get('feature_flags') == [] and protocol.get('instrumented') is False
         and protocol.get('engine') == 'wtime' and 'decoded' in protocol.get('modes', []),
         'original P072 production build is not an uninstrumented decoded binary')
    need(sha(production_root/'bin/wtime-bench') == production.get('binary_sha256')
         and production.get('binary_sha256') == PINS['production_binary']
         and Path(production.get('binary_path', '')).resolve() == (production_root/'bin/wtime-bench').resolve(),
         'original P072 production binary identity differs')
    need(sha(protocol_wire_path) == production.get('protocol_sha256')
         and sha(production_protocol_path) == production.get('protocol_sha256')
         and protocol_wire_path.read_bytes() == production_protocol_path.read_bytes(),
         'original P072 production protocol bytes differ')
    need(production.get('protocol_command') == [str(production_root/'bin/wtime-bench'), '--protocol']
         and production.get('protocol_exit') == 0,
         'original P072 production protocol command differs')
    for name, expected in (('build', production.get('command')),
                           ('protocol', production.get('protocol_command'))):
        command = read_json(production_root / f'{name}.command.json')
        status = read_json(production_root / f'{name}.status.json')
        need(command.get('argv') == expected and command.get('cwd') == str(ROOT)
             and status.get('exit_code') == 0,
             f'original P072 production {name} command log differs')

    current_core = candidate_production._core_source_map(ROOT)
    need(production.get('core_source_map') == current_core,
         'original P072 production core sources are stale')
    need(production.get('runtime_source_map') == {
        name: digest for name, digest in current_core.items()
        if name.startswith('crates/wtime-runtime/src/')},
        'original P072 production runtime source map differs')
    need(production.get('source_identities') == candidate_production._source_identity(ROOT),
         'original P072 production harness/source identity is stale')
    need(sha(ROOT/'tools/verification/producers/candidate_production.py') == production.get('helper_sha256')
         and sha(production_root/'helper.executed.py') == production.get('helper_sha256'),
         'original P072 production producer source differs')
    metadata = {'settings': {'candidate_mode': 'decoded', 'instrumented': False},
                'candidate_binary_sha256': production['binary_sha256'],
                'candidate_sources': production['source_identities'],
                'capabilities': {'candidate': protocol}}
    gates._validate_candidate_build_record(record_path, metadata, {}, expected_root=ROOT)

    build = read_json(pinned_paths['build_budget'])
    source_inputs = _verify_current_inputs(build)
    snapshot = type('RetainedSnapshot', (), {
        'source_inputs': source_inputs,
        'source_sha256': build_budget.hash_inputs(source_inputs),
    })()
    validated_build = gates._validate_build(pinned_paths['build_budget'], expected_mode='default',
                                             label='P072 retained default build budget', snapshot=snapshot)
    need(validated_build.get('features') == [] and validated_build.get('compile_environment') == {}
         and validated_build.get('default_dependency_closure_pass') is True
         and validated_build.get('default_dependency_violations') == [],
         'original P072 default build budget/dependency closure differs')
    budget_stdout = json.loads(store.read(read_json(ATTEMPT/'build-budget.result.json')['stdout']))
    need(budget_stdout.get('mode') == 'default' and budget_stdout.get('budget_pass') is True
         and budget_stdout.get('source_stable') is True,
         'original P072 build-budget command output differs')

    # The two successful build products and budget record are copied unchanged
    # to this fresh verification subdirectory for later gates.
    baseline_copy = output / 'baseline-adapter'
    production_copy = output / 'production'
    baseline_files = _copy_tree(output, baseline_root, baseline_copy)
    production_files = _copy_tree(output, production_root, production_copy)
    build_copy = output / 'build.json'
    _copy_file(output, pinned_paths['build_budget'], build_copy)
    evidence_files = {}
    for name in ('baseline-adapter', 'production-build', 'build-budget', 'dependency-budget'):
        for suffix in ('command.json', 'result.json', 'stdout', 'stderr'):
            source = ATTEMPT / f'{name}.{suffix}'
            destination = output / 'evidence' / source.name
            _copy_file(output, source, destination)
            evidence_files[source.name] = ref(destination)

    for relative, reference in started['helpers'].items():
        need(sha(ROOT/relative) == reference['sha256'], f'P072 reuse helper changed while validating: {relative}')
    for name, path in pinned_paths.items():
        need(sha(path) == PINS[name], f'original pinned P072 input changed during revalidation: {name}')
    need(sha(baseline_copy/'manifest.json') == PINS['baseline_adapter_manifest']
         and sha(production_copy/'build-record.json') == PINS['production_record']
         and sha(production_copy/'bin/wtime-bench') == PINS['production_binary']
         and sha(build_copy) == PINS['build_budget'],
         'staged P072 build product differs from the immutable original')

    result = {
        'schema': 'wtime-p072-build-reuse-v1',
        'status': 'passed',
        'source_stable': True,
        'timing_evidence': False,
        'guest_or_build_launched': False,
        'original_verify_failure': _verify_failure_record(),
        'pinned_originals': {name: ref(path) for name, path in pinned_paths.items()},
        'source_current': {
            'baseline_adapter_mode': adapter['mode'],
            'baseline_adapter_features': adapter['build']['features'],
            'baseline_adapter_compiled_harness_count': len(p123_adapter.compiled_harness_identities()),
            'baseline_adapter_frozen_core_identities': adapter['frozen_core_identities'],
            'production_core_source_map': production['core_source_map'],
            'production_binary_sha256': production['binary_sha256'],
            'production_protocol_sha256': production['protocol_sha256'],
            'build_budget_source_sha256': build['source_sha256'],
            'build_budget_source_input_count': len(source_inputs),
        },
        'staged_products': {
            'baseline_adapter_manifest': ref(baseline_copy/'manifest.json'),
            'baseline_adapter_files': baseline_files,
            'production_record': ref(production_copy/'build-record.json'),
            'production_binary': ref(production_copy/'bin/wtime-bench'),
            'production_files': production_files,
            'build_budget': ref(build_copy),
        },
        'original_commands': original_commands,
        'evidence_files': evidence_files,
        'helpers': started['helpers'],
        'invocation': started['invocation'],
    }
    report_path = inside(output, output/'reuse.json')
    _write_json(output, report_path, result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = revalidate(args.output)
    print(json.dumps({'status': report['status'], 'guest_or_build_launched': report['guest_or_build_launched']}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
