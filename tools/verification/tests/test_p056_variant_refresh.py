"""Pure structural checks for P056 refreshed individual comparisons."""
from __future__ import annotations
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'tools/verification/producers/p056_variants.py'
spec = importlib.util.spec_from_file_location('p056_variants_under_test', SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class P056VariantRefreshTests(unittest.TestCase):
    def test_full_planned_groups_require_two_complete_pair_runs(self):
        plan = {'groups': [
            {'sample_workload': 'alpha', 'stage': 'prepare', 'product_sha256': 'a' * 64},
            {'sample_workload': 'beta', 'stage': 'warm_call', 'product_sha256': 'b' * 64},
            {'sample_workload': 'gamma', 'stage': 'load_and_call', 'product_sha256': 'c' * 64},
        ]}
        runs = []
        for run_id in (0, 1):
            pairs = []
            for group in plan['groups']:
                for sample in range(30):
                    observation = {'result': 7, 'expected': 7, 'stage': group['stage'],
                                   'batch': 1, 'ns_per_operation': 12.0}
                    pairs.append({'pair_id': f'{run_id}:{group['sample_workload']}:{group['stage']}:{sample}',
                                  'workload': group['sample_workload'], 'stage': group['stage'],
                                  'product_sha256': group['product_sha256'], 'order': ['baseline', 'candidate'],
                                  'expected': 7, 'baseline': observation, 'candidate': observation})
            runs.append({'run_id': run_id, 'pairs': pairs})
        self.assertTrue(module.validate_pair_groups(runs, plan))
        runs[1]['pairs'].pop()
        with self.assertRaisesRegex(ValueError, '30 pairs'):
            module.validate_pair_groups(runs, plan)

    def test_source_hash_mutation_is_rejected_inside_fresh_temp_root(self):
        with tempfile.TemporaryDirectory(prefix='p056-source-') as temporary:
            root = Path(temporary).resolve()
            payload = root / 'source' / 'crates' / 'sample.rs'
            payload.parent.mkdir(parents=True)
            self.assertTrue(payload.resolve().is_relative_to(root))
            payload.write_text('stable source\n')
            source_map = {'files': {'crates/sample.rs': digest(payload)},
                          'locations': [{'prefix': 'crates/', 'path': str(payload.parent)}]}
            self.assertTrue(module.validate_source_locations(source_map))
            payload.write_text('mutated source\n')
            with self.assertRaisesRegex(ValueError, 'source hash changed'):
                module.validate_source_locations(source_map)

    def test_old_fusion_core_remains_archive_bound(self):
        with tempfile.TemporaryDirectory(prefix='p056-core-map-') as temporary:
            root = Path(temporary).resolve()
            archive_core = root / 'archive' / 'source' / 'crates' / 'wtime-runtime' / 'src' / 'lib.rs'
            harness = root / 'archive' / 'source' / 'tools' / 'wtime-bench' / 'src' / 'protocol.rs'
            for path in (archive_core, harness):
                path.parent.mkdir(parents=True, exist_ok=True)
                self.assertTrue(path.resolve().is_relative_to(root))
                path.write_text(path.name + '\n')
            old = {'source/crates/wtime-runtime/src/lib.rs': digest(archive_core)}
            compiled_harness = {'tools/wtime-bench/src/protocol.rs': digest(harness)}
            frozen = module.compose_sources(old, compiled_harness)
            current = dict(frozen)
            current['crates/wtime-runtime/src/lib.rs'] = 'f' * 64
            record = module._side_map(frozen, 'frozen_archive', root / 'archive', root / 'adapter')
            self.assertEqual(record['core_source_kind'], 'frozen_archive')
            self.assertNotEqual(record['files']['crates/wtime-runtime/src/lib.rs'],
                                current['crates/wtime-runtime/src/lib.rs'])
            self.assertEqual(record['locations'][0]['path'], str(root / 'archive' / 'source' / 'crates'))


if __name__ == '__main__':
    unittest.main()
