#!/usr/bin/env python3
"""Inert task-status publisher; requires --execute and an exclusive snapshot dir.

It verifies an evidence record and task dependencies before changing one task's
status/outcome/evidence/completion_record, that task's executed-evidence section,
and the INDEX.md progress line. It never updates other task bodies.
"""
from __future__ import annotations

import argparse
import ast
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys

TASK_ID = re.compile(r"P\d{3}")
SHA256 = re.compile(r"[0-9a-f]{64}")


class PublishError(RuntimeError):
    pass


def need(condition: bool, message: str) -> None:
    if not condition:
        raise PublishError(message)


def file_sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path: Path) -> dict:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise PublishError(f"invalid/missing JSON {path}: {exc}") from exc
    need(isinstance(value, dict), f"expected an object in {path}")
    return value


def allowlists(checker: Path) -> tuple[set[str], set[str]]:
    """Read the accepted labels from check_plan.py without importing/executing it."""
    try:
        tree = ast.parse(checker.read_text(encoding="utf-8"), filename=str(checker))
    except (OSError, SyntaxError) as exc:
        raise PublishError(f"cannot read static task-validator allowlists: {exc}") from exc
    found = {}
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        names = {target.id for target in node.targets if isinstance(target, ast.Name)}
        for name in ("STATUSES", "OUTCOMES"):
            if name in names:
                try:
                    value = ast.literal_eval(node.value)
                except (ValueError, TypeError) as exc:
                    raise PublishError(f"validator {name} is not a static literal") from exc
                need(isinstance(value, set) and all(isinstance(item, str) for item in value),
                     f"validator {name} has an unexpected shape")
                found[name] = value
    need(set(found) == {"STATUSES", "OUTCOMES"}, "validator status/outcome allowlists are missing")
    return found["STATUSES"], found["OUTCOMES"]


def roots(performance_root: Path) -> tuple[Path, Path, Path]:
    performance_root = performance_root.resolve()
    need(performance_root.name == "performance" and performance_root.parent.name == "tasks",
         "--performance-root must name the tasks/performance directory")
    workspace = performance_root.parent.parent
    repository = workspace / "wtime"
    need(repository.is_dir(), f"expected repository checkout at {repository}")
    return performance_root, workspace, repository


def resolve_artifact(raw: str, workspace: Path, repository: Path,
                     performance_root: Path) -> Path:
    path = Path(raw)
    if path.is_absolute():
        return path.resolve()
    candidates = [repository / path, workspace / path, performance_root / path]
    existing = [candidate.resolve() for candidate in candidates if candidate.is_file()]
    unique = sorted(set(existing), key=str)
    need(len(unique) == 1, f"artifact path {raw!r} is missing or ambiguous")
    return unique[0]


def copy_snapshot(source: Path, destination: Path) -> dict:
    need(source.is_file(), f"required prewrite snapshot source is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    return {"source": str(source.resolve()), "snapshot": str(destination.resolve()),
            "sha256": file_sha(destination), "bytes": destination.stat().st_size}


def atomic_write(path: Path, content: str) -> None:
    temporary = path.with_name(path.name + f".status-{os.getpid()}.tmp")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def task_progress(tasks: list[dict]) -> str:
    counts = {name: 0 for name in ("done", "todo", "in_progress", "blocked", "deferred")}
    for task in tasks:
        status = task.get("status")
        need(status in counts, f"cannot summarize unknown task status {status!r}")
        counts[status] += 1
    total = len(tasks)
    return ("Progress: " + ", ".join(f"{name}={counts[name]}" for name in counts)
            + f" ({total} total). Read [README.md](README.md), [EXECUTION.md](EXECUTION.md), "
              "and [TASKS.json](TASKS.json).")


def execute(performance_root: Path, record_path: Path, snapshot_dir: Path) -> int:
    performance_root, workspace, repository = roots(performance_root)
    task_index_path = performance_root / "TASKS.json"
    index_path = performance_root / "INDEX.md"
    checker_path = performance_root / "check_plan.py"
    status_root = repository / "target/performance/TASK-status"
    snapshot_dir = snapshot_dir.resolve()
    need(snapshot_dir.parent.resolve() == status_root.resolve(),
         "snapshot directory must be a direct child of target/performance/TASK-status")
    status_root.mkdir(parents=True, exist_ok=True)
    snapshot_dir.mkdir(exist_ok=False)

    # Preserve the exact source state before validating any record or task fields.
    snapshots = {}
    snapshots["helper"] = copy_snapshot(Path(__file__), snapshot_dir / "publish_task_status.executed.py")
    snapshots["TASKS.json"] = copy_snapshot(task_index_path, snapshot_dir / "TASKS.json.before")
    snapshots["INDEX.md"] = copy_snapshot(index_path, snapshot_dir / "INDEX.md.before")
    snapshots["completion_record"] = copy_snapshot(record_path.resolve(), snapshot_dir / "completion-record.input.json")
    initial_manifest = read_json(task_index_path)
    raw_record = read_json(record_path)
    task_rows = initial_manifest.get("tasks", [])
    task_row = next((row for row in task_rows if isinstance(row, dict)
                     and row.get("id") == raw_record.get("task")), None)
    task_md_path = None
    if task_row is not None and isinstance(task_row.get("path"), str):
        candidate_md = (performance_root / task_row["path"]).resolve()
        if performance_root.resolve() in candidate_md.parents and candidate_md.is_file():
            task_md_path = candidate_md
            snapshots["task_markdown"] = copy_snapshot(candidate_md, snapshot_dir / "task.md.before")
    if "task_markdown" not in snapshots:
        snapshots["task_markdown"] = {"unavailable": "task ID/path did not resolve to a file within performance root"}
    snapshot_metadata = {
        "schema": "wtime-task-status-prewrite-snapshot-v1",
        "task": raw_record.get("task"),
        "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "files": snapshots,
    }
    snapshot_manifest = snapshot_dir / "prewrite-snapshot.json"
    snapshot_manifest.write_text(json.dumps(snapshot_metadata, indent=2, sort_keys=True) + "\n",
                                 encoding="utf-8")
    attempt_path = snapshot_dir / "publication-attempt.json"
    attempt = {"schema": "wtime-task-status-publication-v1", "completed": False,
               "snapshot": str(snapshot_manifest.resolve()),
               "snapshot_sha256": file_sha(snapshot_manifest)}

    try:
        need(task_row is not None and isinstance(task_row.get("path"), str)
             and task_md_path is not None,
             "completion record task ID/path is absent from TASKS.json or escapes its directory")
        statuses, outcomes = allowlists(checker_path)
        record = read_json(record_path)
        need(file_sha(record_path) == snapshots["completion_record"]["sha256"],
             "completion record changed after snapshot")
        task_id, new_status, new_outcome = record.get("task"), record.get("status"), record.get("outcome")
        need(isinstance(task_id, str) and TASK_ID.fullmatch(task_id), "completion record has invalid task ID")
        need(new_status in statuses and new_outcome in outcomes,
             "completion record status/outcome are not accepted by check_plan.py")
        need(task_id == task_row.get("id"), "completion record task changed after snapshot")
        need(task_row.get("status") in {"todo", "in_progress"}
             and task_row.get("outcome") == "pending",
             "refusing to overwrite a task that is already done, deferred, blocked, or non-pending")
        need(new_status != "todo", "publisher accepts only progress/completion statuses, not todo")
        if new_status == "done":
            need(new_outcome != "pending", "a done task cannot retain pending outcome")

        dependencies = task_row.get("dependencies", [])
        need(isinstance(dependencies, list), "task dependency list is malformed")
        by_id = {row.get("id"): row for row in task_rows if isinstance(row, dict)}
        for dependency in dependencies:
            dep = by_id.get(dependency)
            need(dep is not None, f"task has a missing dependency {dependency!r}")
            need(dep.get("status") in statuses and dep.get("outcome") in outcomes,
                 f"dependency {dependency} has invalid status/outcome")
        if new_status == "done":
            pending = list(dependencies)
            checked = set()
            while pending:
                dependency = pending.pop()
                if dependency in checked:
                    continue
                checked.add(dependency)
                dep = by_id.get(dependency)
                need(dep is not None, f"task dependency closure has a missing node {dependency!r}")
                need(dep.get("status") in statuses and dep.get("outcome") in outcomes,
                     f"dependency {dependency} has invalid status/outcome")
                need(dep.get("status") == "done",
                     f"cannot complete task while dependency {dependency} is {dep.get('status')}")
                subdeps = dep.get("dependencies", [])
                need(isinstance(subdeps, list), f"dependency {dependency} has a malformed dependency list")
                pending.extend(subdeps)
        enable_when = task_row.get("enable_when")
        if new_status == "done" and enable_when is not None:
            match = re.fullmatch(r"(P\d{3})\.outcome\s*==\s*([a-z_]+)", str(enable_when))
            need(match is not None, "task has malformed enable_when rule")
            enabled_dep = by_id.get(match.group(1))
            need(enabled_dep is not None and enabled_dep.get("outcome") == match.group(2),
                 "task completion does not satisfy its enable_when outcome")

        artifact_hashes = record.get("artifact_hashes")
        need(isinstance(artifact_hashes, dict) and artifact_hashes,
             "completion record must include nonempty artifact_hashes")
        artifacts = []
        artifacts_by_path = {}
        for raw_path, expected_sha in sorted(artifact_hashes.items()):
            need(isinstance(raw_path, str) and isinstance(expected_sha, str)
                 and SHA256.fullmatch(expected_sha), "artifact hash entry is malformed")
            artifact_path = resolve_artifact(raw_path, workspace, repository, performance_root)
            actual_sha = file_sha(artifact_path)
            need(actual_sha == expected_sha,
                 f"artifact hash mismatch for {raw_path}: expected {expected_sha}, got {actual_sha}")
            artifact_ref = {"path": str(artifact_path), "sha256": actual_sha,
                            "bytes": artifact_path.stat().st_size}
            artifacts.append(artifact_ref)
            artifacts_by_path[str(artifact_path)] = artifact_ref

        raw_evidence = record.get("evidence")
        need(isinstance(raw_evidence, list) and len(raw_evidence) == 1,
             "completion record must include exactly one canonical authoritative proof ref")
        proof_refs = []
        seen_proofs = set()
        for raw_ref in raw_evidence:
            need(isinstance(raw_ref, dict) and isinstance(raw_ref.get("path"), str)
                 and isinstance(raw_ref.get("sha256"), str)
                 and SHA256.fullmatch(raw_ref["sha256"])
                 and type(raw_ref.get("bytes")) is int,
                 "completion evidence ref must contain path/SHA/bytes")
            proof_path = resolve_artifact(raw_ref["path"], workspace, repository, performance_root)
            key = str(proof_path)
            full_ref = artifacts_by_path.get(key)
            need(full_ref is not None and full_ref["sha256"] == raw_ref["sha256"]
                 and full_ref["bytes"] == raw_ref["bytes"],
                 f"authoritative proof ref is absent from or differs from artifact_hashes: {raw_ref['path']}")
            need(key not in seen_proofs, "completion record repeats an authoritative proof ref")
            seen_proofs.add(key)
            proof_refs.append(full_ref)
            proof_contents = read_json(proof_path)
            need(proof_contents.get('development') is not True
                 and proof_contents.get('lane') != 'development'
                 and proof_contents.get('completion_eligible', True),
                 'development evidence cannot complete a real task')
            need(proof_contents.get('synthetic') is not True, 'synthetic workflow evidence cannot complete a real task')
            if new_status == 'done':
                need(proof_contents.get('task') == task_id and proof_contents.get('outcome') == new_outcome
                     and (proof_contents.get('passed') is True or proof_contents.get('status') == 'passed'),
                     'canonical proof task/outcome/status differs from completion record')

        # Refuse stale snapshots before preparing all three output files.
        for path, key in ((task_index_path, "TASKS.json"), (index_path, "INDEX.md"),
                          (task_md_path, "task_markdown")):
            need(file_sha(path) == snapshots[key]["sha256"],
                 f"task source changed after snapshot: {path}")
        fresh_manifest = read_json(task_index_path)
        fresh_task = next((row for row in fresh_manifest["tasks"] if row.get("id") == task_id), None)
        need(fresh_task == task_row, "target task row changed after prewrite snapshot")
        task_row = fresh_task

        # Only the declared status/evidence fields change in this task row.
        original_row = dict(task_row)
        task_row["status"] = new_status
        task_row["outcome"] = new_outcome
        task_row["evidence"] = proof_refs
        task_row["completion_record"] = record
        changed_keys = {key for key in set(original_row) | set(task_row)
                        if original_row.get(key) != task_row.get(key)}
        need(changed_keys <= {"status", "outcome", "evidence", "completion_record"},
             "publisher attempted to change non-status task metadata")
        task_md = task_md_path.read_text(encoding="utf-8")
        status_pattern = re.compile(r"^Status:\s+\*\*.*?\*\*$", re.MULTILINE)
        outcome_pattern = re.compile(r"^Outcome:\s+\*\*.*?\*\*$", re.MULTILINE)
        need(len(status_pattern.findall(task_md)) == 1 and len(outcome_pattern.findall(task_md)) == 1,
             "task Markdown must contain one Status and one Outcome line")
        task_md = status_pattern.sub(f"Status: **{new_status}**", task_md, count=1)
        task_md = outcome_pattern.sub(f"Outcome: **{new_outcome}**", task_md, count=1)
        task_md = task_md.split("\n## Executed evidence\n", 1)[0]
        task_md = task_md.replace(
            "No commands in this planning file have been executed as implementation evidence.",
            "Executed evidence is recorded below.")
        task_md += "\n## Executed evidence\n\n```json\n" + json.dumps(record, indent=2, sort_keys=True) + "\n```\n"

        index_text = index_path.read_text(encoding="utf-8")
        lines = index_text.splitlines(keepends=True)
        need(bool(lines), "INDEX.md is empty")
        progress_lines = [index for index, line in enumerate(lines)
                          if re.match(r"^(?:All \d+ tasks are planned|Progress:)", line)]
        need(len(progress_lines) == 1,
             "INDEX.md must have exactly one planned/progress introduction line")
        lines[progress_lines[0]] = task_progress(fresh_manifest["tasks"]) + "\n"
        index_text = "".join(lines)
        manifest_text = json.dumps(fresh_manifest, indent=2, ensure_ascii=False) + "\n"

        # Revalidate source bytes and evidence one final time immediately before writes.
        need(file_sha(record_path) == snapshots["completion_record"]["sha256"],
             "completion record changed before task writes")
        for artifact in artifacts:
            need(file_sha(Path(artifact["path"])) == artifact["sha256"],
                 f"artifact changed before task writes: {artifact['path']}")
        try:
            atomic_write(task_md_path, task_md)
            atomic_write(index_path, index_text)
            atomic_write(task_index_path, manifest_text)
        except Exception:
            for original_path, snapshot_name in (
                (task_index_path, "TASKS.json.before"),
                (index_path, "INDEX.md.before"),
                (task_md_path, "task.md.before"),
            ):
                snapshot_file = snapshot_dir / snapshot_name
                if snapshot_file.is_file():
                    atomic_write(original_path, snapshot_file.read_text(encoding="utf-8"))
            raise

        attempt.update({"completed": True, "task": task_id, "status": new_status,
                        "outcome": new_outcome, "artifact_hashes": artifacts,
                        "files_written": [str(task_md_path), str(index_path), str(task_index_path)]})
    except Exception as exc:
        attempt["error"] = f"{type(exc).__name__}: {exc}"
    attempt_path.write_text(json.dumps(attempt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if not attempt["completed"]:
        print(f"status publication blocked: {attempt['error']}", file=sys.stderr)
        print(f"prewrite evidence retained in {snapshot_dir}", file=sys.stderr)
        return 1
    print(f"published {attempt['task']} as {attempt['status']}/{attempt['outcome']}")
    print(f"prewrite evidence retained in {snapshot_dir}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="publish only after prewrite snapshots")
    parser.add_argument("--performance-root", type=Path)
    parser.add_argument("--completion-record", type=Path)
    parser.add_argument("--snapshot-dir", type=Path,
                        help="new exclusive child of wtime/target/performance/TASK-status")
    args = parser.parse_args()
    if not args.execute:
        print("inert: no files read or changed; pass --execute after evidence is complete")
        return 0
    if args.performance_root is None or args.completion_record is None or args.snapshot_dir is None:
        parser.error("--execute requires --performance-root, --completion-record, and --snapshot-dir")
    return execute(args.performance_root, args.completion_record, args.snapshot_dir)


if __name__ == "__main__":
    raise SystemExit(main())
