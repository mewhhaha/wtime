"""Check the immutable P045 predecessor; never rebuild from live sources."""
from pathlib import Path
import argparse, hashlib, json, shutil, sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
import baseline

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    shutil.copy2(__file__, args.output.parent / 'freeze-helper.executed.py')
    archive = ROOT / 'target/performance/P045-before'
    manifest = baseline.verify_archive(archive)
    original = baseline.verify_archive(ROOT / 'target/performance/baseline')
    proof = json.loads((ROOT / 'target/performance/P045/predeclaration/attempt-0001/receipt.json').read_text())
    sources = {v['source_path']: v['sha256'] for v in manifest['files'].values()
               if v.get('source_path', '').startswith('crates/wtime-runtime/src/')}
    assert manifest['mode'] == 'compact' and len(sources) == 31
    assert proof['status'] == 'passed' and proof['source_stable'] and proof['runtime_source_map'] == sources
    assert hashlib.sha256((archive / 'manifest.json').read_bytes()).hexdigest() == proof['baseline_manifest']['sha256']
    with args.output.open('x') as out:
        json.dump({'status': 'passed', 'runtime_source_map': sources, 'original_files': len(original['files']),
                   'predeclaration': proof, 'runtime_file_count': len(sources)}, out, indent=2)
        out.write('\n')
if __name__ == '__main__': main()
