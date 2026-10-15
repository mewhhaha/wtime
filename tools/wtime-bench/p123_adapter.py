"""Source-bound typed harness builds against immutable archived core crates.

This adapter changes the measurement harness only. Cargo resolves every core
crate to the verified archive, never to the current runtime tree.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
KIND = 'frozen-typed-adapter'
CORE = ('wtime-types', 'wtime-decode', 'wtime-validate', 'wtime-runtime')
REGISTER_ENTRY_CONTRACT = {
    'schema_version': 1,
    'argument': '--register-entry-threshold',
    'mode': 'register',
    'minimum': 1,
    'maximum': 18446744073709551615,
}
ENTRYPOINT = ('#![forbid(unsafe_code)]\nmod protocol;\nfn main() {\n'
    ' let args=std::env::args().skip(1).collect::<Vec<_>>();\n'
    ' if !protocol::try_main(&args) { std::process::exit(2); }\n}\n')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def core_identities(frozen):
    return {name: row['sha256'] for name, row in frozen['files'].items()
            if row.get('role') == 'source'
            and any(name.startswith('source/crates/'+crate+'/') for crate in CORE)}


def harness_identities():
    import compare
    return {name: digest for name, digest in compare.sources(
        include_memory=True, include_simd=True, include_representative=True,
        include_integration=True).items() if name.startswith('tools/wtime-bench/')}


def compiled_harness_identities():
    """Hash only the current Cargo and Rust inputs that build the adapter."""
    paths = [HERE / 'Cargo.toml', HERE / 'Cargo.lock']
    paths.extend((HERE / 'src').rglob('*.rs'))
    return {path.relative_to(ROOT).as_posix(): sha(path)
            for path in sorted(set(paths)) if path.is_file()}


def adapter_features(mode, legacy_api):
    if type(legacy_api) is not bool:
        raise ValueError('typed adapter legacy-api flag must be boolean')
    if mode not in ('decoded', 'compact', 'register'):
        raise ValueError('unsupported adapter mode')
    if legacy_api and mode != 'decoded':
        raise ValueError('legacy API supports decoded mode only')
    return ['runtime-only'] + (['legacy-api'] if legacy_api else []) + (
        ['compact-executor'] if mode == 'compact' else []) + (
        ['register-executor'] if mode == 'register' else [])


def validate_mode_contract(record, cargo_manifest, smoke):
    mode, legacy = record.get('mode'), record.get('legacy_api')
    try:
        features = adapter_features(mode, legacy)
    except ValueError as exc:
        raise ValueError('typed adapter mode/feature contract differs') from exc
    cargo_features = cargo_manifest.get('features', {})
    expected_definitions = {'default': features, 'runtime-only': [], 'legacy-api': []}
    if not legacy:
        expected_definitions.update({
            'compact-executor': ['wtime-runtime/compact-executor'],
            'register-executor': ['wtime-runtime/register-executor'],
        })
    historical = {k: v for k, v in expected_definitions.items() if k != 'register-executor'}
    allowed_definitions = [expected_definitions] if mode == 'register' else [expected_definitions, historical]
    if (record.get('build', {}).get('features') != features
            or cargo_features not in allowed_definitions):
        raise ValueError('typed adapter mode/feature contract differs')
    if (not isinstance(smoke, dict) or smoke.get('instrumented') is not False
            or mode not in smoke.get('modes', []) or smoke.get('engine') != 'wtime'):
        raise ValueError('typed adapter production mode was not probed')
    if mode == 'register':
        capability = smoke.get('register_entry_threshold_contract')
        if (not isinstance(capability, dict) or capability != REGISTER_ENTRY_CONTRACT
                or any(type(capability.get(key)) is not int for key in ('schema_version', 'minimum', 'maximum'))):
            raise ValueError('typed adapter Register threshold capability was not probed')


def _cargo_target_args(target):
    return ['--target-dir', str(target)]


def verify(path, baseline=None, *, check_current=True):
    import baseline as baseline_tool
    path = Path(path).resolve()
    record = baseline_tool.verify_archive(path)
    if record.get('kind') != KIND or record.get('schema') != 1:
        raise ValueError('unsupported typed adapter manifest')
    frozen_path = Path(record.get('baseline', '')).resolve()
    if baseline is not None and frozen_path != Path(baseline).resolve():
        raise ValueError('typed adapter baseline path differs')
    frozen = baseline_tool.verify_archive(frozen_path)
    if sha(frozen_path/'manifest.json') != record.get('baseline_manifest_sha256'):
        raise ValueError('typed adapter baseline manifest identity differs')
    if core_identities(frozen) != record.get('frozen_core_identities'):
        raise ValueError('typed adapter frozen core identities differ')
    if type(check_current) is not bool:
        raise ValueError('typed adapter current-source flag must be boolean')
    shared = record.get('shared_harness_identities')
    if not isinstance(shared, dict) or not shared:
        raise ValueError('typed adapter archived harness identity is missing')
    archived_sources = {
        row.get('source_path', name.removeprefix('source/')): row['sha256']
        for name, row in record['files'].items()
        if name.startswith('source/tools/wtime-bench/') and row.get('role') == 'source'
    }
    if archived_sources != shared:
        raise ValueError('typed adapter pristine harness archive differs from its manifest')
    # baseline.verify_archive above checked exact archive membership, bytes and
    # modes. This second check explicitly binds the copied source tree to the
    # adapter's declared shared-harness identity.
    for name, digest in shared.items():
        if sha(path/'source'/name) != digest:
            raise ValueError('typed adapter pristine harness differs: '+name)
    if check_current:
        current = compiled_harness_identities()
        archived_compiled = {
            name: digest for name, digest in shared.items()
            if name in ('tools/wtime-bench/Cargo.toml', 'tools/wtime-bench/Cargo.lock')
            or (name.startswith('tools/wtime-bench/src/') and name.endswith('.rs'))
        }
        if current != archived_compiled:
            raise ValueError('typed adapter current Cargo/Rust harness differs')
        source_root = HERE/'src'
    else:
        source_root = path/'source/tools/wtime-bench/src'
    for source in sorted(source_root.glob('*.rs')):
        if source.name == 'main.rs':
            continue
        if sha(path/'src'/source.name) != sha(source):
            raise ValueError('typed adapter compiled harness differs: '+source.name)
    if (path/'src/main.rs').read_text() != ENTRYPOINT:
        raise ValueError('typed adapter entrypoint differs')
    if record.get('protocol_sha256') != sha(source_root/'protocol.rs'):
        raise ValueError('typed adapter protocol identity differs')
    manifest = tomllib.loads((path/'Cargo.toml').read_text())
    mode = record.get('mode')
    validate_mode_contract(record, manifest, record.get('protocol'))
    for name in ('wtime-runtime', 'wtime-types'):
        expected = str(frozen_path/'source/crates'/name)
        if manifest['dependencies'][name]['path'] != expected:
            raise ValueError('typed adapter Cargo substituted core sources')
    resolution = record['build']['core_resolution']
    if set(resolution) != set(CORE):
        raise ValueError('typed adapter core resolution is incomplete')
    for name, source in resolution.items():
        if source != str(frozen_path/'source/crates'/name/'Cargo.toml'):
            raise ValueError('typed adapter Cargo core resolution differs')
    binary = path/'binary/wtime-bench'
    if (record.get('binary') != 'binary/wtime-bench' or not os.access(binary, os.X_OK)
            or sha(binary) != record.get('binary_sha256')):
        raise ValueError('typed adapter executable identity differs')
    return record


def _run(command, cwd, logdir, name, env=None):
    (logdir/(name+'.command.json')).write_text(json.dumps(
        {'argv':command, 'cwd':str(cwd)}, indent=2)+'\n')
    with (logdir/(name+'.stdout')).open('x') as stdout, (logdir/(name+'.stderr')).open('x') as stderr:
        proc = subprocess.run(command, cwd=cwd, env=env, stdout=stdout, stderr=stderr)
    (logdir/(name+'.status.json')).write_text(json.dumps({'exit_code':proc.returncode})+'\n')
    if proc.returncode:
        raise RuntimeError(f'{name} failed with {proc.returncode}; see {logdir}')
    return (logdir/(name+'.stdout')).read_text()


def create(baseline, output, *, mode='decoded', legacy_api=True):
    import baseline as baseline_tool
    baseline = Path(baseline).resolve()
    output = Path(output).resolve()
    frozen = baseline_tool.verify_archive(baseline)
    if output.exists():
        return verify(output, baseline)
    before_core = core_identities(frozen)
    before_harness = harness_identities()
    before_manifest = sha(baseline/'manifest.json')
    features = adapter_features(mode, legacy_api)
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, output/'helper.executed.py')
    logs = output/'logs'; logs.mkdir()
    feature_lines = 'default = '+json.dumps(features)+'\nruntime-only = []\nlegacy-api = []\n'
    if not legacy_api:
        feature_lines += 'compact-executor = ["wtime-runtime/compact-executor"]\n'
        feature_lines += 'register-executor = ["wtime-runtime/register-executor"]\n'
    runtime = baseline/'source/crates/wtime-runtime'
    types = baseline/'source/crates/wtime-types'
    cargo = ('[package]\nname = "wtime-frozen-typed"\nversion = "0.1.0"\n'
        'edition = "2024"\npublish = false\n[workspace]\n[features]\n'+feature_lines+
        '[dependencies]\nwtime-runtime = { path = '+json.dumps(str(runtime))+' }\n'
        'wtime-types = { path = '+json.dumps(str(types))+' }\n')
    (output/'Cargo.toml').write_text(cargo)
    (output/'src').mkdir()
    for source in sorted((HERE/'src').glob('*.rs')):
        if source.name != 'main.rs':
            (output/'src'/source.name).write_bytes(source.read_bytes())
    # The main.rs identity is explicitly different; it removes the Wasmtime
    # host harness and forwards only the verified single-observation protocol.
    (output/'src/main.rs').write_text(ENTRYPOINT)
    for name in before_harness:
        dest = output/'source'/name; dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes((ROOT/name).read_bytes())
    for source in (baseline/'fixtures').glob('*'):
        if source.is_file():
            dest = output/'fixtures'/source.name; dest.parent.mkdir(exist_ok=True)
            shutil.copy2(source, dest)
    # A fresh target prevents old archive timestamps reusing candidate objects.
    with tempfile.TemporaryDirectory(prefix='p123-build-', dir=output.parent) as target:
        manifest = str(output/'Cargo.toml')
        _run(['cargo','generate-lockfile','--offline','--manifest-path',manifest], ROOT, logs, 'lock')
        metadata = json.loads(_run(['cargo','metadata','--locked','--offline','--format-version','1',
                                  '--manifest-path',manifest], ROOT, logs, 'metadata'))
        packages = metadata['packages']
        if {p['name'] for p in packages} != {*CORE, 'wtime-frozen-typed'} or any(p['source'] is not None for p in packages):
            raise ValueError('typed adapter resolved external/substituted dependencies')
        resolution = {p['name']:str(Path(p['manifest_path']).resolve()) for p in packages if p['name'] in CORE}
        for name, path in resolution.items():
            if path != str(baseline/'source/crates'/name/'Cargo.toml'):
                raise ValueError('typed adapter resolved a current core crate')
        _run(['cargo','build','--locked','--offline','--release','--manifest-path',manifest,
              '-j','4',*_cargo_target_args(target)], ROOT, logs, 'build')
        (output/'binary').mkdir()
        shutil.copy2(Path(target)/'release/wtime-frozen-typed', output/'binary/wtime-bench')
    protocol = json.loads(_run([str(output/'binary/wtime-bench'),'--protocol'], ROOT, logs, 'protocol'))
    toolchain = _run(['rustc','-Vv'], ROOT, logs, 'toolchain')
    if (before_core != core_identities(baseline_tool.verify_archive(baseline))
            or before_manifest != sha(baseline/'manifest.json') or before_harness != harness_identities()):
        raise ValueError('frozen core or harness changed during adapter build')
    files = {}
    for path in sorted(output.rglob('*')):
        if path.is_file():
            name = path.relative_to(output).as_posix()
            files[name] = {'sha256':sha(path), 'size_bytes':path.stat().st_size,
                           'mode_bits':stat.S_IMODE(path.stat().st_mode),
                           'role':'source' if name.startswith('source/') else 'typed_adapter_artifact'}
            if name.startswith('source/'):
                files[name]['source_path'] = name.removeprefix('source/')
    record = {'schema':1, 'kind':KIND, 'baseline':str(baseline), 'binary':'binary/wtime-bench',
        'baseline_manifest_sha256':before_manifest, 'frozen_core_identities':before_core,
        'shared_harness_identities':before_harness, 'protocol_sha256':sha(HERE/'src/protocol.rs'),
        'binary_sha256':sha(output/'binary/wtime-bench'), 'mode':mode, 'legacy_api':legacy_api,
        'protocol':protocol, 'build':{'features':features, 'core_resolution':resolution,
        'rustc':toolchain, 'external_dependencies':0, 'fresh_target':True}, 'files':files}
    (output/'manifest.json').write_text(json.dumps(record, indent=2)+'\n')
    return verify(output, baseline)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['build','verify'])
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--mode', choices=['decoded','compact','register'], default='decoded')
    parser.add_argument('--current-api', action='store_true')
    args=parser.parse_args()
    record = create(args.baseline,args.output,mode=args.mode,legacy_api=not args.current_api) if args.operation=='build' else verify(args.output,args.baseline)
    print(json.dumps({'kind':record['kind'],'mode':record['mode'],'binary_sha256':record['binary_sha256']}))


if __name__=='__main__':
    main()
