"""Pure validation checks for the tier-aware Wasmtime runner."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import run


class WasmtimeRunnerTests(unittest.TestCase):
    def test_tier_argument_validation(self) -> None:
        self.assertEqual(run.tier_arguments('decoded'), ['decoded'])
        self.assertEqual(run.tier_arguments('compact'), ['compact'])
        self.assertEqual(run.tier_arguments('register', 3), ['register', '3'])
        for mode, threshold in (('nope', None), ('decoded', 3), ('compact', 3),
                                ('register', 0), ('register', -1),
                                ('register', 1 << 64), ('register', True)):
            with self.subTest(mode=mode, threshold=threshold):
                with self.assertRaises(ValueError):
                    run.tier_arguments(mode, threshold)

    def test_fixture_oracles_and_encoded_identities_stay_in_fresh_root(self) -> None:
        with tempfile.TemporaryDirectory(prefix='wasmtime-runner-fixtures-') as temp:
            root = Path(temp).resolve()
            old_root, old_fixtures = run.ROOT, run.FIXTURES
            try:
                run.ROOT = root
                run.FIXTURES = root/'shared-fixtures'
                encoder = root/'encoder'
                out_dir = root/'fresh'/'fixtures'
                self.assertTrue(encoder.resolve().is_relative_to(root))
                self.assertTrue(out_dir.resolve().is_relative_to(root))
                sources = run.build_fixture_sources()
                for case in sources:
                    self.assertTrue((out_dir/(case+'.wat')).resolve().is_relative_to(root))
                    self.assertTrue((out_dir/(case+'.wasm')).resolve().is_relative_to(root))
                encoder.write_text('#!/bin/sh\nprintf wasm > "$4"\n')
                encoder.chmod(0o755)
                directory, records, resolved_encoder = run.write_fixtures(
                    sources, out_dir, encoder)
                self.assertEqual(directory, out_dir.resolve())
                self.assertEqual(resolved_encoder, encoder.resolve())
                self.assertEqual(set(records), set(sources))
                for case, record in records.items():
                    wat = Path(record['wat_path']).resolve()
                    wasm = Path(record['wasm_path']).resolve()
                    self.assertTrue(wat.is_relative_to(root))
                    self.assertTrue(wasm.is_relative_to(root))
                    self.assertEqual(record['wat_sha256'], run.sha(wat))
                    self.assertEqual(record['wasm_sha256'], run.sha(wasm))
                    self.assertEqual(record['expected_result'],
                                     0 if case == 'prepare_large' else
                                     run.oracle(case, run.INPUTS[case]))
                with self.assertRaises(ValueError):
                    run.write_fixtures(sources, out_dir, encoder)
            finally:
                run.ROOT, run.FIXTURES = old_root, old_fixtures

    def test_timing_parser_rejects_duplicate_nonfinite_and_missing_keys(self) -> None:
        valid = '\n'.join(
            f'call_overhead,{stage},{engine},{sample},1,12.5'
            for stage in run.STAGES for engine in run.ENGINES for sample in range(5))
        parsed = run.parse_timings(valid, 'call_overhead', 5)
        self.assertEqual(len(parsed), 50)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            run.parse_timings(valid+'\n'+valid.splitlines()[0], 'call_overhead', 5)
        broken = valid.replace(',12.5', ',nan', 1)
        with self.assertRaisesRegex(ValueError, 'finite'):
            run.parse_timings(broken, 'call_overhead', 5)
        with self.assertRaisesRegex(ValueError, 'missing'):
            run.parse_timings('\n'.join(valid.splitlines()[:-1]), 'call_overhead', 5)
        with self.assertRaisesRegex(ValueError, 'unexpected'):
            run.parse_timings(valid+'\nother,prepare,wtime,0,1,2', 'call_overhead', 5)


if __name__ == '__main__':
    unittest.main()
