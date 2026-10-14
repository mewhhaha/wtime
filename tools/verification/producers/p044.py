"""P044 predecessor integrity and relative build decision; no timing here."""
from pathlib import Path
import argparse,hashlib,json,shutil,sys
sys.dont_write_bytecode=True
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import baseline
METRICS=('clean_seconds','leaf_seconds','runtime_leaf_seconds','release_seconds','stripped_bytes','clean_peak_rss_kib')
def read(path):return json.loads(Path(path).read_text())
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def ref(path):
    path=Path(path).resolve(strict=True);return {'path':str(path),'sha256':sha(path),'bytes':path.stat().st_size}
def write(path,value):
    with Path(path).open('x') as out:json.dump(value,out,indent=2,sort_keys=True);out.write('\n')
def freeze(output):
    archive=ROOT/'target/performance/P044-before';manifest=baseline.verify_archive(archive)
    original=baseline.verify_archive(ROOT/'target/performance/baseline')
    sources={v['source_path']:v['sha256'] for v in manifest['files'].values() if v.get('source_path','').startswith('crates/wtime-runtime/src/')}
    declaration=ROOT/'target/performance/P044/predeclaration/attempt-0001/receipt.json';proof=read(declaration)
    assert manifest['mode']=='compact' and len(sources)==30 and proof['status']=='passed' and proof['source_stable'] and proof['runtime_source_map']==sources
    assert sha(archive/'manifest.json')==proof['baseline_manifest']['sha256']
    # Analysis runs after measurement and owns its own identity. All executable
    # measurement helpers, native driver and six fixture bytes match the archive.
    bench=ROOT/'tools/wtime-bench'
    harness=[bench/'Cargo.toml',bench/'Cargo.lock',bench/'workloads.json']
    harness+=list((bench/'src').rglob('*.rs'))
    harness+=[p for p in bench.rglob('*.py') if 'target' not in p.relative_to(bench).parts and 'tests' not in p.relative_to(bench).parts and p.name not in ('analyze.py','gates.py')]
    harness+=list((bench/'fixtures/integration_p113').glob('*'))
    assert all(manifest['files']['source/'+p.relative_to(ROOT).as_posix()]['sha256']==sha(p) for p in harness if p.is_file())
    budgets={}
    for name in ('build','optional-build'):
        path=ROOT/f'target/verification/runs/P043/verify/attempt-0004/{name}.json';value=read(path)
        assert value['source_stable'] and value['budget_pass']
        assert all(value['source_inputs'].get(key)==digest for key,digest in sources.items())
        budgets[name]=ref(path)
    write(output,{'status':'passed','runtime_source_map':sources,'runtime_file_count':len(sources),'harness_stable':True,
        'baseline_manifest':ref(archive/'manifest.json'),'predeclaration':ref(declaration),
        'predecessor_budgets':budgets,'original_archive_files':len(original['files'])})
def decide(statistics,verify_dir,freeze_dir,output):
    record=read(statistics);assert record['status']=='passed' and record['statistics_computations']==1
    predecessor=read(freeze_dir/'predecessor.json');ratios={};inputs={}
    for name in ('build','optional-build'):
        prior=predecessor['predecessor_budgets'][name];assert ref(prior['path'])==prior
        old=read(prior['path']);new=read(verify_dir/(name+'.json'));assert new['source_stable'] and new['budget_pass']
        ratios[name]={metric:new[metric]/old[metric] for metric in METRICS};inputs[name]=ref(verify_dir/(name+'.json'))
    growth=all(r<=1.25 for mode in ratios.values() for r in mode.values());decision=record['decision']['decision'];reasons=list(record['decision']['reasons'])
    if not growth:
        if decision=='adopted':decision='inconclusive'
        reasons.append('Immediate-predecessor build growth exceeds 1.25; promotion withheld.')
    write(output,{'status':'passed','decision':decision,'statistics':ref(statistics),'build_inputs':inputs,
        'predecessor':ref(freeze_dir/'predecessor.json'),'immediate_predecessor_build_ratios':ratios,
        'build_growth_pass':growth,'reasons':reasons,'default_policy':'decoded','compact_opt_in':True})
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['freeze','decide']);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--statistics',type=Path);p.add_argument('--verify-dir',type=Path);p.add_argument('--freeze-dir',type=Path);p.add_argument('--execute',action='store_true');a=p.parse_args()
    if not a.execute:print('inert: pass --execute');return
    shutil.copy2(__file__,a.output.parent/(a.action+'-helper.executed.py'))
    if a.action=='freeze':freeze(a.output)
    else:decide(a.statistics,a.verify_dir,a.freeze_dir,a.output)
if __name__=='__main__':main()
