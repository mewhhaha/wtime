"""Sequential phases, exact executed checks, and reuse by declared input identities."""
from __future__ import annotations
import os
import hashlib
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import tomllib
from functools import wraps
from .artifacts import ArtifactStore, VerificationError, digest, need, read_json, sha, write_new
from .contracts import PHASES, check_json, expand, identity, json_pointer, load_spec, output_paths, validate_delta
from .context import current, fingerprint, operation


def scoped(method):
    @wraps(method)
    def wrapped(self, *args, **kwargs):
        ctx = None
        try:
            with operation() as ctx:
                return method(self, *args, **kwargs)
        finally:
            if ctx:
                self.last_metrics = ctx.summary()
    return wrapped


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.index-', dir=path.parent)
    os.close(fd)
    temp = Path(name)
    try:
        temp.unlink()
        write_new(temp, value)
        os.replace(temp, path)
        if current():
            current().written(path)
    finally:
        temp.unlink(missing_ok=True)


def execution_environment(repo, stage):
    names = {'PATH', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'RUSTUP_HOME',
             'CARGO_HOME', 'CARGO_BUILD_TARGET', 'CARGO_TARGET_DIR', 'CARGO_INCREMENTAL',
             'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CC', 'CXX', 'AR', 'LD',
             'CFLAGS', 'CPPFLAGS', 'CXXFLAGS', 'LDFLAGS', 'LD_LIBRARY_PATH', 'LD_PRELOAD',
             'LANG', 'LC_ALL', 'LANGUAGE', 'PYTHONPATH', 'PYTHONHOME', 'PYTHONHASHSEED'}
    names.update(name for name in os.environ if name.startswith(('CARGO_PROFILE_', 'CARGO_BUILD_', 'CARGO_TARGET_')))
    names.update(stage.get('environment', []))
    # Store names and hashes only; never persist environment values.
    values = {name: hashlib.sha256(os.environ[name].encode()).hexdigest() if name in os.environ else None
              for name in sorted(names)}
    toolchain = {}
    for name in ('RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CC', 'CXX', 'AR', 'LD'):
        if os.environ.get(name):
            args = shlex.split(os.environ[name])
            path = shutil.which(args[0]) if args else None
            need(path is not None, 'declared compiler/linker wrapper is unavailable: ' + name)
            toolchain[str(Path(path).resolve())] = sha(Path(path).resolve())
    cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    for directory in (repo / '.cargo', cargo_home):
        for name in ('config', 'config.toml'):
            path = directory / name
            if path.is_file():
                toolchain[str(path.resolve())] = sha(path)
    if any(Path(command['argv'][0]).name == 'cargo' for command in stage['commands']):
        config = repo / 'rust-toolchain.toml'
        channel = os.environ.get('RUSTUP_TOOLCHAIN')
        if config.is_file():
            toolchain['rust-toolchain.toml'] = sha(config)
            channel = channel or tomllib.loads(config.read_text()).get('toolchain', {}).get('channel')
        home = Path(os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')))
        settings = home / 'settings.toml'
        if not channel and settings.is_file():
            channel = tomllib.loads(settings.read_text()).get('default_toolchain')
        if channel:
            dirs = [home / 'toolchains' / channel]
            dirs += sorted((home / 'toolchains').glob(channel + '-*'))
            selected = next((p for p in dirs if (p / 'bin/rustc').is_file()), None)
            need(selected is not None, 'selected Rust toolchain is unavailable; activate the setup environment')
            for name in ('rustc', 'cargo'):
                path = selected / 'bin' / name
                toolchain[str(path)] = sha(path)
    return {'variables': values, 'toolchain': toolchain}


def test_results(text, group):
    if group.get('kind') == 'unittest':
        rows = re.findall(r'^\w+\s+\(([^)]+)\)\s+\.\.\.\s+(ok|FAIL|ERROR|skipped)(?:\s|$)', text, re.M)
        names = [name for name, state in rows if state == 'ok']
        totals = re.findall(r'^Ran (\d+) tests? in ', text, re.M)
        need(rows and len(names) == len(set(names)) and all(state == 'ok' for _, state in rows),
             'failed, skipped, duplicate, or missing unittest outcome')
        need(len(totals) == 1 and int(totals[0]) == len(names) and re.search(r'^OK\s*$', text, re.M),
             'unittest totals differ from named outcomes')
        need(set(group['required_names']) <= set(names) | {n.rsplit('.', 1)[-1] for n in names}
             and len(names) >= group.get('minimum_passed', 1), 'required unittest did not execute')
        return {'passed': len(names), 'failed': 0, 'ignored': 0, 'names': names,
                'required_names': sorted(group['required_names']), 'features': []}
    rows = re.findall(r'^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)(?:\s|$)', text, re.M)
    names = [name for name, state in rows if state == 'ok']
    need(len(names) == len(set(names)), 'duplicate executed test name')
    leaves = {name.rsplit('::', 1)[-1] for name in names}
    required = set(group['required_names'])
    need(required <= set(names) | leaves, 'required named test did not execute')
    need(rows and all(state == 'ok' for _, state in rows), 'failed or ignored named test')
    summaries = [tuple(map(int, values)) for values in re.findall(
        r'test result: ok\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;', text)]
    need(summaries and sum(v[0] for v in summaries) == len(names)
         and all(v[1] == v[2] == 0 for v in summaries), 'Cargo summaries differ from actual named outcomes')
    need(len(names) >= group.get('minimum_passed', 1), 'too few executed tests')
    if group.get('unfiltered'):
        filtered = [int(v) for v in re.findall(r'(\d+) filtered out', text)]
        need(filtered and not any(filtered), 'required full suite was filtered')
    return {'passed': len(names), 'failed': 0, 'ignored': 0, 'names': names,
            'required_names': sorted(required), 'features': group.get('features', [])}


class Workflow:
    def __init__(self, repo, store_root, *, lane='full'):
        self.repo = Path(repo).resolve(strict=True)
        self.root = Path(store_root).resolve()
        self._store = None
        need(lane in ('full', 'development'), 'unknown verification lane')
        self.lane = lane
        self.last_metrics = {}

    @property
    def store(self):
        if self._store is None:
            self._store = ArtifactStore(self.root)
        return self._store

    def _identities(self, spec, phase):
        return {name: identity(self.repo, spec['identities'][name])
                for name in spec['stages'][phase]['uses']}

    def _state_path(self, task, phase):
        return self.root / 'state' / task / (phase + '.json')

    def _receipt(self, task, phase):
        path = self._state_path(task, phase)
        if current():
            return current().cached('receipts', (str(path), self.lane, fingerprint(path)),
                                    lambda: self._read_receipt(task, phase))
        return self._read_receipt(task, phase)

    def _read_receipt(self, task, phase):
        path = self._state_path(task, phase)
        need(path.is_file(), 'required phase has no successful receipt: ' + phase)
        ref = read_json(path)
        value = self.store.json(ref)
        need(value.get('schema') == 'wtime-verification-phase-v1' and value.get('task') == task
             and value.get('phase') == phase and value.get('status') == 'passed', 'invalid phase receipt')
        need(value.get('lane', 'full') == self.lane, 'receipt belongs to a different verification lane')
        for artifact in value.get('artifacts', {}).values():
            self.store.verify(artifact)
        return ref, value

    def _inputs(self, spec, phase):
        if current():
            return current().cached('phase_inputs', (str(self.root), self.lane, digest(spec), phase),
                                    lambda: self._compute_inputs(spec, phase))
        return self._compute_inputs(spec, phase)

    def _compute_inputs(self, spec, phase):
        stage = spec['stages'][phase]
        dependencies = {}
        for dep in stage.get('dependencies', []):
            ref, receipt = self._receipt(spec['task'], dep)
            need(receipt['input_key'] == digest(self._inputs(spec, dep)), 'dependency phase is stale: ' + dep)
            dependencies[dep] = ref
        runner_sources = {p.name: sha(p) for p in (Path(__file__), Path(__file__).with_name('contracts.py'))}
        _, planned = self._preflight(spec, phase, self.root / 'runs' / spec['task'] / phase / 'preflight')
        tools = {str(p): sha(p) for _, _, helpers in planned for p in helpers}
        source_delta = None
        if phase != 'freeze' and 'source_delta' in spec and spec['source_delta']['identity'] in stage['uses']:
            rule = spec['source_delta']
            baseline = Path(expand(rule['baseline'], self.repo, self.root, self._context(spec, phase)))
            source_delta = {'declaration': rule, 'baseline_sha256': sha(baseline),
                            'delta': self._source_delta(spec, phase)}
        return {'components': self._identities(spec, phase), 'dependencies': dependencies, 'lane': self.lane,
                'definition': stage, 'runner_sources': runner_sources, 'tools': tools,
                'source_delta': source_delta, 'environment': execution_environment(self.repo, stage)}

    def _context(self, spec, phase, planning=False):
        context = {}
        for dep in spec['stages'][phase].get('dependencies', []):
            if planning:
                context[dep + '_dir'] = str(self.root / 'runs' / spec['task'] / dep / 'preflight')
            else:
                _, receipt = self._receipt(spec['task'], dep)
                for ref in receipt.get('original_outputs', {}).values():
                    from .artifacts import verify_normal
                    verify_normal(ref)
                context[dep + '_dir'] = receipt['attempt']
        return context

    def _preflight(self, spec, phase, directory, planning=False):
        stage = spec['stages'][phase]
        context = self._context(spec, phase, planning)
        outputs = output_paths(stage, self.repo, directory, context)
        planned = []
        for command in stage['commands']:
            argv = [expand(arg, self.repo, directory, context) for arg in command['argv']]
            executable = shutil.which(argv[0])
            need(executable and os.access(executable, os.X_OK), 'command executable is missing: ' + argv[0])
            helpers = [Path(executable).resolve()]
            for arg in argv[1:]:
                p = Path(arg)
                if p.suffix in ('.py', '.rs') and p.is_absolute() and not p.is_relative_to(directory):
                    need(p.is_file(), 'command source is missing: ' + arg)
                    helpers.append(p.resolve())
            if self.lane == 'development' and not spec.get('synthetic', False) and Path(argv[0]).name == 'cargo':
                selected = execution_environment(self.repo, stage)['toolchain']
                cargo_paths = {Path(p).resolve() for p in selected if p.endswith('/bin/cargo')}
                cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
                dispatcher = cargo_home / 'bin/rustup'
                if dispatcher.is_file():
                    cargo_paths.add(dispatcher.resolve())
                need(Path(executable).resolve() in cargo_paths, 'development Cargo command is not the selected toolchain Cargo')
            for group in stage.get('tests', []):
                if group['command'] == command['name']:
                    if not spec.get('synthetic', False) and group.get('kind', 'cargo') == 'cargo':
                        need(Path(argv[0]).name == 'cargo' and len(argv) > 1 and argv[1] == 'test',
                             'production named-test evidence must come from Cargo test')
                        selected = execution_environment(self.repo, stage)['toolchain']
                        cargo_paths = {Path(p).resolve() for p in selected if p.endswith('/bin/cargo')}
                        cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
                        dispatcher = cargo_home / 'bin/rustup'
                        if dispatcher.is_file():
                            cargo_paths.add(dispatcher.resolve())
                        need(cargo_paths and Path(executable).resolve() in cargo_paths,
                             'production test command is not the selected toolchain Cargo')
                    if not spec.get('synthetic', False) and group.get('kind') == 'unittest':
                        need(Path(executable).resolve() == Path(__import__('sys').executable).resolve()
                             and (argv[1:3] == ['-m', 'unittest'] or argv[1:4] == ['-B', '-m', 'unittest'])
                             and '-v' in argv, 'named Python outcomes require verbose unittest')
                    need('--all-features' not in argv and argv.count('--features') <= 1, 'ambiguous test feature configuration')
                    actual = []
                    if '--features' in argv:
                        index = argv.index('--features')
                        need(index + 1 < len(argv), 'test command feature argument is missing')
                        actual = argv[index + 1].split(',')
                    need(set(group.get('features', [])) == set(actual), 'test command feature configuration differs')
            planned.append((command, argv, sorted(set(helpers))))
        for rule in stage.get('checks', []):
            need(rule.get('artifact') in outputs, 'check refers to an undeclared output')
        return outputs, planned

    def _revalidate(self, spec, phase, receipt):
        stage = spec['stages'][phase]
        commands = receipt['commands']
        need(len(commands) == len(stage['commands']), 'cached command count differs')
        by_name = {item['name']: item for item in commands}
        for declaration in stage['commands']:
            item = by_name.get(declaration['name'])
            need(item and item['exit_code'] == 0, 'cached command failed or is missing')
            self.store.verify(item['stdout'])
            self.store.verify(item['stderr'])
            expected = [expand(v, self.repo, Path(receipt['attempt']), self._context(spec, phase)) for v in declaration['argv']]
            need(item['argv'] == expected and item['cwd'] == str(self.repo), 'cached executed command differs')
            if 'expect_stdout' in declaration:
                need(self.store.read(item['stdout']).decode().strip() == declaration['expect_stdout'], 'cached stdout differs')
        for group in stage.get('tests', []):
            item = by_name[group['command']]
            text = self.store.read(item['stdout']).decode(errors='replace') + '\n' + self.store.read(item['stderr']).decode(errors='replace')
            need(test_results(text, group) == receipt['tests'][group['command']], 'cached test evidence differs')
        for rule in stage.get('checks', []):
            check_json(self.store.json(receipt['artifacts'][rule['artifact']]), rule)

    @scoped
    def plan(self, spec_path):
        spec = load_spec(spec_path)
        identities = {name: identity(self.repo, value) for name, value in spec['identities'].items()}
        for phase in spec['stages']:
            directory = self.root / 'runs' / spec['task'] / phase / 'preflight'
            self._preflight(spec, phase, directory, planning=True)
        return {'task': spec['task'], 'phases': list(spec['stages']), 'identities': identities}

    def _source_delta(self, spec, phase):
        if phase == 'freeze' or 'source_delta' not in spec or spec['source_delta']['identity'] not in spec['stages'][phase]['uses']:
            return None
        rule = spec['source_delta']
        baseline_path = Path(expand(rule['baseline'], self.repo, self.root, self._context(spec, phase)))
        baseline = json_pointer(read_json(baseline_path), rule.get('pointer', ''))
        current = identity(self.repo, spec['identities'][rule['identity']])['files']
        return validate_delta(current, baseline, rule.get('allowed_changes', []), rule.get('allowed_additions', []))

    @scoped
    def run(self, spec_path, phase):
        spec_path = Path(spec_path).resolve(strict=True)
        spec = load_spec(spec_path)
        need(bool(spec.get('development_lane')) == (self.lane == 'development'),
             'development contracts cannot produce full verification receipts')
        need(self.lane != 'full' or not spec.get('development_only'),
             'development-only contracts cannot produce full receipts')
        need(phase in spec['stages'], 'phase absent from task contract')
        inputs = self._inputs(spec, phase)
        key = digest(inputs)
        cache = self.root / 'cache' / spec['task'] / phase / (key + '.json')
        if cache.is_file():
            ref = read_json(cache)
            receipt = self.store.json(ref)
            need(receipt.get('input_key') == key and receipt.get('status') == 'passed', 'invalid cached phase')
            need(receipt.get('task') == spec['task'] and receipt.get('phase') == phase
                 and receipt.get('lane', 'full') == self.lane, 'cached receipt task, phase, or lane differs')
            for artifact in receipt['artifacts'].values():
                self.store.verify(artifact)
            self._revalidate(spec, phase, receipt)
            atomic_json(self._state_path(spec['task'], phase), ref)
            return {**receipt, 'receipt': ref, 'reused': True, 'reason': 'inputs_unchanged', 'changed_inputs': []}
        prior = self._state_path(spec['task'], phase)
        previous = self.store.json(read_json(prior))['inputs'] if prior.is_file() else None
        changes = []
        if previous:
            for section in sorted(set(previous) | set(inputs)):
                if previous.get(section) != inputs.get(section):
                    if isinstance(previous.get(section), dict) and isinstance(inputs.get(section), dict):
                        changes.extend(section + ':' + name for name in sorted(set(previous[section]) | set(inputs[section]))
                                       if previous[section].get(name) != inputs[section].get(name))
                    else:
                        changes.append(section)
        base = self.root / 'runs' / spec['task'] / phase
        directory = base / 'attempt-0001'
        number = 1
        while directory.exists():
            number += 1
            directory = base / f'attempt-{number:04d}'
        outputs, planned = self._preflight(spec, phase, directory)
        delta = inputs['source_delta']['delta'] if inputs['source_delta'] else None
        directory.mkdir(parents=True, exist_ok=False)
        write_new(directory / 'inputs.json', inputs)
        artifacts = {'contract': self.store.add(spec_path), 'executed_runner': self.store.add(Path(__file__))}
        for name, component in (inputs['components'].items() if self.lane == 'full' else []):
            for relative, expected in component['files'].items():
                artifacts['input:' + name + ':' + relative] = self.store.add(self.repo / relative, expected)
        helper_guards = {str(p): sha(p) for _, _, helpers in planned for p in helpers}
        for path, expected in helper_guards.items():
            # System interpreters/compilers are identified by hash. Preserve
            # project helper sources and explicitly selected pinned tools.
            retained = {expand(v, self.repo, directory) for v in spec['stages'][phase].get('retain_tools', [])}
            if Path(path).is_relative_to(self.repo) or path in retained:
                artifacts['helper:' + path] = self.store.add(Path(path), expected)
        journal = []
        texts = {}
        try:
            for command, argv, _ in planned:
                label = command['name']
                stdout_path, stderr_path = directory / (label + '.stdout'), directory / (label + '.stderr')
                write_new(directory / (label + '.command.json'), {'argv': argv, 'cwd': str(self.repo)})
                with stdout_path.open('xb') as stdout, stderr_path.open('xb') as stderr:
                    if current():
                        current().count('subprocesses')
                    process = subprocess.run(argv, cwd=self.repo, stdout=stdout, stderr=stderr, check=False)
                text = stdout_path.read_text(errors='replace') + '\n' + stderr_path.read_text(errors='replace')
                texts[label] = text
                record = {'name': label, 'argv': argv, 'cwd': str(self.repo), 'exit_code': process.returncode,
                          'stdout': self.store.add(stdout_path), 'stderr': self.store.add(stderr_path)}
                journal.append(record)
                write_new(directory / (label + '.result.json'), record)
                artifacts['command:' + label] = self.store.add(directory / (label + '.result.json'))
                need(process.returncode == 0, 'command failed: ' + label)
                if 'expect_stdout' in command:
                    need(stdout_path.read_text().strip() == command['expect_stdout'], 'command stdout differs: ' + label)
                for line in stderr_path.read_text(errors='replace').splitlines():
                    match = re.search(r'^\s*Running .*\(([^()]+)\)\s*$', line)
                    if match:
                        executable = Path(match.group(1))
                        executable = executable if executable.is_absolute() else self.repo / executable
                        need(executable.is_file() and os.access(executable, os.X_OK), 'executed Cargo binary is missing')
                        if self.lane == 'full':
                            artifacts['executable:' + label + ':' + executable.name] = self.store.add(executable)
                        else:
                            from .artifacts import normal
                            record.setdefault('development_executables', []).append(normal(executable))
                if self.lane == 'full' and not spec.get('synthetic', False) and any(g['command'] == label and g.get('kind', 'cargo') == 'cargo' for g in spec['stages'][phase].get('tests', [])):
                    need(any(key.startswith('executable:' + label + ':') for key in artifacts),
                         'Cargo named-test evidence lacks executed binary identities')
            need(self._identities(spec, phase) == inputs['components'], 'phase inputs changed during execution')
            need(execution_environment(self.repo, spec['stages'][phase]) == inputs['environment'], 'execution environment/toolchain changed during phase')
            need(all(Path(p).is_file() and sha(p) == expected for p, expected in helper_guards.items()),
                 'executed helper changed during phase')
            results = {}
            for group in spec['stages'][phase].get('tests', []):
                results[group['command']] = test_results(texts[group['command']], group)
            # Validate every output and semantic rule before publishing any success.
            need(all(p.is_file() for p in outputs.values()), 'required output is missing')
            documents = {}
            for rule in spec['stages'][phase].get('checks', []):
                name = rule['artifact']
                if name not in documents:
                    documents[name] = read_json(outputs[name])
                check_json(documents[name], rule)
            artifacts.update(self.store.plan({name: {'path': str(path)} for name, path in outputs.items()}))
            if current():
                current().guard()
            receipt = {'schema': 'wtime-verification-phase-v1', 'task': spec['task'], 'phase': phase,
                       'status': 'passed', 'input_key': key, 'inputs': inputs, 'artifacts': artifacts,
                       'commands': journal, 'tests': results, 'source_delta': delta,
                       'attempt': str(directory), 'timing_evidence': phase == 'measure'}
            receipt['synthetic'] = spec.get('synthetic', False)
            receipt['lane'] = self.lane
            receipt['completion_eligible'] = self.lane == 'full'
            receipt['artifact_reader_sha256'] = sha(Path(__file__).with_name('artifacts.py'))
            from .artifacts import normal
            receipt['original_outputs'] = {name: normal(path) for name, path in outputs.items()}
            write_new(directory / 'receipt.json', receipt)
            ref = self.store.add(directory / 'receipt.json')
            write_new(cache, ref)
            atomic_json(self._state_path(spec['task'], phase), ref)
            return {**receipt, 'receipt': ref, 'reused': False,
                    'reason': 'inputs_changed' if previous else 'no_successful_receipt', 'changed_inputs': changes}
        except BaseException as error:
            write_new(directory / 'failure.json', {'status': 'failed', 'error': type(error).__name__ + ': ' + str(error),
                      'input_key': key, 'commands': journal})
            raise

    @scoped
    def run_phases(self, spec_path, phases):
        return {phase: self.run(spec_path, phase) for phase in phases}

    @scoped
    def finalize(self, spec_path, output):
        need(self.lane == 'full', 'development results cannot finalize a task')
        from .finalize import finalize
        return finalize(self, spec_path, output)
