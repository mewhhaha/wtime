#!/usr/bin/env python3
"""Audit frozen opcode ownership and exercise the decoder's real dispatch.

This is a structural/decoder check, not a substitute for semantic conformance.
Use --require-complete to fail on pending validation/execution/test evidence.
"""
from __future__ import annotations
import argparse
import hashlib
from functools import lru_cache
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
STAGES = ('decode', 'validate', 'execute', 'test')

@lru_cache(maxsize=4096)
def _hash_snapshot(path,mtime_ns,size):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def file_hash(path):
    stat=path.stat()
    return _hash_snapshot(str(path),stat.st_mtime_ns,stat.st_size)


def reserved_keys(ledger):
    for space in ledger.get('reserved', []):
        values=list(space['values'])
        for first,last in space.get('ranges',[]):values.extend([first,last])
        for value in values:
            yield f"{value:02x}" if space['prefix'] is None else f"{space['prefix']:02x}:{value}"


def check_ledger(ledger, scope_features, root=ROOT, probe=None, verify_freshness=True):
    errors, pending = [], []
    assigned = set()
    for row in ledger.get('instructions', []):
        key = row.get('encoding')
        if not isinstance(key, str) or not re.fullmatch(r'[0-9a-f]{2}(?::[0-9]+)?', key):
            errors.append(f'invalid encoding: {key!r}')
        if key in assigned:
            errors.append(f'duplicate encoding: {key}')
        assigned.add(key)
        if not row.get('source') or not row.get('source_commit'):
            errors.append(f'{key}: missing pinned source')
        _owners(row, key, errors, pending, root, ledger.get("execution_reports",{}), verify_freshness)
    reserved = set(reserved_keys(ledger))
    for key in assigned & reserved:
        errors.append(f'reserved opcode has an instruction/handler owner: {key}')
    for prefix in (None, 0xfb, 0xfc, 0xfd, 0xfe):
        spaces=[s for s in ledger.get('reserved',[]) if s['prefix']==prefix]
        if len(spaces)!=1:
            errors.append(f'missing or duplicate reserved space: {prefix}')
            continue
        for value in range(spaces[0]['examined_through']+1):
            if prefix is None and value in (0xfb,0xfc,0xfd,0xfe):
                continue
            key=f'{value:02x}' if prefix is None else f'{prefix:02x}:{value}'
            if key not in assigned|reserved:
                errors.append(f'unaccounted opcode hole: {key}')
    features = set()
    for row in ledger.get('non_opcode_features', []):
        feature=row.get('feature')
        if feature in features:
            errors.append(f'duplicate non-opcode feature: {feature}')
        features.add(feature)
        if not row.get('surface'):
            errors.append(f'{feature}: missing grammar/semantic surface')
        _owners(row, feature, errors, pending, root, ledger.get("execution_reports",{}), verify_freshness)
    for feature in scope_features-features:
        errors.append(f'missing non-opcode feature: {feature}')
    sections=ledger.get('sections',[])
    if {s.get('id') for s in sections} != set(range(14)):
        errors.append('section inventory must cover ids 0 through 13')
    for section in sections:
        for stage in STAGES:
            if not section.get('owners',{}).get(stage):
                errors.append(f"section {section.get('id')}: missing {stage} owner")
    rules=ledger.get('validation_rules',[])
    if ledger.get('schema_version',1)>=2:
        ids=[r.get('id','') for r in rules]
        identity=hashlib.sha256('\n'.join(sorted(ids)).encode()).hexdigest()
        if not rules or len(ids)!=len(set(ids)) or identity!=ledger.get('validation_rule_ids_sha256'):
            errors.append('formal validation rule inventory is missing or changed')
        for rule in rules:
            if not all(rule.get(k) for k in ('id','name','source','source_commit','source_sha256','rule')):
                errors.append('incomplete formal validation rule: '+rule.get('id','?'))
            if rule.get('status')=='pending-premise-audit':pending.append(rule['id']+':premise-audit')
            elif rule.get('status')=='audited':
                if not rule.get('premise_evidence') or not rule.get('test_evidence'):
                    errors.append('audited rule lacks premise/test proof: '+rule['id'])
                if verify_freshness:
                    for proof in rule.get('premise_evidence',[]):
                        audit=root/proof.get('path','')
                        handler=root/proof.get('handler',{}).get('path','')
                        if not audit.is_file() or file_hash(audit)!=proof.get('source_sha256'):
                            errors.append('formal premise audit changed: '+str(audit))
                        if not handler.is_file() or file_hash(handler)!=proof.get('handler_source_sha256'):
                            errors.append('formal premise handler changed: '+str(handler))
                    for proof in rule.get('test_evidence',[]):
                        test=root/proof.get('path','')
                        if proof.get('kind') in ('locked-corpus-case','locked-corpus-file'):
                            report_path=root/proof.get('passing_report','')
                            if not test.is_file() or file_hash(test)!=proof.get('source_sha256') or not report_path.is_file() or file_hash(report_path)!=proof.get('passing_report_sha256'):
                                errors.append('formal locked premise case/report changed: '+str(test));continue
                            if proof.get('kind')=='locked-corpus-case' and case_form_hash(test,proof.get('byte_offset',-1))!=proof.get('form_sha256'):
                                errors.append('formal premise case span changed: '+str(test))
                            for form in proof.get('active_forms',[]):
                                if not form.get('active') or form.get('comment') or case_form_hash(test,form.get('offset',-1))!=form.get('form_sha256'):
                                    errors.append('formal premise references inactive or changed form: '+str(test))
                            actual=json.loads(report_path.read_text())
                            if actual.get('suite')!=proof.get('required_suite') or actual.get('profile')!=proof.get('required_profile') or actual.get('binary_sha256')!=proof.get('report_binary_sha256') or actual.get('selected',0)<=0 or actual.get('selected')!=actual.get('executed') or any(actual.get(k,0) for k in ('failed','skipped','unsupported','text_tool_errors')):
                                errors.append('formal premise case lacks passing corpus: '+str(test))
                            if not any((root/source).resolve()==(root/proof['path']).resolve() and sha==proof['source_sha256'] for source,_,sha in (entry.rpartition(':') for entry in actual.get('source_hash','').split(';'))):
                                errors.append('formal premise case absent from corpus digest: '+str(test))
                            handler_path=rule['premise_evidence'][0]['handler']['path']
                            if actual.get('implementation_sources',{}).get(handler_path)!=file_hash(root/handler_path):
                                errors.append('formal premise corpus implementation changed: '+handler_path)
                        elif not test.is_file() or file_hash(test)!=proof.get('source_sha256') or proof.get('symbol','?') not in test.read_text():
                            errors.append('formal premise test changed: '+str(test))
            else:errors.append('unknown formal rule audit status: '+rule['id'])
    if probe:
        for key in assigned:
            if not probe(key):errors.append(f'missing decoder handler: {key}')
        for key in reserved:
            if probe(key):errors.append(f'reserved opcode acquired a decoder handler: {key}')
    return list(dict.fromkeys(errors)),pending


def active_root_offset(data, offset):
    i=0;depth=0;comment=0;quoted=False
    while i<offset:
        pair=data[i:i+2];c=data[i]
        if comment:
            if pair==b"(;":comment+=1;i+=2;continue
            if pair==b";)":comment-=1;i+=2;continue
        elif quoted:
            if c==92:i+=2;continue
            if c==34:quoted=False
        else:
            if pair==b"(;":comment=1;i+=2;continue
            if pair==b";;":
                end=data.find(b"\n",i)
                if end<0 or end>offset:return False
                i=end;continue
            if c==34:quoted=True
            elif c==40:depth+=1
            elif c==41:depth-=1
        i+=1
    return i==offset and not comment and not quoted and depth==0


def case_form_hash(path, offset):
    data=path.read_bytes();i=offset;depth=0;comment=0;quoted=False
    if i<0 or i>=len(data) or data[i]!=40 or not active_root_offset(data,offset):return None
    while i<len(data):
        pair=data[i:i+2];c=data[i]
        if comment:
            if pair==b"(;":comment+=1;i+=2;continue
            if pair==b";)":comment-=1;i+=2;continue
        elif quoted:
            if c==92:i+=2;continue
            if c==34:quoted=False
        else:
            if pair==b"(;":comment=1;i+=2;continue
            if pair==b";;":
                end=data.find(b"\n",i);i=len(data) if end<0 else end;continue
            if c==34:quoted=True
            elif c==40:depth+=1
            elif c==41:
                depth-=1
                if depth==0:return hashlib.sha256(data[offset:i+1]).hexdigest()
        i+=1
    return None


def _owners(row, name, errors, pending, root, reports=None, verify_freshness=True):
    for stage in STAGES:
        owner=row.get('owners',{}).get(stage,{})
        if not re.fullmatch(r'T[0-9]{3}', owner.get('task','')):
            errors.append(f'{name}: missing {stage} task owner')
        status=owner.get('status')
        if status not in ('implemented','pending-audit','not-applicable'):
            errors.append(f'{name}: invalid/missing {stage} coverage status')
        if status=='pending-audit':pending.append(f'{name}:{stage}')
        if status=='not-applicable' and not owner.get('reason'):errors.append(f'{name}: unexplained inapplicable {stage} stage')
        if status=='implemented':
            evidence=owner.get('evidence',{})
            if evidence.get('kind')=='focused-tests':
                if not evidence.get('reports'):errors.append(f'{name}: focused test evidence is empty')
                for proof in evidence.get('reports',[]):
                    p=root/proof.get('path','')
                    if not p.is_file() or file_hash(p)!=proof.get('report_sha256'):
                        errors.append('focused test report changed: '+str(p));continue
                    actual=json.loads(p.read_text())
                    names=proof.get('tests',[])
                    if not actual.get('passed') or not actual.get('source_stable') or actual.get('exit_code')!=0 or actual.get('command')!=proof.get('command') or actual.get('executed')!=len(names) or not names or not all('test '+name+' ... ok' in actual.get('output','') for name in names):
                        errors.append('focused tests not actually passed: '+str(p))
                    if verify_freshness:
                        for source,sha in actual.get('implementation_sources',{}).items():
                            source_path=root/source
                            if not source_path.is_file() or file_hash(source_path)!=sha:
                                errors.append('implementation changed since focused tests: '+source)
                                break
            elif evidence.get('kind') in ('executed-corpus','corpus-report'):
                report=(reports or {}).get(evidence.get('report'),{})
                p=root/report.get('path','')
                if (not p.is_file() or file_hash(p)!=report.get('report_sha256')
                    or not report.get('binary_sha256') or report.get('selected',0)<=0
                    or report.get('selected')!=report.get('executed')
                    or (evidence.get('kind')=='executed-corpus' and evidence.get('opcode') not in report.get('executed_opcodes',[]))):
                    errors.append(f'{name}: missing or changed actual execution evidence')
                elif p.is_file():
                    actual=json.loads(p.read_text())
                    for source,expected in report.get('implementation_sources',{}).items():
                        source_path=root/source
                        if not source_path.is_file() or file_hash(source_path)!=expected:
                            errors.append(f'implementation changed since execution report: {source}')
                            break
                    if any(actual.get(k,0) for k in ('failed','skipped','unsupported','text_tool_errors')):
                        errors.append(f'{name}: failed corpus cannot establish execution coverage')
                    if verify_freshness:
                        for item in report.get('source_hash','').split(';'):
                            source,sep,expected=item.rpartition(':')
                            if not sep:continue
                            source='docs/architecture/corpus-lock.json' if source=='corpus-lock' else source
                            source_path=root/source
                            if not source_path.is_file() or file_hash(source_path)!=expected:
                                errors.append(f'corpus changed since execution report: {source}')

            else:
                path=root/owner.get('path','')
                symbol=owner.get('symbol')
                if not path.is_file() or not symbol or symbol not in path.read_text():
                    errors.append(f'{name}: missing implemented {stage} handler or evidence')
                if verify_freshness and evidence.get('kind')=='source-handler' and path.is_file():
                    if file_hash(path)!=evidence.get('source_sha256'):
                        errors.append(f'{stage} source changed since evidence audit: {path}')


def probe_decoder(ledger):
    env=os.environ.copy()
    env['WTIME_ASSIGNED_OPCODES']=','.join(row['encoding'] for row in ledger['instructions'])
    env['WTIME_RESERVED_OPCODES']=','.join(reserved_keys(ledger))
    result=subprocess.run(['cargo','test','--locked','-p','wtime-decode','t008_actual_dispatch_coverage','--','--ignored','--exact','instructions::ledger_tests::t008_actual_dispatch_coverage'],cwd=ROOT,env=env,text=True,capture_output=True)
    if result.returncode:
        return [result.stdout+result.stderr]
    if '1 passed' not in result.stdout:
        return ['decoder dispatch probe executed no tests']
    return []


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ledger',type=Path,default=ROOT/'docs/architecture/opcodes.json')
    parser.add_argument('--require-complete',action='store_true')
    parser.add_argument('--no-probe',action='store_true',help='only check ownership/schema; skip actual Rust dispatch')
    parser.add_argument('--report',type=Path,help='write precise coverage gaps as JSON')
    args=parser.parse_args()
    ledger=json.loads(args.ledger.read_text())
    lock=json.loads((ROOT/'docs/architecture/spec-lock.json').read_text())
    scope={s['feature_id'] for s in lock['sources']}-{'proposal-catalog'}
    errors,pending=check_ledger(ledger,scope)
    if not errors and not args.no_probe:
        errors.extend(probe_decoder(ledger))
    if args.require_complete and pending:
        errors.append(f'{len(pending)} coverage owners remain pending audit')
    if args.report:
        summary={'assigned_instructions':len(ledger['instructions']),'errors':errors,'pending':pending,
          'missing_execution_evidence':[{'encoding':r['encoding'],'name':r['name']} for r in ledger['instructions'] if r['owners']['test']['status']!='implemented'],
          'pending_formal_rules':[r['id'] for r in ledger.get('validation_rules',[]) if r['status']!='audited'],
          'pending_nonopcode_stages':[{'feature':r['feature'],'stage':stage} for r in ledger['non_opcode_features'] for stage,owner in r['owners'].items() if owner['status']=='pending-audit']}
        args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(summary,indent=2)+'\n')
    for error in errors:print(error,file=sys.stderr)
    print(f"{len(ledger['instructions'])} assigned opcodes; {len(set(reserved_keys(ledger)))} explicit reserved encodings; {len(ledger['non_opcode_features'])} non-opcode feature rows; {len(pending)} pending coverage owners")
    if not errors:print('Ownership/decoder audit passed; semantic completion is not asserted.')
    return bool(errors)

if __name__=='__main__':
    sys.exit(main())
