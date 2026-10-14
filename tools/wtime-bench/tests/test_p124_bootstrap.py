"""Check the rank sampler against exhaustive bootstrap resamples."""
import itertools
import math
from pathlib import Path
import random
import sys
import unittest
sys.dont_write_bytecode=True
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from bootstrap import median_ranks,draw_median
import p046_gate

class BootstrapTests(unittest.TestCase):
    def test_p124_rank_distribution_matches_all_even_and_odd_resamples(self):
        for count in (2,3,4,5):
            observed={}
            for sample in itertools.product(range(count),repeat=count):
                ordered=sorted(sample);key=(ordered[(count-1)//2],ordered[count//2])
                observed[key]=observed.get(key,0)+1/count**count
            cumulative,ranks=median_ranks(count);previous=0.0
            for upper,rank in zip(cumulative,ranks):
                self.assertAlmostEqual(upper-previous,observed.pop(rank,0),places=12)
                previous=upper
            self.assertTrue(all(abs(value)<1e-12 for value in observed.values()))

    def test_p124_rank_sampler_is_seeded_and_preserves_constant_medians(self):
        left=random.Random(12);right=random.Random(12)
        values=[math.log(index+1) for index in range(30)]
        self.assertEqual([draw_median(values,left) for _ in range(1000)],
                         [draw_median(values,right) for _ in range(1000)])
        self.assertEqual(draw_median([3.0]*30,left),3.0)

    def test_p124_large_groups_keep_explicit_bootstrap_and_reject_bad_counts(self):
        values=list(range(129));left=random.Random(1);right=random.Random(1)
        expected=sorted(values[right.randrange(len(values))] for _ in values)[64]
        self.assertEqual(draw_median(values,left),expected)
        for count in (0,False,129):
            with self.assertRaises(ValueError):median_ranks(count)

    def test_p124_primary_family_uses_equal_workload_weights(self):
        runs=[]
        for seed in (1,2):
            pairs=[{'workload':name,'stage':'warm_call','baseline':{'ns_per_operation':100},
                    'candidate':{'ns_per_operation':ratio*100}}
                   for name,ratio in [('fast',.8),('control',1.0)] for _ in range(30)]
            runs.append({'seed':seed,'pairs':pairs})
        rows=p046_gate._family_statistics({'runs':runs},'test-identity')
        self.assertEqual(len(rows),2)
        for row in rows:
            self.assertEqual(row['workloads'],2)
            self.assertAlmostEqual(row['median_ratio'],math.sqrt(.8))
            self.assertAlmostEqual(row['ci95']['upper'],math.sqrt(.8))
