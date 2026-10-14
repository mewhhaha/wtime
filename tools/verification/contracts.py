"""Preflight task declarations and separate component identities."""
from __future__ import annotations
import fnmatch
from pathlib import Path
import re
import sys
from .artifacts import digest, need, read_json, sha
from .context import current

PHASES = ('freeze', 'verify', 'measure', 'analyze')


def matching_files(repo, pattern):
    if pattern.endswith('/**') or (repo / pattern).is_dir():
        pattern = pattern.rstrip('/') + '/*' if pattern.endswith('/**') else pattern.rstrip('/') + '/**/*'
    return sorted(p for p in repo.glob(pattern) if p.is_file())


def load_spec(path):
    spec = read_json(path)
    need(isinstance(spec, dict) and spec.get('schema_version') == 1, 'unsupported verification contract')
    for flag in ('synthetic', 'development_lane', 'development_only'):
        need(type(spec.get(flag, False)) is bool, 'contract flag must be boolean: ' + flag)
    need(re.fullmatch(r'P\d{3}', str(spec.get('task', ''))), 'invalid task ID')
    need(isinstance(spec.get('identities'), dict) and spec['identities'], 'missing component identities')
    stages = spec.get('stages')
    need(isinstance(stages, dict) and stages and set(stages) <= set(PHASES), 'invalid phase declarations')
    for phase, stage in stages.items():
        need(isinstance(stage, dict) and isinstance(stage.get('uses'), list) and stage['uses'], 'missing phase identities')
        need(set(stage['uses']) <= set(spec['identities']), 'phase consumes an unknown identity')
        deps = stage.get('dependencies', [])
        need(isinstance(deps, list) and len(deps) == len(set(deps))
             and all(dep in stages and PHASES.index(dep) < PHASES.index(phase) for dep in deps), 'invalid phase dependency')
        commands = stage.get('commands', [])
        need(isinstance(commands, list) and commands, 'phase must execute a nonempty command list')
        names = []
        for command in commands:
            need(isinstance(command, dict) and isinstance(command.get('name'), str), 'invalid command declaration')
            need(re.fullmatch(r'[A-Za-z0-9_-]+', command['name']), 'invalid command name')
            argv = command.get('argv')
            need(isinstance(argv, list) and argv and all(isinstance(v, str) and v for v in argv), 'argv must be a nonempty string list')
            names.append(command['name'])
        need(len(names) == len(set(names)), 'duplicate command name')
        need(isinstance(stage.get('outputs'), dict) and (stage['outputs'] or spec.get('development_lane')),
             'phase must declare outputs')
        for group in stage.get('tests', []):
            need(group.get('kind', 'cargo') in ('cargo', 'unittest'), 'unknown named test runner')
            need(group.get('command') in names and isinstance(group.get('required_names'), list)
                 and group['required_names'] and len(group['required_names']) == len(set(group['required_names'])), 'invalid required test group')
            need(type(group.get('minimum_passed', 1)) is int and group.get('minimum_passed', 1) > 0, 'zero-test contract')
    return spec


def identity(repo, declaration):
    need(isinstance(declaration, dict) and isinstance(declaration.get('paths', []), list), 'invalid identity declaration')
    repo = Path(repo).resolve()
    files = {}
    bindings = {}
    optional = set(declaration.get('optional_paths', []))
    for pattern in declaration.get('paths', []):
        need(isinstance(pattern, str) and not Path(pattern).is_absolute() and '..' not in Path(pattern).parts,
             'identity path escapes repository')
        matches = matching_files(repo, pattern)
        need(matches or pattern in optional, 'identity input is missing: ' + pattern)
        for path in matches:
            resolved = path.resolve(strict=True)
            need(resolved.is_relative_to(repo), 'identity symlink escapes repository')
            files[path.relative_to(repo).as_posix()] = sha(resolved)
            bindings[path.relative_to(repo).as_posix()] = str(resolved)
    settings = declaration.get('settings', {})
    need(files or settings, 'empty component identity')
    payload = {'files': files, 'settings': settings, 'declaration': declaration}
    if current():
        key = str(repo) + ':' + digest(declaration)
        def membership():
            return sorted({(p.relative_to(repo).as_posix(), str(p.resolve(strict=True)))
                           for pattern in declaration.get('paths', []) for p in matching_files(repo, pattern)})
        current().probes.setdefault(key, (membership, sorted(bindings.items())))
    return {**payload, 'sha256': digest(payload)}


def expand(value, repo, stage, context=None):
    need(isinstance(value, str), 'path/argument template must be a string')
    try:
        return value.format(repo=str(repo), stage=str(stage), python=sys.executable, **(context or {}))
    except (KeyError, ValueError, IndexError) as error:
        raise ValueError('unknown or malformed argument template: ' + value) from error


def output_paths(stage, repo, directory, context=None):
    outputs = {}
    for name, template in stage['outputs'].items():
        path = Path(expand(template, repo, directory, context)).resolve()
        need(path.is_relative_to(directory.resolve()) and path != directory.resolve(), 'output escaped exclusive attempt')
        need(path not in outputs.values(), 'two output names collide')
        outputs[name] = path
    return outputs


def validate_delta(current, baseline, allowed_changes, allowed_additions):
    need(isinstance(current, dict) and isinstance(baseline, dict), 'invalid source maps')
    removed = set(baseline) - set(current)
    changed = {p for p in set(current) & set(baseline) if current[p] != baseline[p]}
    added = set(current) - set(baseline)
    need(not removed, 'source files were removed')
    need(all(any(fnmatch.fnmatchcase(p, rule) for rule in allowed_changes) for p in changed), 'source change escapes ownership')
    need(all(any(fnmatch.fnmatchcase(p, rule) for rule in allowed_additions) for p in added), 'source addition escapes ownership')
    return {'changed': sorted(changed), 'added': sorted(added), 'unchanged': sorted(set(current) & set(baseline) - changed)}


def json_pointer(value, pointer):
    need(isinstance(pointer, str) and (pointer == '' or pointer.startswith('/')), 'invalid JSON pointer')
    for item in pointer.split('/')[1:]:
        item = item.replace('~1', '/').replace('~0', '~')
        if isinstance(value, list):
            need(item.isdigit() and int(item) < len(value), 'missing JSON array member: ' + pointer)
            value = value[int(item)]
        else:
            need(isinstance(value, dict) and item in value, 'missing JSON member: ' + pointer)
            value = value[item]
    return value


def check_json(doc, rule):
    value = json_pointer(doc, rule.get('pointer', ''))
    expected = rule.get('value')
    op = rule.get('op')
    if op == 'equal':
        ok = type(value) == type(expected) and value == expected
    elif op in ('positive', 'at_least'):
        ok = type(value) in (int, float) and value >= (expected if op == 'at_least' else 0) and (op != 'positive' or value > 0)
    elif op == 'exact_set':
        ok = isinstance(value, list) and len(value) == len(set(value)) and set(value) == set(expected)
    elif op == 'length':
        ok = isinstance(value, (list, dict)) and len(value) == expected
    elif op == 'ref':
        from .artifacts import verify_normal
        verify_normal(value)
        ok = True
    else:
        raise ValueError('unknown JSON check: ' + str(op))
    need(ok, 'output check failed: ' + rule.get('artifact', '') + rule.get('pointer', '') + ' ' + str(op))
