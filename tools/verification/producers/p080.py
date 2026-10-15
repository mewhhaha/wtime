"""Finalize source-current specialist evidence without promoting restored trials."""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore,need,normal,read_json,verify_normal,write_new
import p080_policy
import p080_workflow
import p109
import p110_builds
from p110_finalize import completed_proof

EXPERIMENTS=('P068','P069','P071','P072','P073','P074','P076','P077','P078')


def restored_experiments(plan):
    tasks={row['id']:row for row in plan['tasks']}
    result={}
    for task in EXPERIMENTS:
        row=tasks[task]
        need(row['status']=='done' and row['outcome'] in ('rejected','inconclusive'),
             'specialist retention policy differs: '+task)
        need(bool(row['evidence']),'specialist trial lacks retained evidence: '+task)
        for reference in row['evidence']:verify_normal(reference)
        result[task]={'outcome':row['outcome'],'runtime_changes_retained':False,'evidence':row['evidence']}
    need(tasks['P079']['status']=='done' and tasks['P079']['outcome']=='contract_only',
         'specialist resource contract is incomplete')
    for reference in tasks['P079']['evidence']:verify_normal(reference)
    return result,tasks['P079']['evidence']


def analyze(verify_dir,measure_dir,output):
    current=p109.sources()
    prior=read_json(verify_dir/'prior.json')
    need(prior.get('schema')=='wtime-p080-prior-validation-v1' and prior.get('status')=='passed'
         and prior.get('source_stable') is True,'specialist correctness preflight is incomplete')
    need(prior['p109_full_verification']['core_sources']==current
         and prior['p109_full_verification']['passed']>=781,
         'full selected-tier semantics are stale or incomplete')
    verify_normal(prior['p109_full_verification']['receipt'])
    verify_normal(prior['p109_corpus']['matrix'])
    need(prior['p109_corpus']['assertions']==583040,'full normative matrix changed')
    p067,completion_ref,p067_ref=completed_proof('P067')
    need(p067.get('default_execution')=='decoded','Register milestone changed the default policy')
    restored,resource_history=restored_experiments(read_json(ROOT/'task-plans/performance/TASKS.json'))
    collection_path=measure_dir/'comparisons/summary.json'
    collection=read_json(collection_path)
    need(collection.get('schema')=='wtime-p080-focused-comparisons-v1' and collection.get('status')=='passed'
         and collection.get('source_stable') is True and collection.get('timing_evidence') is True,
         'specialist pair collection is incomplete')
    # Resolve all benchmark modules from the frozen private harness consistently.
    # Its two new lifecycle adapters coexist with the original P067 collector.
    p080_workflow._private_modules(p080_workflow.PRIVATE_ROOT)
    import analyze as paired
    import candidate_gate
    import execution_identity
    import workloads
    before=p109.sources()
    store=ArtifactStore(ROOT/'target/verification')
    collector_forks={}
    owned_collectors={'tools/wtime-bench/continuation_pairing.py','tools/wtime-bench/focused_compare.py'}
    for name,digest in collection['candidate_sources'].items():
        executed=p080_workflow.PRIVATE_ROOT/name
        need(hashlib.sha256(executed.read_bytes()).hexdigest()==digest,
             'specialist execution input changed in its frozen source tree: '+name)
        if name in owned_collectors:
            collector_forks[name]={'executed':normal(executed),'original_checkout_source':store.add(ROOT/name)}
        else:
            need(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,
                 'specialist execution input differs from current source: '+name)
    need(set(collector_forks)==owned_collectors,'private lifecycle collector inventory is incomplete')
    for reference in read_json(verify_normal(collection['artifacts'])).values():store.verify(reference)
    reports={family:row['report'] for family,row in collection['comparisons'].items()}
    reports.update(collection['previous_family_reports']['reports'])
    need(set(reports)=={'gc-exceptions','simd','threads','continuations'},'specialist family portfolio incomplete')
    statistics={};counts={};wait_samples=[];worker_counts={}
    output.parent.mkdir(parents=True,exist_ok=True)
    for family,reference in sorted(reports.items()):
        path=verify_normal(reference)
        if family in ('threads','continuations'):
            raw=candidate_gate.validate(path)
        else:
            raw=read_json(path);execution_identity.verify_inputs(raw)
            need(raw.get('selection')==family,'specialist raw family differs')
        _metadata,runs=paired._validated_runs(raw)
        expected={row['id'] for row in workloads.selection(family)}
        for run in runs:
            ids={key[0] for key in run['groups']}
            need(expected<=ids and ids<=expected|{'call_overhead','prepare_large'},
                 'specialist family omits or substitutes a workload: '+family)
            need(all(len(rows)==30 for rows in run['groups'].values()),'specialist paired group is incomplete')
        summary=paired.analyze(raw)
        destination=output.parent/(family+'-statistics.json');write_new(destination,summary)
        statistics[family]=normal(destination)
        counts[family]={'workloads':len(expected),'runs':2,
                        'pairs':sum(len(rows) for run in runs for rows in run['groups'].values())}
        if family=='threads':
            import thread_compare
            # Retain the driver's individual distributions. A grouped duration
            # is not used to invent individual wait or collection percentiles.
            for run in raw['runs']:
                for pair in run['pairs']:
                    observed=thread_compare.worker_identities(pair['candidate'])
                    if observed:
                        worker_counts.setdefault(pair['workload'],set()).update(observed)
                    if pair['workload']=='threads_wait_notify_latency':
                        wait_samples.append({'run':run['run_id'],'pair_id':pair['pair_id'],
                            'baseline':pair['baseline'],'candidate':pair['candidate']})
    wait_path=output.parent/'individual-wait-observations.json'
    need(len(wait_samples)==60,'individual wait groups are incomplete')
    write_new(wait_path,{'timing_evidence':True,'groups':wait_samples,
                         'scope':'Driver-retained individual samples and summaries; no batch-derived tail estimates.'})
    comparators=p080_policy.validate_comparators([
        {'family':family,'engine':'wasmtime','status':'unrun' if family=='simd' else 'unsupported',
         'ratio':None,'reason':('No matched per-row Wasmtime timing was collected in this SIMD portfolio.' if family=='simd'
             else 'The pinned '+family+' benchmark driver has no matched Wasmtime observation interface; these measurements are wtime-only.')}
        for family in ('gc-exceptions','simd','threads','continuations')])
    builds=p110_builds.collect()
    need(current==before==p109.sources(),'runtime source changed during specialist analysis')
    write_new(output,{'schema':'wtime-p080-specialist-milestone-v1','task':'P080','status':'passed',
        'decision':'inconclusive','timing_evidence':True,'core_sources':current,
        'p067':{'completion':completion_ref,'proof':p067_ref},'correctness':normal(verify_dir/'prior.json'),
        'measurements':normal(collection_path),'family_reports':reports,'statistics':statistics,'counts':counts,
        'collector_forks':collector_forks,'collection_source_root':str(p080_workflow.PRIVATE_ROOT),
        'individual_wait_observations':normal(wait_path),'comparators':comparators,
        'thread_worker_identities':{name:[{'effective_workers':effective,'worker_quota':quota}
            for effective,quota in sorted(values)] for name,values in sorted(worker_counts.items())},
        'retained_specialist_runtime_optimizations':[],'optimized_specialist_paths_requested':0,
        'experiments':restored,'resource_contract_history':resource_history,'current_builds':builds,
        'limitations':['Every specialist trial was restored after rejection or inconclusive promotion evidence; the combined decoded runtime has no newly retained specialist kernel.',
            'Family results compare wtime against its own frozen predecessor and are not universal Wasmtime ratios.',
            'Two lifecycle collector modules run from an explicitly frozen private source tree; their exact sources are retained separately from the original P067 collector. All runtime and other execution inputs match the checkout.',
            'Unsupported comparator labels describe the pinned harness interface; they do not claim engine-wide proposal support.',
            'Wait tails come from individual driver observations; no collection pause or tail claim is inferred from grouped averages.',
            'Thread rows retain requested-worker labels. The predeclared two-CPU worker mask yields a quota of two; rows requesting four workers therefore use two effective workers and do not establish four-worker scaling.',
            'Measurements share a cloud host with the P067 collection; CPU affinity does not establish physical-core isolation.',
            'Native AArch64 execution remains unrun.']})


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--verify-dir',type=Path,required=True)
    parser.add_argument('--measure-dir',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();analyze(args.verify_dir,args.measure_dir,args.output)
