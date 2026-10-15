"""Check P072's restored core and its freshly built production artifact."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}

def check(production, output):
    sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
    import baseline
    archive = ROOT / 'target/performance/P072-before'
    frozen = baseline.verify_archive(archive)
    assert sha(archive / 'manifest.json') == '995a4cb27dfd9fe78c59f6127848ddf6e0988ce3a3f8c7ba5a03a898d657c912'
    sources = {}
    for name, row in frozen['files'].items():
        logical = row.get('source_path', '')
        if row.get('role') == 'source' and (logical.startswith('crates/')
                or logical in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')):
            path = (ROOT / logical).resolve(strict=True)
            assert path.is_relative_to(ROOT) and sha(path) == row['sha256'], logical
            sources[logical] = row['sha256']
    assert sources and 'crates/wtime-runtime/src/continuation.rs' in sources
    production = Path(production).resolve(strict=True)
    record = json.loads(production.read_text())
    binary = Path(record['binary_path']).resolve(strict=True)
    assert record['source_stable'] is True and record['exit_code'] == 0
    assert sha(binary) == record['binary_sha256']
    assert all(record['core_source_map'].get(name) == digest
               for name, digest in sources.items() if '/src/' in name and name.endswith('.rs')
               and not name.startswith('crates/wtime-cli/'))
    result = {'schema': 'wtime-p072-restoration-v1', 'status': 'passed',
              'restored_core_matches_predecessor': True, 'runtime_change_retained': False,
              'timing_evidence': False, 'predecessor': ref(archive / 'manifest.json'),
              'restored_sources': sources, 'production_record': ref(production),
              'production_binary': ref(binary),
              'candidate_completion': ref(ROOT / 'target/performance/P072/shared-finalization/attempt-0003/completion.json'),
              'restore_operation': ref(ROOT / 'target/recovery/P072-restoration-0001/result.json')}
    output = Path(output).resolve()
    assert output.is_relative_to((ROOT / 'target/verification').resolve()) and not output.exists()
    with output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--production', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({'status': check(args.production, args.output)['status']}))
