"""Incremental test groups with receipts that can never close a task."""
from pathlib import Path
import re
from .artifacts import digest, need, normal, read_json, sha, write_new
from .context import operation
from .runner import Workflow


def run(repo, root, contract, selected=None):
    with operation() as ctx:
        raw = read_json(contract)
        need(raw.get('schema_version') == 1 and re.fullmatch(r'P\d{3}', str(raw.get('task', ''))),
             'invalid development task contract')
        identities = raw.get('identities')
        groups = raw.get('development', {}).get('groups')
        need(isinstance(identities, dict) and identities and isinstance(groups, list) and groups,
             'contract must declare development identities and groups')
        names = [group.get('name') for group in groups]
        need(all(isinstance(name, str) and re.fullmatch(r'[A-Za-z0-9_-]+', name) for name in names)
             and len(names) == len(set(names)), 'invalid or duplicate development group')
        need(not selected or set(selected) <= set(names), 'unknown development group')
        need('_development_policy' not in identities, 'reserved development identity')
        results = {}
        for group in groups:
            if selected and group['name'] not in selected:
                continue
            stage = {key: value for key, value in group.items() if key != 'name'}
            need(not stage.get('dependencies'), 'development groups cannot consume full phase receipts')
            stage.setdefault('outputs', {})
            stage.setdefault('tests', [])
            need(isinstance(stage.get('uses'), list) and stage['uses'], 'development group has no input identities')
            need(stage.get('commands'), 'development group has no commands')
            tests_by_command = {test.get('command'): test for test in stage['tests']}
            for command in stage['commands']:
                args = command.get('argv', [])
                need(isinstance(args, list) and args, 'invalid development command')
                if not raw.get('synthetic', False):
                    cargo = Path(args[0]).name == 'cargo'
                    python = args[0] == '{python}'
                    if cargo:
                        need(len(args) > 1 and args[1] in ('test', 'check') and '--locked' in args
                             and not any(arg in ('--release', '-r', '--profile') or arg.startswith('--profile=') for arg in args),
                             'development Cargo commands must be incremental locked test/check')
                        if args[1] == 'test':
                            need(command['name'] in tests_by_command, 'development Cargo tests need named cases')
                    elif python:
                        need(args[1:4] == ['-B', '-m', 'unittest'] and '-v' in args
                             and tests_by_command.get(command['name'], {}).get('kind') == 'unittest',
                             'development Python commands must be verbose named unittest runs')
                    else:
                        need(False, 'development commands cannot build budgets, benchmarks, or statistics')
            stage['uses'] = [*stage['uses'], '_development_policy']
            spec = {'schema_version': 1, 'task': raw['task'], 'development_lane': True,
                    'synthetic': raw.get('synthetic', False), 'identities': {**identities,
                    '_development_policy': {'paths': [], 'settings': {'sha256': sha(Path(__file__))}}},
                    'stages': {'verify': stage}, 'owner_contract': normal(contract)}
            child_root = Path(root) / 'development' / raw['task'] / group['name']
            derived = child_root / 'contracts' / (digest(spec) + '.json')
            if derived.exists():
                need(read_json(derived) == spec, 'derived development contract changed')
            else:
                write_new(derived, spec)
            flow = Workflow(repo, child_root, lane='development')
            receipt = flow.run(derived, 'verify')
            results[group['name']] = {'status': receipt['status'], 'reused': receipt['reused'],
                'reason': receipt['reason'], 'changed_inputs': receipt['changed_inputs'],
                'receipt': receipt['receipt'], 'store': str(child_root),
                'tests': receipt['tests'], 'completion_eligible': False}
        ctx.guard()
    return {'lane': 'development', 'completion_eligible': False, 'groups': results, 'metrics': ctx.summary()}
