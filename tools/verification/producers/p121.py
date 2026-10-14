"""Freeze P121 predecessors or validate four instrumented Compact reports."""
from pathlib import Path
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools/wtime-bench"))
import baseline

HEX = re.compile(r"^[0-9a-f]{64}$")
PROFILE_SUITES = {
    ("mvp", "mvp"),
    ("all-phase3-core", "core3"),
    ("all-phase3-core", "phase3"),
    ("all-phase3-core", "interactions"),
}
MAX_USIZE = sys.maxsize
MAX_U64 = (1 << 64) - 1
MAX_PEAK_ARTIFACT_BYTES = 1024 * 1024


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def ref(path):
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_new(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def verify_p001_archive(archive):
    archive = Path(archive)
    manifest = baseline.verify_archive(archive)
    if not manifest.get("smoke", {}).get("correctness_checked"):
        raise ValueError("P001 frozen smoke record did not pass")
    if len(manifest.get("files", {})) != 85:
        raise ValueError("P001 archive file membership differs from its frozen record")
    return manifest


def freeze(output):
    archive = ROOT / "target/performance/P121-before"
    manifest = baseline.verify_archive(archive)
    core_sources = {
        record["source_path"]: record["sha256"]
        for record in manifest["files"].values()
        if record.get("source_path", "").startswith("crates/wtime-runtime/src/")
        and record.get("role") == "source"
    }
    if manifest.get("mode") != "compact" or len(core_sources) != 32:
        raise ValueError("P121-before is not the declared 32-file Compact predecessor")
    predecessor = ROOT / "target/performance/P121/predeclaration/attempt-0002/receipt.json"
    receipt = read(predecessor)
    if (receipt.get("status") != "passed" or not receipt.get("source_stable")
            or not receipt.get("identical_live_source_verified")
            or receipt.get("mode") != "compact"
            or receipt.get("runtime_source_map") != core_sources
            or receipt.get("baseline_manifest_sha256") != sha(archive / "manifest.json")):
        raise ValueError("P121 predeclaration does not bind the frozen predecessor")
    p001_archive = ROOT / "target/performance/baseline"
    p001 = verify_p001_archive(p001_archive)
    write_new(output, {
        "status": "passed",
        "task": "P121",
        "timing_evidence": False,
        "predecessor": ref(archive / "manifest.json"),
        "predeclaration": ref(predecessor),
        "runtime_source_map": core_sources,
        "runtime_source_file_count": len(core_sources),
        "p001_archive": ref(p001_archive / "manifest.json"),
        "p001_archived_file_count": len(p001["files"]),
        "claims": "integrity and conformance collection only; no timing or promotion claim",
    })


def digest_source_map(value, root=ROOT):
    if not isinstance(value, str) or not value:
        raise ValueError("source_hash is missing")
    entries = value.split(";")
    if len(entries) < 2 or sum(entry.startswith("corpus-lock:") for entry in entries) != 1:
        raise ValueError("source_hash lacks the corpus-lock identity")
    seen = set()
    for entry in entries:
        name, separator, digest = entry.rpartition(":")
        if not separator or not HEX.fullmatch(digest):
            raise ValueError("source_hash contains a malformed source digest")
        if name in seen:
            raise ValueError("source_hash contains duplicate source entries")
        seen.add(name)
        if name == "corpus-lock":
            source = Path(root) / "docs/architecture/corpus-lock.json"
        else:
            source = Path(name)
            if not source.is_absolute():
                source = Path(root) / source
            source = source.resolve(strict=True)
            if not source.is_relative_to(Path(root).resolve()):
                raise ValueError("source_hash points outside the repository")
        if sha(source) != digest:
            raise ValueError(f"source_hash does not match current input {name}")
    if sum(not entry.startswith("corpus-lock:") for entry in entries) == 0:
        raise ValueError("source_hash has no selected corpus files")
    return dict(entry.rpartition(":")[::2] for entry in entries)


def expected_implementation_sources(root=ROOT):
    root = Path(root).resolve()
    sources = {}
    for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
        directory = root / "crates" / crate / "src"
        if not directory.is_dir():
            raise ValueError(f"implementation source map lacks {crate}")
        crate_sources = []
        for path in directory.rglob("*.rs"):
            resolved = path.resolve(strict=True)
            if not resolved.is_file() or not resolved.is_relative_to(root):
                raise ValueError("implementation source path escapes repository root")
            relative = resolved.relative_to(root).as_posix()
            sources[relative] = sha(resolved)
            crate_sources.append(relative)
        if not crate_sources:
            raise ValueError(f"implementation source map lacks {crate}")
    return dict(sorted(sources.items()))


def validate_locked_selection(source_map, suite, root):
    corpus = read(Path(root) / "docs/architecture/corpus-lock.json")
    locked_paths = corpus.get("suites", {}).get(suite, [])
    if not locked_paths:
        raise ValueError(f"suite is absent or empty in locked corpus: {suite}")
    root = Path(root).resolve()
    expected = {
        (root / ".cache" / relative).resolve(strict=True).relative_to(root).as_posix()
        for relative in locked_paths
    }
    actual = {
        (Path(name) if Path(name).is_absolute() else root / name)
        .resolve(strict=True).relative_to(root).as_posix()
        for name in set(source_map) - {"corpus-lock"}
    }
    if actual != expected:
        raise ValueError(f"source_hash membership differs from locked selection: {suite}")
    return corpus


def validate_report(path, binary, binary_hash, root=ROOT):
    report = read(path)
    if report.get("instrumented") is not True:
        raise ValueError(f"report is not instrumentation-enabled: {path}")
    if report.get("execution_mode") != "compact":
        raise ValueError(f"report has foreign execution mode: {path}")
    if not isinstance(report.get("profile"), str) or not isinstance(report.get("suite"), str):
        raise ValueError(f"report lacks an actual supported profile/suite: {path}")
    identity = (report["profile"], report["suite"])
    if identity not in PROFILE_SUITES:
        raise ValueError(f"report uses a foreign profile/suite command: {path}")
    for field in ("binary_sha256",):
        if report.get(field) != binary_hash:
            raise ValueError(f"report does not bind the supplied current binary ({field}): {path}")
    source_map = digest_source_map(report.get("source_hash"), root)
    validate_locked_selection(source_map, report["suite"], root)
    implementations = expected_implementation_sources(root)
    if report.get("implementation_sources") != implementations:
        raise ValueError(f"report binary was built from stale implementation sources: {path}")
    for field in ("selected", "executed"):
        if type(report.get(field)) is not int or not 0 < report[field] <= MAX_USIZE:
            raise ValueError(f"report has zero or malformed {field}: {path}")
    if report["selected"] != report["executed"]:
        raise ValueError(f"report selected/executed counts disagree: {path}")
    workers = report.get("worker_stores_observed")
    if type(workers) is not int or not 0 <= workers <= MAX_USIZE:
        raise ValueError(f"report has malformed worker Store count: {path}")
    for field in ("failed", "skipped", "unsupported", "text_tool_errors"):
        if type(report.get(field)) is not int or report[field] != 0:
            raise ValueError(f"report hides failed/skipped/unsupported outcomes ({field}): {path}")
    counts = report.get("execution_counts")
    if not isinstance(counts, dict):
        raise ValueError(f"report lacks execution counts: {path}")
    total = counts.get("total_dispatched_source_events")
    optimized = counts.get("optimized_compact_source_events")
    fallback = counts.get("decoded_fallback_source_events")
    if any(type(v) is not int or not 0 <= v <= MAX_U64 for v in (total, optimized, fallback)):
        raise ValueError(f"report has malformed execution counts: {path}")
    if total <= 0 or optimized <= 0 or fallback != total - optimized:
        raise ValueError(f"report has zero Compact work or inconsistent fallback counts: {path}")
    prep = report.get("preparation_summary")
    if not isinstance(prep, dict):
        raise ValueError(f"report lacks preparation summary: {path}")
    names = ("compile_count", "published_artifacts", "accepted", "declined",
             "attempted_work_known_sum", "unpublished_artifacts",
             "attempted_work_unknown_artifacts", "artifact_retained_budget_bytes",
             "peak_artifact_retained_budget_bytes")
    if any(type(prep.get(name)) is not int or not 0 <= prep[name] <= MAX_USIZE for name in names):
        raise ValueError(f"report has malformed preparation counts: {path}")
    if (prep["compile_count"] != prep["published_artifacts"] + prep["unpublished_artifacts"]
            or prep["published_artifacts"] <= 0
            or prep["accepted"] + prep["declined"] <= 0
            or prep["attempted_work_known_sum"] <= 0
            or prep["artifact_retained_budget_bytes"] <= 0
            or prep["attempted_work_unknown_artifacts"] != prep["unpublished_artifacts"]
            or prep["peak_artifact_retained_budget_bytes"] > MAX_PEAK_ARTIFACT_BYTES):
        raise ValueError(f"report has zero or inconsistent preparation work: {path}")
    if report.get("failed") or not isinstance(report.get("errors"), list) or report["errors"]:
        raise ValueError(f"report contains hidden failure details: {path}")
    return {
        "report": ref(path), "profile": report["profile"], "suite": report["suite"],
        "selected": report["selected"], "executed": report["executed"],
        "worker_stores_observed": workers,
        "execution_counts": counts, "preparation_summary": prep,
        "source_hash_sha256": hashlib.sha256(report["source_hash"].encode()).hexdigest(),
        "corpus_lock_sha256": source_map["corpus-lock"],
        "selected_corpus_file_count": len(source_map) - 1,
        "implementation_sources_sha256": hashlib.sha256(json.dumps(
            implementations, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        "binary": ref(binary),
    }


def validate(reports, binary, output, root=ROOT):
    if len(reports) != 4:
        raise ValueError("P121 validation requires exactly four full CLI reports")
    binary = Path(binary).resolve(strict=True)
    binary_hash = sha(binary)
    values = [validate_report(path, binary, binary_hash, root) for path in reports]
    identities = [(value["profile"], value["suite"]) for value in values]
    if set(identities) != PROFILE_SUITES or len(set(identities)) != 4:
        raise ValueError("the four reports must cover the locked suite/profile command set exactly once")
    def checked_total(name, limit):
        total = 0
        for value in values:
            number = value["execution_counts"].get(name) if name in value["execution_counts"] else value[name]
            total += number
            if total > limit:
                raise ValueError(f"P121 checked aggregate overflow: {name}")
        return total
    preparation_fields = (
        "compile_count", "published_artifacts", "accepted", "declined",
        "attempted_work_known_sum", "unpublished_artifacts",
        "attempted_work_unknown_artifacts", "artifact_retained_budget_bytes",
    )
    preparation_totals = {}
    for name in preparation_fields:
        total = 0
        for value in values:
            total += value["preparation_summary"][name]
            if total > MAX_USIZE:
                raise ValueError(f"P121 checked aggregate overflow: preparation {name}")
        preparation_totals[name] = total
    preparation_totals["peak_artifact_retained_budget_bytes"] = max(
        value["preparation_summary"]["peak_artifact_retained_budget_bytes"]
        for value in values
    )
    totals = {
        "selected": checked_total("selected", MAX_USIZE),
        "executed": checked_total("executed", MAX_USIZE),
        "worker_stores_observed": checked_total("worker_stores_observed", MAX_USIZE),
        "total_dispatched_source_events": checked_total("total_dispatched_source_events", MAX_U64),
        "optimized_compact_source_events": checked_total("optimized_compact_source_events", MAX_U64),
        "decoded_fallback_source_events": checked_total("decoded_fallback_source_events", MAX_U64),
        "preparation": preparation_totals,
    }
    write_new(output, {
        "status": "passed", "task": "P121", "timing_evidence": False,
        "decision_scope": "full conformance and diagnostic coverage only; no timing claim",
        "current_binary": ref(binary), "reports": values, "totals": totals,
    })


def run_suite(binary, suite, profile, output):
    if (profile, suite) not in PROFILE_SUITES:
        raise ValueError("foreign suite/profile command")
    binary = Path(binary).resolve(strict=True)
    initial = sha(binary)
    command = [str(binary), "run", "--mode", "compact", "--profile", profile,
               "--suite", suite, "--filter", "all", "--report", str(output)]
    write_new(output.parent / (suite + "-command.json"), {
        "argv": command, "cwd": str(ROOT), "binary": ref(binary)})
    retained = output.parent / "wtime-conformance"
    if retained.exists():
        if sha(retained) != initial: raise ValueError("retained binary differs")
    else: shutil.copy2(binary, retained)
    with (output.parent / (suite + "-cli.stdout")).open("xb") as stdout, (output.parent / (suite + "-cli.stderr")).open("xb") as stderr:
        result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    write_new(output.parent / (suite + "-result.json"), {
        "exit_code": result.returncode, "binary_stable": initial == sha(binary)})
    if result.returncode or initial != sha(binary): raise ValueError("suite command failed or binary changed")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "validate", "run-suite"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", action="append", type=Path, default=[])
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--suite")
    parser.add_argument("--profile")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    if not args.execute:
        print("inert: pass --execute")
        return
    args.output.parent.mkdir(parents=True, exist_ok=True)
    helper = args.output.parent / f"{args.action}-{args.suite or 'all'}-helper.executed.py"
    with helper.open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    try:
        if args.action == "freeze":
            freeze(args.output)
        elif args.action == "run-suite":
            run_suite(args.binary, args.suite, args.profile, args.output)
        else:
            if args.binary is None:
                parser.error("validate requires --binary PATH")
            validate(args.report, args.binary, args.output)
    except (OSError, ValueError, KeyError) as exc:
        raise SystemExit(f"P121 {args.action} failed: {exc}") from exc


if __name__ == "__main__":
    main()
