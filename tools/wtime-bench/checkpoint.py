"""Crash-tolerant JSONL checkpoint helpers for the P023 comparator.

The journal is intentionally separate from the report schema: a successful final
report is still the comparator's ordinary schema-v2 JSON, containing every pair.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import tempfile

SCHEMA = 'wtime-compare-checkpoint-v1'


class CheckpointError(ValueError):
    pass


def _canonical_hash(value):
    payload = json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()
    return hashlib.sha256(payload).hexdigest()


def _encode_line(value):
    return (json.dumps(value, separators=(',', ':'), ensure_ascii=True) + '\n').encode()


def _fsync_dir(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class Journal:
    """Append/recover pair records while refusing ambiguous or destructive reuse.

    `identity` must bind all sources, binaries, fixtures, protocol, mode/profile,
    selection, and measurement settings that affect an observation. `run_seeds`
    maps numeric run IDs to their deterministic seeds.
    """

    def __init__(self, output, identity, run_seeds, *, resume=False, prepared=False):
        self.output = Path(output)
        self.report_path = self.output / 'report.json'
        self.path = self.output / 'checkpoint.jsonl'
        self.identity = identity
        self.identity_sha256 = _canonical_hash(identity)
        self.run_seeds = {int(k): int(v) for k, v in run_seeds.items()}
        if not self.run_seeds or len(self.run_seeds) != len(run_seeds):
            raise CheckpointError('run seed map is empty or has duplicate run IDs')
        if len(set(self.run_seeds.values())) != len(self.run_seeds):
            raise CheckpointError('run seed map has conflicting seeds')
        self._ids = set()
        self._pairs = {run_id: [] for run_id in self.run_seeds}
        self._closed = False

        if resume:
            if not self.output.is_dir():
                raise CheckpointError('cannot resume: output directory is missing')
            if self.report_path.exists():
                raise CheckpointError('cannot resume: report already exists; refusing to overwrite it')
            if not self.path.is_file():
                raise CheckpointError('cannot resume: checkpoint journal is missing')
            self._read_and_repair()
            self._stream = self.path.open('ab')
        else:
            if self.output.exists():
                if not prepared or not self.output.is_dir():
                    raise CheckpointError('output already exists; refusing to overwrite report or checkpoint')
            else:
                self.output.mkdir(parents=True)
            if self.report_path.exists() or self.path.exists():
                raise CheckpointError('output collision before checkpoint creation')
            header = {
                'type': 'header', 'schema': SCHEMA,
                'identity': identity, 'identity_sha256': self.identity_sha256,
                'run_seeds': {str(k): self.run_seeds[k] for k in sorted(self.run_seeds)},
            }
            # x-mode ensures an unexpected preexisting journal can never be replaced.
            with self.path.open('xb') as stream:
                stream.write(_encode_line(header))
                stream.flush()
                os.fsync(stream.fileno())
            self._stream = self.path.open('ab')

    @staticmethod
    def preflight(output, *, resume=False):
        """Call before any benchmark work or output writes."""
        output = Path(output)
        report = output / 'report.json'
        journal = output / 'checkpoint.jsonl'
        if resume:
            if not output.is_dir():
                raise CheckpointError('cannot resume: output directory is missing')
            if report.exists():
                raise CheckpointError('cannot resume: report already exists; refusing to overwrite it')
            if not journal.is_file():
                raise CheckpointError('cannot resume: checkpoint journal is missing')
        elif output.exists():
            raise CheckpointError('output already exists; refusing to overwrite report or checkpoint')

    def _read_and_repair(self):
        data = self.path.read_bytes()
        if not data:
            raise CheckpointError('checkpoint journal is empty')
        lines = data.splitlines(keepends=True)
        offset = 0
        events = []
        for index, raw in enumerate(lines):
            final = index == len(lines) - 1
            terminated = raw.endswith(b'\n')
            body = raw[:-1] if terminated else raw
            try:
                item = json.loads(body)
            except (ValueError, UnicodeDecodeError) as exc:
                if final and not terminated and index > 0:
                    # Only a malformed, unterminated tail after the complete header can result from a killed append.
                    with self.path.open('r+b') as stream:
                        stream.truncate(offset)
                        stream.flush()
                        os.fsync(stream.fileno())
                    break
                raise CheckpointError(f'malformed complete journal line {index + 1}') from exc
            if not terminated:
                raise CheckpointError('valid JSON checkpoint record lacks its terminating newline')
            if index == 0:
                self._validate_header(item)
            else:
                events.append(item)
            offset += len(raw)
        else:
            if not events and len(lines) == 1:
                # Header-only journal is valid; it has no pair events yet.
                pass
        for index, event in enumerate(events, start=2):
            self._accept_event(event, index)

    def _validate_header(self, item):
        if not isinstance(item, dict) or item.get('type') != 'header' or item.get('schema') != SCHEMA:
            raise CheckpointError('invalid checkpoint header')
        identity = item.get('identity')
        if not isinstance(identity, dict) or item.get('identity_sha256') != _canonical_hash(identity):
            raise CheckpointError('checkpoint header identity digest is invalid')
        if item.get('identity_sha256') != self.identity_sha256 or identity != self.identity:
            raise CheckpointError('checkpoint source/binary/fixture/settings identity mismatch')
        expected = {str(k): self.run_seeds[k] for k in sorted(self.run_seeds)}
        if item.get('run_seeds') != expected:
            raise CheckpointError('checkpoint seed/run identity mismatch')

    def _accept_event(self, event, line_number):
        if not isinstance(event, dict) or event.get('type') != 'pair':
            raise CheckpointError(f'invalid checkpoint record at line {line_number}')
        run_id, seed, pair = event.get('run_id'), event.get('seed'), event.get('pair')
        if type(run_id) is not int or run_id not in self.run_seeds or type(seed) is not int:
            raise CheckpointError(f'unknown or malformed run identity at line {line_number}')
        if self.run_seeds[run_id] != seed:
            raise CheckpointError(f'conflicting seed for run {run_id} at line {line_number}')
        if not isinstance(pair, dict) or not isinstance(pair.get('pair_id'), str):
            raise CheckpointError(f'missing pair identity at line {line_number}')
        pair_id = pair['pair_id']
        try:
            id_run = int(pair_id.split(':', 1)[0])
        except (ValueError, IndexError) as exc:
            raise CheckpointError(f'malformed pair ID at line {line_number}') from exc
        if id_run != run_id:
            raise CheckpointError(f'pair ID conflicts with run ID at line {line_number}')
        if pair_id in self._ids:
            raise CheckpointError(f'duplicate pair ID at line {line_number}: {pair_id}')
        self._ids.add(pair_id)
        self._pairs[run_id].append(pair)

    def append(self, run_id, seed, pair):
        if self._closed:
            raise CheckpointError('checkpoint journal is closed')
        event = {'type': 'pair', 'run_id': run_id, 'seed': seed, 'pair': pair}
        self._accept_event(event, len(self._ids) + 2)
        try:
            self._stream.write(_encode_line(event))
            self._stream.flush()
        except Exception:
            # Roll back in-memory state if the append failed; the next resume will
            # validate and truncate only a partial final line.
            self._ids.remove(pair['pair_id'])
            self._pairs[run_id].pop()
            raise

    def pairs(self, run_id):
        return list(self._pairs[run_id])

    def runs(self):
        return [{'run_id': run_id, 'seed': self.run_seeds[run_id], 'pairs': self.pairs(run_id)}
                for run_id in self.run_seeds]

    def close(self):
        if not self._closed:
            self._stream.flush()
            os.fsync(self._stream.fileno())
            self._stream.close()
            self._closed = True

    def write_report_once(self, report):
        """Install a complete or explicitly incomplete ordinary report without overwrite."""
        self.close()
        if self.report_path.exists():
            raise CheckpointError('report already exists; refusing to overwrite it')
        report_bytes = (json.dumps(report, indent=2) + '\n').encode()
        fd, temp_name = tempfile.mkstemp(prefix='.report-', suffix='.tmp', dir=self.output)
        temp = Path(temp_name)
        try:
            with os.fdopen(fd, 'wb') as stream:
                stream.write(report_bytes)
                stream.flush()
                os.fsync(stream.fileno())
            # Hard-link publication is atomic and fails if report.json already exists.
            os.link(temp, self.report_path)
            _fsync_dir(self.output)
        finally:
            try:
                temp.unlink()
            except FileNotFoundError:
                pass
