import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import json
import tempfile
import unittest
from pathlib import Path

from checkpoint import CheckpointError, Journal


class JournalTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.identity = {'selection': 'scalar', 'source_sha256': 'src-a',
                         'candidate_sha256': 'bin-a', 'fixture_sha256': 'wasm-a',
                         'mode': 'decoded', 'instrumented': False}
        self.seeds = {0: 101, 1: 202}

    def tearDown(self):
        self.tmp.cleanup()

    def row(self, run=0, seed=101, sample=0):
        return {'pair_id': f'{run}:fixture:warm_call:{sample}', 'workload': 'fixture',
                'stage': 'warm_call', 'input': 7, 'expected': 8, 'wasm_sha256': 'wasm-a',
                'order': ['baseline', 'candidate'], 'baseline': {'ns_per_operation': 12},
                'candidate': {'ns_per_operation': 9}}

    def test_recovery_keeps_all_pairs_and_raw_values(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        expected = [self.row(sample=i) for i in range(3)]
        for pair in expected:
            j.append(0, 101, pair)
        j.close()
        resumed = Journal(output, self.identity, self.seeds, resume=True)
        self.assertEqual(resumed.pairs(0), expected)
        self.assertEqual(resumed.pairs(1), [])
        resumed.append(1, 202, self.row(run=1, seed=202))
        report = {'schema_version': 2, 'passed': True, 'source_stable': True,
                  'runs': resumed.runs(), 'metadata': {'identity': self.identity}}
        expected_report = json.loads(json.dumps(report))
        resumed.write_report_once(report)
        actual = json.loads((output / 'report.json').read_text())
        self.assertEqual(actual, expected_report)
        self.assertEqual(actual['runs'][0]['pairs'], expected)
        with self.assertRaisesRegex(CheckpointError, 'report already exists'):
            Journal(output, self.identity, self.seeds, resume=True)

    def test_only_malformed_unterminated_tail_is_repaired(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        j.append(0, 101, self.row())
        j.close()
        journal = output / 'checkpoint.jsonl'
        with journal.open('ab') as f:
            f.write(b'{"type":"pair","torn":')
        recovered = Journal(output, self.identity, self.seeds, resume=True)
        self.assertEqual(recovered.pairs(0), [self.row()])
        recovered.close()
        self.assertTrue(journal.read_bytes().endswith(b'\n'))

    def test_complete_malformed_line_rejected(self):
        output = self.root / 'out'
        Journal(output, self.identity, self.seeds).close()
        with (output / 'checkpoint.jsonl').open('ab') as f:
            f.write(b'{broken}\n')
        with self.assertRaisesRegex(CheckpointError, 'malformed complete'):
            Journal(output, self.identity, self.seeds, resume=True)

    def test_header_and_seed_conflicts_are_rejected(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        j.close()
        path = output / 'checkpoint.jsonl'
        header = json.loads(path.read_text().splitlines()[0])
        header['run_seeds']['0'] = 999
        path.write_text(json.dumps(header) + '\n')
        with self.assertRaisesRegex(CheckpointError, 'identity mismatch|seed/run identity mismatch'):
            Journal(output, self.identity, self.seeds, resume=True)

    def test_duplicate_pair_and_conflicting_run_id_are_rejected(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        pair = self.row()
        j.append(0, 101, pair)
        j.close()
        with (output / 'checkpoint.jsonl').open('ab') as f:
            f.write(json.dumps({'type': 'pair', 'run_id': 0, 'seed': 101, 'pair': pair}) .encode() + b'\n')
        with self.assertRaisesRegex(CheckpointError, 'duplicate pair ID'):
            Journal(output, self.identity, self.seeds, resume=True)
        output2 = self.root / 'out2'
        j2 = Journal(output2, self.identity, self.seeds)
        with self.assertRaisesRegex(CheckpointError, 'pair ID conflicts'):
            j2.append(1, 202, self.row(run=0))
        j2.close()

    def test_source_and_binary_mismatch_and_missing_checkpoint_fail_closed(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        j.append(0, 101, self.row())
        j.close()
        for key, value in [('source_sha256', 'src-b'), ('candidate_sha256', 'bin-b')]:
            changed = dict(self.identity, **{key: value})
            with self.subTest(key=key), self.assertRaisesRegex(CheckpointError, 'identity mismatch'):
                Journal(output, changed, self.seeds, resume=True)
        missing = self.root / 'missing'
        missing.mkdir()
        with self.assertRaisesRegex(CheckpointError, 'journal is missing'):
            Journal(missing, self.identity, self.seeds, resume=True)
        with self.assertRaisesRegex(CheckpointError, 'output already exists'):
            Journal(output, self.identity, self.seeds)


    def test_partial_header_and_valid_unterminated_tail_are_rejected(self):
        output = self.root / 'out'
        output.mkdir()
        journal = output / 'checkpoint.jsonl'
        journal.write_bytes(b'{"type":"head')
        with self.assertRaisesRegex(CheckpointError, 'malformed complete'):
            Journal(output, self.identity, self.seeds, resume=True)
        journal.write_text(json.dumps({'type': 'header', 'schema': 'wtime-compare-checkpoint-v1',
                                       'identity': self.identity,
                                       'identity_sha256': __import__('checkpoint')._canonical_hash(self.identity),
                                       'run_seeds': {'0': 101, '1': 202}}) + '\n' +
                           json.dumps({'type': 'pair', 'run_id': 0, 'seed': 101,
                                       'pair': self.row() }))
        with self.assertRaisesRegex(CheckpointError, 'lacks its terminating newline'):
            Journal(output, self.identity, self.seeds, resume=True)

    def test_passed_report_is_never_overwritten(self):
        output = self.root / 'out'
        j = Journal(output, self.identity, self.seeds)
        j.append(0, 101, self.row())
        report = {'schema_version': 2, 'passed': True, 'source_stable': True,
                  'runs': j.runs()}
        j.write_report_once(report)
        original = (output / 'report.json').read_bytes()
        with self.assertRaisesRegex(CheckpointError, 'report already exists'):
            j.write_report_once({'passed': False})
        self.assertEqual((output / 'report.json').read_bytes(), original)
        with self.assertRaisesRegex(CheckpointError, 'report already exists'):
            Journal(output, self.identity, self.seeds, resume=True)


if __name__ == '__main__':
    unittest.main()
