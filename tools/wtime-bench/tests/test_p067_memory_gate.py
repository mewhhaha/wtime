import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path.insert(0, str(BENCH))
spec = importlib.util.spec_from_file_location(
    'p067_candidate_gate', ROOT / 'tools/verification/producers/p067_candidate_gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def seeded_entry(workload_id, family, size):
    return {'id': workload_id, 'family': family,
            'params': [{'name': 'bytes', 'baseline': size}],
            'oracle': {'kind': 'python', 'module': 'memory_workloads.py'}}


def nested_observation(entry):
    import memory_scaling
    import memory_workloads
    expected = memory_workloads.expected_memory(entry['id'], entry['params'][0]['baseline'])
    rows = [{key: item[key] for key in
             ('memory_index', 'pages', 'full_memory_sha256', 'range', 'range_sha256')}
            for item in expected['memories']]
    selected = rows[-1]
    return {'memory': {'driver': memory_scaling._driver(entry),
                       'host_setup_excluded': True,
                       'memory_count': len(rows),
                       'memory_index': selected['memory_index'],
                       'memory_pages': selected['pages'],
                       'memory_page_size_log2': 16,
                       'memories': rows}}


class P067MemoryProofTests(unittest.TestCase):
    def cold_report(self, ratio, family='memory'):
        row = {'id':'memory_growth_success_one', 'role':'targeted',
               'family':family, 'stages':['load_and_call']}
        report = {'declaration':{'rows':[row]}, 'cold_products':{}, 'counts':{},
            'runs':[{'seed':seed, 'pairs':[{'workload':row['id'], 'stage':'load_and_call',
                'baseline':{'ns_per_operation':100}, 'candidate':{'ns_per_operation':100*ratio}}
                for _ in range(30)]} for seed in (67001,67002)]}
        summary = {'per_run':[{'workload':row['id'], 'run':run, 'stage':'load_and_call',
            'candidate_over_baseline':{'median':ratio},
            'median_ratio_95ci':{'lower':ratio*.999,'upper':ratio*1.001}} for run in (1,2)]}
        return report, summary

    def test_declared_memory_cold_primary_retains_both_run_decisions(self):
        report, summary = self.cold_report(.5)
        with mock.patch.object(gate.analyze, 'analyze', return_value=summary) as statistics:
            result = gate.decision(report)
        statistics.assert_called_once_with(report)
        self.assertEqual(result['decision'], 'adopted')
        self.assertEqual([row['stages'] for row in result['primary']],
                         [{'memory_growth_success_one':'load_and_call'}]*2)

    def test_memory_cold_regression_remains_rejected(self):
        report, summary = self.cold_report(1.2)
        with mock.patch.object(gate.analyze, 'analyze', return_value=summary):
            self.assertEqual(gate.decision(report)['decision'], 'rejected')

    def test_unknown_cold_primary_fails_before_statistics(self):
        report, summary = self.cold_report(.5, family='unknown')
        with mock.patch.object(gate.analyze, 'analyze', return_value=summary) as statistics:
            with self.assertRaises(ValueError):
                gate.decision(report)
        statistics.assert_not_called()

    def test_valid_seeded_copy_and_fill_validate_every_image(self):
        cases = [
            seeded_entry('memory_copy_distinct_seeded_17', 'memory_copy_distinct_seeded', 17),
            seeded_entry('memory_fill_seeded_17', 'memory_fill_seeded', 17),
        ]
        for entry in cases:
            with self.subTest(workload=entry['id']):
                gate.validate_memory_proof(entry, nested_observation(entry))

    def test_rejects_bad_full_images_ranges_record_order_and_omissions(self):
        entry = seeded_entry('memory_copy_distinct_seeded_17', 'memory_copy_distinct_seeded', 17)
        valid = nested_observation(entry)
        mutations = []
        for index, field, value in (
                (0, 'full_memory_sha256', '0' * 64),
                (1, 'full_memory_sha256', '1' * 64),
                (1, 'range_sha256', '2' * 64),
                (1, 'range', [65536, 65554])):
            changed = copy.deepcopy(valid)
            changed['memory']['memories'][index][field] = value
            mutations.append(changed)
        omitted = copy.deepcopy(valid)
        omitted['memory']['memories'].pop(0)
        mutations.append(omitted)
        reordered = copy.deepcopy(valid)
        reordered['memory']['memories'].reverse()
        mutations.append(reordered)
        for observation in mutations:
            with self.subTest(observation=observation['memory']['memories']):
                with self.assertRaises(ValueError):
                    gate.validate_memory_proof(entry, observation)

    def test_rejects_wrong_record_count_driver_and_selected_shape(self):
        entry = seeded_entry('memory_copy_distinct_seeded_17', 'memory_copy_distinct_seeded', 17)
        for field, value in (('memory_count', 1), ('driver', 'seeded-fill'),
                             ('memory_index', 0), ('memory_pages', 16),
                             ('host_setup_excluded', False)):
            observation = copy.deepcopy(nested_observation(entry))
            observation['memory'][field] = value
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    gate.validate_memory_proof(entry, observation)

    def test_ordinary_memory_proof_and_complete_entry_cache(self):
        entry = {'id': 'memory_fill_64', 'family': 'memory_fill',
                 'params': [{'name': 'bytes', 'baseline': 64}],
                 'oracle': {'kind': 'python', 'module': 'memory_workloads.py'}}
        import focused_compare
        import memory_workloads
        expected = memory_workloads.expected_memory(entry['id'], focused_compare.input_value(entry))
        gate.validate_memory_proof(entry, {'memory': {
            'full_memory_sha256': expected['full_memory_sha256'],
            'all_memory_sha256': expected.get('all_memory_sha256')}})
        with self.assertRaises(ValueError):
            gate.validate_memory_proof(entry, {'memory': {
                'full_memory_sha256': '0' * 64,
                'all_memory_sha256': expected.get('all_memory_sha256')}})
        same_id_different_input = dict(entry)
        same_id_different_input['params'] = [{'name': 'bytes', 'baseline': 65}]
        self.assertEqual(gate.expected(entry), 5760)
        self.assertEqual(gate.expected(same_id_different_input), 5850)


if __name__ == '__main__':
    unittest.main()
