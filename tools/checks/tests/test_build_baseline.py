import importlib.util
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
spec=importlib.util.spec_from_file_location('build',Path(__file__).parents[1]/'build_budget.py')
build=importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
class Build(unittest.TestCase):
    def test_t009_failure_propagates(self):
        with self.assertRaises(subprocess.CalledProcessError) as caught:
            build.run([sys.executable,'-c','raise SystemExit(7)'],env=os.environ)
        self.assertEqual(caught.exception.returncode,7)
    def test_t009_independent_target(self):
        with tempfile.TemporaryDirectory() as path:
            self.assertEqual(list(Path(path).iterdir()),[])
            self.assertNotEqual(Path(path),build.ROOT/'target')
    def test_t009_monotonic_duration(self):
        self.assertGreaterEqual(build.run([sys.executable,'-c','pass'],env=os.environ),0)

    def _workspace(self, root):
        (root/'Cargo.toml').write_text('[workspace]\nmembers = ["crates/wtime-cli", "crates/wtime-runtime"]\n')
        (root/'Cargo.lock').write_text('# fixture lock\n')
        paths = {}
        for name, source in [('wtime-cli', 'main.rs'), ('wtime-runtime', 'numeric.rs')]:
            package = root/'crates'/name
            (package/'src').mkdir(parents=True)
            manifest = package/'Cargo.toml'
            manifest.write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\n')
            leaf = package/'src'/source
            leaf.write_text('fn main() {}\n')
            paths[name] = leaf
        metadata = {'packages': [
            {'id': 'cli', 'name': 'wtime-cli', 'source': None,
             'manifest_path': str(root/'crates/wtime-cli/Cargo.toml'), 'features': {}},
            {'id': 'runtime', 'name': 'wtime-runtime', 'source': None,
             'manifest_path': str(root/'crates/wtime-runtime/Cargo.toml'), 'features': {}},
        ], 'workspace_members': ['cli', 'runtime'], 'resolve': {'nodes': []}}
        return metadata, paths

    def test_measure_builds_only_in_isolated_source_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'repo'
            root.mkdir()
            metadata, leaves = self._workspace(root)
            before = {name: path.read_bytes() for name, path in leaves.items()}
            before_stat = {name: (path.stat().st_mtime_ns, path.stat().st_ctime_ns)
                           for name, path in leaves.items()}
            output = root/'report.json'
            observations = []

            def measured(command, *, env, cwd=build.ROOT, rss_path=None):
                cwd = Path(cwd)
                self.assertNotEqual(cwd, root)
                self.assertTrue((cwd/'Cargo.toml').is_file())
                self.assertEqual({name: path.read_bytes() for name, path in leaves.items()}, before)
                isolated = {name: (cwd/path.relative_to(root)).read_bytes()
                            for name, path in leaves.items()}
                observations.append(isolated)
                self.assertEqual(Path(env['CARGO_TARGET_DIR']).parent, cwd.parent)
                if rss_path is not None:
                    Path(rss_path).write_text('123')
                if '--release' in command:
                    executable = Path(env['CARGO_TARGET_DIR'])/'release/wtime-cli'
                    executable.parent.mkdir(parents=True, exist_ok=True)
                    executable.write_bytes(b'fake executable')
                return 0.01

            def subprocess_run(command, **kwargs):
                return subprocess.CompletedProcess(command, 0)

            with mock.patch.object(build, 'ROOT', root), \
                 mock.patch.object(build, 'cargo_metadata', return_value=metadata), \
                 mock.patch.object(build, '_default_dependency_result', return_value=(True, [])), \
                 mock.patch.object(build, 'run', side_effect=measured), \
                 mock.patch.object(build.subprocess, 'run', side_effect=subprocess_run), \
                 mock.patch.object(build.subprocess, 'check_output',
                                   side_effect=lambda command, **kwargs: {
                                       'cargo': 'cargo 1.0', 'rustc': 'rustc 1.0'
                                   }.get(command[0], 'fixture-cpu')):
                record = build.measure(output)

            self.assertEqual(len(observations), 6)
            self.assertIn(b'// Build measurement touch.', observations[2]['wtime-cli'])
            self.assertIn(b'// Build measurement touch.', observations[3]['wtime-runtime'])
            self.assertTrue(record['source_stable'])
            self.assertTrue(record['target_initially_empty'])
            self.assertEqual({name: path.read_bytes() for name, path in leaves.items()}, before)
            self.assertEqual({name: (path.stat().st_mtime_ns, path.stat().st_ctime_ns)
                              for name, path in leaves.items()}, before_stat)

    def test_staging_rejects_source_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'repo'
            root.mkdir()
            metadata, leaves = self._workspace(root)
            original_root = build.ROOT
            build.ROOT = root
            try:
                inputs = build.source_inputs(metadata)
                copy2 = build.shutil.copy2

                def copy_then_drift(source, destination):
                    result = copy2(source, destination)
                    if Path(source) == leaves['wtime-cli']:
                        Path(source).write_bytes(Path(source).read_bytes()+b'// drift\n')
                    return result

                with mock.patch.object(build.shutil, 'copy2', side_effect=copy_then_drift):
                    with self.assertRaisesRegex(ValueError, 'source drift'):
                        build._stage_workspace(metadata, inputs, root.parent/'staged')
            finally:
                build.ROOT = original_root
