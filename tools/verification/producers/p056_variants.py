"""Refresh isolated P047 and P050 comparisons with the admitted timer/oracles."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path.insert(0, str(BENCH))
import baseline, compare, focused_compare as focused, p123_adapter
import selection_manifest, semantic_receipt, cold_products
P047_SEL = ROOT/'docs/benchmarks/performance/P056-P047-refresh-selection.json'
P050_SEL = ROOT/'docs/benchmarks/performance/P050-selection.json'
P047_ARCH = ROOT/'target/performance/P047-before'
P050_ARCH = ROOT/'target/performance/P050-before'
P055_ARCH = ROOT/'target/performance/P055-before'
P047_RECEIPT = ROOT/'target/verification/runs/P047/verify/attempt-0001/receipt.json'
P047_BUILD = P047_RECEIPT.parent/'production/build-record.json'
P050_RECEIPT = ROOT/'target/verification/runs/P050/verify/attempt-0002/receipt.json'
P050_BUILD = P050_RECEIPT.parent/'production/build-record.json'
SCHEMA = 'wtime-p056-variant-refresh-v1'


def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def ref(path):
    p=Path(path).resolve(strict=True); return {'path':str(p),'sha256':sha(p),'bytes':p.stat().st_size}
def _core(core): return {p.removeprefix('source/'):h for p,h in core.items()}
def compose_sources(core, harness):
    out=_core(core)
    for p,h in harness.items():
        if p in out and out[p]!=h: raise ValueError('source identity conflict: '+p)
        out[p]=h
    if not out or any(not isinstance(h,str) or len(h)!=64 for h in out.values()):
        raise ValueError('malformed variant source map')
    return dict(sorted(out.items()))

def verify_core_match(actual, frozen_core):
    expected=_core(frozen_core); core_names=('wtime-runtime','wtime-types','wtime-decode','wtime-validate')
    found={p:h for p,h in actual.items() if any(p.startswith('crates/'+name+'/') for name in core_names)}
    if found!=expected: raise ValueError('semantic receipt core differs from frozen variant')
    return {'files':len(expected),'sha256':hashlib.sha256(json.dumps(expected,sort_keys=True,separators=(',',':')).encode()).hexdigest()}

def _locate(name, locations):
    rows=[r for r in locations if name.startswith(r['prefix'])]
    if not rows: raise ValueError('source path has no physical source root: '+name)
    row=max(rows,key=lambda r:len(r['prefix']))
    return Path(row['path'])/name[len(row['prefix']):]

def validate_source_locations(record):
    files=record.get('files'); locations=record.get('locations')
    if not isinstance(files,dict) or not files or not isinstance(locations,list):
        raise ValueError('source map lacks files or locations')
    for name,digest in files.items():
        path=_locate(name,locations)
        if not path.is_file() or sha(path)!=digest: raise ValueError('source hash changed: '+name)
    return True

def expected_result(entry):
    if entry['id']=='prepare_large': return None
    module=entry.get('oracle',{}).get('module')
    n=focused.input_value(entry)
    if module=='scalar_workloads.py':
        import scalar_workloads
        return scalar_workloads.oracle(entry['id'],n)
    if module=='run.py':
        import run as legacy_run
        return legacy_run.oracle(entry['id'],n)
    if module=='scalar_p128_workloads.py':
        return focused._p128_scalar_row(entry,n,'load_and_call','compact',1)[1]
    raise ValueError('unsupported P056 oracle module: '+str(module))

def validate_observation_identity(observation, workload, stage, expected):
    # The legacy preparation-only protocol has a numeric wire placeholder,
    # while its independent result oracle is null: no guest call is timed.
    wire_expected = 0 if workload == 'prepare_large' and stage == 'prepare' and expected is None else expected
    if (observation.get('result') != expected or observation.get('expected') != wire_expected
            or observation.get('stage') != stage):
        raise ValueError('pair observation result or stage differs from expected oracle')
    return True

def validate_pair_groups(runs, plan, pair_count=30, entries=None, warm_batches=None):
    import math
    expected={(g['sample_workload'],g['stage']):g for g in plan['groups']}
    if len(runs)!=2: raise ValueError('exactly two independent pair runs are required')
    by_id={e['id']:e for e in entries or []}
    warm_batches=warm_batches or {}
    for run_id,run in enumerate(runs):
        if run.get('run_id')!=run_id: raise ValueError('pair run ids must be ordered 0, 1')
        counts={}; seen_ids=set()
        for pair in run.get('pairs',[]):
            key=(pair.get('workload'),pair.get('stage')); counts[key]=counts.get(key,0)+1
            if key not in expected: raise ValueError('pair does not belong to planned group')
            pair_id=pair.get('pair_id')
            if not isinstance(pair_id,str) or pair_id in seen_ids: raise ValueError('pair ids must be unique within each run')
            seen_ids.add(pair_id)
            group=expected[key]
            if pair.get('product_sha256')!=group.get('product_sha256'): raise ValueError('pair product identity mismatch')
            if pair.get('expected')!=pair.get('baseline',{}).get('result') or pair.get('expected')!=pair.get('candidate',{}).get('result'):
                raise ValueError('paired oracle result mismatch')
            if pair.get('order') not in (['baseline','candidate'],['candidate','baseline']): raise ValueError('invalid pair order')
            for side in ('baseline','candidate'):
                obs=pair.get(side,{})
                ns=obs.get('ns_per_operation'); batch=obs.get('batch')
                if isinstance(ns,bool) or not isinstance(ns,(int,float)) or not math.isfinite(ns) or ns<=0 or type(batch) is not int or batch<1:
                    raise ValueError('pair observation lacks valid timing identity')
                validate_observation_identity(obs, key[0], key[1], pair.get('expected'))
                required_batch=warm_batches.get((run_id,key[0],key[1]),1)
                if batch!=required_batch: raise ValueError('pair batch differs from calibrated or cold batch')
            if entries is not None:
                entry=by_id.get(key[0])
                if entry is None or pair.get('input')!=focused.input_value(entry) or pair.get('wasm_sha256')!=entry['source']['wasm_sha256']:
                    raise ValueError('pair workload identity differs from frozen selection')
                if pair.get('expected')!=expected_result(entry): raise ValueError('pair result differs from independent workload oracle')
        if set(counts)!=set(expected) or any(n!=pair_count for n in counts.values()):
            raise ValueError('each planned group requires 30 pairs in each run')
    return True

def _validate_warm(report, plan):
    required={(r,g['sample_workload'],'warm_call') for r in (0,1) for g in plan['groups'] if g['stage']=='warm_call'}
    found={}; batches={}
    for row in report.get('warm_calibrations',[]):
        key=(row.get('run_id'),row.get('workload'),row.get('stage'))
        if key in found: raise ValueError('duplicate warm calibration record')
        found[key]=row
        if (row.get('target_ns')!=20_000_000 or type(row.get('batch')) is not int
                or not 1<=row['batch']<=compare.SCALAR_MAX_BATCH):
            raise ValueError('invalid warm calibration identity')
        attempts=row.get('attempts',[])
        if not attempts: raise ValueError('warm calibration attempts missing')
        for attempt in attempts:
            if (type(attempt.get('batch')) is not int
                    or not 1<=attempt['batch']<=compare.SCALAR_MAX_BATCH):
                raise ValueError('invalid calibration attempt batch')
            for side in ('baseline','candidate'):
                obs=attempt.get('observations',{}).get(side,{})
                ns=obs.get('ns_per_operation')
                if (obs.get('batch')!=attempt['batch'] or isinstance(ns,bool) or not isinstance(ns,(int,float))
                        or not __import__('math').isfinite(ns) or ns<=0):
                    raise ValueError('invalid calibration timing observation')
        last=attempts[-1]
        if last.get('batch')!=row['batch'] or any(last['observations'][s]['batch']*last['observations'][s]['ns_per_operation']<20_000_000 for s in ('baseline','candidate')):
            raise ValueError('warm calibration did not reach 20 ms on both sides')
        batches[key]=row['batch']
    if set(found)!=required: raise ValueError('warm calibration coverage differs from warm products')
    return batches

def validate_raw_report(report, report_path, *, check_sources=True):
    """Pure validation used by the gate and statistics router; it never runs timers."""
    path=Path(report_path).resolve(strict=True)
    disk_doc=json.loads(path.read_text())
    if isinstance(report,(str,Path)): doc=json.loads(Path(report).read_text())
    else: doc=report
    if doc!=disk_doc: raise ValueError('raw report object differs from its immutable report file')
    if not isinstance(doc,dict) or doc.get('schema')!=SCHEMA or doc.get('schema_version')!=2:
        raise ValueError('unsupported P056 refresh raw report')
    task=doc.get('task')
    if (task not in ('P047','P050') or doc.get('selection')!=task or doc.get('status')!='passed'
            or doc.get('passed') is not True or doc.get('source_stable') is not True
            or doc.get('instrumented') is not False): raise ValueError('P056 refresh report is incomplete')
    selection_ref=doc.get('selection_ref',{}); selpath=Path(selection_ref.get('path','')).resolve(strict=True)
    if ref(selpath)!=selection_ref or json.loads(selpath.read_text())!=doc.get('declaration'):
        raise ValueError('refreshed frozen selection changed')
    declaration=selection_manifest.load(selpath)
    if declaration.get('task')!=task: raise ValueError('refresh selection task mismatch')
    entries=selection_manifest.rows(declaration)
    selection_manifest.load_data(declaration)
    settings=doc.get('metadata',{}).get('settings',{})
    products=focused.products(entries,declaration,settings)
    plan=cold_products.verify(doc.get('cold_products'),products)
    warm_batches=_validate_warm(doc,plan)
    validate_pair_groups(doc.get('runs',[]),plan,entries=entries,warm_batches=warm_batches)
    # Reuse the analyzer's schema-v2 structural validator only; this computes no statistics.
    import analyze
    analyze._validated_runs(doc)
    probes=doc.get('correctness_probes',[])
    expected_probes={(entry['id'],side) for entry in entries for side in ('baseline','candidate')}
    found_probes=set()
    for probe in probes:
        entry=next((item for item in entries if item['id']==probe.get('workload')),None)
        if entry is None or probe.get('side') not in ('baseline','candidate') or probe.get('timing_excluded') is not True:
            raise ValueError('invalid correctness probe identity')
        stage='prepare' if entry['id']=='prepare_large' else 'load_and_call'
        observation=probe.get('observation',{})
        if probe.get('stage')!=stage:
            raise ValueError('correctness probe differs from selected oracle')
        validate_observation_identity(observation,entry['id'],stage,expected_result(entry))
        found_probes.add((entry['id'],probe['side']))
    if found_probes!=expected_probes: raise ValueError('correctness probe coverage differs from selection')
    if doc['counts']!=cold_products.summarize(plan,{f"{g['sample_workload']}\0{g['stage']}":60 for g in plan['groups']}):
        raise ValueError('physical/logical pair counts differ from cold product plan')
    maps=doc.get('source_maps',{})
    if doc.get('metadata',{}).get('candidate_sources')!=maps.get('candidate',{}).get('files'):
        raise ValueError('candidate source map alias differs')
    if settings.get('instrumented') is not False or settings.get('baseline_mode')!='compact' or settings.get('candidate_mode')!='compact':
        raise ValueError('raw settings must bind uninstrumented Compact on both sides')
    import math
    for run in doc.get('runs',[]):
        for pair in run.get('pairs',[]):
            for side in ('baseline','candidate'):
                obs=pair.get(side,{})
                if obs.get('mode')!='compact': raise ValueError(side+' timing mode differs from Compact metadata')
                value=obs.get('ns_per_operation')
                if isinstance(value,bool) or not isinstance(value,(int,float)) or not math.isfinite(value):
                    raise ValueError('non-finite pair timing')
    if check_sources:
        sem=doc.get('semantic_provenance',{})
        def check_semantic_refs(value):
            if isinstance(value,dict):
                if set(value)=={'path','sha256','bytes'} and isinstance(value.get('path'),str):
                    if ref(value['path'])!=value: raise ValueError('semantic provenance reference changed')
                else:
                    for nested in value.values(): check_semantic_refs(nested)
            elif isinstance(value,list):
                for nested in value: check_semantic_refs(nested)
        check_semantic_refs(sem)
        for side in ('baseline','candidate','current_measurement_inputs'):
            validate_source_locations(maps[side])
        helper_ref=doc.get('metadata',{}).get('helper_archive',{})
        if ref(helper_ref.get('path',''))!=helper_ref: raise ValueError('executed helper archive reference changed')
        helper_doc=json.loads(Path(helper_ref['path']).read_text())
        for source_ref in helper_doc.get('files',{}).values():
            if ref(source_ref.get('path',''))!=source_ref: raise ValueError('archived helper source changed')
        for side in ('baseline','candidate'):
            row=doc['counterpart'][side]
            for field in ('binary','manifest','source_archive_manifest'):
                r=row[field]
                if ref(r['path'])!=r: raise ValueError(f'{side} {field} changed or malformed')
            manifest=json.loads(Path(row['manifest']['path']).read_text())
            if row['binary']['sha256']!=manifest.get('binary_sha256'):
                raise ValueError(f'{side} binary differs from adapter/build manifest')
            if doc['metadata'].get(side+'_binary_sha256')!=row['binary']['sha256']:
                raise ValueError(f'{side} binary metadata alias differs')
            if manifest.get('kind')=='frozen-typed-adapter':
                archive=Path(row['source_archive_manifest']['path']).parent
                p123_adapter.verify(Path(row['manifest']['path']).parent,archive)
                expected=compose_sources(p123_adapter.core_identities(baseline.verify_archive(archive)),p123_adapter.compiled_harness_identities())
                if maps[side]['files']!=expected:
                    raise ValueError(side+' source map does not bind its frozen archive and compiled harness')
            elif side=='candidate' and task=='P050':
                if manifest.get('source_stable') is not True or manifest.get('phase')!='build' or manifest.get('exit_code')!=0 or 'compact-executor' not in manifest.get('feature_flags',[]):
                    raise ValueError('current candidate build record is incomplete')
                if maps[side]['files']!=manifest.get('source_identities'):
                    raise ValueError('P050 source map differs from current build identities')
        if task=='P047':
            if maps['candidate'].get('core_source_kind')!='frozen_archive':
                raise ValueError('P047 fuse-only source map must remain archive-bound')
            historical=_historical_semantics('P047',declaration,baseline.verify_archive(P050_ARCH),P047_RECEIPT,P047_BUILD)
            for key,value in historical.items():
                if sem.get(key)!=value: raise ValueError('P047 historical semantic receipt proof differs')
            if sem.get('historical_exact_core_match') is not True:
                raise ValueError('P047 semantic receipt does not certify exact fuse-only core')
        else:
            if maps['candidate'].get('core_source_kind')!='current_production':
                raise ValueError('P050 candidate source map must bind current production')
            current_receipt=sem.get('receipt',{}).get('path')
            current_build=doc['counterpart']['candidate']['manifest']['path']
            if sem.get('build_record')!=doc['counterpart']['candidate']['manifest']:
                raise ValueError('P050 semantic proof does not bind current production build')
            checked=semantic_receipt.validate(current_receipt,{'task':'P056','semantic_cases':declaration['semantic_cases']},maps['candidate']['files'],ROOT)
            if sem.get('status')!=checked['status'] or sem.get('required_cases')!=checked['required_cases']:
                raise ValueError('P050 current semantic receipt proof differs')
            historical=_historical_semantics('P050',declaration,baseline.verify_archive(P055_ARCH),P050_RECEIPT,P050_BUILD)
            if sem.get('historical_p050')!=historical:
                raise ValueError('P050 historical semantic receipt proof differs')
        if doc['metadata'].get('protocol_sha256')!=compare.sha(BENCH/'src/protocol.rs'):
            raise ValueError('measurement protocol helper changed')
        from p046_verify import P001_BINARY,P001_MANIFEST
        p001=ROOT/'target/performance/baseline'; p001_manifest=baseline.verify_archive(p001)
        if sha(p001/'manifest.json')!=P001_MANIFEST or sha(p001/'binary/wtime-bench')!=P001_BINARY:
            raise ValueError('P001 immutable archive changed')
        encoder=p001/'binary/wtime-conformance'
        if doc['metadata'].get('fixture_encoder')!=ref(encoder):
            raise ValueError('fixture encoder does not bind immutable P001 bytes')
        fixture_files=doc['metadata'].get('fixture_files',{})
        if set(fixture_files)!={entry['id'] for entry in entries}:
            raise ValueError('prepared fixture coverage differs from selection')
        for entry in entries:
            fixture_ref=fixture_files[entry['id']]
            if ref(fixture_ref.get('path',''))!=fixture_ref or fixture_ref['sha256']!=entry['source']['wasm_sha256']:
                raise ValueError('prepared fixture bytes differ from selected Wasm: '+entry['id'])
        if maps['candidate']['files']!=doc['metadata'].get('candidate_sources'):
            raise ValueError('candidate source map differs from raw metadata')
    return {'task':task,'selection':task,'report_sha256':sha(path),'source_stable':True,'report':doc}

def validate_report(report_path, *, check_sources=True):
    """Compatibility wrapper for P056 workflow; returns the raw report."""
    return validate_raw_report(report_path,report_path,check_sources=check_sources)['report']

def _selection(path,task):
    d=selection_manifest.load(path)
    if d['task']!=task: raise ValueError('selection task mismatch')
    return d,selection_manifest.rows(d)

def _snapshot():
    m=focused.sources()
    for p in (P047_SEL,P050_SEL,Path(__file__).resolve()): m[str(p.relative_to(ROOT))]=sha(p)
    return m

def _historical_semantics(task, selection, frozen, receipt_path, build_path):
    receipt=json.loads(receipt_path.read_text()); build=json.loads(build_path.read_text())
    identities=build.get('source_identities')
    if receipt.get('status')!='passed' or receipt.get('lane')!='full' or not identities: raise ValueError('full semantic receipt/build record required')
    match=verify_core_match(identities,p123_adapter.core_identities(frozen))
    checked=semantic_receipt.validate(receipt_path,selection,identities,ROOT)
    return {'receipt':ref(receipt_path),'build_record':ref(build_path),'status':checked['status'],
            'required_cases':checked['required_cases'],'exact_core_match':match}

def _side_map(files, core_kind, core_root, harness_root=None):
    roots=[{'prefix':'crates/','path':str(core_root/'source/crates')}]
    if harness_root: roots.append({'prefix':'tools/','path':str(harness_root / 'source/tools')})
    else: roots=[{'prefix':'','path':str(ROOT)}]
    return {'files':files,'locations':roots,'core_source_kind':core_kind}

def _report(task,declaration,entries,left,right,output,semantic,seed,cpu,inputs,encoder):
    be,bmap,brow=left; ce,cmap,crow=right
    caps={s:compare.process(e,['--protocol'],cpu) for s,e in (('baseline',be),('candidate',ce))}
    profiles={e['id']:focused.profile(e) for e in entries}
    for side,cap in caps.items():
        if cap.get('instrumented') is not False or 'compact' not in cap.get('modes',[]): raise ValueError(side+' is not uninstrumented Compact')
        if not {focused.profile(e) for e in entries}<=set(cap.get('features',[])): raise ValueError(side+' lacks workload profile')
    settings={'instrumented':False,'profile':'release','fuel_policy':False,'cancellation_policy':1,
        'baseline_mode':'compact','candidate_mode':'compact','workload_profiles':profiles,'warm_target_ns':20_000_000}
    fixtures=output/'fixtures'; fixtures.mkdir()
    archive=P047_ARCH if task=='P047' else P050_ARCH
    fpaths=focused.prepare_fixtures(entries,archive,fixtures,encoder)
    fixture_refs={k:ref(v['path'] if isinstance(v,dict) else v) for k,v in fpaths.items()}
    plan=cold_products.plan(focused.products(entries,declaration,settings)); byid={e['id']:e for e in entries}
    expected_by_id={entry['id']:expected_result(entry) for entry in entries}
    def call(side,wid,stage,batch):
        e=be if side=='baseline' else ce; entry=byid[wid]; fp=fpaths[wid]['path'] if isinstance(fpaths[wid],dict) else fpaths[wid]
        observation=focused.observe(e,entry,fp,stage,batch,'compact',cpu,output)
        # Exercise the report contract during the correctness probes, before
        # collecting either paired run, and retain the actual validated wire.
        validate_observation_identity(observation,wid,stage,expected_by_id[wid])
        return observation
    report={'schema':SCHEMA,'schema_version':2,'task':task,'selection':task,'status':'running','passed':False,
        'source_stable':False,'instrumented':False,'declaration':declaration,'selection_ref':ref(P047_SEL if task=='P047' else P050_SEL),
        'semantic_provenance':semantic,'counterpart':{'baseline':brow,'candidate':crow},
        'source_maps':{'baseline':bmap,'candidate':cmap,'current_measurement_inputs':inputs},'cold_products':plan,
        'metadata':{'settings':settings,'capabilities':caps,'candidate_sources':cmap['files'],
          'baseline_binary_sha256':sha(be),'candidate_binary_sha256':sha(ce),
          'protocol_sha256':compare.sha(BENCH/'src/protocol.rs'),'fixture_files':fixture_refs,
          'fixture_encoder':ref(encoder),'helper_archive':ref(output.parent/'helper-sources.json'),'selected_cpu':cpu},
        'correctness_probes':[],'warm_calibrations':[],'runs':[]}
    for entry in entries:
        stage='prepare' if entry['id']=='prepare_large' else 'load_and_call'
        for side in ('baseline','candidate'):
            report['correctness_probes'].append({'workload':entry['id'],'stage':stage,'side':side,
                'observation':call(side,entry['id'],stage,1),'timing_excluded':True})
            (output/'partial-report.json').write_text(json.dumps(report,indent=2)+'\n')
    for run_id in (0,1):
        run={'run_id':run_id,'seed':seed+run_id*1_000_003,'pairs':[]}; report['runs'].append(run)
        for group in plan['groups']:
            wid,stage=group['sample_workload'],group['stage']; entry=byid[wid]; batch=1
            if stage=='warm_call':
                batch,audit=focused.calibrate(entry,lambda side,n:call(side,wid,stage,n))
                report['warm_calibrations'].append({'run_id':run_id,'workload':wid,'stage':stage,'batch':batch,
                    'target_ns':20_000_000,'attempts':audit})
            orders=compare.pair_order(run['seed']+int(group['product_sha256'][:8],16),30)
            for sample,order in enumerate(orders):
                pair={'pair_id':f'{run_id}:{wid}:{stage}:{sample}','workload':wid,'stage':stage,
                    'input':focused.input_value(entry),'wasm_sha256':entry['source']['wasm_sha256'],
                    'order':order,'product_sha256':group['product_sha256']}
                for side in order:
                    pair[side]=call(side,wid,stage,batch)
                    with (output.parent/'journal.jsonl').open('a') as stream:
                        stream.write(json.dumps({'event':'pair_observation','task':task,'pair_id':pair['pair_id'],
                            'side':side,'observation':pair[side]},sort_keys=True,allow_nan=False)+'\n')
                pair['expected']=expected_by_id[wid]
                run['pairs'].append(pair)
                with (output/'pairs.jsonl').open('a') as stream:
                    stream.write(json.dumps(pair,sort_keys=True,allow_nan=False)+'\n')
                matched=pair['candidate']['result']==pair['expected']
                with (output.parent/'journal.jsonl').open('a') as stream:
                    stream.write(json.dumps({'event':'pair_complete' if matched else 'pair_oracle_mismatch',
                        'task':task,'pair_id':pair['pair_id'],'workload':wid,'stage':stage,'run_id':run_id},sort_keys=True)+'\n')
                if not matched: raise ValueError(f'{task} oracle mismatch {wid}/{stage}')
            (output/'partial-report.json').write_text(json.dumps(report,indent=2)+'\n')
    report['counts']=cold_products.summarize(plan,{f"{g['sample_workload']}\0{g['stage']}":60 for g in plan['groups']})
    report['status']='passed'; report['passed']=report['source_stable']=True
    validate_pair_groups(report['runs'],plan); _validate_warm(report,plan)
    return report

def run(args):
    verify_dir=args.verify_dir.resolve(strict=True); output=args.output.resolve(); target=(ROOT/'target/verification').resolve()
    if not verify_dir.is_relative_to(target) or not output.is_relative_to(target) or output.exists(): raise ValueError('verify/output path must be fresh and contained under target/verification')
    output.mkdir(parents=True); shutil.copyfile(__file__,output/'producer.executed.py')
    (output/'command.json').write_text(json.dumps({'argv':sys.argv,'cwd':str(ROOT)},indent=2)+'\n')
    journal={'schema':'wtime-p056-variant-refresh-journal-v1','status':'running','argv':sys.argv,'actions':[]}
    (output/'journal.json').write_text(json.dumps(journal,indent=2)+'\n')
    (output/'journal.jsonl').write_text(json.dumps({'event':'start','argv':sys.argv,'cwd':str(ROOT)},sort_keys=True)+'\n')
    # Preserve the exact imported Python helpers/oracles before launching binaries.
    helper_root=output/'helper-sources'; helper_root.mkdir()
    helper_files={}
    for name,digest in focused.sources().items():
        if name.startswith('tools/wtime-bench/') and name.endswith('.py'):
            source=(ROOT/name).resolve(strict=True)
            destination=(helper_root/name).resolve()
            if not destination.is_relative_to(helper_root.resolve()): raise ValueError('helper archive path escapes output')
            destination.parent.mkdir(parents=True,exist_ok=True); shutil.copyfile(source,destination)
            if sha(destination)!=digest: raise ValueError('helper source changed while archiving: '+name)
            helper_files[name]=ref(destination)
    (output/'helper-sources.json').write_text(json.dumps({'python':sys.version,'files':helper_files},indent=2)+'\n')
    report_path=None
    try:
        _snapshot_before=_snapshot()
        p047,p047rows=_selection(P047_SEL,'P047'); p050,p050rows=_selection(P050_SEL,'P050')
        a47=baseline.verify_archive(P047_ARCH); a50=baseline.verify_archive(P050_ARCH)
        hmap=p123_adapter.compiled_harness_identities()
        baseline_report=json.loads((verify_dir/'baselines/report.json').read_text())
        adapter_refs=baseline_report['individual_adapters']
        for reference in adapter_refs.values():
            if ref(reference['path'])!=reference: raise ValueError('individual adapter reference changed')
        b47=Path(adapter_refs['p047-base']['path']).parent
        f47=Path(adapter_refs['p047-only']['path']).parent
        r47=p123_adapter.verify(b47,P047_ARCH); rf=p123_adapter.verify(f47,P050_ARCH)
        oldmap=compose_sources(p123_adapter.core_identities(a47),hmap)
        fusemap=compose_sources(p123_adapter.core_identities(a50),hmap)
        sem47=_historical_semantics('P047',p047,a50,P047_RECEIPT,P047_BUILD)
        # Historical P050's semantic receipt belongs to its inline candidate,
        # which is the same core archived as P055-before, not P050-before.
        sem50_history=_historical_semantics('P050',p050,baseline.verify_archive(P055_ARCH),P050_RECEIPT,P050_BUILD)
        current_build=verify_dir/'production/build-record.json'; cb=json.loads(current_build.read_text())
        candidate=verify_dir/'production/bin/wtime-bench'; current_map=cb.get('source_identities')
        if cb.get('source_stable') is not True or cb.get('phase')!='build' or cb.get('exit_code')!=0 or 'compact-executor' not in cb.get('feature_flags',[]) or sha(candidate)!=cb.get('binary_sha256'):
            raise ValueError('current production build record/binary mismatch')
        # P050 full semantics are reusable only because its recorded Core is byte-identical to this current build.
        core_names=('wtime-runtime','wtime-types','wtime-decode','wtime-validate')
        core_current={p:h for p,h in current_map.items() if any(p.startswith('crates/'+n+'/') for n in core_names)}
        core_p050={p:h for p,h in json.loads(P050_BUILD.read_text())['source_identities'].items() if any(p.startswith('crates/'+n+'/') for n in core_names)}
        if core_current!=core_p050: raise ValueError('P050 semantic receipt core differs from current candidate')
        live=focused.sources(); live.update({p.relative_to(ROOT).as_posix():sha(p) for p in (P047_SEL,P050_SEL,Path(__file__).resolve())})
        current_receipt=verify_dir/'receipt.json'
        current_semantic=semantic_receipt.validate(current_receipt, {'task':'P056','semantic_cases':p050['semantic_cases']}, current_map, ROOT)
        sem50={'receipt':ref(current_receipt),'build_record':ref(current_build),'status':current_semantic['status'],
            'required_cases':current_semantic['required_cases'],'current_core_files':len(core_current),
            'current_core_matches_build':True,'historical_p050':sem50_history}
        from p046_verify import P001_BINARY, P001_MANIFEST
        p001=ROOT/'target/performance/baseline'
        p001_manifest=baseline.verify_archive(p001)
        if sha(p001/'manifest.json')!=P001_MANIFEST or sha(p001/'binary/wtime-bench')!=P001_BINARY:
            raise ValueError('P001 archive differs from admitted baseline')
        encoder= p001/'binary/wtime-conformance'
        encoder_record=p001_manifest.get('files',{}).get('binary/wtime-conformance',{})
        if sha(encoder)!=encoder_record.get('sha256') or encoder_record.get('role')!='fixture_encoder_executable':
            raise ValueError('fixture encoder differs from immutable P001 encoder bytes')
        cpu=min(os.sched_getaffinity(0))
        journal['actions'].append('existing source-bound adapters and current production candidate verified')
        (output/'journal.json').write_text(json.dumps(journal,indent=2)+'\n')
        refs={'P047':ref(P047_SEL),'P050':ref(P050_SEL)}
        p47out=output/'P047'; p47out.mkdir()
        report_path=p47out/'partial-report.json'
        rep47=_report('P047',p047,p047rows,(b47/'binary/wtime-bench',_side_map(oldmap,'frozen_archive',P047_ARCH,b47),
          {'binary':ref(b47/'binary/wtime-bench'),'manifest':ref(b47/'manifest.json'),'source_archive_manifest':ref(P047_ARCH/'manifest.json')}),
          (f47/'binary/wtime-bench',_side_map(fusemap,'frozen_archive',P050_ARCH,f47),
          {'binary':ref(f47/'binary/wtime-bench'),'manifest':ref(f47/'manifest.json'),'source_archive_manifest':ref(P050_ARCH/'manifest.json')}),
          p47out,sem47, args.seed, cpu, {'files':live,'locations':[{'prefix':'','path':str(ROOT)}]}, encoder)
        rep47['semantic_provenance']['historical_exact_core_match']=True
        report_path=p47out/'report.json'; report_path.write_text(json.dumps(rep47,indent=2)+'\n')
        p50out=output/'P050'; p50out.mkdir()
        report_path=p50out/'partial-report.json'
        rep50=_report('P050',p050,p050rows,(f47/'binary/wtime-bench',_side_map(fusemap,'frozen_archive',P050_ARCH,f47),
          {'binary':ref(f47/'binary/wtime-bench'),'manifest':ref(f47/'manifest.json'),'source_archive_manifest':ref(P050_ARCH/'manifest.json')}),
          (candidate,_side_map(current_map,'current_production',ROOT),
          {'binary':ref(candidate),'manifest':ref(current_build),'source_archive_manifest':ref(current_build)}),
          p50out,sem50,args.seed+17,cpu,{'files':live,'locations':[{'prefix':'','path':str(ROOT)}]}, encoder)
        report_path=p50out/'report.json'; report_path.write_text(json.dumps(rep50,indent=2)+'\n')
        if _snapshot()!=_snapshot_before: raise ValueError('source inputs changed during variant refresh')
        for ap,ar in ((b47,P047_ARCH),(f47,P050_ARCH)): p123_adapter.verify(ap,ar)
        aggregate={'schema':'wtime-p056-variant-refresh-set-v1','status':'passed',
          'reports':{'P047':ref(p47out/'report.json'),'P050':ref(p50out/'report.json')},
          'additional_statistics_computations':0}
        (output/'report.json').write_text(json.dumps(aggregate,indent=2)+'\n')
        journal['status']='passed'; journal['actions'].append('two complete schema-v2 datasets retained; no statistics computed')
    except BaseException as exc:
        journal['status']='failed'; journal['error']=type(exc).__name__+': '+str(exc)
        (output/'failure.json').write_text(json.dumps({'status':'failed','error':journal['error'],'retained_report':str(report_path) if report_path else None},indent=2)+'\n')
        raise
    finally: (output/'journal.json').write_text(json.dumps(journal,indent=2)+'\n')
    return aggregate

def main(argv=None):
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--verify-dir',required=True,type=Path)
    p.add_argument('--output',required=True,type=Path); p.add_argument('--seed',type=int,default=20261014)
    d=run(p.parse_args(argv)); print(json.dumps({'status':d['status'],'reports':sorted(d['reports'])}))
if __name__=='__main__': main()
