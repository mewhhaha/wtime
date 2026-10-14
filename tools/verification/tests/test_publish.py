#!/usr/bin/env python3
"""Fresh-TMP tests for proof-only TASKS evidence in publisher v2."""
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HELPER = Path(__file__).resolve().parents[1] / 'publish.py'
spec = importlib.util.spec_from_file_location('task_status_v2_tested', HELPER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def setup_case(base, bad_proof=False, relative_proof=False):
    workspace = base / 'workspace'
    performance = workspace / 'tasks' / 'performance'
    repo = workspace / 'wtime'
    performance.mkdir(parents=True)
    (repo / 'target' / 'performance' / 'TASK-status').mkdir(parents=True)
    (performance / 'check_plan.py').write_text("STATUSES={'todo','in_progress','blocked','deferred','done'}\nOUTCOMES={'pending','adopted','rejected','inconclusive'}\n")
    (performance / 'INDEX.md').write_text('All 1 tasks are planned.\n')
    task_md = performance / 'P900.md'
    task_md.write_text('Status: **todo**\nOutcome: **pending**\n\nNo commands in this planning file have been executed as implementation evidence.\n')
    task_index = {'tasks': [{'id': 'P900', 'path': 'P900.md', 'status': 'todo', 'outcome': 'pending', 'dependencies': []}]}
    (performance / 'TASKS.json').write_text(json.dumps(task_index, indent=2) + '\n')
    proof = workspace / 'artifacts' / 'proof.json' if relative_proof else base / 'proof.json'
    proof.parent.mkdir(parents=True, exist_ok=True); proof.write_text('{"task":"P900","status":"passed","outcome":"adopted"}\n')
    support = base / 'support.log'; support.write_text('full inventory support artifact\n')
    proof_sha = digest(proof)
    proof_record_path = 'artifacts/proof.json' if relative_proof else str(proof)
    raw_proof = {'path': proof_record_path, 'sha256': ('0' * 64 if bad_proof else proof_sha), 'bytes': proof.stat().st_size}
    proof_inventory_path = proof_record_path
    record = {'task': 'P900', 'status': 'done', 'outcome': 'adopted', 'evidence': [raw_proof],
              'artifact_hashes': {proof_inventory_path: proof_sha, str(support): digest(support)}}
    record_path = base / 'completion-record.json'
    record_path.write_text(json.dumps(record, indent=2) + '\n')
    snapshot = repo / 'target' / 'performance' / 'TASK-status' / 'attempt-0001'
    return performance, record_path, snapshot, proof, support, task_index


def test_success_keeps_full_inventory_but_publishes_only_proof():
    original_sha = digest(HELPER)
    with tempfile.TemporaryDirectory(prefix='task-status-v2-ok-', dir='/tmp') as td:
        performance, record_path, snapshot, proof, support, _ = setup_case(Path(td))
        assert module.execute(performance, record_path, snapshot) == 0
        published = json.loads((performance / 'TASKS.json').read_text())['tasks'][0]
        expected = {'path': str(proof.resolve()), 'sha256': digest(proof), 'bytes': proof.stat().st_size}
        assert published['evidence'] == [expected]
        assert published['completion_record']['artifact_hashes'] == {str(proof): digest(proof), str(support): digest(support)}
        attempt = json.loads((snapshot / 'publication-attempt.json').read_text())
        assert attempt['completed'] is True and len(attempt['artifact_hashes']) == 2
        snap = json.loads((snapshot / 'prewrite-snapshot.json').read_text())
        assert {'helper','TASKS.json','INDEX.md','completion_record','task_markdown'} <= set(snap['files'])
        assert snap['files']['helper']['sha256'] == digest(HELPER)
        assert digest(HELPER) == original_sha
        print('proof-only task evidence; full inventory retained: PASS')


def test_workspace_relative_proof_path_resolves_once():
    with tempfile.TemporaryDirectory(prefix='task-status-v2-relative-', dir='/tmp') as td:
        performance, record_path, snapshot, proof, _, _ = setup_case(Path(td), relative_proof=True)
        assert module.execute(performance, record_path, snapshot) == 0
        published = json.loads((performance / 'TASKS.json').read_text())['tasks'][0]
        assert published['evidence'][0]['path'] == str(proof.resolve())
        print('workspace-relative proof normal ref resolves to canonical path: PASS')


def test_bad_proof_ref_fails_closed_without_task_mutation():
    with tempfile.TemporaryDirectory(prefix='task-status-v2-bad-', dir='/tmp') as td:
        performance, record_path, snapshot, _, _, initial = setup_case(Path(td), bad_proof=True)
        before = (performance / 'TASKS.json').read_bytes()
        assert module.execute(performance, record_path, snapshot) == 1
        assert (performance / 'TASKS.json').read_bytes() == before
        attempt = json.loads((snapshot / 'publication-attempt.json').read_text())
        assert attempt['completed'] is False and 'authoritative proof ref' in attempt['error']
        print('mismatched authoritative proof SHA fails closed: PASS')


def test_done_refuses_pending_dependency_without_plan_mutation():
    with tempfile.TemporaryDirectory(prefix='publisher-dependency-', dir='/tmp') as td:
        performance, record_path, snapshot, _, _, _ = setup_case(Path(td))
        task_path = performance / 'TASKS.json'
        manifest = json.loads(task_path.read_text())
        manifest['tasks'][0]['dependencies'] = ['P899']
        manifest['tasks'].append({'id':'P899','status':'in_progress','outcome':'pending','dependencies':[]})
        task_path.write_text(json.dumps(manifest, indent=2) + '\n')
        before = task_path.read_bytes()
        assert module.execute(performance, record_path, snapshot) == 1
        assert task_path.read_bytes() == before
        attempt = json.loads((snapshot / 'publication-attempt.json').read_text())
        assert attempt['completed'] is False and 'dependency P899 is in_progress' in attempt['error']
        print('pending dependency prevents completion without mutation: PASS')


def test_multiple_authoritative_proofs_fail_closed():
    with tempfile.TemporaryDirectory(prefix='publisher-multiple-proof-', dir='/tmp') as td:
        performance, record_path, snapshot, proof, support, _ = setup_case(Path(td))
        record = json.loads(record_path.read_text())
        record['evidence'].append({'path':str(support.resolve()),'sha256':digest(support),'bytes':support.stat().st_size})
        record_path.write_text(json.dumps(record, indent=2) + '\n')
        before = (performance / 'TASKS.json').read_bytes()
        assert module.execute(performance, record_path, snapshot) == 1
        assert (performance / 'TASKS.json').read_bytes() == before
        attempt = json.loads((snapshot / 'publication-attempt.json').read_text())
        assert attempt['completed'] is False and 'exactly one canonical' in attempt['error']
        print('multiple authoritative refs are rejected: PASS')


class PublisherTests(unittest.TestCase):
    pass

for _name, _function in list(globals().items()):
    if _name.startswith('test_') and callable(_function):
        setattr(PublisherTests, _name, lambda self, fn=_function: fn())
del _name, _function

if __name__ == '__main__':
    unittest.main()
