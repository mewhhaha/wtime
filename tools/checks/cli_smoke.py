#!/usr/bin/env python3
"""Execute CLI success, decode, link and trap outcomes using original fixtures."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import hardening

def main():
    before = hardening.source_hash()
    fixtures = Path(__file__).parent / 'fixtures' / 'cli'
    results = []
    with tempfile.TemporaryDirectory(prefix='wtime-cli-') as directory:
        directory = Path(directory)
        for name in ('add', 'import', 'trap'):
            subprocess.run([str(hardening.ROOT/'target/debug/wtime-conformance'), 'encode',
                            str(fixtures/(name+'.wat')), '-o', str(directory/(name+'.wasm'))], check=True)
        (directory/'invalid.wasm').write_bytes(b'bad')
        for name, expected in [('add', 'I32(42)'), ('invalid', 'Decode:'), ('import', 'Link:'), ('trap', 'Trap:')]:
            command = [str(hardening.ROOT/'target/debug/wtime-cli'), 'run', str(directory/(name+'.wasm')), 'run']
            result = subprocess.run(command, text=True, capture_output=True)
            passed = (result.returncode == 0 and expected in result.stdout) if name == 'add' else (result.returncode != 0 and expected in result.stderr)
            results.append(dict(command=command, exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr, passed=passed))
    stable = before == hardening.source_hash()
    record = dict(source_sha256=before, source_stable=stable, executed=len(results),
                  passed=stable and all(result['passed'] for result in results), results=results,
                  fixtures={str(path.relative_to(hardening.ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(fixtures.glob('*.wat'))})
    (hardening.ROOT/'docs/verification/cli.json').write_text(json.dumps(record, indent=2)+'\n')
    return 0 if record['passed'] else 1

if __name__ == '__main__':
    raise SystemExit(main())
