#!/usr/bin/env python3
"""Reproducible benchmark harness for wtime-aot README charts.

Measures four separate categories on the host machine and writes JSON for chart.py:
  1. Blot game runtime (real 4,935-fn gdev: wtime-aot x64, wtime-aot c -O2, Wasmtime)
  2. Representative Wasm kernels runtime (wtime-aot x64, c -O2, rust -O3, V8/Node)
  3. Cold compilation time (multi-core --jobs 0, fresh cache: x64, c -O2, rust -O3, Wasmtime)
  4. Hot compilation & warm cache-hit time (incremental 1-fn edit, warm pen load, build --cache hit)

Usage:
    # Self-contained kernel suite (runs anywhere with clang, rustc, and node):
    python3 -B tools/wtime-aot/bench.py --kernels-only -o tools/wtime-aot/charts/results.json

    # Full suite including an external Blot game package (e.g. gdev):
    python3 -B tools/wtime-aot/bench.py --pen /path/to/pen --gdev-wasm /path/to/game.wasm --gdev-pkg /path/to/package/game -o tools/wtime-aot/charts/results.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

sys.dont_write_bytecode = True

REPO_ROOT = Path(__file__).resolve().parents[2]
WTIME_AOT_BIN = REPO_ROOT / "tools/wtime-aot/target/release/wtime-aot"
DEFAULT_RUST_TOOLCHAIN = os.environ.get("WTIME_AOT_RUST_TOOLCHAIN", "1.99.0")

DRIVER_C = r"""
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef void *(*fn_new)(void *, void *, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);

static double now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
}

int main(int argc, char **argv) {
    if (argc < 6) {
        fprintf(stderr, "usage: driver so_path prep_fn run_fn arg iters [warmup]\n");
        return 2;
    }
    const char *so_path = argv[1];
    const char *prep_fn = argv[2];
    const char *run_fn = argv[3];
    uint64_t arg = strtoull(argv[4], NULL, 0);
    int iters = atoi(argv[5]);
    int warmup = argc >= 7 ? atoi(argv[6]) : 10;

    void *dl = dlopen(so_path, RTLD_NOW);
    if (!dl) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 3; }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export b_call = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    uint8_t tr = 0;
    void *h = b_new(NULL, NULL, 0, &tr);
    if (!h) { fprintf(stderr, "new failed tr=%u\n", tr); return 4; }

    uint64_t res = 0;
    if (strcmp(prep_fn, "-") != 0) {
        if (b_call(h, (const uint8_t *)prep_fn, strlen(prep_fn), arg, &res, &tr) != 0) {
            fprintf(stderr, "prep failed tr=%u\n", tr); return 5;
        }
    }
    for (int i = 0; i < warmup; i++) {
        if (b_call(h, (const uint8_t *)run_fn, strlen(run_fn), arg, &res, &tr) != 0) {
            fprintf(stderr, "warmup failed tr=%u\n", tr); return 6;
        }
    }
    if (strcmp(prep_fn, "-") != 0) {
        if (b_call(h, (const uint8_t *)prep_fn, strlen(prep_fn), arg, &res, &tr) != 0) {
            fprintf(stderr, "re-prep failed tr=%u\n", tr); return 5;
        }
    }
    uint64_t acc_hash = 0;
    double t0 = now_ns();
    for (int i = 0; i < iters; i++) {
        if (b_call(h, (const uint8_t *)run_fn, strlen(run_fn), arg, &res, &tr) != 0) {
            fprintf(stderr, "run failed tr=%u\n", tr); return 7;
        }
        acc_hash = acc_hash * 1315423911ULL ^ res;
    }
    double elapsed_ms = (now_ns() - t0) / 1e6 / (double)iters;
    printf("%.6f %016llx\n", elapsed_ms, (unsigned long long)acc_hash);
    b_drop(h);
    dlclose(dl);
    return 0;
}
"""

NODE_DRIVER_JS = r"""
const fs = require('fs');
const { performance } = require('perf_hooks');
const [,, wasmPath, prepFn, runFn, argStr, itersStr, warmupStr] = process.argv;
const bytes = fs.readFileSync(wasmPath);
const mod = new WebAssembly.Module(bytes);
const inst = new WebAssembly.Instance(mod, {});
const exp = inst.exports;
const arg = Number(argStr) | 0;
const iters = Number(itersStr);
const warmup = Number(warmupStr);
if (prepFn !== '-') exp[prepFn](arg);
for (let i = 0; i < warmup; i++) exp[runFn](arg);
if (prepFn !== '-') exp[prepFn](arg);
let acc = 0n;
const t0 = performance.now();
for (let i = 0; i < iters; i++) {
  const r = BigInt(exp[runFn](arg) >>> 0);
  acc = ((acc * 1315423911n) & 0xffffffffffffffffn) ^ r;
}
const ms = (performance.now() - t0) / iters;
console.log(`${ms.toFixed(6)} ${acc.toString(16).padStart(16, '0')}`);
"""


def leb_u32(v: int) -> bytes:
    out = bytearray()
    while True:
        c = v & 0x7F
        v >>= 7
        if v:
            c |= 0x80
        out.append(c)
        if not v:
            return bytes(out)


def leb_i32(v: int) -> bytes:
    out = bytearray()
    while True:
        c = v & 0x7F
        v >>= 7
        if (v == 0 and (c & 0x40) == 0) or (v == -1 and (c & 0x40) != 0):
            out.append(c)
            return bytes(out)
        out.append(c | 0x80)


def read_leb(b: bytes, i: int) -> tuple[int, int]:
    v = s = 0
    while True:
        c = b[i]
        i += 1
        v |= (c & 0x7F) << s
        if not (c & 0x80):
            return v, i
        s += 7


def section(sid: int, payload: bytes) -> bytes:
    return bytes([sid]) + leb_u32(len(payload)) + payload


def enc_body(locals_spec: list[tuple[int, int]], body: bytearray | bytes) -> bytes:
    p = bytearray(leb_u32(len(locals_spec)))
    for cnt, ty in locals_spec:
        p += leb_u32(cnt) + bytes([ty])
    p += bytes(body) + b"\x0b"
    return leb_u32(len(p)) + bytes(p)


def build_ecs_kernel_wasm() -> bytes:
    """Blot-like ECS + physics + bulk-copy kernel (fixed 256 KiB memory, 5 fns)."""
    types = bytearray([4])
    types += bytes([0x60, 3, 0x7C, 0x7C, 0x7C, 1, 0x7C])
    types += bytes([0x60, 2, 0x7F, 0x7F, 1, 0x7F])
    types += bytes([0x60, 2, 0x7C, 0x7C, 1, 0x7C])
    types += bytes([0x60, 1, 0x7F, 1, 0x7F])
    funcs = bytes([5, 0, 1, 2, 3, 3])
    mem = bytes([1, 0x01, 4, 4])
    globs = bytearray([2])
    globs += bytes([0x7C, 1, 0x44]) + struct.pack("<d", 0.016666667) + bytes([0x0B])
    globs += bytes([0x7F, 1, 0x41]) + leb_i32(1337) + bytes([0x0B])
    exps = bytearray([3])
    for name, kind, idx in [("blot:memory", 2, 0), ("prepare", 0, 3), ("run", 0, 4)]:
        nb = name.encode()
        exps += leb_u32(len(nb)) + nb + bytes([kind]) + leb_u32(idx)

    b0 = bytearray([
        0x20, 0, 0x20, 1, 0x20, 0, 0x20, 1, 0x66, 0x1B,
        0x22, 0,
        0x20, 2, 0x20, 0, 0x20, 2, 0x65, 0x1B,
    ])
    b1 = bytearray([
        0x20, 0, 0x41, 3, 0x6E, 0x41, 31, 0x6C,
        0x20, 1, 0x41, 7, 0x70, 0x6A,
        0x20, 0, 0x41, 10, 0x70, 0x73,
    ])
    b2 = bytearray([0x20, 0, 0x20, 0, 0xA2, 0x20, 1, 0x20, 1, 0xA2, 0xA0])
    b3 = bytearray([
        0x02, 0x40, 0x03, 0x40,
        0x20, 1, 0x20, 0, 0x4F, 0x0D, 1,
        0x20, 1, 0x41, 5, 0x74, 0x22, 2,
        0x20, 1, 0xB7, 0x44,
    ]) + struct.pack("<d", 0.25) + bytearray([
        0xA2, 0x39, 3, 0,
        0x20, 2, 0x20, 1, 0xB7, 0x44,
    ]) + struct.pack("<d", -0.5) + bytearray([
        0xA2, 0x39, 3, 8,
        0x20, 2, 0x44,
    ]) + struct.pack("<d", 1.25) + bytearray([
        0x39, 3, 16,
        0x20, 2, 0x44,
    ]) + struct.pack("<d", -0.75) + bytearray([
        0x39, 3, 24,
        0x20, 1, 0x41, 1, 0x6A, 0x21, 1,
        0x0C, 0, 0x0B, 0x0B, 0x20, 0,
    ])
    b4 = bytearray([
        0x41, 0, 0x21, 1,
        0x23, 1, 0x21, 3,
        0x02, 0x40, 0x03, 0x40,
        0x20, 1, 0x20, 0, 0x4F, 0x0D, 1,
        0x20, 1, 0x41, 5, 0x74, 0x22, 2,
        0x2B, 3, 0, 0x21, 10,
        0x20, 2, 0x2B, 3, 8, 0x21, 11,
        0x20, 2, 0x2B, 3, 16, 0x21, 12,
        0x20, 2, 0x2B, 3, 24, 0x21, 13,
        0x20, 10, 0x20, 12, 0x23, 0, 0xA2, 0xA0, 0x44,
    ]) + struct.pack("<d", -100.0) + bytearray([0x44]) + struct.pack("<d", 100.0) + bytearray([
        0x10, 0, 0x22, 10, 0x20, 4, 0xA0, 0x21, 4,
        0x20, 11, 0x20, 13, 0x23, 0, 0xA2, 0xA0, 0x44,
    ]) + struct.pack("<d", -100.0) + bytearray([0x44]) + struct.pack("<d", 100.0) + bytearray([
        0x10, 0, 0x22, 11, 0x20, 5, 0xA0, 0x21, 5,
        0x20, 10, 0x20, 11, 0x10, 2, 0x20, 6, 0xA0, 0x21, 6,
        0x20, 7, 0x20, 12, 0xA0, 0x21, 7,
        0x20, 8, 0x20, 13, 0xA0, 0x21, 8,
        0x20, 9, 0x20, 10, 0x20, 11, 0x10, 2, 0x23, 0, 0xA2, 0xA0, 0x21, 9,
        0x20, 2, 0x20, 10, 0x39, 3, 0,
        0x20, 2, 0x20, 11, 0x39, 3, 8,
        0x20, 2, 0x41,
    ]) + leb_i32(131072) + bytearray([
        0x6A, 0x20, 2, 0x41, 32, 0xFC, 0x0A, 0, 0,
        0x20, 3, 0x41, 31, 0x6C,
        0x20, 1, 0x41, 7, 0x6A,
        0x20, 1, 0x41, 3, 0x6C, 0x41, 5, 0x6E,
        0x10, 1, 0x73, 0x21, 3,
        0x20, 1, 0x41, 1, 0x6A, 0x21, 1,
        0x0C, 0, 0x0B, 0x0B,
        0x20, 3,
        0x20, 4, 0x20, 5, 0xA0, 0x20, 6, 0xA0, 0x20, 7, 0xA0, 0x20, 8, 0xA0, 0x20, 9, 0xA0,
        0xFC, 0x02, 0x73,
    ])
    code = bytearray([5])
    code += enc_body([], b0)
    code += enc_body([], b1)
    code += enc_body([], b2)
    code += enc_body([(2, 0x7F)], b3)
    code += enc_body([(3, 0x7F), (10, 0x7C)], b4)
    return (
        b"\x00asm\x01\x00\x00\x00"
        + section(1, bytes(types))
        + section(3, funcs)
        + section(5, mem)
        + section(6, bytes(globs))
        + section(7, bytes(exps))
        + section(10, bytes(code))
    )


def strip_gdev_kernel(src_path: Path, dst_path: Path) -> None:
    b = src_path.read_bytes()
    out = bytearray(b[:8])
    i = 8
    while i < len(b):
        sid = b[i]
        i += 1
        slen, i = read_leb(b, i)
        payload = b[i : i + slen]
        i += slen
        if sid == 0:
            nlen, j = read_leb(payload, 0)
            cname = payload[j : j + nlen]
            if cname == b"blot:kernels":
                continue
            if cname == b"blot:abi":
                payload = leb_u32(len(cname)) + cname + b"\x08\x01\x04main\x04\x06\x06\x00\x00"
        elif sid == 7:
            cnt, j = read_leb(payload, 0)
            kept = []
            for _ in range(cnt):
                nlen, j = read_leb(payload, j)
                name = payload[j : j + nlen]
                j += nlen
                kind = payload[j]
                j += 1
                idx, j = read_leb(payload, j)
                if name not in (b"blot:kernel_ctx:plan_routes", b"blot:kernel_clear:plan_routes", b"plan_routes"):
                    kept.append((name, kind, idx))
            np = bytearray(leb_u32(len(kept)))
            for name, kind, idx in kept:
                np += leb_u32(len(name)) + name + bytes([kind]) + leb_u32(idx)
            payload = bytes(np)
        out.append(sid)
        out += leb_u32(len(payload)) + payload
    dst_path.write_bytes(bytes(out))


def edit_one_func_nop(src_path: Path, dst_path: Path, nop_count: int = 1) -> None:
    """Insert `nop_count` `0x01` (nop) instructions into function body 0 to simulate a 1-fn edit."""
    b = src_path.read_bytes()
    out = bytearray(b[:8])
    i = 8
    edited = False
    while i < len(b):
        sid = b[i]
        i += 1
        slen, i = read_leb(b, i)
        payload = b[i : i + slen]
        i += slen
        if sid == 10 and not edited:
            cnt, j = read_leb(payload, 0)
            assert cnt >= 1
            bsize, k = read_leb(payload, j)
            body0 = payload[k : k + bsize]
            assert body0.endswith(b"\x0b"), "function body must end with 0x0b"
            new_body0 = body0[:-1] + (b"\x01" * nop_count) + b"\x0b"
            payload = payload[:j] + leb_u32(len(new_body0)) + new_body0 + payload[k + bsize :]
            edited = True
        out.append(sid)
        out += leb_u32(len(payload)) + payload
    assert edited, "code section not found"
    dst_path.write_bytes(bytes(out))


def hash_tree(root: Path) -> tuple[int, str]:
    h = hashlib.sha256()
    count = 0
    for p in sorted(root.rglob("*")):
        if p.is_file():
            h.update(p.relative_to(root).as_posix().encode() + b"\x00")
            h.update(p.read_bytes() + b"\x00")
            count += 1
    return count, h.hexdigest()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def stage_pkg_without_game_wasm(pkg_dir: Path, dst_dir: Path) -> None:
    """Symlink package assets into `dst_dir` EXCEPT `game.wasm` so writing `dst_dir/game.wasm` never overwrites the source package."""
    dst_dir.mkdir(parents=True, exist_ok=True)
    for item in pkg_dir.iterdir():
        if item.name == "game.wasm":
            continue
        (dst_dir / item.name).symlink_to(item.resolve())
    dst_wasm = dst_dir / "game.wasm"
    if dst_wasm.exists() or dst_wasm.is_symlink():
        dst_wasm.unlink()


def host_info(wtime_aot_bin: Path, pen_bin: Path | None, gdev_wasm: Path | None, rust_toolchain: str) -> dict:
    cpu = "unknown"
    try:
        for line in subprocess.check_output(["lscpu"], text=True).splitlines():
            if line.startswith("Model name:"):
                cpu = line.split(":", 1)[1].strip()
                break
    except Exception:
        cpu = platform.processor() or "unknown"
    clang_v = subprocess.check_output(["clang", "--version"], text=True).splitlines()[0].strip()
    rustc_v = subprocess.check_output(["rustc", "--version"], text=True).strip()
    node_v = subprocess.check_output(["node", "--version"], text=True).strip()
    git_rev = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], cwd=REPO_ROOT, text=True).strip()
    info = {
        "cpu": cpu,
        "logical_cpus": os.cpu_count() or 1,
        "os": f"{platform.system()} {platform.release()}",
        "clang": clang_v,
        "rustc": rustc_v,
        "rust_aot_toolchain": rust_toolchain,
        "node": node_v,
        "git_rev": git_rev,
        "wtime_aot_sha256": sha256_file(wtime_aot_bin),
        "timestamp_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    if pen_bin is not None:
        info["pen_sha256"] = sha256_file(pen_bin)
    if gdev_wasm is not None:
        info["gdev_wasm_sha256"] = sha256_file(gdev_wasm)
    return info


def run_pen_bench(pen_bin: Path, gdir: Path, env: dict, expect_aot: bool, frames: int, pin_cpu0: bool) -> tuple[dict, str]:
    cmd = ([str(pen_bin)] if not pin_cpu0 else ["taskset", "-c", "0", str(pen_bin)]) + [
        "bench", "--game", str(gdir), "--frames", str(frames), "--json",
    ]
    p = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"pen bench failed rc={p.returncode}\nSTDOUT:\n{p.stdout}\nSTDERR:\n{p.stderr}")
    assert ("(wtime-aot)" in p.stderr) == expect_aot, f"backend mismatch expect_aot={expect_aot}: {p.stderr!r}"
    return json.loads(p.stdout), p.stderr.strip()


def run_pen_packets_ready_ms(pen_bin: Path, gdir: Path, outdir: Path, env: dict, expect_aot: bool, frames: int = 1) -> float:
    if outdir.exists():
        shutil.rmtree(outdir)
    p = subprocess.run(
        [str(pen_bin), "packets", str(outdir), "--game", str(gdir), "--frames", str(frames)],
        env=env, capture_output=True, text=True,
    )
    if p.returncode != 0:
        raise RuntimeError(f"pen packets failed rc={p.returncode}\nSTDOUT:\n{p.stdout}\nSTDERR:\n{p.stderr}")
    assert ("(wtime-aot)" in p.stderr) == expect_aot, f"packets backend mismatch: {p.stderr!r}"
    m = re.search(r"game module ready in ([0-9.]+) ms", p.stderr)
    if not m:
        raise RuntimeError(f"missing ready ms in stderr: {p.stderr!r}")
    return float(m.group(1))


def time_cmd_ms(cmd: list[str], env: dict | None = None) -> tuple[float, str]:
    t0 = time.perf_counter()
    p = subprocess.run(cmd, env=env, capture_output=True, text=True)
    dt_ms = (time.perf_counter() - t0) * 1000.0
    if p.returncode != 0:
        raise RuntimeError(f"cmd failed rc={p.returncode}: {' '.join(cmd)}\nSTDOUT:\n{p.stdout}\nSTDERR:\n{p.stderr}")
    return dt_ms, p.stdout.strip()


def run_kernel_driver(cmd: list[str]) -> tuple[float, str]:
    out = subprocess.check_output(cmd, text=True).strip()
    ms_s, hex_res = out.split()
    return float(ms_s), hex_res


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("-o", "--output", type=Path, required=True)
    parser.add_argument("--wtime-aot", type=Path, default=WTIME_AOT_BIN)
    parser.add_argument("--pen", type=Path, default=Path(os.environ["PEN_BIN"]) if os.environ.get("PEN_BIN") else None)
    parser.add_argument("--gdev-wasm", type=Path, default=Path(os.environ["GDEV_WASM"]) if os.environ.get("GDEV_WASM") else None)
    parser.add_argument("--gdev-pkg", type=Path, default=Path(os.environ["GDEV_PKG"]) if os.environ.get("GDEV_PKG") else None)
    parser.add_argument("--rust-toolchain", default=DEFAULT_RUST_TOOLCHAIN)
    parser.add_argument("--kernels-only", action="store_true", help="Run only the self-contained Wasm kernel suite (no external pen/gdev required)")
    parser.add_argument("--gdev-rounds", type=int, default=5)
    parser.add_argument("--kernel-rounds", type=int, default=7)
    parser.add_argument("--cold-rounds", type=int, default=3)
    parser.add_argument("--hot-rounds", type=int, default=7)
    args = parser.parse_args()

    wtime_aot_bin = args.wtime_aot
    assert wtime_aot_bin.is_file(), f"missing wtime-aot binary: {wtime_aot_bin} (build with `cargo build --release --manifest-path tools/wtime-aot/Cargo.toml`)"
    run_gdev = not args.kernels_only
    if run_gdev:
        if args.pen is None or args.gdev_wasm is None or args.gdev_pkg is None:
            raise SystemExit(
                "External Blot game paths are not configured. Pass --kernels-only to run the self-contained "
                "kernel benchmarks, or supply --pen, --gdev-wasm, and --gdev-pkg (or PEN_BIN/GDEV_WASM/GDEV_PKG)."
            )
        assert args.pen.is_file(), f"missing --pen binary: {args.pen}"
        assert args.gdev_wasm.is_file(), f"missing --gdev-wasm: {args.gdev_wasm}"
        assert args.gdev_pkg.is_dir(), f"missing --gdev-pkg directory: {args.gdev_pkg}"

    results: dict = {
        "host": host_info(
            wtime_aot_bin,
            args.pen if run_gdev else None,
            args.gdev_wasm if run_gdev else None,
            args.rust_toolchain,
        )
    }
    base_env = {k: v for k, v in os.environ.items() if not k.startswith("PEN_WTIME_AOT")}

    with tempfile.TemporaryDirectory(prefix="wtime-aot-charts-") as td_s:
        td = Path(td_s)

        # Build native C driver & JS driver for kernels
        driver_c = td / "driver.c"
        driver_bin = td / "driver"
        driver_c.write_text(DRIVER_C)
        subprocess.check_call(["cc", "-O2", str(driver_c), "-ldl", "-o", str(driver_bin)])
        driver_js = td / "driver.js"
        driver_js.write_text(NODE_DRIVER_JS)

        ecs_wasm = td / "ecs_kernel.wasm"
        ecs_wasm.write_bytes(build_ecs_kernel_wasm())

        kernels = [
            ("graph_bfs", REPO_ROOT / "tools/wtime-aot/fixtures/graph_bfs.wasm", "prepare", "run", 1024, 500),
            ("parser_hash", REPO_ROOT / "tools/wtime-aot/fixtures/parser_hash.wasm", "prepare", "run", 4096, 2000),
            ("image_filter", REPO_ROOT / "tools/wtime-aot/fixtures/image_filter.wasm", "prepare", "run", 4096, 500),
            ("ecs_kernel", ecs_wasm, "prepare", "run", 2048, 500),
        ]

        gdir = None
        gdev_wasm = None
        wt_cache = c_cache = x64_cache = None
        env_wt = env_c = env_x64 = None

        if run_gdev:
            # Prepare stripped gdev package dir without symlinking source game.wasm
            gdir = td / "gdev_game"
            stage_pkg_without_game_wasm(args.gdev_pkg, gdir)
            gdev_wasm = gdir / "game.wasm"
            strip_gdev_kernel(args.gdev_wasm, gdev_wasm)

            # ------------------------------------------------------------------
            # 1. Blot game runtime (real 4,935-fn gdev, CPU 0 pinned, 600 frames)
            # ------------------------------------------------------------------
            print("=== [1/4] Real gdev runtime (600 frames x %d rounds, CPU 0) ===" % args.gdev_rounds, flush=True)
            wt_cache = td / "gdev_wt_cache"; wt_cache.mkdir(mode=0o700)
            c_cache = td / "gdev_c_cache"; c_cache.mkdir(mode=0o700)
            x64_cache = td / "gdev_x64_cache"; x64_cache.mkdir(mode=0o700)

            env_wt = dict(base_env, PEN_WTIME_AOT="0", XDG_CACHE_HOME=str(wt_cache))
            env_c = dict(base_env, PEN_WTIME_AOT="1", PEN_WTIME_AOT_BACKEND="c", PEN_WTIME_AOT_C_OPT="auto",
                         PEN_WTIME_AOT_BIN=str(wtime_aot_bin), PEN_WTIME_AOT_CACHE=str(c_cache))
            env_x64 = dict(base_env, PEN_WTIME_AOT="1", PEN_WTIME_AOT_BACKEND="x64",
                           PEN_WTIME_AOT_BIN=str(wtime_aot_bin), PEN_WTIME_AOT_CACHE=str(x64_cache))

            # Warm AOT caches unpinned (multi-core) before pinned runtime rounds
            for _label, expect_aot, env in [("x64", True, env_x64), ("c", True, env_c)]:
                run_pen_bench(args.pen, gdir, env, expect_aot=expect_aot, frames=150, pin_cpu0=False)

            gdev_engines = [("x64", True, env_x64), ("c", True, env_c), ("wasmtime", False, env_wt)]
            gdev_samples = {k: [] for k, _, _ in gdev_engines}
            for r in range(args.gdev_rounds):
                order = list(gdev_engines)
                if r % 2 == 1:
                    order.reverse()
                for name, expect_aot, env in order:
                    d, _ = run_pen_bench(args.pen, gdir, env, expect_aot=expect_aot, frames=600, pin_cpu0=True)
                    gdev_samples[name].append(d["metrics"])

            # Exact 120-frame packet parity check across all 3 engines
            hashes = {}
            for name, expect_aot, env in gdev_engines:
                pdir = td / f"parity_{name}"
                run_pen_packets_ready_ms(args.pen, gdir, pdir, env, expect_aot=expect_aot, frames=120)
                hashes[name] = hash_tree(pdir)
            assert hashes["x64"] == hashes["c"] == hashes["wasmtime"], f"gdev parity mismatch: {hashes}"
            print(f"  120-frame parity EXACT across x64/c/wasmtime: {hashes['x64'][0]} files, sha256={hashes['x64'][1][:16]}...", flush=True)

            # Artifact sizes
            x64_so = next(p for p in (x64_cache / "verified").glob("*.so"))
            c_so = next(p for p in (c_cache / "verified").glob("*.so"))
            cwasm_files = list((wt_cache / "pen").rglob("*.cwasm"))
            cwasm_bytes = cwasm_files[0].stat().st_size if cwasm_files else None

            gdev_summary = {}
            for name, _, _ in gdev_engines:
                ms = gdev_samples[name]
                gdev_summary[name] = {
                    "guest_median_ms": statistics.median(x["guest_median_ms"] for x in ms),
                    "guest_p95_ms": statistics.median(x["guest_p95_ms"] for x in ms),
                    "snapshot_median_ms": statistics.median(x["snapshot_median_ms"] for x in ms),
                    "bench_load_ms": statistics.median(x["load_ms"] for x in ms),
                }
                print(f"  {name:10s}: med={gdev_summary[name]['guest_median_ms']:.4f}ms "
                      f"p95={gdev_summary[name]['guest_p95_ms']:.4f}ms "
                      f"snap={gdev_summary[name]['snapshot_median_ms']:.4f}ms", flush=True)

            results["runtime_gdev"] = {
                "wasm_bytes": gdev_wasm.stat().st_size,
                "functions": 4935,
                "rounds": args.gdev_rounds,
                "frames": 600,
                "parity_files": hashes["x64"][0],
                "parity_sha256": hashes["x64"][1],
                "artifact_bytes": {
                    "x64": x64_so.stat().st_size,
                    "c": c_so.stat().st_size,
                    "wasmtime_cwasm": cwasm_bytes,
                },
                "metrics": gdev_summary,
            }

        # ------------------------------------------------------------------
        # 2. Representative Wasm kernels runtime (CPU 0, 7 rounds) + build caches
        # ------------------------------------------------------------------
        print("\n=== [2/4] Representative Wasm kernels runtime (%d rounds, CPU 0) ===" % args.kernel_rounds, flush=True)
        kernel_runtime = []
        kernel_cold_samples: dict[str, dict[str, list[float]]] = {}
        kernel_hot_hit_samples: dict[str, dict[str, list[float]]] = {}

        for name, wpath, prep, run_fn, arg, iters in kernels:
            kernel_cold_samples[name] = {"x64": [], "c": [], "rust": []}
            kernel_hot_hit_samples[name] = {"x64": [], "c": [], "rust": []}

            so_paths: dict[str, Path] = {}
            warm_caches: dict[str, Path] = {}
            # Measure cold compilation (fresh cache per round) for x64, c, rust
            for backend, extra in [
                ("x64", []),
                ("c", ["--c-opt", "auto"]),
                ("rust", ["--toolchain", args.rust_toolchain]),
            ]:
                for cr in range(args.cold_rounds):
                    cc = td / f"cold_{name}_{backend}_{cr}"
                    cc.mkdir(mode=0o700)
                    cmd = [
                        str(wtime_aot_bin), "build", str(wpath),
                        "--cache", str(cc), "--blot-cdylib", "--backend", backend,
                        "--target-cpu", "native", "--jobs", "0",
                    ] + extra
                    dt_ms, out_path = time_cmd_ms(cmd)
                    kernel_cold_samples[name][backend].append(dt_ms)
                    if cr == args.cold_rounds - 1:
                        so_paths[backend] = Path(out_path)
                        warm_caches[backend] = cc

                # Measure warm build --cache hit latency (fresh CLI invocation)
                cmd_warm = [
                    str(wtime_aot_bin), "build", str(wpath),
                    "--cache", str(warm_caches[backend]), "--blot-cdylib", "--backend", backend,
                    "--target-cpu", "native", "--jobs", "0",
                ] + extra
                for _ in range(args.hot_rounds):
                    dt_ms, _ = time_cmd_ms(cmd_warm)
                    kernel_hot_hit_samples[name][backend].append(dt_ms)

            # Paired runtime measurement across x64, c, rust, v8 (Node TurboFan)
            eng_cmds = [
                ("x64", ["taskset", "-c", "0", str(driver_bin), str(so_paths["x64"]), prep, run_fn, str(arg), str(iters), "20"]),
                ("c", ["taskset", "-c", "0", str(driver_bin), str(so_paths["c"]), prep, run_fn, str(arg), str(iters), "20"]),
                ("rust", ["taskset", "-c", "0", str(driver_bin), str(so_paths["rust"]), prep, run_fn, str(arg), str(iters), "20"]),
                ("v8", ["taskset", "-c", "0", "node", str(driver_js), str(wpath), prep, run_fn, str(arg), str(iters), str(max(2000, iters * 4))]),
            ]
            samples = {k: [] for k, _ in eng_cmds}
            seen_hashes = {}
            for r in range(args.kernel_rounds):
                order = list(eng_cmds)
                if r % 2 == 1:
                    order.reverse()
                for eng, cmd in order:
                    ms, h = run_kernel_driver(cmd)
                    samples[eng].append(ms * 1000.0)  # microseconds
                    seen_hashes.setdefault(eng, set()).add(h)
            all_h = {h for s in seen_hashes.values() for h in s}
            assert len(all_h) == 1, f"kernel parity mismatch on {name}: {seen_hashes}"
            meds_us = {eng: statistics.median(vals) for eng, vals in samples.items()}
            print(f"  {name:14s}: x64={meds_us['x64']:6.2f}us  c={meds_us['c']:6.2f}us  "
                  f"rust={meds_us['rust']:6.2f}us  v8={meds_us['v8']:6.2f}us  parity={next(iter(all_h))}", flush=True)
            kernel_runtime.append({
                "name": name,
                "wasm_bytes": wpath.stat().st_size,
                "arg": arg,
                "iters": iters,
                "parity_hash": next(iter(all_h)),
                "median_us": meds_us,
            })

        results["runtime_kernels"] = {
            "rounds": args.kernel_rounds,
            "cases": kernel_runtime,
        }

        if run_gdev:
            assert gdir is not None and gdev_wasm is not None
            assert wt_cache is not None and env_wt is not None and env_c is not None and env_x64 is not None

            # ------------------------------------------------------------------
            # 3. Cold compilation time on real gdev (multi-core --jobs 0, fresh cache)
            # ------------------------------------------------------------------
            print("\n=== [3/4] Cold compilation on real gdev (multi-core, %d rounds) ===" % args.cold_rounds, flush=True)
            gdev_cold: dict[str, list[float]] = {"x64": [], "c": [], "wasmtime": []}
            last_gdev_c_cache = None
            last_gdev_x64_cache = None
            for cr in range(args.cold_rounds):
                # x64 fresh cache
                xc = td / f"gdev_cold_x64_{cr}"; xc.mkdir(mode=0o700)
                dt_x64, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(gdev_wasm),
                    "--cache", str(xc), "--blot-cdylib", "--backend", "x64",
                    "--target-cpu", "native", "--jobs", "0",
                ])
                gdev_cold["x64"].append(dt_x64)
                last_gdev_x64_cache = xc

                # Wasmtime cold Cranelift compile (unpinned multi-core via pen bench)
                wc = td / f"gdev_cold_wt_{cr}"; wc.mkdir(mode=0o700)
                d_wt, _ = run_pen_bench(
                    args.pen,
                    gdir,
                    dict(base_env, PEN_WTIME_AOT="0", XDG_CACHE_HOME=str(wc)),
                    expect_aot=False, frames=125, pin_cpu0=False,
                )
                gdev_cold["wasmtime"].append(d_wt["metrics"]["load_ms"])

                # Clang -O2 fresh cache (multi-core --jobs 0)
                cc = td / f"gdev_cold_c_{cr}"; cc.mkdir(mode=0o700)
                dt_c, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(gdev_wasm),
                    "--cache", str(cc), "--blot-cdylib", "--backend", "c",
                    "--c-opt", "auto", "--target-cpu", "native", "--jobs", "0",
                ])
                gdev_cold["c"].append(dt_c)
                last_gdev_c_cache = cc
                print(f"  round {cr+1}: x64={dt_x64:.1f}ms  wasmtime={d_wt['metrics']['load_ms']:.1f}ms  c={dt_c:.1f}ms", flush=True)

            results["compile_cold"] = {
                "rounds": args.cold_rounds,
                "gdev_ms": {k: statistics.median(v) for k, v in gdev_cold.items()},
                "kernels_ms": {
                    name: {b: statistics.median(vals) for b, vals in bs.items()}
                    for name, bs in kernel_cold_samples.items()
                },
            }

            # ------------------------------------------------------------------
            # 4. Hot compilation & warm cache-hit times
            # ------------------------------------------------------------------
            print("\n=== [4/4] Hot compilation, incremental 1-fn edit, & warm pen load ===", flush=True)
            # 4a. Incremental 1-function edit on real gdev (warm c-obj/ shard cache for c)
            inc_samples: dict[str, list[float]] = {"x64": [], "c": [], "wasmtime": []}
            for ir in range(args.cold_rounds):
                ed_dir = td / f"gdev_edit_{ir}"
                stage_pkg_without_game_wasm(args.gdev_pkg, ed_dir)
                ed_wasm = ed_dir / "game.wasm"
                edit_one_func_nop(gdev_wasm, ed_wasm, nop_count=ir + 1)

                dt_x64, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(ed_wasm),
                    "--cache", str(last_gdev_x64_cache), "--blot-cdylib", "--backend", "x64",
                    "--target-cpu", "native", "--jobs", "0",
                ])
                inc_samples["x64"].append(dt_x64)

                dt_c, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(ed_wasm),
                    "--cache", str(last_gdev_c_cache), "--blot-cdylib", "--backend", "c",
                    "--c-opt", "auto", "--target-cpu", "native", "--jobs", "0",
                ])
                inc_samples["c"].append(dt_c)

                d_wt, _ = run_pen_bench(
                    args.pen,
                    ed_dir,
                    dict(base_env, PEN_WTIME_AOT="0", XDG_CACHE_HOME=str(wt_cache)),
                    expect_aot=False, frames=125, pin_cpu0=False,
                )
                inc_samples["wasmtime"].append(d_wt["metrics"]["load_ms"])
                print(f"  1-fn edit r{ir+1}: x64={dt_x64:.1f}ms  c(warm c-obj)={dt_c:.1f}ms  wasmtime={d_wt['metrics']['load_ms']:.1f}ms", flush=True)

            # 4b. Warm fresh-process game module load in pen (gdev) — identical `pen packets` ready-ms
            # boundary for all three engines (x64 .so, c .so, Wasmtime cached .cwasm).
            warm_pen_load: dict[str, list[float]] = {"x64": [], "c": [], "wasmtime": []}
            run_pen_packets_ready_ms(args.pen, gdir, td / "wt_warm0", env_wt, expect_aot=False, frames=1)
            for hr in range(args.hot_rounds):
                x64_ready = run_pen_packets_ready_ms(args.pen, gdir, td / f"x64_warm_{hr}", env_x64, expect_aot=True, frames=1)
                warm_pen_load["x64"].append(x64_ready)
                c_ready = run_pen_packets_ready_ms(args.pen, gdir, td / f"c_warm_{hr}", env_c, expect_aot=True, frames=1)
                warm_pen_load["c"].append(c_ready)
                wt_ready = run_pen_packets_ready_ms(args.pen, gdir, td / f"wt_warm_{hr}", env_wt, expect_aot=False, frames=1)
                warm_pen_load["wasmtime"].append(wt_ready)
            print(f"  warm pen packets load (gdev): x64={statistics.median(warm_pen_load['x64']):.2f}ms  "
                  f"c={statistics.median(warm_pen_load['c']):.2f}ms  "
                  f"wasmtime(.cwasm)={statistics.median(warm_pen_load['wasmtime']):.1f}ms", flush=True)

            # 4c. Warm wtime-aot build --cache hit on gdev
            gdev_build_hit: dict[str, list[float]] = {"x64": [], "c": []}
            for _ in range(args.hot_rounds):
                dt_x64, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(gdev_wasm),
                    "--cache", str(last_gdev_x64_cache), "--blot-cdylib", "--backend", "x64",
                    "--target-cpu", "native", "--jobs", "0",
                ])
                gdev_build_hit["x64"].append(dt_x64)
                dt_c, _ = time_cmd_ms([
                    str(wtime_aot_bin), "build", str(gdev_wasm),
                    "--cache", str(last_gdev_c_cache), "--blot-cdylib", "--backend", "c",
                    "--c-opt", "auto", "--target-cpu", "native", "--jobs", "0",
                ])
                gdev_build_hit["c"].append(dt_c)
            print(f"  warm build --cache hit (gdev): x64={statistics.median(gdev_build_hit['x64']):.2f}ms  "
                  f"c={statistics.median(gdev_build_hit['c']):.2f}ms", flush=True)

            results["compile_hot"] = {
                "edit_rounds": args.cold_rounds,
                "hot_rounds": args.hot_rounds,
                "gdev_1fn_edit_ms": {k: statistics.median(v) for k, v in inc_samples.items()},
                "gdev_warm_pen_load_ms": {k: statistics.median(v) for k, v in warm_pen_load.items()},
                "gdev_build_cache_hit_ms": {k: statistics.median(v) for k, v in gdev_build_hit.items()},
                "kernels_build_cache_hit_ms": {
                    name: {b: statistics.median(vals) for b, vals in bs.items()}
                    for name, bs in kernel_hot_hit_samples.items()
                },
            }
        else:
            results["compile_cold"] = {
                "rounds": args.cold_rounds,
                "kernels_ms": {
                    name: {b: statistics.median(vals) for b, vals in bs.items()}
                    for name, bs in kernel_cold_samples.items()
                },
            }
            results["compile_hot"] = {
                "hot_rounds": args.hot_rounds,
                "kernels_build_cache_hit_ms": {
                    name: {b: statistics.median(vals) for b, vals in bs.items()}
                    for name, bs in kernel_hot_hit_samples.items()
                },
            }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(results, indent=2) + "\n")
    print(f"\nWrote {args.output}")


if __name__ == "__main__":
    main()
