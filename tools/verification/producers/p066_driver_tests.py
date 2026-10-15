"""Run the source-bound P066 benchmark-harness tests in both feature lanes."""
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
PRODUCTION_FEATURES = ['runtime-only', 'register-executor']
PRODUCTION_NAMES = [
         'p135_entry_batch_parser_rejects_unsupported_counts_and_strategy_pairs',
         'p135_fresh_batch_times_compile_instantiation_and_exact_calls',
         'p135_oracle_checks_every_call_and_stops_on_mismatch',
         'p135_diagnostic_reports_dispatch_deltas_without_timing_or_promotion_claims',
         'p141_sequence_parser_and_build_reject_before_fixture_io',
         'p141_sequence_executes_every_initial_and_revisit_entry',
         'p141_sequence_oracle_and_fifo_recompile_are_checked',
         'p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity',
         'p127_fixed_metered_policy_sets_fuel_without_changing_other_limits',
         'p127_malformed_or_unsupported_fuel_arguments_are_rejected']
NAMES = PRODUCTION_NAMES + [
         'p140_parser_composes_entry_threshold_and_source_fuel',
         'p140_actual_adaptive_engines_preserve_threshold_and_fuel',
         'p140_bad_entry_settings_are_rejected_before_fixture_io',
         'p140_omitted_threshold_preserves_legacy_wire',
         'p127_single_case_builder_uses_selected_fuel_at_guest_source_boundary',
         'p127_scalar_builder_uses_selected_fuel_at_guest_source_boundary']

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write(root, path, value):
    root = Path(root).resolve()
    path = Path(path).resolve()
    path.relative_to(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')

def run(output, features, prepare_only=False):
    if features not in (','.join(FEATURES), ','.join(PRODUCTION_FEATURES)):
        raise ValueError('P066 requires an exact declared harness feature configuration')
    selected = features.split(',')
    output = Path(output).absolute()
    verification_root = (ROOT / 'target/verification').resolve()
    if output != output.resolve() or not output.is_relative_to(verification_root):
        raise ValueError('P066 driver output must stay under target/verification')
    output.parent.resolve().relative_to(verification_root)
    output.mkdir(parents=True, exist_ok=False)
    helper = output / 'helper.executed.py'
    helper.resolve().relative_to(output)
    shutil.copyfile(__file__, helper)
    before = focused_compare.sources()
    write(output, output / 'invocation.json', {'argv': sys.argv, 'cwd': str(Path.cwd()), 'sources': before})
    for name, digest in before.items():
        target = output / 'sources' / name
        target.resolve().relative_to(output)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, target)
        if sha(target) != digest: raise ValueError('driver source changed during copy')
    manifest = '[package]\nname="wtime-p066-driver-tests"\nversion="0.1.0"\nedition="2024"\n[workspace]\n[features]\ndefault=[]\nruntime-only=[]\nlegacy-api=[]\nperformance-counters=[]\nexperimental-executors=[]\n'
    for feature in FEATURES[1:]: manifest += feature + '=["wtime-runtime/' + feature + '"]\n'
    manifest += '[dependencies]\n'
    for crate in ('wtime-runtime', 'wtime-types'):
        manifest += crate + '={path=' + json.dumps(str(ROOT / 'crates' / crate)) + '}\n'
    cargo_file = output / 'Cargo.toml'
    cargo_file.resolve().relative_to(output)
    cargo_file.write_text(manifest)
    src_dir = output / 'src'
    src_dir.resolve().relative_to(output)
    src_dir.mkdir()
    main_file = src_dir / 'main.rs'
    main_file.resolve().relative_to(output)
    main_file.write_text('#![forbid(unsafe_code)]\n#[path=' + json.dumps(str(ROOT / 'tools/wtime-bench/src/protocol.rs')) + ']\nmod protocol;\nfn main() {}\n')
    def command(name, argv):
        command_file = output / (name + '.command.json')
        stdout_file, stderr_file = output / (name + '.stdout'), output / (name + '.stderr')
        for path in (command_file, stdout_file, stderr_file):
            path.resolve().relative_to(output)
        write(output, command_file, {'argv': argv, 'cwd': str(output)})
        with stdout_file.open('x') as stdout, stderr_file.open('x') as stderr:
            result = subprocess.run(argv, cwd=output, stdout=stdout, stderr=stderr)
        write(output, output / (name + '.result.json'), {'exit_code': result.returncode})
        if result.returncode: raise ValueError(name + ' failed: ' + str(output))
        return (output / (name + '.stdout')).read_text(), (output / (name + '.stderr')).read_text()
    command('lock', ['cargo', 'generate-lockfile', '--offline'])
    if prepare_only:
        if focused_compare.sources() != before:
            raise ValueError('driver sources changed during preparation')
        write(output, output / 'prepared.json', {
            'status': 'passed', 'source_stable': True, 'features': selected,
            'sources': before, 'manifest_sha256': sha(cargo_file),
            'helper': {'path': str(helper), 'sha256': sha(helper)},
            'instrumented': 'execution-reference' in selected, 'timing_evidence': False})
        print(json.dumps({'status': 'passed', 'prepared': True}))
        return
    stdout, stderr = command('tests', ['cargo', 'test', '--locked', '--offline', '--features', features, '--', '--nocapture'])
    sys.path.insert(0, str(ROOT / 'tools'))
    from verification.runner import test_results
    required_names = NAMES if selected == FEATURES else PRODUCTION_NAMES
    tests = test_results(stdout + '\n' + stderr, {'required_names': required_names, 'minimum_passed': 18 if selected == FEATURES else 12,
                                                'features': selected, 'unfiltered': True})
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
    write(output, output / 'report.json', {'status': 'passed', 'source_stable': True, 'features': selected,
                                 'tests': tests, 'sources': before, 'binaries': binaries,
                                 'instrumented': 'execution-reference' in selected, 'timing_evidence': False})
    print(stdout, end='')
    print(forwarded_stderr, end='', file=sys.stderr)

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--features', required=True)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    run(args.output, args.features, args.prepare_only)
