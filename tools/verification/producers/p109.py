"""Current-source conformance and hardening evidence, independent of timers."""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(ROOT/'tools/checks')]
from verification.artifacts import need,normal,read_json,verify_normal,write_new

CORPUS=ROOT/'target/performance/P109-corpus-0002/summary.json'
SUITES={'mvp':'mvp','core3':'stable-core','phase3':'all-phase3-core','interactions':'all-phase3-core'}
CONFIGURATIONS={'decoded':('decoded',None),'compact':('compact',None),
                'register-eager':('register',None),'register-adaptive-3':('register',3)}


def sources():
    paths=[ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'rust-toolchain.toml',*ROOT.glob('crates/*/Cargo.toml'),*ROOT.glob('crates/**/*.rs')]
    return {str(path.relative_to(ROOT)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(paths)}


def validate_counts(report):
    for key in ('selected','executed','failed','skipped','unsupported','text_tool_errors'):
        need(type(report.get(key)) is int and report[key]>=0,'invalid conformance count: '+key)
    need(report['selected']>0 and report['selected']==report['executed'],'conformance is empty or incomplete')
    need(all(report[key]==0 for key in ('failed','skipped','unsupported','text_tool_errors')) and report.get('errors')==[],
         'conformance has failures, skips or unsupported directives')


def validate_configuration(report,suite,configuration):
    need(suite in SUITES and configuration in CONFIGURATIONS,'unknown selected configuration')
    mode,threshold=CONFIGURATIONS[configuration]
    need(report.get('suite')==suite and report.get('profile')==SUITES[suite]
         and report.get('execution_mode')==mode and report.get('register_entry_threshold')==threshold,
         'conformance mode/profile/threshold changed')
    strategy='disabled' if mode!='register' else 'adaptive' if threshold is not None else 'eager'
    need(report.get('register_strategy')==strategy,'conformance strategy changed')
    validate_counts(report)


def source_matches(mapping,root=ROOT):
    need(isinstance(mapping,dict) and bool(mapping),'source map missing')
    for name,digest in mapping.items():
        need(isinstance(name,str) and (root/name).is_file(),'source file missing: '+str(name))
        path=(root/name).resolve(strict=True)
        need(path.is_relative_to(root.resolve()) and hashlib.sha256(path.read_bytes()).hexdigest()==digest,
             'source identity stale: '+name)


def validate_corpus():
    summary=read_json(CORPUS)
    need(summary.get('schema')=='wtime-p109-pinned-corpus-matrix-v1' and summary.get('passed') is True
         and summary.get('source_stable') is True and summary.get('timing_evidence') is False,'normative matrix is incomplete')
    need(summary['source_hashes_before']==summary['source_hashes_after'],'normative source map changed')
    source_matches(summary['source_hashes_before'])
    binary=verify_normal(summary['binary'])
    rows=summary.get('runs',[])
    expected={(suite,mode) for suite in SUITES for mode in CONFIGURATIONS}
    need(len(rows)==len(expected) and {(row['suite'],row['configuration']) for row in rows}==expected,
         'normative matrix omits or duplicates configurations')
    counts={};opcodes=set()
    for row in rows:
        path=verify_normal(row['report']);report=read_json(path)
        validate_configuration(report,row['suite'],row['configuration'])
        need(report.get('binary_sha256')==summary['binary']['sha256'],'wrong normative executable')
        source_matches(report['implementation_sources'])
        need(isinstance(report.get('executed_opcodes'),list) and report['executed_opcodes'],'opcode observations missing')
        opcodes.update(report['executed_opcodes'])
        counts[row['suite']+'/'+row['configuration']]=report['executed']
    for suite in SUITES:
        need(len({counts[suite+'/'+mode] for mode in CONFIGURATIONS})==1,'selected configurations execute different corpus counts')
    return {'matrix':normal(CORPUS),'binary':normal(binary),'assertions':sum(counts.values()),'counts':counts,'opcodes':sorted(opcodes)}


def freeze(output):
    write_new(output,{'status':'passed','core_sources':sources(),'timing_evidence':False})


def verify(verify_dir,output):
    corpus=validate_corpus()
    import hardening
    allocation=read_json(verify_dir/'allocation.json')
    need(allocation.get('passed') is True and allocation.get('source_stable') is True and allocation.get('executed')==1
         and allocation.get('source_sha256')==hardening.source_hash(),'real allocation failure/recovery is unverified')
    need(allocation.get('address_space_bytes')==128*1024*1024 and allocation.get('requested_allocation_bytes')==512*1024*1024,
         'allocation limits differ')
    probe=ROOT/'target/performance/P109-corpus-build/release/wtime-probe'
    fuzz=read_json(verify_dir/'fuzz/report.json');diff=read_json(verify_dir/'differential/report.json')
    for name,report in [('fuzz',fuzz),('differential',diff)]:
        need(report.get('selected')==1000 and report.get('executed')==1000 and report.get('failed')==0,
             name+' checks are incomplete')
        need(report.get('target_sha256')==normal(probe)['sha256'],name+' target differs')
    need(fuzz.get('seed')==164 and fuzz.get('oracle_differences')==0,'fuzz seed or oracle differs')
    need(diff.get('seed')==165 and diff.get('vector_compared')==1000 and diff.get('oracle_not_run')==1000,
         'differential vector coverage or external-oracle label differs')
    write_new(output,{'schema':'wtime-p109-semantic-controls-v1','status':'passed','timing_evidence':False,
        'corpus':corpus,'allocation':normal(verify_dir/'allocation.json'),
        'fuzz':normal(verify_dir/'fuzz/report.json'),'differential':normal(verify_dir/'differential/report.json'),
        'probe':normal(probe),'core_sources':sources(),'external_differential_oracle':'unrun'})


def analyze(verify_dir,output):
    controls=read_json(verify_dir/'controls.json');need(controls['core_sources']==sources(),'hardening source changed')
    policy=read_json(ROOT/'docs/benchmarks/performance/decisions.json')
    need(policy.get('status')=='passed' and policy.get('sources')==sources(),'selected policy is incomplete or stale')
    need({row['name'] for row in policy['tiers']}==set(CONFIGURATIONS),'selected tier set changed')
    receipt=read_json(verify_dir/'receipt.json')
    need(receipt.get('status')=='passed' and receipt.get('lane')=='full','full hardening receipt is missing')
    write_new(output,{'schema':'wtime-p109-selected-conformance-v1','task':'P109','status':'passed','timing_evidence':False,
        'policy':normal(ROOT/'docs/benchmarks/performance/decisions.json'),'controls':normal(verify_dir/'controls.json'),
        'verification_receipt':normal(verify_dir/'receipt.json'),'tests':{name:row['passed'] for name,row in receipt['tests'].items()},
        'core_sources':sources(),'limitations':['Native AArch64 release execution is unrun.','External differential oracle is unrun; deterministic vectors are checked.']})


def main():
    p=argparse.ArgumentParser();p.add_argument('phase',choices=['freeze','verify','analyze'])
    p.add_argument('--verify-dir',type=Path);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    if a.phase=='freeze':freeze(a.output)
    elif a.phase=='verify':verify(a.verify_dir,a.output)
    else:analyze(a.verify_dir,a.output)
if __name__=='__main__':main()
