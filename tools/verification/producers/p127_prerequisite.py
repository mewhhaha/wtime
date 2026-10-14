"""Revalidate executed P020/P018/continuation tests against the current core."""
from pathlib import Path
import argparse,json,sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import focused_compare,semantic_receipt
def run(path,output):
 record=json.loads(Path(path).read_text())
 names={n.rsplit('::',1)[-1] for group in record['tests'].values() for n in group['names']}
 required=sorted(n for n in names if n.startswith(('p020_','p018_')) or n=='t145_suspension_carries_remaining_fuel')
 if len(required)!=12:raise ValueError('expected all four P020, seven P018 and t145 fuel cases')
 value=semantic_receipt.validate(path,{'task':'P050','semantic_cases':required},focused_compare.sources(),ROOT)
 output=Path(output);output.parent.mkdir(parents=True,exist_ok=True)
 with output.open('x') as f:json.dump({'status':'passed','required_case_count':12,'semantic_prerequisite':value,'runtime_core_unchanged':True},f,indent=2);f.write('\n')
 print(json.dumps({'status':'passed','source_current_prerequisites':12}))
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--receipt',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 try:run(a.receipt,a.output)
 except (OSError,ValueError,KeyError) as e:print(str(e),file=sys.stderr);raise SystemExit(1)
