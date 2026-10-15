"""Reject incomplete cold-cost summaries and retain every measured cold row."""
import copy
import unittest
from tools.verification.producers import p110_finalize as final


def statistics():
    products=final.REQUIRED|{('memory_fill_scan','load_and_call')}
    return {'analysis':{'bootstrap_resamples':10_000,'minimum_pairs_per_group_per_run':30},
            'per_run':[{'run':run,'seed':100+run,'workload':case,'stage':stage,'pairs':30,
                        'candidate_over_baseline':{'median':1.0},'median_ratio_95ci':{'lower':0.99,'upper':1.01}}
                       for run in (1,2) for case,stage in sorted(products)]}


class P110ColdReviews(unittest.TestCase):
    def test_complete_cold_groups_keep_two_runs_and_additional_memory_cost(self):
        value=final.cold_review(statistics())
        self.assertFalse(value['requires_review'])
        self.assertEqual(value['products_per_run'],len(final.REQUIRED)+1)
        self.assertEqual(sum(row['workload']=='memory_fill_scan' for row in value['rows']),2)

    def test_over_budget_and_uncertain_intervals_remain_visible(self):
        source=statistics()
        source['per_run'][0]['candidate_over_baseline']['median']=1.4
        source['per_run'][0]['median_ratio_95ci']={'lower':1.3,'upper':1.5}
        source['per_run'][-1]['median_ratio_95ci']={'lower':0.9,'upper':1.5}
        result=final.cold_review(source)
        self.assertTrue(result['requires_review'])
        self.assertEqual({row['review'] for row in result['rows']},{'within_budget','over_budget','inconclusive'})

    def test_missing_duplicate_short_or_same_seed_groups_are_rejected(self):
        original=statistics()
        for mutation in ('missing','duplicate','short','seed','different_membership'):
            with self.subTest(mutation=mutation):
                source=copy.deepcopy(original)
                if mutation=='missing':source['per_run'].pop(0)
                elif mutation=='duplicate':source['per_run'].append(source['per_run'][0])
                elif mutation=='short':source['per_run'][0]['pairs']=29
                elif mutation=='seed':
                    for row in source['per_run']:row['seed']=101
                else:source['per_run'].pop(-1)
                with self.assertRaises(ValueError):final.cold_review(source)

    def test_boolean_counts_nan_and_invalid_confidence_intervals_fail(self):
        for mutation in ('boolean_pairs','nan','interval','boolean_run','resamples'):
            with self.subTest(mutation=mutation):
                source=statistics();row=source['per_run'][0]
                if mutation=='boolean_pairs':row['pairs']=True
                elif mutation=='nan':row['candidate_over_baseline']['median']=float('nan')
                elif mutation=='interval':row['median_ratio_95ci']={'lower':1.1,'upper':0.9}
                elif mutation=='boolean_run':row['run']=True
                else:source['analysis']['bootstrap_resamples']=10_000.0
                with self.assertRaises(ValueError):final.cold_review(source)


if __name__=='__main__':unittest.main()
