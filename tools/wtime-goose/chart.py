#!/usr/bin/env python3
"""SVG bar charts from bench.py reports: how much faster wtime-aot is than Wasmtime and V8.

    chart.py results/X/examples.json results/X/compiler.json -o charts

writes charts/run.svg (time in the guest's start function) and
charts/process.svg (the whole process, against Wasmtime's warmed module
cache where the report has it). A bar is the median over rounds of the
competitor's time divided by wtime-aot's in the same round, on a log scale
centred on parity: right of 1x wtime-aot is faster, left of it slower.
"""

import argparse
import json
import math
import statistics
from pathlib import Path

# Validated categorical slots 1 and 2 of the reference palette, light and dark.
STYLE = """
.c { --surface:#fcfcfb; --ink:#0b0b0b; --ink2:#52514e; --muted:#898781; --grid:#e1e0d9; --axis:#c3c2b7;
     --s1:#2a78d6; --s2:#eb6834; font-family:system-ui,-apple-system,"Segoe UI",sans-serif; }
@media (prefers-color-scheme: dark) {
  .c { --surface:#1a1a19; --ink:#f0efec; --ink2:#c3c2b7; --muted:#898781; --grid:#2c2c2a; --axis:#383835;
       --s1:#3987e5; --s2:#d95926; }
}
.bg { fill:var(--surface); }
.t { fill:var(--ink); font-size:15px; font-weight:600; }
.st { fill:var(--ink2); font-size:12px; }
.lb { fill:var(--ink); font-size:12px; }
.grp { fill:var(--ink2); font-size:11px; font-weight:600; letter-spacing:.04em; }
.v { fill:var(--ink2); font-size:11px; font-variant-numeric:tabular-nums; }
.tick { fill:var(--muted); font-size:11px; font-variant-numeric:tabular-nums; }
.grid { stroke:var(--grid); stroke-width:1; }
.par { stroke:var(--axis); stroke-width:1.5; }
.s1 { fill:var(--s1); } .s2 { fill:var(--s2); }
"""

WIDTH, LEFT, RIGHT, BAR, GAP, ROW_GAP = 760, 150, 70, 10, 2, 12


def paired(case, other, metric):
    """Median over rounds of other/wtime-aot, or None."""
    a, b = case["samples"].get("wtime-aot") or [], case["samples"].get(other) or []
    ratios = [y[metric] / x[metric] for x, y in zip(a, b)
              if x.get(metric) and y.get(metric) and x.get("exit") == 0 and y.get("exit") == 0]
    return statistics.median(ratios) if ratios else None


def label(value):
    return f"{value:,.0f}×" if value >= 10 else f"{value:.2f}×"


def esc(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def bar(x0, x1, y, cls):
    """A bar from the parity line x0 to its data end x1, rounded only at the data end."""
    r = min(4, abs(x1 - x0) / 2, BAR / 2)
    if x1 >= x0:
        d = (f"M{x0:.1f},{y}H{x1 - r:.1f}Q{x1:.1f},{y} {x1:.1f},{y + r}V{y + BAR - r}"
             f"Q{x1:.1f},{y + BAR} {x1 - r:.1f},{y + BAR}H{x0:.1f}Z")
    else:
        d = (f"M{x0:.1f},{y}H{x1 + r:.1f}Q{x1:.1f},{y} {x1:.1f},{y + r}V{y + BAR - r}"
             f"Q{x1:.1f},{y + BAR} {x1 + r:.1f},{y + BAR}H{x0:.1f}Z")
    return f'<path class="{cls}" d="{d}"/>'


def chart(groups, metric, series, title, subtitle):
    """groups: [(group name, [case])]; series: [(engine, legend text)]."""
    values = [paired(c, engine, metric) for _, cases in groups for c in cases for engine, _ in series]
    values = [v for v in values if v]
    lo = min([0.5] + [v / 1.4 for v in values])
    hi = max([2.0] + [v * 1.6 for v in values])
    lo, hi = math.log10(lo), math.log10(hi)
    plot = WIDTH - LEFT - RIGHT

    def x(v):
        return LEFT + (math.log10(v) - lo) / (hi - lo) * plot

    row = len(series) * BAR + (len(series) - 1) * GAP + ROW_GAP
    top = 92
    height = top + sum(22 + row * len(cases) for _, cases in groups) + 34
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" class="c" viewBox="0 0 {WIDTH} {height}" '
           f'width="{WIDTH}" height="{height}" role="img" aria-labelledby="title desc">',
           f"<title id=\"title\">{esc(title)}</title>", f"<desc id=\"desc\">{esc(subtitle)}</desc>",
           f"<style>{STYLE}</style>", f'<rect class="bg" width="{WIDTH}" height="{height}" rx="8"/>',
           f'<text class="t" x="16" y="28">{esc(title)}</text>',
           f'<text class="st" x="16" y="47">{esc(subtitle)}</text>']
    lx = 16
    for k, (_, text) in enumerate(series):
        out.append(f'<rect class="s{k + 1}" x="{lx}" y="62" width="12" height="12" rx="3"/>')
        out.append(f'<text class="lb" x="{lx + 18}" y="72">{esc(text)}</text>')
        lx += 30 + 7 * len(text)
    bottom = height - 34
    ticks = [t * 10 ** e for e in range(-3, 5) for t in (1, 2, 5)]
    for t in ticks:
        if lo <= math.log10(t) <= hi:
            out.append(f'<line class="{"par" if t == 1 else "grid"}" x1="{x(t):.1f}" x2="{x(t):.1f}" '
                       f'y1="{top - 6}" y2="{bottom}"/>')
            text = "parity" if t == 1 else (f"{t:g}×")
            out.append(f'<text class="tick" x="{x(t):.1f}" y="{bottom + 16}" text-anchor="middle">{text}</text>')
    y = top
    for name, cases in groups:
        out.append(f'<text class="grp" x="16" y="{y + 10}">{esc(name.upper())}</text>')
        y += 22
        for case in cases:
            out.append(f'<text class="lb" x="{LEFT - 10}" y="{y + row / 2 - 2:.1f}" text-anchor="end">'
                       f'{esc(case["name"])}</text>')
            for k, (engine, text) in enumerate(series):
                v = paired(case, engine, metric)
                by = y + k * (BAR + GAP)
                if v is None:
                    continue
                out.append(f"<g><title>{esc(case['name'])} {esc(text)}: {label(v)}</title>")
                out.append(bar(x(1), x(v), by, f"s{k + 1}"))
                end = x(v)
                anchor, tx = ("start", end + 4) if v >= 1 else ("end", end - 4)
                out.append(f'<text class="v" x="{tx:.1f}" y="{by + BAR - 1}" text-anchor="{anchor}">{label(v)}</text></g>')
            y += row
    out.append("</svg>")
    return "\n".join(out) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("reports", type=Path, nargs="+")
    parser.add_argument("-o", "--output", type=Path, required=True)
    args = parser.parse_args()
    groups = []
    for path in args.reports:
        report = json.load(open(path))
        cases = [c for c in report["cases"] if "wtime-aot" in c["samples"]]
        if cases:
            groups.append((path.stem, cases))
    engines = {engine for _, cases in groups for c in cases for engine in c["samples"]}
    wasmtime = "wasmtime-cached" if "wasmtime-cached" in engines else "wasmtime"
    args.output.mkdir(parents=True, exist_ok=True)
    note = "competitor time ÷ wtime-aot time, median of paired rounds, log scale; right of parity is faster"
    (args.output / "run.svg").write_text(chart(
        groups, "start_function_ns", [("wasmtime", "vs Wasmtime"), ("v8", "vs V8 (Deno)")],
        "wtime-aot speed-up: running the program", note))
    (args.output / "process.svg").write_text(chart(
        groups, "wall_ns",
        [(wasmtime, "vs Wasmtime" + (", cached module" if wasmtime == "wasmtime-cached" else "")),
         ("v8", "vs V8 (Deno)")],
        "wtime-aot speed-up: whole process (start, load, run, exit)", note))


if __name__ == "__main__":
    main()
