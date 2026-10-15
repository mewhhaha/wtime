"""Retain the complete P073 pairs after an analysis-only stage-reader fix."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools'))
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
from verification.artifacts import ArtifactStore, need, sha, verify_normal, write_new
import candidate_gate
import execution_identity
import focused_compare

ORIGINAL = ROOT/'target/verification/runs/P073/measure/attempt-0001'
RECEIPT_SHA = '70a7ed5e1750ba1b34e514f155ad0f02657fdee590b4a087b0b59bc60bfedb99'
REPORT_SHA = '8b756404324f4cdced0efb39d47df151f7275f405da522fa581d25ecc5447a2b'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    need(sha(ORIGINAL/'receipt.json') == RECEIPT_SHA, 'original measurement receipt changed')
    need(sha(ORIGINAL/'comparison/report.json') == REPORT_SHA, 'original paired observations changed')
    receipt = json.loads((ORIGINAL/'receipt.json').read_text())
    need(receipt['status'] == 'passed' and receipt['phase'] == 'measure'
         and receipt['task'] == 'P073' and receipt['lane'] == 'full'
         and receipt['synthetic'] is False, 'original measurement was not full passing evidence')
    store = ArtifactStore(ROOT/'target/verification')
    for artifact in receipt['artifacts'].values(): store.verify(artifact)
    for reference in receipt['original_outputs'].values(): verify_normal(reference)
    report = candidate_gate.validate(ORIGINAL/'comparison/report.json')
    execution_identity.verify(report['metadata']['candidate_sources'], focused_compare.sources(), focused=True)
    need(len(report['runs']) == 2 and sum(len(run['pairs']) for run in report['runs']) == 1200,
         'incomplete retained pairs')
    need([run['seed'] for run in report['runs']] == [73001,73002], 'retained seeds differ')
    args.output.mkdir(parents=True,exist_ok=False)
    shutil.copytree(ORIGINAL/'comparison',args.output/'comparison')
    need(sha(args.output/'comparison/report.json') == REPORT_SHA, 'retained copy differs')
    write_new(args.output/'reuse.json', {'schema':'wtime-p073-measurement-reuse-v1',
        'status':'passed','original_receipt':str(ORIGINAL/'receipt.json'),
        'original_receipt_sha256':RECEIPT_SHA,'report_sha256':REPORT_SHA,
        'unique_pair_count':1200,'guest_processes_started':0,
        'reason':'Only the analysis stage reader changed; all runtime, timer, fixture, binary and selection inputs match.'})
    print('P073 complete paired observations revalidated without retiming')


if __name__ == '__main__': main()
