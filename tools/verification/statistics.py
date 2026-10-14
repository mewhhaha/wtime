"""Produce one bound statistics/decision record; recomputation is an explicit audit."""
from __future__ import annotations
import importlib.util
from pathlib import Path
import re
import sys
from .artifacts import need, normal, read_json, sha, write_new


ANALYSIS_ONLY = {'tools/wtime-bench/analyze.py', 'tools/wtime-bench/gates.py'}


def bind_sources(gate, report):
    """Adapt the legacy all-Python map without relaxing measured input checks."""
    measured = report.get('metadata', {}).get('candidate_sources')
    selection = report.get('selection')
    original = gate._current_source_identity
    current = original(selection)
    import execution_identity
    try:
        execution_binding = execution_identity.verify(measured, current)
    except ValueError as error:
        raise ValueError('measured runtime/harness/fixture identity or membership changed: '+str(error)) from error
    changed = {name for name in set(current) & set(measured) if measured[name] != current[name]}
    binding = {'measured': measured, 'analysis_inputs': current,
               'analysis_only_changes': {name: {'measured_sha256': measured[name],
                                               'analysis_sha256': current[name]} for name in sorted(changed)},
               'execution_binding': execution_binding}
    def recorded(selection_arg=None):
        need(selection_arg == selection, 'gate selection differs from source binding')
        return measured
    # The legacy checker also validates observations, policies, workloads and
    # builds. Adapt only its source-map lookup after the strict check above.
    gate._current_source_identity = recorded
    def stable():
        need(original(selection) == current, 'analysis inputs changed during evaluation')
    return binding, stable


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, 'cannot load verified producer: ' + str(path))
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


def compute(repo, task, comparison, build, optional_build, contract, candidate_build, output):
    repo = Path(repo).resolve(strict=True)
    bench = repo / 'tools/wtime-bench'
    sys.path.insert(0, str(bench))
    if task == 'P046':
        return compute_portfolio(repo, comparison, build, optional_build, contract, candidate_build, output)
    report = Path(comparison)
    report = report if report.is_file() else report / 'report.json'
    if read_json(report).get('schema') == 'wtime-focused-comparison-v1':
        return compute_candidate(repo, task, report, build, optional_build, contract, candidate_build, output)
    inputs = {'report': normal(report), 'build': normal(build),
              'analyzer': normal(bench / 'analyze.py'), 'gate': normal(bench / 'gates.py'),
              'producer': normal(Path(__file__))}
    for name, path in [('optional_build', optional_build), ('contract', contract), ('candidate_build', candidate_build)]:
        if path:
            inputs[name] = normal(path)
    gate = module(bench / 'gates.py', 'shared_verification_gate')
    need(Path(gate.ROOT).resolve() == repo, 'gate workspace differs from selected repository')
    binding, stable = bind_sources(gate, read_json(report))
    # evaluate already validates the raw observations and calculates the group
    # and primary-family statistics. Do not invoke analyze a second time.
    result = gate.evaluate(task, Path(comparison), Path(build),
                           Path(optional_build) if optional_build else None,
                           Path(contract) if contract else None,
                           Path(candidate_build) if candidate_build else None)
    analysis = result.pop('analysis')
    stable()
    from .artifacts import verify_normal
    for ref in inputs.values():
        verify_normal(ref)
    record = {'schema': 'wtime-verification-statistics-v1', 'task': task, 'status': 'passed',
              'inputs': inputs, 'analysis': analysis, 'decision': result,
              'source_binding': binding,
              'statistics_computations': 1, 'timing_evidence': True}
    write_new(output, record)
    return record


def compute_candidate(repo, task, report, build, optional_build, contract, candidate_build, output):
    """One focused analysis; never substitutes for a milestone portfolio."""
    bench = repo / 'tools/wtime-bench'
    gate = module(bench/'candidate_gate.py', 'shared_candidate_gate')
    inputs = {'report':normal(report), 'build':normal(build), 'gate':normal(bench/'candidate_gate.py'),
              'analyzer':normal(bench/'analyze.py'), 'bootstrap':normal(bench/'bootstrap.py'),
              'producer':normal(Path(__file__)), 'task_plan':normal(repo.parent/'tasks/performance/TASKS.json')}
    for name in ('gates.py','execution_identity.py','focused_compare.py','selection_manifest.py','cold_products.py'):
        inputs['helper_'+name.removesuffix('.py')] = normal(bench/name)
    for name in ('build_budget.py','dependency_budget.py'):
        inputs['helper_'+name.removesuffix('.py')] = normal(repo/'tools/checks'/name)
    for name, path in [('optional_build',optional_build),('contract',contract),('candidate_build',candidate_build)]:
        if path:
            inputs[name] = normal(path)
    result = gate.evaluate(task, report, build, optional_build, contract, candidate_build)
    analysis = result.pop('analysis')
    from .artifacts import verify_normal
    for reference in inputs.values():
        verify_normal(reference)
    record = {'schema':'wtime-verification-statistics-v1','task':task,'status':'passed',
              'inputs':inputs,'analysis':analysis,'decision':result,'statistics_computations':1,
              'timing_evidence':True,'source_binding':{'scope':'frozen candidate declaration'}}
    write_new(output, record)
    return record


def compute_portfolio(repo, portfolio, build, optional_build, contract, candidate_build, output):
    """One full-matrix producer call; each distinct report is analyzed once."""
    repo = Path(repo).resolve(strict=True)
    bench = repo / 'tools/wtime-bench'
    sys.path.insert(0, str(bench))
    gate = module(bench / 'p046_gate.py', 'shared_verification_portfolio_gate')
    inputs = {'portfolio': normal(portfolio), 'build': normal(build),
              'analyzer': normal(bench / 'analyze.py'), 'gate': normal(bench / 'p046_gate.py'),
              'producer': normal(Path(__file__))}
    declaration = read_json(portfolio)
    def bound(reference):
        actual = normal(reference['path'])
        need(actual['sha256'] == reference['sha256'], 'portfolio input identity differs')
        return actual
    for baseline_name, families in declaration.get('reports', {}).items():
        for family, reference in families.items():
            inputs['report_'+baseline_name+'_'+family] = bound(reference)
    for name,reference in declaration.get('cold_controls',{}).items():
        inputs['cold_'+name]=bound(reference)
    for name in ('p121', 'p123', 'retained'):
        reference = declaration[name]
        inputs[name] = bound(reference)
    for name, path in [('optional_build', optional_build), ('contract', contract), ('candidate_build', candidate_build)]:
        if path:
            inputs[name] = normal(path)
    result = gate.evaluate_portfolio(Path(portfolio), Path(build),
        Path(optional_build) if optional_build else None,
        Path(contract) if contract else None, Path(candidate_build) if candidate_build else None)
    analysis = result.pop('analysis')
    source_binding = result.pop('source_binding')
    from .artifacts import verify_normal
    for reference in inputs.values():
        verify_normal(reference)
    record = {'schema': 'wtime-verification-statistics-v1', 'task': 'P046', 'status': 'passed',
              'inputs': inputs, 'analysis': analysis, 'decision': result,
              'source_binding': source_binding, 'statistics_computations': 1,
              'dataset_statistics_computations': result['dataset_statistics_computations'],
              'portfolio_decision_computations': 1, 'timing_evidence': True}
    write_new(output, record)
    return record


def audit(path, repo):
    repo = Path(repo).resolve(strict=True)
    record = read_json(path)
    need(record.get('schema') == 'wtime-verification-statistics-v1', 'not a shared statistics record')
    from .artifacts import verify_normal
    for ref in record['inputs'].values():
        verify_normal(ref)
    bench = repo / 'tools/wtime-bench'
    if record.get('source_binding',{}).get('scope') == 'frozen candidate declaration':
        need(normal(bench/'candidate_gate.py') == record['inputs']['gate'], 'audit candidate gate differs')
        sys.path.insert(0,str(bench))
        gate = module(bench/'candidate_gate.py','shared_candidate_audit')
        inputs = record['inputs']
        def optional(name):
            return Path(inputs[name]['path']) if name in inputs else None
        result = gate.evaluate(record['task'],optional('report'),optional('build'),optional('optional_build'),
                               optional('contract'),optional('candidate_build'))
        analysis = result.pop('analysis')
        need(analysis == record['analysis'] and result == record['decision'], 'recomputed candidate decision differs')
        return {'status':'passed','recomputed':True,'decision_recomputed':True,'record_sha256':sha(path)}
    if record['task'] == 'P046':
        need(normal(bench/'p046_gate.py') == record['inputs']['gate']
             and normal(bench/'analyze.py') == record['inputs']['analyzer'],
             'audit portfolio producer paths differ')
        sys.path.insert(0, str(bench))
        gate = module(bench/'p046_gate.py', 'shared_verification_portfolio_audit')
        inputs = record['inputs']
        def optional(name):
            return Path(inputs[name]['path']) if name in inputs else None
        recomputed = gate.evaluate_portfolio(Path(inputs['portfolio']['path']), Path(inputs['build']['path']),
            optional('optional_build'), optional('contract'), optional('candidate_build'))
        analysis = recomputed.pop('analysis')
        binding = recomputed.pop('source_binding')
        need(analysis == record['analysis'] and binding == record['source_binding'],
             'recomputed portfolio analysis/source binding differs')
        need(recomputed == record['decision'], 'recomputed portfolio decision differs')
        return {'status':'passed','recomputed':True,'decision_recomputed':True,'record_sha256':sha(path)}
    need(normal(bench / 'gates.py') == record['inputs']['gate']
         and normal(bench / 'analyze.py') == record['inputs']['analyzer'],
         'audit producer paths differ from the bound repository')
    sys.path.insert(0, str(bench))
    gate = module(bench / 'gates.py', 'shared_verification_audit_gate')
    inputs = record['inputs']
    need(Path(gate.ROOT).resolve() == repo, 'audit gate workspace differs from selected repository')
    binding, stable = bind_sources(gate, read_json(inputs['report']['path']))
    need(binding == record['source_binding'], 'audit source binding differs from retained statistics')
    def optional(name):
        return Path(inputs[name]['path']) if name in inputs else None
    recomputed = gate.evaluate(record['task'], Path(inputs['report']['path']), Path(inputs['build']['path']),
                               optional('optional_build'), optional('contract'), optional('candidate_build'))
    analysis = recomputed.pop('analysis')
    stable()
    need(analysis == record['analysis'], 'recomputed analysis differs from retained statistics')
    need(recomputed == record['decision'], 'recomputed gate decision differs from retained statistics')
    return {'status': 'passed', 'recomputed': True, 'decision_recomputed': True, 'record_sha256': sha(path)}
