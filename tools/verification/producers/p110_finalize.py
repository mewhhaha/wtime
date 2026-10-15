"""Review final cold costs, reusing P067 statistics rather than retiming them."""
from __future__ import annotations
import argparse
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(ROOT/'tools/wtime-bench'),str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore,need,normal,read_json,verify_normal,write_new
import p108
import p109
import p110

COLD_STAGES=('prepare','instantiate','first_call','load_and_call')
REQUIRED={('prepare_large','prepare'),*((case,stage) for case in
          ('call_overhead','integer_loop','branch_calls') for stage in COLD_STAGES)}


def cold_review(statistics):
    policy=statistics.get('analysis',{})
    need(type(policy.get('bootstrap_resamples')) is int and policy['bootstrap_resamples']>=10_000
         and type(policy.get('minimum_pairs_per_group_per_run')) is int and policy['minimum_pairs_per_group_per_run']>=30,
         'cold statistics lack the full uncertainty protocol')
    selected=[row for row in statistics.get('per_run',[]) if row.get('stage') in COLD_STAGES]
    runs={row.get('run') for row in selected}
    need(runs=={1,2} and all(type(row.get('run')) is int for row in selected),'cold statistics need two complete runs')
    seeds=[];reviews=[]
    for run in (1,2):
        rows=[row for row in selected if row['run']==run]
        keys={(row['workload'],row['stage']) for row in rows}
        need(len(rows)==len(keys) and REQUIRED<=keys,
             'cold statistics omit or duplicate a protected product')
        if run==1:first_keys=keys
        else:need(keys==first_keys,'cold runs contain different product sets')
        run_seeds={row.get('seed') for row in rows}
        need(len(run_seeds)==1 and all(type(seed) is int for seed in run_seeds),'cold run seed differs between products')
        seeds.append(next(iter(run_seeds)))
        for row in rows:
            need(type(row.get('pairs')) is int and row['pairs']>=30,'cold group is incomplete')
            ratio=row.get('candidate_over_baseline',{}).get('median')
            interval=row.get('median_ratio_95ci',{})
            lower,upper=interval.get('lower'),interval.get('upper')
            need(all(type(value) in (int,float) and math.isfinite(value) and value>0 for value in (ratio,lower,upper))
                 and lower<=upper,'cold ratio or confidence interval is invalid')
            limit=1.25 if row['stage']=='prepare' else 1.05
            state='within_budget' if upper<=limit else 'over_budget' if lower>limit else 'inconclusive'
            reviews.append({'run':run,'workload':row['workload'],'stage':row['stage'],'pairs':row['pairs'],
                            'candidate_over_baseline':ratio,'median_ratio_95ci':interval,'limit':limit,'review':state})
    need(seeds[0]!=seeds[1],'cold runs reused their seed')
    return {'runs':2,'products_per_run':len(first_keys),'rows':reviews,
            'requires_review':any(row['review']!='within_budget' for row in reviews)}


def completed_proof(task,*,root=ROOT):
    base=root/'docs/benchmarks/performance'
    completion_path=base/(task+'-completion.json');completion=read_json(completion_path)
    need(completion.get('task')==task and completion.get('status')=='done','budget prerequisite incomplete: '+task)
    need(len(completion.get('evidence',[]))==1,'budget prerequisite proof is ambiguous')
    evidence=read_json(verify_normal(completion['evidence'][0]))
    need(evidence.get('task')==task and evidence.get('passed') is True and evidence.get('synthetic') is False,
         'budget prerequisite is not real completed evidence')
    store=ArtifactStore(Path(evidence['store']))
    receipt=store.json(evidence['phases']['analyze'])
    need(receipt.get('task')==task and receipt.get('status')=='passed' and receipt.get('lane')=='full'
         and receipt.get('synthetic') is False,'budget prerequisite did not use full analysis')
    proof_path=base/(task+'-proof.json')
    need(receipt['original_outputs']['proof']['sha256']==normal(proof_path)['sha256'],
         'published budget prerequisite differs from finalized analysis')
    proof=read_json(proof_path)
    need(proof.get('task')==task and proof.get('status')=='passed','budget prerequisite proof failed')
    return proof,normal(completion_path),normal(proof_path)


def finalize(verify_dir,measure_dir,output):
    import analyze
    import execution_identity
    import p110_builds
    current=p109.sources()
    policy_path=ROOT/'docs/benchmarks/performance/decisions.json'
    p108.validate_policy(read_json(policy_path),sources=current)
    prerequisites={}
    proofs={}
    for task in ('P067','P108','P109'):
        proofs[task],completion,proof=completed_proof(task)
        prerequisites[task]={'completion':completion,'proof':proof}
    need(proofs['P108'].get('sources')==current and proofs['P109'].get('core_sources')==current,
         'final selection or conformance source is stale')
    footprint_path=verify_dir/'footprints/report.json';footprint=read_json(footprint_path)
    need(footprint.get('schema')=='wtime-p110-footprints-v1' and footprint.get('status')=='passed'
         and footprint.get('core_sources')==current,'selected footprint diagnostics are stale')
    verify_normal(footprint['binary'])
    need(len(footprint['records'])==len(p110.CASES)*len(p110.MODES),'footprint matrix is incomplete')
    observed=set()
    for row in footprint['records']:
        path=verify_normal(row['fixture']);mode=row['mode'];key=(str(path.relative_to(ROOT)),mode)
        need(key not in observed and key[0] in p110.CASES and mode in p110.MODES,'footprint matrix duplicates or substitutes a case')
        observed.add(key);p110.validate_observation(row['observation'],mode,path.stat().st_size)
        verify_normal(row['stdout']);verify_normal(row['stderr'])
    collection_path=measure_dir/'cold/report.json';collection=read_json(collection_path)
    need(collection.get('schema')=='wtime-p110-cold-collection-v1' and collection.get('status')=='passed'
         and collection.get('source_stable') is True and collection.get('timing_evidence') is True,
         'final cold collection is incomplete')
    store=ArtifactStore(ROOT/'target/verification')
    for reference in read_json(verify_normal(collection['artifacts'])).values():store.verify(reference)
    need(set(collection['reports'])=={'default-vs-p001','compact-vs-decoded'},'final cold comparison set differs')
    output.parent.mkdir(parents=True,exist_ok=True)
    statistics={};reviews={}
    for name,reference in collection['reports'].items():
        path=verify_normal(reference);raw=read_json(path)
        execution_identity.verify_inputs(raw)
        need(raw.get('selection')=='original-mvp' and raw.get('passed') is True
             and raw.get('source_stable') is True,'final cold comparison failed')
        summary=analyze.analyze(raw)
        destination=output.parent/(name+'-statistics.json');write_new(destination,summary)
        statistics[name]=normal(destination);reviews[name]=cold_review(summary)
    for name in ('eager-vs-decoded','adaptive-vs-decoded'):
        reference=proofs['P067']['statistics'][name]
        value=read_json(verify_normal(reference))
        statistics[name]=reference;reviews[name]=cold_review(value['analysis'])
    # The build reader owns validation of source identity, restored leaf files,
    # fixed build settings and separate optional deployment costs.
    builds=p110_builds.collect()
    need(current==p109.sources(),'core source changed during final budget review')
    write_new(output,{'schema':'wtime-p110-final-budgets-v1','task':'P110','status':'passed',
        'hard_gate_passed':True,'outcome':'inconclusive' if (
            any(row['requires_review'] for row in reviews.values())
            or builds['optional_tier_review']['review_required']) else 'adopted',
        'core_sources':current,'prerequisites':prerequisites,'policy':normal(policy_path),
        'footprints':normal(footprint_path),'builds':builds,'cold_collection':normal(collection_path),
        'statistics':statistics,'cold_reviews':reviews,'timing_evidence':True,
        'optional_policy':'Compact and Register remain experimental opt-ins. Recorded startup costs are not hidden or promoted as default improvements.',
        'limitations':['The original P001 archive has no comparable build-budget report; no longitudinal build-speed ratio is claimed.',
                       'Cold and warm latency are workload-specific shared-cloud observations, not a dedicated-machine guarantee.',
                       'Retained allocation counters overlap and are not a complete heap census; eager Register capacity is bounded by tested runtime budgets.',
                       'No native backend, executable mappings, compiler workers or AOT deployment artifacts are selected.',
                       'Native AArch64 and an external differential oracle remain unrun.']})


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--verify-dir',type=Path,required=True)
    parser.add_argument('--measure-dir',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();finalize(args.verify_dir,args.measure_dir,args.output)
