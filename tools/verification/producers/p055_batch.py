#!/usr/bin/env python3
"""Exercise the actual single-case batch boundary on both paired binaries."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import baseline
import compare
import focused_compare
import p123_adapter

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def need(ok,message):
    if not ok: raise ValueError(message)
def reference(path):
    path=Path(path).resolve(strict=True)
    return {'path':str(path),'sha256':sha(path)}

def run(args):
    out=args.output.resolve()
    out.mkdir(parents=True,exist_ok=False)
    before=focused_compare.sources()
    adapter=p123_adapter.verify(args.baseline)
    record=json.loads(args.candidate_record.read_text())
    need(record['schema']=='wtime-runtime-only-production-v1' and record['source_stable']
         and record['exit_code']==0 and record['mode']=='compact'
         and record['instrumented'] is False,'candidate must be current pure Compact production')
    binaries={'baseline':args.baseline.resolve()/'binary/wtime-bench',
              'candidate':Path(record['binary_path']).resolve(strict=True)}
    need(sha(binaries['baseline'])==adapter['binary_sha256']
         and sha(binaries['candidate'])==record['binary_sha256'],'paired executable bytes differ')
    need(sha(args.fixture)=='0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746',
         'single-case fixture is not the admitted tiny control')
    need(sha(args.scalar_fixture)=='ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1',
         'typed scalar fixture is not the admitted accumulator')
    inputs={'adapter':reference(args.baseline/'manifest.json'),
        'candidate_record':reference(args.candidate_record),'fixture':reference(args.fixture),
        'scalar_fixture':reference(args.scalar_fixture),
        **{name+'_binary':reference(path) for name,path in binaries.items()}}
    observations=[]; rejected=[]; commands=[]
    def command(side,mode,case,argv,expected_rejection=False):
        index=len(commands); destination=out/f'command-{index:03d}'
        need(destination.is_relative_to(out),'command evidence escapes exclusive output')
        complete=[str(binaries[side]),*map(str,argv)]
        descriptor={'argv':complete,'side':side,'mode':mode,'case':case,
            'binary_sha256':inputs[side+'_binary']['sha256'],'cpu':args.cpu}
        destination.with_suffix('.json').write_text(json.dumps(descriptor,indent=2)+'\n')
        result=subprocess.run(complete,cwd=ROOT,capture_output=True,text=True,timeout=30,
            preexec_fn=lambda:os.sched_setaffinity(0,{args.cpu}))
        destination.with_suffix('.stdout').write_text(result.stdout)
        destination.with_suffix('.stderr').write_text(result.stderr)
        commands.append({**descriptor,'exit_code':result.returncode})
        if expected_rejection:
            need(result.returncode!=0,'over-cap command unexpectedly executed')
            need(not result.stdout.strip(),'rejected command emitted a successful guest observation')
            need('batch' in result.stderr and ('exceeds 1000000' in result.stderr
                 or 'requested_batch <= 100_000' in result.stderr),
                 'rejection was not the intended driver batch guard')
            rejected.append({'side':side,'mode':mode,'case':case,'exit_code':result.returncode})
            return None
        need(result.returncode==0,'batch probe failed: '+result.stderr[-2000:])
        row=json.loads(result.stdout)
        return row
    for side in binaries:
        capability=command(side,'protocol','production-protocol',['--protocol'])
        need(capability['instrumented'] is False and {'compact','decoded'}<=set(capability['modes']),
             'both production modes must be available without instrumentation')
        for mode in ('decoded','compact'):
            for batch in (100_000,100_001,1_000_000):
                row=command(side,mode,f'single-{batch}',
                    ['--single-case',args.fixture,'call_overhead',7,8,'warm_call',batch,mode])
                compare.validate_observation(row,'call_overhead','warm_call',7,8,batch,mode)
                if batch==1_000_000:
                    need(row['ns_per_operation']*batch>=20_000_000,'expanded cap still cannot cover 20 ms')
                observations.append({'side':side,'mode':mode,'requested_batch':batch,'row':row})
            command(side,mode,'single-over-cap',
                ['--single-case',args.fixture,'call_overhead',7,8,'warm_call',1_000_001,mode],True)
            command(side,mode,'scalar-over-cap',
                ['--scalar-driver',args.scalar_fixture,'scalar_accumulator_n32',32,496,
                 'warm_call',100_001,mode,'mvp','plain'],True)
    need(focused_compare.sources()==before,'probe source identities changed')
    need(all(sha(ref['path'])==ref['sha256'] for ref in inputs.values()),'probe input bytes changed')
    result={'schema':'wtime-p055-batch-proof-v1','task':'P055','status':'passed',
        'source_stable':True,'timing_evidence':False,'candidate_promotion_evidence':False,
        'inputs':inputs,'sources':before,'single_case_max_batch':1_000_000,'scalar_max_batch':100_000,
        'passed_cases':['legacy_batch_wire_and_oracle','above_100k_batch_is_accepted',
            'maximum_batch_covers_twenty_ms','single_case_over_cap_is_rejected','scalar_cap_is_preserved'],
        'failed_cases':0,'observations':observations,'expected_rejections':rejected,'commands':commands}
    need(len(observations)==12 and len(rejected)==8 and len(commands)==22,'batch probe did incomplete work')
    (out/'report.json').write_text(json.dumps(result,indent=2,sort_keys=True,allow_nan=False)+'\n')
    print(json.dumps({'status':'passed','cases':5,'observations':12,'expected_rejections':8,'commands':22}))
    return 0

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('baseline','candidate-record','fixture','scalar-fixture','output'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--cpu',type=int,default=1)
    return run(parser.parse_args())
if __name__=='__main__': raise SystemExit(main())
