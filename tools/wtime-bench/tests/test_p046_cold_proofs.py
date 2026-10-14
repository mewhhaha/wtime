"""Cold controls cannot be omitted or overridden by a warm speedup."""
from pathlib import Path
import sys
import unittest
sys.dont_write_bytecode=True
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import p046_gate as gate
from test_p124_portfolio import _report

class ColdTests(unittest.TestCase):
    def test_p046_cold_requires_both_full_selections_and_large_prepare(self):
        reports={'structural':_report('original-mvp','decoded'),'p001':_report('original-mvp','p001')}
        matrix={'candidate_binary_sha256':'4'*64,'selected_cpu':2}
        self.assertEqual(len(gate._cold_controls({'cold_controls':reports},matrix,check_sources=False)),2)
        removed=reports.pop('p001')
        with self.assertRaises(gate.PortfolioError):gate._cold_controls({'cold_controls':reports},matrix,check_sources=False)
        reports['p001']=removed
        pairs=reports['structural']['runs'][0]['pairs']
        pairs[:]=[row for row in pairs if row['workload']!='prepare_large']
        with self.assertRaises(gate.PortfolioError):gate._cold_controls({'cold_controls':reports},matrix,check_sources=False)

    def test_p046_cold_over_budget_blocks_other_speed_gains(self):
        summary={'per_run':[{'workload':'prepare_large','stage':'prepare','run':1,
                            'median_ratio_95ci':{'lower':1.26,'upper':1.3}}]}
        decision,reasons=gate._cold_policy({'cold':summary},{'structural':'cold'})
        self.assertEqual(decision,'rejected');self.assertTrue(reasons)

    def test_p046_cold_crossing_interval_is_inconclusive(self):
        summary={'per_run':[{'workload':'call_overhead','stage':'first_call','run':2,
                            'median_ratio_95ci':{'lower':1.04,'upper':1.06}}]}
        decision,_=gate._cold_policy({'cold':summary},{'structural':'cold'})
        self.assertEqual(decision,'inconclusive')

if __name__=='__main__':unittest.main()
