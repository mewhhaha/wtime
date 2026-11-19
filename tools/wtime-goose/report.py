#!/usr/bin/env python3
"""Markdown tables from bench.py reports: medians, and paired wtime/Wasmtime ratios."""

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


if __name__ == "__main__":
    main(sys.argv[1:])
