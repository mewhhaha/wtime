#!/usr/bin/env python3
"""Join task, source, coverage and run evidence for a release readiness decision.

This checker intentionally fails closed. Evidence reports must be created by the
named gate commands, include the current source/test snapshot digest, and be
bound to the manifest by their SHA-256. It never edits task status or evidence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]


def default_tasks_root(root: Path = ROOT) -> Path | None:
    env_root = os.environ.get('WTIME_TASKS_ROOT')
    if env_root:
        return Path(env_root)
    sibling = root.parent / 'tasks'
    if (sibling / 'TASKS.json').is_file():
        return sibling
    return None


TASKS_ROOT = default_tasks_root()
MANIFEST = ROOT / 'docs/architecture/release-manifest.json'
BASE_REQUIRED_EVIDENCE = {
    'core3', 'phase3', 'feature_profiles', 'interactions', 'fuzz',
    'differential', 'hardening', 'build_final', 'api_docs',
    'allocation_failure',
}
REQUIRED_EVIDENCE = BASE_REQUIRED_EVIDENCE | {'native_x86_64', 'native_aarch64'}


def required_evidence_keys(manifest: dict) -> set[str]:
    targets = manifest.get('toolchain', {}).get('supported_targets')
    if not isinstance(targets, list) or not targets:
        return set(REQUIRED_EVIDENCE)
    native: set[str] = set()
    for target in targets:
        if not isinstance(target, str) or not target:
            return set(REQUIRED_EVIDENCE)
        arch = target.split('-', 1)[0]
        if arch in ('x86_64', 'aarch64'):
            native.add(f'native_{arch}')
        else:
            native.add(f'native_{arch}')
    return BASE_REQUIRED_EVIDENCE | (native or {'native_x86_64', 'native_aarch64'})


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def core_source_hash(root: Path = ROOT) -> str:
    """Match the native hardening/build measurement source digest exactly."""
    paths = sorted([root / 'Cargo.lock', root / 'Cargo.toml',
                    *root.glob('crates/**/*.rs'), *root.glob('crates/**/Cargo.toml')])
    digest = hashlib.sha256()
    for path in paths:
        digest.update(str(path.relative_to(root)).encode() + b'\0' + path.read_bytes() + b'\0')
    return digest.hexdigest()


def task_closure(plan: dict, task_id: str = 'T169') -> set[str]:
    by_id = {task['id']: task for task in plan.get('tasks', [])}
    closure: set[str] = set()
    pending = list(by_id.get(task_id, {}).get('dependencies', []))
    while pending:
        task_id = pending.pop()
        if task_id in closure or task_id not in by_id:
            continue
        closure.add(task_id)
        pending.extend(by_id[task_id].get('dependencies', []))
    return closure


def unsupported_items(lock: dict) -> set[str]:
    items = {row.get('feature_id', '') for row in lock.get('excluded_sources', [])}
    for source in lock.get('sources', []):
        feature = source.get('feature_id', '<unknown>')
        items.update(f'{feature}: {clause}' for clause in source.get('unresolved_clauses', []))
    return {item for item in items if item}


def _report(root: Path, entry: dict, key: str, errors: list[str]) -> dict | None:
    label = f'evidence {key}'
    path_value = entry.get('path') if isinstance(entry, dict) else None
    if not path_value:
        errors.append(f'{label}: missing path')
        return None
    path = root / path_value
    if not path.is_file():
        errors.append(f'{label}: report is missing ({path_value})')
        return None
    try:
        report = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        errors.append(f'{label}: report is not valid JSON')
        return None
    if entry.get('sha256') != sha256(path):
        errors.append(f'{label}: report hash mismatch')
    return report


def _stable_core_report(report: dict, label: str, errors: list[str], root: Path) -> None:
    if report.get('passed') is not True or report.get('source_stable') is not True:
        errors.append(f'{label}: report is not a stable passing run')
    if report.get('source_sha256') != core_source_hash(root):
        errors.append(f'{label}: core source hash is missing or stale')


def _worker_candidates(root: Path) -> list[Path]:
    return [
        root / 'target/evidence/debug/wtime-allocation-check',
        root / 'target/debug/wtime-allocation-check',
    ]


def _conformance_candidates(root: Path) -> list[Path]:
    return [
        root / 'target/evidence/release/wtime-conformance',
        root / 'target/evidence/debug/wtime-conformance',
        root / 'target/release/wtime-conformance',
        root / 'target/debug/wtime-conformance',
    ]


def _probe_candidates(root: Path) -> list[Path]:
    return [
        root / 'target/evidence/debug/wtime-probe',
        root / 'target/debug/wtime-probe',
    ]


def _worker_command_matches(command: object, worker: Path, root: Path) -> bool:
    if not isinstance(command, list) or len(command) != 1 or not isinstance(command[0], str):
        return False
    accepted = {str(worker.resolve())}
    try:
        accepted.add(worker.resolve().relative_to(root.resolve()).as_posix())
    except ValueError:
        pass
    return command[0] in accepted


def _check_allocation_failure(report: dict, errors: list[str], root: Path, *,
                              verify_local_binaries: bool = True) -> None:
    """Validate the allocation-failure producer's concrete report schema/provenance."""
    label = 'evidence allocation_failure'
    if report.get('passed') is not True or report.get('source_stable') is not True:
        errors.append(f'{label}: report is not a stable passing run')
    if report.get('source_sha256') != core_source_hash(root):
        errors.append(f'{label}: core source hash is missing or stale')
    if report.get('architecture') not in ('x86_64', 'aarch64'):
        errors.append(f'{label}: native architecture is missing or unsupported')
    command = report.get('command')
    worker_digest = report.get('worker_sha256')
    if verify_local_binaries:
        available = [candidate for candidate in _worker_candidates(root) if candidate.is_file()]
        matching_cmd = [candidate for candidate in available if _worker_command_matches(command, candidate, root)]
        if not matching_cmd:
            errors.append(f'{label}: worker command is missing or does not match the local worker')
        if (not isinstance(worker_digest, str)
                or not re.fullmatch(r'[0-9a-f]{64}', worker_digest)
                or not any(sha256(candidate) == worker_digest for candidate in (matching_cmd or available))):
            errors.append(f'{label}: worker binary fingerprint is missing or stale')
    else:
        allowed_suffixes = (
            'target/evidence/debug/wtime-allocation-check',
            'target/debug/wtime-allocation-check',
        )
        if (not isinstance(command, list) or len(command) != 1
                or not isinstance(command[0], str)
                or not command[0].endswith(allowed_suffixes)):
            errors.append(f'{label}: worker command is missing or malformed')
        if not isinstance(worker_digest, str) or not re.fullmatch(r'[0-9a-f]{64}', worker_digest):
            errors.append(f'{label}: worker binary fingerprint is missing or malformed')
    if (report.get('address_space_bytes') != 128 * 1024 * 1024
            or report.get('requested_allocation_bytes') != 512 * 1024 * 1024):
        errors.append(f'{label}: report does not record the constrained failing allocation')
    stdout = report.get('stdout')
    try:
        result = json.loads(stdout) if isinstance(stdout, str) else {}
    except json.JSONDecodeError:
        result = {}
    if (report.get('executed') != 1 or report.get('exit_code') != 0
            or result.get('allocator_failed') is not True
            or result.get('guest_table_allocation_failed') is not True
            or result.get('element_allocation_failed') is not True
            or result.get('frame_allocation_failed') is not True
            or result.get('label_allocation_failed') is not True
            or result.get('recovery_passed') is not True or result.get('executed') != 1):
        errors.append(f'{label}: worker did not report allocator failure and successful recovery')


def _git_head(root: Path) -> str | None:
    if not (root / '.git').exists():
        return None
    proc = subprocess.run(
        ['git', 'rev-parse', 'HEAD'],
        cwd=root,
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        return None
    rev = proc.stdout.strip()
    return rev if re.fullmatch(r'[0-9a-f]{40}', rev) else None


def _git_revision_matches_core_tree(root: Path, revision: str) -> bool:
    head = _git_head(root)
    if head is None:
        return True
    if revision == head:
        return True
    anc = subprocess.run(
        ['git', 'merge-base', '--is-ancestor', revision, head],
        cwd=root,
        capture_output=True,
    )
    if anc.returncode != 0:
        return False
    diff = subprocess.run(
        ['git', 'diff', '--quiet', f'{revision}..{head}', '--', 'Cargo.toml', 'Cargo.lock', 'crates/'],
        cwd=root,
        capture_output=True,
    )
    return diff.returncode == 0


def _source_hash_entries(source_hash: str) -> dict[str, str]:
    entries: dict[str, str] = {}
    for part in source_hash.split(';'):
        path, sep, digest = part.rpartition(':')
        if sep and path and digest:
            entries[path] = digest
    return entries


def _check_conformance(report: dict, key: str, errors: list[str], root: Path,
                       spec_revision: str, corpus_revision: str, *,
                       verify_local_binaries: bool = True,
                       corpus_lock: dict | None = None,
                       expected_revision: str | None = None,
                       check_git_provenance: bool = False) -> None:
    label = f'evidence {key}'
    selected, executed = report.get('selected', 0), report.get('executed', 0)
    if not isinstance(selected, int) or selected <= 0 or executed != selected:
        errors.append(f'{label}: selected assertions were skipped, empty, or unaccounted')
    if any(report.get(field, 0) != 0 for field in ('failed', 'skipped', 'unsupported', 'text_tool_errors')):
        errors.append(f'{label}: failures, unsupported directives, skips, or tool errors remain')
    expected = {'mvp': ('mvp', 'mvp'),
                'core3': ('stable-core', 'core3'),
                'phase3': ('all-phase3-core', 'phase3'),
                'interaction_corpus': ('all-phase3-core', 'interactions')}.get(key)
    if expected and (report.get('profile'), report.get('suite')) != expected:
        errors.append(f'{label}: profile/suite does not match the release gate')
    source_hash = report.get('source_hash', '')
    if not source_hash.startswith(f'corpus-lock:{corpus_revision};'):
        errors.append(f'{label}: corpus lock hash is missing or stale')
    if isinstance(corpus_lock, dict) and expected:
        suite_name = expected[1]
        suite_files = corpus_lock.get('suites', {}).get(suite_name)
        if isinstance(suite_files, list) and suite_files:
            pinned = {row['path']: row['sha256'] for row in corpus_lock.get('files', []) if 'path' in row and 'sha256' in row}
            recorded = _source_hash_entries(source_hash)
            missing_or_mismatched = []
            for rel in suite_files:
                expected_sha = pinned.get(rel)
                if not expected_sha:
                    continue
                resolved = (root / '.cache' / rel).resolve()
                try:
                    recorded_key = resolved.relative_to(root.resolve()).as_posix()
                except ValueError:
                    recorded_key = rel
                if recorded.get(recorded_key) != expected_sha and recorded.get(rel) != expected_sha:
                    missing_or_mismatched.append(rel)
            if missing_or_mismatched:
                errors.append(f'{label}: full suite selection incomplete ({len(missing_or_mismatched)} corpus file(s) missing or stale)')
            replay = report.get('replay')
            if isinstance(replay, str) and "'--filter' 'all'" not in replay and '--filter all' not in replay:
                errors.append(f'{label}: replay command did not select full suite (--filter all)')
    engine_revision = report.get('engine_revision')
    if not isinstance(engine_revision, str) or not re.fullmatch(r'[0-9a-f]{40}', engine_revision):
        errors.append(f'{label}: engine revision must be a clean 40-character commit SHA')
    elif expected_revision is not None and engine_revision != expected_revision:
        errors.append(f'{label}: engine revision {engine_revision} does not match checkout HEAD {expected_revision}')
    elif check_git_provenance and not _git_revision_matches_core_tree(root, engine_revision):
        errors.append(f'{label}: engine revision {engine_revision} does not match current core tree provenance')
    if not re.fullmatch(r'[0-9a-f]{64}', report.get('binary_sha256', '')):
        errors.append(f'{label}: executed binary fingerprint is missing')
    elif verify_local_binaries:
        available = [binary for binary in _conformance_candidates(root) if binary.is_file()]
        if not available:
            errors.append(f'{label}: current conformance executable is missing')
        elif not any(sha256(binary) == report['binary_sha256'] for binary in available):
            errors.append(f'{label}: executed binary fingerprint is stale')
    implementation_sources = report.get('implementation_sources', {})
    if not implementation_sources:
        errors.append(f'{label}: implementation source fingerprints are missing')
    for source, digest in implementation_sources.items():
        path = root / source
        if not path.is_file() or not re.fullmatch(r'[0-9a-f]{64}', digest) or sha256(path) != digest:
            errors.append(f'{label}: stale implementation source fingerprint: {source}')
    if report.get('core_spec_commit') and report.get('core_spec_commit') != spec_revision:
        errors.append(f'{label}: Core specification revision is stale')


def audit(manifest: dict, *, root: Path = ROOT, tasks_root: Path | None = TASKS_ROOT,
          verify_local_binaries: bool = True) -> list[str]:
    errors: list[str] = []
    if manifest.get('schema_version') != 1:
        errors.append('manifest schema_version must be 1')

    lock_path = root / 'docs/architecture/spec-lock.json'
    ledger_path = root / 'docs/architecture/opcodes.json'
    if not lock_path.is_file() or not ledger_path.is_file():
        return ['missing source lock or opcode ledger']
    lock, ledger = (json.loads(p.read_text()) for p in (lock_path, ledger_path))
    if tasks_root is not None:
        plan_path = tasks_root / 'TASKS.json'
        if not plan_path.is_file():
            return [f'missing task graph: {plan_path}']
        plan = json.loads(plan_path.read_text())
        by_id = {task['id']: task for task in plan.get('tasks', [])}
        required_tasks = task_closure(plan)
        if 'T169' not in by_id:
            errors.append('task graph has no T169 definition')
        for task_id in sorted(required_tasks):
            task = by_id.get(task_id)
            if task is None or task.get('status') != 'done':
                errors.append(f'{task_id}: required task is missing or not done')
            elif task.get('path'):
                task_file = tasks_root / task['path']
                expected_line = f"Status: **{task['status']}**"
                if not task_file.is_file() or expected_line not in task_file.read_text():
                    errors.append(f'{task_id}: task file status does not match the task graph')

    current_source = core_source_hash(root)
    if manifest.get('source_sha256') != current_source:
        errors.append('manifest source_sha256 is missing or stale')
    if manifest.get('spec_revision') != lock.get('core_spec_commit'):
        errors.append('manifest spec_revision does not match the pinned Core specification')
    corpus_lock_path = root / 'docs/architecture/corpus-lock.json'
    if not corpus_lock_path.is_file():
        errors.append('missing pinned corpus lock')
    else:
        corpus_lock = json.loads(corpus_lock_path.read_text())
        if manifest.get('corpus_revision') != corpus_lock.get('revision'):
            errors.append('manifest corpus_revision does not match corpus-lock.json')

    claimed = set(manifest.get('supported_claims', []))
    unsupported = unsupported_items(lock)
    included = {source.get('feature_id') for source in lock.get('sources', [])
                if source.get('feature_id') != 'proposal-catalog'}
    claim_features = {claim.split(':', 1)[0] for claim in claimed}
    for feature in sorted(included - claim_features):
        errors.append(f'included feature is missing from supported snapshot: {feature}')
    for feature in sorted(claim_features - included):
        errors.append(f'supported claim is absent from the pinned source scope: {feature}')
    for source in lock.get('sources', []):
        feature = source.get('feature_id')
        if source.get('unresolved_clauses') and feature in claim_features and not any(
                claim.startswith(feature + ':') for claim in claimed):
            errors.append(f'{feature}: claim must be limited to its specified semantics because source clauses remain unresolved')
    overlap = claimed & unsupported
    for claim in sorted(overlap):
        errors.append(f'unsupported behavior claimed complete: {claim}')
    declared_unsupported = set(manifest.get('unsupported_behavior', []))
    for item in sorted(unsupported - declared_unsupported):
        errors.append(f'unsupported behavior is not explicitly documented: {item}')

    pending_owners = 0
    for row in ledger.get('instructions', []) + ledger.get('non_opcode_features', []):
        for owner in row.get('owners', {}).values():
            if owner.get('status') == 'pending-audit':
                pending_owners += 1
    if pending_owners:
        errors.append(f'opcode/rule ledger has {pending_owners} pending semantic owners')

    evidence = manifest.get('evidence', {})
    required_evidence = required_evidence_keys(manifest)
    missing_keys = required_evidence - set(evidence)
    for key in sorted(missing_keys):
        errors.append(f'missing required evidence block: {key}')
    for key in sorted(required_evidence & set(evidence)):
        entry = evidence[key]
        label = f'evidence {key}'
        path_value = entry.get('path') if isinstance(entry, dict) else None
        if not path_value:
            errors.append(f'{label}: missing path')
            continue
        path = root / path_value
        if not path.is_file():
            errors.append(f'{label}: report is missing ({path_value})')
            continue
        report = _report(root, entry, key, errors)
        if report is None:
            continue
        if key in ('feature_profiles', 'interactions', 'api_docs'):
            _stable_core_report(report, label, errors, root)
            if not isinstance(report.get('executed'), int) or report['executed'] <= 0:
                errors.append(f'{label}: no assertions/tests executed')
            if key == 'api_docs':
                for document, digest in report.get('documents', {}).items():
                    path = root / document
                    if not path.is_file() or sha256(path) != digest:
                        errors.append(f'{label}: stale document fingerprint: {document}')
                if any(result.get('passed') is not True or result.get('exit_code') != 0
                       for result in report.get('results', [])):
                    errors.append(f'{label}: an embedding example failed')
        elif key in ('core3', 'phase3', 'interaction_corpus'):
            _check_conformance(report, key, errors, root, lock['core_spec_commit'],
                               hashlib.sha256(corpus_lock_path.read_bytes()).hexdigest(),
                               verify_local_binaries=verify_local_binaries,
                               corpus_lock=corpus_lock if corpus_lock_path.is_file() else None,
                               check_git_provenance=True)
        elif key in ('fuzz', 'differential'):
            if report.get('failed') != 0 or report.get('selected', 0) <= 0 or report.get('executed') != report.get('selected') or report.get('failures'):
                errors.append(f'{label}: run is empty, incomplete, or has failures')
            if report.get('core_spec_commit') != lock.get('core_spec_commit'):
                errors.append(f'{label}: Core specification revision is stale')
            generator = root / ('tools/checks/fuzz_decode.py' if key == 'fuzz' else 'tools/checks/differential.py')
            if report.get('generator_sha256') != sha256(generator):
                errors.append(f'{label}: generator fingerprint is missing or stale')
            target_digest = report.get('target_sha256')
            if verify_local_binaries:
                probes = [probe for probe in _probe_candidates(root) if probe.is_file()]
                if not probes or not any(sha256(probe) == target_digest for probe in probes):
                    errors.append(f'{label}: target binary fingerprint is missing or stale')
            elif not isinstance(target_digest, str) or not re.fullmatch(r'[0-9a-f]{64}', target_digest):
                errors.append(f'{label}: target binary fingerprint is missing or malformed')
            if key == 'fuzz':
                if report.get('profile') != 'all-phase3-core' or report.get('oracle_unsupported') != 0 or report.get('oracle_differences') != 0:
                    errors.append(f'{label}: required profile or oracle checks are incomplete')
                for source, digest in report.get('corpus_sha256', {}).items():
                    path = root / source
                    if not path.is_file() or sha256(path) != digest:
                        errors.append(f'{label}: stale fixture fingerprint: {source}')
            elif report.get('oracle_compared', 0) <= 0 or report.get('oracle_not_run') != 0:
                errors.append(f'{label}: independent oracle comparison is missing')
        elif key == 'build_final':
            measured_core = report.get('core_source_sha256', report.get('source_sha256'))
            if measured_core != current_source or report.get('source_stable') is not True:
                errors.append(f'{label}: core source hash is missing/stale or source changed during measurement')
            lock_digest = sha256(root / 'Cargo.lock')
            if report.get('lock_sha256') != lock_digest:
                errors.append(f'{label}: Cargo.lock fingerprint is stale')
            if report.get('budget_pass') is not True:
                errors.append(f'{label}: build budget did not pass')
            if report.get('clean_seconds', float('inf')) > report.get('budgets', {}).get('clean_seconds', 0) or report.get('leaf_seconds', float('inf')) > report.get('budgets', {}).get('leaf_seconds', 0) or report.get('stripped_bytes', float('inf')) > report.get('budgets', {}).get('stripped_bytes', 0):
                errors.append(f'{label}: a recorded build budget is exceeded')
        elif key.startswith('native_'):
            arch = key.removeprefix('native_')
            if report.get('source_sha256') != current_source or report.get('source_stable') is not True:
                errors.append(f'{label}: core source hash is missing/stale or source changed during collection')
            if report.get('architecture') != arch or report.get('native_execution') is not True:
                errors.append(f'{label}: evidence was not collected on native {arch}')
            if report.get('passed') is not True or report.get('exit_code') != 0 or report.get('tests_executed', 0) <= 0:
                errors.append(f'{label}: native tests failed or executed zero tests')
            if arch == 'x86_64' and not all(marker in report.get('output', '') for marker in (
                    't045_cancelled_execution_releases_frames_and_store_can_resume',
                    't036_reuse_rejects_stale_generation')):
                errors.append(f'{label}: cancellation or stale-generation regression was not executed')
        elif key == 'allocation_failure':
            _check_allocation_failure(report, errors, root, verify_local_binaries=verify_local_binaries)
    if 'interactions' in evidence:
        interaction_entry = evidence.get('interactions')
        corpus_report = interaction_entry.get('corpus_report') if isinstance(interaction_entry, dict) else None
        corpus_sha = interaction_entry.get('corpus_sha256') if isinstance(interaction_entry, dict) else None
        if not corpus_report or not corpus_sha:
            errors.append('evidence interaction_corpus: missing corpus_report or corpus_sha256')
        else:
            related = {**interaction_entry, 'path': corpus_report, 'sha256': corpus_sha}
            report = _report(root, related, 'interaction_corpus', errors)
            if report is not None:
                _check_conformance(report, 'interaction_corpus', errors, root, lock['core_spec_commit'],
                                   hashlib.sha256(corpus_lock_path.read_bytes()).hexdigest(),
                                   verify_local_binaries=verify_local_binaries,
                                   corpus_lock=corpus_lock if corpus_lock_path.is_file() else None,
                                   check_git_provenance=True)

    toolchain = manifest.get('toolchain', {})
    if not toolchain.get('rustc') or not toolchain.get('supported_targets'):
        errors.append('toolchain version and supported targets must be recorded')
    else:
        build_entry = evidence.get('build_final', {})
        build_path = root / build_entry.get('path', '')
        if build_path.is_file():
            build_report = json.loads(build_path.read_text())
            if not build_report.get('rustc', '').startswith(toolchain['rustc']):
                errors.append('manifest rustc does not match final build report')
            host = next((line.removeprefix('host: ') for line in build_report.get('rustc', '').splitlines()
                         if line.startswith('host: ')), None)
            if host and host not in toolchain['supported_targets']:
                errors.append(f'manifest omits the measured build host target: {host}')
    licenses = manifest.get('licenses', {})
    license_path = root / licenses.get('report_path', '') if licenses.get('report_path') else None
    if (not licenses.get('project_license') or not license_path or not license_path.is_file()
            or licenses.get('report_sha256') != sha256(license_path)
            or '0 violations' not in license_path.read_text()):
        errors.append('project and production dependency license evidence is incomplete')
    return errors


def audit_live_dir(live_dir: Path, *, root: Path = ROOT) -> list[str]:
    """Strictly verify freshly generated CI conformance and allocation reports."""
    errors: list[str] = []
    lock_path = root / 'docs/architecture/spec-lock.json'
    corpus_lock_path = root / 'docs/architecture/corpus-lock.json'
    if not lock_path.is_file() or not corpus_lock_path.is_file():
        return ['missing source lock or corpus lock']
    spec_revision = json.loads(lock_path.read_text())['core_spec_commit']
    corpus_lock = json.loads(corpus_lock_path.read_text())
    corpus_digest = hashlib.sha256(corpus_lock_path.read_bytes()).hexdigest()
    head_rev = _git_head(root)
    for filename, key in (
        ('mvp.json', 'mvp'),
        ('core3.json', 'core3'),
        ('phase3.json', 'phase3'),
        ('interactions.json', 'interaction_corpus'),
    ):
        path = live_dir / filename
        if not path.is_file():
            errors.append(f'live {key}: report is missing ({path})')
            continue
        try:
            report = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            errors.append(f'live {key}: report is not valid JSON')
            continue
        _check_conformance(report, key, errors, root, spec_revision, corpus_digest,
                           verify_local_binaries=True,
                           corpus_lock=corpus_lock,
                           expected_revision=head_rev)
    alloc_path = live_dir / 'allocation-failure.json'
    if not alloc_path.is_file():
        errors.append(f'live allocation_failure: report is missing ({alloc_path})')
    else:
        try:
            alloc_report = json.loads(alloc_path.read_text())
        except (OSError, json.JSONDecodeError):
            errors.append('live allocation_failure: report is not valid JSON')
        else:
            _check_allocation_failure(alloc_report, errors, root, verify_local_binaries=True)
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, default=MANIFEST)
    parser.add_argument('--tasks-root', type=Path, default=TASKS_ROOT)
    parser.add_argument(
        '--portable',
        action='store_true',
        help='verify all source, lock, corpus, generator, fixture, and report digests without requiring local target/ binaries to match the release-machine ELF SHA-256',
    )
    parser.add_argument(
        '--verify-live-dir',
        type=Path,
        default=None,
        help='strictly validate freshly generated conformance/allocation reports in DIR against local target/ binaries',
    )
    args = parser.parse_args()
    if args.verify_live_dir is not None:
        errors = audit_live_dir(args.verify_live_dir)
        if errors:
            print('\n'.join(errors), file=sys.stderr)
            print(f'Live conformance audit blocked: {len(errors)} unresolved gate(s)')
            return 1
        print(f'Live conformance audit passed: {args.verify_live_dir}')
        return 0
    try:
        manifest = json.loads(args.manifest.read_text())
    except (OSError, json.JSONDecodeError) as error:
        print(f'cannot read release manifest: {error}', file=sys.stderr)
        return 2
    errors = audit(
        manifest,
        tasks_root=args.tasks_root,
        verify_local_binaries=not args.portable,
    )
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        print(f'Release audit blocked: {len(errors)} unresolved gate(s)')
        return 1
    print('Release audit passed: required tasks, locks, coverage, reports and claims agree')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
