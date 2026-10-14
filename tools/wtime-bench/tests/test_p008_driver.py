"""Subprocess contract tests for the P008 explicit scalar drivers."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
ENCODER = ROOT / 'target' / 'debug' / 'wtime-conformance'
DRIVER_SOURCE = ROOT / 'tools' / 'wtime-bench' / 'src' / 'scalar_driver.rs'


class ScalarDriverProcessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not ENCODER.is_file():
            raise RuntimeError('build the conformance encoder before driver tests')
        if not DRIVER_SOURCE.is_file():
            raise RuntimeError('scalar_driver.rs is missing')
        supplied = os.environ.get('WTIME_BENCH_BINARY')
        if supplied:
            cls.exe = Path(supplied).resolve()
            if not cls.exe.is_file():
                raise RuntimeError('WTIME_BENCH_BINARY is missing')
            return
        cls.build_dir = tempfile.TemporaryDirectory(prefix='p008-scalar-driver-build-')
        build_root = Path(cls.build_dir.name)
        manifest = build_root / 'Cargo.toml'
        escaped_driver = str(DRIVER_SOURCE).replace('\\', '\\\\').replace('"', '\\"')
        manifest.write_text(f'''[package]
name = "p008-scalar-driver-test"
version = "0.1.0"
edition = "2024"
[workspace]
[features]
default = []
legacy-api = []
execution-reference = ["wtime-runtime/execution-reference"]
[dependencies]
wtime-runtime = {{ path = "{ROOT}/crates/wtime-runtime" }}
wtime-types = {{ path = "{ROOT}/crates/wtime-types" }}
''')
        (build_root / 'src').mkdir()
        (build_root / 'src' / 'main.rs').write_text(f'''#[path = "{escaped_driver}"]
mod scalar_driver;
fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(scalar_driver::try_main(&args), "unsupported scalar driver command");
}}
''')
        cls.exe = build_root / 'target' / 'release' / 'p008-scalar-driver-test'
        env = os.environ.copy()
        env.setdefault('CARGO_NET_OFFLINE', 'true')
        subprocess.run(
            ['cargo', 'build', '--offline', '--release', '--features', 'execution-reference', '--manifest-path', str(manifest)],
            cwd=ROOT,
            env=env,
            check=True,
            text=True,
            timeout=300,
        )

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'build_dir'):
            cls.build_dir.cleanup()

    def encode(self, tempdir, name, wat):
        source = Path(tempdir) / f'{name}.wat'
        wasm = Path(tempdir) / f'{name}.wasm'
        source.write_text(wat)
        subprocess.run([str(ENCODER), 'encode', str(source), '-o', str(wasm)],
                       check=True, capture_output=True, text=True, timeout=30)
        return wasm

    def run_driver(self, wasm, driver, n, expected, profile='mvp', stage='warm_call', batch=2):
        proc = subprocess.run(
            [str(self.exe), '--scalar-driver', str(wasm), 'p008_case', str(n), str(expected),
             stage, str(batch), 'decoded', profile, driver],
            check=True,
            capture_output=True,
            text=True,
            timeout=20,
        )
        return json.loads(proc.stdout)

    def test_host_tick_import_is_bound_before_timing_and_matches_oracle(self):
        with tempfile.TemporaryDirectory() as directory:
            wasm = self.encode(directory, 'host_tick', '''(module
              (import "host" "tick" (func $tick (param i32) (result i32)))
              (func (export "run") (param $n i32) (result i32)
                (local $i i32) (local $sum i32)
                (block $done (loop $loop
                  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
                  (local.set $sum (call $tick (local.get $sum)))
                  (local.set $i (i32.add (local.get $i) (i32.const 1)))
                  (br $loop)))
                (local.get $sum)))''')
            row = self.run_driver(wasm, 'host_tick', 17, 17)
            self.assertEqual(row['result'], 'i32:17')
            self.assertEqual(row['driver'], 'host_tick')
            self.assertFalse(row['diagnostic'])
            self.assertGreaterEqual(row['batch'], 1)
            self.assertGreater(row['ns_per_operation'], 0)

    def test_table_targets_change_result_and_setup_is_outside_warm_batch(self):
        wasm = ROOT / 'tools/wtime-bench/fixtures/scalar/table_controls.wat'
        with tempfile.TemporaryDirectory() as directory:
            encoded = Path(directory) / 'table_controls.wasm'
            subprocess.run([str(ENCODER), 'encode', str(wasm), '-o', str(encoded)],
                           check=True, capture_output=True, text=True, timeout=30)
            result_a = self.run_driver(encoded, 'table_a', 17, 18, profile='all-phase3-core')
            result_b = self.run_driver(encoded, 'table_b', 17, 262143, profile='all-phase3-core')
            self.assertEqual(result_a['result'], 'i32:18')
            self.assertEqual(result_b['result'], 'i32:262143')
            self.assertEqual(result_a['driver'], 'table_a')
            self.assertEqual(result_b['driver'], 'table_b')

    def test_same_instance_table_mutation_changes_next_result_and_restores(self):
        wat = ROOT / 'tools/wtime-bench/fixtures/scalar/table_controls.wat'
        with tempfile.TemporaryDirectory() as directory:
            encoded = Path(directory) / 'table_controls.wasm'
            subprocess.run([str(ENCODER), 'encode', str(wat), '-o', str(encoded)],
                           check=True, capture_output=True, text=True, timeout=30)
            proc = subprocess.run(
                [str(self.exe), '--scalar-table-probe', str(encoded), '17', '18', '262143',
                 'decoded', 'all-phase3-core'],
                check=True, capture_output=True, text=True, timeout=20,
            )
            row = json.loads(proc.stdout)
            self.assertTrue(row['diagnostic'])
            self.assertEqual(row['status'], 'mutation_verified')
            self.assertEqual(row['target_a'], 'i32:18')
            self.assertEqual(row['target_b'], 'i32:262143')
            self.assertEqual(row['target_a_after_restore'], 'i32:18')
            self.assertNotIn('ns_per_operation', row)

    def test_null_and_mismatched_table_probes_report_real_trap_kinds(self):
        wasm = ROOT / 'tools/wtime-bench/fixtures/scalar/table_controls.wat'
        with tempfile.TemporaryDirectory() as directory:
            encoded = Path(directory) / 'table_controls.wasm'
            subprocess.run([str(ENCODER), 'encode', str(wasm), '-o', str(encoded)],
                           check=True, capture_output=True, text=True, timeout=30)
            null = self.run_driver(encoded, 'probe_null', 0, 0, profile='all-phase3-core')
            mismatch = self.run_driver(encoded, 'probe_mismatch', 0, 0, profile='all-phase3-core')
            self.assertTrue(null['diagnostic'])
            self.assertEqual(null['trap_kind'], 'NullReference')
            self.assertIn('null indirect call', null['message'])
            self.assertTrue(mismatch['diagnostic'])
            self.assertEqual(mismatch['trap_kind'], 'IndirectCallTypeMismatch')
            self.assertIn('indirect call type mismatch', mismatch['message'])
            self.assertNotIn('ns_per_operation', null)
            self.assertNotIn('ns_per_operation', mismatch)

    def test_depth_probe_requires_call_depth_resource_limit_at_both_sizes(self):
        with tempfile.TemporaryDirectory() as directory:
            wasm = self.encode(directory, 'recursive', '''(module
              (func $walk (param $n i32) (result i32)
                (if (result i32) (i32.eqz (local.get $n))
                  (then (i32.const 7))
                  (else (call $walk (i32.sub (local.get $n) (i32.const 1))))))
              (func (export "run") (param i32) (result i32) (call $walk (local.get 0))))''')
            for depth in (1024, 10000):
                with self.subTest(depth=depth):
                    row = self.run_driver(wasm, 'depth_probe', depth, 0)
                    self.assertTrue(row['diagnostic'])
                    self.assertEqual(row['status'], 'trap_verified')
                    self.assertEqual(row['trap_kind'], 'ResourceLimit')
                    self.assertIn('call depth', row['message'].lower())
                    self.assertNotIn('ns_per_operation', row)

    def test_tail_call_succeeds_at_depth_10000_under_same_low_limit(self):
        wat = ROOT / 'tools/wtime-bench/fixtures/scalar/tail_calls.wat'
        with tempfile.TemporaryDirectory() as directory:
            wasm = Path(directory) / 'tail_calls.wasm'
            subprocess.run([str(ENCODER), 'encode', str(wat), '-o', str(wasm)],
                           check=True, capture_output=True, text=True, timeout=30)
            n = 10000
            value = 1
            for _ in range(n):
                value = (value * 3 + 1) & 0xFFFFFFFF
            expected = value - 0x100000000 if value & 0x80000000 else value
            row = self.run_driver(wasm, 'tail_probe', n, expected,
                                  profile='all-phase3-core')
            self.assertTrue(row['diagnostic'])
            self.assertEqual(row['status'], 'returned')
            self.assertEqual(row['result'], f'i32:{expected}')
            self.assertNotIn('ns_per_operation', row)

    def test_invalid_policy_is_a_nonzero_configuration_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            wasm = self.encode(directory, 'plain', '(module (func (export "run") (param i32) (result i32) local.get 0))')
            proc = subprocess.run(
                [str(self.exe), '--scalar-driver', str(wasm), 'p008_case', '0', '0',
                 'first_call', '1', 'unknown-mode', 'mvp', 'plain'],
                capture_output=True, text=True, timeout=20,
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertNotIn('"status":"unsupported"', proc.stdout)


if __name__ == '__main__':
    unittest.main()
