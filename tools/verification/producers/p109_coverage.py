"""Refresh a separate selected-tier ledger without rewriting historical evidence."""
from __future__ import annotations
import argparse
import copy
import difflib
import hashlib
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(ROOT/'tools/checks'),str(ROOT/'tools/wtime-bench'),str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore,digest,need,normal,read_json,verify_normal,write_new
import opcode_coverage
import reconcile_opcode_evidence as mapping
import p109


def validate_receipt_sources(receipt,current):
    need(receipt.get('schema')=='wtime-verification-phase-v1' and receipt.get('task')=='P109'
         and receipt.get('phase')=='verify' and receipt.get('status')=='passed'
         and receipt.get('lane')=='full' and receipt.get('synthetic') is False,
         'full real P109 runtime checks missing')
    inputs=receipt.get('inputs',{})
    need(digest(inputs)==receipt.get('input_key'),'runtime receipt inputs changed')
    recorded=inputs.get('components',{}).get('runtime_sources',{}).get('files')
    need(recorded==current,'runtime premise tests were compiled from different sources')


def opcode_sets(ledger,reports):
    assigned={mapping.runtime_opcode(row['encoding']) for row in ledger['instructions']}
    need(len(assigned)==584,'assigned opcode inventory changed')
    need(set(reports)==set(p109.CONFIGURATIONS),'opcode configurations incomplete')
    result={}
    for configuration,rows in reports.items():
        observed=set()
        need(set(rows)==set(p109.SUITES),'opcode suite matrix incomplete')
        for suite,report in rows.items():
            p109.validate_configuration(report,suite,configuration)
            values=report.get('executed_opcodes')
            need(isinstance(values,list) and all(type(value) is int for value in values),'opcode counters malformed')
            observed.update(values)
        need(observed==assigned,'missing or unassigned executed opcode in '+configuration)
        result[configuration]={'assigned':len(assigned),'executed':len(observed),'missing':[]}
    return result


def collect(verify_dir,output):
    corpus=p109.validate_corpus()
    summary=read_json(p109.CORPUS)
    historical=ROOT/'docs/architecture/opcodes.json'
    ledger=copy.deepcopy(read_json(historical))
    receipt=read_json(verify_dir/'receipt.json')
    current=p109.sources();validate_receipt_sources(receipt,current)
    store=ArtifactStore(ROOT/'target/verification')
    from verification.runner import test_results
    commands={row['name']:row for row in receipt['commands']}
    names=set()
    for group in receipt['inputs']['definition']['tests']:
        command=commands[group['command']]
        need(command.get('exit_code')==0,'runtime premise test command failed')
        text=store.read(command['stdout']).decode()+'\n'+store.read(command['stderr']).decode()
        result=test_results(text,group)
        need(result==receipt['tests'][group['command']],'runtime premise test outcomes differ from immutable logs')
        names.update(name.rsplit('::',1)[-1] for name in result['names'])
    required={'p021_multivalue_nesting_and_peak_come_from_validation',
              'p021_wide_local_index_uses_full_validated_layout','p021_budget_decline_is_transactional_and_decoded_remains_usable',
              'p047_fused_results_and_overflow','p050_recursive_imported_effectful_leaves_decline_and_expansion_bounded',
              'p134_host_effect_fuel_and_foreign_refusal','t159_case_01','t158_case_01','t163_case_01'}
    need(required<=names,'selected premise or optimized-path tests are missing: '+str(sorted(required-names)))
    reports={name:{} for name in p109.CONFIGURATIONS}
    references={};matrix_references={}
    for row in summary['runs']:
        path=verify_normal(row['report']);report=read_json(path)
        reports[row['configuration']][row['suite']]=report
        matrix_references[row['configuration']+'/'+row['suite']]=normal(path)
        if row['configuration']=='decoded':references[row['suite']]=(path,report)
    coverage=opcode_sets(ledger,reports)
    output.mkdir(parents=True,exist_ok=False)
    compiled={name:sha for name,sha in current.items() if name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')
              or any(name==f'crates/{package}/Cargo.toml' or name.startswith(f'crates/{package}/src/')
                     for package in ('wtime-types','wtime-decode','wtime-validate','wtime-runtime'))}
    recorded={name:sha for name,sha in summary['source_hashes_before'].items()
              if name in compiled or (name.startswith('crates/') and '/src/' in name)}
    need(compiled==recorded,
         'normative matrix omits a current core source')
    compilation_sources={**current,**receipt['inputs']['components']['conformance']['files']}
    p109.source_matches(compilation_sources)
    review_path=ROOT/'docs/verification/performance/premise-review.md'
    old_root=ROOT/'target/performance/P017-before'
    import baseline
    baseline.verify_archive(old_root)
    reviewed=[]
    for name in ('crates/wtime-validate/src/lib.rs','crates/wtime-runtime/src/linker.rs','crates/wtime-runtime/src/lib.rs'):
        old=old_root/'source'/name;new=ROOT/name
        relative=name.replace('/','_')+'.diff'
        difference=''.join(difflib.unified_diff(old.read_text().splitlines(True),new.read_text().splitlines(True),fromfile=str(old),tofile=str(new)))
        (output/relative).write_text(difference)
        reviewed.append({'old':normal(old),'current':normal(new),'diff':normal(output/relative)})
    review={'schema':'wtime-p109-premise-revalidation-v1','status':'passed','historical_ledger':normal(historical),
            'historical_archive':normal(old_root/'manifest.json'),
            'review':normal(review_path),'compared_sources':reviewed,'core_sources':current,
            'tests':normal(verify_dir/'receipt.json'),'test_input_key':receipt['input_key'],'exhaustive_formal_proof':False,
            'all_test_boundaries_covered':False}
    write_new(output/'premise-review.json',review)
    evidence={}
    for suite,(path,report) in references.items():
        name=str(path)
        evidence[name]={'kind':'executed-corpus','path':name,'origin_report':name,
            'report_sha256':normal(path)['sha256'],'binary_sha256':report['binary_sha256'],
            'profile':report['profile'],'suite':suite,'source_hash':report['source_hash'],
            'selected':report['selected'],'executed':report['executed'],'executed_opcodes':report['executed_opcodes'],
            'implementation_sources':report['implementation_sources']}
    for row in ledger['instructions']:
        for stage,(file,symbol) in mapping.mappings(row['encoding']).items():
            owner=row['owners'][stage]
            owner.update(path=file,symbol=symbol,status='implemented',evidence=mapping.handler(ROOT,file,symbol,row['encoding']))
        opcode=mapping.runtime_opcode(row['encoding'])
        path,report=min((entry for entry in references.values() if opcode in entry[1]['executed_opcodes']),key=lambda entry:entry[1]['selected'])
        row['owners']['test'].update(path=str(path),symbol='executed_opcodes',status='implemented',
            evidence={'kind':'executed-corpus','report':str(path),'opcode':opcode})
    commands={row['name']:row for row in receipt['commands']}
    def focused(group):
        from verification.artifacts import ArtifactStore
        store=ArtifactStore(ROOT/'target/verification')
        command=commands[group];tests=receipt['tests'][group]
        proof=output/(group+'.json')
        text=store.read(command['stdout']).decode()+'\n'+store.read(command['stderr']).decode()
        write_new(proof,{'passed':True,'source_stable':True,'exit_code':0,'command':command['argv'],
            'executed':tests['passed'],'output':text,'implementation_sources':compilation_sources})
        return {'path':str(proof),'report_sha256':normal(proof)['sha256'],'command':command['argv'],'tests':tests['names']}
    for row in ledger['non_opcode_features']:
        for stage,pair in mapping.non_opcode_mappings(row['feature']).items():
            if pair is None:continue
            file,symbol=pair
            row['owners'][stage].update(path=file,symbol=symbol,status='implemented',evidence=mapping.handler(ROOT,file,symbol,row['feature']))
        owner=row['owners']['test']
        if row['feature'] in ('annotations','branch-hinting'):
            groups=['annotations'] if row['feature']=='annotations' else ['decoder-metadata','cli']
            proofs=[focused(group) for group in groups]
            owner.update(path=proofs[0]['path'],symbol='output',status='implemented',evidence={'kind':'focused-tests','reports':proofs})
        else:
            path,_=references['phase3']
            owner.update(path=str(path),symbol='executed',status='implemented',evidence={'kind':'corpus-report','report':str(path)})
    for rule in ledger['validation_rules']:
        for proof in rule['premise_evidence']:
            source=ROOT/proof['handler']['path']
            proof['previous_handler_source_sha256']=proof['handler_source_sha256']
            proof['handler_source_sha256']=normal(source)['sha256']
            proof['selected_source_review']=normal(output/'premise-review.json')
            if 'Context::function' in proof['handler'].get('symbol',''):
                proof['historical_handler_label']=proof['handler']['symbol']
                proof['handler']['symbol']='validate; Context::body; Context::ty; Context::valtype'
        for proof in rule['test_evidence']:
            source=ROOT/proof['path']
            if proof.get('kind') in ('locked-corpus-case','locked-corpus-file'):
                need(normal(source)['sha256']==proof['source_sha256'],'locked premise test changed: '+proof['path'])
                candidates=[]
                for path,report in references.values():
                    members={str(Path(name).resolve()):sha for name,_,sha in (item.rpartition(':') for item in report['source_hash'].split(';'))}
                    if members.get(str(source.resolve()))==proof['source_sha256']:candidates.append((path,report))
                need(bool(candidates),'locked premise case absent from new corpus: '+proof['path'])
                path,report=min(candidates,key=lambda entry:entry[1]['selected'])
                proof.update(passing_report=str(path),passing_report_sha256=normal(path)['sha256'],report_binary_sha256=report['binary_sha256'],required_suite=report['suite'],required_profile=report['profile'])
            else:
                need(proof.get('symbol','').rsplit('::',1)[-1] in names,'named premise test was not run: '+proof.get('symbol',''))
                proof['previous_source_sha256']=proof['source_sha256']
                proof['source_sha256']=normal(source)['sha256']
    ledger['execution_reports']=evidence
    ledger['selected_revalidation']={'historical_ledger':normal(historical),'review':normal(output/'premise-review.json'),
        'source_opcode_coverage':coverage,'corpus':corpus,'matrix_reports':matrix_references,
        'historical_core_commit':ledger.get('core_commit'),'exhaustive_formal_proof':False}
    ledger['coverage_policy']='Current source ownership and executed opcode coverage, with scoped premise revalidation. All historical incomplete boundary-coverage labels remain incomplete.'
    scope={row['feature_id'] for row in read_json(ROOT/'docs/architecture/spec-lock.json')['sources']}-{'proposal-catalog'}
    errors,pending=opcode_coverage.check_ledger(ledger,scope,root=ROOT,verify_freshness=True)
    need(not errors and not pending,'current selected ledger is incomplete: '+str(errors[:8])+str(pending[:8]))
    need(current==p109.sources(),'core source changed during selected ledger validation')
    write_new(output/'opcodes.json',ledger)
    write_new(output/'report.json',{'schema':'wtime-p109-opcode-coverage-v1','status':'passed','timing_evidence':False,
        'coverage':coverage,'matrix_reports':matrix_references,'tests':normal(verify_dir/'receipt.json'),
        'test_input_key':receipt['input_key'],'ledger':normal(output/'opcodes.json'),'review':normal(output/'premise-review.json'),
        'formal_rules':len(ledger['validation_rules']),'exhaustive_formal_proof':False,'core_sources':current})


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--verify-dir',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();collect(args.verify_dir,args.output)
