#!/usr/bin/env python3
"""Attach auditable implementation handlers and successful traced corpus evidence.

Only passing reports with actual executed_opcodes and an immutable binary digest
can discharge an instruction's test owner. No static occurrence is execution.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT=Path(__file__).resolve().parents[2]

def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def runtime_opcode(encoding):
    parts=encoding.split(':');prefix=int(parts[0],16)
    if len(parts)==1:return prefix
    sub=int(parts[1]);return 0xfa00+(sub-256) if prefix==0xfd and sub>=256 else prefix*256+sub

def handler(root,path,symbol,encoding):
    text=(root/path).read_text();matches=list(re.finditer(r'\bfn\s+'+re.escape(symbol)+r'\s*\(',text))
    if len(matches)>1:
        public=list(re.finditer(r'\bpub(?:\([^)]*\))?\s+fn\s+'+re.escape(symbol)+r'\s*\(',text))
        if len(public)==1:matches=public
    if len(matches)!=1:raise ValueError(f'{path}:{symbol}: expected one actual function, found {len(matches)}')
    return {'kind':'source-handler','path':path,'symbol':symbol,'line':text.count('\n',0,matches[0].start())+1,'source_sha256':digest(root/path),'dispatch_opcode':encoding}

def mappings(encoding):
    n=runtime_opcode(encoding)
    validate=('crates/wtime-validate/src/lib.rs','instruction')
    execute=('crates/wtime-runtime/src/execution.rs','invoke')
    if n>>8 in (0xfa,0xfd):
        validate=('crates/wtime-validate/src/lib.rs','simd')
        execute=('crates/wtime-runtime/src/execution.rs','simd_memory') if (0xfd00<=n<=0xfd0b or 0xfd54<=n<=0xfd5d) else ('crates/wtime-runtime/src/simd.rs','execute')
    elif n>>8==0xfe:validate=('crates/wtime-validate/src/atomics.rs','atomic');execute=('crates/wtime-runtime/src/execution.rs','atomic')
    elif n>>8==0xfb:
        validate=('crates/wtime-validate/src/gc.rs','gc')
        execute=('crates/wtime-runtime/src/execution.rs','invoke') if n in (0xfb18,0xfb19,0xfb25,0xfb26) else ('crates/wtime-runtime/src/gc_exec.rs','gc_exec')
    elif 0xe0<=n<=0xe6:validate=('crates/wtime-validate/src/continuations.rs','continuation');execute=('crates/wtime-runtime/src/continuation.rs','cont_step')
    elif n in (8,10,31):validate=('crates/wtime-validate/src/exceptions.rs','exception_instruction')
    elif 0xfc13<=n<=0xfc16:execute=('crates/wtime-runtime/src/wide.rs','execute')
    elif 0xfc08<=n<=0xfc11:execute=('crates/wtime-runtime/src/execution.rs','bulk')
    elif 0x28<=n<=0x3e:execute=('crates/wtime-runtime/src/execution.rs','load_store')
    elif 0x45<=n<=0xc4 or 0xfc00<=n<=0xfc07:execute=('crates/wtime-runtime/src/numeric.rs','exec')
    return {'decode':('crates/wtime-decode/src/instructions.rs','read_instruction'),'validate':validate,'execute':execute}

def non_opcode_mappings(feature):
    d=('crates/wtime-decode/src/module.rs','decode');v=('crates/wtime-validate/src/lib.rs','validate');e=('crates/wtime-runtime/src/linker.rs','instantiate_inner')
    if feature in ('non-trapping_float-to-int_conversions','sign-extension_operators','tail_call','wide-arithmetic'):
        d=('crates/wtime-decode/src/instructions.rs','read_instruction');v=('crates/wtime-validate/src/lib.rs','instruction')
        e=('crates/wtime-runtime/src/wide.rs','execute') if feature=='wide-arithmetic' else ('crates/wtime-runtime/src/numeric.rs','exec') if feature in ('non-trapping_float-to-int_conversions','sign-extension_operators') else ('crates/wtime-runtime/src/execution.rs','invoke')
    elif feature in ('fixed-width_simd','relaxed-simd'):
        d=('crates/wtime-decode/src/instructions.rs','read_instruction');v=('crates/wtime-validate/src/lib.rs','simd');e=('crates/wtime-runtime/src/simd.rs','execute')
    elif feature=='extended-const':v=('crates/wtime-validate/src/lib.rs','constant');e=('crates/wtime-runtime/src/lib.rs','eval_const')
    elif feature in ('garbage_collection','custom-descs'):
        d=('crates/wtime-decode/src/module.rs','subtype_bounded');v=('crates/wtime-validate/src/types.rs','validate_types');e=('crates/wtime-runtime/src/canonical.rs','intern_group')
    elif feature=='stack-switching':v=('crates/wtime-validate/src/continuations.rs','continuation');e=('crates/wtime-runtime/src/continuation.rs','cont_step')
    elif feature=='exception_handling':v=('crates/wtime-validate/src/exceptions.rs','add_tag');e=('crates/wtime-runtime/src/execution.rs','unwind_exception')
    elif feature=='threads':d=('crates/wtime-decode/src/module.rs','limits');v=('crates/wtime-validate/src/atomics.rs','atomic');e=('crates/wtime-runtime/src/execution.rs','atomic')
    elif feature=='annotations':d=('tools/wtime-conformance/src/cli.rs','encode');v=d;e=None
    elif feature=='branch-hinting':d=('crates/wtime-decode/src/metadata.rs','parse');v=('crates/wtime-decode/src/metadata.rs','inspect_branch_hints');e=None
    elif feature=='custom-page-sizes':d=('crates/wtime-decode/src/module.rs','limits');e=('crates/wtime-runtime/src/lib.rs','create_memory')
    return {'decode':d,'validate':v,'execute':e}

def successful_report(path):
    report=json.loads(path.read_text())
    if not(report.get('selected',0)>0 and report.get('executed')==report.get('selected') and all(report.get(k,0)==0 for k in ('failed','skipped','unsupported','text_tool_errors'))):return None
    if not isinstance(report.get('executed_opcodes'),list) or not re.fullmatch('[0-9a-f]{64}',report.get('binary_sha256','')) or not report.get('implementation_sources'):return None
    return report

def reconcile(root,reports):
    path=root/'docs/architecture/opcodes.json';ledger=json.loads(path.read_text());evidence={}
    for p in reports:
        report=successful_report(p)
        if report is None:continue
        report_digest=digest(p)
        snapshot=root/'docs/architecture/opcode-evidence'/f'{report_digest}.json'
        snapshot.parent.mkdir(parents=True,exist_ok=True)
        if not snapshot.exists():snapshot.write_bytes(p.read_bytes())
        name=str(snapshot.relative_to(root))
        evidence[name]={'kind':'executed-corpus','path':name,'origin_report':str(p),'report_sha256':report_digest,'binary_sha256':report['binary_sha256'],'profile':report['profile'],'suite':report['suite'],'source_hash':report['source_hash'],'selected':report['selected'],'executed':report['executed'],'executed_opcodes':report['executed_opcodes'],'implementation_sources':report['implementation_sources'],'compatibility':report.get('compatibility',[]),'compatibility_assertions':report.get('compatibility_assertions',0)}
    discharged=0
    for row in ledger['instructions']:
        for stage,(file,symbol) in mappings(row['encoding']).items():
            item=handler(root,file,symbol,row['encoding']);owner=row['owners'][stage]
            owner.update({'path':file,'symbol':symbol,'status':'implemented','evidence':item})
        owner=row['owners']['test'];opcode=runtime_opcode(row['encoding'])
        found=[e for e in evidence.values() if opcode in e['executed_opcodes'] and all(e['implementation_sources'].get(row['owners'][stage]['path'])==row['owners'][stage]['evidence']['source_sha256'] for stage in ('decode','validate','execute'))]
        if found:
            # Prefer the smallest passing suite that actually executed the opcode.
            best=min(found,key=lambda e:e['selected']);owner.update({'status':'implemented','path':best['path'],'symbol':'executed_opcodes','evidence':{'kind':'executed-corpus','report':best['path'],'opcode':opcode}})
            discharged+=1
        else:
            owner.update({'status':'pending-audit','path':'tools/wtime-conformance/src','symbol':''});owner.pop('evidence',None)
    for row in ledger['non_opcode_features']:
        for stage,pair in non_opcode_mappings(row['feature']).items():
            owner=row['owners'][stage]
            if pair is None:
                owner.update({'status':'not-applicable','reason':'Text annotations and branch hints have no observable runtime execution semantics; see locked scope.'})
                continue
            file,symbol=pair;item=handler(root,file,symbol,row['feature'])
            owner.update({'status':'implemented','path':file,'symbol':symbol,'evidence':item})
        matching=[e for e in evidence.values() if (e['suite']==row['feature'] or (e['suite']=='phase3' and row['feature'] not in ('annotations','branch-hinting')) or (e['suite']=='core3' and row['feature'] not in ('threads','stack-switching','custom-descs','custom-page-sizes','compact-import-section','wide-arithmetic'))) and all(e['implementation_sources'].get(owner['path'])==owner['evidence']['source_sha256'] for owner in row['owners'].values() if owner.get('status')=='implemented' and owner.get('evidence',{}).get('kind')=='source-handler')]
        owner=row['owners']['test']
        if row['feature']=='annotations':
            owner['candidate_tests']=[{'path':'tools/wtime-conformance/tests/annotations.rs','symbols':['t159_case_01','t159_case_02','t159_case_03']}]
        elif row['feature']=='branch-hinting':
            owner['candidate_tests']=[{'path':'crates/wtime-decode/src/metadata.rs','symbols':['t158_case_01','t158_case_02','t158_case_03']},{'path':'crates/wtime-cli/tests/metadata.rs','symbols':['t158_hints_do_not_change_execution']}]
        if matching:
            best=min(matching,key=lambda e:e['selected']);owner.update({'status':'implemented','path':best['path'],'symbol':'executed','evidence':{'kind':'corpus-report','report':best['path']}})
        else:owner.update({'status':'pending-audit'});owner.pop('evidence',None)
    focused={
        'annotations':[('annotations.json','wtime-conformance','t159_',['t159_case_01','t159_case_02','t159_case_03'])],
        'branch-hinting':[('branch-hints-decode.json','wtime-decode','metadata',['metadata::tests::t158_case_01','metadata::tests::t158_case_02','metadata::tests::t158_case_03']),('branch-hints-cli.json','wtime-cli','t158_',['t158_hints_do_not_change_execution'])],
    }
    for row in ledger['non_opcode_features']:
        proofs=[]
        for report_name,package,test_filter,names in focused.get(row['feature'],[]):
            candidates=[f'target/conformance/{report_name}',f'docs/verification/{report_name}']
            found_pair=next(((rel,root/rel) for rel in candidates if (root/rel).is_file()),None)
            if found_pair is None:break
            report_path,p=found_pair
            r=json.loads(p.read_text())
            expected=['cargo','test','--locked','-p',package,test_filter]
            if package=='wtime-conformance' and test_filter=='t159_' and r.get('command')==['cargo','test','--locked','-p',package,'--test','annotations']:expected=r['command']
            if r.get('command')!=expected or not r.get('passed') or not r.get('source_stable') or r.get('exit_code')!=0 or r.get('executed')!=len(names):break
            if not all('test '+name+' ... ok' in r.get('output','') for name in names):break
            if not all((root/file).is_file() and digest(root/file)==sha for file,sha in r.get('implementation_sources',{}).items()):break
            snapshot=root/'docs/architecture/opcode-evidence'/f'{digest(p)}.json'
            snapshot.parent.mkdir(parents=True,exist_ok=True)
            if not snapshot.exists():snapshot.write_bytes(p.read_bytes())
            proofs.append({'path':str(snapshot.relative_to(root)),'origin_report':report_path,'report_sha256':digest(p),'command':expected,'tests':names})
        if proofs and len(proofs)==len(focused.get(row['feature'],[])):
            row['owners']['test'].update({'status':'implemented','path':proofs[0]['path'],'symbol':'output','evidence':{'kind':'focused-tests','reports':proofs}})
    # Preserve commented-out historical syntax as reference, never an active rule.
    inactive_ids={r['id'] for r in ledger.get('inactive_validation_rules',[])}
    for source in {r['source'] for r in ledger.get('validation_rules',[])}:
        pinned=root/'.cache/spec'/source
        if not pinned.is_file():continue
        text=pinned.read_text();depth=0
        for line in text.splitlines():
            match=re.match(r'^rule\s+([^:]+):',line)
            if match and depth:inactive_ids.add(pinned.name+':'+match.group(1))
            i=0
            while i<len(line)-1:
                if line[i:i+2]=='(;':depth+=1;i+=2
                elif line[i:i+2]==';)':depth-=1;i+=2
                elif line[i:i+2]==';;':break
                else:i+=1
    inactive=[r for r in ledger.get('validation_rules',[]) if r['id'] in inactive_ids]
    for rule in inactive:rule.update({'status':'inactive-source-comment','reason':'Declaration is inside a block comment in the pinned normative source.'})
    if inactive:ledger['inactive_validation_rules']=inactive
    ledger['validation_rules']=[r for r in ledger.get('validation_rules',[]) if r['id'] not in inactive_ids]
    ledger['validation_rule_ids_sha256']=hashlib.sha256('\n'.join(sorted(r['id'] for r in ledger['validation_rules'])).encode()).hexdigest()
    audit_path=root/'docs/architecture/validation-premise-audit.json'
    for audit_path in [audit_path,root/'docs/architecture/types-premise-audit.json']:
        if not audit_path.is_file():continue
        audit=json.loads(audit_path.read_text())
        entries={entry['rule_id']:entry for entry in audit.get('entries',audit.get('rules',[]))}
        for rule in ledger.get('validation_rules',[]):
            entry=entries.get(rule['id'])
            if entry is None or not entry.get('all_direct_premises_covered',entry.get('status')=='audited') or entry.get('remaining_direct_premises'):continue
            # The author audits exactly this formal rule's implementation premises.
            # Test execution remains a separate release check, never inferred here.
            proof={'path':str(audit_path.relative_to(root)),'source_sha256':digest(audit_path),'rule_id':rule['id'],'handler':entry['handler'],'premise_checks':entry.get('premise_checks','; '.join(entry['covered_premises'])),'audit_scope':entry['audit_scope'],'covered_premises':entry['covered_premises'],'remaining_direct_premises':entry['remaining_direct_premises'],'all_test_boundaries_covered':entry['all_test_boundaries_covered']}
            proof['handler_source_sha256']=digest(root/entry['handler']['path'])
            tests=[]
            rule.pop('unbound_test_references',None)
            for item in entry['test_evidence']:
                if item.get('kind')=='locked-corpus-case':
                    fresh=[e for e in evidence.values() if e['suite']==item['required_suite'] and e['profile']==item['required_profile'] and e['implementation_sources'].get(entry['handler']['path'])==proof['handler_source_sha256'] and any((root/source).resolve()==(root/item['path']).resolve() and sha==item['source_sha256'] for source,_,sha in (part.rpartition(':') for part in e['source_hash'].split(';')))]
                    if fresh:
                        latest=min(fresh,key=lambda e:e['selected'])
                        item={**item,'passing_report':latest['path'],'passing_report_sha256':latest['report_sha256'],'report_binary_sha256':latest['binary_sha256']}
                    if digest(root/item['path'])!=item['source_sha256']:raise ValueError('changed locked premise case '+item['path'])
                    report=root/item['passing_report']
                    if digest(report)!=item['passing_report_sha256'] or successful_report(report) is None:raise ValueError('invalid premise corpus report')
                    snapshot=root/'docs/architecture/opcode-evidence'/f'{digest(report)}.json'
                    snapshot.parent.mkdir(parents=True,exist_ok=True)
                    if not snapshot.exists():snapshot.write_bytes(report.read_bytes())
                    tests.append({**item,'passing_report':str(snapshot.relative_to(root))})
                elif 'symbol' in item:
                    symbol=item['symbol'].split('::')[-1]
                    if symbol not in (root/item['path']).read_text():raise ValueError('missing premise test '+item['symbol'])
                    tests.append({**item,'symbol':symbol,'source_sha256':digest(root/item['path'])})
                else:
                    matching=[e for e in evidence.values() if e['suite'] in ('phase3','core3') and e['implementation_sources'].get(entry['handler']['path'])==proof['handler_source_sha256'] and any((root/source).resolve()==(root/item['path']).resolve() and sha==item['source_sha256'] for source,_,sha in (part.rpartition(':') for part in e['source_hash'].split(';')))]
                    if matching:
                        report=min(matching,key=lambda e:e['selected'])
                        tests.append({**item,'kind':'locked-corpus-file','passing_report':report['path'],'passing_report_sha256':report['report_sha256'],'report_binary_sha256':report['binary_sha256'],'required_suite':report['suite'],'required_profile':report['profile']})
                    else:
                        rule['unbound_test_references']=entry['test_evidence']
            rule.update({'status':'audited' if tests and entry.get('status') in ('implementation-premises-audited','audited') else 'pending-premise-audit','premise_evidence':[proof],'test_evidence':tests})
    ledger['execution_reports']=evidence
    ledger['schema_version']=2
    ledger['coverage_policy']='Source-handler presence and actual decoder dispatch are checked. An instruction test is complete only with a fully passing, binary-fingerprinted corpus report that records the exact executed opcode. This is instruction coverage, not exhaustive path coverage or a full semantic proof. Non-opcode rules remain separately audited.'
    ledger['reconciliation']={'instruction_test_owners_discharged':discharged,'instruction_test_owners_pending':len(ledger['instructions'])-discharged,'successful_traced_reports':len(evidence)}
    path.write_text(json.dumps(ledger,indent=2)+'\n');return ledger['reconciliation']

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--reports',type=Path,nargs='*',default=[]);a=p.parse_args()
    print(json.dumps(reconcile(ROOT,a.reports)))
if __name__=='__main__':main()
