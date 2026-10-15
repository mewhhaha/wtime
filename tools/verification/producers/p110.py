"""Selected portfolio resource diagnostics; these observations are not timings."""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(Path(__file__).resolve().parent)]
from verification.artifacts import need,normal,read_json,write_new
import p109

MODES=('decoded','compact','register-eager','register-adaptive-3')
CASES=('tools/wtime-bench/fixtures/call_overhead.wasm',
       'tools/wtime-bench/fixtures/prepare_large.wasm',
       'tools/wtime-bench/fixtures/representative/graph_bfs.wasm')


def freeze(output):
    write_new(output,{'status':'passed','core_sources':p109.sources(),'timing_evidence':False})


def validate_observation(value,mode,length):
    need(value.get('schema')=='wtime-module-footprint-v1' and value.get('status')=='passed'
         and value.get('timing_evidence') is False and value.get('instrumented') is True
         and value.get('complete_heap_census') is False and value.get('mode')==mode,
         'footprint scope or mode changed')
    need(type(value.get('guest_invocations')) is int and value['guest_invocations']==0,'footprint invoked guest code')
    for name in ('wasm_bytes','source_bytes','lowered_control_bytes','function_plan_bytes'):
        need(type(value.get(name)) is int and value[name]>=0,'invalid footprint field '+name)
    need(value['wasm_bytes']==value['source_bytes']==length,'footprint source byte length changed')
    compact=value.get('compact_preparation_counts')
    if mode!='compact':need(compact is None,'unselected Compact artifact reported')
    elif compact is not None:
        need(set(compact)=={'accepted_functions','declined_functions','attempted_work','retained_bytes'}
             and all(type(item) is int and item>=0 for item in compact.values()),'invalid Compact footprint')
    cache=value.get('register_cache_snapshot')
    if mode=='register-adaptive-3':
        if value.get('register_cache_state')=='unavailable':
            need(cache is None,'unavailable adaptive cache has a snapshot')
            return
        need(value.get('register_cache_state')=='empty','adaptive cache state is unspecified')
        need(isinstance(cache,dict) and cache.get('state_after_compile')=='empty' and cache.get('threshold')==3
             and cache.get('entry_count')==0 and cache.get('active_body') is None,'adaptive cache was populated during compile')
        need(all(type(cache.get(name)) is int and cache[name]>=0 for name in
                 ('entry_limit','entry_count','retained_bytes','byte_limit','remaining_attempted_work','remaining_retained_work')),
             'adaptive footprint limits are malformed')
        lifecycle=cache.get('lifecycle')
        need(isinstance(lifecycle,dict) and set(lifecycle)=={'preparation_starts','builder_starts','completed',
             'published','publication_failures','declined','exhausted','invalid','aborted','evictions'},
             'adaptive lifecycle counters are incomplete')
        need(cache['retained_bytes']<=cache['byte_limit'] and all(type(v) is int and v==0 for v in lifecycle.values()),
             'adaptive compile-only cache charged execution/preparation')
    else:need(cache is None and value.get('register_cache_state')=='not_requested',
              'cache snapshot reported for a non-adaptive policy')


def footprints(binary,output):
    core=p109.sources();original_binary=normal(binary)
    output.mkdir(parents=True,exist_ok=False)
    executable=output/'wtime-footprint-inspect';shutil.copy2(binary,executable)
    binary_ref=normal(executable)
    need(binary_ref['sha256']==original_binary['sha256'],'footprint executable copy changed')
    records=[]
    for relative in CASES:
        path=ROOT/relative;need(path.is_file(),'required footprint fixture missing: '+relative)
        fixture=normal(path)
        for mode in MODES:
            stem=path.stem+'-'+mode
            argv=[str(executable.resolve()),str(path),'mvp',mode]
            result=subprocess.run(argv,cwd=ROOT,text=True,capture_output=True,timeout=120)
            (output/(stem+'.stdout')).write_text(result.stdout)
            (output/(stem+'.stderr')).write_text(result.stderr)
            need(result.returncode==0,'footprint inspection failed: '+result.stderr[-2000:])
            from verification.artifacts import parse_json
            value=parse_json(result.stdout);validate_observation(value,mode,path.stat().st_size)
            need(value['profile']=='mvp' and value['input']==str(path),'footprint fixture/profile changed')
            records.append({'mode':mode,'fixture':fixture,'command':argv,'exit_code':result.returncode,
                'stdout':normal(output/(stem+'.stdout')),'stderr':normal(output/(stem+'.stderr')),'observation':value})
    need(core==p109.sources() and normal(binary)==original_binary and normal(executable)==binary_ref,
         'footprint source or executable changed')
    write_new(output/'report.json',{'schema':'wtime-p110-footprints-v1','status':'passed',
        'core_sources':core,'binary':binary_ref,'records':records,'timing_evidence':False,
        'complete_heap_census':False,'guest_invocations':0,
        'limitations':['Counters report their own allocation scopes, not total live heap or RSS.',
                       'Diagnostic features may add bookkeeping; these counters are not uninstrumented timing evidence.',
                       'Eager Register artifact bytes are covered by runtime budget checks, not exposed as a public module counter.']})


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('phase',choices=('freeze','footprints'))
    parser.add_argument('--binary',type=Path);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if args.phase=='freeze':freeze(args.output)
    else:footprints(args.binary,args.output)
