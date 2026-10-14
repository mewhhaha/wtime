#!/usr/bin/env python3
"""Validate plan files, proposal coverage, links and the dependency DAG (stdlib only)."""
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent
plan = json.loads((root / 'TASKS.json').read_text())
sources = json.loads((root / 'SOURCES.json').read_text())
tasks = plan['tasks']
by_id = {t['id']: t for t in tasks}
errors = []
if len(by_id) != len(tasks):
    errors.append('Duplicate task IDs')
if len({t['key'] for t in tasks}) != len(tasks):
    errors.append('Duplicate task keys')
if len({t['path'] for t in tasks}) != len(tasks):
    errors.append('Duplicate task paths')
known_status = {'todo', 'in_progress', 'blocked', 'done'}
for t in tasks:
    path = root / t['path']
    if not path.is_file():
        errors.append(f"Missing task file: {t['path']}")
        continue
    text = path.read_text()
    if not text.startswith(f"# {t['id']} — {t['title']}"):
        errors.append(f"Heading differs from manifest: {t['id']}")
    if t['status'] not in known_status or f"Status: **{t['status']}**" not in text:
        errors.append(f"Status differs from manifest: {t['id']}")
    for field in ('owned_paths', 'implementation_steps', 'fixture_cases', 'test_commands'):
        if not t.get(field):
            errors.append(f"Missing implementation detail: {t['id']} {field}")
    if t.get('implementation_model') != {'model': 'gpt-6.1-sol', 'reasoning_effort': 'low'}:
        errors.append(f"Wrong implementation model: {t['id']}")
    if len(t.get('implementation_steps', [])) < 3 or len(t.get('fixture_cases', [])) < 3:
        errors.append(f"Too few concrete steps/cases: {t['id']}")
    for step in t.get('implementation_steps', []):
        if step not in text:
            errors.append(f"Recipe not reflected in task file: {t['id']}")
    if len(t['dependencies']) != len(set(t['dependencies'])):
        errors.append(f"Duplicate dependency: {t['id']}")
    for dep in t['dependencies']:
        if dep not in by_id:
            errors.append(f"Unknown dependency: {t['id']} -> {dep}")
        elif dep == t['id']:
            errors.append(f"Self dependency: {t['id']}")
        elif f"[{dep} — " not in text:
            errors.append(f"Task file missing prerequisite link: {t['id']} -> {dep}")
    if t['dependency_keys'] != [by_id[d]['key'] for d in t['dependencies'] if d in by_id]:
        errors.append(f"Dependency keys differ: {t['id']}")

marks = {}
def visit(task_id, chain):
    if marks.get(task_id) == 1:
        errors.append('Dependency cycle: ' + ' -> '.join(chain + [task_id]))
        return
    if marks.get(task_id) == 2 or task_id not in by_id:
        return
    marks[task_id] = 1
    for dep in by_id[task_id]['dependencies']:
        visit(dep, chain + [task_id])
    marks[task_id] = 2
for task_id in by_id:
    visit(task_id, [])

proposal_ids = {s['id'] for s in sources['proposals']}
for t in tasks:
    for proposal in t['proposals']:
        if proposal not in proposal_ids:
            errors.append(f"Unknown proposal tag: {t['id']} -> {proposal}")
for s in sources['proposals']:
    covered = [t['id'] for t in tasks if s['id'] in t['proposals']]
    if covered != s['tasks']:
        errors.append(f"Source/task coverage mismatch: {s['id']}")
    if s['scope'] == 'included' and not covered:
        errors.append(f"Included proposal lacks tasks: {s['id']}")
    if s['scope'] == 'excluded' and covered:
        errors.append(f"Excluded proposal has implementation tasks: {s['id']}")
    if not re.fullmatch('[0-9a-f]{40}', s['source_commit'] or ''):
        errors.append(f"Missing exact source commit: {s['id']}")

task_files = {p.relative_to(root).as_posix() for p in root.glob('*/T[0-9]*.md')}
if task_files != {t['path'] for t in tasks}:
    errors.append('Task file set differs from manifest')

for path in root.rglob('*.md'):
    for target in re.findall(r'\]\(([^)]+)\)', path.read_text()):
        if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:', target):
            continue
        relative = target.split('#', 1)[0]
        if relative and not (path.parent / relative).exists():
            errors.append(f"Broken link in {path.relative_to(root)}: {target}")

if errors:
    print('\n'.join(errors), file=sys.stderr)
    sys.exit(1)
included = sum(s['scope'] == 'included' for s in sources['proposals'])
ready = [t['id'] for t in tasks if t['status'] == 'todo' and all(by_id[d]['status'] == 'done' for d in t['dependencies'])]
print(f"PASS: {len(tasks)} tasks; acyclic dependencies; links and statuses match")
print(f"PASS: {included} included proposals covered; {len(sources['proposals']) - included} excluded")
print('Ready tasks: ' + ', '.join(ready))
