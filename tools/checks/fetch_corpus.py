#!/usr/bin/env python3
"""Fetch exact corpus commits and verify every selected byte against the lock."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / '.cache'
LOCK = ROOT / 'docs/architecture/corpus-lock.json'

def main():
    selecting = len(sys.argv) == 4 and sys.argv[1] == '--select'
    manifest = json.loads(LOCK.read_text())
    if selecting:
        suite, prefix = sys.argv[2:4]
        selected = manifest['suites'].get(suite)
        if selected is None:
            selected = [f['path'] for f in manifest['files'] if Path(f['path']).stem == suite and f['source'] == 'spec']
        paths = {p for p in selected if prefix == 'all' or Path(p).stem.startswith(prefix)}
        if not paths:
            raise SystemExit('zero selected files or unsupported suite: ' + suite)
    else:
        paths = {f['path'] for f in manifest['files']}
    needed = {f['source'] for f in manifest['files'] if f['path'] in paths}
    for source in manifest['sources']:
        if source['name'] not in needed:
            continue
        if source.get('local'):
            continue
        checkout = CACHE / source['name']
        if not checkout.exists():
            if selecting:
                raise SystemExit('missing corpus; run python3 tools/checks/fetch_corpus.py')
            checkout.mkdir(parents=True)
            subprocess.run(['git', '-C', str(checkout), 'init'], check=True)
            subprocess.run(['git', '-C', str(checkout), 'remote', 'add', 'origin', source['repository']], check=True)
            subprocess.run(['git', '-C', str(checkout), 'fetch', '--depth', '1', 'origin', source['revision']], check=True)
            subprocess.run(['git', '-C', str(checkout), 'checkout', '--detach', source['revision']], check=True)
        revision = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
        if revision != source['revision']:
            raise SystemExit('wrong corpus revision: ' + source['name'])
    for item in manifest['files']:
        if item['path'] not in paths:
            continue
        data = (CACHE / item['path']).read_bytes()
        if hashlib.sha256(data).hexdigest() != item['sha256']:
            raise SystemExit('corpus integrity error: ' + item['path'])
    if selecting:
        for path in sorted(paths):
            print(CACHE / path)
        return
    print(f'verified {len(paths)} pinned files from {len(needed)} repositories')

if __name__ == '__main__':
    main()
