#!/usr/bin/env python3
"""Render separate per-category SVG charts for wtime-aot from bench.py JSON.

    python3 -B tools/wtime-aot/chart.py tools/wtime-aot/charts/results.json -o tools/wtime-aot/charts

Writes four standalone light/dark SVG images matching tools/wtime-goose/chart.py's visual language:
  - runtime-gdev.svg    : Real 4,935-fn Blot game (gdev) steady-state runtime (ms, linear)
  - runtime-kernels.svg : Representative Wasm kernels runtime (us, log scale)
  - compile-cold.svg    : Cold multi-core compilation time (ms/s, log scale)
  - compile-hot.svg     : Hot compilation, incremental 1-fn rebuild, & warm cache-hit load (ms/s, log scale)
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

STYLE = """
.c { --surface:#fcfcfb; --ink:#0b0b0b; --ink2:#52514e; --muted:#898781; --grid:#e1e0d9; --axis:#c3c2b7;
     --x64:#2a78d6; --c:#eb6834; --wt:#2b9348; --rust:#8b5cf6;
     font-family:system-ui,-apple-system,"Segoe UI",sans-serif; }
@media (prefers-color-scheme: dark) {
  .c { --surface:#1a1a19; --ink:#f0efec; --ink2:#c3c2b7; --muted:#898781; --grid:#2c2c2a; --axis:#383835;
       --x64:#3987e5; --c:#d95926; --wt:#40b362; --rust:#a78bfa; }
}
.bg { fill:var(--surface); }
.t { fill:var(--ink); font-size:15px; font-weight:600; }
.st { fill:var(--ink2); font-size:12px; }
.lb { fill:var(--ink); font-size:12px; }
.grp { fill:var(--ink2); font-size:11px; font-weight:600; letter-spacing:.04em; }
.v { fill:var(--ink2); font-size:11px; font-variant-numeric:tabular-nums; }
.tick { fill:var(--muted); font-size:11px; font-variant-numeric:tabular-nums; }
.grid { stroke:var(--grid); stroke-width:1; }
.ax { stroke:var(--axis); stroke-width:1.5; }
.x64 { fill:var(--x64); }
.c { fill:var(--c); }
.wt { fill:var(--wt); }
.rust { fill:var(--rust); }
"""

# Note: rename `.c` bar class to `.bc` so it does not collide with root `.c` SVG class!
STYLE = STYLE.replace(".c { fill:var(--c); }", ".bc { fill:var(--c); }")

CLASS_FOR = {
    "x64": "x64",
    "c": "bc",
    "wasmtime": "wt",
    "v8": "wt",
    "rust": "rust",
}

WIDTH = 780
BAR = 11
GAP = 3
ROW_GAP = 14


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
        s = t / 1000.0
        return f"{s:g} s"
    return f"{t:g} ms"


def fmt_tick_us(t: float) -> str:
    if t >= 1000.0:
        return f"{t / 1000.0:g} ms"
    return f"{t:g} µs"


def hbar(x0: float, x1: float, y: float, cls: str) -> str:
    x1 = max(x1, x0 + 2.0)
    r = min(4.0, (x1 - x0) / 2.0, BAR / 2.0)
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
    right_pad: int = 86,
    note_fn=None,
) -> str:
    """Generic grouped horizontal bar chart (linear or log10, lower is better)."""
    all_vals = [
        v
        for _, rows in groups
        for _, vals in rows
        for k, _ in series
        if (v := vals.get(k)) is not None and v > 0
    ]
    assert all_vals, f"no positive values for {title}"

    longest_label = max(len(lbl) for _, rows in groups for lbl, _ in rows)
    left = max(136, 26 + int(6.7 * longest_label))
    plot = WIDTH - left - right_pad

    if scale == "linear":
        hi_raw = max(all_vals) * 1.15
        # Nice ceiling step
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
        lo_exp = math.floor(math.log10(lo_val / 1.35))
        hi_exp = math.ceil(math.log10(hi_val * 1.45))
        lo_log = math.log10(max(10 ** lo_exp, lo_val / 1.8))
        hi_log = math.log10(hi_val * 1.65)
        ticks = []
        for e in range(lo_exp - 1, hi_exp + 2):
            for m in (1, 2, 5):
                cand = m * (10 ** e)
                if lo_log - 1e-9 <= math.log10(cand) <= hi_log + 1e-9:
                    ticks.append(cand)
        # If dense, keep only powers of 10 (plus 2/5 if sparse)
        if len(ticks) > 9:
            ticks = [t for t in ticks if abs(math.log10(t) - round(math.log10(t))) < 1e-9]

        def x_of(v: float) -> float:
            return left + (math.log10(v) - lo_log) / (hi_log - lo_log) * plot

        x_zero = left
    else:
        raise ValueError(scale)

    top = 94
    group_heights = []
    for gname, rows in groups:
        gh = (22 if gname else 4)
        for _, vals in rows:
            present = sum(1 for k, _ in series if vals.get(k) is not None)
            gh += present * BAR + max(0, present - 1) * GAP + ROW_GAP
        group_heights.append(gh)
    bottom = top + sum(group_heights)
    height = bottom + 36

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" class="c" viewBox="0 0 {WIDTH} {height}" '
        f'width="{WIDTH}" height="{height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{esc(title)}</title>',
        f'<desc id="desc">{esc(subtitle)}</desc>',
        f"<style>{STYLE}</style>",
        f'<rect class="bg" width="{WIDTH}" height="{height}" rx="8"/>',
        f'<text class="t" x="16" y="28">{esc(title)}</text>',
        f'<text class="st" x="16" y="47">{esc(subtitle)}</text>',
    ]

    # Legend
    lx = 16
    for key, text in series:
        cls = CLASS_FOR[key]
        out.append(f'<rect class="{cls}" x="{lx}" y="62" width="12" height="12" rx="3"/>')
        out.append(f'<text class="lb" x="{lx + 18}" y="72">{esc(text)}</text>')
        lx += 30 + int(6.6 * len(text))

    # Gridlines & ticks
    for t in ticks:
        tx = x_of(t)
        is_axis = (scale == "linear" and t == 0)
        out.append(
            f'<line class="{"ax" if is_axis else "grid"}" x1="{tx:.1f}" x2="{tx:.1f}" '
            f'y1="{top - 6}" y2="{bottom}"/>'
        )
        out.append(
            f'<text class="tick" x="{tx:.1f}" y="{bottom + 17}" text-anchor="middle">'
            f'{esc(fmt_tick(t))}</text>'
        )
    if scale == "log":
        out.append(f'<line class="ax" x1="{left:.1f}" x2="{left:.1f}" y1="{top - 6}" y2="{bottom}"/>')

    # Groups & rows
    y = top
    for gname, rows in groups:
        if gname:
            out.append(f'<text class="grp" x="16" y="{y + 11}">{esc(gname.upper())}</text>')
            y += 22
        else:
            y += 4
        for rlabel, vals in rows:
            active = [(k, text, vals[k]) for k, text in series if vals.get(k) is not None]
            row_h = len(active) * BAR + max(0, len(active) - 1) * GAP
            out.append(
                f'<text class="lb" x="{left - 10}" y="{y + row_h / 2.0 + 4.0:.1f}" text-anchor="end">'
                f'{esc(rlabel)}</text>'
            )
            for idx, (k, text, v) in enumerate(active):
                by = y + idx * (BAR + GAP)
                end = x_of(v)
                val_txt = fmt_val(v)
                if note_fn is not None:
                    extra = note_fn(rlabel, k, v, vals)
                    if extra:
                        val_txt = f"{val_txt} {extra}"
                out.append(f"<g><title>{esc(rlabel)} — {esc(text)}: {esc(val_txt)}</title>")
                out.append(hbar(x_zero, end, by, CLASS_FOR[k]))
                out.append(
                    f'<text class="v" x="{max(end, x_zero + 2.0) + 6:.1f}" y="{by + BAR - 1.5:.1f}" '
                    f'text-anchor="start">{esc(val_txt)}</text></g>'
                )
            y += row_h + ROW_GAP

    out.append("</svg>")
    return "\n".join(out) + "\n"


def build_all(data: dict, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    host = data["host"]
    cpu_short = host["cpu"].replace(" 8-Core Processor", "").replace(" Processor", "")
    host_tag = f"{cpu_short}, {host['logical_cpus']}T"

    # ------------------------------------------------------------------
    # 1. Real gdev runtime (separate category image)
    # ------------------------------------------------------------------
    gd = data["runtime_gdev"]
    m = gd["metrics"]
    gdev_rows = [
        ("frame median", {k: m[k]["guest_median_ms"] for k in ("x64", "c", "wasmtime")}),
        ("frame p95", {k: m[k]["guest_p95_ms"] for k in ("x64", "c", "wasmtime")}),
        ("snapshot", {k: m[k]["snapshot_median_ms"] for k in ("x64", "c", "wasmtime")}),
    ]

    def gdev_note(_row: str, k: str, v: float, vals: dict[str, float]) -> str:
        if k == "x64" and vals.get("wasmtime"):
            return f"({v / vals['wasmtime']:.2f}× WT)"
        return ""

    (out_dir / "runtime-gdev.svg").write_text(
        render_bar_chart(
            title="Blot game runtime: gdev (3.57 MiB, 4,935 functions)",
            subtitle=f"median ms per frame / snapshot over {gd['rounds']}×{gd['frames']} frames (CPU 0, lower is better; {cpu_short})",
            series=[
                ("x64", "wtime-aot x64"),
                ("c", "wtime-aot c (-O2)"),
                ("wasmtime", "Wasmtime"),
            ],
            groups=[("", gdev_rows)],
            scale="linear",
            fmt_val=fmt_ms,
            fmt_tick=fmt_tick_ms,
            right_pad=130,
            note_fn=gdev_note,
        )
    )

    # ------------------------------------------------------------------
    # 2. Representative Wasm kernels runtime (separate category image)
    # ------------------------------------------------------------------
    kr = data["runtime_kernels"]
    kernel_rows = [(c["name"], c["median_us"]) for c in kr["cases"]]
    (out_dir / "runtime-kernels.svg").write_text(
        render_bar_chart(
            title="Wasm kernels runtime: native backends vs V8",
            subtitle=f"median µs per call over {kr['rounds']} paired rounds (CPU 0, log scale, lower is better; {cpu_short})",
            series=[
                ("x64", "wtime-aot x64"),
                ("c", "wtime-aot c (-O2)"),
                ("rust", "wtime-aot rust (-O3)"),
                ("v8", f"V8 ({host['node']})"),
            ],
            groups=[("", kernel_rows)],
            scale="log",
            fmt_val=fmt_us,
            fmt_tick=fmt_tick_us,
            right_pad=78,
        )
    )

    # ------------------------------------------------------------------
    # 3. Cold compilation time (separate category image)
    # ------------------------------------------------------------------
    cc = data["compile_cold"]
    cold_groups = [
        ("gdev game (3.57 MiB, 4,935 functions)", [("gdev", cc["gdev_ms"])]),
        ("representative kernels (0.3–1 KiB)", [(name, vals) for name, vals in cc["kernels_ms"].items()]),
    ]

    def cold_note(row: str, k: str, v: float, vals: dict[str, float]) -> str:
        if row == "gdev" and k == "x64" and vals.get("c") and vals.get("wasmtime"):
            return f"({vals['wasmtime'] / v:.1f}× vs WT, {vals['c'] / v:.0f}× vs c)"
        return ""

    (out_dir / "compile-cold.svg").write_text(
        render_bar_chart(
            title="Cold compilation time (empty cache, multi-core)",
            subtitle=f"median wall time over {cc['rounds']} fresh-cache runs (log scale, lower is better; {host_tag})",
            series=[
                ("x64", "wtime-aot x64"),
                ("c", "wtime-aot c (-O2)"),
                ("rust", "wtime-aot rust (-O3)"),
                ("wasmtime", "Wasmtime (Cranelift)"),
            ],
            groups=cold_groups,
            scale="log",
            fmt_val=fmt_ms,
            fmt_tick=fmt_tick_ms,
            right_pad=145,
            note_fn=cold_note,
        )
    )

    # ------------------------------------------------------------------
    # 4. Hot compilation & warm cache-hit time (separate category image)
    # ------------------------------------------------------------------
    ch = data["compile_hot"]
    kernel_hit_med = {}
    for b in ("x64", "c", "rust"):
        vals = [ch["kernels_build_cache_hit_ms"][k][b] for k in ch["kernels_build_cache_hit_ms"] if b in ch["kernels_build_cache_hit_ms"][k]]
        if vals:
            kernel_hit_med[b] = sum(vals) / len(vals)

    hot_groups = [
        ("incremental 1-fn edit (gdev, warm object cache)", [("1-fn rebuild", ch["gdev_1fn_edit_ms"])]),
        ("warm fresh-process load (gdev: .so vs cached .cwasm)", [("warm load", ch["gdev_warm_pen_load_ms"])]),
        ("warm build --cache hit (CLI)", [
            ("gdev (3.57 MiB)", ch["gdev_build_cache_hit_ms"]),
            ("kernels (mean)", kernel_hit_med),
        ]),
    ]
    (out_dir / "compile-hot.svg").write_text(
        render_bar_chart(
            title="Hot compilation, incremental rebuild, & warm load",
            subtitle=f"median wall time with warm caches (log scale, lower is better; {host_tag})",
            series=[
                ("x64", "wtime-aot x64"),
                ("c", "wtime-aot c (-O2)"),
                ("rust", "wtime-aot rust (-O3)"),
                ("wasmtime", "Wasmtime"),
            ],
            groups=hot_groups,
            scale="log",
            fmt_val=fmt_ms,
            fmt_tick=fmt_tick_ms,
            right_pad=88,
        )
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("results", type=Path)
    parser.add_argument("-o", "--output", type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.results.read_text())
    build_all(data, args.output)
    print(f"Rendered 4 separate SVG charts in {args.output}")


if __name__ == "__main__":
    main()
