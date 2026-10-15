"""Source-bound adaptive conformance plumbing gate, without performance claims."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(ROOT/'tools/wtime-bench')]
from verification.artifacts import ArtifactStore,VerificationError,need,normal,read_json,verify_normal,write_new

ARCHIVE=ROOT/'target/performance/P143-before'
ARCHIVE_SHA='e8347277e31254a42ca49b06019abd9b6dfebf8d582038294da4b7e5eda1c7ad'
OWNED=('tools/wtime-conformance/src/','tools/wtime-bench/src/tier_state_driver.rs',
       'tools/wtime-bench/src/entry_sequence_driver.rs',
       'tools/wtime-bench/src/entry_batch_driver.rs',
       'tools/wtime-bench/src/protocol.rs',
       'tools/wtime-bench/tier_check.py','tools/wtime-bench/tests/test_p143_tier_threshold.py')


def freeze(output):
    import baseline
    need(hashlib.sha256((ARCHIVE/'manifest.json').read_bytes()).hexdigest()==ARCHIVE_SHA,'P143 predecessor changed')
    archive=baseline.verify_archive(ARCHIVE)
    need(archive['repository_commit']=='c3e40d4ba568ac26240a0b23bdf76bad9be39f47','P143 predecessor commit differs')
    core={};owned={}
    for relative,record in archive['files'].items():
        if record.get('role')!='source':continue
        name=record.get('source_path','')
        if name.startswith('crates/') or name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml'):
            need((ROOT/name).read_bytes()==(ARCHIVE/relative).read_bytes(),'core changed: '+name)
            core[name]=record['sha256']
        if name.startswith(OWNED):owned[name]=record['sha256']
    write_new(output,{'schema':'wtime-p143-freeze-v1','task':'P143','status':'passed',
        'predecessor':normal(ARCHIVE/'manifest.json'),'core_unchanged':True,
        'core_sources':core,'owned_sources':owned,'timing_evidence':False})


def validate_tier(report,threshold,candidate='register'):
    need(report.get('passed') is True and report.get('source_stable') is True,'tier check failed or source changed')
    need(candidate in ('decoded','register') and report.get('requested_tier')==candidate
         and report.get('baseline_tier')=='reference','wrong compared modes')
    need(report.get('register_entry_threshold')==threshold
         and report.get('register_strategy')==('disabled' if candidate=='decoded' else 'eager' if threshold is None else 'adaptive'),'threshold strategy differs')
    counts=report.get('counts',{})
    for key in ('failed','skipped','unsupported'):
        need(type(counts.get(key)) is int and counts[key]==0,'non-passing '+key)
    for left,right in [('mode_runs_expected','mode_runs_executed'),('state_mode_runs_expected','state_mode_runs_executed'),
                       ('state_fuel_probes_expected','state_fuel_probes_executed'),
                       ('conformance_mode_runs_expected','conformance_mode_runs_executed'),('all_checks_expected','all_checks_executed')]:
        need(type(counts.get(left)) is int and counts[left]>0 and counts[left]==counts.get(right),'incomplete '+left)
    for key,expected in [('mode_runs_executed',6),('state_mode_runs_executed',6),
                         ('state_fuel_probes_executed',68),('conformance_mode_runs_executed',4),('all_checks_executed',89)]:
        need(counts.get(key)==expected,'required corpus or prefix checks missing: '+key)
    need(len(report.get('conformance',[]))==4,'conformance reports missing')
    need({(row.get('suite'),row.get('mode')) for row in report['conformance']}==
         {(suite,mode) for suite in ('reference_types','interactions') for mode in ('reference',candidate)},
         'conformance suite/mode portfolio differs')
    for item in report['conformance']:
        data=item['report'];mode=item['mode']
        expected=threshold if mode=='register' else None
        need(data.get('register_entry_threshold')==expected,'conformance threshold not forwarded')
        need(data.get('register_strategy')==('adaptive' if expected else 'eager' if mode=='register' else 'disabled'),'conformance strategy mismatch')
        need(data['selected']>0 and data['executed']>0 and data['failed']==data['skipped']==data['unsupported']==0,'conformance did not execute cleanly')
        need(data.get('executed_opcodes') and item.get('corpus_sources'),'conformance source/opcode evidence missing')
    return counts


def summarize(verify_dir,freeze_path,output):
    import tier_check
    freeze_record=read_json(freeze_path)
    need(freeze_record['core_unchanged'] is True,'P143 changes core behavior')
    verify_normal(freeze_record['predecessor'])
    for name,digest in freeze_record['core_sources'].items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,'core changed after freeze: '+name)
    receipt=read_json(verify_dir/'receipt.json')
    need(receipt['task']=='P143' and receipt['status']=='passed' and receipt['lane']=='full'
         and receipt['synthetic'] is False,'P143 verify receipt incomplete')
    store=ArtifactStore(ROOT/'target/verification')
    for reference in receipt['artifacts'].values():store.verify(reference)
    for reference in receipt['original_outputs'].values():verify_normal(reference)
    reports={name:read_json(verify_dir/name/'report.json') for name in ('decoded','explicit','adaptive')}
    counts={name:validate_tier(report,3 if name=='adaptive' else None,'decoded' if name=='decoded' else 'register') for name,report in reports.items()}
    need(counts['decoded']==counts['explicit']==counts['adaptive'],'adaptive corpus count differs')
    need(all(report['implementation_sources']==tier_check._source_identity() for report in reports.values()),
         'tier source identities differ or are stale')
    need(all(report['conformance_sources_before']==report['conformance_sources_after']
             ==reports['explicit']['conformance_sources_before'] for report in reports.values()),'corpus changed between tiers')
    corpus={}
    for name,report in reports.items():
        corpus[name]=[{'suite':item['suite'],'mode':item['mode'],'selected':item['report']['selected'],
                      'executed':item['report']['executed'],'failed':item['report']['failed'],
                      'skipped':item['report']['skipped'],'unsupported':item['report']['unsupported'],
                      'source_hash':item['report']['source_hash'],'register_entry_threshold':item['report']['register_entry_threshold']}
                     for item in report['conformance']]
    write_new(output,{'schema':'wtime-p143-threshold-gate-v1','task':'P143','status':'passed',
        'freeze':normal(freeze_path),'verification_receipt':normal(verify_dir/'receipt.json'),
        'tier_reports':{name:normal(verify_dir/name/'report.json') for name in reports},
        'counts':counts,'conformance':corpus,'tests':{name:value['passed'] for name,value in receipt['tests'].items()},
        'core_unchanged':True,'default_execution':'decoded','timing_evidence':False,
        'limitations':['Threshold forwarding, optimized/fallback dispatch, and exact state/corpus verification only; typed-protocol timings are diagnostic and support no performance claim.']})


def main():
    p=argparse.ArgumentParser();p.add_argument('phase',choices=['freeze','summarize']);p.add_argument('--output',required=True,type=Path)
    p.add_argument('--verify-dir',type=Path);p.add_argument('--freeze',type=Path);a=p.parse_args()
    if a.phase=='freeze':freeze(a.output)
    else:summarize(a.verify_dir,a.freeze,a.output)


if __name__=='__main__':main()
