"""Focused staged regression tests for benchmark source identity coverage."""
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

BENCH = pathlib.Path(os.environ.get('WTIME_BENCH_TOOL_DIR', pathlib.Path(__file__).resolve().parents[1])).resolve()
REPO_ROOT = BENCH.parents[1]
CHECKS = REPO_ROOT / 'tools/checks'
sys.path.insert(0, str(CHECKS))
sys.path.insert(0, str(BENCH))
import baseline
import compare
import workloads
import gates

class SourceBindingTests(unittest.TestCase):
    def test_compare_sources_covers_imported_helpers_toolchain_and_nested_rust(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            bench = root / 'tools/wtime-bench'
            conformance = root / 'tools/wtime-conformance'
            for rel, data in {
                'Cargo.toml': 'workspace', 'Cargo.lock': 'lock', 'rust-toolchain.toml': 'toolchain',
                '.cargo/config.toml': 'build config', 'crates/core/Cargo.toml': 'crate manifest',
                'crates/core/src/lib.rs': 'runtime', 'tools/wtime-bench/Cargo.toml': 'bench manifest',
                'tools/wtime-bench/Cargo.lock': 'bench lock', 'tools/wtime-bench/workloads.json': '{}',
                'tools/wtime-bench/workloads.py': 'schema', 'tools/wtime-bench/baseline.py': 'archive helper',
                'tools/wtime-bench/run.py': 'runner', 'tools/wtime-bench/imported_helper.py': 'helper',
                'tools/wtime-bench/tests/test_ignore.py': 'test only',
                'tools/wtime-bench/src/nested/deep_driver.rs': 'nested harness source',
                'tools/wtime-bench/fixtures/scalar/a.wat': 'scalar source',
                'tools/wtime-bench/fixtures/memory/a.wat': 'memory source',
                'tools/wtime-conformance/Cargo.toml': 'encoder manifest',
                'tools/wtime-conformance/src/bin/deep.rs': 'encoder source',
            }.items():
                path = root / rel; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(data)
            with mock.patch.object(compare, 'ROOT', root), mock.patch.object(compare, 'HERE', bench):
                default = compare.sources()
                memory = compare.sources(include_memory=True)
            expected = {
                'tools/wtime-bench/workloads.py', 'tools/wtime-bench/baseline.py',
                'tools/wtime-bench/run.py', 'tools/wtime-bench/imported_helper.py',
                'tools/wtime-bench/src/nested/deep_driver.rs',
                'tools/wtime-conformance/src/bin/deep.rs', 'rust-toolchain.toml',
                '.cargo/config.toml', 'tools/wtime-bench/workloads.json',
            }
            self.assertTrue(expected.issubset(default))
            self.assertNotIn('tools/wtime-bench/tests/test_ignore.py', default)
            self.assertNotIn('tools/wtime-bench/fixtures/memory/a.wat', default)
            self.assertIn('tools/wtime-bench/fixtures/memory/a.wat', memory)
            self.assertEqual(default['tools/wtime-bench/workloads.py'], memory['tools/wtime-bench/workloads.py'])

    def test_fresh_archive_inventory_captures_workload_schema_and_registry(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            paths = [
                'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'tools/wtime-bench/Cargo.toml',
                'tools/wtime-bench/Cargo.lock', 'tools/wtime-bench/README.md',
                'tools/wtime-bench/run.py', 'tools/wtime-bench/baseline.py',
                'tools/wtime-bench/workloads.py', 'tools/wtime-bench/workloads.json',
                'tools/wtime-bench/fixtures/scalar/a.wat', 'tools/wtime-conformance/src/nested/a.rs',
                'tools/wtime-bench/src/nested/driver.rs',
            ]
            for rel in paths:
                path=root/rel; path.parent.mkdir(parents=True, exist_ok=True); path.write_text('x')
            found={p.relative_to(root).as_posix() for p in baseline._source_paths(root)}
            required={
                'tools/wtime-bench/workloads.py','tools/wtime-bench/workloads.json',
                'tools/wtime-bench/baseline.py','tools/wtime-bench/run.py',
                'tools/wtime-conformance/src/nested/a.rs','tools/wtime-bench/src/nested/driver.rs',
                'rust-toolchain.toml',
            }
            self.assertTrue(required.issubset(found))

    def test_selection_memory_detection_uses_selected_oracle_module(self):
        with mock.patch.object(workloads, 'selection', return_value=[
            {'oracle': {'module': 'scalar_workloads.py'}},
            {'oracle': {'module': 'memory_workloads.py'}},
        ]):
            self.assertTrue(gates.compare.selection_uses_oracle_module('mixed-memory', 'memory_workloads.py'))
            self.assertFalse(gates.compare.selection_uses_oracle_module('mixed-memory', 'simd_workloads.py'))
        self.assertFalse(gates.compare.selection_uses_oracle_module(None, 'memory_workloads.py'))

    def test_gate_passes_derived_memory_flag_and_none_keeps_default_off(self):
        with mock.patch.object(workloads, 'selection', return_value=[
            {'oracle': {'module': 'memory_workloads.py'}}
        ]), mock.patch.object(gates.compare, 'sources', return_value={'ok': 'hash'}) as sources:
            self.assertEqual(gates._current_source_identity('custom-selection'), {'ok': 'hash'})
            sources.assert_called_once_with(include_memory=True, include_simd=False)
        with mock.patch.object(gates.compare, 'sources', return_value={'ok': 'hash'}) as sources:
            self.assertEqual(gates._current_source_identity(None), {'ok': 'hash'})
            sources.assert_called_once_with(include_memory=False, include_simd=False)

    def test_workload_schema_change_changes_source_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp); bench=root/'tools/wtime-bench'
            paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','tools/wtime-bench/Cargo.toml',
                   'tools/wtime-bench/Cargo.lock','tools/wtime-bench/workloads.json',
                   'tools/wtime-bench/workloads.py','tools/wtime-bench/helper.py']
            for rel in paths:
                path=root/rel; path.parent.mkdir(parents=True,exist_ok=True); path.write_text(rel)
            with mock.patch.object(compare,'ROOT',root), mock.patch.object(compare,'HERE',bench):
                before=compare.sources()
                (bench/'workloads.py').write_text('changed schema bytes')
                after=compare.sources()
            self.assertNotEqual(before['tools/wtime-bench/workloads.py'],after['tools/wtime-bench/workloads.py'])
            self.assertEqual(before['tools/wtime-bench/helper.py'],after['tools/wtime-bench/helper.py'])

if __name__ == '__main__':
    unittest.main()
