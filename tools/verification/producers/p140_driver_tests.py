"""Run the actual benchmark harness tests without the Wasmtime dependency."""
from pathlib import Path
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
import focused_compare

FEATURES = ['runtime-only', 'compact-executor', 'register-executor', 'execution-reference']
NAMES = ['p140_parser_composes_entry_threshold_and_source_fuel',
         'p140_actual_adaptive_engines_preserve_threshold_and_fuel',
         'p140_bad_entry_settings_are_rejected_before_fixture_io',
         'p140_omitted_threshold_preserves_legacy_wire']

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')

def run(output, features):
    if features != ','.join(FEATURES):
        raise ValueError('P140 requires the exact declared combined-feature harness')
    output = Path(output).absolute()
    if output != output.resolve() or not output.is_relative_to(ROOT / 'target/verification'):
        raise ValueError('P140 driver output must stay under target/verification')
    output.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(__file__, output / 'helper.executed.py')
    before = focused_compare.sources()
    write(output / 'invocation.json', {'argv': sys.argv, 'cwd': str(Path.cwd()), 'sources': before})
    for name, digest in before.items():
        target = output / 'sources' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, target)
        if sha(target) != digest: raise ValueError('driver source changed during copy')
    manifest = '[package]\nname="wtime-p140-driver-tests"\nversion="0.1.0"\nedition="2024"\n[workspace]\n[features]\ndefault=[]\nruntime-only=[]\nlegacy-api=[]\nperformance-counters=[]\nexperimental-executors=[]\n'
    for feature in FEATURES[1:]: manifest += feature + '=["wtime-runtime/' + feature + '"]\n'
    manifest += '[dependencies]\n'
    for crate in ('wtime-runtime', 'wtime-types'):
        manifest += crate + '={path=' + json.dumps(str(ROOT / 'crates' / crate)) + '}\n'
    (output / 'Cargo.toml').write_text(manifest)
    (output / 'src').mkdir()
    (output / 'src/main.rs').write_text('#![forbid(unsafe_code)]\n#[path=' + json.dumps(str(ROOT / 'tools/wtime-bench/src/protocol.rs')) + ']\nmod protocol;\nfn main() {}\n')
    def command(name, argv):
        write(output / (name + '.command.json'), {'argv': argv, 'cwd': str(output)})
        with (output / (name + '.stdout')).open('x') as stdout, (output / (name + '.stderr')).open('x') as stderr:
            result = subprocess.run(argv, cwd=output, stdout=stdout, stderr=stderr)
        write(output / (name + '.result.json'), {'exit_code': result.returncode})
        if result.returncode: raise ValueError(name + ' failed: ' + str(output))
        return (output / (name + '.stdout')).read_text(), (output / (name + '.stderr')).read_text()
    command('lock', ['cargo', 'generate-lockfile', '--offline'])
    stdout, stderr = command('tests', ['cargo', 'test', '--locked', '--offline', '--features', features, '--', '--nocapture'])
    sys.path.insert(0, str(ROOT / 'tools'))
    from verification.runner import test_results
    tests = test_results(stdout + '\n' + stderr, {'required_names': NAMES, 'minimum_passed': 15, 'features': FEATURES})
    binaries = {}
    forwarded_stderr = stderr
    for match in re.finditer(r'^\s*Running .*\(([^()]+)\)\s*$', stderr, re.MULTILINE):
        path = Path(match.group(1))
        if not path.is_absolute(): path = output / path
        resolved = str(path.resolve(strict=True))
        binaries[resolved] = sha(path)
        # Cargo runs inside the generated harness; its retained raw log stays
        # unchanged. Report absolute executable paths to the outer verifier.
        forwarded_stderr = forwarded_stderr.replace('(' + match.group(1) + ')', '(' + resolved + ')')
    if not binaries or focused_compare.sources() != before: raise ValueError('driver binary or source stability proof is missing')
    write(output / 'report.json', {'status': 'passed', 'source_stable': True, 'features': FEATURES,
                                 'tests': tests, 'sources': before, 'binaries': binaries,
                                 'instrumented': True, 'timing_evidence': False})
    print(stdout, end='')
    print(forwarded_stderr, end='', file=sys.stderr)

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--features', required=True)
    args = parser.parse_args()
    run(args.output, args.features)
