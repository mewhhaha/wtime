"""Freeze, verify inputs, and collect the Register milestone without analysis."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[3]
BENCH=ROOT/'tools/wtime-bench'
sys.path[:0]=[str(ROOT/'tools'),str(BENCH)]
from verification.artifacts import ArtifactStore, need, normal, read_json, verify_normal, write_new

ARCHIVE=ROOT/'target/performance/P067-before'
ENTRY_ARCHIVE=ROOT/'target/performance/P067-register-before'
ARCHIVE_SHA='490bf815837fa2225c3b52b3856194a30686c1800b38188cc78c4d84f80cf3ef'
PREDECESSOR='e2a525adc025e83eb952857703f8c58f342031f8'
BUILDS=ROOT/'target/performance/P067-builds'
SEEDS=(67001,67002)
SELECTIONS={name:ROOT/f'docs/benchmarks/performance/P067-{name}-selection.json' for name in ('eager','adaptive')}


def core_map(archive):
    return {row['source_path']:row['sha256'] for row in archive['files'].values()
        if row.get('role')=='source' and (row.get('source_path') in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')
        or (row.get('source_path','').startswith('crates/') and
            (row['source_path'].endswith('.rs') or row['source_path'].endswith('/Cargo.toml'))))}


def freeze(output):
    import baseline
    import p123_adapter
    import selection_manifest
    import entry_batch_adapter
    need(hashlib.sha256((ARCHIVE/'manifest.json').read_bytes()).hexdigest()==ARCHIVE_SHA,'P067 predecessor archive changed')
    archived=baseline.verify_archive(ARCHIVE)
    need(archived['repository_commit']==PREDECESSOR,'P067 predecessor commit differs')
    sources=core_map(archived)
    for name,digest in sources.items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,'P067 core differs from predecessor: '+name)
    predecessor={}
    for name,path in SELECTIONS.items():
        frozen=output.parent/(name+'-selection-freeze.json')
        selection_manifest.freeze(path,ARCHIVE/'manifest.json',frozen)
        predecessor[name]=normal(frozen)
    builds=read_json(BUILDS/'builds.json')
    need(builds.get('status')=='passed' and builds.get('timing_evidence') is False,'P067 comparator builds incomplete')
    for name in ('decoded','compact','register','p001'):
        archive=ROOT/'target/performance/baseline' if name=='p001' else ARCHIVE
        record=p123_adapter.verify(BUILDS/name,archive)
        need(record['mode']==('decoded' if name=='p001' else name),'P067 comparator mode differs')
    need(core_map(baseline.verify_archive(ENTRY_ARCHIVE))==sources,'Register entry archive core differs from portable predecessor')
    entry_batch_adapter.verify(BUILDS/'entry-baseline',ENTRY_ARCHIVE,BENCH)
    entry_batch_adapter.verify_current(BUILDS/'entry-current',ROOT,BENCH)
    # A fresh-lifetime counter proves exact invocation counts independently of timers.
    rows=[{'fixture':str(BENCH/'fixtures/p135/counter.wasm'),'case':'p135_counter',
        'input':7,'expected':8,'calls':calls,'baseline_strategy':'eager','baseline_threshold':0,
        'candidate_strategy':'adaptive','candidate_threshold':threshold}
        for threshold in (3,10) for calls in (1,10,100,10000)]
    sequences=[{'fixture':str(ROOT/'crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wasm'),
                'input':7,'schedule_threshold':threshold} for threshold in (3,10)]
    write_new(output.parent/'entry-rows.json',rows)
    write_new(output.parent/'sequence-rows.json',sequences)
    write_new(output,{'schema':'wtime-p067-freeze-v1','task':'P067','status':'passed','timing_evidence':False,
        'predecessor':normal(ARCHIVE/'manifest.json'),'entry_predecessor':normal(ENTRY_ARCHIVE/'manifest.json'),'p001':normal(ROOT/'target/performance/baseline/manifest.json'),
        'core_sources':sources,'selections':predecessor,'builds':normal(BUILDS/'builds.json'),
        'entry_rows':normal(output.parent/'entry-rows.json'),'sequence_rows':normal(output.parent/'sequence-rows.json')})


def verify_prior(output):
    """Reuse source-current P144 runtime controls, including every required name."""
    from verification.runner import test_results
    import p138
    completion=read_json(ROOT/'docs/benchmarks/performance/P144-completion.json')
    evidence_path=verify_normal(completion['evidence'][0]);evidence=read_json(evidence_path)
    need(evidence['passed'] is True and evidence['task']=='P144','P144 controls did not pass')
    store=ArtifactStore(ROOT/'target/verification')
    receipt=store.json(evidence['phases']['verify'])
    need(receipt['status']=='passed' and receipt['lane']=='full' and receipt['synthetic'] is False,'P144 controls are not real full verification')
    consumed=receipt['inputs']['components']['core']['files']
    for name,digest in consumed.items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,'P144 runtime control source is stale: '+name)
    names=set()
    commands={row['name']:row for row in receipt['commands']}
    for group in receipt['inputs']['definition']['tests']:
        if not group['command'].startswith(('runtime-','performance-')):continue
        command=commands[group['command']]
        logs=store.read(command['stdout']).decode()+'\n'+store.read(command['stderr']).decode()
        result=test_results(logs,group)
        need(result==receipt['tests'][group['command']],'P144 named control outcomes differ')
        names.update(name.rsplit('::',1)[-1] for name in result['names'])
    required=set(p138.CASE_NAMES+p138.P134_CASES+p138.P065_CASES)
    need(required<=names,'P144 omitted Register/fuel/root controls')
    capacity=read_json(ROOT/'docs/benchmarks/performance/P144-proof.json')
    need(capacity['status']=='passed' and capacity['timing_evidence'] is False,'P144 capacity diagnostics missing')
    for row in capacity['entry_sequences'].values():verify_normal(row['report'])
    write_new(output,{'schema':'wtime-p067-prior-controls-v1','status':'passed','source_current':True,
        'evidence':normal(evidence_path),'required_names':sorted(required),'capacity':normal(ROOT/'docs/benchmarks/performance/P144-proof.json'),
        'reused_rust_tests':sum(receipt['tests'][name]['passed'] for name in ('runtime-default','runtime-diagnostic','performance-diagnostic')),
        'timing_evidence':False})


def invoke(directory,name,argv):
    write_new(directory/(name+'.command.json'),{'argv':argv,'cwd':str(ROOT)})
    with (directory/(name+'.stdout')).open('x') as stdout,(directory/(name+'.stderr')).open('x') as stderr:
        result=subprocess.run(argv,cwd=ROOT,stdout=stdout,stderr=stderr)
    write_new(directory/(name+'.result.json'),{'exit_code':result.returncode})
    need(result.returncode==0,'P067 command failed: '+name)


def measure(frozen,verify_dir,output):
    import focused_compare
    freeze_record=read_json(frozen)
    need(freeze_record['status']=='passed','P067 freeze missing')
    for key in ('predecessor','entry_predecessor','p001','builds','entry_rows','sequence_rows'):verify_normal(freeze_record[key])
    before=focused_compare.sources()
    output.mkdir(parents=True,exist_ok=False)
    reports={}
    candidate=str(BUILDS/'register/binary/wtime-bench')
    for mode in ('decoded','compact'):
        name='eager-vs-'+mode
        destination=output/name
        invoke(output,name,[sys.executable,'-B',str(BENCH/'focused_compare.py'),
            '--manifest',str(SELECTIONS['eager']),'--frozen',freeze_record['selections']['eager']['path'],
            '--semantic-receipt',str(verify_dir/'receipt.json'),'--baseline',str(BUILDS/mode),
            '--candidate',candidate,'--candidate-mode','register','--run-seeds',*map(str,SEEDS),'--output',str(destination)])
        reports[name]=normal(destination/'report.json')
    name='adaptive-vs-decoded';destination=output/name
    invoke(output,name,[sys.executable,'-B',str(BENCH/'focused_compare.py'),
        '--manifest',str(SELECTIONS['adaptive']),'--frozen',freeze_record['selections']['adaptive']['path'],
        '--semantic-receipt',str(verify_dir/'receipt.json'),'--baseline',str(BUILDS/'decoded'),
        '--candidate',candidate,'--candidate-mode','register','--register-entry-threshold','3',
        '--run-seeds',*map(str,SEEDS),'--output',str(destination)])
    reports[name]=normal(destination/'report.json')
    # The exhaustive legacy comparator retains all SIMD rows. Analysis labels the
    # model-derived strict-float projection separately, without hiding extra rows.
    for family in ('scalar','memory','simd','representative','original-mvp'):
        name='p001-'+family;destination=output/name
        invoke(output,name,[sys.executable,'-B',str(BENCH/'compare.py'),'--baseline',str(BUILDS/'p001'),
            '--candidate',candidate,'--candidate-mode','register','--selection',family,
            '--pairs','30','--seed','67001','--output',str(destination)])
        reports[name]=normal(destination/'report.json')
    for name,tool,baseline_flag,rows in [('entry-totals','entry_batch_compare.py','--adapter','entry_rows'),
                                           ('entry-sequence','entry_sequence_compare.py','--baseline','sequence_rows')]:
        destination=output/name
        invoke(output,name,[sys.executable,'-B',str(BENCH/tool),'--archive',str(ENTRY_ARCHIVE),baseline_flag,str(BUILDS/'entry-baseline'),
            '--candidate',str(BUILDS/'entry-current'),'--repo',str(ROOT),'--harness',str(BENCH),
            '--rows',freeze_record[rows]['path'],'--pairs','30','--seeds',*map(str,SEEDS),'--output',str(destination)])
        reports[name]=normal(destination/'report.json')
    need(focused_compare.sources()==before,'P067 execution sources changed during measurement')
    store=ArtifactStore(ROOT/'target/verification')
    artifacts={str(path.relative_to(output)):store.add(path) for path in sorted(output.rglob('*')) if path.is_file()}
    write_new(output/'artifacts.json',artifacts)
    write_new(output/'report.json',{'schema':'wtime-p067-measurements-v1','task':'P067','status':'passed',
        'timing_evidence':True,'source_stable':True,'reports':reports,'artifacts':normal(output/'artifacts.json'),
        'source_inputs':before,'freeze':normal(frozen),'verify_receipt':normal(verify_dir/'receipt.json')})


def main():
    p=argparse.ArgumentParser();p.add_argument('phase',choices=['freeze','verify-prior','measure'])
    p.add_argument('--output',type=Path,required=True);p.add_argument('--freeze',type=Path);p.add_argument('--verify-dir',type=Path)
    a=p.parse_args()
    if a.phase=='freeze':freeze(a.output)
    elif a.phase=='verify-prior':verify_prior(a.output)
    else:measure(a.freeze,a.verify_dir,a.output)
if __name__=='__main__':main()
