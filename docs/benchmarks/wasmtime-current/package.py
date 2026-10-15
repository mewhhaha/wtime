#!/usr/bin/env python3
"""Package the completed P146 Wasmtime comparison without remeasurement."""
from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import tarfile

ROOT = Path(__file__).resolve().parents[3]
PACKAGE = ROOT / 'docs/benchmarks/wasmtime-current'
RECOVERY = ROOT / 'target/recovery/wasmtime-current-package-0001'
RUN = ROOT / 'target/verification/runs/P146'
COMPLETION = ROOT / 'target/verification/P146-completion-0001'
CAPTURES = ROOT / 'target/recovery/wasmtime-current-0001'
CONTRACT = ROOT / 'docs/benchmarks/wasmtime-current-verification.json'


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def ref(path: Path, *, local: str | None = None) -> dict:
    return {'path': local if local is not None else str(path),
            'sha256': digest(path), 'bytes': path.stat().st_size}


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n', encoding='utf-8')


def copy_exact(source: Path, filename: str, copied: list[dict]) -> Path:
    destination = PACKAGE / filename
    shutil.copyfile(source, destination)
    if source.read_bytes() != destination.read_bytes():
        raise RuntimeError(f'copy mismatch: {source}')
    copied.append({'file': filename, 'source': str(source),
                   'sha256': digest(destination), 'bytes': destination.stat().st_size})
    return destination


def archive_inputs() -> dict[str, Path]:
    members: dict[str, Path] = {}

    def add(path: Path):
        if not path.is_file():
            return
        resolved = path.resolve(strict=True)
        try:
            name = resolved.relative_to(ROOT).as_posix()
        except ValueError as exc:
            raise RuntimeError(f'archive input outside repository: {resolved}') from exc
        if name in members:
            raise RuntimeError(f'duplicate archive path: {name}')
        members[name] = resolved

    excluded = {
        'target/verification/runs/P146/verify/attempt-0001/bin/decoded',
        'target/verification/runs/P146/verify/attempt-0001/bin/compact',
        'target/verification/runs/P146/verify/attempt-0001/bin/register',
    }
    for path in RUN.rglob('*'):
        if path.is_file() and path.relative_to(ROOT).as_posix() not in excluded:
            add(path)
    for path in COMPLETION.rglob('*'):
        add(path)
    add(CONTRACT)
    helpers = [
        ROOT/'tools/wtime-bench/run.py',
        ROOT/'tools/wtime-bench/src/main.rs',
        ROOT/'tools/wtime-bench/wasmtime_current.py',
        ROOT/'tools/wtime-bench/wasmtime_report.py',
        ROOT/'tools/wtime-bench/bootstrap.py',
        ROOT/'tools/wtime-bench/tests/test_wasmtime_runner.py',
        ROOT/'tools/wtime-bench/tests/test_wasmtime_report.py',
        ROOT/'tools/verify.py',
    ]
    helpers += sorted((ROOT/'tools/verification').glob('*.py'))
    for path in helpers:
        add(path)
    for path in CAPTURES.rglob('*'):
        add(path)
    add(RECOVERY/'package.py')
    add(RECOVERY/'command.json')
    return members


def create_archive(members: dict[str, Path], filemap_path: Path) -> list[dict]:
    entries = [{'path': name, 'sha256': digest(path), 'bytes': path.stat().st_size}
               for name, path in sorted(members.items())]
    archive_path = PACKAGE/'provenance-records.tar.gz'
    with archive_path.open('xb') as raw:
        with gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|', format=tarfile.GNU_FORMAT) as archive:
                for entry in entries:
                    path = members[entry['path']]
                    info = tarfile.TarInfo(entry['path'])
                    info.size = entry['bytes']
                    info.mode = 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ''
                    info.mtime = 0
                    with path.open('rb') as content:
                        archive.addfile(info, content)
    write_json(filemap_path, {'schema': 'wasmtime-current-archive-filemap-v1',
                              'member_count': len(entries), 'members': entries})
    return entries


def validate_archive(archive_path: Path, expected: list[dict]) -> None:
    wanted = {row['path']: row for row in expected}
    observed = {}
    with archive_path.open('rb') as raw:
        with gzip.GzipFile(fileobj=raw, mode='rb') as compressed:
            with tarfile.open(fileobj=compressed, mode='r|') as archive:
                for member in archive:
                    if not member.isfile() or member.name in observed:
                        raise RuntimeError(f'invalid archive member: {member.name}')
                    content = archive.extractfile(member)
                    if content is None:
                        raise RuntimeError(f'missing archive data: {member.name}')
                    h = hashlib.sha256()
                    count = 0
                    while True:
                        block = content.read(1024 * 1024)
                        if not block:
                            break
                        count += len(block)
                        h.update(block)
                    observed[member.name] = {'path': member.name,
                                             'sha256': h.hexdigest(), 'bytes': count}
    if observed != wanted:
        raise RuntimeError('streamed archive membership or content identity mismatch')


def main() -> None:
    if PACKAGE.exists() and any(path.name != 'README.md' for path in PACKAGE.iterdir()):
        raise RuntimeError('package output directory contains prior package artifacts')
    PACKAGE.mkdir(parents=True, exist_ok=True)
    copied: list[dict] = []
    measurement = RUN/'measure/attempt-0001'
    verification = RUN/'verify/attempt-0001'
    copies = [
        (RUN/'freeze/attempt-0001/freeze.json', 'source-freeze.json'),
        (verification/'decoded.json', 'build-decoded.json'),
        (verification/'compact.json', 'build-compact.json'),
        (verification/'register.json', 'build-register.json'),
        (verification/'verify.json', 'verify.json'),
        (measurement/'manifest.json', 'measurement-manifest.json'),
        (RUN/'analyze/attempt-0001/analysis.json', 'analysis.json'),
        (RUN/'analyze/attempt-0001/results.md', 'results.md'),
        (COMPLETION/'completion.json', 'completion.json'),
        (COMPLETION/'evidence.json', 'evidence.json'),
        (CONTRACT, 'contract.json'),
    ]
    for source, filename in copies:
        copy_exact(source, filename, copied)
    report_rows = []
    for tier in ('decoded', 'compact', 'register-eager', 'register-adaptive-3'):
        for run in (1, 2):
            filename = f'{tier}-run{run}.json'
            copied_path = copy_exact(measurement/filename, filename, copied)
            report_rows.append({'tier': tier, 'run': run, **ref(copied_path, local=filename)})
    write_json(PACKAGE/'raw-filemap.json', {
        'schema': 'wasmtime-current-raw-filemap-v1',
        'source_manifest': {'path': str(measurement/'manifest.json'),
                            'sha256': digest(measurement/'manifest.json'),
                            'bytes': (measurement/'manifest.json').stat().st_size},
        'copied_files': copied,
        'measurement_manifest_copy_unchanged':
            digest(PACKAGE/'measurement-manifest.json') == digest(measurement/'manifest.json'),
    })
    binaries = []
    for tier, filename in (('decoded', 'decoded'), ('compact', 'compact'), ('register', 'register')):
        binding = json.loads((verification/(filename+'.json')).read_text(encoding='utf-8'))
        binary = Path(binding['binary']['path']).resolve(strict=True)
        actual = ref(binary)
        if actual['sha256'] != binding['binary']['sha256'] or actual['bytes'] != binding['binary']['bytes']:
            raise RuntimeError(f'{tier} executable identity differs from retained build record')
        binaries.append({'tier': tier, **actual, 'payload_in_archive': False})
    write_json(PACKAGE/'binary-identities.json', {
        'schema': 'wasmtime-current-binary-identities-v1',
        'note': 'Retained ELF executables are identified by hash and byte count; payloads are excluded.',
        'binaries': binaries,
    })
    shutil.copyfile(RECOVERY/'package.py', PACKAGE/'package.py')
    shutil.copyfile(RECOVERY/'command.json', PACKAGE/'package-command.json')
    members = archive_inputs()
    filemap_path = PACKAGE/'archive-filemap.json'
    entries = create_archive(members, filemap_path)
    validate_archive(PACKAGE/'provenance-records.tar.gz', entries)
    validation = {
        'status': 'passed', 'timing_performed': False, 'build_performed': False,
        'copied_files_exact': all(row['sha256'] == digest(PACKAGE/row['file'])
                                  and row['bytes'] == (PACKAGE/row['file']).stat().st_size
                                  for row in copied),
        'measurement_manifest_unchanged':
            digest(PACKAGE/'measurement-manifest.json') == digest(measurement/'manifest.json'),
        'archive_streamed_gzip_tar_membership_and_hashes': 'passed',
        'archive_members': len(entries),
        'archive_sha256': digest(PACKAGE/'provenance-records.tar.gz'),
        'archive_bytes': (PACKAGE/'provenance-records.tar.gz').stat().st_size,
        'cas_objects_rehydrated': False,
        'cas_note': 'Original phase receipts are retained in the run tree and archive; their referenced CAS object store was not rehydrated.',
    }
    write_json(PACKAGE/'validation.json', validation)
    manifest = {
        'schema': 'wasmtime-current-portable-package-v1',
        'informational_only': True,
        'tiers': ['decoded', 'compact', 'register-eager', 'register-adaptive-3'],
        'runs_per_tier': 2,
        'reports': report_rows,
        'artifacts': {name: ref(PACKAGE/filename, local=filename) for name, filename in {
            'source_freeze': 'source-freeze.json', 'build_decoded': 'build-decoded.json',
            'build_compact': 'build-compact.json', 'build_register': 'build-register.json',
            'verification': 'verify.json', 'measurement_manifest': 'measurement-manifest.json',
            'analysis': 'analysis.json', 'results': 'results.md', 'completion': 'completion.json',
            'evidence': 'evidence.json', 'contract': 'contract.json',
            'raw_filemap': 'raw-filemap.json', 'binary_identities': 'binary-identities.json',
            'archive': 'provenance-records.tar.gz', 'archive_filemap': 'archive-filemap.json',
            'validation': 'validation.json', 'packaging_helper': 'package.py',
            'packaging_command': 'package-command.json',
        }.items()},
        'validation': validation,
        'cas': {'objects_rehydrated': False,
                'disclosure': 'Receipts are included by their original bytes; referenced CAS objects are not included or claimed rehydrated.'},
    }
    write_json(PACKAGE/'manifest.json', manifest)
    if not validation['copied_files_exact']:
        raise RuntimeError('exact copy validation failed')
    if digest(PACKAGE/'measurement-manifest.json') != digest(measurement/'manifest.json'):
        raise RuntimeError('measurement manifest copy changed')
    print(json.dumps({'status': 'passed', 'package': str(PACKAGE),
                      'archive_members': len(entries),
                      'archive_sha256': validation['archive_sha256'],
                      'report_count': len(report_rows)}, sort_keys=True))


if __name__ == '__main__':
    main()
