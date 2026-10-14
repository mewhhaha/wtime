"""Revalidate the completed P046 semantic/build work without running it again.

This explicit migration consumes the immutable attempt-0003 receipt. It accepts
analysis-only changes, never changed runtime, timer, fixture or build inputs.
The updated analysis tests run in the contract's analysis phase.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools'))
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
from verification.artifacts import ArtifactStore, digest, need, normal, read_json, sha, verify_normal, write_new
from verification.contracts import identity
from verification.runner import execution_environment, test_results
import execution_identity
import compare
import p123_adapter

ORIGINAL = ROOT/'target/verification/runs/P046/verify/attempt-0003'
RECEIPT_SHA = '3a4e0b46a8f8570e010cef0709c3fcc361a97cb413f42134146c66b16bd5bc44'


def revalidate(output, original=ORIGINAL):
    original, output = Path(original).resolve(strict=True), Path(output).resolve()
    need(sha(original/'receipt.json') == RECEIPT_SHA, 'unexpected P046 verification receipt')
    receipt = read_json(original/'receipt.json')
    inputs = read_json(original/'inputs.json')
    need(receipt['status'] == 'passed' and receipt['task'] == 'P046' and receipt['phase'] == 'verify',
         'original verification did not pass')
    need(digest(inputs) == receipt['input_key'], 'verification inputs do not match their receipt')
    store = ArtifactStore(ROOT/'target/verification')
    for reference in receipt['artifacts'].values():
        store.verify(reference)
    for reference in receipt['original_outputs'].values():
        verify_normal(reference)
    for name in ('compilation','corpus'):
        old = inputs['components'][name]
        need(identity(ROOT,old['declaration']) == old, 'semantic/build inputs changed: '+name)
    need(execution_environment(ROOT, inputs['definition']) == inputs['environment'], 'verification environment changed')
    for command in receipt['commands']:
        need(command['exit_code'] == 0, 'original verification command failed')
        store.verify(command['stdout']); store.verify(command['stderr'])
    for group in inputs['definition'].get('tests',[]):
        command = next(row for row in receipt['commands'] if row['name'] == group['command'])
        text = store.read(command['stdout']).decode()+ '\n'+store.read(command['stderr']).decode()
        need(test_results(text,group) == receipt['tests'][group['command']], 'original named tests differ')
    # Verify every nested byte, not merely the outer report which refers to it.
    inventory = read_json(original/'inventory.json')
    for name, reference in inventory['files'].items():
        path = (original/name).resolve(strict=True)
        need(path.is_relative_to(original), 'old verification inventory escapes its root')
        need(sha(path) == reference['sha256'] and path.stat().st_size == reference['bytes'], 'old verification artifact changed: '+name)
    production = read_json(original/'production/build-record.json')
    binding = execution_identity.verify(production['source_identities'], compare.sources(
        include_memory=True,include_simd=True,include_representative=True))
    # Non-benchmark producers that supplied the accepted semantic evidence remain exact.
    old_harness = inputs['components']['harness']['files']
    for name,value in old_harness.items():
        if not name.startswith('tools/wtime-bench/'):
            need(sha(ROOT/name) == value, 'semantic/build producer changed: '+name)
    baselines = read_json(original/'baselines/report.json')
    for key in ('p001_adapter','structural_adapter'):
        reference = baselines[key]
        need(sha(reference['path']) == reference['sha256'], 'adapter manifest changed')
        p123_adapter.verify(Path(reference['path']).parent)
    output.mkdir(parents=True,exist_ok=True)
    # Small immutable payloads are copied; all their original absolute references
    # remain explicitly bound to the successful attempt above.
    for name in ('production','baselines','retained','tier'):
        shutil.copytree(original/name,output/name)
    for name in ('build.json','optional-build.json','inventory.json','artifacts.tar.gz'):
        shutil.copyfile(original/name,output/name)
    result = {'schema':'p046-verification-reuse-v1','status':'passed','source_stable':True,
        'original_receipt':normal(original/'receipt.json'),'execution_binding':binding,
        'commands_rerun':0,'wasm_timings_collected':0,
        'scope':'unchanged semantic/build inputs; current analysis checks run separately'}
    write_new(output/'reuse.json',result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    print(json.dumps({'status':revalidate(args.output)['status']}))
