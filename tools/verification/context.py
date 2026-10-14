"""One-operation memoization, change guards, and inexpensive work counters."""
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
import hashlib
import time

_active = ContextVar('verification_operation', default=None)
CHUNK = 1024 * 1024


def fingerprint(path):
    st = Path(path).stat()
    return (st.st_dev, st.st_ino, st.st_mode, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def current():
    return _active.get()


class Operation:
    def __init__(self):
        self.started = time.perf_counter()
        self.finished = None
        self.files = {}
        self.hashes = {}
        self.memo = {}
        self.probes = {}
        self.counters = {}

    def count(self, name, value=1):
        self.counters[name] = self.counters.get(name, 0) + value

    def observe(self, path):
        from .artifacts import need
        path = Path(path).absolute()
        stamp = fingerprint(path)
        need(path not in self.files or self.files[path] == stamp,
             'file changed during verification: ' + str(path))
        self.files[path] = stamp
        return stamp

    def written(self, path):
        # A state-index replacement performed by this operation is intentional.
        path = Path(path).absolute()
        self.files[path] = fingerprint(path)
        self.hashes.pop(path, None)

    def hash(self, path):
        from .artifacts import need
        path = Path(path).resolve(strict=True)
        stamp = self.observe(path)
        self.count('hash_requests')
        if path in self.hashes:
            self.count('hash_cache_hits')
            return self.hashes[path]
        result = hashlib.sha256()
        with path.open('rb') as stream:
            for chunk in iter(lambda: stream.read(CHUNK), b''):
                result.update(chunk)
                self.count('file_bytes_hashed', len(chunk))
        need(fingerprint(path) == stamp, 'file changed while hashing: ' + str(path))
        self.hashes[path] = result.hexdigest()
        self.count('hash_passes')
        return self.hashes[path]

    def cached(self, namespace, key, producer):
        entry = (namespace, key)
        if entry in self.memo:
            self.count(namespace + '_cache_hits')
            return self.memo[entry]
        self.count(namespace + '_computations')
        value = producer()
        self.memo[entry] = value
        return value

    def guard(self):
        from .artifacts import need
        for path, expected in self.files.items():
            need(path.exists() and fingerprint(path) == expected,
                 'file changed during verification: ' + str(path))
        for name, (probe, expected) in self.probes.items():
            need(probe() == expected, 'input membership changed during verification: ' + name)

    def summary(self):
        return {'wall_seconds': (self.finished or time.perf_counter()) - self.started,
                **dict(sorted(self.counters.items()))}


@contextmanager
def operation():
    existing = current()
    if existing is not None:
        yield existing
        return
    value = Operation()
    token = _active.set(value)
    try:
        yield value
        value.guard()
    finally:
        value.finished = time.perf_counter()
        _active.reset(token)
