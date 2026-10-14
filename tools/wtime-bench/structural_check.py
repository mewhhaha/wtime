#!/usr/bin/env python3
"""Fail-closed source-bound P035 structural milestone checker.

Input schema `wtime-p035-input-v1` is a normalized evidence index. Every file
reference is {path, sha256, bytes?}; measurement summaries bind each run to
its comparison report, analysis, journal, current source map, binary and exact
workload IDs. The checker never runs benchmarks or infers missing results.
"""
from __future__ import annotations
import argparse, hashlib, json, math, sys
from pathlib import Path

SCHEMA='wtime-p035-input-v1'
RUNTIME_COUNT=20
LEGACY={'call_overhead','integer_loop','branch_calls'}
FOUNDATIONS={'P018','P019','P020','P021','P022'}
CANDIDATES={f'P{i:03d}' for i in range(23,35)}
SELECTIONS={'scalar':85,'memory':35,'representative':9}

class CheckError(ValueError): pass
def need(ok,msg):
    if not ok: raise CheckError(msg)
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def canonical_sha(value): return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def json_ref(ref,label):
    verify_ref(ref,label)
    try: val=json.loads(Path(ref['path']).read_text(encoding='utf-8'))
    except (OSError,json.JSONDecodeError) as e: raise CheckError(f'{label} JSON invalid: {e}') from e
    need(isinstance(val,dict),f'{label} must be an object')
    return val
def verify_ref(ref,label):
    need(isinstance(ref,dict) and isinstance(ref.get('path'),str) and isinstance(ref.get('sha256'),str),f'{label} must be a path/SHA reference')
    p=Path(ref['path']); need(p.is_file(),f'{label} is missing: {p}')
    need(digest(p)==ref['sha256'],f'{label} is stale: {p}')
    if 'bytes' in ref: need(p.stat().st_size==ref['bytes'],f'{label} byte count changed: {p}')
    return p

def runtime_map(mapping,label):
    need(isinstance(mapping,dict),f'{label} runtime map missing')
    out={k:v for k,v in mapping.items() if k.startswith('crates/wtime-runtime/src/')}
    need(len(out)==RUNTIME_COUNT,f'{label} must bind all {RUNTIME_COUNT} runtime source files (found {len(out)})')
    need(all(isinstance(v,str) and len(v)==64 for v in out.values()),f'{label} contains malformed source hashes')
    return out

def archived_runtime_map(manifest,label):
    files=manifest.get('files',{})
    out={}
    for archive_path,row in files.items():
        if not isinstance(row,dict) or row.get('role')!='source': continue
        rel=row.get('source_path') or archive_path.removeprefix('source/')
        if rel.startswith('crates/wtime-runtime/src/'): out[rel]=row.get('sha256')
    return runtime_map(out,label)

def validate_tasks(data):
    tasks=data.get('tasks',{})
    need(isinstance(tasks,dict), 'task outcome inventory missing')
    index=json_ref(data.get('task_index'),'performance task index')
    index_rows=index.get('tasks',[])
    indexed={row.get('id'):row for row in index_rows if isinstance(row,dict)}
    for task in FOUNDATIONS|{'P017'}:
        row=tasks.get(task,{}) ; actual=indexed.get(task,{})
        need((row.get('status'),row.get('outcome'))==(actual.get('status'),actual.get('outcome')),f'{task} normalized outcome differs from TASKS.json')
    for task in FOUNDATIONS:
        row=tasks.get(task,{})
        need(row.get('status')=='done' and row.get('outcome') in ('complete','contract_only'),f'{task} required foundation is not closed')
    adopted=tasks.get('P017',{})
    need(adopted.get('status')=='done' and adopted.get('outcome')=='adopted' and adopted.get('promoted') is True,'P017 is not the verified adopted baseline')
    for task in sorted(CANDIDATES):
        row=tasks.get(task,{})
        actual=indexed.get(task,{})
        need((row.get('status'),row.get('outcome'))==(actual.get('status'),actual.get('outcome')),f'{task} normalized outcome differs from TASKS.json')
        need(row.get('status')=='done' and row.get('outcome') in ('rejected','inconclusive','adopted','contract_only'),f'{task} remains pending or has unknown outcome')
        if row['outcome'] in ('rejected','inconclusive'):
            need(row.get('promoted') is False,f'{task} rejected/inconclusive source is incorrectly marked promoted')
        if row['outcome']=='adopted': need(row.get('promoted') is True,f'{task} adopted outcome lacks explicit promotion')
        for key,ref in row.get('artifacts',{}).items(): verify_ref(ref,f'{task} {key}')
    p034=tasks['P034']
    actual=indexed.get('P034',{})
    need((p034.get('status'),p034.get('outcome'))==(actual.get('status'),actual.get('outcome')),'P034 normalized outcome differs from TASKS.json')
    need(p034.get('status')=='done' and p034.get('outcome') in ('rejected','inconclusive') and p034.get('promoted') is False,'P035 requires a closed, rejected/inconclusive, non-promoted P034 result')
    return tasks

def validate_p017(data):
    p=data.get('p017',{})
    decision=json_ref(p.get('decision'),'P017 decision')
    report=json_ref(p.get('report'),'P017 report')
    build=json_ref(p.get('build'),'P017 build')
    tests=verify_ref(p.get('runtime_tests'),'P017 runtime tests')
    source=verify_ref(p.get('module_source'),'current Module source hunk')
    need(decision.get('decision')=='adopted' and decision.get('task')=='P017','P017 decision is not adopted')
    need(report.get('passed') is True and report.get('source_stable') is True,'P017 comparison report failed/stale')
    need(build.get('source_stable') is True and build.get('mode')=='default' and build.get('features')==[],'P017 default build is not source-stable/default')
    log=tests.read_text(errors='replace')
    names=('p017_large_clone_shares_all_payload_storage','p017_clone_retains_executable_after_original_drop')
    need(all(f'test ' in log and name+' ... ok' in log for name in names),'P017 immutable-payload behavior tests are absent or failed')
    text=source.read_text(errors='replace')
    need('struct ModuleData' in text and 'Arc<ModuleData>' in text,'P017 immutable ModuleData/Arc hunk is absent from selected source')
    identity=decision.get('source_identity',{})
    need(identity.get('comparison_report_sha256')==digest(p['report']['path']),'P017 decision/report hash mismatch')
    return {'decision':p['decision'],'report':p['report'],'build':p['build'],'tests':p['runtime_tests'],'module_source':p['module_source']}

def validate_closure(data):
    p=data.get('p033_closure',{})
    done=json_ref(p.get('completion'),'P033 completed evidence')
    rollback=json_ref(p.get('rollback'),'P033 restore identity')
    need(done.get('task')=='P033' and done.get('status')=='done' and done.get('candidate_promoted') is False,'P033 predecessor is not a finalized non-promoted closure')
    need(rollback.get('production_restored') is True and rollback.get('reverted') is True and rollback.get('freshness_proven') is True,'P033 restored-source closure is not fresh')
    return rollback

def validate_source_chain(data,rollback):
    before=json_ref(data.get('p034_before_manifest'),'immediate P034 predecessor archive manifest')
    need(before.get('mode')=='decoded','P034 predecessor archive must be decoded')
    before_root=Path(data.get('p034_before_archive_root',''))
    p033_archive=json_ref(data.get('p033_closure',{}).get('archive_manifest'),'P033 predecessor archive manifest')
    p033_root=Path(data.get('p033_closure',{}).get('archive_root',''))
    need(before_root.is_dir() and p033_root.is_dir(),'P034/P033 archive source trees are missing')
    b=archived_runtime_map(before,'P034-before')
    p=archived_runtime_map(p033_archive,'P033 predecessor')
    need(p033_archive.get('mode')=='decoded','P033 predecessor archive must be decoded')
    need(b==p,'P034 archive is not the immediate P033 restored source state')
    for label,manifest,source_hashes,archive_root in (('P034-before',before,b,before_root),('P033 predecessor',p033_archive,p,p033_root)):
        for rel,source_hash in source_hashes.items():
            path=archive_root/'source'/rel
            need(path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest()==source_hash,f'{label} archived source differs from its manifest: {rel}')
    current_root=Path(data.get('project_root',''))
    need(current_root.is_dir(),'current project root is missing')
    c={rel:hashlib.sha256((current_root/rel).read_bytes()).hexdigest() for rel in b if (current_root/rel).is_file()}
    need(len(c)==RUNTIME_COUNT,'current production runtime source file is missing')
    need(c==runtime_map(data.get('current_runtime_sources'),'current runtime manifest'),'current source manifest differs from live production files')
    need(c==b,'current production runtime differs from immediate P034 predecessor')
    restored=rollback.get('restored_runtime',{})
    for name,row in restored.items():
        need(name in c and c[name]==row.get('restored_sha256') and row.get('byte_identical') is True,'P033 restored closure does not bind current runtime file '+name)
    return {'runtime_files':len(c),'runtime_source_sha256':hashlib.sha256(json.dumps(c,sort_keys=True,separators=(',',':')).encode()).hexdigest()}

def report_pairs(report,label,expected_ids):
    need(report.get('schema_version')==2 and report.get('passed') is True and report.get('source_stable') is True,f'{label} comparison report failed/stale')
    runs=report.get('runs',[]); run_ids=[r.get('run_id') for r in runs]
    need(len(runs)==2 and run_ids in ([0,1],[1,2]),f'{label} must contain exactly two ordered native runs')
    groups={}; pairs={}; analysis_groups={}; seen_ids=set()
    for analysis_run,run in enumerate(runs,1):
        rid=run['run_id']; run_ids=set()
        for pair in run.get('pairs',[]):
            wid=pair.get('workload'); stage=pair.get('stage'); pair_id=pair.get('pair_id')
            need(wid in expected_ids and isinstance(stage,str) and stage and isinstance(pair_id,str),f'{label} has unsupported/malformed pair')
            key=(rid,wid,stage); groups[key]=groups.get(key,0)+1; run_ids.add(wid)
            analysis_key=(analysis_run,wid,stage)
            analysis_groups[analysis_key]=analysis_groups.get(analysis_key,0)+1
            need((rid,pair_id) not in pairs,f'{label} duplicate pair id {pair_id}')
            for side in ('baseline','candidate'):
                val=pair.get(side,{})
                ns=val.get('ns_per_operation')
                need(val.get('mode')=='decoded' and type(ns) in (int,float) and math.isfinite(ns) and ns>0,f'{label} has zero/invalid {side} timing')
            pairs[(rid,pair_id)]=pair
        need(run_ids==expected_ids,f'{label} run {rid} has missing/extra workload IDs')
    need(groups and all(n>=30 for n in groups.values()),f'{label} has a group with fewer than 30 pairs')
    need({key[1] for key in groups}==expected_ids,f'{label} full workload coverage missing')
    return groups,pairs,{str(r['run_id']):r.get('seed') for r in runs},analysis_groups

def validate_journal(ref,label,report,report_pair_map,run_seeds):
    path=verify_ref(ref,label)
    try: lines=[json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]
    except (OSError,json.JSONDecodeError) as exc: raise CheckError(f'{label} malformed JSONL: {exc}') from exc
    need(lines and lines[0].get('type')=='header' and lines[0].get('schema')=='wtime-compare-checkpoint-v1',f'{label} native header missing')
    header=lines[0]; identity=header.get('identity',{})
    need(header.get('identity_sha256')==canonical_sha(identity),f'{label} identity digest mismatch')
    need(identity.get('selection')==report.get('selection') and identity.get('metadata')==report.get('metadata'),f'{label} header/report identity mismatch')
    need(header.get('run_seeds')==run_seeds,f'{label} run seeds differ from report')
    events=lines[1:]; need(len(events)==len(report_pair_map),f'{label} pair event count differs from report')
    seen={}
    for event in events:
        need(event.get('type')=='pair',f'{label} contains non-pair event')
        key=(event.get('run_id'),event.get('pair',{}).get('pair_id'))
        need(key in report_pair_map and key not in seen,f'{label} has missing/duplicate/extra pair event')
        need(event.get('pair')==report_pair_map[key],f'{label} pair payload differs from report')
        seen[key]=True
    need(set(seen)==set(report_pair_map),f'{label} does not reconcile exactly with report pairs')

def validate_analysis(ref,label,analysis,expected_groups):
    need(analysis.get('schema_version')==1,f'{label} analysis schema is not version 1')
    need(isinstance(analysis.get('metadata'),dict),f'{label} analysis metadata is missing')
    rows=analysis.get('per_run',[])
    need(analysis.get('source_schema_version')==2 and rows,f'{label} native paired analysis is missing')
    found={}
    for row in rows:
        key=(row.get('run'),row.get('workload'),row.get('stage'))
        need(key in expected_groups and key not in found,f'{label} has unexpected/duplicate analysis group {key}')
        need(row.get('pairs')==expected_groups[key],f'{label} analysis pair count differs for {key}')
        found[key]=row
    need(set(found)==set(expected_groups),f'{label} analysis groups differ from raw report')
    return rows

def validate_fixture_map(report,registry,selection,expected_ids):
    entries={row.get('id'):row for row in registry.get('workloads',[]) if isinstance(row,dict)}
    need(expected_ids<=set(entries),f'{selection} registry entries are incomplete')
    fixture_rows=report.get('metadata',{}).get('fixtures',[])
    found={row.get('id'):row for row in fixture_rows if isinstance(row,dict)}
    need(set(found)==expected_ids,f'{selection} native fixture inventory has missing/extra IDs')
    for wid in expected_ids:
        source=entries[wid].get('source',{})
        row=found[wid]
        wat=row.get('source_sha256') or row.get('wat_sha256')
        wasm=row.get('sha256') or row.get('wasm_sha256')
        need(wat==source.get('sha256') and wasm==source.get('wasm_sha256'),
             f'{selection} fixture WAT/Wasm hashes differ from registry: {wid}')
    return found

def validate_portfolio(data):
    registry=json_ref(data.get('workload_registry'),'workload registry')
    selections=registry.get('selections',{})
    for name,count in SELECTIONS.items():
        row=selections.get(name,{})
        ids=row.get('workloads',[])
        need(row.get('status')=='ready' and len(ids)==count and len(set(ids))==count,f'{name} selection must contain exactly {count} unique ready IDs')
    expected={name:set(selections[name]['workloads']) for name in SELECTIONS}
    current={}
    measurements=data.get('measurements',[])
    need(isinstance(measurements,list),'fresh measurements must be a list')
    for suite in measurements:
        name=suite.get('selection'); need(name in expected,f'unknown P035 current selection {name}')
        need(name not in current,f'duplicate current portfolio selection {name}')
        need(suite.get('instrumented') is False and suite.get('mode')=='decoded','instrumented/non-decoded timing is not eligible')
        report=json_ref(suite.get('report'),f'{name} native comparison report')
        analysis=json_ref(suite.get('analysis'),f'{name} native paired analysis')
        need(report.get('selection')==name and report.get('metadata',{}).get('settings',{}).get('instrumented') is False and report['metadata']['settings'].get('candidate_mode')=='decoded','report mode/selection does not match P035')
        need(report['metadata']['settings'].get('baseline_mode')=='decoded','baseline mode is not decoded')
        need(suite.get('baseline_kind')=='current-P034-predecessor','current portfolio must be measured against immediate P034 predecessor')
        need(suite.get('source_map_sha256')==data.get('current_source_map_sha256'),'measurement source map differs from current selected source')
        current_sources=data.get('current_runtime_sources',{})
        report_sources=report.get('metadata',{}).get('candidate_sources',{})
        need(all(report_sources.get(rel)==sha for rel,sha in current_sources.items()),'measurement report candidate source map differs from current runtime')
        binary=verify_ref(suite.get('candidate_binary'),f'{name} candidate binary')
        need(digest(binary)==report['metadata'].get('candidate_binary_sha256'),'candidate binary hash differs from report')
        groups,pairs,seeds,analysis_groups=report_pairs(report,name,expected[name])
        fixture_rows=validate_fixture_map(report,registry,name,expected[name])
        if name=='representative':
            for pair in pairs.values():
                fixture=fixture_rows[pair['workload']]
                need(pair.get('wat_sha256')==fixture.get('wat_sha256') and
                     pair.get('wasm_sha256')==fixture.get('wasm_sha256') and
                     pair.get('stage')=='run_after_setup',
                     'representative pair fixture hash or setup-excluded stage differs')
        validate_journal(suite.get('journal'),f'{name} raw journal',report,pairs,seeds)
        validate_analysis(suite.get('analysis'),name,analysis,analysis_groups)
        need(suite.get('p001_workloads',[])==[],'P001-comparable IDs belong in the separate original-mvp comparison')
        current[name]=len(expected[name])
    need(set(current)==set(expected),'fresh current portfolio must cover scalar, memory, and representative selections')
    old=data.get('p001_legacy',{})
    p001_ids=set(old.get('compatible_workloads',[])); validate_p001_ids(p001_ids)
    p001_manifest=json_ref(old.get('manifest'),'frozen P001 archive manifest')
    adapter_manifest=json_ref(old.get('adapter_manifest'),'P001 legacy adapter manifest')
    need(adapter_manifest.get('kind')=='legacy-api-adapter' and adapter_manifest.get('baseline_manifest_sha256')==digest(old['manifest']['path']),'P001 adapter is not bound to frozen P001 archive')
    adapter_binary=old.get('adapter_binary'); verify_ref(adapter_binary,'P001 legacy adapter binary')
    need(digest(adapter_binary['path'])==adapter_manifest.get('files',{}).get(adapter_manifest.get('binary'),{}).get('sha256'),'P001 adapter binary hash mismatch')
    p001_binary=old.get('binary'); verify_ref(p001_binary,'frozen P001 binary')
    p001_bin_expected=p001_manifest.get('files',{}).get('binary/wtime-bench',{}).get('sha256')
    need(digest(p001_binary['path'])==p001_bin_expected,'P001 legacy binary differs from frozen P001 manifest')
    compatible=data.get('p001_comparison',{})
    report=json_ref(compatible.get('report'),'P001 original-mvp comparison report')
    analysis=json_ref(compatible.get('analysis'),'P001 original-mvp analysis')
    need(report.get('selection')=='original-mvp' and report.get('metadata',{}).get('baseline_archive_kind')=='legacy-api-adapter','P001 report did not use the legacy adapter')
    need(report.get('metadata',{}).get('baseline_manifest_sha256')==digest(old['adapter_manifest']['path']) and report.get('metadata',{}).get('baseline_binary_sha256')==digest(adapter_binary['path']),'P001 report is not bound to the adapter archive/binary')
    need(report.get('metadata',{}).get('settings',{}).get('baseline_mode')=='decoded' and
         report.get('metadata',{}).get('settings',{}).get('instrumented') is False,
         'P001 comparison mode/instrumentation mismatch')
    p001_groups,p001_pairs,p001_seeds,p001_analysis_groups=report_pairs(report,'P001 original-mvp',LEGACY)
    need(set(report['metadata'].get('legacy_control_ids',[]))==LEGACY or
         set(report['metadata'].get('settings',{}).get('scalar_warm_calibration',{}).get('legacy_control_ids',[]))==LEGACY,
         'P001 report must independently declare exactly the three compatible legacy controls')
    validate_journal(compatible.get('journal'),'P001 original-mvp journal',report,p001_pairs,p001_seeds)
    validate_analysis(compatible.get('analysis'),'P001 original-mvp',analysis,p001_analysis_groups)
    fixture_map={row.get('id'):row.get('sha256') for row in report.get('metadata',{}).get('fixtures',[])}
    for workload in LEGACY:
        archived=p001_manifest.get('files',{}).get(f'fixtures/{workload}.wasm',{}).get('sha256')
        need(archived and fixture_map.get(workload)==archived,'P001 legacy comparison fixture identity mismatch: '+workload)
    return {'current_rows':sum(current.values()),'current_selections':current,'p001_compatible_ids':sorted(p001_ids)}

def validate_build_and_dependencies(data,current_sources):
    budgets=data.get('budgets',{})
    build=json_ref(budgets.get('build_report'),'default build budget report')
    need(build.get('schema_version')==2 and build.get('mode')=='default' and build.get('features')==[],
         'build budget is not a default, no-feature schema-v2 run')
    need(build.get('source_stable') is True and build.get('budget_pass') is True,
         'default build budget failed or sources changed')
    before=build.get('source_inputs'); after=build.get('source_inputs_after')
    need(isinstance(before,dict) and before==after,'build source-input map is missing or changed')
    need(all(before.get(path)==sha for path,sha in current_sources.items()),
         'build inputs do not bind all current runtime source files')
    limits=build.get('budgets',{})
    need(set(limits)=={'clean_seconds','leaf_seconds','stripped_bytes'},
         'build report hard-limit schema changed')
    for metric,limit in limits.items():
        value=build.get(metric)
        need(type(value) in (int,float) and math.isfinite(value) and value>=0,
             f'build metric {metric} is absent/invalid')
        need(value<=limit,f'build metric {metric} exceeds its declared hard limit')
    need(build.get('default_dependency_closure_pass') is True and
         build.get('default_dependency_violations')==[],
         'default dependency closure is not clean')
    dep=verify_ref(budgets.get('dependency_report'),'dependency budget transcript')
    dep_text=dep.read_text(errors='replace')
    need('Production dependency/unsafe budget: 0 violations' in dep_text,
         'dependency budget transcript does not prove zero violations')
    need(budgets.get('dependency_exit_code')==0,'dependency budget command did not exit successfully')
    return {'build':budgets['build_report'],'dependency':budgets['dependency_report']}

def validate_conformance(data,current_source_map):
    conformance=data.get('conformance',{})
    need(conformance.get('exit_code')==0 and conformance.get('tier')=='decoded' and
         conformance.get('selection')=='integration','decoded integration conformance did not pass')
    report=json_ref(conformance.get('report'),'decoded integration conformance report')
    need(report.get('schema_version')==1 and report.get('passed') is True and
         report.get('source_stable') is True and report.get('requested_tier')=='decoded' and
         report.get('baseline_tier')=='reference' and report.get('selection')=='integration',
         'integration conformance native report failed or has wrong mode')
    need(report.get('instrumented') is True and
         report.get('diagnostic_instrumentation',{}).get('enabled') is True and
         report.get('diagnostic_instrumentation',{}).get('feature')=='performance-counters',
         'integration report lacks its declared correctness-only counter instrumentation')
    counts=report.get('counts',{})
    expected={'selected_cases':3,'semantic_cases':3,'mode_runs_expected':6,'mode_runs_executed':6,
              'state_cases':3,'state_mode_runs_expected':6,'state_mode_runs_executed':6,
              'state_fuel_probes_expected':68,'state_fuel_probes_executed':68,
              'conformance_suites':2,'conformance_mode_runs_expected':4,'conformance_mode_runs_executed':4,
              'failed':0,'skipped':0,'unsupported':0}
    need(all(counts.get(k)==v for k,v in expected.items()),'integration conformance counts are incomplete')
    need(report.get('errors')==[],'integration conformance report contains errors')
    sources=report.get('implementation_sources',{})
    need(isinstance(sources,dict) and
         all(sources.get(path)==sha for path,sha in current_source_map.items()) and
         report.get('implementation_sources_sha256')==canonical_sha(sources),
         'integration conformance implementation source identity differs')
    b=report.get('binary',{}); cb=report.get('conformance_binary',{})
    verify_ref(conformance.get('binary'),'conformance benchmark binary')
    verify_ref(conformance.get('conformance_binary'),'instrumented conformance binary')
    need(digest(conformance['binary']['path'])==b.get('sha256') and
         digest(conformance['conformance_binary']['path'])==cb.get('sha256'),
         'conformance binary identities differ from native report')
    need(report.get('fixture_identities_before')==report.get('fixture_identities_after'),
         'conformance fixture identities changed during run')
    return conformance['report']

def validate_profiles(rows,registry,current_source_map,source_map_sha):
    need(isinstance(rows,list) and rows,'diagnostic profile inventory missing')
    expected_by_selection={name:set(registry['selections'][name]['workloads']) for name in SELECTIONS}
    supported_by_selection={
        'scalar':LEGACY,
        'memory':{'memory_fill_scan'},
        'representative':expected_by_selection['representative'],
    }
    for selection,expected_ids in expected_by_selection.items():
        row=next((item for item in rows if item.get('selection')==selection),None)
        need(row is not None,f'profile inventory missing {selection}')
        need(row.get('source_map_sha256')==source_map_sha,'profile source map differs from P035 source')
        need(row.get('status')=='passed' and row.get('instrumented') is True and
             row.get('role')=='diagnostic_profile',f'{selection} diagnostic report is not a successful instrumented profile run')
        report=json_ref(row.get('report'),f'{selection} diagnostic profile report')
        need(report.get('schema_version')==1 and report.get('passed') is True and
             report.get('selection')==selection and report.get('production_timing') is False and
             report.get('evidence_kind')=='bounded_diagnostic_attribution',
             f'{selection} native profile report failed or has wrong evidence kind')
        source_inputs=report.get('metadata',{}).get('source_inputs',{})
        need(isinstance(source_inputs,dict) and all(source_inputs.get(path)==sha for path,sha in current_source_map.items()),
             f'{selection} profile does not bind all current runtime sources')
        cases=report.get('cases',[])
        by_case={item.get('workload'):item for item in cases if isinstance(item,dict)}
        need(len(by_case)==len(cases) and set(by_case)==expected_ids,
             f'{selection} profile case inventory differs from selected workload IDs')
        actual_supported={wid for wid,item in by_case.items() if item.get('counter_status')=='passed'}
        actual_unsupported={wid for wid,item in by_case.items() if item.get('counter_status')=='unsupported'}
        need(actual_supported==supported_by_selection[selection],
             f'{selection} actual supported counter cases differ from predeclared diagnostic routes')
        need(actual_supported|actual_unsupported==expected_ids,
             f'{selection} profile has failed, unknown, or unclassified cases')
        for wid in actual_unsupported:
            item=by_case[wid]
            need(isinstance(item.get('reason'),str) and item['reason'].strip(),
                 f'{selection}/{wid} unsupported profile has no recorded reason')
        count_rows=report.get('counters',[])
        counters={item.get('workload'):item for item in count_rows if isinstance(item,dict)}
        need(set(counters)==actual_supported and len(counters)==len(count_rows),
             f'{selection} counter observation records do not match supported IDs')
        entries={item.get('id'):item for item in registry.get('workloads',[]) if isinstance(item,dict)}
        for wid in actual_supported:
            observation=counters[wid]
            need(observation.get('instrumented') is True and observation.get('role')=='diagnostic_profile' and
                 observation.get('binary_sha256')==report.get('metadata',{}).get('artifacts',{}).get('counters',{}).get('binary_sha256'),
                 f'{selection}/{wid} counter observation is not bound to the diagnostic binary')
            raw=observation.get('raw_observation',{})
            need(raw.get('diagnostic') is True and raw.get('error') is None,
                 f'{selection}/{wid} lacks an actual validated counter observation')
            src=entries[wid].get('source',{})
            need(observation.get('fixture_sha256')==src.get('wasm_sha256') and
                 observation.get('wat_sha256')==src.get('sha256'),
                 f'{selection}/{wid} diagnostic fixture hashes differ from workload registry')
            artifact=observation.get('artifact')
            artifact_path=Path(row['report']['path']).parent/artifact
            need(isinstance(artifact,str) and artifact_path.is_file() and
                 digest(artifact_path)==observation.get('artifact_sha256') and
                 artifact_path.resolve().is_relative_to(Path(row['report']['path']).parent.resolve()),
                 f'{selection}/{wid} counter artifact is missing or stale')
        cpu_rows={item.get('workload'):item for item in report.get('cpu_profiles',[]) if isinstance(item,dict)}
        need(len(cpu_rows)==len(report.get('cpu_profiles',[])) and set(cpu_rows)==expected_ids,
             f'{selection} CPU-profile capability inventory differs from selected IDs')
        for wid,item in cpu_rows.items():
            need(type(item.get('available')) is bool,f'{selection}/{wid} CPU-profile availability is not explicit')
            if item['available']:
                for path_key,sha_key in (('data','data_sha256'),('report','report_sha256')):
                    artifact_path=Path(row['report']['path']).parent/item.get(path_key,'')
                    need(artifact_path.is_file() and digest(artifact_path)==item.get(sha_key) and
                         artifact_path.resolve().is_relative_to(Path(row['report']['path']).parent.resolve()),
                         f'{selection}/{wid} available CPU-profile artifact is missing/stale')
            elif item.get('attempted') is True:
                need(item.get('capability_gap') or item.get('exit_code')!=0 or item.get('report_exit_code')!=0,
                     f'{selection}/{wid} failed CPU profiling is being represented without a reason')
        counts=report.get('counts',{})
        need(counts.get('selected')==len(expected_ids) and
             counts.get('actual_counter_cases')==len(actual_supported) and
             counts.get('explicit_unsupported_cases')==len(actual_unsupported),
             f'{selection} profile counts do not reconcile with case records')
    need(len(rows)==len(SELECTIONS) and {r.get('selection') for r in rows}==set(SELECTIONS),
         'diagnostic profile inventory contains duplicate/extra selection rows')

def validate_p001_ids(ids):
    need(set(ids)==LEGACY,'P001 comparison must cover exactly the three compatible legacy workload IDs')

def validate(data):
    need(data.get('schema')==SCHEMA and data.get('task')=='P035','P035 input schema/task mismatch')
    for key in ('task_plan','task_index','workload_registry'):
        verify_ref(data.get(key),key)
    tasks=validate_tasks(data)
    p017=validate_p017(data)
    rollback=validate_closure(data)
    current_source_map=runtime_map(data.get('current_runtime_sources'),'current runtime source map')
    need(data.get('current_source_map_sha256')==canonical_sha(current_source_map),'current runtime source map digest mismatch')
    source=validate_source_chain(data,rollback)
    portfolio=validate_portfolio(data)
    budget_refs=validate_build_and_dependencies(data,current_source_map)
    conformance_ref=validate_conformance(data,current_source_map)
    validate_profiles(data.get('profiles'),json_ref(data.get('workload_registry'),'workload registry'),
                      current_source_map,data.get('current_source_map_sha256'))
    need(data.get('aspiration_rule')=='report_per_case_no_invented_aggregate','P035 must not invent an aggregate 10x decision rule')
    return {'task':'P035','status':'passed','selected_baseline':'P017','source_chain':source,'portfolio':portfolio,
            'task_outcomes':{k:{'status':v['status'],'outcome':v['outcome'],'promoted':v.get('promoted')} for k,v in tasks.items() if k in FOUNDATIONS|CANDIDATES|{'P017'}},
            'p017_proof':p017,'budgets':budget_refs,'conformance_report':conformance_ref,
            'decision_limit':'10x aspiration reported per compatible legacy case; no fabricated aggregate or P001 comparison for current-only rows'}

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--input',type=Path,required=True); p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    try:
        data=json.loads(args.input.read_text(encoding='utf-8')); result=validate(data)
        result['input']={'path':str(args.input.resolve()),'sha256':digest(args.input)}
        payload=json.dumps(result,indent=2,sort_keys=True,allow_nan=False)+'\n'
        with args.output.open('x',encoding='utf-8') as stream: stream.write(payload)
        print(json.dumps({'task':'P035','status':'passed','output':str(args.output)},sort_keys=True)); return 0
    except (CheckError,OSError,json.JSONDecodeError) as e:
        print(f'P035 check failed closed: {e}',file=sys.stderr); return 2
if __name__=='__main__': raise SystemExit(main())
