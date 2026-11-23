#!/usr/bin/env python3
"""Run a focused Cargo gate and retain actual positive-count test evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import hardening

def implementation_sources():
    root=hardening.ROOT
    paths=[root/'Cargo.toml',root/'Cargo.lock',*root.glob('crates/**/*.rs'),*root.glob('crates/**/Cargo.toml'),*root.glob('tools/wtime-conformance/**/*.rs'),root/'tools/wtime-conformance/Cargo.toml']
    return {str(path.relative_to(root)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(set(paths))}

def collect(command, report):
    before=hardening.source_hash()
    sources=implementation_sources()
    result=subprocess.run(command,cwd=hardening.ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    counts=[int(n) for n in re.findall(r'test result: ok\. (\d+) passed;',result.stdout)]
    stable=before==hardening.source_hash() and sources==implementation_sources()
    record={'command':command,'source_sha256':before,'source_stable':stable,
            'executed':sum(counts),'passed':result.returncode==0 and sum(counts)>0 and stable,
            'exit_code':result.returncode,'output':result.stdout}
    record['implementation_sources']=sources
    report.parent.mkdir(parents=True,exist_ok=True)
    report.write_text(json.dumps(record,indent=2)+'\n')
    return record['passed']

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--package',required=True);parser.add_argument('--test',default='');parser.add_argument('--filter',default='');parser.add_argument('--report',required=True,type=Path)
    args=parser.parse_args()
    command=['cargo','test','--locked','-p',args.package]
    if args.test:command.extend(['--test',args.test])
    if args.filter:command.append(args.filter)
    return 0 if collect(command,args.report) else 1
if __name__=='__main__':raise SystemExit(main())
