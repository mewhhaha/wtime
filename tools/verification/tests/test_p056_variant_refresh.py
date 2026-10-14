"""Pure structural checks for P056 refreshed individual comparisons."""
from __future__ import annotations
import hashlib
import importlib.util
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'tools/verification/producers/p056_variants.py'
spec = importlib.util.spec_from_file_location('p056_variants_under_test', SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
import focused_compare


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class P056VariantRefreshTests(unittest.TestCase):
    def test_prepare_observation_uses_zero_wire_oracle_and_null_result(self):
        entry = {'id': 'prepare_large', 'oracle': {'module': 'run.py'},
                 'driver': {'kind': 'legacy', 'profile': 'scalar'},
                 'source': {'wasm_sha256': 'a' * 64}}
        wire = {'schema_version': 2, 'case': 'prepare_large', 'stage': 'prepare',
                'input': 0, 'expected': 0, 'result': None, 'mode': 'compact',
                'ns_per_operation': 20.0, 'batch': 1}
        with tempfile.TemporaryDirectory(prefix='p056-prepare-wire-') as temporary:
            root = Path(temporary).resolve()
            fixture = root / 'fixture.wasm'
            self.assertTrue(fixture.resolve().is_relative_to(root))
            with mock.patch.object(focused_compare.compare, 'process', return_value=wire) as process:
                observation = focused_compare.observe(
                    root / 'mock-bench', entry, fixture, 'prepare', 1, 'compact', 0, root)
        process.assert_called_once()
        self.assertTrue(module.validate_observation_identity(
            observation, 'prepare_large', 'prepare', None))
        self.assertEqual(observation['expected'], 0)
        self.assertIsNone(observation['result'])

        for changes in ({'stage': 'load_and_call'}, {'result': 0}, {'expected': None}):
            forged = dict(observation)
            forged.update(changes)
            with self.subTest(changes=changes), self.assertRaisesRegex(
                    ValueError, 'result or stage'):
                module.validate_observation_identity(forged, 'prepare_large', 'prepare', None)

    def test_nonprepare_observations_keep_strict_semantic_oracle(self):
        observation = {'stage': 'load_and_call', 'expected': 17, 'result': 17}
        self.assertTrue(module.validate_observation_identity(
            observation, 'call_overhead', 'load_and_call', 17))
        for changes in ({'stage': 'prepare'}, {'result': 18}, {'expected': 0}):
            forged = dict(observation)
            forged.update(changes)
            with self.subTest(changes=changes), self.assertRaisesRegex(
                    ValueError, 'result or stage'):
                module.validate_observation_identity(
                    forged, 'call_overhead', 'load_and_call', 17)

    def test_calibrated_warm_batch_is_required_for_both_complete_runs(self):
        plan = {'groups': [
            {'sample_workload': 'beta', 'stage': 'warm_call', 'product_sha256': 'b' * 64},
        ]}
        calibrations = []
        runs = []
        for run_id in (0, 1):
            calibration_observation = {
                'batch': 2, 'ns_per_operation': 10_000_000.0,
            }
            calibrations.append({
                'run_id': run_id, 'workload': 'beta', 'stage': 'warm_call',
                'batch': 2, 'target_ns': 20_000_000,
                'attempts': [{
                    'batch': 2,
                    'observations': {'baseline': calibration_observation,
                                     'candidate': calibration_observation},
                }],
            })
            pairs = []
            for sample in range(30):
                observation = {
                    'result': 7, 'expected': 7, 'stage': 'warm_call',
                    'batch': 2, 'ns_per_operation': 10_000_000.0,
                }
                pairs.append({
                    'pair_id': f'{run_id}:beta:warm_call:{sample}',
                    'workload': 'beta', 'stage': 'warm_call',
                    'product_sha256': 'b' * 64,
                    'order': ['baseline', 'candidate'], 'expected': 7,
                    'baseline': dict(observation), 'candidate': dict(observation),
                })
            runs.append({'run_id': run_id, 'pairs': pairs})

        warm_batches = module._validate_warm(
            {'warm_calibrations': calibrations}, plan)
        self.assertEqual(warm_batches, {
            (0, 'beta', 'warm_call'): 2,
            (1, 'beta', 'warm_call'): 2,
        })
        self.assertTrue(module.validate_pair_groups(
            runs, plan, warm_batches=warm_batches))

        runs[0]['pairs'][0]['baseline']['batch'] = 1
        with self.assertRaisesRegex(ValueError, 'calibrated or cold batch'):
            module.validate_pair_groups(runs, plan, warm_batches=warm_batches)

    def test_full_planned_groups_require_two_complete_pair_runs(self):
        plan = {'groups': [
            {'sample_workload': 'alpha', 'stage': 'prepare', 'product_sha256': 'a' * 64},
            {'sample_workload': 'beta', 'stage': 'warm_call', 'product_sha256': 'b' * 64},
            {'sample_workload': 'gamma', 'stage': 'load_and_call', 'product_sha256': 'c' * 64},
            {'sample_workload': 'prepare_large', 'stage': 'prepare', 'product_sha256': 'd' * 64},
        ]}
        runs = []
        for run_id in (0, 1):
            pairs = []
            for group in plan['groups']:
                for sample in range(30):
                    expected = None if group['sample_workload'] == 'prepare_large' else 7
                    observation = {'result': expected, 'expected': 0 if expected is None else expected, 'stage': group['stage'],
                                   'batch': 1, 'ns_per_operation': 12.0}
                    pairs.append({'pair_id': f'{run_id}:{group['sample_workload']}:{group['stage']}:{sample}',
                                  'workload': group['sample_workload'], 'stage': group['stage'],
                                  'product_sha256': group['product_sha256'], 'order': ['baseline', 'candidate'],
                                  'expected': expected, 'baseline': observation, 'candidate': observation})
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
