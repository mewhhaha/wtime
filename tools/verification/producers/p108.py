"""Choose a source-bound portable portfolio without promoting failed experiments."""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools'))
from verification.artifacts import need, normal, read_json, verify_normal, write_new

MILESTONES=('P035','P046','P056','P067','P080')
RETAINED={'P017':'decoded','P047':'compact','P050':'compact'}
TIERS=[
    {'name':'decoded','mode':'decoded','register_entry_threshold':None,'support':'default','features':[]},
    {'name':'compact','mode':'compact','register_entry_threshold':None,'support':'experimental','features':['compact-executor']},
    {'name':'register-eager','mode':'register','register_entry_threshold':None,'support':'experimental','features':['register-executor']},
    {'name':'register-adaptive-3','mode':'register','register_entry_threshold':3,'support':'experimental','features':['register-executor']},
]


def source_identity(root=ROOT):
    paths=[root/'Cargo.toml',root/'Cargo.lock',root/'rust-toolchain.toml',*root.glob('crates/*/Cargo.toml'),*root.glob('crates/**/*.rs')]
    return {str(path.relative_to(root)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(paths)}


def validate_policy(value,*,sources=None):
    need(isinstance(value,dict) and value.get('schema')=='wtime-performance-portfolio-v1','unknown portfolio schema')
    need(value.get('default')=='decoded','unaccepted default policy')
    need(value.get('tiers')==TIERS,'supported portfolio settings differ')
    need(value.get('native_enabled') is False and value.get('aot_enabled') is False,'unaccepted native or AOT branch')
    need(value.get('cancellation_policy')=='exact_source_events','cancellation policy was weakened')
    need(type(value.get('default_external_dependencies')) is int and value['default_external_dependencies']==0
         and value.get('default_unsafe') is False,'default closure violates policy')
    milestones=value.get('milestones',{})
    need(isinstance(milestones,dict) and set(milestones)==set(MILESTONES),'required milestone is missing')
    for task,record in milestones.items():
        need(record.get('status')=='done' and record.get('outcome') in {'complete','adopted','rejected','inconclusive','contract_only'},'milestone incomplete: '+task)
        need(isinstance(record.get('evidence'),list) and len(record['evidence'])>0,'milestone lacks acceptance artifacts')
        for reference in record['evidence']:verify_normal(reference)
    need(value.get('retained_optimizations')==RETAINED,'retained optimization set differs')
    need(all(tier['support']=='experimental' for tier in value['tiers'] if tier['mode']!='decoded'),
         'a non-adopted tier was silently promoted')
    retired=value.get('unselected',[])
    need(isinstance(retired,list) and all(isinstance(row,dict) for row in retired)
         and len({row.get('id') for row in retired})==len(retired),'duplicate unselected entries')
    for row in retired:
        need(row.get('outcome') in {'rejected','inconclusive','not_selected'} and isinstance(row.get('reason'),str) and row['reason'],
             'unselected experiment has no explicit reason')
        need(row.get('enabled') is False,'unselected branch is enabled')
    need(isinstance(value.get('sources'),dict) and bool(value['sources']),'source identity missing')
    if sources is not None:need(value['sources']==sources,'portfolio source identity is stale')
    return value['tiers']


def selected_tiers(path,*,check_sources=True):
    return validate_policy(read_json(path),sources=source_identity() if check_sources else None)


def build(plan_path,output):
    tasks={row['id']:row for row in read_json(plan_path)['tasks']}
    milestones={}
    for task in MILESTONES:
        row=tasks[task]
        need(row['status']=='done','portfolio prerequisite incomplete: '+task)
        milestones[task]={'status':row['status'],'outcome':row['outcome'],'evidence':row['evidence']}
    for task,mode in RETAINED.items():
        allowed={'adopted'} if mode=='decoded' else {'adopted','inconclusive'}
        need(tasks[task]['status']=='done' and tasks[task]['outcome'] in allowed,'retained optimization has no compatible acceptance: '+task)
    unselected=[]
    for task,row in sorted(tasks.items()):
        optional=row.get('branch') in ('optional','native') or row.get('kind')=='experiment'
        failed=row.get('status')=='done' and row.get('outcome') in ('rejected','inconclusive')
        if (not optional and not failed) or task in RETAINED or task in MILESTONES:continue
        if row['status']=='done' and row['outcome'] not in ('rejected','inconclusive'):continue
        outcome=row['outcome'] if row['status']=='done' else 'not_selected'
        unselected.append({'id':task,'outcome':outcome,'enabled':False,
            'reason':'Measured candidate did not pass promotion; its runtime changes were restored.' if outcome!='not_selected'
                else 'Optional research branch is not selected for the small portable portfolio; no implementation or speed claim.'})
    value={'schema':'wtime-performance-portfolio-v1','task':'P108','status':'passed','timing_evidence':False,
        'default':'decoded','tiers':TIERS,'native_enabled':False,'aot_enabled':False,
        'cancellation_policy':'exact_source_events','default_external_dependencies':0,'default_unsafe':False,
        'retained_optimizations':RETAINED,'milestones':milestones,'unselected':unselected,'sources':source_identity(),
        'limitations':['Compact and Register are explicit experimental opt-ins; their availability is not a speed guarantee.',
            'Adaptive threshold 3 is an explicitly tested configuration, not a universal recommendation.',
            'Native AArch64 release evidence remains unrun; browser and JavaScript-only behavior is excluded.']}
    validate_policy(value,sources=source_identity());write_new(output,value)


def validate_selected_tiers(selected,tiers,*,root=ROOT):
    need(selected.get('schema')=='wtime-selected-tier-check-v1' and selected.get('passed') is True,'selected tier expansion failed')
    need(selected.get('source_stable') is True and selected.get('timing_evidence') is False,
         'selected tier evidence is unstable or mislabeled')
    reports=selected.get('tiers',{})
    need(set(reports)=={row['name'] for row in tiers},'selected tier expansion omitted a policy')
    expected_configurations=[{key:row[key] for key in ('name','mode','register_entry_threshold','features','support')} for row in tiers]
    need(selected.get('configurations')==expected_configurations,'selected tier configurations differ from policy')
    executables=selected.get('executables',{})
    need(set(executables)==set(reports),'selected tier executables are incomplete')
    for tier in tiers:
        report=read_json(verify_normal(reports[tier['name']]))
        counts=report['counts']
        need(report.get('passed') is True and report.get('source_stable') is True
             and report.get('requested_tier')==tier['mode']
             and report.get('register_entry_threshold')==tier['register_entry_threshold'],'selected mode/configuration drifted')
        need(all(type(counts.get(key)) is int and counts[key]==0 for key in ('failed','skipped','unsupported'))
             and type(counts.get('all_checks_executed')) is int and counts['all_checks_executed']==89,
             'selected tier checks are incomplete')
        for name,digest in report['implementation_sources'].items():
            need(hashlib.sha256((root/name).read_bytes()).hexdigest()==digest,'selected tier source is stale')
        retained=executables[tier['name']]
        need(set(retained)=={'benchmark','conformance','diagnostic'},'selected tier retained executables are incomplete')
        expected={'benchmark':report['binary']['sha256'],
                  'conformance':report['conformance_binary']['sha256'],
                  'diagnostic':report['diagnostic_instrumentation']['binary_sha256']}
        for logical,reference in retained.items():
            verify_normal(reference)
            need(reference['sha256']==expected[logical],'retained executable differs from the one tested')
    return reports


def finalize(policy_path,verify_dir,output):
    policy=read_json(policy_path);tiers=validate_policy(policy,sources=source_identity())
    receipt=read_json(verify_dir/'receipt.json')
    need(receipt.get('task')=='P108' and receipt.get('status')=='passed' and receipt.get('lane')=='full'
         and receipt.get('synthetic') is False,'portfolio verification is incomplete')
    selected=read_json(verify_dir/'selected/report.json')
    need(selected.get('decisions')==normal(policy_path),'selected tier check used a different policy')
    reports=validate_selected_tiers(selected,tiers)
    write_new(output,{'schema':'wtime-p108-portfolio-v1','task':'P108','status':'passed','timing_evidence':False,
        'policy':normal(policy_path),'selected_tiers':reports,'verification_receipt':normal(verify_dir/'receipt.json'),
        'sources':source_identity(),'limitations':policy['limitations']})


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--plan',type=Path,default=Path('/workspace/tasks/performance/TASKS.json'))
    parser.add_argument('--policy',type=Path);parser.add_argument('--verify-dir',type=Path)
    parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    if args.policy:
        need(args.verify_dir is not None,'policy finalization requires verification evidence')
        finalize(args.policy,args.verify_dir,args.output)
    else:build(args.plan,args.output)
if __name__=='__main__':main()
