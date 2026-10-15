"""Revalidate immutable successful adapter builds without compiling again."""
from pathlib import Path
import argparse
import json
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
from tools.verification.producers import p139, p142_freeze as freezing
from tools.verification.runner import execution_environment
import entry_batch_adapter as adapter
import entry_sequence_compare as pairs

ATTEMPT = ROOT / 'target/verification/runs/P142/verify/attempt-0001'
PINS = {'baseline': '8244b9df05b7af05154be5f9a610de562a16464bb0d967415ffe81747cc7b420',
        'candidate': '890b3ab7f31d363112e37062388c5c547e400100f16e2bbfea9738628b83f5d5'}
HELPERS = ['tools/verification/producers/p142_reuse_builds.py', *freezing.HELPERS,
           'tools/wtime-bench/entry_batch_adapter.py', 'tools/wtime-bench/entry_sequence_compare.py',
           'tools/wtime-bench/entry_sequence_policy.py', 'tools/wtime-bench/checkpoint.py',
           'tools/verification/runner.py']

def verify(baseline, candidate, output):
    output = freezing.start(output, HELPERS)
    roots = {'baseline': baseline.resolve(strict=True), 'candidate': candidate.resolve(strict=True)}
    p139.need(roots == {side: ATTEMPT / side for side in PINS}, 'P142 build reuse roots differ')
    prior = p139.read_json(ATTEMPT / 'inputs.json', 'retained build environment')
    environment = execution_environment(ROOT, {'commands': [], 'environment': [
        'CARGO_HOME', 'RUSTUP_HOME', 'PATH', 'RUSTUP_TOOLCHAIN', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER']})
    p139.need(prior['environment'] == environment, 'P142 build environment changed')
    records = {}
    for side, root in roots.items():
        p139.need(p139.sha(root / 'adapter.json') == PINS[side], 'P142 successful build record changed')
        records[side] = (adapter.verify(root, freezing.ARCHIVE, ROOT / 'tools/wtime-bench')
                         if side == 'baseline' else adapter.verify_current(root, ROOT, ROOT / 'tools/wtime-bench'))
        command = p139.read_json(ATTEMPT / f'p142-build-{side}.result.json', 'original adapter build command')
        p139.need(command.get('exit_code') == 0, 'P142 original adapter build did not succeed')
    p139.need(pairs.common_identity(records['baseline']) == pairs.common_identity(records['candidate']),
               'P142 reused adapter configurations differ')
    result = {'schema': 'wtime-p142-build-reuse-v1', 'status': 'passed', 'source_stable': True,
              'guest_or_build_launched': False, 'timing_evidence': False,
              'builds': {side: p139.ref(root / 'adapter.json') for side, root in roots.items()},
              'prior_inputs': p139.ref(ATTEMPT / 'inputs.json'),
              'environment': environment, 'common': pairs.common_identity(records['baseline'])}
    p139.write_new(output / 'report.json', result)
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({'status': verify(args.baseline, args.candidate, args.output)['status'],
                      'guest_or_build_launched': False}))
