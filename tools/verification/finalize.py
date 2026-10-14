"""Finalization reads verified receipts and writes an index; it executes no commands."""
from pathlib import Path
from .artifacts import digest, need, normal, sha, write_new
from .contracts import load_spec


def completion(proof_path, task, outcome):
    ref = normal(proof_path)
    return {'task': task, 'status': 'done', 'outcome': outcome,
            'evidence': [ref], 'artifact_hashes': {ref['path']: ref['sha256']}}


def finalize(workflow, spec_path, output):
    spec = load_spec(spec_path)
    need(not spec.get('development_lane') and not spec.get('development_only'),
         'development results cannot finalize a task')
    config = spec.get('completion', {})
    required = config.get('required_phases', ['freeze', 'verify', 'measure'])
    need(required and set(required) <= set(spec['stages']), 'incomplete finalization contract')
    outcome = config.get('outcome')
    if 'decision' in config:
        from .contracts import json_pointer
        rule = config['decision']
        need(rule.get('phase') in required, 'decision phase is not required')
        _, receipt = workflow._receipt(spec['task'], rule['phase'])
        need(rule.get('artifact') in receipt['artifacts'], 'decision artifact is absent')
        outcome = json_pointer(workflow.store.json(receipt['artifacts'][rule['artifact']]), rule.get('pointer', '/decision'))
    else:
        need(outcome == 'contract_only', 'measured outcomes must come from a verified decision artifact')
    need(outcome in ('adopted', 'rejected', 'inconclusive', 'contract_only'), 'missing final outcome')
    phases = {}
    for phase in required:
        ref, receipt = workflow._receipt(spec['task'], phase)
        need(receipt.get('completion_eligible', True) and receipt.get('lane', 'full') == 'full',
             'development results cannot satisfy required full phases')
        need(receipt['input_key'] == digest(workflow._inputs(spec, phase)), 'phase receipt is stale: ' + phase)
        workflow._revalidate(spec, phase, receipt)
        phases[phase] = ref
    output = Path(output).resolve()
    from .context import current
    if current():
        current().guard()
    need(not output.exists(), 'finalization output already exists')
    output.mkdir(parents=True, exist_ok=False)
    proof = {'schema': 'wtime-verification-evidence-v1', 'task': spec['task'], 'status': 'passed',
             'passed': True, 'outcome': outcome, 'decision': outcome,
             'candidate_promoted': outcome == 'adopted', 'phases': phases,
             'store': str(workflow.root), 'contract': normal(spec_path),
             'synthetic': spec.get('synthetic', False),
             'finalizer_sha256': sha(Path(__file__)), 'limitations': config.get('limitations', [])}
    path = output / 'evidence.json'
    write_new(path, proof)
    proposal = output / 'completion.json'
    write_new(proposal, completion(path, spec['task'], outcome))
    return {'evidence': str(path), 'completion_record': str(proposal), 'outcome': outcome}
