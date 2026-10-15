"""Validate legacy continuation stage/lifecycle analysis without timing guests."""
import copy
import json
from pathlib import Path
import sys
import unittest

BENCH = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(BENCH), str(Path(__file__).resolve().parent)]
import analyze
import continuation_pairing as pairing
import selection_manifest
import test_continuation_pairing as fixtures


def report():
    declaration = selection_manifest.load(BENCH.parents[1]/'docs/benchmarks/performance/P073-selection.json')
    entries = {entry['id']:entry for entry in selection_manifest.rows(declaration)}
    runs = []
    for index, seed in enumerate((73001,73002)):
        pairs = []
        for selected in declaration['rows']:
            entry = entries[selected['id']]
            for stage in selected['stages']:
                if pairing.matches(entry):
                    wire = fixtures._bind_wire() if entry['id'] == 'continuation_bound_nested' else fixtures._depth_wire(entry['id'])
                    observation = pairing.validate_observation(entry, wire, 1, expected_mode='decoded', expected_stage=stage)
                    expected = pairing.expected(entry)
                else:
                    observation = {'batch':1,'ns_per_operation':10.0,'mode':'decoded','result':0}
                    expected = 0
                for sample in range(30):
                    pairs.append({'pair_id':f'{index}-{entry["id"]}-{stage}-{sample}',
                        'workload':entry['id'],'stage':stage,'input':0,
                        'wasm_sha256':entry['source']['wasm_sha256'],'expected':expected,
                        'order':['baseline','candidate'],
                        'baseline':copy.deepcopy(observation),'candidate':copy.deepcopy(observation)})
        runs.append({'seed':seed,'pairs':pairs})
    return {'schema_version':2,'schema':'wtime-focused-comparison-v1','selection':'P073',
        'declaration':declaration,'passed':True,'source_stable':True,
        'metadata':{'settings':{'instrumented':False,'baseline_mode':'decoded','candidate_mode':'decoded'}},
        'runs':runs}


class ContinuationAnalysisTests(unittest.TestCase):
    def test_all_frozen_stages_keep_full_lifecycle_validation(self):
        _, runs = analyze._validated_runs(report())
        self.assertEqual(len(runs), 2)
        self.assertTrue(runs)

    def test_lifecycle_expected_and_raw_wire_tampering_fail(self):
        for field in ['expected','raw_wire_observation','stage']:
            bad = report()
            pair = next(row for row in bad['runs'][0]['pairs'] if row['workload'] == 'continuation_depth_4')
            if field == 'expected': pair['expected']['resume']['result'] = ['i32:999']
            elif field == 'raw_wire_observation': pair['candidate'][field]['result'] = ['i32:999']
            else: pair['stage'] = 'accounting'
            with self.subTest(field=field), self.assertRaises(analyze.ReportError):
                analyze._validated_runs(bad)

    def test_missing_selected_stage_or_forged_fixture_fails(self):
        bad = report()
        for run in bad['runs']:
            run['pairs'] = [pair for pair in run['pairs'] if not
                (pair['workload'] == 'continuation_bound_nested' and pair['stage'] == 'bind')]
        with self.assertRaises(analyze.ReportError): analyze._validated_runs(bad)
        bad = report()
        bad['runs'][0]['pairs'][0]['wasm_sha256'] = '0'*64
        with self.assertRaises(analyze.ReportError): analyze._validated_runs(bad)
