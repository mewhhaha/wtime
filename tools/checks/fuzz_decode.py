#!/usr/bin/env python3
"""Seeded, process-isolated binary mutation and validation smoke runner."""
from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import resource
import subprocess
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[2]
BAD={'panic','timeout','signal','protocol-error','host-error'}

def uleb(n):
    out=bytearray()
    while True:
        b=n&127;n>>=7;out.append(b|(128 if n else 0))
        if not n:return bytes(out)

def sleb(n):
    out=bytearray()
    while True:
        b=n&127;n>>=7;done=(n==0 and b&64==0)or(n==-1 and b&64!=0);out.append(b|(0 if done else 128))
        if done:return bytes(out)

def section(i,payload):return bytes([i])+uleb(len(payload))+payload

def module_expr(expr,result=0x7f):
    ty=bytes([1,0x60,0,1,result]);body=b'\0'+expr+b'\x0b'
    return b'\0asm\x01\0\0\0'+section(1,ty)+section(3,b'\x01\0')+section(7,b'\x01\x03run\0\0')+section(10,b'\x01'+uleb(len(body))+body)

def structure_seeds(rng):
    """Independent byte encodings exercise length, control and cyclic type graphs."""
    seeds=[b'\0asm\x01\0\0\0',module_expr(b'\x41'+sleb(rng.randrange(-2**31,2**31)))]
    for depth in (1,8,64,256):
        seeds.append(module_expr(b'\x02\x40'*depth+b'\x0b'*depth+b'\x41\0'))
    for count in (1,8,128):
        # One recursive group of structs with nullable references to adjacent types.
        group=b'\x01\x4e'+uleb(count)
        for index in range(count):group+=b'\x5f\x01\x63'+sleb((index+1)%count)+b'\0'
        seeds.append(b'\0asm\x01\0\0\0'+section(1,group))
    return seeds

def mutate(data,rng):
    b=bytearray(data);kind=rng.randrange(7)
    if kind==0 and b:b[rng.randrange(len(b))]^=1<<rng.randrange(8)
    if kind==1 and b:
        at=rng.randrange(len(b));del b[at:at+rng.randrange(1,min(16,len(b)-at)+1)]
    elif kind==2:b[rng.randrange(len(b)+1):rng.randrange(len(b)+1)]=bytes([rng.randrange(256)])
    elif kind==3:b=b[:rng.randrange(len(b)+1)]
    elif kind==4:
        at=rng.randrange(len(b)+1);b[at:at]=b'\xff'*rng.randrange(1,12)+b'\x7f'
    elif kind==5 and b:
        at=rng.randrange(len(b));b[at]=rng.choice([0,0x40,0x4e,0x60,0x63,0x7f,0x80,0xff])
    elif kind==6:
        at=rng.randrange(len(b)+1);b[at:at]=b'\0'*rng.randrange(1,32)
    return bytes(b)

def _limits(memory_mb,cpu_seconds):
    def apply():
        if memory_mb is not None:resource.setrlimit(resource.RLIMIT_AS,(memory_mb*1024*1024,)*2)
        resource.setrlimit(resource.RLIMIT_CPU,(cpu_seconds,cpu_seconds+1))
        resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    return apply

def run_process(command,timeout=2.0,memory_mb=256):
    try:
        p=subprocess.run(command,capture_output=True,text=True,timeout=timeout,preexec_fn=_limits(memory_mb,max(1,math.ceil(timeout))))
    except subprocess.TimeoutExpired:return {'status':'timeout'}
    if p.returncode<0:return {'status':'signal','signal':-p.returncode,'stderr':p.stderr[-4000:]}
    try:
        result=json.loads(p.stdout)
        if not isinstance(result,dict) or 'status' not in result:raise ValueError()
        return result
    except (ValueError,TypeError):return {'status':'protocol-error','returncode':p.returncode,'stdout':p.stdout[-4000:],'stderr':p.stderr[-4000:]}

def run_oracle(command,path,timeout=2.0):
    try:p=subprocess.run([*command,str(path)],capture_output=True,text=True,timeout=timeout)
    except subprocess.TimeoutExpired:return {'status':'timeout'}
    text=p.stderr.lower()
    if any(word in text for word in ('not enabled','unsupported feature','feature is disabled','unimplemented')):
        return {'status':'unsupported','message':p.stderr[-4000:]}
    return {'status':'valid' if p.returncode==0 else 'invalid','message':p.stderr[-4000:]}

def minimize(data,predicate,budget=64):
    """Bounded delta deletion. The caller preserves the exact failure category."""
    current=data;stride=max(1,len(current)//2);attempts=0
    while stride and attempts<budget:
        changed=False
        for offset in range(0,len(current),stride):
            candidate=current[:offset]+current[offset+stride:];attempts+=1
            if predicate(candidate):current=candidate;changed=True;break
            if attempts>=budget:break
        if not changed:stride//=2
    return current

def save_failure(directory,data,metadata,predicate=None):
    directory=Path(directory);directory.mkdir(parents=True,exist_ok=True)
    minimized=minimize(data,predicate) if predicate else data
    digest=hashlib.sha256(minimized).hexdigest();path=directory/(digest+'.wasm');path.write_bytes(minimized)
    metadata={**metadata,'sha256':digest,'original_bytes':len(data),'minimized_bytes':len(minimized),'input':str(path),'hex':minimized.hex()}
    path.with_suffix('.json').write_text(json.dumps(metadata,indent=2)+'\n');return metadata

def fuzz(seed,iterations,command,profile,directory,timeout=2,corpus=(),oracle=None):
    rng=random.Random(seed);seeds=structure_seeds(rng)+[Path(p).read_bytes() for p in corpus]
    report={'seed':seed,'profile':profile,'selected':iterations,'executed':0,'failed':0,'statuses':{},'oracle_unsupported':0,'oracle_differences':0,'failures':[],'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'corpus_sha256':{str(p):hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in corpus},'core_spec_commit':'7e9fb78c63d65018cc111422f657947b3df80d7b'}
    report['target_sha256']=hashlib.sha256(Path(command[0]).read_bytes()).hexdigest() if Path(command[0]).is_file() else None
    directory=Path(directory);directory.mkdir(parents=True,exist_ok=True)
    path=directory/'current.wasm'
    for index in range(iterations):
        base=seeds[index%len(seeds)]
        data=base if index<len(seeds) else mutate(base,rng)
        path.write_bytes(data);result=run_process([*command,profile,str(path)],timeout)
        status=result['status'];report['executed']+=1;report['statuses'][status]=report['statuses'].get(status,0)+1
        if status in BAD:
            def same(candidate):
                path.write_bytes(candidate)
                return run_process([*command,profile,str(path)],timeout)['status']==status
            meta=save_failure(directory/'failures',data,{'seed':seed,'iteration':index,'profile':profile,'result':result,'replay_command':[*command,profile,'<input>']},same)
            report['failed']+=1;report['failures'].append(meta)
        if oracle:
            path.write_bytes(data);reference=run_oracle(oracle,path,timeout)
            if reference['status']=='unsupported':report['oracle_unsupported']+=1
            elif status in ('valid','invalid','malformed') and reference['status'] in ('valid','invalid') and (status=='valid')!=(reference['status']=='valid'):
                report['oracle_differences']+=1
                report['failures'].append(save_failure(directory/'differences',data,{'seed':seed,'iteration':index,'profile':profile,'target':result,'oracle':reference,'classification':'requires revision/profile triage'}))
    path.unlink(missing_ok=True);return report

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--seed',type=int,default=164);p.add_argument('--iterations',type=int,default=1000);p.add_argument('--profile',choices=['mvp','stable-core','all-phase3-core'],default='stable-core');p.add_argument('--target',type=Path,default=ROOT/'target/debug/wtime-probe');p.add_argument('--corpus',type=Path,action='append',default=[]);p.add_argument('--oracle',type=Path);p.add_argument('--timeout',type=float,default=2);p.add_argument('--output',type=Path,default=ROOT/'target/fuzz');args=p.parse_args()
    if args.iterations<1:p.error('--iterations must be positive')
    report=fuzz(args.seed,args.iterations,[str(args.target)],args.profile,args.output,args.timeout,args.corpus,[str(args.oracle),'validate']if args.oracle else None)
    args.output.mkdir(parents=True,exist_ok=True);(args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));return bool(report['failed']or report['oracle_differences'])
if __name__=='__main__':sys.exit(main())
