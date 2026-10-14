#!/usr/bin/env python3
"""Validate the performance planning snapshot without running implementation code."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urlsplit


TASK_ID = re.compile(r"P\d{3}")
STATUSES = {"todo", "in_progress", "done", "blocked", "deferred"}
OUTCOMES = {
    "pending", "complete", "passed", "failed", "adopted", "rejected",
    "inconclusive", "contract_only", "go", "no_go", "not_selected",
}
LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")


def _norm(value: str) -> str:
    return " ".join(value.split())


def _digest(path: Path) -> str | None:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


def _section(text: str, heading: str) -> str:
    match = re.search(
        rf"^##\s+{re.escape(heading)}\s*$([\s\S]*?)(?=^##\s+|\Z)",
        text,
        flags=re.MULTILINE | re.IGNORECASE,
    )
    return match.group(1) if match else ""


def _check_local_links(markdown: Path, base: Path, errors: list[str]) -> None:
    text = markdown.read_text(encoding="utf-8")
    for raw in LINK.findall(text):
        # Strip optional Markdown title and decode URL escaping.
        target = raw.strip().split(maxsplit=1)[0].strip("<>")
        parsed = urlsplit(target)
        if parsed.scheme or target.startswith("//") or not parsed.path:
            continue
        dest = (markdown.parent / unquote(parsed.path)).resolve()
        if not dest.exists():
            errors.append(f"{markdown.relative_to(base)}: missing local link target {parsed.path!r}")
            continue
        fragment = unquote(parsed.fragment)
        if fragment and dest.is_file() and dest.suffix.lower() in {".md", ".markdown"}:
            body = dest.read_text(encoding="utf-8")
            anchors = {
                re.sub(r"[^a-z0-9 -]", "", h.group(1).lower()).strip().replace(" ", "-")
                for h in re.finditer(r"^#{1,6}\s+(.+?)\s*#*\s*$", body, re.MULTILINE)
            }
            if fragment.lower() not in anchors:
                errors.append(f"{markdown.relative_to(base)}: missing Markdown anchor #{fragment} in {parsed.path!r}")


def validate(root: Path) -> tuple[list[str], list[str], dict[str, int]]:
    """Return errors, warnings, and useful plan counts for a planning directory."""
    errors: list[str] = []
    warnings: list[str] = []
    manifest_path = root / "TASKS.json"
    index_path = root / "INDEX.md"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return [f"cannot read TASKS.json: {exc}"], warnings, {}

    tasks = manifest.get("tasks")
    if not isinstance(tasks, list):
        return ["TASKS.json: tasks must be an array"], warnings, {}

    if manifest.get("schema_version") != 1:
        errors.append("TASKS.json: unsupported schema_version (expected 1)")
    model = manifest.get("implementation_model", {})
    if model.get("model") != "gpt-6.1-sol" or model.get("reasoning_effort") != "low":
        errors.append("TASKS.json: implementation_model must be gpt-6.1-sol / low")

    by_id: dict[str, dict[str, Any]] = {}
    keys: set[str] = set()
    paths: set[str] = set()
    valid_task_paths: set[str] = set()
    for pos, task in enumerate(tasks, 1):
        label = task.get("id", f"task at index {pos}") if isinstance(task, dict) else f"task at index {pos}"
        if not isinstance(task, dict):
            errors.append(f"{label}: task entry must be an object")
            continue
        tid, key, path = task.get("id"), task.get("key"), task.get("path")
        if not isinstance(tid, str) or not TASK_ID.fullmatch(tid):
            errors.append(f"{label}: invalid task id")
        elif tid in by_id:
            errors.append(f"{tid}: duplicate task id")
        else:
            by_id[tid] = task
        if not isinstance(key, str) or not key:
            errors.append(f"{label}: key must be nonempty text")
        elif key in keys:
            errors.append(f"{tid}: duplicate task key {key!r}")
        else:
            keys.add(key)
        if not isinstance(path, str) or not path:
            errors.append(f"{tid}: path must be nonempty text")
            continue
        if path in paths:
            errors.append(f"{tid}: duplicate task path {path!r}")
        paths.add(path)
        task_file = (root / path).resolve()
        if root.resolve() not in task_file.parents:
            errors.append(f"{tid}: task path escapes task directory")
            continue
        valid_task_paths.add(path)
        if not task_file.is_file():
            errors.append(f"{tid}: missing task file {path!r}")
            continue

        doc = task_file.read_text(encoding="utf-8")
        title = task.get("title")
        if not re.search(rf"^#\s+{re.escape(tid)}\s+—\s+{re.escape(str(title))}\s*$", doc, re.MULTILINE):
            errors.append(f"{tid}: task heading does not match manifest id/title")
        for field, pattern in (("status", r"^Status:\s+\*\*(.*?)\*\*"), ("outcome", r"^Outcome:\s+\*\*(.*?)\*\*")):
            value = task.get(field)
            allowed = STATUSES if field == "status" else OUTCOMES
            if value not in allowed:
                errors.append(f"{tid}: unknown {field} {value!r}")
            match = re.search(pattern, doc, re.MULTILINE)
            if not match or match.group(1).strip() != value:
                errors.append(f"{tid}: task file {field} disagrees with manifest")
        if task.get("implementation_model", {}) != {"model": "gpt-6.1-sol", "reasoning_effort": "low"}:
            errors.append(f"{tid}: implementation_model must be gpt-6.1-sol / low")
        owned = task.get("owned_paths")
        if not isinstance(owned, list) or not owned or any(not isinstance(p, str) for p in owned):
            errors.append(f"{tid}: owned_paths must be a nonempty list of strings")
        else:
            ownership_text = _section(doc, "Scope and ownership")
            for owned_path in owned:
                if f"`{owned_path}`" not in ownership_text:
                    errors.append(f"{tid}: owned path missing from Scope and ownership: {owned_path}")
        contract = task.get("input_output_contract")
        if not isinstance(contract, str) or not contract.strip():
            errors.append(f"{tid}: input_output_contract must be nonempty")
        elif _norm(contract) not in _norm(_section(doc, "Input/output contract")):
            errors.append(f"{tid}: input/output contract differs from task file")
        steps, cases, commands = task.get("implementation_steps"), task.get("fixture_cases"), task.get("test_commands")
        if not isinstance(steps, list) or len(steps) < 3:
            errors.append(f"{tid}: requires at least 3 implementation steps")
        elif any(not isinstance(x, str) or not _norm(x) for x in steps):
            errors.append(f"{tid}: implementation_steps must be nonempty strings")
        else:
            recipe = _section(doc, "Implementation recipe")
            for item in steps:
                if _norm(item) not in _norm(recipe):
                    errors.append(f"{tid}: implementation step missing or drifted: {item}")
        if not isinstance(cases, list) or len(cases) < 3:
            errors.append(f"{tid}: requires at least 3 fixture cases")
        elif any(not isinstance(x, str) or not _norm(x) for x in cases):
            errors.append(f"{tid}: fixture_cases must be nonempty strings")
        else:
            case_section = _section(doc, "Meaningful cases")
            for item in cases:
                if _norm(item) not in _norm(case_section):
                    errors.append(f"{tid}: fixture case missing or drifted: {item}")
        if not isinstance(commands, list) or not commands or any(not isinstance(x, str) or not x.strip() for x in commands):
            errors.append(f"{tid}: test_commands must be a nonempty list of strings")
        else:
            verification = _section(doc, "Verification")
            for command in commands:
                if _norm(command) not in _norm(verification):
                    errors.append(f"{tid}: test command missing or drifted: {command}")
        if "before_edit_commands" in task:
            before_commands = task["before_edit_commands"]
            if not isinstance(before_commands, list) or any(not isinstance(x, str) or not x.strip() for x in before_commands):
                errors.append(f"{tid}: before_edit_commands must be a list of nonempty strings")
            elif before_commands:
                before_section = _section(doc, "Before editing")
                for command in before_commands:
                    if _norm(command) not in _norm(before_section):
                        errors.append(f"{tid}: before-edit command missing or drifted: {command}")
        if not isinstance(task.get("execution_mode"), str) or not task["execution_mode"].strip():
            errors.append(f"{tid}: execution_mode must be nonempty text")
        if not isinstance(task.get("evidence_contract"), str) or not task["evidence_contract"].strip():
            errors.append(f"{tid}: evidence_contract must be nonempty text")

    # Dependencies and conditional gates form a DAG. Conditional references must also
    # be actual prerequisites, directly or through a dependency chain.
    graph: dict[str, list[str]] = {}
    for tid, task in by_id.items():
        deps = task.get("dependencies")
        if not isinstance(deps, list):
            errors.append(f"{tid}: dependencies must be a list")
            deps = []
        graph[tid] = []
        for dep in deps:
            if dep not in by_id:
                errors.append(f"{tid}: missing dependency {dep!r}")
            else:
                graph[tid].append(dep)
        reviews = task.get("reviews_optional_tasks", [])
        if not isinstance(reviews, list):
            errors.append(f"{tid}: reviews_optional_tasks must be a list")
            reviews = []
        for ref in reviews:
            if ref not in by_id:
                errors.append(f"{tid}: optional review references missing task {ref!r}")
            elif ref == tid:
                errors.append(f"{tid}: optional review cannot reference itself")
        condition = task.get("enable_when")
        if condition is not None:
            match = re.fullmatch(r"(P\d{3})\.outcome\s*==\s*([a-z_]+)", str(condition))
            if not match:
                errors.append(f"{tid}: malformed enable_when condition {condition!r}")
            elif match.group(1) not in by_id:
                errors.append(f"{tid}: enable_when references missing task {match.group(1)}")

    color: dict[str, int] = {}
    def visit(tid: str, stack: list[str]) -> None:
        color[tid] = 1
        for dep in graph.get(tid, []):
            if color.get(dep) == 1:
                errors.append(f"dependency cycle: {' -> '.join(stack + [dep])}")
            elif color.get(dep, 0) == 0:
                visit(dep, stack + [dep])
        color[tid] = 2
    for tid in graph:
        if color.get(tid, 0) == 0:
            visit(tid, [tid])

    def ancestors(tid: str) -> set[str]:
        seen: set[str] = set()
        todo = list(graph.get(tid, []))
        while todo:
            dep = todo.pop()
            if dep not in seen:
                seen.add(dep)
                todo.extend(graph.get(dep, []))
        return seen
    for tid, task in by_id.items():
        condition = task.get("enable_when")
        match = re.fullmatch(r"(P\d{3})\.outcome\s*==\s*([a-z_]+)", str(condition)) if condition is not None else None
        if match and match.group(1) in by_id and match.group(1) not in ancestors(tid):
            errors.append(f"{tid}: conditional task {match.group(1)} is not a prerequisite (transitively)")

    # Manifest task set and index links must agree with the files on disk.
    on_disk = {p.relative_to(root).as_posix() for p in root.rglob("P*.md")}
    if on_disk != valid_task_paths:
        for p in sorted(on_disk - valid_task_paths):
            errors.append(f"unlisted task file: {p}")
        for p in sorted(valid_task_paths - on_disk):
            errors.append(f"manifest task file absent from disk: {p}")
    if index_path.exists():
        index = index_path.read_text(encoding="utf-8")
        for task in tasks:
            path = task.get("path") if isinstance(task, dict) else None
            if path and f"]({path})" not in index:
                errors.append(f"INDEX.md: missing task link {path}")
    else:
        errors.append("missing INDEX.md")

    for md in sorted(root.rglob("*.md")):
        _check_local_links(md, root, errors)

    # Historical report hashes are immutable evidence. The old broad plan identity is
    # a planning snapshot marker; later real implementation edits merit a warning.
    workspace = root.parent.parent
    repository = Path(str(manifest.get("repository", "")))
    for item in manifest.get("historical_benchmarks", []):
        path_text, expected = item.get("path"), item.get("sha256")
        candidate = (repository / path_text) if repository.is_absolute() else (workspace / path_text)
        actual = _digest(candidate)
        if actual is None:
            errors.append(f"historical benchmark missing: {path_text}")
        elif actual != expected:
            errors.append(f"historical benchmark SHA-256 changed: {path_text}")
    legacy = manifest.get("legacy_plan", {})
    legacy_path = Path(str(legacy.get("path", "")))
    actual = _digest(legacy_path)
    expected = legacy.get("sha256_at_planning")
    if actual is None:
        errors.append(f"legacy plan missing: {legacy_path}")
    elif actual != expected:
        warnings.append(f"legacy plan SHA-256 differs from planning snapshot: {legacy_path}")

    counts: dict[str, int] = {}
    for task in tasks:
        if isinstance(task, dict):
            counts[task.get("status", "<missing>")] = counts.get(task.get("status", "<missing>"), 0) + 1
    return errors, warnings, counts


def _self_test() -> None:
    """Prove key corruptions are caught using a tiny temporary plan fixture."""
    with tempfile.TemporaryDirectory(prefix="check-plan-") as tmp:
        root = Path(tmp)
        (root / "g").mkdir()
        task = {
            "id": "P001", "key": "one", "title": "One task", "group": "g",
            "status": "todo", "outcome": "pending", "kind": "foundation",
            "dependencies": [], "owned_paths": ["src/one.py"],
            "input_output_contract": "one input creates one output.",
            "implementation_steps": ["First step.", "Second step.", "Third step."],
            "fixture_cases": ["Case one.", "Case two.", "Case three."],
            "measurement_selection": "scalar", "execution_mode": "decoded",
            "semantic_contracts": [], "enable_when": None,
            "implementation_model": {"model": "gpt-6.1-sol", "reasoning_effort": "low"},
            "test_commands": ["python3 -m unittest"], "before_edit_commands": [],
            "path": "g/P001-one.md", "evidence": [],
            "branch": "portable", "decision_rule": "integrity", "reviews_optional_tasks": [],
            "minimum_focused_cases": 3,
            "evidence_contract": "Record actual command and three cases.", "external_constraints": [],
        }
        body = """# P001 — One task\n\nStatus: **todo**\n\nOutcome: **pending**\n\n## Scope and ownership\n\n- `src/one.py`\n\n## Input/output contract\n\none input creates one output.\n\n## Implementation recipe\n\n1. First step.\n2. Second step.\n3. Third step.\n\n## Meaningful cases\n\n- Case one.\n- Case two.\n- Case three.\n\n## Verification\n\n```sh\npython3 -m unittest\n```\n"""
        (root / "g/P001-one.md").write_text(body, encoding="utf-8")
        (root / "INDEX.md").write_text("[task](g/P001-one.md)\n", encoding="utf-8")
        legacy_file = root / "legacy.json"
        legacy_file.write_text("{}", encoding="utf-8")
        manifest = {
            "schema_version": 1,
            "implementation_model": {"model": "gpt-6.1-sol", "reasoning_effort": "low"},
            "tasks": [task],
            "legacy_plan": {"path": str(legacy_file), "sha256_at_planning": _digest(legacy_file)},
            "historical_benchmarks": [],
        }
        def save() -> None:
            (root / "TASKS.json").write_text(json.dumps(manifest), encoding="utf-8")
        save()
        errors, _, _ = validate(root)
        assert not errors, errors

        task["status"], task["outcome"] = "deferred", "not_selected"
        (root / "g/P001-one.md").write_text(
            body.replace("Status: **todo**", "Status: **deferred**")
                .replace("Outcome: **pending**", "Outcome: **not_selected**"),
            encoding="utf-8",
        )
        save()
        errors, _, _ = validate(root)
        assert not errors, f"declared deferred/not_selected values should be accepted: {errors}"
        task["status"], task["outcome"] = "todo", "pending"
        (root / "g/P001-one.md").write_text(body, encoding="utf-8")
        save()

        def rejected(label: str) -> None:
            found, _, _ = validate(root)
            assert found, f"self-test mutation was not rejected: {label}"

        # Duplicate IDs and a cycle/missing dependency are separate graph failures.
        manifest["tasks"].append(dict(task))
        save(); rejected("duplicate ID")
        manifest["tasks"].pop()
        task["dependencies"] = ["P999"]
        save(); rejected("missing dependency")
        task["dependencies"] = ["P001"]
        save(); rejected("cycle")
        task["dependencies"] = []
        task["status"] = "finished-ish"
        save(); rejected("invalid status")
        task["status"] = "todo"
        task["implementation_steps"][0] = "Drifted step."
        save(); rejected("drifted recipe")
        task["implementation_steps"][0] = "First step."
        task["outcome"] = "maybe"
        save(); rejected("invalid outcome")
        task["outcome"] = "pending"
        task["path"] = "g/P001-absent.md"
        save(); rejected("missing task file")
        task["path"] = "g/P001-one.md"
        save()
        (root / "INDEX.md").write_text("[task](g/missing.md)\n", encoding="utf-8")
        errors, _, _ = validate(root)
        assert any("missing local link" in e for e in errors), errors
        (root / "INDEX.md").write_text("[task](g/P001-one.md)\n", encoding="utf-8")
        (root / "g/P001-one.md").write_text(body.replace("First step.", "Changed step."), encoding="utf-8")
        errors, _, _ = validate(root)
        assert any("missing or drifted" in e for e in errors), errors
        before_command = "python3 tools/check-before.py --capture"
        task["before_edit_commands"] = [before_command]
        before_body = body + "\n## Before editing\n\n```sh\n" + before_command + "\n```\n"
        (root / "g/P001-one.md").write_text(before_body, encoding="utf-8")
        save()
        errors, _, _ = validate(root)
        assert not errors, f"matching before-edit command should be accepted: {errors}"
        (root / "g/P001-one.md").write_text(before_body.replace(before_command, "python3 tools/other.py"), encoding="utf-8")
        errors, _, _ = validate(root)
        assert any("before-edit command missing or drifted" in e for e in errors), errors
    print("self-test: corruption checks passed; deferred/not_selected accepted")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="run corruption tests in a temporary fixture")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent, help="performance plan directory")
    args = parser.parse_args(argv)
    if args.self_test:
        _self_test()
        print("self-test: repository files were not modified")
        return 0
    errors, warnings, counts = validate(args.root.resolve())
    total = sum(counts.values())
    print(f"plan: {total} tasks; statuses: " + ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    for warning in warnings:
        print(f"WARNING: {warning}", file=sys.stderr)
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(f"checker: {len(errors)} error(s), {len(warnings)} warning(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
