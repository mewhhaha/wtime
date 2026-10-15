"""Explicit artifact check for the finalized P145 dependency (not generic CI)."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parent))
import p067


class FinalizedPathEvidence(unittest.TestCase):
    def test_finalized_paths_are_current_complete_and_untimed(self):
        result=p067.path_coverage()
        self.assertIs(result['timing_evidence'],False)
        self.assertEqual(result['coverage']['eager']['invocations'],173)
        self.assertEqual(result['coverage']['adaptive']['invocations'],376)
        self.assertEqual(result['coverage']['eager']['classes'],{
            'optimized':77,'prefix_exit':56,'mixed':0,'whole_function_fallback':40})
