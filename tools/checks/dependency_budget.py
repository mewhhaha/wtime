#!/usr/bin/env python3
"""Check the production dependency closure and inventory unsafe Rust."""
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]

def violations(metadata, sources):
    packages = {p['id']: p for p in metadata['packages']}
    nodes = {p['id']: p for p in metadata['resolve']['nodes']}
    pending = [p['id'] for p in packages.values() if p['name'] in {
        'wtime-types', 'wtime-decode', 'wtime-validate', 'wtime-runtime', 'wtime-cli'}]
    seen, errors = set(), []
    while pending:
        ident = pending.pop()
        if ident in seen:
            continue
        seen.add(ident)
        package = packages[ident]
        if package.get('source') is not None or package['name'] not in {
            'wtime-types', 'wtime-decode', 'wtime-validate', 'wtime-runtime', 'wtime-cli'}:
            errors.append('external production dependency: ' + package['name'])
        for dep in nodes[ident].get('deps', []):
            if any(k['kind'] != 'dev' for k in dep['dep_kinds']):
                pending.append(dep['pkg'])
    for path, source in sources.items():
        # Conservative inventory includes comments; false positives require review.
        for match in re.finditer(r'\bunsafe\s+(?:\{|fn\b|impl\b|trait\b|extern\b)', source):
            number = source.count('\n', 0, match.start()) + 1
            errors.append(f'unreviewed unsafe: {path}:{number}')
    return errors

def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version=1'], cwd=ROOT))
    sources = {str(p.relative_to(ROOT)): p.read_text() for p in (ROOT/'crates').rglob('*.rs')}
    errors = violations(metadata, sources)
    for error in errors:
        print(error)
    tools = [p for p in metadata['packages'] if p.get('source')]
    for package in tools:
        print(f"tool dependency: {package['name']} {package['version']} license={package.get('license')}")
    print(f'Production dependency/unsafe budget: {len(errors)} violations')
    return bool(errors)

if __name__ == '__main__':
    raise SystemExit(main())
