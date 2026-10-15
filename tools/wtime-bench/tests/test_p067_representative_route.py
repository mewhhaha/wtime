"""Contained routing checks for the P067 representative Register comparator."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

sys.dont_write_bytecode = True
BENCH = Path(__file__).resolve().parents[1]
ROOT = BENCH.parents[1]
sys.path.insert(0, str(BENCH))

import baseline
import checkpoint
import representative_compare


class _ReachedBaseline(Exception):
    pass


class P067RepresentativeRouteTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='p067-representative-route-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.audit = (self.root / 'audit').resolve()
        self.assertTrue(self.audit.is_relative_to(self.root))
        self.audit.mkdir()
        # Keep the exact helper and synthetic invocation record inside this
        # fresh root before run() is exercised. The mocked route never starts
        # a guest, writes benchmark output, or accesses a real archive.
        (self.audit / 'representative_compare.py').write_bytes(
            (BENCH / 'representative_compare.py').read_bytes())
        (self.audit / 'test_source.py').write_bytes(Path(__file__).read_bytes())
        self.output = self.root / 'output'
        self.assertTrue(self.output.resolve().is_relative_to(self.root))
        (self.audit / 'invocation.json').write_text(json.dumps({
            'test': self.id(),
            'synthetic_args': {'selection': 'representative', 'pairs': 30,
                               'candidate_mode': 'register'},
            'mocked_io': ['checkpoint.Journal.preflight', 'baseline.verify_archive'],
        }, sort_keys=True))

    def args(self, **overrides):
        values = dict(selection='representative', workload_subset=None,
                      candidate_mode='register', pairs=30, output=self.output,
                      baseline=self.root / 'synthetic-baseline', candidate=self.root / 'synthetic-candidate',
                      seed=17, cpu=None)
        values.update(overrides)
        return SimpleNamespace(**values)

    def test_register_route_passes_mode_guard_and_reaches_baseline_validation(self):
        with mock.patch.object(checkpoint.Journal, 'preflight') as preflight, \
                mock.patch.object(baseline, 'verify_archive', side_effect=_ReachedBaseline) as verify:
            with self.assertRaises(_ReachedBaseline):
                representative_compare.run(self.args())
        preflight.assert_called_once_with(self.output)
        verify.assert_called_once_with(self.root / 'synthetic-baseline')

    def test_reference_and_unknown_modes_fail_before_any_preflight_io(self):
        for mode in ('reference', 'ref', 'unknown'):
            with self.subTest(mode=mode), \
                    mock.patch.object(checkpoint.Journal, 'preflight') as preflight, \
                    mock.patch.object(baseline, 'verify_archive') as verify:
                with self.assertRaises(representative_compare.RepresentativeComparisonError):
                    representative_compare.run(self.args(candidate_mode=mode))
                preflight.assert_not_called()
                verify.assert_not_called()

    def test_small_boolean_or_invalid_pair_counts_fail_before_io(self):
        for pairs in (29, True, False, 30.0, '30', None):
            with self.subTest(pairs=pairs), \
                    mock.patch.object(checkpoint.Journal, 'preflight') as preflight, \
                    mock.patch.object(baseline, 'verify_archive') as verify:
                with self.assertRaises(representative_compare.RepresentativeComparisonError):
                    representative_compare.run(self.args(pairs=pairs))
                preflight.assert_not_called()
                verify.assert_not_called()

    def test_workload_subset_is_rejected_before_io(self):
        with mock.patch.object(checkpoint.Journal, 'preflight') as preflight, \
                mock.patch.object(baseline, 'verify_archive') as verify:
            with self.assertRaises(representative_compare.RepresentativeComparisonError):
                representative_compare.run(self.args(workload_subset=['representative_add_n1']))
        preflight.assert_not_called()
        verify.assert_not_called()


if __name__ == '__main__':
    unittest.main()
