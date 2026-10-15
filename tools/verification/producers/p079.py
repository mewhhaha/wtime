"""Source-bound specialist allocation and retained-capacity gate; no timings."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'tools'), str(ROOT/'tools/wtime-bench'), str(ROOT/'tools/checks')]
from verification.artifacts import ArtifactStore, need, normal, read_json, verify_normal, write_new

ARCHIVE = ROOT/'target/performance/P079-before'
ARCHIVE_SHA = '7baf9264b0a1e6eb79d195b40f2c44d112824b24e29ed8eda09565ecaf875318'
PROBES = {
    'crates/wtime-runtime/src/gc.rs': 'p079_gc.rs',
    'crates/wtime-runtime/src/continuation.rs': 'p079_continuation.rs',
}
PREREQUISITES = ('P068','P069','P071','P072','P073','P074','P076','P077','P078')
CASES = {'gc_large_small','gc_failure','continuation_large_small'}


def integer(value, label):
    need(type(value) is int and value >= 0, label+' must be a nonnegative integer')
    return value


def validate_metrics(rows):
    need(isinstance(rows,list) and len(rows)==3, 'exactly three physical retention cases are required')
    by = {row.get('case'):row for row in rows if isinstance(row,dict)}
    need(set(by)==CASES and len(by)==len(rows),'missing, duplicate or unknown physical case')
    def snapshot(row):
        keys={'live','payload_charge_bytes','field_capacity_bytes','slot_capacity_bytes','free_capacity_bytes','slot_count'}
        need(isinstance(row,dict) and set(row)==keys,'invalid GC capacity snapshot')
        for key,value in row.items():integer(value,key)
        need(row['field_capacity_bytes']<=row['payload_charge_bytes'],'field storage exceeds payload charge')
    large=by['gc_large_small']
    for key in ('before','large','cleared','small','final'):snapshot(large.get(key))
    before,peak,cleared,small,final=(large[key] for key in ('before','large','cleared','small','final'))
    need(all(value==0 for value in before.values()),'fresh arena retained storage')
    need(peak['live']==peak['slot_count']==1024 and peak['field_capacity_bytes']>0,'large graph did not allocate')
    need(small['live']==4 and 0<small['payload_charge_bytes']<peak['payload_charge_bytes'],'small graph has wrong retained payload')
    for row in (cleared,final):
        need(row['live']==row['payload_charge_bytes']==row['field_capacity_bytes']==0,'collection retained object payload')
    need(peak['slot_capacity_bytes']==cleared['slot_capacity_bytes']==small['slot_capacity_bytes']==final['slot_capacity_bytes']>0,
         'arena slot capacity changed after peak or was not measured')
    need(cleared['free_capacity_bytes']==small['free_capacity_bytes']==final['free_capacity_bytes']>0,
         'arena free-list capacity grew after peak')
    need(all(row['slot_count']==1024 for row in (cleared,small,final)), 'small reuse grew arena slots')
    need(large.get('metadata_lifetime')=='arena' and large.get('root_hashmap_capacity')==0,'metadata lifetime/root ownership differs')
    failure=by['gc_failure']
    snapshot(failure.get('before'));snapshot(failure.get('final'))
    need(failure['before']['live']==1 and failure['before']['payload_charge_bytes']>0,'failure control lacks a live object')
    need(failure.get('object_failure_atomic') is True and failure.get('byte_failure_atomic') is True,'failure published partial state')
    need(failure['final']['live']==failure['final']['payload_charge_bytes']==failure['final']['field_capacity_bytes']==0,
         'failure recovery retained payload')
    continuation=by['continuation_large_small']
    sizes=continuation.get('capture_bytes')
    need(isinstance(sizes,list) and len(sizes)==4,'missing repeated continuation capacities')
    for value in sizes:integer(value,'continuation capacity')
    need(sizes[0]==sizes[2]>sizes[1]==sizes[3]>0,'continuation large/small physical state changed')
    need(continuation.get('physical_scan_matches') is True,'continuation charge differs from owned capacities')
    for key in ('final_continuation_bytes','final_gc_payload_bytes','final_states'):
        integer(continuation.get(key),key)
    need(all(continuation.get(key)==0 for key in ('final_continuation_bytes','final_gc_payload_bytes','final_states')),
         'consumed continuation retained payload')
    integer(continuation.get('registry_capacity'),'continuation registry capacity')
    need(continuation.get('registry_lifetime')=='store','continuation registry lifetime differs')
    return by


def metrics(path):
    rows=[]
    for line in path.read_text().splitlines():
        if line.startswith('P079_METRIC '):rows.append(json.loads(line[len('P079_METRIC '):]))
    validate_metrics(rows)
    return rows


def freeze(output):
    import baseline
    need(hashlib.sha256((ARCHIVE/'manifest.json').read_bytes()).hexdigest()==ARCHIVE_SHA,'P079 archive changed')
    archive=baseline.verify_archive(ARCHIVE)
    need(archive['repository_commit'].startswith('3cc88d8'),'P079 predecessor differs')
    checked={}
    for relative,record in archive['files'].items():
        if record.get('role')!='source':continue
        name=record.get('source_path','')
        if not name.startswith('crates/') and name not in ('Cargo.toml','Cargo.lock','rust-toolchain.toml'):continue
        original=(ARCHIVE/relative).read_text() if name in PROBES else (ARCHIVE/relative).read_bytes()
        current=(ROOT/name).read_text() if name in PROBES else (ROOT/name).read_bytes()
        if name in PROBES:
            block='#[cfg(test)]\n#[path = "../tests/performance/'+PROBES[name]+'"]\nmod p079;\n'
            need(current.count(block)==1,'test-only probe inclusion changed')
            current=current.replace(block,'',1)
        need(current==original,'production source differs: '+name)
        checked[name]=record['sha256']
    canonical=ROOT.parent/'tasks/performance/TASKS.json'
    mirrored=ROOT/'task-plans/performance/TASKS.json'
    need(canonical.read_bytes()==mirrored.read_bytes(),'canonical task manifest differs from bound mirror')
    task_rows=read_json(mirrored)['tasks']
    decisions={row['id']:{'status':row['status'],'outcome':row['outcome']} for row in task_rows if row['id'] in PREREQUISITES}
    need(set(decisions)==set(PREREQUISITES) and all(row['status']=='done' and row['outcome'] in ('rejected','inconclusive') for row in decisions.values()),
         'default specialist selection changed; review gate scope')
    write_new(output,{'schema':'wtime-p079-freeze-v1','status':'passed','predecessor':normal(ARCHIVE/'manifest.json'),
        'production_sources_unchanged':True,'source_inventory':checked,'specialist_decisions':decisions,
        'runtime_sources':{name:digest for name,digest in checked.items()
            if name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')
            or (name.startswith(('crates/wtime-runtime/','crates/wtime-types/','crates/wtime-decode/','crates/wtime-validate/'))
                and (name.endswith('.rs') or name.endswith('/Cargo.toml')))},
        'default_execution':'decoded','selected_specialist_candidates':[],'timing_evidence':False})


def preserve_allocation(output):
    path=ROOT/'target/conformance/allocation-failure.json'
    output.mkdir(parents=True,exist_ok=False)
    if path.exists():shutil.copyfile(path,output/'previous-report.json')
    write_new(output/'capture.json',{'prior_report':normal(path) if path.exists() else None})


def summarize(verify_dir,freeze_path,output):
    import build_budget
    import hardening
    import p138
    frozen=read_json(freeze_path)
    need(frozen.get('schema')=='wtime-p079-freeze-v1' and frozen.get('status')=='passed'
         and frozen.get('production_sources_unchanged') is True
         and frozen.get('default_execution')=='decoded'
         and frozen.get('selected_specialist_candidates')==[], 'P079 freeze is incomplete')
    verify_normal(frozen['predecessor'])
    receipt=read_json(verify_dir/'receipt.json')
    need(receipt['task']=='P079' and receipt['phase']=='verify' and receipt['status']=='passed'
         and receipt['lane']=='full' and receipt['synthetic'] is False,'P079 verification is incomplete')
    store=ArtifactStore(ROOT/'target/verification')
    for reference in receipt['artifacts'].values():store.verify(reference)
    for reference in receipt['original_outputs'].values():verify_normal(reference)
    groups={name:metrics(verify_dir/(name+'.stderr')) for name in ('runtime-default','runtime-combined')}
    allocation=read_json(verify_dir/'allocation.json')
    need(allocation.get('passed') is True and allocation.get('source_stable') is True
         and allocation.get('executed')==1 and allocation.get('exit_code')==0
         and allocation.get('source_sha256')==hardening.source_hash(),'allocator failure/recovery is stale or failed')
    worker=verify_dir/'allocation-worker'
    need(allocation.get('worker_sha256')==hashlib.sha256(worker.read_bytes()).hexdigest(),'allocation worker changed')
    need(allocation.get('address_space_bytes')==128*1024*1024
         and allocation.get('requested_allocation_bytes')==512*1024*1024,'allocator failure limits differ')
    build_refs={}
    for filename,expected in [('default-build.json',[]),('register-build.json',['wtime-runtime/register-executor']),
                              ('experimental-build.json',['wtime-runtime/experimental-executors'])]:
        path=verify_dir/filename
        report=read_json(path)
        need(report.get('features')==expected and report.get('budget_pass') is True
             and report.get('source_stable') is True and build_budget.check_budget(report), 'build budget failed: '+filename)
        need(report['source_inputs']==report['source_inputs_after']==report['isolated_source_inputs_after'], 'build source changed')
        build_refs[filename]={'report':normal(path),'clean_seconds':report['clean_seconds'],
            'leaf_seconds':report['leaf_seconds'],'runtime_leaf_seconds':report['runtime_leaf_seconds'],
            'stripped_bytes':report['stripped_bytes'],'peak_rss_kib':report['clean_peak_rss_kib']}
    logs=[verify_dir/'runtime-combined.stderr',verify_dir/'performance-combined.stdout']
    traces=p138.validate_traces(logs)
    cases=p138.validate_case_records(logs)
    rows=p138._json_markers(logs,'P138_TRACE')
    capacity={'max_retained_bytes':max(row['retained_bytes'] for row in rows),
              'max_ready_entries':max(len(row['rows']) for row in rows),'trace_check':traces,'case_count':len(cases),
              'accounting_note':'Prepared code retention charge; not activation bytes or process RSS.'}
    allocation_copy=output.parent/'allocation.json'
    write_new(allocation_copy,allocation)
    write_new(output,{'schema':'wtime-p079-specialist-budgets-v1','task':'P079','status':'passed','timing_evidence':False,
        'default_execution':'decoded','selected_specialist_candidates':[],
        'freeze':normal(freeze_path),'verification_receipt':normal(verify_dir/'receipt.json'),
        'tests':{key:value['passed'] for key,value in receipt['tests'].items()},
        'physical_retention':groups,'optional_register_cache':capacity,'builds':build_refs,
        'allocation_failure':normal(allocation_copy),'allocator_scope':'real host allocation failure in memory constructor; GC tests exercise explicit budgets',
        'physical_scope':'Exact Vec capacities for arena slots, free list and object fields; excludes allocator padding, root/continuation HashMap bucket layout and process RSS.',
        'metadata_lifetime':'Reusable arena metadata lives until arena drop; continuation registry metadata lives until Store drop.'})


def main():
    p=argparse.ArgumentParser();p.add_argument('phase',choices=['freeze','preserve-allocation','capture-allocation','summarize'])
    p.add_argument('--output',required=True,type=Path);p.add_argument('--verify-dir',type=Path);p.add_argument('--freeze',type=Path)
    a=p.parse_args()
    if a.phase=='freeze':freeze(a.output)
    elif a.phase=='preserve-allocation':preserve_allocation(a.output)
    elif a.phase=='capture-allocation':
        report=read_json(ROOT/'target/conformance/allocation-failure.json')
        worker=a.output.parent/'allocation-worker'
        need(not worker.exists(),'allocation worker output already exists')
        shutil.copyfile(ROOT/'target/debug/wtime-allocation-check',worker)
        need(hashlib.sha256(worker.read_bytes()).hexdigest()==report.get('worker_sha256'),'allocation worker changed before retention')
        write_new(a.output,report)
    else:summarize(a.verify_dir,a.freeze,a.output)


if __name__=='__main__':main()
