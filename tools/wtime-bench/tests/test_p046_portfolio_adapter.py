from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/verification/producers'))
import portfolio_compare as adapter


class PortfolioAdapterTests(unittest.TestCase):
    def sources(self):
        return {'crates/wtime-runtime/src/lib.rs':'1'*64,
                'tools/wtime-bench/compare.py':'2'*64,
                'tools/wtime-bench/representative_compare.py':'3'*64,
                'tools/wtime-bench/representative_workloads.py':'4'*64}

    def test_p046_adapter_excludes_unused_new_tools_from_frozen_harness_requirement(self):
        before=self.sources()
        after={**before,'tools/wtime-bench/focused_compare.py':'5'*64,
               'tools/wtime-bench/gates.py':'6'*64,'tools/wtime-bench/selection_manifest.py':'7'*64}
        self.assertEqual(adapter.measurement_sources(before),adapter.measurement_sources(after))
        self.assertIn(adapter.SOURCE,adapter.measurement_sources(after))
        self.assertEqual(adapter.measurement_sources(after)[adapter.SOURCE],adapter.compare.sha(ROOT/adapter.SOURCE))

    def test_p046_adapter_keeps_timer_oracle_and_runtime_changes_visible(self):
        before=self.sources()
        for name in before:
            with self.subTest(name=name):
                after={**before,name:'f'*64}
                self.assertNotEqual(adapter.measurement_sources(before),adapter.measurement_sources(after))

    def test_p046_adapter_is_scoped_and_restores_sources_after_failure(self):
        original=adapter.compare.sources
        with mock.patch.object(sys,'argv',['portfolio_compare.py','--selection','scalar']):
            with self.assertRaisesRegex(ValueError,'full representative'):
                adapter.main()
        with mock.patch.object(sys,'argv',['portfolio_compare.py','--selection','representative']), \
             mock.patch.object(adapter.compare,'main',side_effect=RuntimeError('fixture failure')):
            with self.assertRaisesRegex(RuntimeError,'fixture failure'):
                adapter.main()
        self.assertIs(adapter.compare.sources,original)


if __name__=='__main__':
    unittest.main()
