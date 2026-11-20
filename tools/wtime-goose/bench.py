#!/usr/bin/env python3
"""Paired wtime vs Wasmtime runs of direct Goose Wasm modules (goose-to-wasm).

Each case is one module, its program arguments and stdin. Every round runs each
engine once in a fresh process, in an order that alternates by round, pinned to
one CPU. Both hosts write the same timing JSON boundaries:

  compile_instantiate_ns  preparation (wtime: decode, validate, lower;
                          Wasmtime: Cranelift compile, no cache) + instantiation
  start_function_ns       the program's `_start`
  (wasmtime-cached loads Cranelift output from a cache warmed before timing)
  wall_ns                 the whole process, measured here

Every run's stdout and exit code must match the case's reference (the first
Wasmtime run, and the expected file when one is given).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_once(argv: list[str], stdin: bytes, cwd: Path, cpu: str, timeout: float) -> dict:
    with tempfile.TemporaryDirectory() as scratch:
        timing = Path(scratch) / "timing.json"
        # The timing file goes after the executable, or where an engine's
        # command line puts @TIMING@.
        if "@TIMING@" in argv:
            engine_argv = [str(timing) if a == "@TIMING@" else a for a in argv]
        else:
            engine_argv = [argv[0], "--timing-json", str(timing), *argv[1:]]
        command = ["timeout", str(int(timeout)), "taskset", "-c", cpu, *engine_argv]
        with open(Path(scratch) / "out", "wb") as out, open(Path(scratch) / "err", "wb") as err:
            started = time.perf_counter_ns()
            process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=out, stderr=err, cwd=cwd)
            process.stdin.write(stdin)
            process.stdin.close()
            _, status, usage = os.wait4(process.pid, 0)
            wall = time.perf_counter_ns() - started
        engine = json.loads(timing.read_text()) if timing.exists() else {}
        stdout = (Path(scratch) / "out").read_bytes()
        stderr = (Path(scratch) / "err").read_bytes().decode(errors="replace")
    return {"wall_ns": wall, "exit": os.waitstatus_to_exitcode(status), "max_rss_kib": usage.ru_maxrss,
            "stdout": stdout, "stderr": stderr, **engine}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cases", type=Path, required=True, help="JSON list of cases")
    parser.add_argument("--wtime", type=Path, required=True)
    parser.add_argument("--wasmtime", type=Path, required=True)
    parser.add_argument("--wtime-before", type=Path, help="also run this earlier wtime-goose build (engine wtime-before-<tier>)")
    parser.add_argument("--tiers", default="decoded", help="comma-separated wtime tiers")
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--cpu", default="1")
    parser.add_argument("--timeout", type=float, default=7200)
    parser.add_argument("--only", help="comma-separated case names")
    parser.add_argument("--wasmtime-cache", type=Path, help="also run Wasmtime from a warmed compiled-module cache in this directory")
    parser.add_argument("--deno", type=Path, help="also run V8 through this Deno binary (engine v8), with --v8-host")
    parser.add_argument("--v8-host", type=Path, help="goose-to-wasm's scripts/run_wasm.mjs, the V8 host Deno runs")
    parser.add_argument("--aot-dir", type=Path,
                        help="also run programs compiled ahead of time by tools/wtime-aot (engine wtime-aot): "
                             "aot-<module stem> in this directory for each case")
    parser.add_argument("-o", "--output", type=Path, required=True)
    args = parser.parse_args()
    args.wtime, args.wasmtime = args.wtime.resolve(), args.wasmtime.resolve()
    if args.wtime_before:
        args.wtime_before = args.wtime_before.resolve()
    if bool(args.deno) != bool(args.v8_host):
        parser.error("--deno and --v8-host go together")
    if args.deno:
        args.deno, args.v8_host = args.deno.resolve(), args.v8_host.resolve()

    cases = json.loads(args.cases.read_text())
    if args.only:
        wanted = set(args.only.split(","))
        cases = [case for case in cases if case["name"] in wanted]
    tiers = args.tiers.split(",")
    report = {
        "schema": "wtime-goose-comparison-v1",
        "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {"machine": platform.machine(), "kernel": platform.release(), "cpus": os.cpu_count(), "cpu": args.cpu,
                 "model": next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                                if line.startswith("model name")), "")},
        "binaries": {"wtime": {"path": str(args.wtime), "sha256": sha256(args.wtime)},
                     "wasmtime": {"path": str(args.wasmtime), "sha256": sha256(args.wasmtime)},
                     **({"wtime_before": {"path": str(args.wtime_before), "sha256": sha256(args.wtime_before)}}
                        if args.wtime_before else {}),
                     **({"deno": {"path": str(args.deno), "sha256": sha256(args.deno),
                                  "version": subprocess.run([str(args.deno), "--version"], capture_output=True,
                                                            text=True).stdout.strip()},
                         "v8_host": {"path": str(args.v8_host), "sha256": sha256(args.v8_host)}}
                        if args.deno else {})},
        "rounds": args.rounds,
        "cases": [],
    }
    for case in cases:
        module = Path(case["module"]).resolve()
        cwd = Path(case.get("cwd", ".")).resolve()
        stdin = Path(case["stdin"]).read_bytes() if case.get("stdin") else b""
        program = ["--", *case["arguments"]] if "arguments" in case else []
        engines = {"wasmtime": [str(args.wasmtime), str(module), "--no-cache", *program]}
        if args.wasmtime_cache:
            args.wasmtime_cache.mkdir(parents=True, exist_ok=True)
            cache = args.wasmtime_cache.resolve() / f"{case['name']}.cwasm"
            engines["wasmtime-cached"] = [str(args.wasmtime), str(module), "--cache", str(cache), *program]
            run_once(engines["wasmtime-cached"], stdin, cwd, args.cpu, args.timeout)  # warm the cache
        if args.aot_dir:
            stem = "".join(c if c.isascii() and c.isalnum() else "-" for c in module.stem)
            engines["wtime-aot"] = [str(args.aot_dir.resolve() / f"aot-{stem}"), *program]
        if args.deno:
            # Deno's V8 with its default tiers (Liftoff, then TurboFan), all
            # permissions for the host's file access, the module's memory64
            # and a cold code cache, as for Wasmtime --no-cache.
            engines["v8"] = [str(args.deno), "run", "-A", "--no-config", str(args.v8_host),
                             str(module), "--timing-json", "@TIMING@", *program]
        for tier in tiers:
            engines[f"wtime-{tier}"] = [str(args.wtime), "--tier", tier, str(module), *program]
            if args.wtime_before:
                engines[f"wtime-before-{tier}"] = [str(args.wtime_before), "--tier", tier, str(module), *program]
        expected = Path(case["expected"]).read_bytes() if case.get("expected") else None
        rows = {name: [] for name in engines}
        failures = []
        reference = None
        names = list(engines)
        for round_index in range(args.rounds):
            order = names if round_index % 2 == 0 else names[::-1]
            for name in order:
                result = run_once(engines[name], stdin, cwd, args.cpu, args.timeout)
                outcome = (result["exit"], result["stdout"])
                if reference is None and name == "wasmtime":
                    reference = outcome
                ok = (expected is None or result["stdout"] == expected) and (reference is None or outcome == reference)
                if not ok:
                    failures.append({"engine": name, "round": round_index, "exit": result["exit"],
                                     "stderr": result["stderr"][-2000:], "stdout_sha256": hashlib.sha256(result["stdout"]).hexdigest()})
                rows[name].append({k: v for k, v in result.items() if k not in ("stdout", "stderr")})
                print(f"{case['name']:>16} r{round_index} {name:>26}: wall {result['wall_ns']/1e6:10.1f} ms  "
                      f"prep {result.get('compile_instantiate_ns', 0)/1e6:9.1f} ms  run {result.get('start_function_ns', 0)/1e6:10.1f} ms"
                      f"{'' if ok else '  MISMATCH exit=' + str(result['exit'])}", flush=True)
        summary = {}
        for name, samples in rows.items():
            summary[name] = {key: statistics.median(sample[key] for sample in samples)
                             for key in ("wall_ns", "compile_instantiate_ns", "start_function_ns", "max_rss_kib")
                             if all(key in s for s in samples)}
        for reference_name in ("wasmtime", "v8"):
            if reference_name not in rows:
                continue
            for name in names:
                if name == reference_name:
                    continue
                ratios = {}
                for key in ("wall_ns", "start_function_ns", "compile_instantiate_ns"):
                    pairs = [a[key] / b[key] for a, b in zip(rows[name], rows[reference_name]) if a.get(key) and b.get(key)]
                    if pairs:
                        ratios[key] = statistics.median(pairs)
                summary[name][f"paired_ratio_vs_{reference_name}"] = ratios
        report["cases"].append({
            "name": case["name"], "description": case.get("description", ""),
            "module": {"path": str(module), "sha256": sha256(module), "bytes": module.stat().st_size},
            "arguments": case.get("arguments"), "stdin": case.get("stdin"),
            "output_sha256": hashlib.sha256(reference[1]).hexdigest() if reference else None,
            "outputs_match": not failures, "failures": failures, "summary": summary, "samples": rows,
        })
        args.output.write_text(json.dumps(report, indent=1) + "\n")
    report["finished"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    return 0 if all(case["outputs_match"] for case in report["cases"]) else 1


if __name__ == "__main__":
    sys.exit(main())
