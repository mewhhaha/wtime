"""Execute all typed family routes against a hash-bound frozen P001 core.

Each workload/stage is an oracle probe with batch one. These intervals are not
paired performance evidence and cannot support a speed or promotion claim.
"""
from pathlib import Path
import argparse
import gzip
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tarfile

ROOT=Path(__file__).resolve().parents[3]
HERE=ROOT/'tools/wtime-bench'
sys.path.insert(0,str(HERE))
P001_MANIFEST='7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472'
P001_BINARY='735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498'
COUNTS={'scalar':85,'memory':35,'simd':114,'representative':9}

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def need(condition,message):
    if not condition:raise ValueError(message)

def validate_report(report):
    import workloads
    need(report.get('schema')=='p123-capabilities-v1' and report.get('status')=='passed', 'invalid capability report')
    for key,value in [('timing_evidence',False),('source_stable',True),('compile_source_stable',True)]:
        need(report.get(key) is value,'invalid capability '+key)
    need(report.get('baseline_manifest_sha256')==P001_MANIFEST and report.get('original_binary_sha256')==P001_BINARY,'capability report does not bind immutable P001')
    for key in ('binary_sha256','adapter_manifest_sha256'):
        need(isinstance(report.get(key),str) and re.fullmatch('[0-9a-f]{64}',report[key]), 'missing capability '+key)
    need(set(report.get('families',{}))==set(COUNTS),'incomplete capability family matrix')
    for family,count in COUNTS.items():
        value=report['families'][family]
        need(all(type(value.get(k)) is int and value[k]==v for k,v in [('supported',count),('passed',count),('unsupported',0)]),'invalid capability counts '+family)
        selected=workloads.selection(family);expected={row['id']:row for row in selected}
        rows=value.get('rows',[])
        need(len(rows)==count and {row.get('id') for row in rows}==set(expected),'capability row membership '+family)
        for row in rows:
            entry=expected[row['id']]
            need(row.get('passed') is True and row.get('stages')==entry['stages'],'capability stage/result '+row['id'])
            need(row.get('fixture_sha256')==entry['source']['wasm_sha256'],'capability fixture '+row['id'])
            observations=row.get('observations',[])
            need(len(observations)==len(entry['stages']) and [x.get('stage') for x in observations]==entry['stages'],'capability observations '+row['id'])
            need(all(x.get('oracle_checked') is True and x.get('mode')=='decoded' for x in observations),'capability oracle/mode '+row['id'])
    return report

def probe(adapter,output):
    import baseline,compare,p123_adapter,workloads
    import scalar_workloads as scalar
    import memory_workloads as memory
    import memory_scaling
    import simd_workloads as simd
    import simd_scaling
    import representative_workloads as representative
    import representative_compare
    import representative_scaling
    import run as original_workloads
    adapter=Path(adapter).resolve();output=Path(output).resolve()
    record=p123_adapter.verify(adapter)
    frozen=Path(record['baseline'])
    need(record['legacy_api'] is True and record['mode']=='decoded','P123 probes require P001 decoded API')
    need(sha(frozen/'manifest.json')==P001_MANIFEST and sha(frozen/'binary/wtime-bench')==P001_BINARY,'P001 original identities differ')
    output.mkdir(parents=True,exist_ok=False)
    shutil.copy2(__file__,output/'helper.executed.py')
    # Preserve every producer/model helper before the first guest command.
    before=p123_adapter.harness_identities()
    for name,digest in before.items():
        destination=output/'executed'/name;destination.parent.mkdir(parents=True,exist_ok=True)
        destination.write_bytes((ROOT/name).read_bytes());need(sha(destination)==digest,'helper changed during capture')
    binary=adapter/'binary/wtime-bench';encoder=frozen/'binary/wtime-conformance'
    before_binary=sha(binary);before_encoder=sha(encoder);before_adapter=sha(adapter/'manifest.json')
    shutil.copy2(binary,output/'wtime-bench')
    shutil.copy2(adapter/'manifest.json',output/'adapter-manifest.json')
    (output/'observations').mkdir()
    commands=output/'commands.jsonl'
    original=compare.process
    sequence=0
    def captured(exe,args,cpu,timeout=180):
        nonlocal sequence
        sequence+=1
        argv=[str(exe),*[str(x) for x in args]]
        with commands.open('a') as log:log.write(json.dumps({'argv':argv,'cpu':cpu,'cwd':str(ROOT)})+'\n')
        value=original(exe,args,cpu,timeout)
        raw=json.dumps(value,sort_keys=True).encode()
        (output/'observations'/f'{sequence:04d}.json.gz').write_bytes(gzip.compress(raw,mtime=0))
        return value
    compare.process=captured
    cpu=min(__import__('os').sched_getaffinity(0))
    families={}
    fixtures={}
    try:
        for family in COUNTS:
            rows=workloads.selection(family)
            directory=output/'fixtures'/family;directory.mkdir(parents=True)
            encoded={}
            for entry in rows:
                source=HERE/entry['source']['path'];dest=directory/source.with_suffix('.wasm').name
                need(sha(source)==entry['source']['sha256'],'WAT identity differs '+entry['id'])
                if not dest.exists():
                    command=[str(encoder),'encode',str(source),'-o',str(dest)]
                    with commands.open('a') as log:log.write(json.dumps({'argv':command,'cwd':str(ROOT)})+'\n')
                    process=subprocess.run(command,cwd=ROOT,capture_output=True,text=True)
                    (directory/(source.stem+'.encode.stdout')).write_text(process.stdout)
                    (directory/(source.stem+'.encode.stderr')).write_text(process.stderr)
                    need(process.returncode==0,'encoder failed '+source.name)
                need(sha(dest)==entry['source']['wasm_sha256'],'encoded fixture differs '+entry['id'])
                encoded[entry['id']]=dest
            if family=='scalar':
                typed_rows=[entry for entry in rows if entry.get('oracle',{}).get('module')=='scalar_workloads.py']
                compare._preflight_scalar_side(binary,'decoded',typed_rows,encoded,cpu,scalar)
            completed=[]
            for entry in rows:
                parameter=entry['params'][0]
                value=parameter.get('baseline',parameter.get('value'));path=encoded[entry['id']]
                if family=='simd':
                    value=next((entry.get('oracle_arguments',{}).get(key)
                                for key in ('n','case','index','vectors','value')
                                if key in entry.get('oracle_arguments',{})),None)
                    need(value is not None,'SIMD independent oracle argument missing')
                observations=[]
                for stage in entry['stages']:
                    if entry.get('oracle',{}).get('module')=='run.py':
                        checked=compare.observation(binary,path,entry['id'],value,
                                                    original_workloads.oracle(entry['id'],value),stage,1,'decoded',cpu)
                    elif family=='scalar':
                        checked=compare.scalar_observation(binary,path,entry,value,scalar.oracle(entry['id'],value),stage,1,'decoded',cpu)
                    elif family=='memory':
                        expected=memory.expected_memory(entry['id'],value)
                        checked=compare.memory_observation(binary,path,entry,value,expected,stage,1,'decoded',cpu,memory_module=memory)
                    elif family=='simd':
                        allowed=simd_scaling.allowed_wire_results(entry,simd)
                        checked=compare._simd_observation(binary,path,entry,simd_scaling.typed_input(entry),value,allowed,stage,1,'decoded',simd_scaling.execution_profile(entry),cpu,model=simd,expected_batch=1)
                    else:
                        expected=representative.oracle(entry['id'],value)
                        wire=captured(binary,['--representative-staged',str(path),entry['id'],str(value),
                                              f"i32:{expected['result']}",expected['output_sha256'],stage,'1','decoded','mvp'],cpu)
                        checked=representative_compare.validate_observation(entry,wire,representative,representative_scaling,1,expected_mode='decoded')
                    # Full images have been checked by independent models above;
                    # bind their bytes without retaining repeated 64KiB strings.
                    raw=json.dumps(checked,sort_keys=True).encode()
                    observations.append({'stage':stage,'mode':'decoded','oracle_checked':True,
                                         'checked_observation_sha256':hashlib.sha256(raw).hexdigest(),
                                         'result':checked.get('result'),'profile':checked.get('profile')})
                if family=='memory' and entry['id']=='memory64_i32_extended':
                    compare.memory_boundary_probe(binary,path,entry,memory.expected_memory(entry['id'],memory.PAGE),'decoded',cpu,memory)
                completed.append({'id':entry['id'],'passed':True,'stages':entry['stages'],
                                  'fixture_sha256':sha(path),'observations':observations})
            families[family]={'supported':len(completed),'passed':len(completed),'unsupported':0,'rows':completed}
            print(json.dumps({'family':family,'passed':len(completed)}),flush=True)
    finally:compare.process=original
    source_stable=(before==p123_adapter.harness_identities() and before_binary==sha(binary)
                   and before_encoder==sha(encoder) and before_adapter==sha(adapter/'manifest.json'))
    p123_adapter.verify(adapter)
    report={'schema':'p123-capabilities-v1','task':'P123','status':'passed','timing_evidence':False,
            'source_stable':source_stable,'compile_source_stable':True,
            'baseline_manifest_sha256':sha(frozen/'manifest.json'),'original_binary_sha256':sha(frozen/'binary/wtime-bench'),
            'adapter_manifest_sha256':before_adapter,'binary_sha256':before_binary,
            'adapter':{'path':str(adapter),'manifest_sha256':before_adapter},
            'encoder_sha256':before_encoder,'families':families,
            'source_identities':before,'frozen_core_identities':record['frozen_core_identities'],
            'commands_sha256':sha(commands),'actual_stage_probes':sum(len(x['observations']) for f in families.values() for x in f['rows'])}
    validate_report(report)
    inventory={str(p.relative_to(output)):sha(p) for p in sorted(output.rglob('*')) if p.is_file()}
    (output/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
    with tarfile.open(output/'artifacts.tar.gz','w:gz') as package:
        for name in [*inventory,'inventory.json']:
            package.add(output/name,arcname=name,recursive=False)
    report['artifacts_sha256']=sha(output/'artifacts.tar.gz')
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    return report

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--adapter',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();probe(args.adapter,args.output)
if __name__=='__main__':main()
