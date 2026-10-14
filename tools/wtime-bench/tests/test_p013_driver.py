"""P013 actual-process checks for the staged shared-memory thread driver.

The integrated test should set WTIME_BENCH_THREAD_DRIVER to the current-source
protocol executable. These tests use the existing encoder and never build it.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path('/workspace/wtime')
BENCH_ROOT = ROOT / 'tools/wtime-bench'
BENCH = os.environ.get('WTIME_BENCH_THREAD_DRIVER')
ENCODER = os.environ.get(
    'WTIME_CONFORMANCE_ENCODER', str(ROOT / 'target/debug/wtime-conformance')
)
sys.path.insert(0, str(BENCH_ROOT))
import thread_workloads


@unittest.skipUnless(
    BENCH and Path(BENCH).is_file(),
    'set WTIME_BENCH_THREAD_DRIVER to a built current-source protocol binary',
)
class ThreadDriverSubprocessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        hashes = thread_workloads.encode_fixtures(ENCODER, cls.temp.name)
        if hashes != thread_workloads.WASM_SHA256:
            raise AssertionError('encoded thread fixture hashes differ from workload model')
        cls.wasm = {
            name: Path(cls.temp.name) / (Path(name).stem + '.wasm')
            for name in thread_workloads.WASM_SHA256
        }
        cls.entries = thread_workloads.entries()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def run_driver(self, *args, timeout=10):
        return subprocess.run(
            [str(BENCH), *map(str, args)], cwd=ROOT, text=True,
            capture_output=True, timeout=timeout,
        )

    def observe(self, *args, timeout=10):
        process = self.run_driver(*args, timeout=timeout)
        self.assertEqual(process.returncode, 0, process.stderr)
        try:
            row = json.loads(process.stdout)
        except json.JSONDecodeError as exc:
            self.fail(f'driver did not return JSON: {exc}; stdout={process.stdout!r}')
        self.assertEqual(row.get('schema_version'), 1)
        self.assertEqual(row.get('status'), 'ok')
        return row

    def test_every_registered_kernel_runs_with_oracle_checked_state(self):
        entries = self.entries
        self.assertEqual(len(entries), 18)
        observed = set()
        for entry in entries:
            workload = entry['id']
            config = entry['thread_config']
            requested = config['requested_workers']
            iterations = config['iterations_per_worker']
            fixture = Path(entry['source']['path']).name
            with self.subTest(workload=workload):
                if workload == 'threads_wait_notify_latency':
                    row = self.observe(
                        '--wait-latency', self.wasm[fixture], 100,
                        entry['measurement_protocol']['wait_timeout_ns'],
                        timeout=30,
                    )
                    self.assertEqual(row['case'], workload)
                    self.assertEqual(row['samples'], 100)
                    self.assertEqual(row['sample_count'], 100)
                    self.assertEqual(row['sample_kind'], 'one_wait_notify_round_trip')
                    self.assertTrue(row['registration_observed'])
                    self.assertIn('successful notify returned one', row['registration_evidence'])
                    self.assertEqual(row['outcome'], 'notified')
                    self.assertEqual(len(row['notify_to_return_ns']), 100)
                    self.assertTrue(all(type(ns) is int and ns >= 0
                                        for ns in row['notify_to_return_ns']))
                    observations = [
                        {'sample_kind': row['sample_kind'],
                         'registration_observed': row['registration_observed'],
                         'outcome': row['outcome'], 'notify_to_return_ns': ns}
                        for ns in row['notify_to_return_ns']
                    ]
                    summary = thread_workloads.summarize_latency(observations)
                    self.assertEqual(summary['sample_count'], 100)
                    self.assertEqual(summary['p99_status'], 'reported')
                    self.assertEqual(
                        summary['p99_ns'],
                        sorted(row['notify_to_return_ns'])[98],
                    )
                    self.assertEqual(len(summary['samples_ns']), 100)
                elif workload == 'threads_sc_store_buffering':
                    row = self.observe('--sc-litmus', self.wasm[fixture], 1024, 73,
                                       timeout=60)
                    self.assertEqual(row['case'], workload)
                    self.assertEqual(row['trials'], 1024)
                    self.assertEqual(row['memories_per_trial'], 2)
                    self.assertTrue(row['reset_per_trial'])
                    results = row['trial_results']
                    self.assertEqual(len(results), 1024)
                    self.assertTrue(all(len(pair) == 2 for pair in results))
                    self.assertTrue(all(value in (0, 1) for pair in results for value in pair))
                    self.assertTrue(all(pair != [0, 0] for pair in results))
                    self.assertEqual(row['forbidden_result'], [0, 0])
                    self.assertTrue(all(type(ns) is int and ns > 0
                                        for ns in row['trial_wall_ns']))
                    self.assertEqual(len(row['trial_seeds']), 1024)
                else:
                    trials = 1
                    row = self.observe(
                        '--thread-case', self.wasm[fixture], workload,
                        requested, iterations, trials, 0x1301,
                    )
                    self.assertEqual(row['case'], workload)
                    self.assertEqual(row['requested_workers'], requested)
                    effective = thread_workloads.effective_workers(
                        requested, row['worker_quota']
                    )
                    self.assertEqual(row['effective_workers'], effective)
                    self.assertEqual(row['iterations_per_worker'], iterations)
                    self.assertEqual(row['total_operations_per_trial'], effective * iterations)
                    oracle = thread_workloads.oracle(workload, effective, iterations)
                    self.assertEqual(row['trials'], trials)
                    self.assertEqual(row['trial_worker_returns'], [oracle['worker_returns']])
                    self.assertEqual(row['trial_final_memory_values'], [oracle['final_memory_values']])
                    self.assertEqual(len(row['trial_wall_ns']), trials)
                    self.assertGreater(row['trial_wall_ns'][0], 0)
                    self.assertEqual(len(row['trial_seeds']), trials)
                    self.assertEqual(row['timing_boundary'],
                                     'start_barrier_to_last_worker_result_receive; setup/readback/join excluded')
                observed.add(workload)
        self.assertEqual(observed, {entry['id'] for entry in entries})

    def test_wait_cleanup_controls_timeout_and_cancellation_observations(self):
        fixture = self.wasm['wait_notify.wat']
        row = self.observe('--wait-cleanup', fixture, 1_000_000, timeout=10)
        self.assertEqual(row['case'], 'threads_wait_cleanup')
        self.assertEqual(row['timeout_result'], thread_workloads.wait_oracle('timeout_cleanup')['wait_result'])
        self.assertEqual(row['timeout_notify_after'],
                         thread_workloads.wait_oracle('timeout_cleanup')['notify_after_cleanup'])
        self.assertEqual(row['cancellation_error_kind'], 'ResourceLimit')
        self.assertIn(row['cancellation_message'], {'execution cancelled', 'execution cancelled during atomic wait'})
        self.assertEqual(row['cancellation_notify_after'],
                         thread_workloads.wait_oracle('cancel_cleanup')['notify_after_cleanup'])
        self.assertIs(row['registration_observer_available'], False)
        # Cancellation may race ahead of actual queue registration; the latency
        # case above separately proves a real registration via successful notify(1).
        self.assertNotIn('cancellation_registration_observed', row)

    def test_invalid_thread_and_wait_settings_fail_without_hanging(self):
        atomic = self.wasm['atomic_add.wat']
        cases = (
            ('--thread-case', atomic, 'threads_atomic_add_same_w2_n32', 3, 32, 1, 4),
            ('--thread-case', atomic, 'threads_atomic_add_same_w1_n32', 1, 32, 0, 4),
            ('--wait-latency', self.wasm['wait_notify.wat'], 99, 1_000_000),
            ('--wait-latency', self.wasm['wait_notify.wat'], 100, 0),
            ('--wait-cleanup', self.wasm['wait_notify.wat'], 0),
        )
        for args in cases:
            with self.subTest(args=args):
                process = self.run_driver(*args, timeout=5)
                self.assertNotEqual(process.returncode, 0)
                self.assertTrue(process.stderr.strip(), 'invalid settings need a diagnostic')

    def test_fixture_sources_and_encoded_hashes_match_registry(self):
        for entry in self.entries:
            source = BENCH_ROOT / entry['source']['path']
            filename = source.name
            with self.subTest(workload=entry['id']):
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                                 entry['source']['sha256'])
                self.assertEqual(hashlib.sha256(self.wasm[filename].read_bytes()).hexdigest(),
                                 entry['source']['wasm_sha256'])


if __name__ == '__main__':
    unittest.main()
