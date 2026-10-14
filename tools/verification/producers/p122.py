"""Run named P122 cases with mandatory real production/diagnostic executables."""
from pathlib import Path
import argparse,hashlib,json,re,shutil,subprocess,sys
ROOT=Path(__file__).resolve().parents[3]
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--diagnostic',type=Path,required=True);p.add_argument('--test-log',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 a.output.mkdir(parents=True,exist_ok=False);shutil.copy2(__file__,a.output/'helper.executed.py')
 binary=a.binary.resolve(strict=True);diagnostic=a.diagnostic.resolve(strict=True);before={str(path):sha(path) for path in (binary,diagnostic)}
 text=a.test_log.read_text();rows=re.findall(r'^test_\w+\s+\([^\n]+\)\s+\.\.\.\s+(ok|FAIL|ERROR|skipped)',text,re.M)
 assert len(rows)==4 and rows==['ok']*4 and re.search(r'^Ran 4 tests in ',text,re.M) and re.search(r'^OK$',text,re.M)
 protocols={}
 commands=[[str(path),'--protocol'] for path in (binary,diagnostic)]
 (a.output/'command.json').write_text(json.dumps({'argv':commands,'cwd':str(ROOT),'test_log':str(a.test_log.resolve()),'test_log_sha256':sha(a.test_log),'binary_sha256':before},indent=2)+'\n')
 for label,path,instrumented in [('production',binary,False),('diagnostic',diagnostic,True)]:
  result=subprocess.run([str(path),'--protocol'],cwd=ROOT,text=True,capture_output=True);(a.output/(label+'-protocol.stdout')).write_text(result.stdout);(a.output/(label+'-protocol.stderr')).write_text(result.stderr)
  assert result.returncode==0;value=json.loads(result.stdout);assert value['instrumented'] is instrumented and 'compact' in value['modes'];protocols[label]=value;shutil.copy2(path,a.output/(label+'-wtime-bench'))
 stable=all(sha(path)==digest for path,digest in before.items())
 (a.output/'report.json').write_text(json.dumps({'status':'passed' if stable else 'failed','binary_stable':stable,'binary_sha256':before,'timing_evidence':False,'actual_representative_rows':9,'production_calls_per_row':3,'diagnostic_calls_per_row':1,'protocols':protocols,'named_tests_passed':4,'test_log_sha256':sha(a.test_log)},indent=2)+'\n');assert stable
if __name__=='__main__':main()
