"""Validate and analyze complete Register milestone evidence, without retiming."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
BENCH=ROOT/'tools/wtime-bench'
sys.path[:0]=[str(ROOT/'tools'),str(BENCH),str(ROOT/'tools/checks')]
from verification.artifacts import ArtifactStore, VerificationError, need, normal, read_json, verify_normal, write_new


def validate_coverage(eager,adaptive):
    import selection_manifest
    import simd_workloads
    import workloads
    for value in (eager,adaptive):
        selection_manifest.load_data(value)
        need(value['task']=='P067','milestone selection has wrong task')
    controls={'call_overhead','prepare_large'}
    scalar={row['id'] for row in workloads.selection('scalar')}
    memory={row['id'] for row in workloads.selection('memory')}
    representative={row['id'] for row in workloads.selection('representative')}
    floating={row['id'] for row in simd_workloads.entries()
        if row.get('family') in {'simd_float_arithmetic','simd_scalar_float_arithmetic','simd_f64_arithmetic','simd_scalar_f64_arithmetic'}
        or row.get('work_units',{}).get('unit') in {'strict_float_edge_control','strict_f64_minmax_edge_control','strict_scalar_float_edge_control'}}
    need((len(scalar),len(memory),len(representative),len(floating))==(85,53,9,26),'milestone family cardinalities drifted')
    expected={'eager':scalar|memory|representative|floating|controls,'adaptive':scalar|representative|controls}
    for name,value in [('eager',eager),('adaptive',adaptive)]:
        need({row['id'] for row in value['rows']}==expected[name],'missing or substituted '+name+' milestone rows')
        entries={row['id']:row for row in selection_manifest.rows(value)}
        for row in value['rows']:
            need(row['stages']==entries[row['id']]['stages'],'milestone stages omitted for '+row['id'])
            if row['id'] in controls:need(row['role']=='protected','cold control is not protected')
            if row['id'].endswith('_held_out') and row['id'] in representative:
                need(row['role']=='held_out','representative held-out row was reclassified')
    need(eager.get('execution_settings') is None,'eager selection unexpectedly uses a threshold')
    need(adaptive.get('execution_settings')=={'register_entry_threshold':{'baseline':None,'candidate':3}},'adaptive threshold policy differs')
    return {'scalar':85,'memory':53,'strict_float':26,'representative':9,
            'unique_eager':len(expected['eager']),'unique_adaptive':len(expected['adaptive'])}


def choose(decisions,*,hard_gates):
    need(hard_gates is True,'semantic, integrity or hard-budget gate failed')
    need(isinstance(decisions,dict) and set(decisions)=={'decoded','compact'},'both portable comparisons are required')
    need(all(value in ('adopted','rejected','inconclusive') for value in decisions.values()),'unknown comparison decision')
    outcome='rejected' if 'rejected' in decisions.values() else 'inconclusive' if 'inconclusive' in decisions.values() else 'adopted'
    return {'decision':outcome,'recommendation':'eligible_for_P108' if outcome=='adopted' else 'keep_register_opt_in',
            'default_execution':'decoded','universal_threshold':None}


def path_coverage():
    import p145
    completion=read_json(ROOT/'docs/benchmarks/performance/P145-completion.json')
    need(completion.get('task')=='P145' and completion.get('status')=='done','path-coverage task is incomplete')
    evidence=read_json(verify_normal(completion['evidence'][0]))
    need(evidence.get('passed') is True and evidence.get('task')=='P145','path evidence did not pass')
    store=ArtifactStore(ROOT/'target/verification')
    receipt=store.json(evidence['phases']['analyze'])
    path=ROOT/'docs/benchmarks/performance/P145-proof.json';proof=read_json(path)
    need(receipt['status']=='passed' and receipt['original_outputs']['proof']['sha256']==normal(path)['sha256'],
         'path proof differs from finalized evidence')
    need(proof.get('status')=='passed' and proof.get('timing_evidence') is False and proof['core_sources']==p145.current_core(),
         'path proof is stale or makes a timing claim')
    coverage=p145.validate_report(verify_normal(proof['freeze']),verify_normal(proof['observations']))
    need(coverage==proof['coverage'],'path coverage differs from validated observations')
    return {'proof':normal(path),'coverage':coverage,'timing_evidence':False}


def analyze(verify_dir,measure_dir,output):
    import analyze as paired
    import build_budget
    import candidate_gate
    import entry_batch_analyze
    import entry_sequence_analyze
    import execution_identity
    import p143
    import p067_workflow
    import workloads
    coverage=validate_coverage(read_json(p067_workflow.SELECTIONS['eager']),read_json(p067_workflow.SELECTIONS['adaptive']))
    paths=path_coverage()
    collection=read_json(measure_dir/'report.json')
    need(collection.get('schema')=='wtime-p067-measurements-v1' and collection.get('status')=='passed'
         and collection.get('timing_evidence') is True and collection.get('source_stable') is True,'milestone measurements incomplete')
    context_path=ROOT/'target/recovery/P080-isolate-0001/runs/attempt-0001/result.json'
    context=read_json(context_path)
    need(context.get('status')=='passed' and context.get('shared_encoder_restored') is True,
         'known fixture-encoder replacement has no restoration record')
    store=ArtifactStore(ROOT/'target/verification')
    for reference in read_json(verify_normal(collection['artifacts'])).values():store.verify(reference)
    expected={'eager-vs-decoded','eager-vs-compact','adaptive-vs-decoded',
              'p001-scalar','p001-memory','p001-simd','p001-representative','p001-original-mvp','entry-totals','entry-sequence'}
    need(set(collection['reports'])==expected,'milestone comparison set incomplete')
    prior=read_json(verify_dir/'prior-controls.json')
    need(prior.get('status')=='passed' and prior.get('source_current') is True,'source-current runtime controls missing')
    verify_normal(prior['capacity']);verify_normal(prior['evidence'])
    tiers={name:read_json(verify_dir/name/'report.json') for name in ('eager','adaptive','decoded')}
    counts={name:p143.validate_tier(report,3 if name=='adaptive' else None,'decoded' if name=='decoded' else 'register')
            for name,report in tiers.items()}
    builds={}
    for name,features in [('default',[]),('register',['wtime-runtime/register-executor']),('compact',['wtime-runtime/compact-executor'])]:
        report=read_json(verify_dir/(name+'-build.json'))
        need(report.get('features')==features and report.get('source_stable') is True
             and report.get('budget_pass') is True and build_budget.check_budget(report),'hard build budget failed: '+name)
        need(report['source_inputs']==report['source_inputs_after']==report['isolated_source_inputs_after'],'build source changed: '+name)
        builds[name]={'report':normal(verify_dir/(name+'-build.json')),'clean_seconds':report['clean_seconds'],
                      'runtime_leaf_seconds':report['runtime_leaf_seconds'],'stripped_bytes':report['stripped_bytes'],
                      'peak_rss_kib':report['clean_peak_rss_kib']}
    output.parent.mkdir(parents=True,exist_ok=True)
    stats={};decisions={};raw_counts={}
    for name in ('eager-vs-decoded','eager-vs-compact','adaptive-vs-decoded'):
        path=verify_normal(collection['reports'][name]);report=candidate_gate.validate(path)
        expected_selection=read_json(p067_workflow.SELECTIONS['adaptive' if name.startswith('adaptive') else 'eager'])
        need(report['declaration']==expected_selection,'comparison substituted milestone selection')
        decision=candidate_gate.decision(report)
        destination=output.parent/(name+'-statistics.json');write_new(destination,decision)
        stats[name]=normal(destination);raw_counts[name]=report['counts']
        if name.startswith('eager'):decisions[name.removeprefix('eager-vs-')]=decision['decision']
    for family in ('scalar','memory','simd','representative','original-mvp'):
        name='p001-'+family;path=verify_normal(collection['reports'][name]);report=read_json(path)
        execution_identity.verify_inputs(report)
        need(report['selection']==family and report.get('passed') is True and report.get('source_stable') is True,'P001 family failed: '+family)
        _metadata,runs=paired._validated_runs(report)
        expected_ids={row['id'] for row in workloads.selection(family)}
        for run in runs:
            need({key[0] for key in run['groups']}==expected_ids,'P001 family omitted workloads: '+family)
        decision=paired.analyze(report)
        destination=output.parent/(name+'-statistics.json');write_new(destination,decision)
        stats[name]=normal(destination)
        raw_counts[name]={'runs':len(runs),'workloads':len(expected_ids),'pairs':sum(len(rows) for run in runs for rows in run['groups'].values())}
    entry_batch_analyze.run_analysis(verify_normal(collection['reports']['entry-totals']),output.parent/'entry-totals')
    entry_sequence_analyze.run_analysis(verify_normal(collection['reports']['entry-sequence']),output.parent/'entry-sequence')
    outcome=choose(decisions,hard_gates=True)
    write_new(output,{'schema':'wtime-p067-milestone-v1','task':'P067','status':'passed',**outcome,
        'coverage':coverage,'execution_paths':paths,'tier_checks':counts,'runtime_controls':normal(verify_dir/'prior-controls.json'),
        'measurements':normal(measure_dir/'report.json'),'statistics':stats,'sample_counts':raw_counts,'builds':builds,
        'register_comparisons':decisions,'timing_evidence':True,
        'collection_context':{'encoder_restoration':normal(context_path),
            'background_work':'Independent benchmarks on CPUs 1, 2 and 3 and bounded untimed verification/build work on CPU 4 overlapped this CPU 0 collection.',
            'fixture_encoder_event':'An unrelated Cargo test replaced the shared preparation encoder after the first comparison had encoded its fixtures. Its hash-verified original bytes were restored before the final integrity check. Retained Wasm fixtures and timed runtime executables were unchanged.',
            'physical_core_idle':'unknown/shared cloud'},
        'limitations':['Full SIMD P001 control includes 114 rows; the primary explicit Register strict-float projection has 26 model-defined rows.',
          'Adaptive threshold 3 is measured on scalar and representative routes; memory and strict float use eager Register only.',
          'No universal promotion threshold is inferred from per-entry observations or paired batch totals.',
          'Shared-cloud measurements do not establish a stable dedicated-machine performance claim or native AArch64 results.']})


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--verify-dir',type=Path)
    parser.add_argument('--measure-dir',type=Path);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--paths-only',action='store_true');args=parser.parse_args()
    if args.paths_only:write_new(args.output,path_coverage())
    else:
        need(args.verify_dir is not None and args.measure_dir is not None,'analysis requires verification and measurement directories')
        analyze(args.verify_dir,args.measure_dir,args.output)
if __name__=='__main__':main()
