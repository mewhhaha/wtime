#!/usr/bin/env python3
"""Deterministic execution comparison against vectors and an optional pinned CLI.

The optional JSON oracle receives PROFILE MODULE EXPORT and returns the same JSON
shape as wtime-probe. The wasmtime adapter runs one self-contained export whose
return value observes its memory/table mutations. The oracle is development-only.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
_spec=importlib.util.spec_from_file_location('fuzz_decode',Path(__file__).with_name('fuzz_decode.py'))
fuzz=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(fuzz)

def generated_cases(seed,count):
    rng=random.Random(seed)
    for index in range(count):
        x=rng.randrange(-2**31,2**31);y=rng.randrange(-2**31,2**31);kind=index%10
        profile='stable-core'
        if kind==0:
            data=fuzz.module_expr(b'\x41'+fuzz.sleb(x)+b'\x41'+fuzz.sleb(y)+b'\x6a')
            name='i32-add';expected={'status':'returned','values':[{'type':'i32','bits':str((x+y)&0xffffffff)}]}
        elif kind==1:
            # Write x to address zero, reload it, and add y: return value observes mutation.
            expr=b'\x41\0\x41'+fuzz.sleb(x)+b'\x36\x02\0\x41\0\x28\x02\0\x41'+fuzz.sleb(y)+b'\x6a'
            raw=fuzz.module_expr(expr);split=raw.index(b'\x07\x07\x01\x03run')
            data=raw[:split]+fuzz.section(5,b'\x01\0\x01')+raw[split:]
            name='memory-write-read';expected={'status':'returned','values':[{'type':'i32','bits':str((x+y)&0xffffffff)}]}
        elif kind==2:
            header=b'\0asm\x01\0\0\0';types=fuzz.section(1,b'\x01\x60\0\x01\x7f');funcs=fuzz.section(3,b'\x02\0\0')
            table=fuzz.section(4,b'\x01\x70\0\x01');exports=fuzz.section(7,b'\x01\x03run\0\x01')
            elems=fuzz.section(9,b'\x01\0\x41\0\x0b\x01\0')
            helper=b'\0\x41'+fuzz.sleb(x)+b'\x0b'
            # Rewrite slot zero with helper then indirect call, observing table state.
            run=b'\0\x41\0\xd2\0\x26\0\x41\0\x11\0\0\x0b'
            data=header+types+funcs+table+exports+elems+fuzz.section(10,b'\x02'+fuzz.uleb(len(helper))+helper+fuzz.uleb(len(run))+run)
            name='table-write-indirect-call';expected={'status':'returned','values':[{'type':'i32','bits':str(x&0xffffffff)}]}
        elif kind==3:
            data=fuzz.module_expr(b'\0');name='unreachable-trap';expected={'status':'trap','trap':'Unreachable'}
        elif kind==4:
            data=fuzz.module_expr(b'\x41'+fuzz.sleb(x)+b'\xc0');name='sign-extension'
            result=(x&255)-256 if x&128 else x&255
            expected={'status':'returned','values':[{'type':'i32','bits':str(result&0xffffffff)}]}
        elif kind==5:
            data=fuzz.module_expr(b'\x43\x01\0\xc0\x7f\xfc\0');name='saturating-nan'
            expected={'status':'returned','values':[{'type':'i32','bits':'0'}]}
        elif kind==6:
            data=fuzz.module_expr(b'\x41'+fuzz.sleb(x)+b'\xfd\x11\xfd\x1b\0');name='simd-splat-extract'
            expected={'status':'returned','values':[{'type':'i32','bits':str(x&0xffffffff)}]}
        elif kind==7:
            # Explicit memory index 1, independently encoded memarg flag bit 6.
            expr=b'\x41\0\x41'+fuzz.sleb(x)+b'\x36\x42\x01\0\x41\0\x28\x42\x01\0'
            raw=fuzz.module_expr(expr);split=raw.index(b'\x07\x07\x01\x03run')
            data=raw[:split]+fuzz.section(5,b'\x02\0\x01\0\x01')+raw[split:]
            name='indexed-memory-write-read';expected={'status':'returned','values':[{'type':'i32','bits':str(x&0xffffffff)}]}
        elif kind==8:
            # Low/high pairs; discard high result and observe wrapping low word.
            expr=b'\x42'+fuzz.sleb(x)+b'\x42\0\x42'+fuzz.sleb(y)+b'\x42\0\xfc\x13\x1a\xa7'
            data=fuzz.module_expr(expr);name='wide-add-low';profile='all-phase3-core'
            expected={'status':'returned','values':[{'type':'i32','bits':str((x+y)&0xffffffff)}]}
        else:
            # One-byte pages, memory64: write/read the single allocated byte.
            byte=x&255
            expr=b'\x42\0\x41'+fuzz.sleb(byte)+b'\x3a\0\0\x42\0\x2d\0\0'
            raw=fuzz.module_expr(expr);split=raw.index(b'\x07\x07\x01\x03run')
            data=raw[:split]+fuzz.section(5,b'\x01\x0c\x01\0')+raw[split:]
            name='memory64-custom-page-byte';profile='all-phase3-core'
            expected={'status':'returned','values':[{'type':'i32','bits':str(byte)}]}
        if kind in (0,1,3):profile='mvp'
        yield {'name':f'{index:04d}-{name}','bytes':data,'expected':expected,'export':'run','profile':profile}

def normalize(result):
    if result.get('status')=='returned':
        return {'status':'returned','values':result.get('values',result.get('results',[{'values':[]}])[0]['values'])}
    return {k:v for k,v in result.items() if k in ('status','trap')}

def allowed_value(actual,expected):
    if actual.get('type')!=expected.get('type'):return False
    if 'allowed' in expected:return actual.get('bits') in [str(v) for v in expected['allowed']]
    if 'nan' in expected:
        bits=int(actual['bits']);width=32 if actual['type']=='f32' else 64
        fraction=bits&((1<<(23 if width==32 else 52))-1)
        exponent=(bits>>(23 if width==32 else 52))&((1<<(8 if width==32 else 11))-1)
        if exponent!=((1<<(8 if width==32 else 11))-1) or fraction==0:return False
        quiet=1<<(22 if width==32 else 51)
        return fraction==quiet if expected['nan']=='canonical' else bool(fraction&quiet)
    return actual.get('bits')==str(expected.get('bits'))

def compare(actual,expected):
    actual=normalize(actual)
    if actual.get('status')!=expected.get('status'):return False
    if expected['status']=='returned':
        left=actual.get('values',[]);right=expected.get('values',[])
        return len(left)==len(right) and all(allowed_value(a,e) for a,e in zip(left,right))
    if expected['status']=='trap':return expected.get('trap') in (None,actual.get('trap'))
    return True

def oracle_run(binary,kind,path,profile,export,timeout):
    if kind=='node':return fuzz.run_process([str(binary),'--max-old-space-size=64',str(Path(__file__).with_name('node_oracle.js')),profile,str(path),export],timeout,memory_mb=None)
    if kind=='json':return fuzz.run_process([str(binary),profile,str(path),export],timeout)
    try:p=subprocess.run([str(binary),'run','--invoke',export,str(path)],capture_output=True,text=True,timeout=timeout)
    except subprocess.TimeoutExpired:return {'status':'timeout'}
    stderr=p.stderr.lower()
    if any(x in stderr for x in ('unsupported','not enabled','disabled feature')):return {'status':'unsupported','message':p.stderr}
    if p.returncode:
        if 'unreachable' in stderr:return {'status':'trap','trap':'Unreachable'}
        return {'status':'oracle-error','message':p.stderr}
    numbers=p.stdout.strip().splitlines()
    try:return {'status':'returned','values':[{'type':'i32','bits':str(int(numbers[-1])&0xffffffff)}]}
    except (ValueError,IndexError):return {'status':'oracle-error','message':p.stdout}

def run(seed,count,target,output,oracle=None,oracle_kind='json',timeout=2):
    output=Path(output);output.mkdir(parents=True,exist_ok=True)
    report={'seed':seed,'selected':count,'executed':0,'failed':0,'vector_compared':0,'oracle_compared':0,'oracle_unsupported':0,'oracle_not_run':0,'failures':[],'target_sha256':hashlib.sha256(Path(target).read_bytes()).hexdigest(),'oracle_identity':hashlib.sha256(Path(oracle).read_bytes()).hexdigest() if oracle else None,'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'core_spec_commit':'7e9fb78c63d65018cc111422f657947b3df80d7b','oracle_adapter_sha256':hashlib.sha256(Path(__file__).with_name('node_oracle.js').read_bytes()).hexdigest() if oracle_kind=='node' else None}
    for case in generated_cases(seed,count):
        path=output/(case['name']+'.wasm');path.write_bytes(case['bytes'])
        actual=fuzz.run_process([str(target),case['profile'],str(path),case['export']],timeout);report['executed']+=1;report['vector_compared']+=1
        failure=None
        if not compare(actual,case['expected']):failure={'kind':'vector-mismatch','target':actual,'expected':case['expected']}
        if oracle:
            reference=oracle_run(oracle,oracle_kind,path,case['profile'],case['export'],timeout)
            if reference['status']=='unsupported':report['oracle_unsupported']+=1
            elif reference['status'] in ('returned','trap'):
                report['oracle_compared']+=1
                if not compare(actual,normalize(reference)):failure={'kind':'oracle-mismatch','target':actual,'oracle':reference}
            else:failure={'kind':'oracle-error','oracle':reference}
        else:report['oracle_not_run']+=1
        if failure:
            report['failed']+=1;report['failures'].append(fuzz.save_failure(output/'failures',case['bytes'],{'seed':seed,'case':case['name'],'profile':case['profile'],**failure,'replay_command':[str(target),case['profile'],'<input>',case['export']]}))
    return report

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--seed',type=int,default=165);p.add_argument('--count',type=int,default=100);p.add_argument('--target',type=Path,default=ROOT/'target/debug/wtime-probe');p.add_argument('--output',type=Path,default=ROOT/'target/differential');p.add_argument('--oracle',type=Path);p.add_argument('--oracle-kind',choices=['json','wasmtime','node'],default='json');p.add_argument('--oracle-sha256');p.add_argument('--timeout',type=float,default=2);a=p.parse_args()
    if a.count<1:p.error('--count must be positive')
    if a.oracle and (not a.oracle_sha256 or hashlib.sha256(a.oracle.read_bytes()).hexdigest()!=a.oracle_sha256):p.error('external oracle requires matching --oracle-sha256')
    report=run(a.seed,a.count,a.target,a.output,a.oracle,a.oracle_kind,a.timeout);(a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));return bool(report['failed'])
if __name__=='__main__':sys.exit(main())
