"""Collect final default and Compact costs with the existing paired MVP protocol.

Register cold rows are reused from P067's direct paired comparisons. This
collector never derives a paired ratio from two independent comparisons.
"""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[3]
PRIVATE=ROOT/'target/performance/P080-isolated-source'
BUILDS=ROOT/'target/performance/P067-builds'
sys.path.insert(0,str(ROOT/'tools'))
from verification.artifacts import ArtifactStore,need,normal,write_new


def measure(output):
    # The isolated preparation encoder cannot be replaced by unrelated Cargo
    # tests. Every logical measured source must still match the checkout.
    sys.path.insert(0,str(PRIVATE/'tools/wtime-bench'))
    import compare
    import execution_identity
    before=execution_identity.project(compare.sources())
    for name,digest in before.items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,
             'isolated cold measurement source differs: '+name)
    output.mkdir(parents=True,exist_ok=False)
    configurations=(('default-vs-p001','p001','decoded','decoded',110001),
                    ('compact-vs-decoded','decoded','compact','compact',110011))
    reports={}
    for name,baseline,candidate,mode,seed in configurations:
        argv=['taskset','-c','2',sys.executable,'-B',str(PRIVATE/'tools/wtime-bench/compare.py'),
              '--baseline',str(BUILDS/baseline),
              '--candidate',str(BUILDS/candidate/'binary/wtime-bench'),
              '--candidate-mode',mode,'--selection','original-mvp',
              '--pairs','30','--seed',str(seed),'--cpu','2','--output',str(output/name)]
        write_new(output/(name+'.command.json'),{'argv':argv,'cwd':str(PRIVATE)})
        with (output/(name+'.stdout')).open('x') as stdout,(output/(name+'.stderr')).open('x') as stderr:
            result=subprocess.run(argv,cwd=PRIVATE,stdout=stdout,stderr=stderr)
        write_new(output/(name+'.result.json'),{'exit_code':result.returncode})
        need(result.returncode==0,'final cold comparison failed: '+name)
        reports[name]=normal(output/name/'report.json')
    need(before==execution_identity.project(compare.sources()),'isolated cold measurement sources changed')
    for name,digest in before.items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,
             'live cold measurement source changed: '+name)
    store=ArtifactStore(ROOT/'target/verification')
    artifacts={str(path.relative_to(output)):store.add(path) for path in sorted(output.rglob('*')) if path.is_file()}
    write_new(output/'artifacts.json',artifacts)
    write_new(output/'report.json',{'schema':'wtime-p110-cold-collection-v1','task':'P110','status':'passed',
        'source_stable':True,'timing_evidence':True,'reports':reports,'source_inputs':before,
        'private_root':str(PRIVATE),'artifacts':normal(output/'artifacts.json'),
        'scope':'All original MVP stages: tiny call, arithmetic loop, mixed branch/call module, large compile-only module.',
        'register_rows':'Reuse P067 direct eager/adaptive versus decoded comparisons; never combine independent ratios.',
        'environment':'CPU 2 in shared cloud; CPU 0 P067 and CPU 1 P080 collections may overlap; physical-core idleness unknown.'})


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();measure(args.output)
