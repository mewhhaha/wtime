//! Stage 1 18-export ABI behavioral parity, C-vs-x64 deviation checks, CPUID/CET SHSTK
//! fault-injection, RLIMIT_AS & per-site mprotect fault-injection, watchdog signal-at-floor,
//! 128 KiB pthread stack/reentry/R14+R15-clobber/MXCSR isolation, and CLI cache/exit-code tests.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use super::lower::TargetIsa;
use super::runtime::{CpuidSource, MprotectSite, RdsspSource, RuntimeTestHooks, SyscallFault};
use super::tests_unit::{
    code_sec, enc_body, enc_str, export_sec, func_sec, leb_i32, leb_u32, section, type_sec,
    wasm_module,
};
use super::{X64CompileOptions, compile_wasm_to_x64_so};

fn compile_c_test_bin(
    root: &Path,
    name: &str,
    src: &str,
    extra_flags: &[&str],
) -> Option<std::path::PathBuf> {
    let Ok(cc) = crate::cgen::detect_cc_with_deadline("native", None, None) else {
        if std::env::var_os("WTIME_AOT_REQUIRE_NATIVE_TESTS").is_some_and(|v| v == "1") {
            panic!(
                "WTIME_AOT_REQUIRE_NATIVE_TESTS=1 is set, but no host C compiler was detected for {name}"
            );
        }
        return None;
    };
    let c_path = root.join(format!("{name}.c"));
    let bin = root.join(name);
    crate::build::assert_inside_test_root(root, &c_path);
    crate::build::assert_inside_test_root(root, &bin);
    fs::write(&c_path, src).unwrap();
    let mut cmd = Command::new(&cc.cc_path);
    cmd.args(["-O1", "-o"])
        .arg(&bin)
        .arg(&c_path)
        .args(extra_flags);
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "failed to compile {name}.c: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(bin)
}

/// Builds a rich Blot ABI 8 Wasm module exercising all 18 exports, tricky names (`\x01a`, `\x017`,
/// `a??=b`, `""`), re-exported imports, folded negative I32, folded RefNull, mutable I32/I64/F32/F64,
/// `blot:allocate`, `blot:reset`, `blot:finish_invocation`, recursion, large frame, and callbacks.
fn build_abi18_wasm() -> Vec<u8> {
    // Types:
    // 0: (i32) -> (i32)
    // 1: () -> (i32)
    // 2: () -> ()
    // 3: (i64) -> (i64)
    // 4: (f32) -> (f32)
    // 5: (f64) -> (f64)
    // 6: (externref) -> (externref)
    let mut imp_p = vec![3];
    // imp 0: "" :: "" () -> ()  (tests empty import module & name non-null .rodata pointers!)
    enc_str(&mut imp_p, "");
    enc_str(&mut imp_p, "");
    imp_p.push(0);
    leb_u32(&mut imp_p, 2);
    // imp 1: "\x01a" :: "a??=b" (i32) -> (i32) (tricky escape + trigraph import AND re-exported!)
    enc_str(&mut imp_p, "\x01a");
    enc_str(&mut imp_p, "a??=b");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);
    // imp 2: "env" :: "cb_f32" (f32) -> (f32) (re-exported import!)
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "cb_f32");
    imp_p.push(0);
    leb_u32(&mut imp_p, 4);

    // Globals:
    // g0: mut i32 = 100 (pinned!)
    // g1: immut i32 = -5 (folded negative I32!)
    // g2: immut externref = null (folded RefNull!)
    // g3: mut f32 = 1.5
    // g4: mut i64 = 0x1122334455667788
    // g5: mut f64 = 2.25
    // g6: immut i32 = 7 (non-exported)
    let mut glob_p = vec![6];
    glob_p.extend_from_slice(&[0x7f, 1, 0x41]);
    leb_i32(&mut glob_p, 100);
    glob_p.push(0x0b);
    glob_p.extend_from_slice(&[0x7f, 0, 0x41]);
    leb_i32(&mut glob_p, -5);
    glob_p.push(0x0b);
    glob_p.extend_from_slice(&[0x6f, 0, 0xd0, 0x6f, 0x0b]);
    glob_p.extend_from_slice(&[0x7d, 1, 0x43]);
    glob_p.extend_from_slice(&1.5f32.to_bits().to_le_bytes());
    glob_p.push(0x0b);
    glob_p.extend_from_slice(&[0x7e, 1, 0x42]);
    super::tests_unit::leb_i64(&mut glob_p, 0x1122_3344_5566_7788);
    glob_p.push(0x0b);
    glob_p.extend_from_slice(&[0x7c, 1, 0x44]);
    glob_p.extend_from_slice(&2.25f64.to_bits().to_le_bytes());
    glob_p.push(0x0b);

    // f3 (type 0): blot:allocate(arg): if arg == -99 trap unreachable; if arg == -50 call imp 1 (reentrant callback!);
    //              else g0 += arg; return g0
    let mut b3 = Vec::new();
    b3.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b3, -99);
    b3.extend_from_slice(&[0x46, 0x04, 0x40, 0x00, 0x0b]);
    b3.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b3, -50);
    b3.extend_from_slice(&[
        0x46, 0x04, 0x7f, 0x41, 7, 0x10, 1, 0x05, 0x23, 0, 0x20, 0, 0x6a, 0x24, 0, 0x23, 0, 0x0b,
    ]);

    // f4 (type 0): blot:reset(arg): g0 = arg; return arg + 1
    let b4 = vec![0x20, 0, 0x24, 0, 0x20, 0, 0x41, 1, 0x6a];

    // f5 (type 1): blot:finish_invocation() -> g0
    let b5 = vec![0x23, 0];

    // f6 (type 0): call_cb(arg) -> calls imp 1(arg), also touches g0 so g0 is hottest & pinned in R12!
    let b6 = vec![0x23, 0, 0x41, 1, 0x6a, 0x24, 0, 0x20, 0, 0x10, 1];

    // f7 (type 0): recurse(n): if n <= 0 return 0 else return 1 + recurse(n - 1)
    let b7 = vec![
        0x20, 0, 0x41, 0, 0x4c, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 7, 0x41,
        1, 0x6a, 0x0b,
    ];

    // f8 (type 5): mxcsr_check(x: f64) -> (x + 1e-320) * 1.0 (verifies subnormals & round-to-nearest-even under 0x1f80!)
    let mut b8 = vec![0x20, 0, 0x44];
    b8.extend_from_slice(&f64::from_bits(1).to_bits().to_le_bytes()); // smallest subnormal 5e-324!
    b8.push(0xa0); // f64.add (DAZ/FTZ would flush to 0!)

    // f9 (type 3): echo_i64(x) -> x ^ 1
    let b9 = vec![0x20, 0, 0x42, 1, 0x85];
    // f10 (type 4): echo_f32(x) -> x + 1.0f32
    let mut b10 = vec![0x20, 0, 0x43];
    b10.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
    b10.push(0x92);
    // f11 (type 6): echo_ref(r) -> r
    let b11 = vec![0x20, 0];
    // f12 (type 2): void_fn() -> ()
    let b12 = vec![];
    // f13 (type 0): large_frame() with 200 live i64 locals -> forces > 1.6 KiB spill frame
    let mut b13 = Vec::new();
    for i in 1..=200u32 {
        b13.push(0x20);
        b13.push(0);
        b13.push(0xac);
        b13.push(0x42);
        super::tests_unit::leb_i64(&mut b13, i as i64);
        b13.push(0x7c);
        b13.push(0x21);
        leb_u32(&mut b13, i);
    }
    b13.extend_from_slice(&[0x42, 0]);
    for i in 1..=200u32 {
        b13.push(0x20);
        leb_u32(&mut b13, i);
        b13.push(0x7c);
    }
    b13.push(0xa7);

    wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]),
            (&[], &[0x7f]),
            (&[], &[]),
            (&[0x7e], &[0x7e]),
            (&[0x7d], &[0x7d]),
            (&[0x7c], &[0x7c]),
            (&[0x6f], &[0x6f]),
        ]),
        section(2, &imp_p),
        func_sec(&[0, 0, 1, 0, 0, 5, 3, 4, 6, 2, 0]), // f3..f13
        section(5, &[1, 0x01, 1, 4]),                 // memory min=1 max=4
        section(6, &glob_p),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("blot:allocate", 0, 3),
            ("blot:reset", 0, 4),
            ("blot:finish_invocation", 0, 5),
            ("call_cb", 0, 6),
            ("recurse", 0, 7),
            ("mxcsr_check", 0, 8),
            ("echo_i64", 0, 9),
            ("echo_f32", 0, 10),
            ("echo_ref", 0, 11),
            ("void_fn", 0, 12),
            ("large_frame", 0, 13),
            ("", 0, 12),         // empty export name!
            ("\x01a", 0, 3),     // control + hex digit!
            ("\x017", 0, 4),     // control + octal digit!
            ("a??=b", 0, 5),     // C trigraph!
            ("reexp_i32", 0, 1), // re-exported import (i32)->(i32)
            ("reexp_f32", 0, 2), // re-exported import (f32)->(f32)
            ("g_mut_i32", 3, 0),
            ("g_neg_i32", 3, 1),
            ("g_ref_null", 3, 2),
            ("g_mut_f32", 3, 3),
            ("g_mut_i64", 3, 4),
            ("g_mut_f64", 3, 5),
        ]),
        code_sec(&[
            enc_body(&[], &b3),
            enc_body(&[], &b4),
            enc_body(&[], &b5),
            enc_body(&[], &b6),
            enc_body(&[], &b7),
            enc_body(&[], &b8),
            enc_body(&[], &b9),
            enc_body(&[], &b10),
            enc_body(&[], &b11),
            enc_body(&[], &b12),
            enc_body(&[(200, 0x7e)], &b13),
        ]),
    ])
}

#[test]
fn abi18_full_behavioral_parity_reentry_mxcsr_pthread_and_c_deviations() {
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-abi18");
    let wasm = build_abi18_wasm();
    let x64_so = root.0.join("abi18_x64.so");
    crate::build::assert_inside_test_root(&root.0, &x64_so);
    let so_bytes = compile_wasm_to_x64_so(
        &wasm,
        &X64CompileOptions {
            isa: TargetIsa::default(),
            jobs: 2,
            deadline: None,
            opt: true,
            verify: true,
            hooks: Default::default(),
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(&x64_so, &so_bytes).unwrap();

    // Also compile a 0-length active data OOB module to test wt_initialize 0-len OOB trap (2)!
    let oob_init_wasm = wasm_module(&[
        section(5, &[1, 0x01, 0, 1]),
        section(11, &[1, 0x00, 0x41, 1, 0x0b, 0]), // active data off=1, len=0 on 0-page memory -> TRAP_OOB (2)!
    ]);
    let oob_init_so = root.0.join("oob_init.so");
    fs::write(
        &oob_init_so,
        compile_wasm_to_x64_so(&oob_init_wasm, &X64CompileOptions::default()).unwrap(),
    )
    .unwrap();

    // Also compile a 0-capacity memory + empty global export module (for Deviation #2 & empty-global `(NULL, 0)`).
    let mut z_glob = vec![1, 0x7f, 0x00, 0x41];
    leb_i32(&mut z_glob, -5);
    z_glob.push(0x0b);
    let zero_mem_wasm = wasm_module(&[
        section(5, &[1, 0x01, 0, 0]),
        section(6, &z_glob),
        export_sec(&[("blot:memory", 2, 0), ("", 3, 0)]),
    ]);
    let zero_x64_so = root.0.join("zero_x64.so");
    fs::write(
        &zero_x64_so,
        compile_wasm_to_x64_so(&zero_mem_wasm, &X64CompileOptions::default()).unwrap(),
    )
    .unwrap();

    // Also compile with `--backend c` to verify C-vs-x64 parity & all 6 documented deviations!
    let c_cache = root.0.join("c-cache");
    let c_so = root.0.join("abi18_c.so");
    let zero_c_so = root.0.join("zero_c.so");
    let has_c = crate::cgen::build_c_blot_cdylib(
        &wasm,
        &c_cache,
        "native",
        Some(2),
        crate::cgen::COpt::Auto,
        Some(Duration::from_secs(120)),
        &c_so,
    )
    .is_ok()
        && crate::cgen::build_c_blot_cdylib(
            &zero_mem_wasm,
            &c_cache,
            "native",
            Some(2),
            crate::cgen::COpt::Auto,
            Some(Duration::from_secs(120)),
            &zero_c_so,
        )
        .is_ok();

    let Some(driver) = compile_c_test_bin(
        &root.0,
        "abi18_driver",
        ABI18_DRIVER_C,
        &["-ldl", "-pthread"],
    ) else {
        return;
    };
    let mut cmd = Command::new(&driver);
    cmd.arg(&x64_so).arg(&oob_init_so).arg(&zero_x64_so);
    if has_c {
        cmd.arg(&c_so).arg(&zero_c_so);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "abi18 driver failed ({}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

const ABI18_DRIVER_C: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <stddef.h>
#include <string.h>
#include <dlfcn.h>
#include <pthread.h>
#include <xmmintrin.h>

typedef int32_t (*host_fn_t)(void *, void *, uint32_t, uint64_t, uint64_t, uint64_t *);
typedef uint32_t (*fn_u32)(void);
typedef int32_t (*fn_trap_msg)(uint32_t, const uint8_t **, size_t *); // uint32_t to pass dirty upper edi!
typedef int32_t (*fn_imp_name)(uint32_t, const uint8_t **, size_t *, const uint8_t **, size_t *);
typedef void *(*fn_new)(void *, host_fn_t, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_call_i32)(void *, int32_t, int32_t *, uint8_t *);
typedef int32_t (*fn_call_void)(void *, int32_t *, uint8_t *);
typedef void (*fn_mem_data)(void *, uint8_t **, size_t *);
typedef int64_t (*fn_mem_grow)(void *, uint64_t);
typedef int32_t (*fn_get_global)(void *, const uint8_t *, size_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_set_global)(void *, const uint8_t *, size_t, uint64_t);

#define CHECK(cond, msg, ...) do { if (!(cond)) { fprintf(stderr, "FAIL: " msg "\n", ##__VA_ARGS__); exit(1); } } while (0)

typedef struct {
    fn_call_export call_export;
    fn_call_i32 call_alloc;
    fn_call_i32 active_alloc;
    fn_call_i32 active_reset;
    fn_mem_data active_mem_data;
    fn_mem_grow mem_grow;
    fn_get_global active_get_global;
    fn_set_global set_global;
    fn_drop drop;
    void *h;
    int mode;
    int depth_seen;
    int hit_depth_trap;
    int saw_active_trap;
    uint32_t observed_cb_mxcsr;
} CbCtx;

// Helper that deliberately clobbers R14 and R15 before calling blot_aot_active_allocate!
__attribute__((noinline))
static int32_t call_active_with_clobbered_r14_r15(fn_call_i32 fn, void *ac, int32_t arg, int32_t *res, uint8_t *tr) {
    uint64_t ret = (uint64_t)(uintptr_t)fn;
    __asm__ volatile(
        "push %%r14\n\t"
        "push %%r15\n\t"
        "mov $0xdeadbeef14141414, %%r14\n\t"
        "mov $0xdeadbeef15151515, %%r15\n\t"
        "call *%%rax\n\t"
        "pop %%r15\n\t"
        "pop %%r14\n\t"
        : "+a"(ret), "+D"(ac), "+S"(arg), "+d"(res), "+c"(tr)
        :
        : "r8", "r9", "r10", "r11", "memory"
    );
    return (int32_t)ret;
}

// Helper verifying blot_aot_mem_grow preserves caller's R13, R14, R15!
__attribute__((noinline))
static int64_t call_mem_grow_check_callee_regs(fn_mem_grow fn, void *h, uint64_t delta, int *ok_out) {
    uint64_t after[3] = {0, 0, 0};
    uint64_t *after_p = after;
    uint64_t ret = (uint64_t)(uintptr_t)fn;
    __asm__ volatile(
        "push %%r13\n\t"
        "push %%r14\n\t"
        "push %%r15\n\t"
        "push %%rdx\n\t"
        "mov $0x1313131313131313, %%r13\n\t"
        "mov $0x1414141414141414, %%r14\n\t"
        "mov $0x1515151515151515, %%r15\n\t"
        "call *%%rax\n\t"
        "pop %%rcx\n\t"
        "mov %%r13, 0(%%rcx)\n\t"
        "mov %%r14, 8(%%rcx)\n\t"
        "mov %%r15, 16(%%rcx)\n\t"
        "pop %%r15\n\t"
        "pop %%r14\n\t"
        "pop %%r13\n\t"
        : "+a"(ret), "+d"(after_p), "+D"(h), "+S"(delta)
        :
        : "rcx", "r8", "r9", "r10", "r11", "memory"
    );
    *ok_out = (after[0] == 0x1313131313131313ULL &&
               after[1] == 0x1414141414141414ULL &&
               after[2] == 0x1515151515151515ULL);
    return (int64_t)ret;
}

static int32_t test_cb(void *u, void *ac, uint32_t idx, uint64_t tok, uint64_t arg, uint64_t *res) {
    (void)tok;
    CbCtx *c = (CbCtx *)u;
    c->observed_cb_mxcsr = _mm_getcsr();
    if (c->mode == 1) {
        // Test callback-time defensive guards:
        // (a) top-level call_export on same handle -> rc=2, trap=19 (TRAP_HOST)
        uint64_t r = 0xaaaa; uint8_t tr = 0;
        CHECK(c->call_export(c->h, (const uint8_t *)"void_fn", 7, 0, &r, &tr) == 2 && tr == 19 && r == 0xaaaa,
              "in_call reentry guard failed");
        // (b) blot_aot_mem_grow during in_call -> -1
        CHECK(c->mem_grow(c->h, 0) == -1, "mem_grow during in_call must return -1");
        // (c) blot_aot_set_global during in_call -> 1
        CHECK(c->set_global(c->h, (const uint8_t *)"g_mut_i32", 9, 999) == 1, "set_global during in_call must return 1");
        // (d) blot_aot_drop during in_call -> must be a safe no-op!
        c->drop(c->h);
        // (e) active_mem_data & active_get_global work & observe synced pinned global!
        uint8_t *mp = NULL; size_t ml = 0;
        c->active_mem_data(ac, &mp, &ml);
        CHECK(mp != NULL && ml >= 65536, "active_mem_data failed");
        uint64_t gv = 0; uint8_t gk = 255;
        CHECK(c->active_get_global(ac, (const uint8_t *)"g_mut_i32", 9, &gv, &gk) == 0 && gk == 0,
              "active_get_global failed");
        // (f) active_allocate with deliberately clobbered R14 & R15!
        int32_t ares = 0; uint8_t atr = 0;
        CHECK(call_active_with_clobbered_r14_r15(c->active_alloc, ac, 10, &ares, &atr) == 0 && ares == (int32_t)gv + 10,
              "active_allocate after R14/R15 clobber failed: ares=%d gv=%llu", ares, (unsigned long long)gv);
        // (g) active_allocate triggering guest trap (-99) writes *trap_out=1 & leaves *res_out untouched!
        int32_t untouched = 0x55aa55aa; uint8_t atr2 = 0;
        CHECK(c->active_alloc(ac, -99, &untouched, &atr2) == 1 && atr2 == 1 && untouched == 0x55aa55aa,
              "active_allocate trap failed: atr2=%u untouched=%x", atr2, untouched);
        c->saw_active_trap = 1;
        *res = (uint64_t)(uint32_t)ares;
        return 0;
    } else if (c->mode == 2) {
        // Exhaust MAX_ACTIVE_REENTRY_DEPTH = 16!
        c->depth_seen++;
        int32_t ares = 0x7777; uint8_t atr = 0;
        int32_t rc = c->active_alloc(ac, -50, &ares, &atr);
        if (rc == 1 && atr == 8 && ares == 0x7777) {
            c->hit_depth_trap = 1;
            *res = (uint64_t)c->depth_seen;
            return 0;
        }
        CHECK(rc == 0, "unexpected active_depth rc=%d tr=%u depth=%d", rc, atr, c->depth_seen);
        *res = (uint64_t)(uint32_t)ares;
        return 0;
    }
    // Default mode 0: echo arg + idx * 10
    if (idx == 2) {
        // f32 echo: return arg bits directly
        *res = arg;
    } else {
        *res = arg + idx * 10;
    }
    return 0;
}

typedef struct {
    fn_call_export call_export;
    void *h;
    int rc;
} PthreadArg;

static void *small_pthread_fn(void *p) {
    PthreadArg *a = (PthreadArg *)p;
    uint64_t res = 0; uint8_t tr = 0;
    // Deep recursion on a 128 KiB host pthread runs on the 8 MiB guest stack and traps cleanly (code 8)!
    if (a->call_export(a->h, (const uint8_t *)"recurse", 7, 10000, &res, &tr) != 0 || res != 10000) {
        fprintf(stderr, "128 KiB pthread shallow-10000 recursion failed: res=%llu tr=%u\n", (unsigned long long)res, tr);
        a->rc = 1; return NULL;
    }
    if (a->call_export(a->h, (const uint8_t *)"recurse", 7, 1000000, &res, &tr) != 1 || tr != 8) {
        fprintf(stderr, "128 KiB pthread deep recursion did not trap 8: tr=%u\n", tr);
        a->rc = 2; return NULL;
    }
    if (a->call_export(a->h, (const uint8_t *)"large_frame", 11, 5, &res, &tr) != 0) {
        fprintf(stderr, "128 KiB pthread large_frame failed: tr=%u\n", tr);
        a->rc = 3; return NULL;
    }
    a->rc = 0;
    return NULL;
}

static void check_maps_perms(const char *so_path) {
    FILE *f = fopen("/proc/self/maps", "r");
    CHECK(f != NULL, "fopen /proc/self/maps");
    char line[512];
    int saw_r = 0, saw_rx = 0, saw_rw = 0, saw_rwx = 0;
    while (fgets(line, sizeof(line), f)) {
        if (strstr(line, so_path)) {
            char perms[8] = {0};
            if (sscanf(line, "%*x-%*x %4s", perms) == 1) {
                if (strcmp(perms, "r--p") == 0) saw_r = 1;
                if (strcmp(perms, "r-xp") == 0) saw_rx = 1;
                if (strcmp(perms, "rw-p") == 0) saw_rw = 1;
                if (perms[1] == 'w' && perms[2] == 'x') saw_rwx = 1;
            }
        }
    }
    fclose(f);
    CHECK(saw_r && saw_rx && saw_rw && !saw_rwx,
          "ELF maps perms unexpected: r=%d rx=%d rw=%d rwx=%d", saw_r, saw_rx, saw_rw, saw_rwx);
}

int main(int argc, char **argv) {
    CHECK(argc >= 3, "need x64_so and oob_init_so");
    void *dl = dlopen(argv[1], RTLD_NOW);
    CHECK(dl != NULL, "dlopen x64: %s", dlerror());
    check_maps_perms(argv[1]);

    fn_u32 abi_ver = (fn_u32)dlsym(dl, "blot_aot_abi_version");
    fn_trap_msg trap_msg = (fn_trap_msg)dlsym(dl, "blot_aot_trap_message");
    fn_u32 imp_cnt = (fn_u32)dlsym(dl, "blot_aot_import_count");
    fn_imp_name imp_name = (fn_imp_name)dlsym(dl, "blot_aot_import_name");
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export call_export = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    fn_call_i32 call_alloc = (fn_call_i32)dlsym(dl, "blot_aot_call_allocate");
    fn_call_i32 call_reset = (fn_call_i32)dlsym(dl, "blot_aot_call_reset");
    fn_call_void call_fin = (fn_call_void)dlsym(dl, "blot_aot_call_finish_invocation");
    fn_mem_data mem_data = (fn_mem_data)dlsym(dl, "blot_aot_mem_data");
    fn_mem_grow mem_grow = (fn_mem_grow)dlsym(dl, "blot_aot_mem_grow");
    fn_get_global get_global = (fn_get_global)dlsym(dl, "blot_aot_get_global");
    fn_set_global set_global = (fn_set_global)dlsym(dl, "blot_aot_set_global");
    fn_mem_data active_mem_data = (fn_mem_data)dlsym(dl, "blot_aot_active_mem_data");
    fn_call_i32 active_alloc = (fn_call_i32)dlsym(dl, "blot_aot_active_allocate");
    fn_call_i32 active_reset = (fn_call_i32)dlsym(dl, "blot_aot_active_reset");
    fn_get_global active_get_global = (fn_get_global)dlsym(dl, "blot_aot_active_get_global");
    CHECK(abi_ver && trap_msg && imp_cnt && imp_name && b_new && b_drop && call_export &&
          call_alloc && call_reset && call_fin && mem_data && mem_grow && get_global && set_global &&
          active_mem_data && active_alloc && active_reset && active_get_global,
          "all 18 symbols must resolve via dlsym");

    // 1. abi_version & trap_message (dirty upper edi bits + codes 0, 1..22, 23, NULL outs)
    CHECK(abi_ver() == 1, "abi_version != 1");
    const uint8_t *p = NULL; size_t l = 0;
    CHECK(trap_msg(0, &p, &l) == 1 && p == NULL, "trap 0 should fail");
    CHECK(trap_msg(23, &p, &l) == 1 && p == NULL, "trap 23 should fail");
    CHECK(trap_msg(0xdead0002u, &p, &l) == 0 && l == strlen("Trap: out of bounds") && memcmp(p, "Trap: out of bounds", l) == 0,
          "dirty edi trap_message(2) mismatch");
    CHECK(trap_msg(22, &p, &l) == 0 && l == strlen("Unsupported CPU ISA") && memcmp(p, "Unsupported CPU ISA", l) == 0,
          "trap_message(22) mismatch");
    CHECK(trap_msg(1, NULL, NULL) == 0, "trap_message NULL outs");

    // 2. import_count & import_name (empty string non-null ptr, tricky bytes, OOB idx)
    CHECK(imp_cnt() == 3, "import_count != 3");
    const uint8_t *mp = NULL, *np = NULL; size_t ml = 99, nl = 99;
    CHECK(imp_name(0, &mp, &ml, &np, &nl) == 0 && mp != NULL && ml == 0 && np != NULL && nl == 0,
          "empty import strings must return non-null .rodata pointers");
    CHECK(imp_name(1, &mp, &ml, &np, &nl) == 0 && ml == 2 && memcmp(mp, "\x01""a", 2) == 0 && nl == 5 && memcmp(np, "a??=b", 5) == 0,
          "tricky import name mismatch");
    CHECK(imp_name(3, NULL, NULL, NULL, NULL) == 1, "import_name OOB");

    // 3. Null-handle defensive guards
    uint64_t r64 = 0x1234; int32_t r32 = 0x1234; uint8_t tr = 0;
    CHECK(call_export(NULL, (const uint8_t *)"void_fn", 7, 0, &r64, &tr) == 3 && r64 == 0x1234 && tr == 0, "null h call_export");
    CHECK(call_alloc(NULL, 1, &r32, &tr) == 2 && tr == 19 && r32 == 0x1234, "null h call_alloc");
    CHECK(call_reset(NULL, 1, &r32, &tr) == 2 && tr == 19, "null h call_reset");
    CHECK(call_fin(NULL, &r32, &tr) == 2 && tr == 19, "null h call_fin");
    CHECK(mem_grow(NULL, 0) == -1, "null h mem_grow");
    CHECK(get_global(NULL, (const uint8_t *)"g_mut_i32", 9, &r64, &tr) == 1, "null h get_global");
    CHECK(set_global(NULL, (const uint8_t *)"g_mut_i32", 9, 1) == 1, "null h set_global");
    CHECK(active_alloc(NULL, 1, &r32, &tr) == 2 && tr == 19, "null ac active_alloc");
    CHECK(active_get_global(NULL, (const uint8_t *)"g_mut_i32", 9, &r64, &tr) == 1, "null ac active_get_global");
    b_drop(NULL);
    mem_data(NULL, NULL, NULL);
    active_mem_data(NULL, NULL, NULL);

    // 4. host_fn == NULL -> TRAP_HOST (19) on import call
    void *h_nohost = b_new(NULL, NULL, 0, &tr);
    CHECK(h_nohost != NULL, "new with NULL host_fn");
    tr = 0;
    CHECK(call_export(h_nohost, (const uint8_t *)"call_cb", 7, 5, &r64, &tr) == 2 && tr == 19, "NULL host_fn must trap 19");
    b_drop(h_nohost);

    // 5. Normal instance with CbCtx
    CbCtx cb;
    memset(&cb, 0, sizeof(cb));
    cb.call_export = call_export; cb.call_alloc = call_alloc;
    cb.active_alloc = active_alloc; cb.active_reset = active_reset;
    cb.active_mem_data = active_mem_data; cb.mem_grow = mem_grow;
    cb.active_get_global = active_get_global; cb.set_global = set_global; cb.drop = b_drop;
    void *h = b_new(&cb, test_cb, 0, &tr);
    CHECK(h != NULL, "b_new failed tr=%u", tr);
    cb.h = h;

    // 6. Export widths, tricky names, (NULL, 0) empty match, re-exported imports
    CHECK(call_export(h, NULL, 0, 0, &r64, &tr) == 0 && r64 == 0, "(NULL, 0) must match empty export");
    CHECK(call_export(h, NULL, 5, 0, &r64, &tr) == 3, "(NULL, 5) must return 3");
    CHECK(call_export(h, (const uint8_t *)"\x01""a", 2, 5, &r64, &tr) == 0 && r64 == 105, "tricky \\x01a export");
    CHECK(call_export(h, (const uint8_t *)"\x01""7", 2, 200, &r64, &tr) == 0 && r64 == 201, "tricky \\x017 export");
    CHECK(call_export(h, (const uint8_t *)"a??=b", 5, 0, &r64, &tr) == 0 && r64 == 200, "tricky a??=b export");
    CHECK(call_export(h, (const uint8_t *)"reexp_i32", 9, 0xdead000000000005ULL, &r64, &tr) == 0 && r64 == 15,
          "re-exported i32 import width/value: %llu", (unsigned long long)r64);
    float f_in = 3.25f; uint32_t f_bits = 0; memcpy(&f_bits, &f_in, 4);
    CHECK(call_export(h, (const uint8_t *)"reexp_f32", 9, (uint64_t)f_bits, &r64, &tr) == 0 && r64 == (uint64_t)f_bits,
          "re-exported f32 import");
    CHECK(call_export(h, (const uint8_t *)"echo_i64", 8, 0xfedcba9876543210ULL, &r64, &tr) == 0 && r64 == 0xfedcba9876543211ULL,
          "echo_i64");
    CHECK(call_export(h, (const uint8_t *)"echo_ref", 8, 0x123456789abcdef0ULL, &r64, &tr) == 0 && r64 == 0x123456789abcdef0ULL,
          "echo_ref");
    CHECK(call_alloc(h, 7, &r32, &tr) == 0 && r32 == 207, "call_alloc");
    CHECK(call_reset(h, 50, &r32, &tr) == 0 && r32 == 51, "call_reset");
    CHECK(call_fin(h, &r32, &tr) == 0 && r32 == 50, "call_fin");

    // 7. Globals: folded negative I32 (32-bit zero-ext!), folded RefNull (val=0, kind=4),
    //    (NULL, 0) non-matching global safely returns 1 without deref, dirty 64-bit set_global -> 32-bit zero-ext!
    uint64_t gv = 0; uint8_t gk = 255;
    CHECK(get_global(h, (const uint8_t *)"g_neg_i32", 9, &gv, &gk) == 0 && gk == 0 && gv == 0x00000000fffffffbULL,
          "folded negative i32 must be 32-bit zero-ext, got %016llx", (unsigned long long)gv);
    CHECK(get_global(h, NULL, 0, &gv, &gk) == 1, "(NULL, 0) missing global must return 1 without deref");
    CHECK(get_global(h, NULL, 5, &gv, &gk) == 1, "(NULL, 5) must return 1 without deref");
    CHECK(get_global(h, (const uint8_t *)"g_ref_null", 10, &gv, &gk) == 0 && gk == 4 && gv == 0,
          "folded RefNull global failed: gv=%llu gk=%u", (unsigned long long)gv, gk);
    CHECK(set_global(h, (const uint8_t *)"g_mut_i32", 9, 0xdeadbeef12345678ULL) == 0, "set_global i32");
    CHECK(get_global(h, (const uint8_t *)"g_mut_i32", 9, &gv, &gk) == 0 && gk == 0 && gv == 0x0000000012345678ULL,
          "dirty set_global i32 must read back 32-bit zero-ext: %016llx", (unsigned long long)gv);
    CHECK(set_global(h, (const uint8_t *)"g_mut_f32", 9, 0xdeadbeef3f800000ULL) == 0, "set_global f32");
    CHECK(get_global(h, (const uint8_t *)"g_mut_f32", 9, &gv, &gk) == 0 && gk == 2 && gv == 0x000000003f800000ULL,
          "dirty set_global f32 must read back 32-bit zero-ext: %016llx", (unsigned long long)gv);
    CHECK(set_global(h, (const uint8_t *)"g_neg_i32", 9, 1) == 1, "set_global immutable must return 1");

    // 8. blot_aot_mem_grow preserving R13/R14/R15!
    int regs_ok = 0;
    int64_t oldp = call_mem_grow_check_callee_regs(mem_grow, h, 2, &regs_ok);
    CHECK(oldp == 1 && regs_ok == 1, "mem_grow failed: oldp=%lld regs_ok=%d", (long long)oldp, regs_ok);
    CHECK(mem_grow(h, 0x100000000ULL) == -1, "mem_grow 64-bit delta must not truncate to 0!");

    // 9. Host MXCSR isolation (FZ=1<<15 | DAZ=1<<6 | round-toward-zero=3<<13 | masks=0x1f80 = 0xffc0)
    uint32_t orig_mxcsr = _mm_getcsr();
    uint32_t custom_mxcsr = 0x1f80 | (3u << 13) | (1u << 15) | (1u << 6);
    _mm_setcsr(custom_mxcsr);
    CHECK(call_export(h, (const uint8_t *)"mxcsr_check", 11, 0, &r64, &tr) == 0 && r64 == 1ULL,
          "guest must run with MXCSR=0x1f80 (no DAZ/FTZ): got %016llx", (unsigned long long)r64);
    CHECK((_mm_getcsr() & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored after normal call!");

    // 10. Callback reentry, R14/R15 clobber, drop-guard, in_call guards, MXCSR preserved in callback!
    cb.mode = 1;
    CHECK(call_export(h, (const uint8_t *)"call_cb", 7, 1, &r64, &tr) == 0 && cb.saw_active_trap == 1,
          "reentry/R14+R15/drop-guard test failed");
    CHECK((cb.observed_cb_mxcsr & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored during callback!");
    CHECK((_mm_getcsr() & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored after callback call!");

    // 11. Active reentry depth 16 exhaustion -> TRAP_CALL_STACK (8)
    cb.mode = 2; cb.depth_seen = 0; cb.hit_depth_trap = 0;
    CHECK(call_export(h, (const uint8_t *)"call_cb", 7, 1, &r64, &tr) == 0 && cb.hit_depth_trap == 1 && cb.depth_seen == 17,
          "active_depth=16 exhaustion failed: hit=%d depth=%d", cb.hit_depth_trap, cb.depth_seen);
    _mm_setcsr(orig_mxcsr);

    // 12. 128 KiB host pthread recursion & large-frame check
    pthread_t th; pthread_attr_t attr;
    pthread_attr_init(&attr);
    CHECK(pthread_attr_setstacksize(&attr, 128 * 1024) == 0, "pthread_attr_setstacksize");
    PthreadArg pa = { call_export, h, -1 };
    CHECK(pthread_create(&th, &attr, small_pthread_fn, &pa) == 0, "pthread_create");
    pthread_attr_destroy(&attr);
    pthread_join(th, NULL);
    CHECK(pa.rc == 0, "128 KiB pthread test failed: rc=%d", pa.rc);

    b_drop(h);
    dlclose(dl);

    // 13. wt_initialize 0-len active data OOB -> TRAP_OOB (2)
    void *dl_oob = dlopen(argv[2], RTLD_NOW);
    CHECK(dl_oob != NULL, "dlopen oob_init: %s", dlerror());
    fn_new oob_new = (fn_new)dlsym(dl_oob, "blot_aot_new");
    tr = 0;
    CHECK(oob_new(NULL, NULL, 0, &tr) == NULL && tr == 2, "wt_initialize 0-len OOB must fail with trap 2, got %u", tr);
    dlclose(dl_oob);

    // 13b. 0-capacity x64 memory (mem_base == NULL, mem_len == 0) + empty global name (NULL, 0) match!
    CHECK(argc >= 4, "need zero_x64_so");
    void *dl_z = dlopen(argv[3], RTLD_NOW);
    CHECK(dl_z != NULL, "dlopen zero_x64: %s", dlerror());
    fn_new z_new = (fn_new)dlsym(dl_z, "blot_aot_new");
    fn_drop z_drop = (fn_drop)dlsym(dl_z, "blot_aot_drop");
    fn_mem_data z_mem = (fn_mem_data)dlsym(dl_z, "blot_aot_mem_data");
    fn_get_global z_getg = (fn_get_global)dlsym(dl_z, "blot_aot_get_global");
    void *hz = z_new(NULL, NULL, 0, &tr);
    CHECK(hz != NULL, "zero_x64 new failed");
    uint8_t *zptr = (uint8_t *)0x1; size_t zlen = 99;
    z_mem(hz, &zptr, &zlen);
    CHECK(zptr == NULL && zlen == 0, "x64 0-cap memory must have NULL base and 0 len");
    CHECK(z_getg(hz, NULL, 0, &gv, &gk) == 0 && gk == 0 && gv == 0x00000000fffffffbULL,
          "empty global name (NULL, 0) match failed");
    z_drop(hz);
    dlclose(dl_z);

    // 14. Optional C-vs-x64 parity & all 6 documented deviations check when C .so provided
    if (argc >= 6) {
        void *dlc = dlopen(argv[4], RTLD_NOW);
        CHECK(dlc != NULL, "dlopen c_so: %s", dlerror());
        fn_trap_msg c_tm = (fn_trap_msg)dlsym(dlc, "blot_aot_trap_message");
        fn_new c_new = (fn_new)dlsym(dlc, "blot_aot_new");
        fn_drop c_drop = (fn_drop)dlsym(dlc, "blot_aot_drop");
        fn_call_export c_call = (fn_call_export)dlsym(dlc, "blot_aot_call_export");
        fn_get_global c_getg = (fn_get_global)dlsym(dlc, "blot_aot_get_global");
        // Verify verbatim parity of all trap messages 1..=22 between C and x64!
        void *dlx = dlopen(argv[1], RTLD_NOW);
        fn_trap_msg x_tm = (fn_trap_msg)dlsym(dlx, "blot_aot_trap_message");
        for (uint32_t code = 1; code <= 22; code++) {
            const uint8_t *xp = NULL, *cp = NULL; size_t xl = 0, cl = 0;
            CHECK(x_tm(code, &xp, &xl) == 0 && c_tm(code, &cp, &cl) == 0, "trap_msg(%u) status", code);
            CHECK(xl == cl && memcmp(xp, cp, xl) == 0, "trap_msg(%u) text mismatch between x64 and C", code);
        }
        dlclose(dlx);
        void *hc = c_new(NULL, NULL, 0, &tr);
        CHECK(hc != NULL, "C new");
        // Tricky names match identically between C and x64!
        CHECK(c_call(hc, (const uint8_t *)"\x01""a", 2, 5, &r64, &tr) == 0 && r64 == 105, "C tricky \\x01a");
        CHECK(c_call(hc, (const uint8_t *)"\x01""7", 2, 200, &r64, &tr) == 0 && r64 == 201, "C tricky \\x017");
        CHECK(c_call(hc, (const uint8_t *)"a??=b", 5, 0, &r64, &tr) == 0 && r64 == 200, "C tricky a??=b");
        // Both x64 (folded RefNull) and C (Ctx-stored RefNull) return (rc=0, val=0, kind=4)!
        CHECK(c_getg(hc, (const uint8_t *)"g_ref_null", 10, &gv, &gk) == 0 && gk == 4 && gv == 0,
              "C RefNull global parity mismatch: gv=%llu gk=%u", (unsigned long long)gv, gk);
        // Both x64 and C now save/set/restore MXCSR=0x1f80 around guest execution (preserving subnormals!).
        uint32_t saved_mxcsr = _mm_getcsr();
        uint32_t custom_c_mxcsr = 0x1f80 | (1u << 15) | (1u << 6);
        _mm_setcsr(custom_c_mxcsr);
        CHECK(c_call(hc, (const uint8_t *)"mxcsr_check", 11, 0, &r64, &tr) == 0 && r64 == 1ULL,
              "C backend must run with MXCSR=0x1f80 (subnormals preserved): expected 1, got %llu", (unsigned long long)r64);
        CHECK((_mm_getcsr() & 0xffc0u) == (custom_c_mxcsr & 0xffc0u),
              "C backend must restore host MXCSR after call");
        _mm_setcsr(saved_mxcsr);
        c_drop(hc);
        dlclose(dlc);

        // Deviation 2: C backend 0-cap memory has non-null 1-byte static dummy vs x64 NULL base!
        void *dlzc = dlopen(argv[5], RTLD_NOW);
        CHECK(dlzc != NULL, "dlopen zero_c: %s", dlerror());
        fn_new zc_new = (fn_new)dlsym(dlzc, "blot_aot_new");
        fn_drop zc_drop = (fn_drop)dlsym(dlzc, "blot_aot_drop");
        fn_mem_data zc_mem = (fn_mem_data)dlsym(dlzc, "blot_aot_mem_data");
        void *hzc = zc_new(NULL, NULL, 0, &tr);
        CHECK(hzc != NULL, "zero_c new");
        uint8_t *zcptr = NULL; size_t zclen = 99;
        zc_mem(hzc, &zcptr, &zclen);
        CHECK(zcptr != NULL && zclen == 0, "C 0-cap memory dummy deviation");
        zc_drop(hzc);
        dlclose(dlzc);
    }
    return 0;
}
"#;

// ---- 2. Masked CPUID & CET SHSTK Fault-Injection Tests (§4.3.5) ----

#[test]
fn fault_injection_cpuid_and_cet_shstk() {
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-cpuid");
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        export_sec(&[("blot:allocate", 0, 0), ("inc", 0, 0)]),
        code_sec(&[enc_body(&[], &[0x20, 0, 0x41, 1, 0x6a])]),
    ]);
    let Some(driver) = compile_c_test_bin(&root.0, "cpuid_driver", CPUID_DRIVER_C, &["-ldl"])
    else {
        return;
    };

    const OK_ECX1: u32 = (1 << 9) | (1 << 19) | (1 << 20) | (1 << 23); // SSSE3 + SSE4.1 + SSE4.2 + POPCNT
    const OK_EXT_MAX: u32 = 0x8000_0001;
    const OK_EXT_ECX: u32 = 1 << 5; // ABM / LZCNT
    const OK_EBX7: u32 = (1 << 3) | (1 << 8); // BMI1 + BMI2
    const CET_SS: u32 = 1 << 7;

    let cases: [(&str, bool, CpuidSource, RdsspSource, bool); 12] = [
        (
            "v2_missing_leaf1",
            false,
            CpuidSource::Mock {
                max_leaf: 0,
                ecx1: OK_ECX1,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v2_missing_sse41",
            false,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: 1 << 23,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v2_missing_popcnt",
            false,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: 1 << 19,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v3_missing_leaf7",
            true,
            CpuidSource::Mock {
                max_leaf: 6,
                ecx1: OK_ECX1,
                ext_max: OK_EXT_MAX,
                ext_ecx: OK_EXT_ECX,
                ebx7: OK_EBX7,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        // Unsigned check: 0x8000_0000 < 0x8000_0001 must fail even with bit 31 set!
        (
            "v3_ext_max_unsigned",
            true,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: 0x8000_0000,
                ext_ecx: OK_EXT_ECX,
                ebx7: OK_EBX7,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v3_missing_lzcnt",
            true,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: OK_EXT_MAX,
                ext_ecx: 0,
                ebx7: OK_EBX7,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v3_missing_bmi1",
            true,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: OK_EXT_MAX,
                ext_ecx: OK_EXT_ECX,
                ebx7: 1 << 8,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        (
            "v3_missing_bmi2",
            true,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: OK_EXT_MAX,
                ext_ecx: OK_EXT_ECX,
                ebx7: 1 << 3,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0 },
            false,
        ),
        // Preserved max_leaf >= 7 probes CET_SS EVEN ON x64-v2!
        (
            "v2_cet_active_fails",
            false,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: CET_SS,
            },
            RdsspSource::Mock { ssp: 0x7fff_1000 },
            false,
        ),
        (
            "v2_cet_off_passes",
            false,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: CET_SS,
            },
            RdsspSource::Mock { ssp: 0 },
            true,
        ),
        // When CET_SS == 0, non-zero mock ssp is bypassed completely!
        (
            "v2_no_cet_bypasses",
            false,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: 0,
                ext_ecx: 0,
                ebx7: 0,
                ecx7: 0,
            },
            RdsspSource::Mock { ssp: 0x7fff_1000 },
            true,
        ),
        (
            "v3_all_ok",
            true,
            CpuidSource::Mock {
                max_leaf: 7,
                ecx1: OK_ECX1,
                ext_max: OK_EXT_MAX,
                ext_ecx: OK_EXT_ECX,
                ebx7: OK_EBX7,
                ecx7: CET_SS,
            },
            RdsspSource::Mock { ssp: 0 },
            true,
        ),
    ];

    for (tag, is_v3, cpuid, rdssp, expect_ok) in cases {
        let so_path = root.0.join(format!("{tag}.so"));
        let bytes = compile_wasm_to_x64_so(
            &wasm,
            &X64CompileOptions {
                isa: TargetIsa {
                    has_abm_bmi1: is_v3,
                    has_bmi2: is_v3,
                },
                jobs: 1,
                deadline: None,
                opt: true,
                verify: true,
                hooks: RuntimeTestHooks {
                    cpuid,
                    rdssp,
                    syscall_fault: SyscallFault::None,
                },
                ..Default::default()
            },
        )
        .unwrap();
        fs::write(&so_path, &bytes).unwrap();
        let mode = if expect_ok {
            "expect_ok"
        } else {
            "expect_fail22"
        };
        let out = Command::new(&driver)
            .args([so_path.to_str().unwrap(), mode])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cpuid case {tag} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Also test top-level `blot_aot_call_export` & `blot_aot_call_allocate` when `cpu_has_cet_ss == 1`
    // and calling thread later migrates onto a CET-active thread (mock `rdsspq != 0` inside call wrappers):
    // Here `blot_aot_new` succeeds with `cet_ss == 0`, then test flips `ctx->cpu_has_cet_ss = 1` (offset 113)
    // on a `.so` compiled with `RdsspSource::Mock { ssp: 0x7fff_2000 }`!
    // Verify: (a) unknown export STILL returns `3` (precedence step 3 before step 4!),
    //         (b) known export returns `1` with `*trap_out == 22`, leaving `*res_out` untouched!
    let migrate_so = root.0.join("cet_migrate.so");
    let bytes = compile_wasm_to_x64_so(
        &wasm,
        &X64CompileOptions {
            isa: TargetIsa::default(),
            jobs: 1,
            deadline: None,
            opt: true,
            verify: true,
            hooks: RuntimeTestHooks {
                cpuid: CpuidSource::Mock {
                    max_leaf: 7,
                    ecx1: OK_ECX1,
                    ext_max: 0,
                    ext_ecx: 0,
                    ebx7: 0,
                    ecx7: 0,
                },
                rdssp: RdsspSource::Mock { ssp: 0x7fff_2000 },
                syscall_fault: SyscallFault::None,
            },
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(&migrate_so, &bytes).unwrap();
    let out = Command::new(&driver)
        .args([migrate_so.to_str().unwrap(), "expect_cet_call"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cet migrate call test failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

const CPUID_DRIVER_C: &str = r#"
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>

typedef void *(*fn_new)(void *, void *, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_call_i32)(void *, int32_t, int32_t *, uint8_t *);

// Verify blot_aot_new preserves RBX across CPUID failure & success paths!
__attribute__((noinline))
static void *call_new_check_rbx(fn_new fn, void *u, void *cb, uint64_t lim, uint8_t *tr, int *rbx_ok) {
    uint64_t rbx_after = 0;
    void *ret;
    __asm__ volatile(
        "push %%rbx\n\t"
        "mov $0xbbbbbbbbbbbbbbbb, %%rbx\n\t"
        "call *%6\n\t"
        "mov %%rbx, %1\n\t"
        "pop %%rbx\n\t"
        : "=a"(ret), "=r"(rbx_after)
        : "D"(u), "S"(cb), "d"(lim), "c"(tr), "r"(fn)
        : "r8", "r9", "r10", "r11", "memory"
    );
    *rbx_ok = (rbx_after == 0xbbbbbbbbbbbbbbbbULL);
    return ret;
}

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    void *dl = dlopen(argv[1], RTLD_NOW);
    if (!dl) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 3; }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export call_export = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    fn_call_i32 call_alloc = (fn_call_i32)dlsym(dl, "blot_aot_call_allocate");

    uint8_t tr = 0;
    int rbx_ok = 0;
    void *h = call_new_check_rbx(b_new, NULL, NULL, 0, &tr, &rbx_ok);
    if (!rbx_ok) { fprintf(stderr, "RBX clobbered in blot_aot_new!\n"); return 4; }

    if (strcmp(argv[2], "expect_fail22") == 0) {
        if (h != NULL || tr != 22) {
            fprintf(stderr, "expected NULL & trap 22, got h=%p tr=%u\n", h, tr);
            return 5;
        }
    } else if (strcmp(argv[2], "expect_ok") == 0) {
        if (h == NULL) { fprintf(stderr, "expected OK, got trap=%u\n", tr); return 6; }
        uint64_t r = 0; tr = 0;
        if (call_export(h, (const uint8_t *)"inc", 3, 41, &r, &tr) != 0 || r != 42) {
            fprintf(stderr, "inc(41) failed: r=%llu tr=%u\n", (unsigned long long)r, tr);
            return 7;
        }
        b_drop(h);
    } else if (strcmp(argv[2], "expect_cet_call") == 0) {
        if (h == NULL) { fprintf(stderr, "expected initial new OK\n"); return 8; }
        // Simulate thread migration onto CET-enabled host thread: set ctx->cpu_has_cet_ss = 1 (offset 113).
        ((uint8_t *)h)[113] = 1;
        uint64_t r = 0xfeedfaceULL; tr = 0;
        // Unknown export MUST return 3 BEFORE CET check!
        if (call_export(h, (const uint8_t *)"no_such", 7, 0, &r, &tr) != 3 || r != 0xfeedfaceULL || tr != 0) {
            fprintf(stderr, "unknown export under CET should return 3, got tr=%u\n", tr);
            return 9;
        }
        // Known export MUST return 1 with trap 22 and leave res_out untouched!
        if (call_export(h, (const uint8_t *)"inc", 3, 41, &r, &tr) != 1 || tr != 22 || r != 0xfeedfaceULL) {
            fprintf(stderr, "known export under CET should return 1/trap 22, got tr=%u r=%llx\n", tr, (unsigned long long)r);
            return 10;
        }
        int32_t r32 = 0x12345678; tr = 0;
        if (call_alloc(h, 5, &r32, &tr) != 1 || tr != 22 || r32 != 0x12345678) {
            fprintf(stderr, "call_allocate under CET should return 1/trap 22, got tr=%u\n", tr);
            return 11;
        }
        b_drop(h);
    }
    dlclose(dl);
    return 0;
}
"#;

// ---- 3. Syscall Fault-Injection (`RLIMIT_AS` + per-site `mprotect`) (§4.3.5) ----

#[test]
fn fault_injection_rlimit_as_and_per_site_mprotect() {
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-syscall-faults");
    // Module with min=1, max=4 memory and a `grow` export.
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        section(5, &[1, 0x01, 1, 4]),
        export_sec(&[("blot:memory", 2, 0), ("grow", 0, 0)]),
        code_sec(&[enc_body(&[], &[0x20, 0, 0x40, 0x00])]),
    ]);
    let Some(driver) = compile_c_test_bin(
        &root.0,
        "syscall_fault_driver",
        SYSCALL_FAULT_DRIVER_C,
        &["-ldl"],
    ) else {
        return;
    };

    let build_variant = |name: &str, fault: SyscallFault| {
        let p = root.0.join(format!("{name}.so"));
        let b = compile_wasm_to_x64_so(
            &wasm,
            &X64CompileOptions {
                isa: TargetIsa::default(),
                jobs: 1,
                deadline: None,
                opt: true,
                verify: true,
                hooks: RuntimeTestHooks {
                    cpuid: CpuidSource::Real,
                    rdssp: RdsspSource::Real,
                    syscall_fault: fault,
                },
                ..Default::default()
            },
        )
        .unwrap();
        fs::write(&p, &b).unwrap();
        p
    };

    let so_ok = build_variant("ok", SyscallFault::None);
    let so_stack = build_variant(
        "fail_stack",
        SyscallFault::FailMprotectAt(MprotectSite::GuestStack),
    );
    let so_init_mem = build_variant(
        "fail_init_mem",
        SyscallFault::FailMprotectAt(MprotectSite::InitialMemory),
    );
    let so_grow = build_variant(
        "fail_grow",
        SyscallFault::FailMprotectAt(MprotectSite::GrowMemory),
    );

    let out = Command::new(&driver)
        .args([
            so_ok.to_str().unwrap(),
            so_stack.to_str().unwrap(),
            so_init_mem.to_str().unwrap(),
            so_grow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "syscall fault driver failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

const SYSCALL_FAULT_DRIVER_C: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <unistd.h>
#include <sys/resource.h>
#include <sys/wait.h>

typedef void *(*fn_new)(void *, void *, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef void (*fn_mem_data)(void *, uint8_t **, size_t *);
typedef int64_t (*fn_mem_grow)(void *, uint64_t);

static int count_anon_maps(void) {
    FILE *f = fopen("/proc/self/maps", "r");
    if (!f) return -1;
    char line[512];
    int count = 0;
    while (fgets(line, sizeof(line), f)) {
        // Count anonymous private regions (no pathname at end of line)
        size_t len = strlen(line);
        if (len > 0 && line[len - 1] == '\n') line[--len] = 0;
        // 5 spaces separate address, perms, offset, dev, inode; if nothing non-space follows, it's anon.
        int fields = 0, in_sp = 1;
        for (size_t i = 0; i < len; i++) {
            if (line[i] == ' ' || line[i] == '\t') { in_sp = 1; }
            else { if (in_sp) { fields++; in_sp = 0; } }
        }
        if (fields == 5) count++;
    }
    fclose(f);
    return count;
}

static void check_new_fail_no_leak(const char *so_path, const char *label) {
    void *dl = dlopen(so_path, RTLD_NOW);
    if (!dl) { fprintf(stderr, "%s dlopen: %s\n", label, dlerror()); exit(1); }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    // Warm up libc stdio/maps counting once, then measure before & after 10 failed calls.
    int before = count_anon_maps();
    for (int i = 0; i < 10; i++) {
        uint8_t tr = 0;
        void *h = b_new(NULL, NULL, 0, &tr);
        if (h != NULL || tr != 12) {
            fprintf(stderr, "%s iter %d expected NULL/trap 12, got h=%p tr=%u\n", label, i, h, tr);
            exit(2);
        }
    }
    int after = count_anon_maps();
    if (after > before + 1) {
        fprintf(stderr, "%s leaked anonymous mappings: before=%d after=%d\n", label, before, after);
        exit(3);
    }
    dlclose(dl);
}

int main(int argc, char **argv) {
    if (argc < 5) return 10;

    // 1. Per-site mprotect failures in blot_aot_new (guest stack & initial memory): trap 12, zero leaks!
    check_new_fail_no_leak(argv[2], "FailMprotectAt(GuestStack)");
    check_new_fail_no_leak(argv[3], "FailMprotectAt(InitialMemory)");

    // 2. Per-site mprotect failure in grow_core: returns -1 and leaves mem_len unchanged!
    void *dlg = dlopen(argv[4], RTLD_NOW);
    if (!dlg) { fprintf(stderr, "dlopen fail_grow: %s\n", dlerror()); return 11; }
    fn_new g_new = (fn_new)dlsym(dlg, "blot_aot_new");
    fn_drop g_drop = (fn_drop)dlsym(dlg, "blot_aot_drop");
    fn_call_export g_call = (fn_call_export)dlsym(dlg, "blot_aot_call_export");
    fn_mem_data g_mem = (fn_mem_data)dlsym(dlg, "blot_aot_mem_data");
    fn_mem_grow g_grow = (fn_mem_grow)dlsym(dlg, "blot_aot_mem_grow");
    uint8_t tr = 0;
    void *hg = g_new(NULL, NULL, 0, &tr);
    if (!hg) { fprintf(stderr, "fail_grow new failed tr=%u\n", tr); return 12; }
    uint64_t res = 0;
    if (g_call(hg, (const uint8_t *)"grow", 4, 1, &res, &tr) != 0 || res != 0x00000000ffffffffULL) {
        fprintf(stderr, "Wasm memory.grow on mprotect failure expected 0xffffffff, got %016llx tr=%u\n", (unsigned long long)res, tr);
        return 13;
    }
    if (g_grow(hg, 1) != -1) { fprintf(stderr, "blot_aot_mem_grow on mprotect failure expected -1\n"); return 14; }
    uint8_t *mp = NULL; size_t ml = 0;
    g_mem(hg, &mp, &ml);
    if (ml != 65536) { fprintf(stderr, "mem_len mutated after failed grow: %zu\n", ml); return 15; }
    g_drop(hg);
    dlclose(dlg);

    // 3. Forked child with tight RLIMIT_AS verifying mmap reservation failure cleanup + trap 12!
    pid_t pid = fork();
    if (pid == 0) {
        void *dl = dlopen(argv[1], RTLD_NOW);
        if (!dl) _exit(20);
        fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
        // Current VM size + 2 MiB: enough for 4 KiB Ctx mmap, TOO SMALL for 8 MiB guest stack mmap!
        FILE *sf = fopen("/proc/self/statm", "r");
        unsigned long vm_pages = 0;
        if (!sf || fscanf(sf, "%lu", &vm_pages) != 1) _exit(21);
        fclose(sf);
        struct rlimit rl;
        rl.rlim_cur = rl.rlim_max = (vm_pages * 4096UL) + (2UL << 20);
        if (setrlimit(RLIMIT_AS, &rl) != 0) _exit(22);
        int before = count_anon_maps();
        for (int i = 0; i < 8; i++) {
            uint8_t t = 0;
            void *h = b_new(NULL, NULL, 0, &t);
            if (h != NULL || t != 12) _exit(23);
        }
        int after = count_anon_maps();
        if (after > before + 1) _exit(24);
        _exit(0);
    }
    int status = 0;
    if (waitpid(pid, &status, 0) != pid || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        fprintf(stderr, "RLIMIT_AS child failed: status=0x%x exit=%d\n", status, WIFEXITED(status) ? WEXITSTATUS(status) : -1);
        return 16;
    }
    return 0;
}
"#;

// ---- 4. Signal-at-Floor Watchdog-Controlled Forked Child Test (§4.3.5) ----

#[test]
fn fault_injection_signal_at_floor_watchdog() {
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-signal-floor");
    // Zero-host-call Wasm guest:
    // - f0 (opaque non-inlined load helper): `() -> i32 { i32.load(0) }`
    // - f1 (`floor_spin(depth: i32) -> i32`):
    //     if depth <= 0:
    //       store 1 to mem[4] (proving ALREADY at floor depth on the guest stack!)
    //       loop { if f0() != 0 { return 123 } }
    //     else:
    //       // spill 16 locals so each frame uses a predictable ~160 B, recursing cleanly to <256 B above stack_floor!
    //       return floor_spin(depth - 1)
    // - f2 (`probe_max_depth(d: i32) -> i32`): recurses with identical frame size `d` times & returns 1
    //   so harness binary-searches the exact maximum depth `D_max` that fits above `stack_floor` (< 256 B headroom)!
    let b_frame = |is_spin: bool| {
        let mut b = Vec::new();
        // Force a 16-local (~160 B) frame identically in both spin & probe!
        for i in 1..=16u32 {
            b.push(0x20);
            b.push(0);
            b.push(0x41);
            leb_i32(&mut b, i as i32);
            b.push(0x6a);
            b.push(0x21);
            leb_u32(&mut b, i);
        }
        b.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x7f]); // if d == 0
        if is_spin {
            // mem[4] = 1
            b.extend_from_slice(&[0x41, 4, 0x41, 1, 0x36, 2, 0]);
            // loop { br_if_eq0 f0() }
            b.extend_from_slice(&[0x03, 0x40, 0x10, 0, 0x45, 0x0d, 0, 0x0b, 0x41]);
            leb_i32(&mut b, 123);
        } else {
            b.extend_from_slice(&[0x41, 1]);
        }
        b.push(0x05); // else
        b.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6b, 0x10, if is_spin { 1 } else { 2 }]);
        // Keep all 16 locals genuinely live across the recursive call using an opaque i32.load(8) (== 0),
        // so constant folding & DCE cannot eliminate them and every frame is ~160 B!
        b.extend_from_slice(&[0x20, 1]);
        for i in 2..=16u32 {
            b.push(0x20);
            leb_u32(&mut b, i);
            b.push(0x73); // i32.xor
        }
        b.extend_from_slice(&[0x41, 8, 0x28, 2, 0, 0x71, 0x6a]); // (xor_all & mem[8]) + rec_res
        b.push(0x0b);
        enc_body(&[(16, 0x7f)], &b)
    };

    let wasm = wasm_module(&[
        type_sec(&[(&[], &[0x7f]), (&[0x7f], &[0x7f])]),
        func_sec(&[0, 1, 1]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("floor_spin", 0, 1),
            ("probe_depth", 0, 2),
        ]),
        code_sec(&[
            enc_body(&[], &[0x41, 0, 0x28, 2, 0]), // f0: i32.load(0)
            b_frame(true),
            b_frame(false),
        ]),
    ]);

    let so_path = root.0.join("signal_floor.so");
    let so_bytes = compile_wasm_to_x64_so(
        &wasm,
        &X64CompileOptions {
            isa: TargetIsa::default(),
            jobs: 1,
            deadline: None,
            opt: true,
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(&so_path, &so_bytes).unwrap();

    let Some(driver) = compile_c_test_bin(
        &root.0,
        "signal_floor_driver",
        SIGNAL_FLOOR_DRIVER_C,
        &["-ldl", "-pthread"],
    ) else {
        return;
    };
    let out = Command::new(&driver).arg(&so_path).output().unwrap();
    assert!(
        out.status.success(),
        "signal-at-floor test failed ({}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

const SIGNAL_FLOOR_DRIVER_C: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>
#include <signal.h>
#include <ucontext.h>
#include <pthread.h>
#include <unistd.h>
#include <dlfcn.h>
#include <sys/wait.h>

typedef void *(*fn_new)(void *, void *, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef void (*fn_mem_data)(void *, uint8_t **, size_t *);

static _Atomic uintptr_t g_captured_rip = 0;
static _Atomic uintptr_t g_captured_rsp = 0;
static _Atomic uint32_t *g_stop_word = NULL;

static void sigusr1_handler(int sig, siginfo_t *si, void *uctx) {
    (void)sig; (void)si;
    ucontext_t *uc = (ucontext_t *)uctx;
    atomic_store_explicit(&g_captured_rip, (uintptr_t)uc->uc_mcontext.gregs[REG_RIP], memory_order_relaxed);
    atomic_store_explicit(&g_captured_rsp, (uintptr_t)uc->uc_mcontext.gregs[REG_RSP], memory_order_relaxed);
    if (g_stop_word) {
        atomic_store_explicit(g_stop_word, 1u, memory_order_release);
    }
}

typedef struct {
    pthread_t guest_tid;
    _Atomic uint32_t *ready_word;
} SenderArg;

static void *sender_thread(void *p) {
    SenderArg *a = (SenderArg *)p;
    // Spin until guest proves it is ALREADY at floor depth on the guest stack (mem[4] == 1)!
    while (atomic_load_explicit(a->ready_word, memory_order_acquire) == 0u) {
        usleep(50);
    }
    pthread_kill(a->guest_tid, SIGUSR1);
    return NULL;
}

static int find_so_text_range(const char *so_path, uintptr_t *lo_out, uintptr_t *hi_out) {
    FILE *f = fopen("/proc/self/maps", "r");
    if (!f) return 0;
    char line[512];
    while (fgets(line, sizeof(line), f)) {
        if (strstr(line, so_path)) {
            unsigned long lo = 0, hi = 0;
            char perms[8] = {0};
            if (sscanf(line, "%lx-%lx %4s", &lo, &hi, perms) == 3 && strcmp(perms, "r-xp") == 0) {
                *lo_out = (uintptr_t)lo;
                *hi_out = (uintptr_t)hi;
                fclose(f);
                return 1;
            }
        }
    }
    fclose(f);
    return 0;
}

static int run_child(const char *so_path) {
    void *dl = dlopen(so_path, RTLD_NOW);
    if (!dl) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 1; }
    uintptr_t text_lo = 0, text_hi = 0;
    if (!find_so_text_range(so_path, &text_lo, &text_hi)) {
        fprintf(stderr, "could not locate .text in /proc/self/maps\n");
        return 2;
    }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export call_export = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    fn_mem_data mem_data = (fn_mem_data)dlsym(dl, "blot_aot_mem_data");

    uint8_t tr = 0;
    void *h = b_new(NULL, NULL, 0, &tr);
    if (!h) { fprintf(stderr, "new failed tr=%u\n", tr); return 3; }
    // Read ctx->stack_floor directly from Ctx offset 40 (documented fixed header offset).
    uintptr_t stack_floor = *(const uintptr_t *)((const uint8_t *)h + 40);
    uint8_t *mp = NULL; size_t ml = 0;
    mem_data(h, &mp, &ml);
    if (!mp || ml < 16) return 4;

    // Binary search exact max recursion depth D_max (< 1 frame above stack_floor!).
    uint64_t lo = 1, hi = 200000;
    while (lo < hi) {
        uint64_t mid = lo + (hi - lo + 1) / 2;
        uint64_t res = 0; uint8_t t = 0;
        if (call_export(h, (const uint8_t *)"probe_depth", 11, mid, &res, &t) == 0) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if (lo >= 199999) {
        fprintf(stderr, "probe_depth never hit stack floor (lo=%llu)\n", (unsigned long long)lo);
        return 11;
    }
    // Leave 1 frame margin so calling f0() inside the spin does not trap call stack!
    uint64_t spin_depth = lo > 1 ? lo - 1 : 1;

    g_stop_word = (_Atomic uint32_t *)(mp + 0);
    _Atomic uint32_t *ready_word = (_Atomic uint32_t *)(mp + 4);
    atomic_store_explicit(g_stop_word, 0u, memory_order_relaxed);
    atomic_store_explicit(ready_word, 0u, memory_order_relaxed);

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = sigusr1_handler;
    sa.sa_flags = SA_SIGINFO; // Deliver directly on the active guest stack!
    sigemptyset(&sa.sa_mask);
    if (sigaction(SIGUSR1, &sa, NULL) != 0) return 5;

    pthread_t sth;
    SenderArg sarg = { pthread_self(), ready_word };
    if (pthread_create(&sth, NULL, sender_thread, &sarg) != 0) return 6;

    uint64_t res = 0; tr = 0;
    int32_t rc = call_export(h, (const uint8_t *)"floor_spin", 10, spin_depth, &res, &tr);
    pthread_join(sth, NULL);

    uintptr_t cap_rip = atomic_load_explicit(&g_captured_rip, memory_order_relaxed);
    uintptr_t cap_rsp = atomic_load_explicit(&g_captured_rsp, memory_order_relaxed);
    if (rc != 0 || res != 123) {
        fprintf(stderr, "floor_spin failed: rc=%d res=%llu tr=%u depth=%llu\n",
                rc, (unsigned long long)res, tr, (unsigned long long)spin_depth);
        return 7;
    }
    if (cap_rip < text_lo || cap_rip >= text_hi) {
        fprintf(stderr, "CAPTURED_RIP 0x%lx outside .text [0x%lx, 0x%lx)\n",
                (unsigned long)cap_rip, (unsigned long)text_lo, (unsigned long)text_hi);
        return 8;
    }
    if (cap_rsp < stack_floor || cap_rsp > stack_floor + 4096) {
        fprintf(stderr, "CAPTURED_RSP 0x%lx outside [stack_floor=0x%lx, +4096] (delta=%ld)\n",
                (unsigned long)cap_rsp, (unsigned long)stack_floor, (long)(cap_rsp - stack_floor));
        return 9;
    }
    b_drop(h);
    dlclose(dl);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) return 10;
    pid_t pid = fork();
    if (pid == 0) {
        _exit(run_child(argv[1]));
    }
    // 2-second watchdog -> SIGKILL on any hang!
    for (int ms = 0; ms < 2000; ms += 10) {
        int status = 0;
        pid_t r = waitpid(pid, &status, WNOHANG);
        if (r == pid) {
            if (WIFEXITED(status)) return WEXITSTATUS(status);
            fprintf(stderr, "signal-at-floor child terminated by signal %d\n", WTERMSIG(status));
            return 20;
        }
        usleep(10000);
    }
    kill(pid, SIGKILL);
    waitpid(pid, NULL, 0);
    fprintf(stderr, "signal-at-floor child timed out (>2s)!\n");
    return 21;
}
"#;

// ---- 5. Expired Deadline Budget Test (Subprocess CLI & Cache tests live in `tests/x64_cli.rs`) ----

#[test]
fn compile_budget_expired_deadline_fails_cleanly_not_admission_rejected() {
    let ok_bytes = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        export_sec(&[("inc", 0, 0)]),
        code_sec(&[enc_body(&[], &[0x20, 0, 0x41, 1, 0x6a])]),
    ]);
    let expired = compile_wasm_to_x64_so(
        &ok_bytes,
        &X64CompileOptions {
            deadline: Some(Instant::now() - Duration::from_secs(1)),
            ..Default::default()
        },
    );
    assert!(matches!(expired, Err(super::X64CompileError::Failed(_))));
}

// ---- 6. Stage 2 Host Callback XMM8..XMM13 Clobber + Reentry Preservation Test ----

#[test]
fn stage2_host_callback_xmm8_13_clobber_preservation() {
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-xmm-clobber");
    // Host import 0: env::clobber_xmm(i32) -> (i32)
    let mut imp_p = vec![1];
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "clobber_xmm");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);

    // f1 (type 0): blot:allocate(n) -> n + 100 (also touches XMM0..XMM15 inside guest reentry!)
    let mut b1 = Vec::new();
    for i in 1..=10u32 {
        b1.push(0x44);
        b1.extend_from_slice(&(i as f64 * 1000.0).to_bits().to_le_bytes());
        b1.push(0x21);
        leb_u32(&mut b1, i);
    }
    b1.push(0x20);
    b1.push(1);
    for i in 2..=10u32 {
        b1.push(0x20);
        leb_u32(&mut b1, i);
        b1.push(0xa0);
    }
    b1.extend_from_slice(&[0xfc, 0x02, 0x20, 0, 0x6a]); // trunc_sat + n

    // f2 (type 1: (f64) -> (f64)): holds 10 distinct f64 values live across host import 0!
    let mut b2 = Vec::new();
    for i in 1..=10u32 {
        b2.push(0x20);
        b2.push(0);
        b2.push(0x44);
        b2.extend_from_slice(&(i as f64 * 1.5).to_bits().to_le_bytes());
        b2.push(0xa0);
        b2.push(0x21);
        leb_u32(&mut b2, i);
    }
    b2.extend_from_slice(&[0x41, 7, 0x10, 0, 0xb7, 0x21, 11]); // r = f64(clobber_xmm(7))
    b2.extend_from_slice(&[0x20, 11]);
    for i in 1..=10u32 {
        b2.push(0x20);
        leb_u32(&mut b2, i);
        b2.push(0xa0);
    }

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7c], &[0x7c])]),
        section(2, &imp_p),
        func_sec(&[0, 1]),
        export_sec(&[("blot:allocate", 0, 1), ("test_xmm", 0, 2)]),
        code_sec(&[enc_body(&[(10, 0x7c)], &b1), enc_body(&[(11, 0x7c)], &b2)]),
    ]);
    let so_path = root.0.join("xmm_clobber.so");
    crate::build::assert_inside_test_root(&root.0, &so_path);
    let so_bytes = compile_wasm_to_x64_so(
        &wasm,
        &X64CompileOptions {
            isa: TargetIsa::default(),
            jobs: 1,
            deadline: None,
            opt: true,
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(&so_path, &so_bytes).unwrap();

    const DRIVER_C: &str = r#"
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef int32_t (*host_fn_t)(void*, void*, uint32_t, uint64_t, uint64_t, uint64_t*);
typedef void* (*fn_new)(void*, host_fn_t, uint64_t, uint8_t*);
typedef void (*fn_drop)(void*);
typedef int32_t (*fn_call_export)(void*, const uint8_t*, size_t, uint64_t, uint64_t*, uint8_t*);
typedef int32_t (*fn_active_alloc)(void*, int32_t, int32_t*, uint8_t*);

static fn_active_alloc g_active_alloc = NULL;

static int32_t clobber_cb(void *u, void *ac, uint32_t idx, uint64_t tok, uint64_t arg, uint64_t *res) {
    (void)u; (void)idx; (void)tok;
    // Explicitly trash ALL 16 XMM registers (xmm0..xmm15) with garbage bit patterns!
    uint64_t junk[2] = { 0xdeadbeefcafebabeULL, 0x0123456789abcdefULL };
    __asm__ __volatile__(
        "movdqu (%0), %%xmm0\n\t"  "movdqu (%0), %%xmm1\n\t"
        "movdqu (%0), %%xmm2\n\t"  "movdqu (%0), %%xmm3\n\t"
        "movdqu (%0), %%xmm4\n\t"  "movdqu (%0), %%xmm5\n\t"
        "movdqu (%0), %%xmm6\n\t"  "movdqu (%0), %%xmm7\n\t"
        "movdqu (%0), %%xmm8\n\t"  "movdqu (%0), %%xmm9\n\t"
        "movdqu (%0), %%xmm10\n\t" "movdqu (%0), %%xmm11\n\t"
        "movdqu (%0), %%xmm12\n\t" "movdqu (%0), %%xmm13\n\t"
        "movdqu (%0), %%xmm14\n\t" "movdqu (%0), %%xmm15\n\t"
        : : "r"(junk)
        : "xmm0","xmm1","xmm2","xmm3","xmm4","xmm5","xmm6","xmm7",
          "xmm8","xmm9","xmm10","xmm11","xmm12","xmm13","xmm14","xmm15"
    );
    // Also reenter guest via blot_aot_active_allocate (which uses XMM regs internally)!
    int32_t out = 0; uint8_t tr = 0;
    if (g_active_alloc(ac, (int32_t)arg, &out, &tr) != 0) return 1;
    // Trash all 16 XMM regs AGAIN right before returning to HELPER_HOST_CALL!
    __asm__ __volatile__(
        "movdqu (%0), %%xmm8\n\t"  "movdqu (%0), %%xmm9\n\t"
        "movdqu (%0), %%xmm10\n\t" "movdqu (%0), %%xmm11\n\t"
        "movdqu (%0), %%xmm12\n\t" "movdqu (%0), %%xmm13\n\t"
        : : "r"(junk)
        : "xmm8","xmm9","xmm10","xmm11","xmm12","xmm13"
    );
    *res = (uint64_t)(uint32_t)out;
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) return 1;
    void *dl = dlopen(argv[1], RTLD_NOW);
    if (!dl) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 2; }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export call_export = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    g_active_alloc = (fn_active_alloc)dlsym(dl, "blot_aot_active_allocate");
    uint8_t tr = 0;
    void *h = b_new(NULL, clobber_cb, 0, &tr);
    if (!h) { fprintf(stderr, "b_new failed tr=%u\n", tr); return 3; }

    double x = 2.25;
    uint64_t arg_bits = 0, res_bits = 0;
    memcpy(&arg_bits, &x, 8);
    int32_t rc = call_export(h, (const uint8_t *)"test_xmm", 8, arg_bits, &res_bits, &tr);
    if (rc != 0) { fprintf(stderr, "call failed rc=%d tr=%u\n", rc, tr); return 4; }
    double actual = 0.0;
    memcpy(&actual, &res_bits, 8);
    // Expected: active_alloc(7) = (1+2+..+10)*1000 + 7 = 55007.0,
    // plus sum_{i=1..10} (2.25 + 1.5*i) = 22.5 + 82.5 = 105.0 -> 55112.0 exact!
    double expected = 55112.0;
    uint64_t exp_bits = 0;
    memcpy(&exp_bits, &expected, 8);
    if (res_bits != exp_bits) {
        fprintf(stderr, "XMM8..13 clobbered across host callback! got %.17g (0x%016llx), expected %.17g (0x%016llx)\n",
                actual, (unsigned long long)res_bits, expected, (unsigned long long)exp_bits);
        return 5;
    }
    b_drop(h);
    dlclose(dl);
    return 0;
}
"#;
    let Some(driver) = compile_c_test_bin(&root.0, "xmm_clobber_driver", DRIVER_C, &["-ldl"])
    else {
        return;
    };
    let out = Command::new(&driver).arg(&so_path).output().unwrap();
    assert!(
        out.status.success(),
        "xmm clobber driver failed ({:?}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rust_c_x64_blot_abi_parity_and_safety() {
    let root = crate::build::create_fresh_test_root("wtime-aot-3backend-abi");

    // Module exercising:
    // - embedded NUL import ("a\0b"."c\0d": (i32)->(i32))
    // - memory min=1 max=64 (to test mem_limit_pages=2)
    // - mutable i32 global "g0" + immut externref null global "g_ref"
    // - empty export "" () -> ()
    // - blot:allocate (i32) -> (i32)
    // - call_imp (i32) -> (i32)
    // - mxcsr_subnormal (f64) -> (f64)
    // - div_bool (i32) -> (i32): 1 iff (0.0 / ((x == 5) as f64)) is NaN
    // - recurse (i32) -> (i32)
    let mut imp_p = vec![1];
    enc_str(&mut imp_p, "a\0b");
    enc_str(&mut imp_p, "c\0d");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);

    let mut glob_p = vec![2];
    glob_p.extend_from_slice(&[0x7f, 1, 0x41]);
    leb_i32(&mut glob_p, 10);
    glob_p.push(0x0b);
    glob_p.extend_from_slice(&[0x6f, 0, 0xd0, 0x6f, 0x0b]);

    // f1: "" () -> ()
    let b1: Vec<u8> = vec![];
    // f2: blot:allocate(a) -> g0 += a; return g0
    let b2: Vec<u8> = vec![0x23, 0, 0x20, 0, 0x6a, 0x24, 0, 0x23, 0];
    // f3: call_imp(a) -> call 0 (a)
    let b3: Vec<u8> = vec![0x20, 0, 0x10, 0];
    // f4: mxcsr_subnormal(x: f64) -> x + 5e-324
    let mut b4: Vec<u8> = vec![0x20, 0, 0x44];
    b4.extend_from_slice(&1u64.to_le_bytes());
    b4.push(0xa0);
    // f5: div_bool(x: i32) -> let d = 0.0 / f64.convert_i32_s(x == 5); (d != d) as i32
    let mut b5: Vec<u8> = vec![0x44];
    b5.extend_from_slice(&0.0f64.to_bits().to_le_bytes());
    b5.extend_from_slice(&[0x20, 0, 0x41, 5, 0x46, 0xb7, 0xa3, 0x22, 1, 0x20, 1, 0x62]);
    // f6: recurse(n) -> if n <= 0 { 0 } else { 1 + recurse(n - 1) }
    let b6: Vec<u8> = vec![
        0x20, 0, 0x41, 0, 0x4c, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 6, 0x41,
        1, 0x6a, 0x0b,
    ];
    // f7: call_mxcsr_internal(x: f64) -> call 4 (mxcsr_subnormal) so f4 has >=1 internal caller!
    let b7: Vec<u8> = vec![0x20, 0, 0x10, 4];

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]), // t0
            (&[], &[]),         // t1
            (&[0x7c], &[0x7c]), // t2
        ]),
        section(2, &imp_p),
        func_sec(&[1, 0, 0, 2, 0, 0, 2]),
        // Omitted max (flags=0x00, min=1) to verify Rust/C/x64 honor mem_limit_pages
        // without reserving 4 GiB under tight RLIMIT_AS!
        section(5, &[1, 0x00, 1]),
        section(6, &glob_p),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("", 0, 1),
            ("blot:allocate", 0, 2),
            ("call_imp", 0, 3),
            ("mxcsr_subnormal", 0, 4),
            ("div_bool", 0, 5),
            ("recurse", 0, 6),
            ("call_mxcsr_internal", 0, 7),
            ("g0", 3, 0),
            ("g_ref", 3, 1),
        ]),
        code_sec(&[
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[], &b4),
            enc_body(&[(1, 0x7c)], &b5),
            enc_body(&[], &b6),
            enc_body(&[], &b7),
        ]),
    ]);

    let wasm_path = root.0.join("parity3.wasm");
    fs::write(&wasm_path, &wasm).unwrap();

    // 1. x64 backend
    let x64_so = root.0.join("parity3_x64.so");
    let x64_bytes = compile_wasm_to_x64_so(&wasm, &X64CompileOptions::default()).unwrap();
    fs::write(&x64_so, &x64_bytes).unwrap();

    // 2. C backend
    let c_cache = root.0.join("c_cache");
    let c_so = root.0.join("parity3_c.so");
    crate::cgen::build_c_blot_cdylib(
        &wasm,
        &c_cache,
        "native",
        Some(2),
        crate::cgen::COpt::Auto,
        Some(Duration::from_secs(120)),
        &c_so,
    )
    .unwrap();

    // 3. Rust backend (--blot-cdylib, 1.99.0)
    let rust_cache = root.0.join("rust_cache");
    let rt_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt");
    let rust_opts = crate::build::Options {
        toolchain: "1.99.0",
        target_cpu: "native",
        jobs: Some(2),
        time_limit: Duration::from_secs(180),
        target: crate::codegen::Target::BlotCdylib,
        backend: crate::build::Backend::Rust,
        c_opt: crate::cgen::COpt::Auto,
    };
    let rust_so = crate::build::build(
        &wasm_path,
        &rust_cache,
        Some("parity3.wasm"),
        &rt_path,
        &rust_opts,
    )
    .expect("rust --blot-cdylib build must succeed on 1.99.0");

    const DRIVER_3_C: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <dlfcn.h>
#include <pthread.h>
#include <immintrin.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

typedef int32_t (*host_fn_t)(void *, void *, uint32_t, uint64_t, uint64_t, uint64_t *);
typedef uint32_t (*fn_u32)(void);
typedef int32_t (*fn_trap_msg)(uint8_t, const uint8_t **, size_t *);
typedef int32_t (*fn_imp_name)(uint32_t, const uint8_t **, size_t *, const uint8_t **, size_t *);
typedef void *(*fn_new)(void *, host_fn_t, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_call_i32)(void *, int32_t, int32_t *, uint8_t *);
typedef void (*fn_mem_data)(void *, uint8_t **, size_t *);
typedef int64_t (*fn_mem_grow)(void *, uint64_t);
typedef int32_t (*fn_get_global)(void *, const uint8_t *, size_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_set_global)(void *, const uint8_t *, size_t, uint64_t);

#define CHECK(cond, ...) do { if (!(cond)) { fprintf(stderr, "FAIL(%s): ", tag); fprintf(stderr, __VA_ARGS__); fprintf(stderr, "\n"); return 1; } } while (0)

typedef struct {
    const char *tag;
    void *h;
    fn_call_export call_export;
    fn_call_i32 active_alloc;
    fn_mem_data mem_data;
    fn_mem_data active_mem_data;
    fn_mem_grow mem_grow;
    fn_get_global getg;
    fn_get_global active_getg;
    fn_set_global setg;
    fn_drop drop;
    uint32_t seen_mxcsr;
    int ok;
} CbState;

static int32_t cb_fn(void *u, void *ac, uint32_t idx, uint64_t tok, uint64_t arg, uint64_t *res) {
    (void)tok;
    CbState *s = (CbState *)u;
    const char *tag = s->tag;
    s->seen_mxcsr = _mm_getcsr();
    uint64_t r64 = 0xaaaa; uint8_t tr = 0;
    if (idx != 0) return 1;
    // Reentry guards inside callback (must fail closed without aliasing active guest state)
    if (s->call_export(s->h, (const uint8_t *)"", 0, 0, &r64, &tr) != 2 || tr != 19) return 1;
    if (s->mem_grow(s->h, 0) != -1) return 1;
    if (s->setg(s->h, (const uint8_t *)"g0", 2, 999) != 1) return 1;
    uint8_t *outer_mp = (uint8_t *)1; size_t outer_ml = 99;
    s->mem_data(s->h, &outer_mp, &outer_ml);
    if (outer_mp != NULL || outer_ml != 0) return 1;
    uint64_t ogv = 0; uint8_t ogk = 255;
    if (s->getg(s->h, (const uint8_t *)"g0", 2, &ogv, &ogk) != 1) return 1;
    s->drop(s->h); // must be a safe no-op!
    // Active call APIs work cleanly via ActiveCall*
    uint8_t *amp = NULL; size_t aml = 0;
    s->active_mem_data(ac, &amp, &aml);
    if (amp == NULL || aml != 131072) return 1;
    int32_t ares = 0; uint8_t atr = 0;
    if (s->active_alloc(ac, 5, &ares, &atr) != 0 || ares != 15) return 1;
    uint64_t gv = 0; uint8_t gk = 255;
    if (s->active_getg(ac, (const uint8_t *)"g0", 2, &gv, &gk) != 0 || gk != 0 || gv != 15) return 1;
    (void)tag;
    s->ok = 1;
    *res = arg + 100;
    return 0;
}

typedef struct {
    fn_call_export call_export;
    void *h;
    int rc;
} ThArg;

static void *small_th(void *p) {
    ThArg *a = (ThArg *)p;
    uint64_t r = 0; uint8_t tr = 0;
    if (a->call_export(a->h, (const uint8_t *)"recurse", 7, 5, &r, &tr) != 0 || r != 5) {
        a->rc = 10; return NULL;
    }
    if (a->call_export(a->h, (const uint8_t *)"recurse", 7, 1000000, &r, &tr) != 1 || tr != 8) {
        a->rc = 11; return NULL;
    }
    a->rc = 0;
    return NULL;
}

static int test_one(const char *tag, const char *so_path) {
    void *dl = dlopen(so_path, RTLD_NOW);
    CHECK(dl != NULL, "dlopen: %s", dlerror());
    fn_u32 imp_cnt = (fn_u32)dlsym(dl, "blot_aot_import_count");
    fn_trap_msg trap_msg = (fn_trap_msg)dlsym(dl, "blot_aot_trap_message");
    fn_imp_name imp_name = (fn_imp_name)dlsym(dl, "blot_aot_import_name");
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export call_export = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    fn_call_i32 call_alloc = (fn_call_i32)dlsym(dl, "blot_aot_call_allocate");
    fn_call_i32 active_alloc = (fn_call_i32)dlsym(dl, "blot_aot_active_allocate");
    fn_mem_data mem_data = (fn_mem_data)dlsym(dl, "blot_aot_mem_data");
    fn_mem_data active_mem_data = (fn_mem_data)dlsym(dl, "blot_aot_active_mem_data");
    fn_mem_grow mem_grow = (fn_mem_grow)dlsym(dl, "blot_aot_mem_grow");
    fn_get_global getg = (fn_get_global)dlsym(dl, "blot_aot_get_global");
    fn_get_global active_getg = (fn_get_global)dlsym(dl, "blot_aot_active_get_global");
    fn_set_global setg = (fn_set_global)dlsym(dl, "blot_aot_set_global");

    // 0. Trap message parity for code 22 across all 3 backends
    const uint8_t *tm_ptr = NULL; size_t tm_len = 0;
    CHECK(trap_msg(22, &tm_ptr, &tm_len) == 0 && tm_len == 19 && memcmp(tm_ptr, "Unsupported CPU ISA", 19) == 0,
          "trap_message(22) mismatch len=%zu", tm_len);

    // 1. Embedded NUL import name byte-exact length
    CHECK(imp_cnt() == 1, "import_count");
    const uint8_t *mp = NULL, *np = NULL; size_t ml = 0, nl = 0;
    CHECK(imp_name(0, &mp, &ml, &np, &nl) == 0 && ml == 3 && memcmp(mp, "a\0b", 3) == 0 && nl == 3 && memcmp(np, "c\0d", 3) == 0,
          "embedded NUL import name mismatch ml=%zu nl=%zu", ml, nl);

    // 2. Null handle guards
    uint64_t r64 = 0x1234; int32_t r32 = 0x1234; uint8_t tr = 0;
    CHECK(call_export(NULL, (const uint8_t *)"", 0, 0, &r64, &tr) == 3, "null call_export");
    CHECK(call_alloc(NULL, 1, &r32, &tr) == 2 && tr == 19, "null call_alloc");
    CHECK(active_alloc(NULL, 1, &r32, &tr) == 2 && tr == 19, "null active_alloc");
    CHECK(mem_grow(NULL, 0) == -1, "null mem_grow");
    CHECK(getg(NULL, (const uint8_t *)"g0", 2, &r64, &tr) == 1, "null getg");
    CHECK(active_getg(NULL, (const uint8_t *)"g0", 2, &r64, &tr) == 1, "null active_getg");
    CHECK(setg(NULL, (const uint8_t *)"g0", 2, 1) == 1, "null setg");
    uint8_t *mp0 = (uint8_t *)1; size_t ml0 = 99;
    mem_data(NULL, &mp0, &ml0);
    CHECK(mp0 == NULL && ml0 == 0, "null mem_data");
    b_drop(NULL);

    // 3. mem_limit_pages = 2 (module omits max; must NOT allocate 4 GiB!)
    CbState cb;
    memset(&cb, 0, sizeof(cb));
    cb.tag = tag; cb.call_export = call_export; cb.active_alloc = active_alloc;
    cb.mem_data = mem_data; cb.active_mem_data = active_mem_data;
    cb.mem_grow = mem_grow; cb.getg = getg; cb.active_getg = active_getg; cb.setg = setg; cb.drop = b_drop;
    void *h = b_new(&cb, cb_fn, 2, &tr);
    CHECK(h != NULL, "new failed tr=%u", tr);
    cb.h = h;
    CHECK(mem_grow(h, 1) == 1, "mem_grow(1) under limit=2");
    CHECK(mem_grow(h, 1) == -1, "mem_grow past limit=2 must fail");

    // 3b. Verify omitted-max backing reservation respects RLIMIT_AS (256 MiB << 4 GiB)
    pid_t pid = fork();
    CHECK(pid >= 0, "fork failed");
    if (pid == 0) {
        struct rlimit rl = { 256ULL * 1024ULL * 1024ULL, 256ULL * 1024ULL * 1024ULL };
        if (setrlimit(RLIMIT_AS, &rl) != 0) _exit(90);
        uint8_t ctr = 0;
        void *ch = b_new(NULL, NULL, 2, &ctr);
        if (!ch) _exit(91);
        b_drop(ch);
        ctr = 0;
        void *big = b_new(NULL, NULL, 8192, &ctr); // 512 MiB > 256 MiB RLIMIT_AS
        if (big != NULL || ctr != 12) _exit(92);
        _exit(0);
    }
    int wstatus = 0;
    CHECK(waitpid(pid, &wstatus, 0) == pid && WIFEXITED(wstatus) && WEXITSTATUS(wstatus) == 0,
          "RLIMIT_AS backing cap check failed status=0x%x exit=%d", wstatus, WIFEXITED(wstatus) ? WEXITSTATUS(wstatus) : -1);

    // 4. (NULL, 0), (NULL, 4), invalid UTF-8
    CHECK(call_export(h, NULL, 0, 0, &r64, &tr) == 0, "(NULL, 0) empty export");
    CHECK(call_export(h, NULL, 4, 0, &r64, &tr) == 3, "(NULL, 4) export");
    const uint8_t bad_utf8[2] = {0xff, 0xfe};
    CHECK(call_export(h, bad_utf8, 2, 0, &r64, &tr) == 3, "invalid UTF-8 call_export");
    CHECK(getg(h, bad_utf8, 2, &r64, &tr) == 1, "invalid UTF-8 getg");
    CHECK(setg(h, bad_utf8, 2, 1) == 1, "invalid UTF-8 setg");
    uint64_t gv = 99; uint8_t gk = 255;
    CHECK(getg(h, (const uint8_t *)"g_ref", 5, &gv, &gk) == 0 && gk == 4 && gv == 0, "g_ref null");

    // 5. Float div 0.0 / (cmp as f64) -> NaN regression
    CHECK(call_export(h, (const uint8_t *)"div_bool", 8, 0, &r64, &tr) == 0 && r64 == 1, "div_bool(0) must be NaN");
    CHECK(call_export(h, (const uint8_t *)"div_bool", 8, 5, &r64, &tr) == 0 && r64 == 0, "div_bool(5) must be 0.0");

    // 6. MXCSR isolation (including export that ALSO has an internal caller!) + callback reentry
    uint32_t orig_mxcsr = _mm_getcsr();
    uint32_t custom_mxcsr = 0x1f80 | (1u << 15) | (1u << 6);
    _mm_setcsr(custom_mxcsr);
    CHECK(call_export(h, (const uint8_t *)"mxcsr_subnormal", 15, 0, &r64, &tr) == 0 && r64 == 1ULL,
          "mxcsr_subnormal flushed to %llu", (unsigned long long)r64);
    CHECK(call_export(h, (const uint8_t *)"call_mxcsr_internal", 19, 0, &r64, &tr) == 0 && r64 == 1ULL,
          "call_mxcsr_internal flushed to %llu", (unsigned long long)r64);
    CHECK((_mm_getcsr() & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored after call");
    CHECK(call_export(h, (const uint8_t *)"call_imp", 8, 7, &r64, &tr) == 0 && r64 == 107 && cb.ok == 1,
          "call_imp callback failed r64=%llu ok=%d", (unsigned long long)r64, cb.ok);
    CHECK((cb.seen_mxcsr & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored in callback");
    CHECK((_mm_getcsr() & 0xffc0u) == (custom_mxcsr & 0xffc0u), "host MXCSR not restored after callback");
    _mm_setcsr(orig_mxcsr);

    // 7. 128 KiB pthread clean stack overflow trap
    pthread_t th; pthread_attr_t attr;
    pthread_attr_init(&attr);
    CHECK(pthread_attr_setstacksize(&attr, 128 * 1024) == 0, "pthread_attr_setstacksize");
    ThArg ta = { call_export, h, -1 };
    CHECK(pthread_create(&th, &attr, small_th, &ta) == 0, "pthread_create");
    pthread_attr_destroy(&attr);
    pthread_join(th, NULL);
    CHECK(ta.rc == 0, "128 KiB pthread failed rc=%d", ta.rc);

    b_drop(h);
    dlclose(dl);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    if (test_one("x64", argv[1]) != 0) return 10;
    if (test_one("c", argv[2]) != 0) return 11;
    if (test_one("rust", argv[3]) != 0) return 12;
    return 0;
}
"#;
    let Some(driver) =
        compile_c_test_bin(&root.0, "abi3_driver", DRIVER_3_C, &["-ldl", "-pthread"])
    else {
        return;
    };
    let out = Command::new(&driver)
        .args([
            x64_so.to_str().unwrap(),
            c_so.to_str().unwrap(),
            rust_so.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "3-backend ABI parity driver failed ({:?}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rust_c_x64_small_stack_ctx_heap_and_long_export_parity() {
    let root = crate::build::create_fresh_test_root("wtime-aot-small-stack-long-exp");

    fn leb(out: &mut Vec<u8>, mut n: u32) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    fn sec(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
        out.push(id);
        leb(out, payload.len() as u32);
        out.extend_from_slice(payload);
    }

    // Build a module exercising:
    // 1. A 65,537-byte valid export name returning 42 (must succeed on all 3 backends).
    // 2. 16 mutable externref tables x 4,096 entries (512 KiB > 256 KiB thread stack)
    //    proving blot_aot_new initializes Ctx/tables on the heap without stack overflow.
    // 3. A large-frame function `big_frame` with 4,096 live i64 locals loaded from memory
    //    across an opaque host call so LLVM must spill ~32 KiB+; invoking it on a padded
    //    256 KiB pthread stack must trap cleanly with TRAP_CALL_STACK (8), never SIGSEGV.
    let long_name = "a".repeat(65_537);
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    // Types: 0: (i32) -> i64 (host tick), 1: () -> i32 (long export), 2: (i32) -> i32 (big_frame)
    sec(
        &mut wasm,
        1,
        &[
            3, 0x60, 1, 0x7f, 1, 0x7e, 0x60, 0, 1, 0x7f, 0x60, 1, 0x7f, 1, 0x7f,
        ],
    );
    // Import: env.tick : type 0
    sec(
        &mut wasm,
        2,
        &[1, 3, b'e', b'n', b'v', 4, b't', b'i', b'c', b'k', 0x00, 0],
    );
    // Defined funcs: f1 = type 1 (long export), f2 = type 2 (big_frame)
    sec(&mut wasm, 3, &[2, 1, 2]);
    // 16 fixed-size mutable externref tables of 4,096 entries each (512 KiB total)
    let mut tabs = vec![16];
    for _ in 0..16 {
        tabs.push(0x6f); // externref
        tabs.push(0x01); // min + max
        leb(&mut tabs, 4096);
        leb(&mut tabs, 4096);
    }
    sec(&mut wasm, 4, &tabs);
    // Memory: min 1 page, max 2 pages
    sec(&mut wasm, 5, &[1, 0x01, 1, 2]);
    // Exports: long_name -> f1, "big_frame" -> f2
    let mut exps = vec![2];
    leb(&mut exps, long_name.len() as u32);
    exps.extend_from_slice(long_name.as_bytes());
    exps.extend_from_slice(&[0x00, 1]);
    leb(&mut exps, 9);
    exps.extend_from_slice(b"big_frame");
    exps.extend_from_slice(&[0x00, 2]);
    sec(&mut wasm, 7, &exps);

    // Body 0 (f1): i32.const 42; end
    let b1 = vec![0, 0x41, 42, 0x0b];
    // Body 1 (f2, param l0: i32): 768 i64 locals loaded from memory across call env.tick
    // (forces >6 KiB real frame spill while compiling in seconds under LLVM -O3).
    let n_locals = 768u32;
    let mut b2 = vec![1];
    leb(&mut b2, n_locals);
    b2.push(0x7e); // i64
    for k in 1..=n_locals {
        b2.extend_from_slice(&[0x20, 0, 0x29, 0x03]); // local.get 0; i64.load align=3
        leb(&mut b2, (k - 1) * 8);
        b2.push(0x21); // local.set k
        leb(&mut b2, k);
    }
    b2.extend_from_slice(&[0x20, 0, 0x10, 0]); // acc = call env.tick(l0)
    for k in 1..=n_locals {
        b2.push(0x20); // local.get k
        leb(&mut b2, k);
        b2.push(0x85); // i64.xor
    }
    b2.extend_from_slice(&[0xa7, 0x0b]); // i32.wrap_i64; end

    let mut code = vec![2];
    leb(&mut code, b1.len() as u32);
    code.extend_from_slice(&b1);
    leb(&mut code, b2.len() as u32);
    code.extend_from_slice(&b2);
    sec(&mut wasm, 10, &code);

    let wasm_path = root.0.join("mod.wasm");
    fs::write(&wasm_path, &wasm).unwrap();

    // 1. x64 backend
    let x64_so = root.0.join("x64.so");
    let x64_bytes = compile_wasm_to_x64_so(&wasm, &X64CompileOptions::default()).unwrap();
    fs::write(&x64_so, &x64_bytes).unwrap();

    // 2. C backend
    let c_cache = root.0.join("c_cache");
    let c_so = root.0.join("c.so");
    crate::cgen::build_c_blot_cdylib(
        &wasm,
        &c_cache,
        "native",
        Some(2),
        crate::cgen::COpt::O1,
        Some(Duration::from_secs(120)),
        &c_so,
    )
    .expect("c compile");

    // 3. Rust backend (--blot-cdylib, 1.99.0)
    let rust_cache = root.0.join("rust_cache");
    let rt_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt");
    let rust_opts = crate::build::Options {
        toolchain: "1.99.0",
        target_cpu: "native",
        jobs: Some(2),
        time_limit: Duration::from_secs(180),
        target: crate::codegen::Target::BlotCdylib,
        backend: crate::build::Backend::Rust,
        c_opt: crate::cgen::COpt::O1,
    };
    let rust_so = crate::build::build(
        &wasm_path,
        &rust_cache,
        Some("mod.wasm"),
        &rt_path,
        &rust_opts,
    )
    .expect("rust compile");

    const DRIVER_C: &str = r#"
#define _GNU_SOURCE
#include <dlfcn.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int32_t (*host_fn_t)(void *, void *, uint32_t, uint64_t, uint64_t, uint64_t *);
typedef void *(*fn_new)(void *, host_fn_t, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);

static int32_t tick_cb(void *u, void *ac, uint32_t idx, uint64_t tok, uint64_t arg, uint64_t *res) {
    (void)u; (void)ac; (void)idx; (void)tok; (void)arg;
    *res = 7;
    return 0;
}

typedef struct {
    const char *label;
    const char *so_path;
    int rc;
} Job;

static int __attribute__((noinline)) call_near_floor(
    uintptr_t floor,
    fn_call_export call_fn,
    void *h,
    const char *long_name,
    const char *label,
    uint64_t *r64,
    uint8_t *tr
) {
    volatile uint8_t frame[4096];
    frame[0] = 1;
    frame[sizeof(frame) - 1] = 2;
    uintptr_t sp = (uintptr_t)&frame[0];
    if (sp > floor + 10 * 1024) {
        return call_near_floor(floor, call_fn, h, long_name, label, r64, tr);
    }
    // At this exact depth (~6-10 KiB above stack_floor), a tiny export MUST still succeed
    // (proving we have not crossed stack_floor), while `big_frame` MUST trap in its
    // pre-frame stub on C and Rust (and succeed on x64's dedicated 8 MiB stack).
    uint64_t lr = 0; uint8_t ltr = 0;
    int lrc = call_fn(h, (const uint8_t *)long_name, 65537, 0, &lr, &ltr);
    if (lrc != 0 || lr != 42) {
        fprintf(stderr, "[%s] small export near floor failed lrc=%d lr=%llu ltr=%u\n",
                label, lrc, (unsigned long long)lr, ltr);
        return -99;
    }
    return call_fn(h, (const uint8_t *)"big_frame", 9, 0, r64, tr);
}

static void *worker(void *arg) {
    Job *j = (Job *)arg;
    void *dl = dlopen(j->so_path, RTLD_NOW);
    if (!dl) { fprintf(stderr, "[%s] dlopen: %s\n", j->label, dlerror()); j->rc = 1; return NULL; }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export b_call = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    uint8_t tr = 0;
    // 1. Instantiate 512 KiB of externref tables on a 256 KiB thread stack!
    void *h = b_new(NULL, tick_cb, 2, &tr);
    if (!h) { fprintf(stderr, "[%s] blot_aot_new failed tr=%u\n", j->label, tr); dlclose(dl); j->rc = 2; return NULL; }

    // 2. Call 65,537-byte export name (must return 42 on all 3 backends).
    char *long_name = (char *)malloc(65537);
    memset(long_name, 'a', 65537);
    uint64_t r64 = 0;
    int rc = b_call(h, (const uint8_t *)long_name, 65537, 0, &r64, &tr);
    if (rc != 0 || r64 != 42) {
        fprintf(stderr, "[%s] 65537-byte export failed rc=%d r64=%llu tr=%u\n", j->label, rc, (unsigned long long)r64, tr);
        free(long_name); b_drop(h); dlclose(dl); j->rc = 3; return NULL;
    }

    // 3a. Unpadded big_frame on 256 KiB stack has ~120 KiB headroom and must succeed on all 3 backends.
    tr = 0;
    r64 = 0;
    rc = b_call(h, (const uint8_t *)"big_frame", 9, 0, &r64, &tr);
    if (rc != 0 || r64 != 7) {
        fprintf(stderr, "[%s] unpadded big_frame expected ok(7), got rc=%d r64=%llu tr=%u\n",
                j->label, rc, (unsigned long long)r64, tr);
        free(long_name); b_drop(h); dlclose(dl); j->rc = 6; return NULL;
    }

    // 3b. Step down to ~6-10 KiB above this thread's stack_floor:
    //    x64 runs on its dedicated 8 MiB stack (succeeds returning 7), while
    //    C and Rust run on the host thread stack and must trap cleanly (rc=1, trap=8)
    //    in their pre-frame wrapper rather than overflowing the stack guard page.
    pthread_attr_t cur_attr;
    void *saddr = NULL; size_t ssize = 0;
    if (pthread_getattr_np(pthread_self(), &cur_attr) != 0 ||
        pthread_attr_getstack(&cur_attr, &saddr, &ssize) != 0) {
        fprintf(stderr, "[%s] pthread_getattr_np failed\n", j->label);
        free(long_name); b_drop(h); dlclose(dl); j->rc = 7; return NULL;
    }
    pthread_attr_destroy(&cur_attr);
    uintptr_t headroom = ssize >= (256ULL << 10) ? (128ULL << 10) : (uintptr_t)(ssize / 2);
    uintptr_t floor = (uintptr_t)saddr + headroom;
    tr = 0;
    r64 = 0;
    rc = call_near_floor(floor, b_call, h, long_name, j->label, &r64, &tr);
    free(long_name);
    if (rc == -99) {
        b_drop(h); dlclose(dl); j->rc = 8; return NULL;
    }
    if (strcmp(j->label, "x64") == 0) {
        if (rc != 0 || r64 != 7) {
            fprintf(stderr, "[x64] big_frame expected ok(7), got rc=%d r64=%llu tr=%u\n", rc, (unsigned long long)r64, tr);
            b_drop(h); dlclose(dl); j->rc = 4; return NULL;
        }
    } else {
        if (rc != 1 || tr != 8) {
            fprintf(stderr, "[%s] big_frame near stack_floor expected trap 8, got rc=%d r64=%llu tr=%u\n",
                    j->label, rc, (unsigned long long)r64, tr);
            b_drop(h); dlclose(dl); j->rc = 5; return NULL;
        }
    }

    b_drop(h);
    dlclose(dl);
    j->rc = 0;
    return NULL;
}

static int run_one(const char *label, const char *so_path) {
    pthread_t th;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    if (pthread_attr_setstacksize(&attr, 256 * 1024) != 0) return 99;
    Job job = { label, so_path, -1 };
    if (pthread_create(&th, &attr, worker, &job) != 0) return 98;
    pthread_attr_destroy(&attr);
    pthread_join(th, NULL);
    return job.rc;
}

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    int r;
    if ((r = run_one("x64", argv[1])) != 0) return 10 + r;
    if ((r = run_one("c", argv[2])) != 0) return 20 + r;
    if ((r = run_one("rust", argv[3])) != 0) return 30 + r;
    return 0;
}
"#;
    let Some(driver) = compile_c_test_bin(
        &root.0,
        "small_stack_driver",
        DRIVER_C,
        &["-ldl", "-pthread"],
    ) else {
        return;
    };
    let out = Command::new(&driver)
        .args([
            x64_so.to_str().unwrap(),
            c_so.to_str().unwrap(),
            rust_so.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "small-stack/long-export 3-backend driver failed ({:?}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
