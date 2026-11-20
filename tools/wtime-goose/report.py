#!/usr/bin/env python3
"""Markdown tables from bench.py reports: medians, and paired wtime/Wasmtime and wtime/V8 ratios."""

import json
import sys


def ms(ns):
    if ns is None:
        return "–"
    return f"{ns / 1e6:,.1f} ms" if ns < 1e10 else f"{ns / 1e9:,.1f} s"


def ratio(value):
    return "–" if value is None else (f"{value:,.0f}×" if value >= 10 else f"{value:.2f}×")


def main(paths):
    for path in paths:
        report = json.load(open(path))
        print(f"### {path}\n")
        print(f"{report['rounds']} rounds, CPU {report['host']['cpu']} ({report['host']['model']}). "
              "Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.\n")
        engines = [name for name in report["cases"][0]["summary"]]
        tiers = [name for name in engines if name != "wasmtime"]
        print("| Case | Wasmtime run | " + " | ".join(f"{t} run" for t in tiers) + " | "
              + " | ".join(f"{t} / Wasmtime run" for t in tiers) + " | outputs |")
        print("|---|" + "---:|" * (1 + 2 * len(tiers)) + "---|")
        for case in report["cases"]:
            s = case["summary"]
            row = [case["name"], ms(s["wasmtime"].get("start_function_ns"))]
            row += [ms(s[t].get("start_function_ns")) for t in tiers]
            row += [ratio(s[t]["paired_ratio_vs_wasmtime"].get("start_function_ns")) for t in tiers]
            row.append("match" if case["outputs_match"] else "MISMATCH")
            print("| " + " | ".join(row) + " |")
        print()
        print("| Case | Wasmtime prep | " + " | ".join(f"{t} prep" for t in tiers) + " | Wasmtime process | "
              + " | ".join(f"{t} process" for t in tiers) + " |")
        print("|---|" + "---:|" * (2 + 2 * len(tiers)))
        for case in report["cases"]:
            s = case["summary"]
            row = [case["name"], ms(s["wasmtime"].get("compile_instantiate_ns"))]
            row += [ms(s[t].get("compile_instantiate_ns")) for t in tiers]
            row += [ms(s["wasmtime"].get("wall_ns"))] + [ms(s[t].get("wall_ns")) for t in tiers]
            print("| " + " | ".join(row) + " |")
        print()
        if "v8" in engines:
            wtimes = [name for name in engines if name.startswith("wtime")]
            print("Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round "
                  "wtime/V8 pairs, below 1 when wtime is faster.\n")
            header = ["Case", "V8 run"] + [f"{t} run" for t in wtimes] + [f"{t} / V8 run" for t in wtimes]
            header += ["V8 prep"] + [f"{t} prep" for t in wtimes] + ["V8 process"] + [f"{t} process" for t in wtimes]
            header += [f"{t} / V8 process" for t in wtimes]
            print("| " + " | ".join(header) + " |")
            print("|---|" + "---:|" * (len(header) - 1))
            for case in report["cases"]:
                s = case["summary"]
                row = [case["name"], ms(s["v8"].get("start_function_ns"))]
                row += [ms(s[t].get("start_function_ns")) for t in wtimes]
                row += [ratio(s[t].get("paired_ratio_vs_v8", {}).get("start_function_ns")) for t in wtimes]
                row += [ms(s["v8"].get("compile_instantiate_ns"))] + [ms(s[t].get("compile_instantiate_ns")) for t in wtimes]
                row += [ms(s["v8"].get("wall_ns"))] + [ms(s[t].get("wall_ns")) for t in wtimes]
                row += [ratio(s[t].get("paired_ratio_vs_v8", {}).get("wall_ns")) for t in wtimes]
                print("| " + " | ".join(row) + " |")
            print()


if __name__ == "__main__":
    main(sys.argv[1:])
