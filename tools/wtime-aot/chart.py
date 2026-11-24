#!/usr/bin/env python3
"""Render clean light/dark SVG charts for wtime-aot from bench.py JSON.

    python3 -B tools/wtime-aot/chart.py tools/wtime-aot/charts/results.json -o tools/wtime-aot/charts

Writes standalone light/dark SVG images:
  - summary.svg         : Headline speedup / size-reduction vs Wasmtime (log parity chart)
  - runtime-kernels.svg : Representative Wasm kernels runtime (us, log scale)
  - compile-cold.svg    : Cold multi-core compilation time (ms/s, log scale)
  - compile-hot.svg     : Incremental 1-fn rebuild, warm load, & warm cache-hit (ms/s, log scale)
  - runtime-gdev.svg    : (optional, only when `runtime_gdev` is present in JSON)
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

STYLE = """
.c { --surface:#fcfcfb; --border:#e6e4dc; --ink:#111110; --ink2:#575652; --muted:#898781; --grid:#e6e4dc; --axis:#c4c2b8;
     --x64:#2563eb; --c:#ea580c; --rust:#7c3aed; --wt:#16a34a; --v8:#0d9488;
     font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif; }
@media (prefers-color-scheme: dark) {
  .c { --surface:#161615; --border:#2b2b28; --ink:#f2f1ed; --ink2:#b8b6ad; --muted:#85837c; --grid:#292926; --axis:#3f3f3a;
       --x64:#3b82f6; --c:#f97316; --rust:#a78bfa; --wt:#4ade80; --v8:#2dd4bf; }
}
.bg { fill:var(--surface); stroke:var(--border); stroke-width:1; }
.t { fill:var(--ink); font-size:15px; font-weight:600; letter-spacing:-0.01em; }
.st { fill:var(--ink2); font-size:12px; }
.lb { fill:var(--ink); font-size:12px; font-weight:500; }
.lg { fill:var(--ink2); font-size:11.5px; }
.grp { fill:var(--ink2); font-size:10.5px; font-weight:600; letter-spacing:.06em; }
.v { fill:var(--ink2); font-size:11px; font-variant-numeric:tabular-nums; }
.v-hi { fill:var(--ink); font-size:11px; font-weight:600; font-variant-numeric:tabular-nums; }
.tick { fill:var(--muted); font-size:11px; font-variant-numeric:tabular-nums; }
.grid { stroke:var(--grid); stroke-width:1; }
.ax { stroke:var(--axis); stroke-width:1.5; }
.x64 { fill:var(--x64); }
.bc { fill:var(--c); }
.rust { fill:var(--rust); }
.wt { fill:var(--wt); }
.v8 { fill:var(--v8); }
"""

CLASS_FOR = {
    "x64": "x64",
    "c": "bc",
    "rust": "rust",
    "wasmtime": "wt",
    "v8": "v8",
}

WIDTH = 820
BAR = 11
GAP = 3.5
ROW_GAP = 16


def esc(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def fmt_ms(v: float) -> str:
    if v >= 10_000.0:
        return f"{v / 1000.0:.1f} s"
    if v >= 1_000.0:
        return f"{v / 1000.0:.2f} s"
    if v >= 100.0:
        return f"{v:.0f} ms"
    if v >= 10.0:
        return f"{v:.1f} ms"
    if v >= 1.0:
        return f"{v:.2f} ms"
    return f"{v:.3f} ms"


def fmt_us(v: float) -> str:
    if v >= 1000.0:
        return f"{v / 1000.0:.2f} ms"
    if v >= 100.0:
        return f"{v:.0f} µs"
    if v >= 10.0:
        return f"{v:.1f} µs"
    return f"{v:.2f} µs"


def fmt_tick_ms(t: float) -> str:
    if t >= 1000.0:
        return f"{t / 1000.0:g} s"
    return f"{t:g} ms"


def fmt_tick_us(t: float) -> str:
    if t >= 1000.0:
        return f"{t / 1000.0:g} ms"
    return f"{t:g} µs"


def hbar(x0: float, x1: float, y: float, cls: str) -> str:
    x1 = max(x1, x0 + 2.5)
    r = min(3.5, (x1 - x0) / 2.0, BAR / 2.0)
    d = (
        f"M{x0:.1f},{y:.1f}H{x1 - r:.1f}"
        f"Q{x1:.1f},{y:.1f} {x1:.1f},{y + r:.1f}"
        f"V{y + BAR - r:.1f}"
        f"Q{x1:.1f},{y + BAR:.1f} {x1 - r:.1f},{y + BAR:.1f}"
        f"H{x0:.1f}Z"
    )
    return f'<path class="{cls}" d="{d}"/>'


def render_bar_chart(
    *,
    title: str,
    subtitle: str,
    series: list[tuple[str, str]],
    groups: list[tuple[str, list[tuple[str, dict[str, float]]]]],
    scale: str,
    fmt_val,
    fmt_tick,
    right_pad: int = 104,
    note_fn=None,
) -> str:
    """Grouped horizontal bar chart (linear or log10, lower is better)."""
    all_vals = [
        v
        for _, rows in groups
        for _, vals in rows
        for k, _ in series
        if (v := vals.get(k)) is not None and v > 0
    ]
    assert all_vals, f"no positive values for {title}"

    longest_label = max(len(lbl) for _, rows in groups for lbl, _ in rows)
    left = max(136, 30 + int(6.8 * longest_label))
    plot = WIDTH - left - right_pad

    if scale == "linear":
        hi_raw = max(all_vals) * 1.15
        mag = 10 ** math.floor(math.log10(hi_raw))
        step = mag
        for m in (0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0):
            cand = mag * m
            if hi_raw / cand <= 6:
                step = cand
                break
        hi = math.ceil(hi_raw / step) * step
        ticks = []
        t = 0.0
        while t <= hi + step * 1e-6:
            ticks.append(round(t, 6))
            t += step

        def x_of(v: float) -> float:
            return left + (v / hi) * plot

        x_zero = left
    elif scale == "log":
        lo_val = min(all_vals)
        hi_val = max(all_vals)
        lo_exp = math.floor(math.log10(lo_val / 1.4))
        hi_exp = math.ceil(math.log10(hi_val * 1.45))
        lo_log = math.log10(max(10 ** lo_exp, lo_val / 1.9))
        hi_log = math.log10(hi_val * 1.65)
        ticks = []
        for e in range(lo_exp - 1, hi_exp + 2):
            for m in (1, 2, 5):
                cand = m * (10 ** e)
                if lo_log - 1e-9 <= math.log10(cand) <= hi_log + 1e-9:
                    ticks.append(cand)
        if len(ticks) > 9:
            ticks = [t for t in ticks if abs(math.log10(t) - round(math.log10(t))) < 1e-9]

        def x_of(v: float) -> float:
            return left + (math.log10(v) - lo_log) / (hi_log - lo_log) * plot

        x_zero = left
    else:
        raise ValueError(scale)

    top = 100
    group_heights = []
    for gname, rows in groups:
        gh = 24 if gname else 4
        for _, vals in rows:
            present = sum(1 for k, _ in series if vals.get(k) is not None)
            gh += present * BAR + max(0, present - 1) * GAP + ROW_GAP
        group_heights.append(gh)
    bottom = top + sum(group_heights)
    height = int(math.ceil(bottom + 38))

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" class="c" viewBox="0 0 {WIDTH} {height}" '
        f'width="{WIDTH}" height="{height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{esc(title)}</title>',
        f'<desc id="desc">{esc(subtitle)}</desc>',
        f"<style>{STYLE}</style>",
        f'<rect class="bg" x="0.5" y="0.5" width="{WIDTH - 1}" height="{height - 1}" rx="10"/>',
        f'<text class="t" x="20" y="30">{esc(title)}</text>',
        f'<text class="st" x="20" y="49">{esc(subtitle)}</text>',
    ]

    # Legend (single crisp row)
    lx = 20
    for key, text in series:
        cls = CLASS_FOR[key]
        out.append(f'<rect class="{cls}" x="{lx}" y="64" width="11" height="11" rx="3"/>')
        out.append(f'<text class="lg" x="{lx + 16}" y="73.5">{esc(text)}</text>')
        lx += 30 + int(6.5 * len(text))

    # Gridlines & axis ticks
    for t in ticks:
        tx = x_of(t)
        is_axis = scale == "linear" and t == 0
        out.append(
            f'<line class="{"ax" if is_axis else "grid"}" x1="{tx:.1f}" x2="{tx:.1f}" '
            f'y1="{top - 6}" y2="{bottom:.1f}"/>'
        )
        out.append(
            f'<text class="tick" x="{tx:.1f}" y="{bottom + 18:.1f}" text-anchor="middle">'
            f"{esc(fmt_tick(t))}</text>"
        )
    if scale == "log":
        out.append(
            f'<line class="ax" x1="{left:.1f}" x2="{left:.1f}" y1="{top - 6}" y2="{bottom:.1f}"/>'
        )

    # Groups & rows
    y = float(top)
    for gname, rows in groups:
        if gname:
            out.append(f'<text class="grp" x="20" y="{y + 11:.1f}">{esc(gname.upper())}</text>')
            y += 24
        else:
            y += 4
        for rlabel, vals in rows:
            active = [(k, text, vals[k]) for k, text in series if vals.get(k) is not None]
            best_val = min(v for _, _, v in active)
            row_h = len(active) * BAR + max(0, len(active) - 1) * GAP
            out.append(
                f'<text class="lb" x="{left - 12}" y="{y + row_h / 2.0 + 4.0:.1f}" text-anchor="end">'
                f"{esc(rlabel)}</text>"
            )
            for idx, (k, text, v) in enumerate(active):
                by = y + idx * (BAR + GAP)
                end = x_of(v)
                val_txt = fmt_val(v)
                if note_fn is not None:
                    extra = note_fn(rlabel, k, v, vals)
                    if extra:
                        val_txt = f"{val_txt} {extra}"
                vcls = "v-hi" if abs(v - best_val) <= best_val * 0.01 else "v"
                out.append(f"<g><title>{esc(rlabel)} — {esc(text)}: {esc(val_txt)}</title>")
                out.append(hbar(x_zero, end, by, CLASS_FOR[k]))
                out.append(
                    f'<text class="{vcls}" x="{max(end, x_zero + 2.5) + 7:.1f}" y="{by + BAR - 1.5:.1f}" '
                    f'text-anchor="start">{esc(val_txt)}</text></g>'
                )
            y += row_h + ROW_GAP

    out.append("</svg>")
    return "\n".join(out) + "\n"


def hbar_parity(x0: float, x1: float, y: float, cls: str) -> str:
    """Horizontal bar extending from parity line x0 to x1 (rounded at the data end)."""
    if abs(x1 - x0) < 2.0:
        x1 = x0 + 2.0 if x1 >= x0 else x0 - 2.0
    r = min(3.5, abs(x1 - x0) / 2.0, BAR / 2.0)
    if x1 >= x0:
        d = (
            f"M{x0:.1f},{y:.1f}H{x1 - r:.1f}"
            f"Q{x1:.1f},{y:.1f} {x1:.1f},{y + r:.1f}"
            f"V{y + BAR - r:.1f}"
            f"Q{x1:.1f},{y + BAR:.1f} {x1 - r:.1f},{y + BAR:.1f}"
            f"H{x0:.1f}Z"
        )
    else:
        d = (
            f"M{x0:.1f},{y:.1f}H{x1 + r:.1f}"
            f"Q{x1:.1f},{y:.1f} {x1:.1f},{y + r:.1f}"
            f"V{y + BAR - r:.1f}"
            f"Q{x1:.1f},{y + BAR:.1f} {x1 + r:.1f},{y + BAR:.1f}"
            f"H{x0:.1f}Z"
        )
    return f'<path class="{cls}" d="{d}"/>'


def fmt_ratio(v: float) -> str:
    if v >= 10.0:
        return f"{v:.0f}×"
    return f"{v:.2f}×"


def render_parity_chart(
    *,
    title: str,
    subtitle: str,
    series: list[tuple[str, str]],
    groups: list[tuple[str, list[tuple[str, dict[str, tuple[float, str]]]]]],
    right_pad: int = 168,
) -> str:
    """Parity-anchored log10 speedup chart (>1× right of parity is faster/smaller)."""
    all_vals = [
        ratio
        for _, rows in groups
        for _, vals in rows
        for k, _ in series
        if (item := vals.get(k)) is not None and (ratio := item[0]) > 0
    ]
    assert all_vals, f"no positive ratios for {title}"

    longest_label = max(len(lbl) for _, rows in groups for lbl, _ in rows)
    left = max(148, 30 + int(6.8 * longest_label))
    plot = WIDTH - left - right_pad

    lo_val = min(0.75, min(all_vals) / 1.25)
    hi_val = max(2.0, max(all_vals) * 1.45)
    lo_log = math.log10(lo_val)
    hi_log = math.log10(hi_val)

    ticks = [t for t in (0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0) if lo_log - 1e-9 <= math.log10(t) <= hi_log + 1e-9]

    def x_of(v: float) -> float:
        return left + (math.log10(v) - lo_log) / (hi_log - lo_log) * plot

    x_par = x_of(1.0)
    top = 100
    group_heights = []
    for gname, rows in groups:
        gh = 24 if gname else 4
        for _, vals in rows:
            present = sum(1 for k, _ in series if vals.get(k) is not None)
            gh += present * BAR + max(0, present - 1) * GAP + ROW_GAP
        group_heights.append(gh)
    bottom = top + sum(group_heights)
    height = int(math.ceil(bottom + 38))

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" class="c" viewBox="0 0 {WIDTH} {height}" '
        f'width="{WIDTH}" height="{height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{esc(title)}</title>',
        f'<desc id="desc">{esc(subtitle)}</desc>',
        f"<style>{STYLE}</style>",
        f'<rect class="bg" x="0.5" y="0.5" width="{WIDTH - 1}" height="{height - 1}" rx="10"/>',
        f'<text class="t" x="20" y="30">{esc(title)}</text>',
        f'<text class="st" x="20" y="49">{esc(subtitle)}</text>',
    ]

    lx = 20
    for key, text in series:
        cls = CLASS_FOR[key]
        out.append(f'<rect class="{cls}" x="{lx}" y="64" width="11" height="11" rx="3"/>')
        out.append(f'<text class="lg" x="{lx + 16}" y="73.5">{esc(text)}</text>')
        lx += 30 + int(6.5 * len(text))

    for t in ticks:
        tx = x_of(t)
        is_par = abs(t - 1.0) < 1e-9
        out.append(
            f'<line class="{"ax" if is_par else "grid"}" x1="{tx:.1f}" x2="{tx:.1f}" '
            f'y1="{top - 6}" y2="{bottom:.1f}"/>'
        )
        t_lbl = "1× (Wasmtime)" if is_par else f"{t:g}×"
        out.append(
            f'<text class="tick" x="{tx:.1f}" y="{bottom + 18:.1f}" text-anchor="middle">'
            f"{esc(t_lbl)}</text>"
        )

    y = float(top)
    for gname, rows in groups:
        if gname:
            out.append(f'<text class="grp" x="20" y="{y + 11:.1f}">{esc(gname.upper())}</text>')
            y += 24
        else:
            y += 4
        for rlabel, vals in rows:
            active = [(k, text, vals[k][0], vals[k][1]) for k, text in series if vals.get(k) is not None]
            best_val = max(r for _, _, r, _ in active)
            row_h = len(active) * BAR + max(0, len(active) - 1) * GAP
            out.append(
                f'<text class="lb" x="{left - 12}" y="{y + row_h / 2.0 + 4.0:.1f}" text-anchor="end">'
                f"{esc(rlabel)}</text>"
            )
            for idx, (k, text, ratio, detail) in enumerate(active):
                by = y + idx * (BAR + GAP)
                end = x_of(ratio)
                lbl = f"{fmt_ratio(ratio)}  {detail}" if detail else fmt_ratio(ratio)
                vcls = "v-hi" if ratio >= 1.05 and abs(ratio - best_val) <= best_val * 0.01 else "v"
                out.append(f"<g><title>{esc(rlabel)} — {esc(text)}: {esc(lbl)}</title>")
                out.append(hbar_parity(x_par, end, by, CLASS_FOR[k]))
                tx = max(end, x_par) + 7.0
                out.append(
                    f'<text class="{vcls}" x="{tx:.1f}" y="{by + BAR - 1.5:.1f}" '
                    f'text-anchor="start">{esc(lbl)}</text></g>'
                )
            y += row_h + ROW_GAP

    out.append("</svg>")
    return "\n".join(out) + "\n"


def build_all(data: dict, out_dir: Path) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    host = data["host"]
    cpu_short = host["cpu"].replace(" 8-Core Processor", "").replace(" Processor", "")
    host_tag = f"{cpu_short}, {host['logical_cpus']}T"
    wt_ver = host.get("wasmtime")
    if isinstance(wt_ver, str) and wt_ver.lower().startswith("wasmtime "):
        wt_ver = wt_ver.split(None, 1)[1].strip()
    wt_label = f"Wasmtime {wt_ver}" if wt_ver else "Wasmtime"
    rendered = 0

    large_meta = data.get("large_module") or data.get("runtime_gdev")
    large_mib = f"{large_meta['wasm_bytes'] / (1024 * 1024):.2f} MiB" if large_meta else None
    large_fns = large_meta["functions"] if large_meta else None

    # Optional: real Blot game runtime chart when present
    if "runtime_gdev" in data:
        gd = data["runtime_gdev"]
        gdev_mib = f"{gd['wasm_bytes'] / (1024 * 1024):.2f} MiB"
        m = gd["metrics"]
        gdev_rows = [
            ("frame median", {k: m[k]["guest_median_ms"] for k in ("x64", "c", "wasmtime")}),
            ("frame p95", {k: m[k]["guest_p95_ms"] for k in ("x64", "c", "wasmtime")}),
            ("snapshot", {k: m[k]["snapshot_median_ms"] for k in ("x64", "c", "wasmtime")}),
        ]

        def gdev_note(_row: str, k: str, v: float, vals: dict[str, float]) -> str:
            if k == "x64" and vals.get("wasmtime"):
                return f"({vals['wasmtime'] / v:.2f}× vs WT)"
            return ""

        (out_dir / "runtime-gdev.svg").write_text(
            render_bar_chart(
                title=f"Blot game runtime: gdev ({gdev_mib}, {gd['functions']:,} functions)",
                subtitle=f"median ms per frame / snapshot over {gd['rounds']}×{gd['frames']} frames (CPU 0, lower is better; {cpu_short})",
                series=[
                    ("x64", "wtime-aot x64"),
                    ("c", "wtime-aot c (-O2)"),
                    ("wasmtime", wt_label),
                ],
                groups=[("", gdev_rows)],
                scale="linear",
                fmt_val=fmt_ms,
                fmt_tick=fmt_tick_ms,
                right_pad=140,
                note_fn=gdev_note,
            )
        )
        rendered += 1

    # 1. Representative Wasm kernels runtime (all 5 engines when Wasmtime is present)
    kr = data["runtime_kernels"]
    kernel_rows = [(c["name"], c["median_us"]) for c in kr["cases"]]
    has_wt_kernels = any("wasmtime" in c["median_us"] for c in kr["cases"])
    kernel_series = [
        ("x64", "wtime-aot x64"),
        ("c", "wtime-aot c (-O2)"),
        ("rust", "wtime-aot rust (-O3)"),
    ]
    if has_wt_kernels:
        kernel_series.append(("wasmtime", wt_label))
    kernel_series.append(("v8", f"V8 (Node {host['node']})"))

    def kernel_note(_row: str, k: str, v: float, vals: dict[str, float]) -> str:
        wt = vals.get("wasmtime")
        if not wt:
            return ""
        if k in ("x64", "rust") and wt / v >= 1.15:
            return f"({wt / v:.1f}× vs WT)"
        return ""

    (out_dir / "runtime-kernels.svg").write_text(
        render_bar_chart(
            title="Execution time: Wasm kernels (wtime-aot vs Wasmtime & V8)",
            subtitle=f"median µs per call, {kr['rounds']} paired rounds on CPU 0, exact parity (log scale, lower is better; {cpu_short})",
            series=kernel_series,
            groups=[("", kernel_rows)],
            scale="log",
            fmt_val=fmt_us,
            fmt_tick=fmt_tick_us,
            right_pad=126 if has_wt_kernels else 88,
            note_fn=kernel_note,
        )
    )
    rendered += 1

    # 2. Cold compilation time
    cc = data["compile_cold"]
    large_cold_vals = cc.get("large_ms") or cc.get("gdev_ms")
    cold_groups = []
    if large_cold_vals is not None:
        tag = f"5,000-fn application ({large_mib}, {large_fns:,} functions)" if large_mib and large_fns else "large Wasm application"
        cold_groups.append((tag, [("5k-fn app", large_cold_vals)]))
    cold_groups.append(("self-contained kernels (0.3–1 KiB)", [(name, vals) for name, vals in cc["kernels_ms"].items()]))

    def cold_note(row: str, k: str, v: float, vals: dict[str, float]) -> str:
        if row == "5k-fn app" and k == "x64":
            parts = []
            if vals.get("wasmtime"):
                parts.append(f"{vals['wasmtime'] / v:.1f}× vs WT")
            if vals.get("c"):
                parts.append(f"{vals['c'] / v:.0f}× vs c")
            return f"({', '.join(parts)})" if parts else ""
        return ""

    has_wt_cold = (large_cold_vals is not None and "wasmtime" in large_cold_vals) or any(
        "wasmtime" in vals for vals in cc["kernels_ms"].values()
    )
    cold_series = [
        ("x64", "wtime-aot x64"),
        ("c", "wtime-aot c (-O2)"),
        ("rust", "wtime-aot rust (-O3)"),
    ]
    if has_wt_cold:
        cold_series.append(("wasmtime", f"{wt_label} (Cranelift)"))

    (out_dir / "compile-cold.svg").write_text(
        render_bar_chart(
            title="Cold compilation time (empty cache, multi-core)",
            subtitle=f"median fresh-process compile + artifact write over {cc['rounds']} runs (log scale, lower is better; {host_tag})",
            series=cold_series,
            groups=cold_groups,
            scale="log",
            fmt_val=fmt_ms,
            fmt_tick=fmt_tick_ms,
            right_pad=160,
            note_fn=cold_note,
        )
    )
    rendered += 1

    # 3. Incremental rebuild, warm artifact load, & warm cache hit
    ch = data["compile_hot"]
    inc_vals = ch.get("large_1fn_edit_ms") or ch.get("gdev_1fn_edit_ms")
    load_vals = ch.get("large_warm_load_ms") or ch.get("gdev_warm_pen_load_ms")
    hit_large_vals = ch.get("large_build_cache_hit_ms") or ch.get("gdev_build_cache_hit_ms")

    kernel_hit_med = {}
    for b in ("x64", "c", "rust"):
        vals = [
            ch["kernels_build_cache_hit_ms"][k][b]
            for k in ch["kernels_build_cache_hit_ms"]
            if b in ch["kernels_build_cache_hit_ms"][k]
        ]
        if vals:
            kernel_hit_med[b] = sum(vals) / len(vals)

    hot_groups = []
    if inc_vals is not None:
        hot_groups.append((
            f"incremental 1-function edit ({large_fns:,} fns, warm function/object cache)" if large_fns else "incremental 1-function edit",
            [("1-fn rebuild", inc_vals)],
        ))
    if load_vals is not None:
        hot_groups.append((
            "warm fresh-process artifact load (.so dlopen vs .cwasm deserialize)",
            [("warm load", load_vals)],
        ))
    hit_rows = []
    if hit_large_vals is not None:
        hit_rows.append((f"5k-fn app ({large_mib})" if large_mib else "5k-fn app", hit_large_vals))
    hit_rows.append(("kernels (mean)", kernel_hit_med))
    hot_groups.append(("warm build --cache hit (CLI)", hit_rows))

    def hot_note(row: str, k: str, v: float, vals: dict[str, float]) -> str:
        if k == "x64":
            parts = []
            if vals.get("wasmtime") and vals["wasmtime"] / v >= 1.15:
                parts.append(f"{vals['wasmtime'] / v:.1f}× vs WT")
            if row == "1-fn rebuild" and vals.get("c") and vals["c"] / v >= 1.15:
                parts.append(f"{vals['c'] / v:.1f}× vs c")
            return f"({', '.join(parts)})" if parts else ""
        return ""

    hot_series = [
        ("x64", "wtime-aot x64"),
        ("c", "wtime-aot c (-O2)"),
        ("rust", "wtime-aot rust (-O3)"),
    ]
    if (inc_vals and "wasmtime" in inc_vals) or (load_vals and "wasmtime" in load_vals):
        hot_series.append(("wasmtime", wt_label))

    (out_dir / "compile-hot.svg").write_text(
        render_bar_chart(
            title="Incremental rebuild, warm load, & cache hit",
            subtitle=f"median wall time with warm caches (log scale, lower is better; {host_tag})",
            series=hot_series,
            groups=hot_groups,
            scale="log",
            fmt_val=fmt_ms,
            fmt_tick=fmt_tick_ms,
            right_pad=160,
            note_fn=hot_note,
        )
    )
    rendered += 1

    # 4. Headline summary parity chart vs Wasmtime (when Wasmtime data is present)
    if has_wt_kernels:
        summary_groups = []
        build_rows = []
        if large_cold_vals and large_cold_vals.get("x64") and large_cold_vals.get("wasmtime"):
            vx, vw = large_cold_vals["x64"], large_cold_vals["wasmtime"]
            build_rows.append(("Cold compile", {"x64": (vw / vx, f"({fmt_ms(vx)} vs {fmt_ms(vw)})")}))
        if inc_vals and inc_vals.get("x64") and inc_vals.get("wasmtime"):
            vx, vw = inc_vals["x64"], inc_vals["wasmtime"]
            build_rows.append(("1-fn edit rebuild", {"x64": (vw / vx, f"({fmt_ms(vx)} vs {fmt_ms(vw)})")}))
        if load_vals and load_vals.get("x64") and load_vals.get("wasmtime"):
            vx, vw = load_vals["x64"], load_vals["wasmtime"]
            build_rows.append(("Warm artifact load", {"x64": (vw / vx, f"({fmt_ms(vx)} vs {fmt_ms(vw)})")}))
        art = large_meta.get("artifact_bytes", {}) if large_meta else {}
        if art.get("x64") and art.get("wasmtime_cwasm"):
            sx, sw = art["x64"] / (1024 * 1024), art["wasmtime_cwasm"] / (1024 * 1024)
            build_rows.append(("Artifact size", {"x64": (sw / sx, f"({sx:.2f} vs {sw:.2f} MiB)")}))
        if build_rows:
            blabel = f"build, load, & size ({large_fns:,}-fn, {large_mib} app)" if large_fns and large_mib else "build, load, & size"
            summary_groups.append((blabel, build_rows))

        exec_rows = []
        ratios_x64 = []
        ratios_rust = []
        for c in kr["cases"]:
            mu = c["median_us"]
            vw = mu.get("wasmtime")
            if not vw:
                continue
            row_map = {}
            if mu.get("x64"):
                r = vw / mu["x64"]
                ratios_x64.append(r)
                row_map["x64"] = (r, f"({fmt_us(mu['x64'])} vs {fmt_us(vw)})")
            if mu.get("rust"):
                r = vw / mu["rust"]
                ratios_rust.append(r)
                row_map["rust"] = (r, f"({fmt_us(mu['rust'])} vs {fmt_us(vw)})")
            exec_rows.append((c["name"], row_map))
        if ratios_x64 and ratios_rust:
            gm_x64 = math.prod(ratios_x64) ** (1.0 / len(ratios_x64))
            gm_rust = math.prod(ratios_rust) ** (1.0 / len(ratios_rust))
            exec_rows.insert(0, (
                "Kernels geomean",
                {"x64": (gm_x64, "(4 kernels)"), "rust": (gm_rust, "(4 kernels)")},
            ))
        summary_groups.append(("execution speedup on self-contained Wasm kernels (CPU 0)", exec_rows))

        (out_dir / "summary.svg").write_text(
            render_parity_chart(
                title=f"At a glance: wtime-aot vs {wt_label}",
                subtitle=f"speedup / size-reduction ratio vs {wt_label} (log scale, right of 1× is faster or smaller; {host_tag})",
                series=[
                    ("x64", "wtime-aot x64 (pure Rust, 0 LLVM)"),
                    ("rust", "wtime-aot rust (-O3)"),
                ],
                groups=summary_groups,
                right_pad=186,
            )
        )
        rendered += 1
    return rendered


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("results", type=Path)
    parser.add_argument("-o", "--output", type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.results.read_text())
    count = build_all(data, args.output)
    print(f"Rendered {count} separate SVG chart(s) in {args.output}")


if __name__ == "__main__":
    main()
