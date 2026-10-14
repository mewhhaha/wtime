#!/usr/bin/env python3
"""Collect native runtime evidence and reject incomplete release evidence."""
import argparse
import hashlib
import json
import platform
import re
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
def source_hash(root=ROOT):
    digest=hashlib.sha256()
    for path in sorted([root/'Cargo.lock',root/'Cargo.toml',*root.glob('crates/**/*.rs'),*root.glob('crates/**/Cargo.toml')]):
        digest.update(str(path.relative_to(root)).encode()+b'\0'+path.read_bytes()+b'\0')
    return digest.hexdigest()
def collect(path):
    command=['cargo','test','--locked','-p','wtime-runtime','--tests']
    before=source_hash()
    result=subprocess.run(command,cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    counts=[int(n) for n in re.findall(r'test result: ok\. (\d+) passed;',result.stdout)]
    stable=source_hash()==before
    passed=result.returncode==0 and sum(counts)>0 and stable
    record={'architecture':platform.machine(),'source_sha256':before,'source_stable':stable,'command':command,
            'native_execution':True,'passed':passed,'tests_executed':sum(counts),'exit_code':result.returncode,
            'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'output':result.stdout}
    path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(record,indent=2)+'\n')
    return passed

def check_native(reports):
    errors=[];seen=set();hashes=set()
    for report in reports:
        arch={'arm64':'aarch64','AMD64':'x86_64'}.get(report.get('architecture'),report.get('architecture'))
        seen.add(arch);hashes.add(report.get('source_sha256'))
        if report.get('native_execution') is not True: errors.append(f'{arch}: not native execution')
        if report.get('source_stable') is False: errors.append(f'{arch}: source changed during execution')
        if report.get('passed') is not True or report.get('exit_code')!=0 or report.get('tests_executed',0)<=0: errors.append(f'{arch}: unsuccessful or empty tests')
    if not {'x86_64','aarch64'}<=seen: errors.append('both native x86_64 and aarch64 evidence required')
    if len(hashes)!=1 or not all(hashes): errors.append('source digests missing or inconsistent')
    return errors

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--native-report',type=Path);parser.add_argument('reports',type=Path,nargs='*');args=parser.parse_args()
    if args.native_report:return 0 if collect(args.native_report) else 1
    errors=check_native([json.loads(p.read_text()) for p in args.reports]);print('\n'.join(errors) if errors else 'Both native architecture gates passed');return bool(errors)
if __name__=='__main__':raise SystemExit(main())
