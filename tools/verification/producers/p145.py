"""Untimed, source-bound per-workload Register path observations."""
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
from verification.artifacts import ArtifactStore, VerificationError, need, normal, read_json, verify_normal, write_new

FIELDS={'entry','result','register_dispatches','register_exits','fallback_frames','classification',
        'working_peak_bytes','working_peak_frames','capacity_samples','cache_retained_bytes','active_scopes_after'}


def integer(value,label):
    need(type(value) is int and 0<=value<2**64,label+' must be a u64 integer')
    return value


def classify(dispatches,exits,fallback):
    for label,value in [('dispatches',dispatches),('exits',exits),('fallback',fallback)]:integer(value,label)
    if exits:return 'prefix_exit'
    if dispatches and fallback:return 'mixed'
    if dispatches:return 'optimized'
    need(fallback>0,'empty execution path')
    return 'whole_function_fallback'


def validate_wire(payload,strategy,threshold,expected,compile_only=False):
    need(isinstance(payload,dict) and set(payload)=={'schema','instrumented','timing_evidence','compiled','compile_only','strategy','threshold','invocations'},'path wire fields differ')
    need(payload['schema']=='wtime-workload-path-v1' and payload['instrumented'] is True
         and payload['timing_evidence'] is False,'path wire is not an untimed instrumented observation')
    need(payload['compiled'] is True and payload['compile_only'] is compile_only,'compile-only status differs')
    need(strategy in ('eager','adaptive') and type(threshold) is int
         and ((strategy=='eager' and threshold==0) or (strategy=='adaptive' and threshold==3)),'unsupported path strategy')
    need(payload['strategy']==strategy and type(payload['threshold']) is int and payload['threshold']==threshold,'path strategy differs')
    rows=payload['invocations']
    need(isinstance(rows,list) and len(rows)==(0 if compile_only else 1 if strategy=='eager' else threshold+1),'incomplete invocation sequence')
    for index,row in enumerate(rows,1):
        need(isinstance(row,dict) and set(row)==FIELDS,'invocation fields differ')
        need(type(row['entry']) is int and row['entry']==index,'invocation order differs')
        need(isinstance(row['result'],str) and (row['result']==expected or (expected=='trap' and row['result'].startswith('trap:'))),'path oracle mismatch')
        for field in FIELDS-{'result','classification','cache_retained_bytes'}:integer(row[field],field)
        need(row['classification']==classify(row['register_dispatches'],row['register_exits'],row['fallback_frames']),'path classification contradicts counters')
        need(row['active_scopes_after']==0 and row['capacity_samples']>0,'path scope incomplete')
        hot=row['register_dispatches']>0 or row['register_exits']>0
        need((row['working_peak_bytes']>0)==hot and row['working_peak_frames']==int(hot),'activation disagrees with actual path')
        if strategy=='eager':need(row['cache_retained_bytes'] is None,'eager path reports a lazy-cache charge')
        else:integer(row['cache_retained_bytes'],'cache_retained_bytes')
    return rows


def current_core():
    import p067_workflow
    archive=read_json(p067_workflow.ARCHIVE/'manifest.json')
    need(hashlib.sha256((p067_workflow.ARCHIVE/'manifest.json').read_bytes()).hexdigest()==p067_workflow.ARCHIVE_SHA,'predecessor changed')
    sources=p067_workflow.core_map(archive)
    for name,digest in sources.items():
        need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,'core source changed: '+name)
    return sources


def tool_sources():
    paths=[ROOT/'tools/wtime-path-probe'/name for name in ('Cargo.toml','Cargo.lock')]
    paths.extend(sorted((ROOT/'tools/wtime-path-probe/src').rglob('*.rs')))
    return {str(path.relative_to(ROOT)):hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def case_settings(entry):
    import candidate_gate
    import focused_compare
    import memory_scaling
    import scaling
    compile_only=entry['id']=='prepare_large'
    module=entry['oracle']['module']
    driver='compile_only' if compile_only else 'representative' if module=='representative_workloads.py' else (
        memory_scaling._driver(entry) if module=='memory_workloads.py' else
        scaling._driver(entry) if module=='scalar_workloads.py' else entry.get('driver',{}).get('kind','plain'))
    need(driver in {'plain','host_tick','table_a','representative','alias','multi','seeded-copy','seeded-fill','compile_only'},'unsupported setup driver')
    parts=[]
    for param in entry.get('params',[]):
        kind=param['type'];value=param.get('baseline',param.get('value'))
        need(kind in {'i32','i64','f32_bits','f64_bits','v128_bits'} and value is not None,'invalid typed argument')
        parts.append(kind+':'+str(value))
    expected=candidate_gate.expected(entry)
    need(compile_only or type(expected) is int or expected=='trap','unsupported independent oracle')
    return {'id':entry['id'],'profile':focused_compare.profile(entry),'driver':driver,
        'arguments':';'.join(parts) if parts else '-','expected':'-' if compile_only else 'trap' if expected=='trap' else 'i32:'+str(expected),
        'compile_only':compile_only,'required_features':entry.get('required_features',[])}


def validate_cases(cases,entries):
    need(isinstance(cases,list) and len(cases)==len(entries),'case count differs')
    wanted={entry['id']:case_settings(entry) for entry in entries}
    seen=set()
    for case in cases:
        need(isinstance(case,dict) and case.get('id') in wanted and case['id'] not in seen,'unknown or duplicate case')
        seen.add(case['id'])
        need(set(case)==set(wanted[case['id']])|{'fixture','source'},'unexpected or missing case fields')
        need({key:case.get(key) for key in wanted[case['id']]}==wanted[case['id']],'case settings differ: '+case['id'])
        source=next(entry['source'] for entry in entries if entry['id']==case['id'])
        need(case['fixture']['sha256']==source['wasm_sha256'],'fixture identity differs')
        verify_normal(case['fixture'])
        if case['id']!='prepare_large':
            need(case['source']['sha256']==source['sha256'],'WAT identity differs')
            verify_normal(case['source'])
        else:need(case['source'] is None,'generator row must use the frozen generator identity')
    need(seen==set(wanted),'missing case')


def frozen_entries(freeze):
    import p067
    import selection_manifest
    declarations={name:read_json(verify_normal(freeze['selections'][name])) for name in ('eager','adaptive')}
    p067.validate_coverage(declarations['eager'],declarations['adaptive'])
    return {name:selection_manifest.rows(value) for name,value in declarations.items()}


def freeze(output):
    import focused_compare
    import p067
    import p067_workflow
    import selection_manifest
    before=current_core()
    declarations={name:read_json(path) for name,path in p067_workflow.SELECTIONS.items()}
    p067.validate_coverage(declarations['eager'],declarations['adaptive'])
    entries={name:selection_manifest.rows(value) for name,value in declarations.items()}
    directory=output.parent/'fixtures';directory.mkdir(parents=True,exist_ok=False)
    fixtures=focused_compare.prepare_fixtures(entries['eager'],p067_workflow.ARCHIVE,directory,ROOT/'target/debug/wtime-conformance')
    cases={}
    for name,rows in entries.items():
        cases[name]=[]
        for entry in rows:
            case=case_settings(entry)
            case['fixture']=normal(fixtures[entry['id']])
            case['source']=normal(BENCH/entry['source']['path']) if entry['id']!='prepare_large' else None
            cases[name].append(case)
        validate_cases(cases[name],rows)
    need(before==current_core(),'core changed while freezing')
    write_new(output,{'schema':'wtime-p145-freeze-v1','status':'passed','timing_evidence':False,'core_sources':before,
        'selections':{name:normal(path) for name,path in p067_workflow.SELECTIONS.items()},'cases':cases,
        'foundation_evidence':{task:normal(ROOT/f'docs/benchmarks/performance/{task}-proof.json') for task in ('P143','P144')}})


def argv_for(binary,case,strategy):
    threshold=0 if strategy=='eager' else 3
    return [str(binary),str(verify_normal(case['fixture'])),'profile:'+case['profile'],strategy,str(threshold),
        case['arguments'],case['expected'],case['driver'],str(0 if case['compile_only'] else 1 if strategy=='eager' else 4)]


def collect(freeze_path,binary,output):
    before=current_core();frozen=read_json(freeze_path)
    need(frozen.get('status')=='passed' and frozen['core_sources']==before,'freeze is stale')
    need(binary.resolve()==(ROOT/'tools/wtime-path-probe/target/release/wtime-path-probe').resolve(),'unexpected build output path')
    tool_before=tool_sources();built_binary=normal(binary)
    entries=frozen_entries(frozen)
    output.mkdir(parents=True,exist_ok=False)
    executable=output/'wtime-path-probe';shutil.copy2(binary,executable)
    records={};cas=ArtifactStore(ROOT/'target/verification');artifacts={}
    for strategy in ('eager','adaptive'):
        validate_cases(frozen['cases'][strategy],entries[strategy]);records[strategy]=[]
        directory=output/strategy;directory.mkdir()
        for case in frozen['cases'][strategy]:
            argv=argv_for(executable,case,strategy);base=directory/case['id']
            command=base.with_suffix('.command.json');write_new(command,{'argv':argv,'cwd':str(ROOT)})
            result=subprocess.run(argv,cwd=ROOT,capture_output=True,text=True,timeout=180)
            stdout=base.with_suffix('.json');stderr=base.with_suffix('.stderr')
            stdout.write_text(result.stdout);stderr.write_text(result.stderr)
            status=base.with_suffix('.result.json');write_new(status,{'returncode':result.returncode})
            need(result.returncode==0,'path probe failed: '+case['id']+': '+result.stderr[-2000:])
            validate_wire(read_json(stdout),strategy,0 if strategy=='eager' else 3,case['expected'],case['compile_only'])
            record={'id':case['id'],'stdout':normal(stdout),'stderr':normal(stderr),'command':normal(command),'result':normal(status)}
            records[strategy].append(record)
            for path in (command,stdout,stderr,status):artifacts[str(path.relative_to(output))]=cas.add(path)
    artifacts['wtime-path-probe']=cas.add(executable)
    need(before==current_core(),'core changed during path collection')
    need(tool_sources()==tool_before and normal(binary)==built_binary,'tool source or build output changed during collection')
    write_new(output/'records.json',records);write_new(output/'artifacts.json',artifacts)
    write_new(output/'report.json',{'schema':'wtime-p145-observations-v1','status':'passed','timing_evidence':False,
        'freeze':normal(freeze_path),'executable':normal(executable),'records':normal(output/'records.json'),
        'artifacts':normal(output/'artifacts.json'),'source_stable':True,'tool_sources':tool_before,'build_output':built_binary})


def validate_report(freeze_path,report_path):
    frozen=read_json(freeze_path);report=read_json(report_path)
    need(frozen['core_sources']==current_core(),'path report core is stale')
    need(report.get('schema')=='wtime-p145-observations-v1' and report.get('status')=='passed' and
         report.get('timing_evidence') is False and report.get('source_stable') is True,'invalid observation report')
    need(verify_normal(report['freeze'])==freeze_path.resolve(),'different frozen selection')
    binary=verify_normal(report['executable']);cas=ArtifactStore(ROOT/'target/verification')
    need(report['tool_sources']==tool_sources(),'tool sources changed')
    built=verify_normal(report['build_output'])
    need(built==(ROOT/'tools/wtime-path-probe/target/release/wtime-path-probe').resolve() and
         report['build_output']['sha256']==report['executable']['sha256'],'executable is not the recorded build output')
    artifacts=read_json(verify_normal(report['artifacts']))
    for reference in artifacts.values():cas.verify(reference)
    records=read_json(verify_normal(report['records']));entries=frozen_entries(frozen);summary={}
    need(set(records)=={'eager','adaptive'},'wrong strategies')
    expected_artifacts={'wtime-path-probe'}
    for strategy in records:
        validate_cases(frozen['cases'][strategy],entries[strategy])
        cases={case['id']:case for case in frozen['cases'][strategy]}
        need(len(records[strategy])==len(cases) and {row['id'] for row in records[strategy]}==set(cases),'incomplete or duplicate path records')
        counts={name:0 for name in ('optimized','prefix_exit','mixed','whole_function_fallback')};invocations=0;observed={}
        for record in records[strategy]:
            need(set(record)=={'id','stdout','stderr','command','result'},'unexpected or missing record fields')
            case=cases[record['id']]
            need(read_json(verify_normal(record['command']))=={'argv':argv_for(binary,case,strategy),'cwd':str(ROOT)},'path argv changed')
            result=read_json(verify_normal(record['result']))
            need(set(result)=={'returncode'} and type(result['returncode']) is int and result['returncode']==0,'probe did not succeed')
            verify_normal(record['stderr'])
            for key in ('stdout','stderr','command','result'):
                path=verify_normal(record[key]);name=path.relative_to(report_path.parent).as_posix()
                expected_artifacts.add(name)
                need(name in artifacts and artifacts[name]['sha256']==record[key]['sha256'] and
                     artifacts[name]['bytes']==record[key]['bytes'],'CAS artifact binding differs')
            rows=validate_wire(read_json(verify_normal(record['stdout'])),strategy,0 if strategy=='eager' else 3,case['expected'],case['compile_only'])
            observed[case['id']]=rows
            for row in rows:counts[row['classification']]+=1
            invocations+=len(rows)
        need(counts['optimized']>0 and counts['whole_function_fallback']>0,'optimized and fallback coverage are both required')
        integer_rows=observed['integer_loop']
        need(integer_rows[-1]['classification']=='optimized','integer loop did not run the optimized path')
        if strategy=='adaptive':
            need([row['classification'] for row in integer_rows]==['whole_function_fallback','whole_function_fallback','optimized','optimized'],
                 'integer loop did not transition at threshold three')
        else:
            need(observed['simd_scalar_float_arithmetic_n17'][0]['classification']=='optimized','strict scalar float did not run optimized')
            need(observed['simd_float_arithmetic_n17'][0]['classification']=='whole_function_fallback','vector fallback coverage differs')
        summary[strategy]={'selected':len(cases),'runnable':len(cases)-1,'compile_only':['prepare_large'],'invocations':invocations,'classes':counts}
    need(set(artifacts)==expected_artifacts and artifacts['wtime-path-probe']['sha256']==report['executable']['sha256'],'artifact index incomplete')
    return summary


def summarize(freeze_path,report_path,output):
    summary=validate_report(freeze_path,report_path)
    receipt_path=report_path.parent.parent/'receipt.json';receipt=read_json(receipt_path)
    need(receipt.get('task')=='P145' and receipt.get('status')=='passed' and receipt.get('lane')=='full'
         and receipt.get('synthetic') is False,'source-bound full verification receipt missing')
    components=receipt['inputs']['components']
    need(components['runtime_sources']['files']==current_core() and components['tool']['files']==tool_sources(),
         'executed build sources differ')
    build=[row for row in receipt['commands'] if row['name']=='build']
    need(len(build)==1 and build[0]['argv']==['cargo','build','--locked','--offline','-j','1','--release','--manifest-path','tools/wtime-path-probe/Cargo.toml']
         and build[0]['exit_code']==0,'source-bound Cargo build did not succeed')
    need(receipt['original_outputs']['binary']['sha256']==read_json(report_path)['executable']['sha256'],
         'build executable differs from verified output')
    foundations=read_json(freeze_path)['foundation_evidence']
    need(set(foundations)=={'P143','P144'},'threshold and capacity foundations are both required')
    for task,reference in foundations.items():
        proof=read_json(verify_normal(reference))
        need(proof.get('task')==task and proof.get('status')=='passed' and proof.get('timing_evidence') is False,'invalid foundation proof')
    prior_path=report_path.parent.parent/'prior-controls.json'
    prior=read_json(prior_path)
    need(prior.get('status')=='passed' and prior.get('source_current') is True,'current P144 controls missing')
    for name in ('evidence','capacity'):verify_normal(prior[name])
    write_new(output,{'schema':'wtime-p145-proof-v1','task':'P145','status':'passed','timing_evidence':False,
        'freeze':normal(freeze_path),'observations':normal(report_path),'coverage':summary,'core_sources':current_core(),
        'runtime_controls':normal(prior_path),'foundation_evidence':foundations,'verification_receipt':normal(receipt_path),
        'tests':{name:value['passed'] for name,value in receipt['tests'].items()},
        'limitations':['Untimed diagnostic build; no execution-policy, promotion or performance claim.',
                       'Path classes describe each full invocation; mixed means observed optimized and fallback frames without a prefix exit.']})


def main():
    parser=argparse.ArgumentParser();parser.add_argument('phase',choices=['freeze','collect','summarize'])
    parser.add_argument('--freeze',type=Path);parser.add_argument('--binary',type=Path)
    parser.add_argument('--report',type=Path);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if args.phase=='freeze':freeze(args.output)
    elif args.phase=='collect':collect(args.freeze,args.binary,args.output)
    else:summarize(args.freeze,args.report,args.output)


if __name__=='__main__':main()
