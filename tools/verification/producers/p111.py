"""Bind the separately built default CLI and finalize its CI lane evidence."""
from pathlib import Path
import argparse
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'tools'),str(ROOT/'tools/checks'),str(Path(__file__).resolve().parent)]
from verification.artifacts import need,normal,read_json,write_new
import performance_ci


def freeze(output):
    write_new(output,{'status':'passed','core_sources':performance_ci.current_core_sources(ROOT),'timing_evidence':False})


def bind(binary,output):
    write_new(output,{'schema':performance_ci.BUILD_SCHEMA,'build_argv':performance_ci.BUILD_ARGV,
        'build_exit_code':0,'binary':normal(binary),'core_sources':performance_ci.current_core_sources(ROOT)})


def finalize(verify_dir,output):
    smoke=read_json(verify_dir/'smoke.json')
    performance_ci.validate_smoke_report(smoke,ROOT)
    reference=read_json(verify_dir/'reference.json')
    need(reference['status']=='unrun' and reference['decision']=='unrun','undeclared reference promotion')
    prerequisites={}
    for task in ('P108','P109','P110'):
        path=ROOT/f'docs/benchmarks/performance/{task}-completion.json';record=read_json(path)
        need(record.get('task')==task and record.get('status')=='done','CI prerequisite incomplete')
        prerequisites[task]=normal(path)
    receipt=read_json(verify_dir/'receipt.json')
    need(receipt.get('status')=='passed' and receipt.get('lane')=='full','CI checks did not pass')
    write_new(output,{'schema':'wtime-p111-ci-v1','task':'P111','status':'passed','timing_evidence':False,
        'core_sources':performance_ci.current_core_sources(ROOT),
        'smoke':normal(verify_dir/'smoke.json'),'reference':normal(verify_dir/'reference.json'),
        'prerequisites':prerequisites,'verification_receipt':normal(verify_dir/'receipt.json'),
        'limitations':['Shared-cloud smoke proves correctness and identity only; dedicated performance comparison is explicitly unrun.']})


def main():
    p=argparse.ArgumentParser();p.add_argument('phase',choices=['freeze','bind','finalize'])
    p.add_argument('--binary',type=Path);p.add_argument('--verify-dir',type=Path);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    if a.phase=='freeze':freeze(a.output)
    elif a.phase=='bind':bind(a.binary,a.output)
    else:finalize(a.verify_dir,a.output)
if __name__=='__main__':main()
