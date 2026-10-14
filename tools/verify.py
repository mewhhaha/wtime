#!/usr/bin/env python3
"""Shared verification CLI. Phase execution and publication require --execute."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from verification.artifacts import VerificationError, need
from verification.runner import Workflow


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--store', type=Path, help='shared local payload store; default target/verification')
    parser.add_argument('--profile-output', type=Path, help='exclusive JSON file for operation work counters')
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('perf')
    p.add_argument('--output', type=Path, required=True, help='exclusive tooling performance output directory')
    p.add_argument('--budget', type=Path)
    p.add_argument('--execute', action='store_true')
    for name in ('plan', 'run', 'finalize', 'dev', 'close'):
        p = sub.add_parser(name)
        p.add_argument('--contract', type=Path, required=True)
        if name == 'run':
            p.add_argument('--phase', choices=['freeze', 'verify', 'measure', 'analyze', 'all'], required=True)
        if name in ('finalize', 'close'):
            p.add_argument('--output', type=Path, required=True)
        if name == 'dev':
            p.add_argument('--group', action='append', help='run selected groups; default all affected groups')
        if name != 'plan':
            p.add_argument('--execute', action='store_true')
    p = sub.add_parser('p042-contract')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    p = sub.add_parser('migrate-p042')
    p.add_argument('--contract', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    p = sub.add_parser('finalize-migration')
    p.add_argument('--prepared', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    p = sub.add_parser('statistics')
    p.add_argument('--task', required=True)
    p.add_argument('--comparison', type=Path, required=True)
    p.add_argument('--build', type=Path, required=True)
    p.add_argument('--optional-build', type=Path)
    p.add_argument('--contract', type=Path)
    p.add_argument('--candidate-build-record', type=Path)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    p = sub.add_parser('audit-statistics')
    p.add_argument('--record', type=Path, required=True)
    p = sub.add_parser('publish')
    p.add_argument('--performance-root', type=Path, required=True)
    p.add_argument('--completion', type=Path, required=True)
    p.add_argument('--snapshot', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    for name in ('source-map', 'check-sources'):
        p = sub.add_parser(name)
        p.add_argument('--directory', required=True)
        if name == 'source-map':
            p.add_argument('--output', type=Path, required=True)
            p.add_argument('--execute', action='store_true')
        else:
            p.add_argument('--baseline', type=Path, required=True)
            p.add_argument('--pointer', default='/runtime_source_map')
    args = parser.parse_args(argv)
    if args.command not in ('plan', 'audit-statistics', 'check-sources') and not args.execute:
        print('inert: pass --execute to perform the declared operation')
        return 0
    try:
        repo = args.repo.resolve(strict=True)
        store = args.store or repo / 'target/verification'
        if args.command == 'perf':
            from verification.performance import run
            result = run(repo, args.output, args.budget)
        elif args.command in ('source-map', 'check-sources'):
            from verification.artifacts import read_json, write_new
            from verification.contracts import identity, json_pointer
            sources = identity(repo, {'paths': [args.directory.rstrip('/') + '/**/*.rs']})['files']
            if args.command == 'source-map':
                write_new(args.output, sources)
                result = {'source_map': str(args.output)}
            else:
                need(sources == json_pointer(read_json(args.baseline), args.pointer), 'current sources differ from declared predecessor')
                print('valid')
                return 0
        elif args.command == 'plan':
            flow = Workflow(repo, store)
            result = flow.plan(args.contract)
            result['metrics'] = flow.last_metrics
        elif args.command == 'run':
            from verification.contracts import PHASES, load_spec
            flow = Workflow(repo, store)
            phases = [p for p in PHASES if p in load_spec(args.contract)['stages']] if args.phase == 'all' else [args.phase]
            result = flow.run_phases(args.contract, phases)
            result = {p: {k: v[k] for k in ('status', 'reused', 'receipt', 'reason', 'changed_inputs')} for p, v in result.items()}
            result['metrics'] = flow.last_metrics
        elif args.command == 'dev':
            from verification.development import run
            result = run(repo, store, args.contract, args.group)
        elif args.command == 'close':
            from verification.contracts import PHASES, load_spec
            from verification.context import operation
            flow = Workflow(repo, store)
            with operation() as ctx:
                spec = load_spec(args.contract)
                need(not spec.get('development_lane') and not spec.get('development_only'),
                     'development results cannot close tasks')
                phases = [p for p in PHASES if p in spec['stages']]
                executed = flow.run_phases(args.contract, phases)
                result = flow.finalize(args.contract, args.output)
                result['phases'] = {p: {k: v[k] for k in ('status', 'reused', 'reason', 'changed_inputs')} for p, v in executed.items()}
            result['metrics'] = ctx.summary()
        elif args.command == 'finalize':
            flow = Workflow(repo, store)
            result = flow.finalize(args.contract, args.output)
            result['metrics'] = flow.last_metrics
        elif args.command == 'p042-contract':
            from verification.legacy_p042 import make_contract
            make_contract(repo, args.output)
            result = {'contract': str(args.output)}
        elif args.command == 'migrate-p042':
            from verification.legacy_p042 import prepare_migration
            result = prepare_migration(repo, args.contract, store)
        elif args.command == 'finalize-migration':
            from verification.legacy_p042 import finalize_migration
            result = finalize_migration(args.prepared, args.output)
        elif args.command == 'statistics':
            from verification.statistics import compute
            value = compute(repo, args.task, args.comparison, args.build, args.optional_build,
                            args.contract, args.candidate_build_record, args.output)
            result = {'status': value['status'], 'decision': value['decision']['decision'], 'record': str(args.output)}
        elif args.command == 'audit-statistics':
            from verification.statistics import audit
            result = audit(args.record, repo)
        else:
            from verification.publish import execute
            return execute(args.performance_root, args.completion, args.snapshot)
        if args.profile_output:
            from verification.artifacts import write_new
            write_new(args.profile_output, {'command': args.command, 'metrics': result.get('metrics', {}), 'status': 'passed'})
        print(json.dumps(result, sort_keys=True))
        return 0
    except (VerificationError, OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
        print('verification failed: ' + type(error).__name__ + ': ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
