"""Immutable payloads, with content identity independent of their original paths."""
from __future__ import annotations
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import tempfile
from .context import CHUNK, current, fingerprint


class VerificationError(ValueError):
    pass


def need(condition, message):
    if not condition:
        raise VerificationError(message)


def sha(path):
    if current() is not None:
        return current().hash(path)
    result = hashlib.sha256()
    stamp = fingerprint(path)
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    need(fingerprint(path) == stamp, 'file changed while hashing: ' + str(path))
    return result.hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                    allow_nan=False).encode()).hexdigest()


def parse_json(raw):
    def invalid(value):
        raise VerificationError('non-finite JSON value: ' + value)
    value = json.loads(raw, parse_constant=invalid)
    def finite(node):
        if isinstance(node, float):
            need(math.isfinite(node), 'non-finite JSON number')
        elif isinstance(node, dict):
            for child in node.values():
                finite(child)
        elif isinstance(node, list):
            for child in node:
                finite(child)
    finite(value)
    return value


def read_json(path):
    if current() is not None:
        current().observe(path)
    return parse_json(Path(path).read_text())


def write_new(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def normal(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}


def verify_normal(value):
    need(isinstance(value, dict) and isinstance(value.get('path'), str), 'missing file reference')
    path = Path(value['path']).resolve(strict=True)
    need(path.is_file() and sha(path) == value.get('sha256')
         and path.stat().st_size == value.get('bytes'), 'stale file reference: ' + str(path))
    if 'mode' in value:
        need(oct(path.stat().st_mode & 0o7777) == value['mode'], 'file mode differs: ' + str(path))
    return path


class ArtifactStore:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.objects = self.root / 'objects'
        self.objects.mkdir(parents=True, exist_ok=True)
        need(self.objects.resolve().is_relative_to(self.root), 'object directory escaped store')

    def _path(self, ref):
        need(isinstance(ref, dict) and re.fullmatch(r'[0-9a-f]{64}', str(ref.get('sha256', ''))),
             'invalid artifact identity')
        name = ref.get('path')
        need(isinstance(name, str) and not Path(name).is_absolute(), 'artifact path must be relative')
        try:
            path = (self.root / name).resolve(strict=True)
        except OSError as error:
            raise VerificationError('missing artifact object: ' + name) from error
        need(path.is_relative_to(self.objects.resolve()) and path.is_file(), 'artifact escaped store')
        expected = f"objects/{ref['sha256'][:2]}/{ref['sha256']}" + ('.gz' if ref.get('encoding') == 'gzip' else '')
        need(name == expected and ref.get('encoding') in ('raw', 'gzip'), 'artifact path/encoding differs')
        need(type(ref.get('bytes')) is int and ref['bytes'] >= 0, 'invalid original artifact length')
        need(type(ref.get('stored_bytes')) is int and ref['stored_bytes'] >= 0, 'invalid stored artifact length')
        need(re.fullmatch(r'0o[0-7]{3,4}', str(ref.get('mode', ''))), 'invalid artifact mode')
        return path

    def _consume(self, ref, collect=False):
        path = self._path(ref)
        need(path.stat().st_mode & 0o777 == 0o444, 'stored artifact mode is not immutable')
        stamp = fingerprint(path)
        need(path.stat().st_size == ref['stored_bytes'], 'stored artifact length changed: ' + str(path))
        ctx = current()
        if ctx:
            ctx.observe(path)
        key = (str(path), ref['sha256'], ref['bytes'], ref.get('stored_sha256'),
               ref['stored_bytes'], ref['encoding'])
        if ctx and ('artifact', key) in ctx.memo:
            ctx.count('artifact_cache_hits')
            cached = ctx.memo[('artifact', key)]
            if not collect:
                return None
            if cached is not None:
                return cached
        raw_hash, stored_hash = hashlib.sha256(), hashlib.sha256()
        raw_size = stored_size = 0
        keep = collect or (ctx is not None and ref['bytes'] <= 128 * 1024)
        chunks = [] if keep else None
        class Reader:
            def read(self, size=-1):
                nonlocal stored_size
                data = source.read(size)
                stored_hash.update(data)
                stored_size += len(data)
                if ctx:
                    ctx.count('artifact_stored_bytes_read', len(data))
                return data
        try:
            with path.open('rb') as source:
                reader = Reader()
                stream = gzip.GzipFile(fileobj=reader, mode='rb') if ref['encoding'] == 'gzip' else reader
                try:
                    while True:
                        chunk = stream.read(min(CHUNK, ref['bytes'] - raw_size + 1))
                        if not chunk:
                            break
                        raw_size += len(chunk)
                        need(raw_size <= ref['bytes'], 'artifact decompression exceeds recorded length')
                        raw_hash.update(chunk)
                        if ctx:
                            ctx.count('artifact_raw_bytes_verified', len(chunk))
                        if chunks is not None:
                            chunks.append(chunk)
                finally:
                    if ref['encoding'] == 'gzip':
                        stream.close()
        except (OSError, EOFError) as error:
            raise VerificationError('invalid compressed artifact: ' + str(path)) from error
        need(fingerprint(path) == stamp, 'artifact changed during verification: ' + str(path))
        need(stored_size == ref['stored_bytes'] and stored_hash.hexdigest() == ref.get('stored_sha256'),
             'stored artifact changed: ' + str(path))
        need(raw_size == ref['bytes'] and raw_hash.hexdigest() == ref['sha256'],
             'original artifact identity differs: ' + str(path))
        raw = b''.join(chunks) if chunks is not None else None
        if ctx:
            ctx.count('artifact_verifications')
            ctx.memo[('artifact', key)] = raw if ref['bytes'] <= 128 * 1024 else None
            ctx.hashes[path] = stored_hash.hexdigest()
        return raw if collect else None

    def read(self, ref):
        return self._consume(ref, collect=True)

    def verify(self, ref):
        self._consume(ref)

    def json(self, ref):
        return parse_json(self.read(ref))

    def _validate(self, path, expected):
        if isinstance(expected, str):
            need(re.fullmatch(r'[0-9a-f]{64}', expected), 'invalid expected digest')
            expected = {'sha256': expected}
        need(expected is None or isinstance(expected, dict), 'invalid expected artifact identity')
        path = Path(path).resolve(strict=True)
        need(path.is_file(), 'artifact is not a file: ' + str(path))
        info = {'sha256': sha(path), 'bytes': path.stat().st_size,
                'mode': oct(path.stat().st_mode & 0o7777)}
        for key in ('sha256', 'bytes', 'mode'):
            if expected and key in expected:
                expected_value = oct(expected[key]) if key == 'mode' and type(expected[key]) is int else expected[key]
                need(info[key] == expected_value, 'artifact ' + key + ' differs: ' + str(path))
        return path, info

    def add(self, path, expected=None, *, _validated=None):
        if _validated is None:
            stamp = fingerprint(path)
            path, info = self._validate(path, expected)
            need(fingerprint(path) == stamp, 'artifact changed during preflight: ' + str(path))
        else:
            path, info, stamp = _validated
            need(fingerprint(path) == stamp, 'artifact changed after storage preflight: ' + str(path))
        with path.open('rb') as stream:
            encoding = 'gzip' if stream.read(4) == b'\x7fELF' else 'raw'
        relative = f"objects/{info['sha256'][:2]}/{info['sha256']}" + ('.gz' if encoding == 'gzip' else '')
        dest = self.root / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        need(dest.parent.resolve().is_relative_to(self.objects.resolve()), 'object destination escaped store')
        stored_digest = None
        if not dest.exists():
            fd, temp_name = tempfile.mkstemp(prefix='.payload-', dir=dest.parent)
            temp = Path(temp_name)
            try:
                with os.fdopen(fd, 'wb') as output, path.open('rb') as source:
                    encoded_hash = hashlib.sha256()
                    class Writer:
                        def write(self, data):
                            written = output.write(data)
                            encoded_hash.update(data[:written])
                            return written
                        def flush(self):
                            return output.flush()
                    encoded = Writer()
                    need(fingerprint(path) == stamp, 'artifact changed before copy: ' + str(path))
                    copied_hash, copied_size = hashlib.sha256(), 0
                    def copy_to(destination):
                        nonlocal copied_size
                        for chunk in iter(lambda: source.read(CHUNK), b''):
                            destination.write(chunk)
                            copied_hash.update(chunk)
                            copied_size += len(chunk)
                            if current():
                                current().count('artifact_source_bytes_copied', len(chunk))
                    if encoding == 'gzip':
                        with gzip.GzipFile(fileobj=encoded, mode='wb', filename='', mtime=0, compresslevel=6) as compressed:
                            copy_to(compressed)
                    else:
                        copy_to(encoded)
                    need(copied_hash.hexdigest() == info['sha256'] and copied_size == info['bytes']
                         and fingerprint(path) == stamp, 'artifact changed during copy: ' + str(path))
                os.chmod(temp, 0o444)
                # link is exclusive and atomic; a concurrent writer may have stored it first.
                try:
                    os.link(temp, dest)
                    stored_digest = encoded_hash.hexdigest()
                except FileExistsError:
                    pass
            finally:
                temp.unlink(missing_ok=True)
        result = {**info, 'path': relative, 'encoding': encoding,
                  'stored_sha256': stored_digest or sha(dest), 'stored_bytes': dest.stat().st_size}
        self.verify(result)
        need(fingerprint(path) == stamp,
             'artifact changed during storage: ' + str(path))
        return result

    def plan(self, bindings):
        need(isinstance(bindings, dict), 'artifact bindings must be an object')
        validated = {}
        for name, binding in bindings.items():
            need(isinstance(name, str) and re.fullmatch(r'[A-Za-z0-9_.:/-]+', name)
                 and '..' not in name.split('/') and not name.startswith('/'), 'invalid logical artifact name')
            need(isinstance(binding, dict) and isinstance(binding.get('path'), str), 'invalid artifact binding')
            stamp = fingerprint(binding['path'])
            path, info = self._validate(binding['path'], binding)
            need(fingerprint(path) == stamp, 'artifact changed during preflight: ' + str(path))
            validated[name] = (path, info, stamp)
        # No payload is copied until every binding has passed preflight.
        return {name: self.add(path, identity, _validated=(path, identity, stamp))
                for name, (path, identity, stamp) in validated.items()}

    def materialize(self, ref, destination):
        raw = self.read(ref)
        destination = Path(destination)
        need(not destination.exists(), 'materialization refuses an existing destination')
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open('xb') as stream:
            stream.write(raw)
        os.chmod(destination, int(ref['mode'], 8))
        need(sha(destination) == ref['sha256'] and destination.stat().st_size == ref['bytes'],
             'materialized artifact differs')
        return destination

    def normal_ref(self, ref):
        self.verify(ref)
        need(ref['encoding'] == 'raw', 'compressed object has no raw file reference')
        return normal(self.root / ref['path'])
