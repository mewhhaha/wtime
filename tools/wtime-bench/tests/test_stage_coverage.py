"""Promotion contracts cover declared cold-only stages as well as warm inputs."""
import sys
from pathlib import Path
import unittest
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gates

class StageCoverageTests(unittest.TestCase):
    stages = {'primary': {'warm_call', 'prepare'}, 'cold_control': {'load_and_call'}}
    contract = {'primary_workloads':['primary'],'protected_workloads':['cold_control']}
    def rows(self):
        rows=[]
        for run in (1,2):
            for workload, stages in self.stages.items():
                for stage in stages:
                    rows.append({'run':run,'workload':workload,'stage':stage,'input':'1','fixture':'f'*64,
                        'candidate_over_baseline':{'median':0.8 if stage=='warm_call' else 1.0},
                        'median_ratio_95ci':{'lower':0.79 if stage=='warm_call' else 0.99,
                            'upper':0.81 if stage=='warm_call' else 1.01}})
        return rows
    def evaluate(self,rows):
        family={'median_ratio':0.8,'ci95':{'lower':0.79,'upper':0.81}}
        with mock.patch.object(gates,'_selection_stages',return_value=self.stages), \
             mock.patch.object(gates,'_family_bootstrap',return_value=family):
            return gates._comparison_decision({'per_run':rows},'mixed',report={'validated_fixture':'test'},
                contract=self.contract,contract_hash='a'*64)
    def test_cold_only_control_does_not_require_undeclared_warm_stage(self):
        decision,reasons,checks=self.evaluate(self.rows())
        self.assertEqual(decision,'adopted');self.assertEqual(reasons,[])
        self.assertTrue(any(row.get('workload')=='cold_control' and row.get('stage')=='load_and_call'
            for row in checks['checks']))
    def test_each_input_fixture_requires_every_declared_stage(self):
        rows=self.rows();extra=dict(rows[0]);extra.update(stage='warm_call',input='2',fixture='e'*64)
        rows.append(extra)
        with self.assertRaisesRegex(gates.GateError,'missing prepare input/fixture groups'):
            self.evaluate(rows)
    def test_missing_cold_control_in_either_run_fails(self):
        rows=[r for r in self.rows() if not (r['workload']=='cold_control' and r['run']==2)]
        with self.assertRaisesRegex(gates.GateError,'missing input/fixture coverage for cold_control'):
            self.evaluate(rows)
    def test_primary_warm_stage_remains_required(self):
        rows=[r for r in self.rows() if not (r['workload']=='primary' and r['run']==2 and r['stage']=='warm_call')]
        with self.assertRaisesRegex(gates.GateError,'missing warm_call input/fixture groups'):
            self.evaluate(rows)

if __name__=='__main__':unittest.main()
