import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('differential',Path(__file__).resolve().parents[1]/'differential.py')
diff=importlib.util.module_from_spec(spec);spec.loader.exec_module(diff)

class DifferentialTests(unittest.TestCase):
    def test_t165_case_01_wrong_result(self):
        expected={'status':'returned','values':[{'type':'i32','bits':'42'}]}
        self.assertFalse(diff.compare({'status':'returned','values':[{'type':'i32','bits':'41'}]},expected))
    def test_t165_case_02_nan_and_relaxed(self):
        self.assertTrue(diff.allowed_value({'type':'f32','bits':str(0x7fc00000)},{'type':'f32','nan':'canonical'}))
        self.assertFalse(diff.allowed_value({'type':'f32','bits':str(0x7fc00001)},{'type':'f32','nan':'canonical'}))
        self.assertTrue(diff.allowed_value({'type':'f64','bits':str(0x7ff8000000000001)},{'type':'f64','nan':'arithmetic'}))
        self.assertTrue(diff.allowed_value({'type':'i32','bits':'2'},{'type':'i32','allowed':['1','2']}))
        self.assertFalse(diff.allowed_value({'type':'i32','bits':'3'},{'type':'i32','allowed':['1','2']}))
    def test_t165_case_03_side_effects(self):
        cases=list(diff.generated_cases(165,4))
        self.assertIn('memory-write-read',cases[1]['name'])
        self.assertIn(b'\x36\x02\0',cases[1]['bytes'])
        self.assertIn('table-write-indirect-call',cases[2]['name'])
        self.assertIn(b'\x26\0',cases[2]['bytes'])
        for case in cases[1:3]:
            wrong={'status':'returned','values':[{'type':'i32','bits':str((int(case['expected']['values'][0]['bits'])+1)&0xffffffff)}]}
            self.assertFalse(diff.compare(wrong,case['expected']))
    def test_t165_trap_distinct(self):
        self.assertFalse(diff.compare({'status':'invalid'},{'status':'trap','trap':'Unreachable'}))
    def test_t165_seed_replays(self):
        self.assertEqual(list(diff.generated_cases(165,8)),list(diff.generated_cases(165,8)))
    def test_t166_expanded_proposal_vectors(self):
        cases=list(diff.generated_cases(165,10))
        self.assertEqual(len({c['name'].split('-',1)[1] for c in cases}),10)
        self.assertEqual(cases[5]['expected']['values'][0]['bits'],'0')
        self.assertIn(b'\xfd\x11\xfd\x1b\0',cases[6]['bytes'])
        self.assertIn(b'\xfc\x13',cases[8]['bytes'])
        self.assertEqual(cases[9]['profile'],'all-phase3-core')

if __name__=='__main__':unittest.main()
