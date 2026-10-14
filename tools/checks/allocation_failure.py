#!/usr/bin/env python3
"""Force a real recoverable allocation failure in a process-isolated constructor."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import resource
import subprocess
import hardening

def collect(worker,output):
    before=hardening.source_hash()
    def limits():
        resource.setrlimit(resource.RLIMIT_AS,(128*1024*1024,)*2)
        resource.setrlimit(resource.RLIMIT_CPU,(5,6))
        resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    command=[str(worker.resolve())]
    try:
        result=subprocess.run(command,cwd=hardening.ROOT,text=True,capture_output=True,timeout=10,preexec_fn=limits)
        data=json.loads(result.stdout)
        passed=result.returncode==0 and data.get('allocator_failed') is True and data.get('recovery_passed') is True and data.get('executed')==1
        code=result.returncode;stdout=result.stdout;stderr=result.stderr
    except (subprocess.TimeoutExpired,ValueError,OSError) as error:
        passed=False;data={};code=None;stdout='';stderr=str(error)
    stable=hardening.source_hash()==before
    record={'passed':passed and stable,'source_stable':stable,'source_sha256':before,'architecture':platform.machine(),
            'command':command,'worker_sha256':hashlib.sha256(worker.read_bytes()).hexdigest() if worker.is_file() else None,'address_space_bytes':128*1024*1024,
            'requested_allocation_bytes':512*1024*1024,'executed':data.get('executed',0),'exit_code':code,'stdout':stdout,'stderr':stderr}
    output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(record,indent=2)+'\n')
    return record['passed']
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--worker',type=Path,default=hardening.ROOT/'target/debug/wtime-allocation-check');parser.add_argument('--report',type=Path,default=hardening.ROOT/'docs/verification/allocation-failure.json');args=parser.parse_args()
    return 0 if collect(args.worker,args.report) else 1
if __name__=='__main__':raise SystemExit(main())
