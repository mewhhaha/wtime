"""Small source-bound integration producers; no performance decision here."""
from pathlib import Path
import argparse, hashlib, json, os, shutil, subprocess, traceback
import integration_workloads as model

ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def ref(path):
    path=Path(path).resolve(strict=True)
    return {'path':str(path),'sha256':sha(path),'bytes':path.stat().st_size}
def write(path,value):
    with Path(path).open('x') as out: json.dump(value,out,indent=2,sort_keys=True);out.write('\n')
def runtime_map(): return {p.relative_to(ROOT).as_posix():sha(p) for p in sorted((ROOT/'crates/wtime-runtime/src').rglob('*.rs'))}
def probe(binary,output):
    output.mkdir(parents=True,exist_ok=False)
    shutil.copy2(__file__,output/'helper.executed.py')
    shutil.copy2(HERE/'integration_workloads.py',output/'model.executed.py')
    before=runtime_map(); binary_before=ref(binary)
    registry=json.loads((HERE/'workloads.json').read_text()); rows={r['id']:r for r in registry['workloads']}
    cpu=min(os.sched_getaffinity(0)); results=[]
    for case in model.SELECTED:
        row=rows[case];wat=HERE/row['source']['path'];wasm=wat.with_suffix('.wasm')
        assert sha(wat)==row['source']['sha256'] and sha(wasm)==row['source']['wasm_sha256']
        argv=[str(binary), '--integration-driver', str(wasm),case,'first_call',str(model.INPUT),'1','compact','--probe']
        write(output/(case+'.command.json'),{'argv':argv,'cwd':str(ROOT),'cpu':cpu})
        with (output/(case+'.stdout.json')).open('xb') as stdout,(output/(case+'.stderr')).open('xb') as stderr:
            process=subprocess.run(argv,cwd=ROOT,stdout=stdout,stderr=stderr,preexec_fn=lambda:os.sched_setaffinity(0,{cpu}))
        assert process.returncode==0, f'{case}: process failed; inspect archived stderr'
        wire=json.loads((output/(case+'.stdout.json')).read_text())
        assert wire.get('timing_excluded') is True and wire.get('diagnostic') is False
        verified=model.validate_preflight(wire,case,model.INPUT)
        results.append({'id':case,'status':'passed','argv':argv,'exit_code':process.returncode,
            'result':verified['result'],'memory_sha256':verified['memory_sha256'],
            'stdout':ref(output/(case+'.stdout.json')),'stderr':ref(output/(case+'.stderr')),
            'wat':ref(wat),'wasm':ref(wasm),'lifecycle':wire['lifecycle']})
    assert before==runtime_map() and binary_before==ref(binary)
    write(output/'report.json',{'schema':'wtime-integration-preflight-v1','status':'passed','source_stable':True,
        'case_count':len(results),'timing_evidence':False,'instrumented':False,'mode':'compact',
        'binary':binary_before,'runtime_source_map':before,'model':ref(output/'model.executed.py'),'cases':results})
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['probe'])
    p.add_argument('--binary',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--execute',action='store_true')
    args=p.parse_args()
    if not args.execute: print('inert: pass --execute');return 0
    try: probe(args.binary.resolve(strict=True),args.output.resolve());return 0
    except BaseException:
        if args.output.exists(): (args.output/'failure.txt').write_text(traceback.format_exc())
        raise
if __name__=='__main__': raise SystemExit(main())
