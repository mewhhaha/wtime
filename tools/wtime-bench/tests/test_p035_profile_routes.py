"""Synthetic bounded route tests; no builds/native calls or output writes."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest
sys.dont_write_bytecode=True
HERE=Path(__file__).resolve().parent
TOOL=HERE.parent if HERE.name=='tests' else Path('/workspace/wtime/tools/wtime-bench');sys.path.insert(0,str(TOOL))
import workloads
spec=importlib.util.spec_from_file_location('p035_profile_routes_tested',(TOOL if HERE.name=='tests' else HERE)/'profile_routes.py')
if spec is None or spec.loader is None:raise RuntimeError('missing staged profile routes')
routes=importlib.util.module_from_spec(spec);spec.loader.exec_module(routes)


class ProfileRouteDraftTests(unittest.TestCase):
    def test_actual_registered_rows_have_exact_bounded_supported_and_gap_counts(self):
        for name,supported,gaps in [('scalar',3,82),('memory',1,34),('representative',9,0)]:
            rows=workloads.selection(name)
            decisions=[routes.counter_route(row,name) for row in rows]
            self.assertEqual(sum(value!='unsupported' for value in decisions),supported)
            self.assertEqual(decisions.count('unsupported'),gaps)

    def test_legacy_id_cannot_smuggle_a_different_oracle_or_reference_input(self):
        row=workloads.selection('scalar')[0]
        wrong=copy.deepcopy(row);wrong['oracle']['module']='gc_workloads.py'
        with self.assertRaises(ValueError):routes.counter_route(wrong,'scalar')
        wrong=copy.deepcopy(row);wrong['params'][0]['type']='reference'
        with self.assertRaises(ValueError):routes.counter_route(wrong,'scalar')

    def test_unknown_modern_oracle_fails_instead_of_becoming_an_unsupported_placeholder(self):
        row=next(row for row in workloads.selection('scalar') if row['id'].startswith('scalar_'))
        self.assertEqual(routes.counter_route(row,'scalar'),'unsupported')
        wrong=copy.deepcopy(row);wrong['oracle']['module']='unregistered_fake.py'
        with self.assertRaises(ValueError):routes.counter_route(wrong,'scalar')


if __name__=='__main__':unittest.main()
