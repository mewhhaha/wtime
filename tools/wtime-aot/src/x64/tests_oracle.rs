//! Stage 1 Differential Oracle (`wtime-runtime` vs `--backend x64`) across all 4 matrix cells:
//! `(opt=false, v2)`, `(opt=false, v3)`, `(opt=true, v2)`, `(opt=true, v3)`,
//! running `verify_ir` + `verify_alloc` on every single function.
#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code, unused_imports)
)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::{ErrorKind, FuncType, TrapCode, ValType};

use super::lower::TargetIsa;
use super::module::{x64_validate_features, x64_validate_limits};
use super::tests_unit::{
    code_sec, enc_body, enc_str, export_sec, func_sec, leb_i32, leb_i64, leb_u32, section,
    type_sec, wasm_module,
};
use super::{X64CompileOptions, compile_wasm_to_x64_so};

struct OracleDriver {
    _dir: crate::build::TestTempRoot,
    bin: PathBuf,
}

static ORACLE_DRIVER: OnceLock<Option<OracleDriver>> = OnceLock::new();

pub(super) fn get_oracle_driver() -> Option<&'static Path> {
    ORACLE_DRIVER
        .get_or_init(|| {
            let require = std::env::var_os("WTIME_AOT_REQUIRE_NATIVE_TESTS").is_some_and(|v| v == "1");
            let cc = match crate::cgen::detect_cc_with_deadline("native", None, None) {
                Ok(cc) => cc,
                Err(e) if require => panic!("WTIME_AOT_REQUIRE_NATIVE_TESTS=1 is set, but C compiler probe failed: {e}"),
                Err(_) => return None,
            };
            let root = crate::build::create_fresh_test_root("wtime-aot-x64-oracle-drv");
            let c_path = root.0.join("oracle_driver.c");
            let bin = root.0.join("oracle_driver");
            crate::build::assert_inside_test_root(&root.0, &c_path);
            crate::build::assert_inside_test_root(&root.0, &bin);
            fs::write(&c_path, ORACLE_DRIVER_C).ok()?;
            let st = Command::new(&cc.cc_path)
                .args(["-O1", "-o"])
                .arg(&bin)
                .arg(&c_path)
                .args(["-ldl", "-lm"])
                .status()
                .ok()?;
            if !st.success() {
                if require {
                    panic!("WTIME_AOT_REQUIRE_NATIVE_TESTS=1 is set, but oracle driver compilation failed ({st})");
                }
                return None;
            }
            Some(OracleDriver { _dir: root, bin })
        })
        .as_ref()
        .map(|d| d.bin.as_path())
}

const ORACLE_DRIVER_C: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <stddef.h>
#include <string.h>
#include <dlfcn.h>
#include <fenv.h>

typedef int32_t (*host_fn_t)(void *, void *, uint32_t, uint64_t, uint64_t, uint64_t *);
typedef void *(*fn_new)(void *, host_fn_t, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call_export)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef void (*fn_mem_data)(void *, uint8_t **, size_t *);
typedef int32_t (*fn_get_global)(void *, const uint8_t *, size_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_set_global)(void *, const uint8_t *, size_t, uint64_t);

typedef struct {
    uint32_t idx;
    uint64_t tok;
    uint64_t arg;
} HostLogEntry;

typedef struct {
    HostLogEntry log[256];
    size_t count;
} HostState;

static int32_t oracle_host_cb(void *u, void *ac, uint32_t idx, uint64_t tok, uint64_t arg, uint64_t *res) {
    (void)ac;
    HostState *hs = (HostState *)u;
    if (hs->count < 256) {
        hs->log[hs->count].idx = idx;
        hs->log[hs->count].tok = tok;
        hs->log[hs->count].arg = arg;
        hs->count++;
    }
    // Import index 1 fails the host call intentionally to test TRAP_HOST (19) parity!
    if (idx == 1) {
        return 1;
    }
    if (res) {
        *res = (arg ^ 0x55ULL) + (uint64_t)idx * 1000ULL;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) return 2;
    const int host_round_down = argc > 2;
    if (host_round_down && fesetround(FE_DOWNWARD) != 0) return 5;
    void *dl = dlopen(argv[1], RTLD_NOW);
    if (!dl) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 3; }
    fn_new b_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop b_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call_export b_call = (fn_call_export)dlsym(dl, "blot_aot_call_export");
    fn_mem_data b_mem = (fn_mem_data)dlsym(dl, "blot_aot_mem_data");
    fn_get_global b_getg = (fn_get_global)dlsym(dl, "blot_aot_get_global");
    fn_set_global b_setg = (fn_set_global)dlsym(dl, "blot_aot_set_global");
    if (!b_new || !b_drop || !b_call || !b_mem || !b_getg || !b_setg) {
        fprintf(stderr, "missing export symbol\n");
        return 4;
    }

    HostState hs;
    memset(&hs, 0, sizeof(hs));
    uint8_t init_trap = 0;
    void *h = b_new(&hs, oracle_host_cb, 0, &init_trap);
    if (!h) {
        printf("NEW_FAIL %u\n", (unsigned)init_trap);
        dlclose(dl);
        return 0;
    }
    printf("NEW_OK\n");

    char line[512];
    while (fgets(line, sizeof(line), stdin)) {
        char cmd[32];
        if (sscanf(line, "%31s", cmd) != 1) continue;
        if (strcmp(cmd, "call") == 0) {
            char name[128];
            unsigned long long arg = 0;
            if (sscanf(line, "%*s %127s %llx", name, &arg) != 2) continue;
            uint64_t res = 0xdeadbeefcafebabeULL;
            uint8_t tr = 0;
            hs.count = 0;
            int32_t rc = b_call(h, (const uint8_t *)name, strlen(name), (uint64_t)arg, &res, &tr);
            if (host_round_down && fegetround() != FE_DOWNWARD) return 6;
            printf("CALL %d %016llx %u %zu", rc, (unsigned long long)res, (unsigned)tr, hs.count);
            for (size_t i = 0; i < hs.count; i++) {
                printf(" %u:%016llx:%016llx", hs.log[i].idx,
                       (unsigned long long)hs.log[i].tok,
                       (unsigned long long)hs.log[i].arg);
            }
            printf("\n");
        } else if (strcmp(cmd, "getg") == 0) {
            char name[128];
            if (sscanf(line, "%*s %127s", name) != 1) continue;
            uint64_t val = 0;
            uint8_t kind = 255;
            int32_t rc = b_getg(h, (const uint8_t *)name, strlen(name), &val, &kind);
            printf("GLOBAL %d %u %016llx\n", rc, (unsigned)kind, (unsigned long long)val);
        } else if (strcmp(cmd, "mem") == 0) {
            unsigned long off = 0, len = 0;
            if (sscanf(line, "%*s %lu %lu", &off, &len) != 2) continue;
            uint8_t *mp = NULL;
            size_t ml = 0;
            b_mem(h, &mp, &ml);
            printf("MEM %zu ", ml);
            if (mp && off + len <= ml) {
                for (unsigned long i = 0; i < len; i++) {
                    printf("%02x", (unsigned)mp[off + i]);
                }
            }
            printf("\n");
        }
    }

    b_drop(h);
    dlclose(dl);
    return 0;
}
"#;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
enum ArgSpec {
    None,
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

impl ArgSpec {
    fn to_u64(self) -> u64 {
        match self {
            ArgSpec::None => 0,
            ArgSpec::I32(v) => u64::from(v as u32),
            ArgSpec::I64(v) => v as u64,
            ArgSpec::F32(b) => u64::from(b),
            ArgSpec::F64(b) => b,
        }
    }
    fn to_wtime_args(self) -> Vec<Value> {
        match self {
            ArgSpec::None => vec![],
            ArgSpec::I32(v) => vec![Value::I32(v)],
            ArgSpec::I64(v) => vec![Value::I64(v)],
            ArgSpec::F32(b) => vec![Value::F32(b)],
            ArgSpec::F64(b) => vec![Value::F64(b)],
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RetMode {
    Exact,
    /// Arithmetic-NaN suite: if interpreter returns NaN, x64 must return a quiet/arithmetic NaN
    /// of the same float width; non-NaN values must still match bit-for-bit!
    ArithNanF32,
    ArithNanF64,
}

#[derive(Clone, Debug)]
struct OracleCall {
    export: String,
    arg: ArgSpec,
    ret_mode: RetMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Ok(u64),
    Trap(u8),
}

fn wtime_err_to_trap(err: &wtime_types::Error) -> u8 {
    match (err.kind, err.trap_code) {
        (ErrorKind::Host, _) => 19,
        (_, Some(TrapCode::Unreachable)) => 1,
        (_, Some(TrapCode::MemoryOutOfBounds)) => 2,
        (_, Some(TrapCode::TableOutOfBounds)) => 3,
        (_, Some(TrapCode::IndirectCallTypeMismatch | TrapCode::NullReference)) => 4,
        (_, Some(TrapCode::IntegerDivisionByZero)) => 5,
        (_, Some(TrapCode::IntegerOverflow)) => 6,
        (_, Some(TrapCode::InvalidConversionToInteger)) => 7,
        _ => panic!("unexpected wtime-runtime error: {err:?}"),
    }
}

fn val_to_u64(v: Value) -> u64 {
    match v {
        Value::I32(x) => u64::from(x as u32),
        Value::I64(x) => x as u64,
        Value::F32(b) => u64::from(b),
        Value::F64(b) => b,
        Value::Ref(_) => 0,
        Value::V128(_) => panic!("v128 should be wrapped via scalar/memory export"),
    }
}

fn is_f32_nan(bits: u32) -> bool {
    (bits & 0x7f80_0000) == 0x7f80_0000 && (bits & 0x007f_ffff) != 0
}
fn is_f32_qnan(bits: u32) -> bool {
    (bits & 0x7fc0_0000) == 0x7fc0_0000
}
fn is_f64_nan(bits: u64) -> bool {
    (bits & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000 && (bits & 0x000f_ffff_ffff_ffff) != 0
}
fn is_f64_qnan(bits: u64) -> bool {
    (bits & 0x7ff8_0000_0000_0000) == 0x7ff8_0000_0000_0000
}

type CallbackEvent = (u32, u64, u64);

#[derive(Clone, Debug)]
struct RunTrace {
    outcomes: Vec<(Outcome, Vec<CallbackEvent>)>,
    globals: Vec<(String, u64)>,
    mem_slices: Vec<(usize, Vec<u8>)>,
}

fn run_wtime_oracle(
    wasm: &[u8],
    calls: &[OracleCall],
    check_globals: &[&str],
    check_mem: &[(u64, usize)],
) -> RunTrace {
    let cfg = Config {
        features: x64_validate_features(),
        limits: x64_validate_limits(),
    };
    let engine = Engine::new(cfg).unwrap();
    let module = engine.compile(wasm).unwrap();
    let mut store = Store::new(&engine);
    let mut linker = Linker::new(&engine);
    let cb_log: Arc<Mutex<Vec<(u32, u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));

    for (idx, (mname, iname, _, fty)) in module.function_imports().into_iter().enumerate() {
        let idx_u32 = idx as u32;
        let log = Arc::clone(&cb_log);
        let fty_owned: FuncType = fty.clone();
        let hf = store
            .create_host_func(fty_owned.clone(), move |args| {
                let (tok, arg_bits) = match (fty_owned.params.as_slice(), args) {
                    ([], []) => (0u64, 0u64),
                    ([ValType::I32], [Value::I32(v)]) => (0u64, u64::from(*v as u32)),
                    ([ValType::F32], [Value::F32(b)]) => (0u64, u64::from(*b)),
                    ([ValType::Ref(_)], [_]) => (0u64, 0u64),
                    ([ValType::Ref(_), ValType::I32], [_, Value::I32(v)]) => {
                        (0u64, u64::from(*v as u32))
                    }
                    ([ValType::Ref(_), ValType::I64], [_, Value::I64(v)]) => (0u64, *v as u64),
                    _ => (0u64, 0u64),
                };
                log.lock().unwrap().push((idx_u32, tok, arg_bits));
                if idx_u32 == 1 {
                    return Err(wtime_types::Error::new(
                        ErrorKind::Host,
                        "simulated host failure",
                    ));
                }
                let res_u64 = (arg_bits ^ 0x55) + u64::from(idx_u32) * 1000;
                let out = match fty_owned.results.as_slice() {
                    [] => vec![],
                    [ValType::I32] => vec![Value::I32(res_u64 as u32 as i32)],
                    [ValType::I64] => vec![Value::I64(res_u64 as i64)],
                    [ValType::F32] => vec![Value::F32(res_u64 as u32)],
                    [ValType::F64] => vec![Value::F64(res_u64)],
                    _ => vec![],
                };
                Ok(out)
            })
            .unwrap();
        linker.define(mname, iname, External::Func(hf)).unwrap();
    }

    let inst = linker.instantiate(&mut store, &module).unwrap();
    let mut outcomes = Vec::with_capacity(calls.len());
    for c in calls {
        cb_log.lock().unwrap().clear();
        let External::Func(f) = inst.export(&store, &c.export).unwrap() else {
            panic!("export {} is not a func", c.export);
        };
        let outcome = match store.invoke(f, &c.arg.to_wtime_args()) {
            Ok(RunOutcome::Returned(vals)) => {
                let bits = vals.first().copied().map_or(0, val_to_u64);
                Outcome::Ok(bits)
            }
            Ok(RunOutcome::Exception(_)) => Outcome::Trap(1),
            Err(e) => Outcome::Trap(wtime_err_to_trap(&e)),
            other => panic!("unexpected outcome {other:?}"),
        };
        let trace = cb_log.lock().unwrap().clone();
        outcomes.push((outcome, trace));
    }

    let mut globals = Vec::new();
    for &gname in check_globals {
        let External::Global(g) = inst.export(&store, gname).unwrap() else {
            panic!("export {gname} is not a global");
        };
        let v = store.get_global(g).unwrap();
        globals.push((gname.to_string(), val_to_u64(v)));
    }

    let mut mem_slices = Vec::new();
    if !check_mem.is_empty() {
        let External::Memory(m) = inst.export(&store, "blot:memory").unwrap() else {
            panic!("missing blot:memory export");
        };
        for &(off, len) in check_mem {
            let bytes = store.read_memory(m, off, len).unwrap();
            mem_slices.push((len, bytes));
        }
    }

    RunTrace {
        outcomes,
        globals,
        mem_slices,
    }
}

fn run_blot_so(
    driver: &Path,
    so_path: &Path,
    calls: &[OracleCall],
    check_globals: &[&str],
    check_mem: &[(u64, usize)],
    host_round_down: bool,
) -> RunTrace {
    let mut script = String::new();
    for c in calls {
        script.push_str(&format!("call {} {:016x}\n", c.export, c.arg.to_u64()));
    }
    for &g in check_globals {
        script.push_str(&format!("getg {g}\n"));
    }
    for &(off, len) in check_mem {
        script.push_str(&format!("mem {off} {len}\n"));
    }

    let mut command = Command::new(driver);
    command.arg(so_path);
    if host_round_down {
        command.arg("--round-down");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oracle driver");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "oracle driver crashed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("NEW_OK"),
        "blot_aot_new failed: {stdout}"
    );

    let mut outcomes = Vec::with_capacity(calls.len());
    for _ in calls {
        let line = lines.next().expect("missing CALL line");
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts[0], "CALL");
        let rc: i32 = parts[1].parse().unwrap();
        let res = u64::from_str_radix(parts[2], 16).unwrap();
        let trap: u8 = parts[3].parse().unwrap();
        let nlog: usize = parts[4].parse().unwrap();
        let mut trace = Vec::with_capacity(nlog);
        for item in &parts[5..5 + nlog] {
            let mut sp = item.split(':');
            let idx: u32 = sp.next().unwrap().parse().unwrap();
            let tok = u64::from_str_radix(sp.next().unwrap(), 16).unwrap();
            let arg = u64::from_str_radix(sp.next().unwrap(), 16).unwrap();
            trace.push((idx, tok, arg));
        }
        let outcome = if rc == 0 {
            assert_eq!(trap, 0);
            Outcome::Ok(res)
        } else {
            assert!(rc == 1 || rc == 2, "unexpected call rc={rc}");
            // Verify result out was untouched on trap!
            assert_eq!(res, 0xdeadbeefcafebabe, "res_out was clobbered on trap!");
            Outcome::Trap(trap)
        };
        outcomes.push((outcome, trace));
    }

    let mut globals = Vec::new();
    for &g in check_globals {
        let line = lines.next().expect("missing GLOBAL line");
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts[0], "GLOBAL");
        assert_eq!(parts[1], "0", "get_global({g}) failed");
        let val = u64::from_str_radix(parts[3], 16).unwrap();
        globals.push((g.to_string(), val));
    }

    let mut mem_slices = Vec::new();
    for &(_, len) in check_mem {
        let line = lines.next().expect("missing MEM line");
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts[0], "MEM");
        let hex = parts.get(2).copied().unwrap_or("");
        let mut bytes = Vec::with_capacity(len);
        for i in 0..len {
            let b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
            bytes.push(b);
        }
        mem_slices.push((len, bytes));
    }

    RunTrace {
        outcomes,
        globals,
        mem_slices,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn assert_oracle_matrix(
    name: &str,
    wasm: &[u8],
    calls: &[OracleCall],
    check_globals: &[&str],
    check_mem: &[(u64, usize)],
) {
    let Some(driver) = get_oracle_driver() else {
        return;
    };
    let expected = run_wtime_oracle(wasm, calls, check_globals, check_mem);
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-oracle");
    let cells = [
        ("unopt-v2", false, false),
        ("unopt-v3", false, true),
        ("opt-v2", true, false),
        ("opt-v3", true, true),
    ];
    for (tag, opt, v3) in cells {
        if v3
            && (!std::is_x86_feature_detected!("lzcnt")
                || !std::is_x86_feature_detected!("bmi1")
                || !std::is_x86_feature_detected!("bmi2")
                || !std::is_x86_feature_detected!("avx2"))
        {
            continue;
        }
        let so_bytes = compile_wasm_to_x64_so(
            wasm,
            &X64CompileOptions {
                isa: TargetIsa {
                    has_abm_bmi1: v3,
                    has_bmi2: v3,
                    has_avx2: v3,
                },
                jobs: 2,
                deadline: None,
                opt,
                verify: true, // Runs BOTH verify_ir and verify_alloc on every function!
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("[{name}/{tag}] compile failed: {e}"));
        let so_path = root.0.join(format!("{name}_{tag}.so"));
        crate::build::assert_inside_test_root(&root.0, &so_path);
        fs::write(&so_path, &so_bytes).unwrap();
        let actual = run_blot_so(driver, &so_path, calls, check_globals, check_mem, false);
        for (i, ((exp_o, exp_tr), (act_o, act_tr))) in
            expected.outcomes.iter().zip(&actual.outcomes).enumerate()
        {
            assert_eq!(
                act_tr, exp_tr,
                "[{name}/{tag}] call #{i} ({}) callback trace mismatch",
                calls[i].export
            );
            match (exp_o, act_o, calls[i].ret_mode) {
                (Outcome::Ok(eb), Outcome::Ok(ab), RetMode::Exact) => {
                    assert_eq!(
                        ab, eb,
                        "[{name}/{tag}] call #{i} ({}, arg={:?}): x64={ab:#018x} != wtime={eb:#018x}",
                        calls[i].export, calls[i].arg
                    );
                }
                (Outcome::Ok(eb), Outcome::Ok(ab), RetMode::ArithNanF32) => {
                    let e32 = *eb as u32;
                    let a32 = *ab as u32;
                    assert_eq!(
                        *ab >> 32,
                        0,
                        "[{name}/{tag}] call #{i} upper 32 bits non-zero for f32"
                    );
                    if is_f32_nan(e32) {
                        assert!(
                            is_f32_qnan(a32),
                            "[{name}/{tag}] call #{i} expected quiet arithmetic NaN f32, got {a32:#010x}"
                        );
                    } else {
                        assert_eq!(a32, e32, "[{name}/{tag}] call #{i} f32 mismatch");
                    }
                }
                (Outcome::Ok(eb), Outcome::Ok(ab), RetMode::ArithNanF64) => {
                    if is_f64_nan(*eb) {
                        assert!(
                            is_f64_qnan(*ab),
                            "[{name}/{tag}] call #{i} expected quiet arithmetic NaN f64, got {ab:#018x}"
                        );
                    } else {
                        assert_eq!(ab, eb, "[{name}/{tag}] call #{i} f64 mismatch");
                    }
                }
                (Outcome::Trap(et), Outcome::Trap(at), _) => {
                    assert_eq!(
                        at, et,
                        "[{name}/{tag}] call #{i} ({}) trap mismatch: x64={at} != wtime={et}",
                        calls[i].export
                    );
                }
                _ => panic!(
                    "[{name}/{tag}] call #{i} ({}, arg={:?}) outcome mismatch: x64={act_o:?} != wtime={exp_o:?}",
                    calls[i].export, calls[i].arg
                ),
            }
        }
        assert_eq!(
            actual.globals, expected.globals,
            "[{name}/{tag}] post-run globals mismatch"
        );
        assert_eq!(
            actual.mem_slices, expected.mem_slices,
            "[{name}/{tag}] post-run memory mismatch"
        );
    }
}

fn c(export: &str, arg: ArgSpec) -> OracleCall {
    OracleCall {
        export: export.to_string(),
        arg,
        ret_mode: RetMode::Exact,
    }
}

// ---- Suite 1: Integer arithmetic, bit ops, shifts/rotates (RCX dying + spilled), div/rem, sign-ext ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_int_arith_bits_shifts_div_rem_and_sign_ext() {
    // f0: i32 bitops (clz, ctz, popcnt, extend8_s, extend16_s) -> packed checksum
    let mut b0 = Vec::new();
    // (clz(x) * 31 + ctz(x)) ^ (popcnt(x) << 8) ^ extend8_s(x) ^ extend16_s(x)
    b0.extend_from_slice(&[0x20, 0, 0x67, 0x41, 31, 0x6c, 0x20, 0, 0x68, 0x6a]);
    b0.extend_from_slice(&[0x20, 0, 0x69, 0x41, 8, 0x74, 0x73]);
    b0.extend_from_slice(&[0x20, 0, 0xc0, 0x73, 0x20, 0, 0xc1, 0x73]);

    // f1: i64 bitops (clz, ctz, popcnt, extend8_s, extend16_s, extend32_s)
    let mut b1 = Vec::new();
    b1.extend_from_slice(&[0x20, 0, 0x79, 0x20, 0, 0x7a, 0x7c, 0x20, 0, 0x7b, 0x85]);
    b1.extend_from_slice(&[
        0x20, 0, 0xc2, 0x85, 0x20, 0, 0xc3, 0x85, 0x20, 0, 0xc4, 0x85,
    ]);

    // f2: variable shifts/rotates under high GP register pressure (forces RCX reuse & spilled shift count!)
    let mut b2 = Vec::new();
    // Compute 12 live temps to force spills, then shift/rotate each with variable counts.
    for i in 1..=12i32 {
        b2.push(0x20);
        b2.push(0);
        b2.push(0x41);
        leb_i32(&mut b2, i * 17);
        b2.push(0x6a);
        b2.push(0x21);
        leb_u32(&mut b2, i as u32);
    }
    // Combine shl, shr_s, shr_u, rotl, rotr using local[i] as value and local[13 - i] as shift count.
    b2.extend_from_slice(&[0x41, 0]); // acc = 0
    for i in 1..=10u32 {
        b2.push(0x20);
        leb_u32(&mut b2, i);
        b2.push(0x20);
        leb_u32(&mut b2, 13 - i);
        let shift_op = match i % 5 {
            0 => 0x74, // shl
            1 => 0x75, // shr_s
            2 => 0x76, // shr_u
            3 => 0x77, // rotl
            _ => 0x78, // rotr
        };
        b2.push(shift_op);
        b2.push(0x73); // xor into acc
    }
    // Also exercise 64-bit shift/rotate with value dying right at the shift op:
    b2.extend_from_slice(&[0x20, 11, 0xac, 0x20, 12, 0xad, 0x89, 0xa7, 0x73]); // rotl64 -> wrap -> xor

    // f3: i32.div_s(x, d) via x packed: hi16 = mode (0=div_s,1=rem_s,2=div_u,3=rem_u,4=div_s_min,5=rem_s_min)
    let mut b3 = Vec::new();
    // mode = x & 7; if mode == 0: i32.div_s(i32::MIN, -1) [trap 6]
    // mode == 1: i32.rem_s(i32::MIN, -1) [0!]
    // mode == 2: i32.div_s(42, 0) [trap 5]
    // mode == 3: i32.div_u(0xffff_ffff, 3)
    // mode == 4: i32.rem_u(0xffff_ffff, 7)
    // else: i64.rem_s(i64::MIN, -1) -> wrap [0!]
    b3.extend_from_slice(&[0x20, 0, 0x41, 0, 0x46, 0x04, 0x7f]);
    b3.push(0x41);
    leb_i32(&mut b3, i32::MIN);
    b3.push(0x41);
    leb_i32(&mut b3, -1);
    b3.push(0x6d);
    b3.extend_from_slice(&[0x05, 0x20, 0, 0x41, 1, 0x46, 0x04, 0x7f]);
    b3.push(0x41);
    leb_i32(&mut b3, i32::MIN);
    b3.push(0x41);
    leb_i32(&mut b3, -1);
    b3.push(0x6f);
    b3.extend_from_slice(&[0x05, 0x20, 0, 0x41, 2, 0x46, 0x04, 0x7f]);
    b3.extend_from_slice(&[0x41, 42, 0x41, 0, 0x6d]);
    b3.extend_from_slice(&[0x05, 0x20, 0, 0x41, 3, 0x46, 0x04, 0x7f]);
    b3.push(0x41);
    leb_i32(&mut b3, -1);
    b3.extend_from_slice(&[0x41, 3, 0x6e]);
    b3.extend_from_slice(&[0x05, 0x20, 0, 0x41, 4, 0x46, 0x04, 0x7f]);
    b3.push(0x41);
    leb_i32(&mut b3, -1);
    b3.extend_from_slice(&[0x41, 7, 0x70]);
    b3.extend_from_slice(&[0x05]);
    b3.push(0x42);
    leb_i64(&mut b3, i64::MIN);
    b3.push(0x42);
    leb_i64(&mut b3, -1);
    b3.extend_from_slice(&[0x81, 0xa7]);
    b3.extend_from_slice(&[0x0b, 0x0b, 0x0b, 0x0b, 0x0b]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e])]),
        func_sec(&[0, 1, 0, 0]),
        export_sec(&[
            ("bits32", 0, 0),
            ("bits64", 0, 1),
            ("shifts", 0, 2),
            ("divrem", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[], &b1),
            enc_body(&[(12, 0x7f)], &b2),
            enc_body(&[], &b3),
        ]),
    ]);

    let calls = vec![
        c("bits32", ArgSpec::I32(0)),
        c("bits32", ArgSpec::I32(1)),
        c("bits32", ArgSpec::I32(i32::MIN)),
        c("bits32", ArgSpec::I32(-1)),
        c("bits32", ArgSpec::I32(0x1234_80ff)),
        c("bits64", ArgSpec::I64(0)),
        c("bits64", ArgSpec::I64(1)),
        c("bits64", ArgSpec::I64(i64::MIN)),
        c("bits64", ArgSpec::I64(-1)),
        c("bits64", ArgSpec::I64(0x0102_0304_80ff_0010)),
        c("shifts", ArgSpec::I32(0x1357_9bdf)),
        c("shifts", ArgSpec::I32(-777)),
        c("divrem", ArgSpec::I32(0)), // trap 6 (INT_MIN / -1)
        c("divrem", ArgSpec::I32(1)), // 0 (INT_MIN % -1)
        c("divrem", ArgSpec::I32(2)), // trap 5 (div0)
        c("divrem", ArgSpec::I32(3)), // div_u
        c("divrem", ArgSpec::I32(4)), // rem_u
        c("divrem", ArgSpec::I32(5)), // i64.rem_s(MIN, -1) == 0
    ];
    assert_oracle_matrix("int_ops", &wasm, &calls, &[], &[]);
}

// ---- Suite 2: All 16 float-to-int truncations + spilled u64->fp + FP min/max/cmp + NaN sets ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_all_16_truncs_spilled_u64_to_fp_and_nan_semantics() {
    let mut sigs: Vec<(&[u8], &[u8])> = vec![
        (&[0x7d], &[0x7f]), // 0: f32 -> i32
        (&[0x7c], &[0x7f]), // 1: f64 -> i32
        (&[0x7d], &[0x7e]), // 2: f32 -> i64
        (&[0x7c], &[0x7e]), // 3: f64 -> i64
        (&[0x7e], &[0x7d]), // 4: i64 -> f32
        (&[0x7e], &[0x7c]), // 5: i64 -> f64
        (&[0x7d], &[0x7d]), // 6: f32 -> f32
        (&[0x7c], &[0x7c]), // 7: f64 -> f64
    ];
    let _ = &mut sigs;

    let trunc_specs: [(&str, u32, &[u8]); 16] = [
        ("t_i32_f32_s", 0, &[0x20, 0, 0xa8]),
        ("t_i32_f32_u", 0, &[0x20, 0, 0xa9]),
        ("t_i32_f64_s", 1, &[0x20, 0, 0xaa]),
        ("t_i32_f64_u", 1, &[0x20, 0, 0xab]),
        ("t_i64_f32_s", 2, &[0x20, 0, 0xae]),
        ("t_i64_f32_u", 2, &[0x20, 0, 0xaf]),
        ("t_i64_f64_s", 3, &[0x20, 0, 0xb0]),
        ("t_i64_f64_u", 3, &[0x20, 0, 0xb1]),
        ("t_i32_f32_ss", 0, &[0x20, 0, 0xfc, 0x00]),
        ("t_i32_f32_su", 0, &[0x20, 0, 0xfc, 0x01]),
        ("t_i32_f64_ss", 1, &[0x20, 0, 0xfc, 0x02]),
        ("t_i32_f64_su", 1, &[0x20, 0, 0xfc, 0x03]),
        ("t_i64_f32_ss", 2, &[0x20, 0, 0xfc, 0x04]),
        ("t_i64_f32_su", 2, &[0x20, 0, 0xfc, 0x05]),
        ("t_i64_f64_ss", 3, &[0x20, 0, 0xfc, 0x06]),
        ("t_i64_f64_su", 3, &[0x20, 0, 0xfc, 0x07]),
    ];

    let mut ftypes: Vec<u32> = trunc_specs.iter().map(|s| s.1).collect();
    let mut exports: Vec<(&str, u8, u32)> = trunc_specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.0, 0, i as u32))
        .collect();
    let mut bodies: Vec<Vec<u8>> = trunc_specs.iter().map(|s| enc_body(&[], s.2)).collect();

    // f16: spilled f32.convert_i64_u under GP register pressure (specifically tests sticky-bit capture!)
    let mut b_sp_f32 = Vec::new();
    for i in 1..=11u32 {
        b_sp_f32.push(0x20);
        b_sp_f32.push(0);
        b_sp_f32.push(0x42);
        leb_i64(&mut b_sp_f32, i as i64);
        b_sp_f32.push(0x7c);
        b_sp_f32.push(0x21);
        leb_u32(&mut b_sp_f32, i);
    }
    // Convert local 0 (live across 11 live GP locals -> forced spill!), then add (sum_of_11 & 0) as f32.
    b_sp_f32.extend_from_slice(&[0x20, 0, 0xb5]);
    b_sp_f32.extend_from_slice(&[0x20, 1]);
    for i in 2..=11u32 {
        b_sp_f32.push(0x20);
        leb_u32(&mut b_sp_f32, i);
        b_sp_f32.push(0x85); // xor
    }
    b_sp_f32.extend_from_slice(&[0x42, 0, 0x83, 0xb4, 0x92]); // & 0 -> convert -> f32.add (preserves exact bits!)
    ftypes.push(4);
    exports.push(("spilled_u64_f32", 0, bodies.len() as u32));
    bodies.push(enc_body(&[(11, 0x7e)], &b_sp_f32));

    // f17: spilled f64.convert_i64_u
    let mut b_sp_f64 = Vec::new();
    for i in 1..=11u32 {
        b_sp_f64.push(0x20);
        b_sp_f64.push(0);
        b_sp_f64.push(0x42);
        leb_i64(&mut b_sp_f64, i as i64);
        b_sp_f64.push(0x7c);
        b_sp_f64.push(0x21);
        leb_u32(&mut b_sp_f64, i);
    }
    b_sp_f64.extend_from_slice(&[0x20, 0, 0xba]); // f64.convert_i64_u
    b_sp_f64.extend_from_slice(&[0x20, 1]);
    for i in 2..=11u32 {
        b_sp_f64.push(0x20);
        leb_u32(&mut b_sp_f64, i);
        b_sp_f64.push(0x85);
    }
    b_sp_f64.extend_from_slice(&[0x42, 0, 0x83, 0xb9, 0xa0]);
    ftypes.push(5);
    exports.push(("spilled_u64_f64", 0, bodies.len() as u32));
    bodies.push(enc_body(&[(11, 0x7e)], &b_sp_f64));

    // f18: f32.min(x, +0.0) and f19: f32.max(x, -0.0) (tests -0.0/+0.0 AND NaN propagation!)
    let mut b_min32 = vec![0x20, 0, 0x43];
    b_min32.extend_from_slice(&0.0f32.to_bits().to_le_bytes());
    b_min32.push(0x96); // f32.min
    ftypes.push(6);
    exports.push(("min32_p0", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_min32));

    let mut b_max32 = vec![0x20, 0, 0x43];
    b_max32.extend_from_slice(&(-0.0f32).to_bits().to_le_bytes());
    b_max32.push(0x97); // f32.max
    ftypes.push(6);
    exports.push(("max32_n0", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_max32));

    // f20: f64.min(x, +0.0) and f21: f64.add(x, 1.0) (arithmetic NaN suite)
    let mut b_min64 = vec![0x20, 0, 0x44];
    b_min64.extend_from_slice(&0.0f64.to_bits().to_le_bytes());
    b_min64.push(0xa4);
    ftypes.push(7);
    exports.push(("min64_p0", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_min64));

    let mut b_add64 = vec![0x20, 0, 0x44];
    b_add64.extend_from_slice(&1.0f64.to_bits().to_le_bytes());
    b_add64.push(0xa0);
    ftypes.push(7);
    exports.push(("add64_1", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_add64));

    // f22: exact sign-bit ops (f32.abs/neg/copysign) preserve NaN payloads deterministically!
    let mut b_sign32 = vec![0x20, 0, 0x8c, 0x43]; // f32.neg(x)
    b_sign32.extend_from_slice(&(-1.0f32).to_bits().to_le_bytes());
    b_sign32.push(0x98); // copysign(neg(x), -1.0)
    b_sign32.push(0x8b); // abs
    ftypes.push(6);
    exports.push(("sign32_exact", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_sign32));

    // f23: unordered f64 comparisons packed into 6-bit mask (eq|ne|lt|gt|le|ge with 1.0)
    let mut b_fcmp = Vec::new();
    for (bit, op) in [0x61u8, 0x62, 0x63, 0x64, 0x65, 0x66]
        .into_iter()
        .enumerate()
    {
        b_fcmp.push(0x20);
        b_fcmp.push(0);
        b_fcmp.push(0x44);
        b_fcmp.extend_from_slice(&1.0f64.to_bits().to_le_bytes());
        b_fcmp.push(op);
        if bit > 0 {
            b_fcmp.extend_from_slice(&[0x41, bit as u8, 0x74, 0x72]); // << bit | acc
        }
    }
    ftypes.push(1);
    exports.push(("fcmp64_mask", 0, bodies.len() as u32));
    bodies.push(enc_body(&[], &b_fcmp));

    let wasm = wasm_module(&[
        type_sec(&sigs),
        func_sec(&ftypes),
        export_sec(&exports),
        code_sec(&bodies),
    ]);

    let f32_cases: [u32; 16] = [
        0.0f32.to_bits(),
        (-0.0f32).to_bits(),
        (-0.5f32).to_bits(),
        (-0.99999994f32).to_bits(),
        (-1.0f32).to_bits(),
        1.5f32.to_bits(),
        f32::from_bits(1).to_bits(), // subnormal
        (-2147483648.0f32).to_bits(),
        (-2147483904.0f32).to_bits(),
        2147483520.0f32.to_bits(),
        2147483648.0f32.to_bits(),
        4294967296.0f32.to_bits(),
        (-9223372036854775808.0f32).to_bits(),
        9223372036854775808.0f32.to_bits(),
        18446744073709551616.0f32.to_bits(),
        0x7f80_0001, // sNaN
    ];
    let f64_cases: [u64; 18] = [
        0.0f64.to_bits(),
        (-0.0f64).to_bits(),
        (-0.5f64).to_bits(),
        (-0.9999999999f64).to_bits(),
        (-1.0f64).to_bits(),
        (-2147483648.5f64).to_bits(), // MUST -> INT32_MIN for I32F64S!
        (-2147483649.0f64).to_bits(), // MUST trap 6 for I32F64S!
        2147483647.9f64.to_bits(),
        2147483648.0f64.to_bits(),
        4294967295.9f64.to_bits(),
        4294967296.0f64.to_bits(),
        (-9223372036854775808.0f64).to_bits(),
        (-9223372036854777856.0f64).to_bits(),
        9223372036854774784.0f64.to_bits(),
        9223372036854775808.0f64.to_bits(),
        18446744073709549568.0f64.to_bits(),
        18446744073709551616.0f64.to_bits(),
        0x7ff0_0000_0000_0001, // sNaN
    ];

    let mut calls = Vec::new();
    for &(name, sig, _) in &trunc_specs {
        if sig == 0 || sig == 2 {
            for &b in &f32_cases {
                calls.push(c(name, ArgSpec::F32(b)));
            }
            calls.push(c(name, ArgSpec::F32(f32::INFINITY.to_bits())));
            calls.push(c(name, ArgSpec::F32(f32::NEG_INFINITY.to_bits())));
            calls.push(c(name, ArgSpec::F32(0x7fc0_1234))); // qNaN
        } else {
            for &b in &f64_cases {
                calls.push(c(name, ArgSpec::F64(b)));
            }
            calls.push(c(name, ArgSpec::F64(f64::INFINITY.to_bits())));
            calls.push(c(name, ArgSpec::F64(f64::NEG_INFINITY.to_bits())));
            calls.push(c(name, ArgSpec::F64(0x7ff8_1234_5678_9abc))); // qNaN
        }
    }

    // Spilled u64 -> fp sticky-bit cases from §4.3.4:
    calls.push(c(
        "spilled_u64_f32",
        ArgSpec::I64(0x8000_0080_0000_0001u64 as i64),
    ));
    calls.push(c(
        "spilled_u64_f32",
        ArgSpec::I64(0x8000_0080_0000_0000u64 as i64),
    ));
    calls.push(c(
        "spilled_u64_f64",
        ArgSpec::I64(0x8000_0000_0000_0401u64 as i64),
    ));
    calls.push(c(
        "spilled_u64_f64",
        ArgSpec::I64(0x8000_0000_0000_0400u64 as i64),
    ));

    // -0.0 / +0.0 exact min/max + arithmetic NaN outcome set checks:
    calls.push(c("min32_p0", ArgSpec::F32((-0.0f32).to_bits())));
    calls.push(c("max32_n0", ArgSpec::F32(0.0f32.to_bits())));
    calls.push(OracleCall {
        export: "min32_p0".into(),
        arg: ArgSpec::F32(0x7f80_0001),
        ret_mode: RetMode::ArithNanF32,
    });
    calls.push(OracleCall {
        export: "max32_n0".into(),
        arg: ArgSpec::F32(0x7fc1_2345),
        ret_mode: RetMode::ArithNanF32,
    });
    calls.push(OracleCall {
        export: "min64_p0".into(),
        arg: ArgSpec::F64(0x7ff0_0000_0000_0001),
        ret_mode: RetMode::ArithNanF64,
    });
    calls.push(OracleCall {
        export: "add64_1".into(),
        arg: ArgSpec::F64(0x7ff0_0000_0000_0005),
        ret_mode: RetMode::ArithNanF64,
    });
    calls.push(c("sign32_exact", ArgSpec::F32(0xffc1_2345))); // payload preserved bit-for-bit!
    calls.push(c("fcmp64_mask", ArgSpec::F64(0x7ff8_0000_0000_0000))); // NaN -> only NE bit set (0b000010 = 2)
    calls.push(c("fcmp64_mask", ArgSpec::F64(1.0f64.to_bits())));

    assert_oracle_matrix("trunc_fp", &wasm, &calls, &[], &[]);
}

// ---- Suite 3: Multi-value ABI (10 GP args, 10 V128 args, 24 i64 overflow returns, recursive),
//               V128 subset, br_table, implicit-else + loop-carried locals, live Xmm0 across helpers ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_multivalue_v128_cfg_and_live_xmm_across_helpers() {
    // Type 0: (i32) -> (i32)
    // Type 1: 10 i64 args -> 24 i64 overflow results!
    // Type 2: 10 v128 args -> (i64) (9th & 10th v128 stack args non-overlapping!)
    // Type 3: (i32, i32) -> (i32, i32) (recursive multi-value fib-pair!)
    // Type 4: (f64) -> (f64)
    let ten_i64 = [0x7eu8; 10];
    let twentyfour_i64 = [0x7eu8; 24];
    let ten_v128 = [0x7bu8; 10];

    // f0 (type 1): returns 24 distinct i64 values derived from all 10 i64 params (4 on stack!).
    let mut b0 = Vec::new();
    for r in 0..24i64 {
        b0.push(0x20);
        leb_u32(&mut b0, (r % 10) as u32);
        b0.push(0x42);
        leb_i64(&mut b0, r * 101 + 7);
        b0.push(0x7c); // i64.add
    }

    // f1 (type 0): calls f0 with 10 i64 args, folds all 24 returned i64 values into an i32 checksum!
    let mut b1 = Vec::new();
    for p in 0..10i64 {
        b1.push(0x20);
        b1.push(0);
        b1.push(0xac); // extend_i32_s
        b1.push(0x42);
        leb_i64(&mut b1, p * 13 + 3);
        b1.push(0x7c);
    }
    b1.extend_from_slice(&[0x10, 0]); // call f0 -> 24 i64s on stack
    // Fold all 24 i64s: mix = mix * 31 + top
    for _ in 0..23 {
        b1.push(0x42);
        leb_i64(&mut b1, 31);
        b1.push(0x7e); // mul
        b1.push(0x7c); // add
    }
    b1.push(0xa7); // i32.wrap_i64

    // f2 (type 2): 10 v128 args -> mixes extract_lane(0) and extract_lane(1) of all 10 v128 args (including 9th & 10th on stack!)
    let mut b2 = Vec::new();
    b2.extend_from_slice(&[0x42, 0]); // acc
    for p in 0..10u32 {
        b2.push(0x20);
        leb_u32(&mut b2, p);
        b2.extend_from_slice(&[0xfd, 0x1d, 0, 0x7c]); // i64x2.extract_lane 0; add
        b2.push(0x20);
        leb_u32(&mut b2, p);
        b2.extend_from_slice(&[0xfd, 0x1d, 1, 0x85]); // i64x2.extract_lane 1; xor
    }

    // f3 (type 0): constructs 10 distinct v128 values using const, splat (1/2/4/8), replace_lane, load/store -> calls f2
    let mut b3 = Vec::new();
    for p in 0..10u32 {
        // v128.const + replace_lane 0 and 1
        b3.extend_from_slice(&[0xfd, 0x0c]);
        b3.extend_from_slice(&[p as u8; 16]);
        b3.push(0x20);
        b3.push(0);
        b3.push(0xac);
        b3.push(0x42);
        leb_i64(&mut b3, (p as i64) * 1000 + 1);
        b3.push(0x7c);
        b3.extend_from_slice(&[0xfd, 0x1e, 0]); // replace_lane 0
        b3.push(0x20);
        b3.push(0);
        b3.push(0xac);
        b3.push(0x42);
        leb_i64(&mut b3, (p as i64) * 1000 + 2);
        b3.push(0x7c);
        b3.extend_from_slice(&[0xfd, 0x1e, 1]); // replace_lane 1
    }
    // Also exercise i8x16/i16x8/i32x4/i64x2 splat + v128.store + v128.load at offset 0:
    b3.extend_from_slice(&[0x10, 2]); // call f2 -> i64
    b3.extend_from_slice(&[0x41, 0, 0x20, 0, 0xfd, 0x0f, 0xfd, 0x0b, 0, 0]); // store i8x16.splat(arg)
    b3.extend_from_slice(&[0x41, 16, 0x20, 0, 0xfd, 0x10, 0xfd, 0x0b, 0, 0]); // store i16x8.splat(arg)
    b3.extend_from_slice(&[0x41, 32, 0x20, 0, 0xfd, 0x11, 0xfd, 0x0b, 0, 0]); // store i32x4.splat(arg)
    b3.extend_from_slice(&[0x41, 48, 0x20, 0, 0xac, 0xfd, 0x12, 0xfd, 0x0b, 0, 0]); // store i64x2.splat(arg)
    b3.extend_from_slice(&[0x41, 0, 0xfd, 0x00, 0, 0, 0xfd, 0x1d, 0, 0x85]); // load 0 -> lane 0 xor
    b3.extend_from_slice(&[0x41, 16, 0xfd, 0x00, 0, 0, 0xfd, 0x1d, 1, 0x85]); // load 16 -> lane 1 xor
    b3.extend_from_slice(&[0x41, 32, 0xfd, 0x00, 0, 0, 0xfd, 0x1d, 0, 0x85]); // load 32 -> lane 0 xor
    b3.extend_from_slice(&[0x41, 48, 0xfd, 0x00, 0, 0, 0xfd, 0x1d, 1, 0x85]); // load 48 -> lane 1 xor
    b3.push(0xa7); // wrap_i64

    // f4 (type 3): recursive multi-value (n, acc) -> (fib(n), sum)
    let mut b4 = Vec::new();
    b4.extend_from_slice(&[
        0x20, 0, 0x41, 1, 0x4c, // n <= 1
        0x04, 0x40, 0x20, 0, 0x20, 1, 0x0f, // return (n, acc)
        0x0b, 0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x20, 0, 0x6a, 0x10,
        4, // (a, b) = f4(n-1, acc+n)
        0x21, 3, 0x21, 2, // l3=b, l2=a
        0x20, 2, 0x20, 0, 0x6a, 0x20, 3,
    ]);
    // f5 (type 0): wrapper calling f4(n, 0) + implicit-else + loop-carried local + br_table
    let mut b5 = Vec::new();
    b5.extend_from_slice(&[0x20, 0, 0x41, 0, 0x10, 4, 0x6a, 0x21, 1]); // l1 = sum of recursive mv
    // loop carrying l1 with implicit-else `if (l2 & 1) { l1 += 3 }` and unreachable block + br_table
    b5.extend_from_slice(&[
        0x41, 5, 0x21, 2, // l2 = 5
        0x03, 0x40, // loop
        0x20, 2, 0x41, 1, 0x71, 0x04, 0x40, // if (l2 & 1) (implicit else!)
        0x20, 1, 0x41, 3, 0x6a, 0x21, 1, 0x0b,
        // br_table inside block(result i32): 0->10, 1->20, default->30
        0x02, 0x7f, 0x02, 0x40, 0x02, 0x40, 0x20, 2, 0x0e, 2, 0, 1,
        0, // br_table [0, 1] default 0
        0x0b, 0x41, 10, 0x0c, 1, 0x0b, 0x41, 20, 0x0b, 0x20, 1, 0x6a, 0x21, 1, 0x20, 2, 0x41, 1,
        0x6b, 0x22, 2, 0x0d, 0, // l2--; br_if loop
        0x0b, 0x20, 1,
    ]);

    // f6 (type 4): live Xmm0 (f64) surviving across ALL HelperKinds: Trunc, MemGrow, MemCopy, MemFill!
    let mut b6 = Vec::new();
    // Keep x (f64) live across trunc, memory.fill, memory.copy, memory.grow, then add them all!
    b6.extend_from_slice(&[
        0x20, 0, 0xaa, // i32.trunc_f64_s(x) (HelperKind::Trunc)
        0x21, 1, 0x41, 128, 1, 0x41, 0x5a, 0x41, 16, 0xfc, 0x0b,
        0x00, // memory.fill(128, 0x5a, 16)
        0x41, 144, 1, 0x41, 128, 1, 0x41, 16, 0xfc, 0x0a, 0x00,
        0x00, // memory.copy(144, 128, 16)
        0x41, 1, 0x40, 0x00, // memory.grow(1) -> old_pages
        0x20, 1, 0x6a, // old_pages + trunc(x)
        0xb7, // f64.convert_i32_s
        0x20, 0,
        0xa0, // + x (x MUST have survived in Xmm0 / spill across all 4 helper kinds!)
    ]);

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]),
            (&ten_i64, &twentyfour_i64),
            (&ten_v128, &[0x7e]),
            (&[0x7f, 0x7f], &[0x7f, 0x7f]),
            (&[0x7c], &[0x7c]),
        ]),
        func_sec(&[1, 0, 2, 0, 3, 0, 4]),
        section(5, &[1, 0x01, 1, 4]), // memory min=1 max=4
        export_sec(&[
            ("blot:memory", 2, 0),
            ("mv24", 0, 1),
            ("v128_10", 0, 3),
            ("cfg_mv", 0, 5),
            ("live_xmm_helpers", 0, 6),
        ]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[(2, 0x7f)], &b4),
            enc_body(&[(2, 0x7f)], &b5),
            enc_body(&[(1, 0x7f)], &b6),
        ]),
    ]);

    let calls = vec![
        c("mv24", ArgSpec::I32(17)),
        c("mv24", ArgSpec::I32(-42)),
        c("v128_10", ArgSpec::I32(0x1234_5678)),
        c("cfg_mv", ArgSpec::I32(8)),
        c("live_xmm_helpers", ArgSpec::F64(123.75f64.to_bits())),
    ];
    assert_oracle_matrix("mv_v128_cfg", &wasm, &calls, &[], &[(0, 64), (128, 32)]);
}

// ---- Suite 4: Memory (overlap copy, len==0 OOB & 0-cap NULL, offset > i32::MAX spilled store,
//               load-run trap ordering), externref tables, call_indirect, post-trap state parity ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_memory_tables_indirect_and_post_trap_parity() {
    // 2 host imports: idx 0 = ok_cb(i32)->(i32), idx 1 = fail_cb(i32)->(i32) (returns host error!)
    let mut imp_p = vec![2];
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "ok");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "fail");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);

    // Tables: table 0 = funcref min=6; table 1 = externref min=4 max=4
    let tbl_p = vec![2, 0x70, 0x00, 6, 0x6f, 0x01, 4, 4];
    // Memory: min=1 max=2
    let mem_p = vec![1, 0x01, 1, 2];
    // Globals: g0 = mut i32 (10), g1 = mut i64 (20)
    let mut glob_p = vec![2];
    glob_p.extend_from_slice(&[0x7f, 1, 0x41, 10, 0x0b]);
    glob_p.extend_from_slice(&[0x7e, 1, 0x42, 20, 0x0b]);
    // Active elem on table 0: slot 0 = import 0, slot 1 = f2 (type 0), slot 2 = f3 (type 1, mismatch!), slot 3 = null (hole!)
    let elem_p = vec![
        2, 0x00, 0x41, 0, 0x0b, 3, 0, 2, 3, 0x00, 0x41, 4, 0x0b, 1, 2,
    ];

    // f2 (type 0: (i32)->(i32)): x * 3 + 1
    let b2 = vec![0x20, 0, 0x41, 3, 0x6c, 0x41, 1, 0x6a];
    // f3 (type 1: (i64)->(i64)): x + 9 (wrong signature for type 0 call_indirect!)
    let b3 = vec![0x20, 0, 0x42, 9, 0x7c];

    // f4 (type 0): call_indirect dispatch test (idx = arg, uses equivalent type 2 == (i32)->(i32))
    let b4 = vec![0x41, 7, 0x20, 0, 0x11, 2, 0];

    // f5 (type 0): memory.copy / memory.fill overlapping forward/backward + len==0 OOB!
    let mut b5 = Vec::new();
    // arg == 0: fill 0..16 with 0x11, write 0..8 bytes 1..8, overlapping backward copy (dst=4, src=0, len=8), then forward copy (dst=2, src=4, len=8)
    // arg == 1: memory.copy(dst=65537, src=0, len=0) -> MUST trap OOB (2)!
    // arg == 2: memory.fill(dst=65537, val=0, len=0) -> MUST trap OOB (2)!
    // arg == 3: memory.copy(dst=65536, src=65536, len=0) -> in-bounds len==0 -> OK (returns 99)!
    b5.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x7f]);
    b5.extend_from_slice(&[0x41, 0, 0x41, 0x11, 0x41, 16, 0xfc, 0x0b, 0x00]);
    for i in 0..8u8 {
        b5.extend_from_slice(&[0x41, i, 0x41, i + 1, 0x3a, 0, 0]);
    }
    b5.extend_from_slice(&[0x41, 4, 0x41, 0, 0x41, 8, 0xfc, 0x0a, 0x00, 0x00]); // backward overlap
    b5.extend_from_slice(&[0x41, 2, 0x41, 4, 0x41, 8, 0xfc, 0x0a, 0x00, 0x00]); // forward overlap
    b5.extend_from_slice(&[0x41, 42]);
    b5.extend_from_slice(&[0x05, 0x20, 0, 0x41, 1, 0x46, 0x04, 0x7f]);
    b5.push(0x41);
    leb_i32(&mut b5, 65537);
    b5.extend_from_slice(&[0x41, 0, 0x41, 0, 0xfc, 0x0a, 0x00, 0x00, 0x41, 0]);
    b5.extend_from_slice(&[0x05, 0x20, 0, 0x41, 2, 0x46, 0x04, 0x7f]);
    b5.push(0x41);
    leb_i32(&mut b5, 65537);
    b5.extend_from_slice(&[0x41, 0, 0x41, 0, 0xfc, 0x0b, 0x00, 0x41, 0]);
    b5.extend_from_slice(&[0x05]);
    b5.push(0x41);
    leb_i32(&mut b5, 65536);
    b5.push(0x41);
    leb_i32(&mut b5, 65536);
    b5.extend_from_slice(&[0x41, 0, 0xfc, 0x0a, 0x00, 0x00, 0x41, 99]);
    b5.extend_from_slice(&[0x0b, 0x0b, 0x0b]);

    // f6 (type 0): externref table.get / table.set / OOB trap 3
    let mut b6 = Vec::new();
    // table.set 1 (arg, ref.null extern); table.get 1 (arg); ref.is_null
    b6.extend_from_slice(&[0x20, 0, 0xd0, 0x6f, 0x26, 1, 0x20, 0, 0x25, 1, 0xd1]);

    // f7 (type 0): post-trap observable state parity!
    // Writes mem[64..68] = 0x11223344, increments g0 by 5, sets g1 = 999, calls import 0 (ok),
    // then based on arg: 0 -> OOB load (trap 2), 1 -> import 1 (trap 19), 2 -> unreachable (trap 1), 3 -> offset > i32::MAX store (trap 2)
    let mut b7 = Vec::new();
    b7.extend_from_slice(&[0x41, 64]);
    b7.push(0x41);
    leb_i32(&mut b7, 0x1122_3344);
    b7.extend_from_slice(&[0x36, 2, 0]);
    b7.extend_from_slice(&[0x23, 0, 0x41, 5, 0x6a, 0x24, 0]);
    b7.push(0x42);
    leb_i64(&mut b7, 999);
    b7.extend_from_slice(&[0x24, 1]);
    b7.extend_from_slice(&[0x20, 0, 0x10, 0, 0x1a]); // call ok_cb(arg); drop
    b7.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x7f]);
    // load-run trap order: load valid [64] then OOB [65536]
    b7.extend_from_slice(&[0x41, 64, 0x28, 2, 0]);
    b7.push(0x41);
    leb_i32(&mut b7, 65536);
    b7.extend_from_slice(&[0x28, 2, 0, 0x6a]);
    b7.extend_from_slice(&[0x05, 0x20, 0, 0x41, 1, 0x46, 0x04, 0x7f]);
    b7.extend_from_slice(&[0x41, 77, 0x10, 1]); // call fail_cb -> TRAP_HOST
    b7.extend_from_slice(&[0x05, 0x20, 0, 0x41, 2, 0x46, 0x04, 0x7f]);
    b7.extend_from_slice(&[0x00]); // unreachable -> TRAP_UNREACHABLE
    b7.extend_from_slice(&[0x05]);
    // offset > i32::MAX (0x8000_0004) store with spilled val -> MUST trap OOB (2)!
    b7.extend_from_slice(&[0x41, 0, 0x41, 9, 0x36, 2]);
    leb_u32(&mut b7, 0x8000_0004);
    b7.extend_from_slice(&[0x41, 0, 0x0b, 0x0b, 0x0b]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e]), (&[0x7f], &[0x7f])]),
        section(2, &imp_p),
        func_sec(&[0, 1, 0, 0, 0, 0]), // f2..f7
        section(4, &tbl_p),
        section(5, &mem_p),
        section(6, &glob_p),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("g0", 3, 0),
            ("g1", 3, 1),
            ("ci", 0, 4),
            ("mops", 0, 5),
            ("etbl", 0, 6),
            ("post_trap", 0, 7),
        ]),
        section(9, &elem_p),
        code_sec(&[
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[], &b4),
            enc_body(&[], &b5),
            enc_body(&[], &b6),
            enc_body(&[], &b7),
        ]),
    ]);

    let calls = vec![
        c("ci", ArgSpec::I32(0)),  // imported func via call_indirect!
        c("ci", ArgSpec::I32(1)),  // defined func hit
        c("ci", ArgSpec::I32(2)),  // type mismatch -> trap 4
        c("ci", ArgSpec::I32(3)),  // null hole -> trap 4
        c("ci", ArgSpec::I32(4)),  // defined func hit via 2nd elem seg
        c("ci", ArgSpec::I32(6)),  // table OOB -> trap 3
        c("ci", ArgSpec::I32(-1)), // unsigned table OOB -> trap 3
        c("mops", ArgSpec::I32(0)),
        c("mops", ArgSpec::I32(1)), // copy len==0 OOB -> trap 2
        c("mops", ArgSpec::I32(2)), // fill len==0 OOB -> trap 2
        c("mops", ArgSpec::I32(3)), // copy len==0 at mem_len -> OK!
        c("etbl", ArgSpec::I32(0)),
        c("etbl", ArgSpec::I32(3)),
        c("etbl", ArgSpec::I32(4)),      // extern table OOB -> trap 3
        c("post_trap", ArgSpec::I32(0)), // OOB after mutations + callback
        c("post_trap", ArgSpec::I32(1)), // host fail after mutations + ok callback
        c("post_trap", ArgSpec::I32(2)), // unreachable after mutations + ok callback
        c("post_trap", ArgSpec::I32(3)), // offset > i32::MAX OOB
    ];
    assert_oracle_matrix(
        "mem_tbl_ci_trap",
        &wasm,
        &calls,
        &["g0", "g1"],
        &[(0, 16), (64, 8)],
    );
}

// ---- Suite 5: 0-capacity memory (`min=0, max=0`, `R14 == NULL`) len==0 safety ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_zero_capacity_memory_null_base_safety() {
    // Active 0-length data segment at offset 0 on 0-capacity memory (`R14 == NULL`) MUST succeed,
    // and memory.copy/fill with len==0 at 0 MUST succeed while offset=1, len=0 MUST trap OOB (2)!
    let mem_p = vec![1, 0x01, 0, 0];
    let data_p = vec![1, 0x00, 0x41, 0x00, 0x0b, 0]; // active 0-len data at off=0
    let mut b0 = Vec::new();
    // arg == 0: copy(0,0,0) + fill(0,9,0) + memory.size -> 0
    // arg == 1: copy(1,0,0) -> trap 2
    // arg == 2: fill(1,0,0) -> trap 2
    // arg == 3: memory.grow(1) -> -1 (0xffff_ffff zero-ext)
    b0.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x7f]);
    b0.extend_from_slice(&[0x41, 0, 0x41, 0, 0x41, 0, 0xfc, 0x0a, 0, 0]);
    b0.extend_from_slice(&[0x41, 0, 0x41, 9, 0x41, 0, 0xfc, 0x0b, 0]);
    b0.extend_from_slice(&[0x3f, 0]);
    b0.extend_from_slice(&[0x05, 0x20, 0, 0x41, 1, 0x46, 0x04, 0x7f]);
    b0.extend_from_slice(&[0x41, 1, 0x41, 0, 0x41, 0, 0xfc, 0x0a, 0, 0, 0x41, 0]);
    b0.extend_from_slice(&[0x05, 0x20, 0, 0x41, 2, 0x46, 0x04, 0x7f]);
    b0.extend_from_slice(&[0x41, 1, 0x41, 0, 0x41, 0, 0xfc, 0x0b, 0, 0x41, 0]);
    b0.extend_from_slice(&[0x05, 0x41, 1, 0x40, 0, 0x0b, 0x0b, 0x0b]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        section(5, &mem_p),
        export_sec(&[("blot:memory", 2, 0), ("zcap", 0, 0)]),
        code_sec(&[enc_body(&[], &b0)]),
        section(11, &data_p),
    ]);
    let calls = vec![
        c("zcap", ArgSpec::I32(0)),
        c("zcap", ArgSpec::I32(1)),
        c("zcap", ArgSpec::I32(2)),
        c("zcap", ArgSpec::I32(3)),
    ];
    assert_oracle_matrix("zero_cap_mem", &wasm, &calls, &[], &[]);
}

// ---- Suite 6: Stage 2 optimizations differential oracle (magic div/rem, inline <=64B copy/fill,
//               CmpSelect GP+FP+NaN, 10 live f64 across direct + host calls, inlining, wrap safety) ----

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage2_magic_div_inline_copy_fill_cmpselect_callee_xmm_and_wrap() {
    // Host import 0: env::bump(i32) -> (i32) (returns arg + 10 via oracle harness)
    let mut imp_p = vec![1];
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "bump");
    imp_p.push(0);
    leb_u32(&mut imp_p, 0);

    // f1 (type 0: (i32)->(i32)): internal pure small leaf helper (inlined + elided!)
    let b1 = vec![0x20, 0, 0x41, 7, 0x6c, 0x41, 13, 0x6a];

    // f2 (type 0: (i32)->(i32)): magic const div_s/div_u/rem_s/rem_u across non-pow2 & pow2 & negative divisors
    let mut b2 = vec![0x41, 0]; // acc
    for (op, d) in [
        (0x6du8, 3i32),
        (0x6d, -3),
        (0x6d, 5),
        (0x6d, -7),
        (0x6d, 10),
        (0x6d, -100),
        (0x6d, 0x4000_0001),
        (0x6e, 3),
        (0x6e, 5),
        (0x6e, 7),
        (0x6e, 10),
        (0x6e, 100),
        (0x6e, 0x4000_0001),
        (0x6f, 3),
        (0x6f, -3),
        (0x6f, 7),
        (0x6f, -10),
        (0x6f, -1),
        (0x6f, 0x4000_0001),
        (0x70, 3),
        (0x70, 7),
        (0x70, 10),
        (0x70, 100),
        (0x70, 0x4000_0001),
    ] {
        b2.push(0x41);
        leb_i32(&mut b2, 31);
        b2.push(0x6c); // acc *= 31
        b2.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(&mut b2, d);
        b2.push(op);
        b2.push(0x73); // acc ^= res
    }
    // Also chain inlined f1!
    b2.extend_from_slice(&[0x20, 0, 0x10, 1, 0x73]);

    // f3 (type 1: (i64)->(i64)): 64-bit magic const div/rem across positive/negative/large divisors
    let mut b3 = vec![0x42, 0];
    for (op, d) in [
        (0x7fu8, 3i64),
        (0x7f, -3),
        (0x7f, 7),
        (0x7f, -10),
        (0x7f, 100),
        (0x80, 3),
        (0x80, 7),
        (0x80, 10),
        (0x80, 1000),
        (0x81, 3),
        (0x81, -7),
        (0x81, -1),
        (0x81, 100),
        (0x82, 3),
        (0x82, 7),
        (0x82, 10),
        (0x82, 1000),
    ] {
        b3.push(0x42);
        leb_i64(&mut b3, 31);
        b3.push(0x7e);
        b3.extend_from_slice(&[0x20, 0, 0x42]);
        leb_i64(&mut b3, d);
        b3.push(op);
        b3.push(0x85);
    }

    // f4 (type 0: (i32)->(i32)): inline <=64B memory.copy (all sizes 0..=64, forward & backward overlaps!) + memory.fill!
    let mut b4 = Vec::new();
    // Fill 0..256 with deterministic bytes `i*3+1`, run overlapping copy & fill per `arg` length class, hash 0..256!
    b4.extend_from_slice(&[0x41, 0, 0x21, 1]); // i = 0
    b4.extend_from_slice(&[0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x41]);
    leb_i32(&mut b4, 256);
    b4.extend_from_slice(&[
        0x4f, 0x0d, 1, 0x20, 1, 0x20, 1, 0x41, 3, 0x6c, 0x20, 0, 0x6a, 0x3a, 0, 0, 0x20, 1, 0x41,
        1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b,
    ]);
    // Run every tricky inline length (both backward overlap dst=src+shift and forward overlap dst=src-shift) + fill!
    for &(len, bwd_shift, fwd_shift) in &[
        (0i32, 0i32, 0i32),
        (1, 1, 1),
        (2, 1, 1),
        (3, 1, 2),
        (4, 2, 1),
        (5, 3, 2),
        (7, 3, 4),
        (8, 4, 3),
        (9, 5, 4),
        (15, 7, 6),
        (16, 8, 7),
        (17, 9, 8),
        (24, 7, 9),
        (31, 15, 11),
        (32, 16, 15),
        (33, 5, 7),
        (40, 13, 11),
        (48, 17, 15),
        (49, 19, 13),
        (57, 23, 17),
        (64, 15, 19),
    ] {
        // Backward overlap: dst = 64 + bwd_shift, src = 64, len
        b4.push(0x41);
        leb_i32(&mut b4, 64 + bwd_shift);
        b4.push(0x41);
        leb_i32(&mut b4, 64);
        b4.push(0x41);
        leb_i32(&mut b4, len);
        b4.extend_from_slice(&[0xfc, 0x0a, 0, 0]);
        // Forward overlap: dst = 64 - fwd_shift, src = 64, len
        b4.push(0x41);
        leb_i32(&mut b4, 64 - fwd_shift);
        b4.push(0x41);
        leb_i32(&mut b4, 64);
        b4.push(0x41);
        leb_i32(&mut b4, len);
        b4.extend_from_slice(&[0xfc, 0x0a, 0, 0]);
        // Inline fill at 160 with value `(len ^ arg)`
        b4.push(0x41);
        leb_i32(&mut b4, 160);
        b4.push(0x20);
        b4.push(0);
        b4.push(0x41);
        leb_i32(&mut b4, len);
        b4.push(0x73);
        b4.push(0x41);
        leb_i32(&mut b4, len);
        b4.extend_from_slice(&[0xfc, 0x0b, 0]);
    }
    // Hash mem[0..256] into i32 result!
    b4.extend_from_slice(&[0x41, 0, 0x21, 1, 0x41, 0, 0x21, 2]);
    b4.extend_from_slice(&[0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x41]);
    leb_i32(&mut b4, 256);
    b4.extend_from_slice(&[
        0x4f, 0x0d, 1, 0x20, 2, 0x41, 31, 0x6c, 0x20, 1, 0x2d, 0, 0, 0x6a, 0x21, 2, 0x20, 1, 0x41,
        1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f5 (type 2: (f64)->(i32)): CmpSelect for ALL 6 FP comparisons (including unordered NaN!) with BOTH GP & FP select!
    let mut b5 = vec![0x41, 0]; // acc = 0
    for (bit, op) in [0x61u8, 0x62, 0x63, 0x64, 0x65, 0x66]
        .into_iter()
        .enumerate()
    {
        // GP CmpSelect: select(10 + bit, 20 + bit, f64.cmp(x, 1.5))
        b5.push(0x41);
        leb_i32(&mut b5, 10 + bit as i32);
        b5.push(0x41);
        leb_i32(&mut b5, 20 + bit as i32);
        b5.push(0x20);
        b5.push(0);
        b5.push(0x44);
        b5.extend_from_slice(&1.5f64.to_bits().to_le_bytes());
        b5.push(op);
        b5.push(0x1b); // select
        b5.extend_from_slice(&[0x41, (bit * 3) as u8, 0x74, 0x73]); // (sel << bit*3) ^ acc
        // FP CmpSelect: select(3.0, -5.0, f64.cmp(x, 1.5)) -> trunc_sat_s -> xor
        b5.push(0x44);
        b5.extend_from_slice(&3.0f64.to_bits().to_le_bytes());
        b5.push(0x44);
        b5.extend_from_slice(&(-5.0f64).to_bits().to_le_bytes());
        b5.push(0x20);
        b5.push(0);
        b5.push(0x44);
        b5.extend_from_slice(&1.5f64.to_bits().to_le_bytes());
        b5.push(op);
        b5.push(0x1b); // select f64
        b5.extend_from_slice(&[0xfc, 0x02, 0x73]); // i32.trunc_sat_f64_s ^ acc
    }

    // f6 (type 3: (f64)->(f64)): 10 live f64 values surviving BOTH a direct Wasm call (f7) AND host import 0!
    // f7 (type 3: (f64)->(f64)): non-inlined FP callee clobbering Xmm0..Xmm13 (computes 14 live f64s!)
    let mut b7 = Vec::new();
    for i in 1..=14u32 {
        b7.push(0x20);
        b7.push(0);
        b7.push(0x44);
        b7.extend_from_slice(&(i as f64 * 1.125).to_bits().to_le_bytes());
        b7.push(0xa0); // f64.add
        b7.push(0x21);
        leb_u32(&mut b7, i);
    }
    b7.push(0x20);
    b7.push(1);
    for i in 2..=14u32 {
        b7.push(0x20);
        leb_u32(&mut b7, i);
        b7.push(0xa0);
    }
    // Pad to >50 insts so f7 is NOT inlined (actually exercises callee-saved Xmm8..Xmm13 across a real call!).
    for _ in 0..10 {
        b7.push(0x44);
        b7.extend_from_slice(&0.5f64.to_bits().to_le_bytes());
        b7.push(0xa0);
    }

    let mut b6 = Vec::new();
    for i in 1..=10u32 {
        b6.push(0x20);
        b6.push(0);
        b6.push(0x44);
        b6.extend_from_slice(&(i as f64 * 0.25 + 1.0).to_bits().to_le_bytes());
        b6.push(0xa2); // f64.mul
        b6.push(0x21);
        leb_u32(&mut b6, i);
    }
    // Call direct f7 (clobbering caller+callee XMM regs!) AND host import 0 (HELPER_HOST_CALL sub rsp, 120!)
    b6.extend_from_slice(&[0x20, 0, 0x10, 7, 0x21, 11]); // r_dir = f7(x)
    b6.extend_from_slice(&[0x41, 5, 0x10, 0, 0xb7, 0x20, 11, 0xa0, 0x21, 11]); // r_dir += f64(bump(5))
    // Now sum all 10 live f64 locals + r_dir!
    b6.extend_from_slice(&[0x20, 11]);
    for i in 1..=10u32 {
        b6.push(0x20);
        leb_u32(&mut b6, i);
        b6.push(0xa0);
    }

    // f8 (type 0: (i32)->(i32)): wrap-around addressing check in unbounded 4 GiB memory!
    // Stores 0x55667788 at offset 8, then loads (arg + 16) with memarg offset 0; for arg = -8 (0xffff_fff8),
    // u32 wrap gives 8 -> MUST return 0x55667788, NOT trap OOB!
    let mut b8 = vec![0x41, 8, 0x41];
    leb_i32(&mut b8, 0x5566_7788);
    b8.extend_from_slice(&[0x36, 2, 0]);
    // Dominating load at offset 0 (not using arg, or conditional) + load(arg + 16):
    b8.extend_from_slice(&[0x20, 0, 0x41, 16, 0x6a, 0x28, 2, 0]);

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]), // 0
            (&[0x7e], &[0x7e]), // 1
            (&[0x7c], &[0x7f]), // 2
            (&[0x7c], &[0x7c]), // 3
        ]),
        section(2, &imp_p),
        func_sec(&[0, 0, 1, 0, 2, 3, 3, 0]), // f1..f8
        section(5, &[1, 0x00, 1]),           // unbounded 4 GiB max memory!
        export_sec(&[
            ("blot:memory", 2, 0),
            ("magic32", 0, 2),
            ("magic64", 0, 3),
            ("copyfill", 0, 4),
            ("cmpselect_fp", 0, 5),
            ("live_xmm10", 0, 6),
            ("wrap_load", 0, 8),
        ]),
        code_sec(&[
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[(2, 0x7f)], &b4),
            enc_body(&[], &b5),
            enc_body(&[(11, 0x7c)], &b6),
            enc_body(&[(14, 0x7c)], &b7),
            enc_body(&[], &b8),
        ]),
    ]);

    let calls = vec![
        c("magic32", ArgSpec::I32(0)),
        c("magic32", ArgSpec::I32(1)),
        c("magic32", ArgSpec::I32(-1)),
        c("magic32", ArgSpec::I32(1234567)),
        c("magic32", ArgSpec::I32(-1234567)),
        c("magic32", ArgSpec::I32(i32::MIN)),
        c("magic32", ArgSpec::I32(i32::MAX)),
        c("magic64", ArgSpec::I64(0)),
        c("magic64", ArgSpec::I64(-1)),
        c("magic64", ArgSpec::I64(i64::MIN)),
        c("magic64", ArgSpec::I64(i64::MAX)),
        c("magic64", ArgSpec::I64(-9_876_543_210)),
        c("copyfill", ArgSpec::I32(7)),
        c("copyfill", ArgSpec::I32(131)),
        c("cmpselect_fp", ArgSpec::F64(0.0f64.to_bits())),
        c("cmpselect_fp", ArgSpec::F64((-0.0f64).to_bits())),
        c("cmpselect_fp", ArgSpec::F64(1.5f64.to_bits())),
        c("cmpselect_fp", ArgSpec::F64(2.5f64.to_bits())),
        c("cmpselect_fp", ArgSpec::F64(f64::NAN.to_bits())),
        c("cmpselect_fp", ArgSpec::F64(0x7ff0_0000_0000_0001)), // sNaN
        c("live_xmm10", ArgSpec::F64(3.5f64.to_bits())),
        c("live_xmm10", ArgSpec::F64((-7.25f64).to_bits())),
        c("wrap_load", ArgSpec::I32(-8)), // 0xffff_fff8 + 16 wraps u32 -> 8!
        c("wrap_load", ArgSpec::I32(-16)), // wraps u32 -> 0
    ];
    assert_oracle_matrix("stage2_opt", &wasm, &calls, &[], &[(0, 256)]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage2_audit_regressions_magic64_rem_deep_blocks_and_dead_br_table() {
    // 1. f0: (i64)->(i64): `(x + 1) rem_u 10` where `x + 1` dies in Rax (clobbered by `mul`)!
    let b0 = vec![0x20, 0, 0x42, 1, 0x7c, 0x42, 10, 0x82];
    // 2. f1: (i64)->(i64): `(x + 1) rem_s -7` where `x + 1` dies in Rax (clobbered by `imul`)!
    let mut b1 = vec![0x20, 0, 0x42, 1, 0x7c, 0x42];
    leb_i64(&mut b1, -7);
    b1.push(0x81);
    // 3. f2: (i32)->(i32): 72 nested `(block (result i32) ...)` containing `(if (local.get 0) (then (nop)))`
    //    and returning `(i32.add (local.get 0) (i32.const 99))` -> >64-hop reverse merge chain!
    let mut b2 = Vec::new();
    for _ in 0..72 {
        b2.extend_from_slice(&[0x02, 0x7f]); // block (result i32)
    }
    b2.extend_from_slice(&[0x20, 0, 0x04, 0x40, 0x01, 0x0b]); // if (arg) { nop }
    b2.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b2, 99);
    b2.push(0x6a); // arg + 99
    b2.extend(std::iter::repeat_n(0x0b, 72)); // end 72 blocks
    // 4. f3: helper(flag, x): if (flag) { x + 5 } else { x * 3 }
    let b3 = vec![
        0x20, 0, 0x04, 0x7f, 0x20, 1, 0x41, 5, 0x6a, 0x05, 0x20, 1, 0x41, 3, 0x6c, 0x0b,
    ];
    // 5. f4: (i32)->(i32): dead `if (0) { block { br_table 0 1 (arg) } }` (pruned in first rebuild)
    //    followed by `call f3(0, arg)` (inlined, folded dead -> triggers second rebuild_cfg_and_prune!).
    let b4 = vec![
        0x41, 0, 0x04, 0x40, // if (0)
        0x02, 0x40, // block
        0x20, 0, 0x0e, 1, 0, 1,    // br_table [0] default=1
        0x0b, // end block
        0x0b, // end if
        0x41, 0, 0x20, 0, 0x10, 3, // call f3(0, arg)
    ];

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7e], &[0x7e]),       // 0: (i64)->(i64)
            (&[0x7f], &[0x7f]),       // 1: (i32)->(i32)
            (&[0x7f, 0x7f], &[0x7f]), // 2: (i32, i32)->(i32)
        ]),
        func_sec(&[0, 0, 1, 2, 1]),
        export_sec(&[
            ("rem_u64_rax", 0, 0),
            ("rem_s64_rax", 0, 1),
            ("deep72_blocks", 0, 2),
            ("dead_brt_double_rebuild", 0, 4),
        ]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[], &b4),
        ]),
    ]);

    let calls = vec![
        c("rem_u64_rax", ArgSpec::I64(24)),
        c("rem_u64_rax", ArgSpec::I64(0)),
        c("rem_u64_rax", ArgSpec::I64(-2)),
        c("rem_u64_rax", ArgSpec::I64(i64::MAX - 1)),
        c("rem_s64_rax", ArgSpec::I64(24)),
        c("rem_s64_rax", ArgSpec::I64(-25)),
        c("rem_s64_rax", ArgSpec::I64(i64::MIN)),
        c("rem_s64_rax", ArgSpec::I64(i64::MAX - 1)),
        c("deep72_blocks", ArgSpec::I32(0)),
        c("deep72_blocks", ArgSpec::I32(17)),
        c("dead_brt_double_rebuild", ArgSpec::I32(0)),
        c("dead_brt_double_rebuild", ArgSpec::I32(11)),
    ];
    assert_oracle_matrix("stage2_audit_regressions", &wasm, &calls, &[], &[]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage25_remat_fixed_mem_r13_self_nan_and_dynamic_memcopy_fill() {
    // Fixed 1-page memory (`min=1, max=1`) frees R13 as a 4th callee-saved GP register, and no
    // hot mutable global frees R12 as a 3rd callee-saved GP register.
    // Exercises:
    //  f0: dynamic `memory.copy` + `memory.fill` across ALL fast/medium/large buckets
    //      (`0, 1, 2, 3, 4, 7, 8, 15, 16, 24, 32, 33, 64, 100, 1024, 1025, 1200`), both forward and
    //      overlapping backward (shift 1 and shift 17), plus `memory.grow(0)`/`memory.grow(1)` on fixed memory,
    //      while keeping 10 GP locals live across every helper call (proving R12/R13 preservation!).
    //  f1: `Loc::Remat(0)` + non-zero constants in `CmpSelect` (proving RFLAGS preservation!),
    //      direct immediate `Store` + `GlobalSet` + overflow-return `Remat` values!
    //  f2: self-NaN `f32.eq(x, x)` / `f32.ne(x, x)` in `CmpSelect`, `CmpBranch`, and `F32Cmp`
    //      on +0.0, -0.0, 1.5, +inf, -inf, qNaN, sNaN.
    //  f3: 10-return multi-value helper returning remat constants (0, 0.0f32, 0.0f64, 42, -1) into overflow buffer.

    // f0(len: i32) -> i32:
    //   fills [0..2048] with distinct byte pattern `((i * 37 + 13) & 255)`,
    //   keeps 8 GP locals (l2..l9) initialized from `len` live across:
    //     1) dynamic memory.fill(4096, 0xA5, len)
    //     2) dynamic forward memory.copy(8192, 0, len)
    //     3) dynamic backward-overlap memory.copy(1 + 12288, 12288, len) after seeding [12288..12288+len]
    //     4) dynamic backward-overlap shift-17 memory.copy(17 + 16384, 16384, len)
    //     5) memory.grow(0) (must return 1) and memory.grow(1) (must return -1 on fixed 1-page memory)
    //   hashes affected ranges + all 8 preserved GP locals!
    let mut b0 = Vec::new();
    // Seed [0..2048] with non-repeating pattern using l1 = i
    b0.extend_from_slice(&[0x41, 0, 0x21, 1, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x41]);
    leb_i32(&mut b0, 2048);
    b0.extend_from_slice(&[
        0x4f, 0x0d, 1, 0x20, 1, 0x20, 1, 0x41, 37, 0x6c, 0x41, 13, 0x6a, 0x41,
    ]);
    leb_i32(&mut b0, 255);
    b0.extend_from_slice(&[
        0x71, 0x3a, 0, 0, // store8 [i]
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b,
    ]);
    // Seed l2..l9 (8 GP locals) with distinct values derived from `len` (forces R12 & R13 usage!).
    for k in 0..8u32 {
        b0.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(&mut b0, (k as i32 + 1) * 97);
        b0.extend_from_slice(&[0x73, 0x21]); // xor, local.set (2+k)
        leb_u32(&mut b0, 2 + k);
    }
    // 1. Dynamic memory.fill(4096, 0xA5, len)
    b0.push(0x41);
    leb_i32(&mut b0, 4096);
    b0.push(0x41);
    leb_i32(&mut b0, 0xA5);
    b0.extend_from_slice(&[0x20, 0, 0xfc, 0x0b, 0x00]);
    // 2. Dynamic forward memory.copy(8192, 0, len)
    b0.push(0x41);
    leb_i32(&mut b0, 8192);
    b0.extend_from_slice(&[0x41, 0, 0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    // 3. Seed [12288..12288+len] from [0..len], then backward overlap shift +1: copy(12289, 12288, len)
    b0.push(0x41);
    leb_i32(&mut b0, 12288);
    b0.extend_from_slice(&[0x41, 0, 0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    b0.push(0x41);
    leb_i32(&mut b0, 12289);
    b0.push(0x41);
    leb_i32(&mut b0, 12288);
    b0.extend_from_slice(&[0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    // 4. Seed [16384..16384+len] from [0..len], then backward overlap shift +17: copy(16401, 16384, len)
    b0.push(0x41);
    leb_i32(&mut b0, 16384);
    b0.extend_from_slice(&[0x41, 0, 0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    b0.push(0x41);
    leb_i32(&mut b0, 16401);
    b0.push(0x41);
    leb_i32(&mut b0, 16384);
    b0.extend_from_slice(&[0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    // 4b. Overlapping FORWARD shifts (-1, -17, -32) exercising the `src - dst < 32` guard and `== 32` boundary!
    for (base, shift) in [(20480i32, 1i32), (24576, 17), (28672, 32)] {
        b0.push(0x41);
        leb_i32(&mut b0, base + shift);
        b0.extend_from_slice(&[0x41, 0, 0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
        b0.push(0x41);
        leb_i32(&mut b0, base);
        b0.push(0x41);
        leb_i32(&mut b0, base + shift);
        b0.extend_from_slice(&[0x20, 0, 0xfc, 0x0a, 0x00, 0x00]);
    }
    // 5. memory.grow(0) + memory.grow(1) + memory.size on fixed 1-page memory
    b0.extend_from_slice(&[
        0x41, 0, 0x40, 0x00, 0x41, 1, 0x40, 0x00, 0x6a, 0x3f, 0x00, 0x6a,
    ]);
    // Fold all 8 live GP locals l2..l9 into accumulator in l10:
    b0.extend_from_slice(&[0x21, 10]);
    for k in 0..8u32 {
        b0.extend_from_slice(&[0x20, 10, 0x41, 31, 0x6c, 0x20]);
        leb_u32(&mut b0, 2 + k);
        b0.extend_from_slice(&[0x73, 0x21, 10]);
    }
    // Hash bytes across all fill/copy target regions (forward, backward-overlap, forward-overlap):
    for base in [4096i32, 8192, 12288, 16384, 20480, 24576, 28672] {
        b0.extend_from_slice(&[0x41, 0, 0x21, 1, 0x02, 0x40, 0x03, 0x40]);
        b0.extend_from_slice(&[0x20, 1, 0x20, 0, 0x41, 20, 0x6a, 0x4f, 0x0d, 1]);
        b0.extend_from_slice(&[0x20, 10, 0x41, 31, 0x6c, 0x20, 1, 0x41]);
        leb_i32(&mut b0, base);
        b0.extend_from_slice(&[0x6a, 0x2d, 0, 0, 0x6a, 0x21, 10]);
        b0.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b]);
    }
    b0.extend_from_slice(&[0x20, 10]);

    // f3 (type 2): () -> (10 i32s + 10 f32s) returning rematerialized zeros & constants into overflow buffer!
    let mut b3 = Vec::new();
    for k in 0..10i32 {
        b3.push(0x41);
        leb_i32(&mut b3, if k % 2 == 0 { 0 } else { k * 7 - 3 });
    }
    for k in 0..10u32 {
        b3.push(0x43);
        let bits = if k % 2 == 0 {
            0.0f32.to_bits()
        } else {
            (k as f32 + 0.5).to_bits()
        };
        b3.extend_from_slice(&bits.to_le_bytes());
    }

    // f1(x: i32) -> i32: tests Remat(0) in CmpSelect (RFLAGS preservation!), direct remat Store & GlobalSet, & f3 overflow ret!
    let mut b1 = Vec::new();
    // Direct immediate stores of 0 and non-zero remat constants to [0..32]:
    b1.extend_from_slice(&[0x41, 0, 0x41, 0, 0x36, 2, 0]); // store i32 0
    b1.extend_from_slice(&[0x41, 4, 0x42, 0, 0x37, 3, 0]); // store i64 0
    b1.extend_from_slice(&[0x41, 12, 0x43, 0, 0, 0, 0, 0x38, 2, 0]); // store f32 0.0
    b1.extend_from_slice(&[0x41, 16, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0x39, 3, 0]); // store f64 0.0
    b1.extend_from_slice(&[0x41, 0, 0x24, 0]); // global.set 0 = 0
    // CmpSelect with Remat(0) as both tv and fv (must NOT clobber EFLAGS with xor!):
    // a = select(0, 77, x > 5); b = select(99, 0, x < -5)
    b1.extend_from_slice(&[0x41, 0, 0x41]);
    leb_i32(&mut b1, 77);
    b1.extend_from_slice(&[0x20, 0, 0x41, 5, 0x4a, 0x1b]);
    b1.extend_from_slice(&[0x41]);
    leb_i32(&mut b1, 99);
    b1.extend_from_slice(&[0x41, 0, 0x20, 0, 0x41]);
    leb_i32(&mut b1, -5);
    b1.extend_from_slice(&[0x48, 0x1b, 0x6a]); // a + b
    // Call f3() -> 10 i32s + 10 f32s; fold all 20 returned values!
    b1.extend_from_slice(&[0x10, 3]);
    b1.extend(std::iter::repeat_n(0x92u8, 9)); // f32.add
    b1.extend_from_slice(&[0xfc, 0x00]); // i32.trunc_sat_f32_s
    b1.extend(std::iter::repeat_n(0x6au8, 11)); // i32.add (12 i32s -> 1 i32)
    b1.extend_from_slice(&[0x23, 0, 0x6a]); // + global 0

    // f2(x: f32) -> i32: tests self-NaN `f32.eq(x, x)` & `f32.ne(x, x)` in CmpSelect, CmpBranch, and F32Cmp!
    let mut b2 = Vec::new();
    // r1 = select(x, 42.0f32, f32.eq(x, x)) -> i32.trunc_sat_f32_s
    b2.extend_from_slice(&[0x20, 0, 0x43]);
    b2.extend_from_slice(&42.0f32.to_bits().to_le_bytes());
    b2.extend_from_slice(&[0x20, 0, 0x20, 0, 0x5b, 0x1b, 0xfc, 0x00]);
    // r2 = if f32.ne(x, x) { 1000 } else { 2000 }
    b2.extend_from_slice(&[0x20, 0, 0x20, 0, 0x5c, 0x04, 0x7f, 0x41]);
    leb_i32(&mut b2, 1000);
    b2.extend_from_slice(&[0x05, 0x41]);
    leb_i32(&mut b2, 2000);
    b2.extend_from_slice(&[0x0b, 0x6a]);
    // r3 = f32.eq(x, x) * 10 + f32.ne(x, x) * 100
    b2.extend_from_slice(&[0x20, 0, 0x20, 0, 0x5b, 0x41, 10, 0x6c, 0x6a]);
    b2.extend_from_slice(&[0x20, 0, 0x20, 0, 0x5c, 0x41]);
    leb_i32(&mut b2, 100);
    b2.extend_from_slice(&[0x6c, 0x6a]);

    let mut f3_rets = vec![0x7fu8; 10];
    f3_rets.extend([0x7du8; 10]);

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]), // 0: (i32)->(i32)
            (&[0x7d], &[0x7f]), // 1: (f32)->(i32)
            (&[], &f3_rets),    // 2: ()->(10 i32, 10 f32)
        ]),
        func_sec(&[0, 0, 1, 2]),
        section(5, &[1, 0x01, 1, 1]), // FIXED 1-page memory (frees R13!)
        // 1 mutable global with only 2 accesses -> below pin threshold -> frees R12!
        section(6, &[1, 0x7f, 1, 0x41, 7, 0x0b]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("mem_dyn", 0, 0),
            ("remat_sel", 0, 1),
            ("self_nan", 0, 2),
        ]),
        code_sec(&[
            enc_body(&[(10, 0x7f)], &b0),
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
        ]),
    ]);

    let mut calls = Vec::new();
    for len in [
        0, 1, 2, 3, 4, 7, 8, 15, 16, 24, 32, 33, 63, 64, 65, 100, 1024, 1025, 1200,
    ] {
        calls.push(c("mem_dyn", ArgSpec::I32(len)));
    }
    for x in [-10, -5, 0, 5, 10] {
        calls.push(c("remat_sel", ArgSpec::I32(x)));
    }
    for bits in [
        0.0f32.to_bits(),
        (-0.0f32).to_bits(),
        1.5f32.to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        0x7fc0_0000, // qNaN
        0x7f80_0001, // sNaN
    ] {
        calls.push(c("self_nan", ArgSpec::F32(bits)));
    }
    assert_oracle_matrix("stage25_features", &wasm, &calls, &[], &[(0, 32)]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage3_undo_if_forwarding_guarded_licm_bmi2_and_holes() {
    // f0(x: i32) -> i32: nested if/else, implicit else, loop-inside-if, unreachable-arm-in-if
    // exercising the O(writes) `local_undo` SSA builder across all local mutation opcodes.
    let mut b0 = Vec::new();
    // l1 = x * 3 + 1; l2 = x ^ 0x55; l3 = 7
    b0.extend_from_slice(&[0x20, 0, 0x41, 3, 0x6c, 0x41, 1, 0x6a, 0x21, 1]);
    b0.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b0, 0x55);
    b0.extend_from_slice(&[0x73, 0x21, 2, 0x41, 7, 0x21, 3]);
    // Outer if (x & 1): mutate l1, nested if (x & 2) { mutate l2; loop 3 times } else { mutate l3 },
    // nested if (x == -999) { l1 = 99; unreachable } (no else!)
    b0.extend_from_slice(&[0x20, 0, 0x41, 1, 0x71, 0x04, 0x40]);
    b0.extend_from_slice(&[0x20, 1, 0x41, 10, 0x6a, 0x22, 1, 0x1a]); // local.tee 1
    b0.extend_from_slice(&[0x20, 0, 0x41, 2, 0x71, 0x04, 0x40]);
    b0.extend_from_slice(&[0x20, 2, 0x41, 17, 0x6a, 0x21, 2]);
    // loop inside if: l4 = 0; loop { l3 += l4; l4 += 1; br_if(l4 < 3) 0 }
    b0.extend_from_slice(&[
        0x41, 0, 0x21, 4, 0x03, 0x40, 0x20, 3, 0x20, 4, 0x6a, 0x21, 3, 0x20, 4, 0x41, 1, 0x6a,
        0x22, 4, 0x41, 3, 0x48, 0x0d, 0, 0x0b,
    ]);
    b0.extend_from_slice(&[0x05, 0x20, 3, 0x41, 33, 0x6b, 0x21, 3, 0x0b]);
    // implicit-else if with unreachable:
    b0.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b0, -999);
    b0.extend_from_slice(&[0x46, 0x04, 0x40, 0x41, 99, 0x21, 1, 0x00, 0x0b]);
    // else of outer if: mutate only l2, leave l1 & l3 at entry values!
    b0.extend_from_slice(&[0x05, 0x20, 2, 0x41, 5, 0x6c, 0x21, 2, 0x0b]);
    // implicit-else outer if (x & 4): mutate l1 only!
    b0.extend_from_slice(&[
        0x20, 0, 0x41, 4, 0x71, 0x04, 0x40, 0x20, 1, 0x41, 13, 0x73, 0x21, 1, 0x0b,
    ]);
    // return l1 * 31 + l2 * 7 + l3
    b0.extend_from_slice(&[
        0x20, 1, 0x41, 31, 0x6c, 0x20, 2, 0x41, 7, 0x6c, 0x6a, 0x20, 3, 0x6a,
    ]);

    // f1(x: i32) -> i32: store-to-load forwarding, disjoint & overlapping stores, i8/i16 narrow forwarding,
    // and MemCopyConst with dominated src check.
    let mut b1 = Vec::new();
    // Store [0]=x, [4]=x^0x1234, reload [0] (disjoint forward!), overwrite [2]=0xab (partial overlap!), reload [0] (must NOT forward!)
    b1.extend_from_slice(&[0x41, 0, 0x20, 0, 0x36, 2, 0]);
    b1.extend_from_slice(&[0x41, 4, 0x20, 0, 0x41]);
    leb_i32(&mut b1, 0x1234);
    b1.extend_from_slice(&[0x73, 0x36, 2, 0]);
    b1.extend_from_slice(&[0x41, 0, 0x28, 2, 0, 0x21, 1]); // l1 = load[0] (forwarded)
    b1.extend_from_slice(&[0x41, 2, 0x41]);
    leb_i32(&mut b1, 0xab);
    b1.extend_from_slice(&[0x3a, 0, 0]); // store8 [2] = 0xab (overlaps [0..4]!)
    b1.extend_from_slice(&[0x41, 0, 0x28, 2, 0, 0x21, 2]); // l2 = load[0] (must see mutated byte 2!)
    // Masked vs unmasked i8 store -> load8_u + load8_s:
    b1.extend_from_slice(&[0x41, 8, 0x20, 0, 0x41]);
    leb_i32(&mut b1, 0xff);
    b1.extend_from_slice(&[0x71, 0x3a, 0, 0]); // store8 [8] = x & 0xff
    b1.extend_from_slice(&[0x41, 8, 0x2d, 0, 0]); // load8_u [8]
    b1.extend_from_slice(&[0x41, 8, 0x2c, 0, 0, 0x6a]); // + load8_s [8]
    b1.extend_from_slice(&[0x41, 12, 0x20, 0, 0x3a, 0, 0]); // store8 [12] = x (unbounded!)
    b1.extend_from_slice(&[0x41, 12, 0x2d, 0, 0, 0x6a]); // + load8_u [12] (must mask!)
    // Dominated src load + MemCopyConst(32, 0, 16) + load[32]
    b1.extend_from_slice(&[0x41, 12, 0x28, 2, 0, 0x1a]);
    b1.extend_from_slice(&[0x41, 32, 0x41, 0, 0x41, 16, 0xfc, 0x0a, 0x00, 0x00]);
    b1.extend_from_slice(&[0x41, 32, 0x28, 2, 0, 0x6a]);
    b1.extend_from_slice(&[0x20, 1, 0x6a, 0x20, 2, 0x73]);

    // f2(d: i32) -> i32: guarded LICM `DivU` & `RemU` with 0-iteration `d == 0` trap-safety!
    // for i in 0..d { sum = sum * 3 + (1000u / d) + (1000u % d) + (i * 5) + (i * 9) + (i << 1) }
    let mut b2 = Vec::new();
    b2.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b2.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // if i >=u d break
    b2.extend_from_slice(&[0x20, 2, 0x41, 3, 0x6c]); // sum * 3 (lea!)
    b2.push(0x41);
    leb_i32(&mut b2, 1000);
    b2.extend_from_slice(&[0x20, 0, 0x6e, 0x6a]); // + 1000u / d
    b2.push(0x41);
    leb_i32(&mut b2, 1000);
    b2.extend_from_slice(&[0x20, 0, 0x70, 0x6a]); // + 1000u % d
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x6c, 0x6a]); // + i * 5 (lea!)
    b2.extend_from_slice(&[0x20, 1, 0x41, 9, 0x6c, 0x6a]); // + i * 9 (lea!)
    b2.extend_from_slice(&[0x20, 1, 0x41, 1, 0x74, 0x6a, 0x21, 2]); // + (i << 1) (lea!)
    b2.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f3(s: i32) -> i32: BMI2 variable shifts (i32 + i64) + constant rotates + live values across shifts
    // + multi-accumulator loop exercising live-range holes!
    let mut b3 = Vec::new();
    // a0..a3 live across all variable shifts so RCX gets allocated on BMI2!
    b3.extend_from_slice(&[0x20, 0, 0x41, 11, 0x6a, 0x21, 1]); // l1
    b3.extend_from_slice(&[0x20, 0, 0x41, 23, 0x6c, 0x21, 2]); // l2
    b3.extend_from_slice(&[0x20, 0, 0x41, 37, 0x73, 0x21, 3]); // l3
    b3.extend_from_slice(&[0x20, 0, 0x41, 53, 0x6b, 0x21, 4]); // l4
    // i32 shl/shr_s/shr_u with dynamic shift count s, plus const rotl 7 / rotr 13:
    b3.extend_from_slice(&[
        0x20, 1, 0x20, 0, 0x74, // l1 << s
        0x20, 2, 0x20, 0, 0x75, 0x73, // ^ (l2 >>s s)
        0x20, 3, 0x20, 0, 0x76, 0x6a, // + (l3 >>u s)
        0x20, 4, 0x41, 7, 0x77, 0x73, // ^ rotl(l4, 7)
        0x20, 1, 0x41, 13, 0x78, 0x6a, // + rotr(l1, 13)
        0x21, 5,
    ]);
    // i64 shl/shr_s/shr_u + const rotl 19 / rotr 41:
    b3.extend_from_slice(&[
        0x20, 2, 0xac, 0x42, 32, 0x86, 0x20, 3, 0xad, 0x84, 0x21, 6, // l6: i64
        0x20, 0, 0xac, 0x21, 7, // l7: i64 shift count
        0x20, 6, 0x20, 7, 0x86, // shl
        0x20, 6, 0x20, 7, 0x87, 0x85, // ^ shr_s
        0x20, 6, 0x20, 7, 0x88, 0x7c, // + shr_u
        0x20, 6, 0x42, 19, 0x89, 0x85, // ^ rotl 19
        0x20, 6, 0x42, 41, 0x8a, 0x7c, // + rotr 41
        0xa7, // i32.wrap_i64
        0x20, 5, 0x73, 0x20, 1, 0x6a, 0x20, 2, 0x6a, 0x20, 3, 0x6a, 0x20, 4, 0x6a,
    ]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0, 0, 0]),
        section(5, &[1, 0x01, 1, 2]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("undo_if", 0, 0),
            ("mem_fwd", 0, 1),
            ("guarded_div", 0, 2),
            ("bmi2_shifts", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[(4, 0x7f)], &b0),
            enc_body(&[(2, 0x7f)], &b1),
            enc_body(&[(2, 0x7f)], &b2),
            enc_body(&[(5, 0x7f), (2, 0x7e)], &b3),
        ]),
    ]);

    let mut calls = Vec::new();
    for x in [0, 1, 2, 3, 4, 5, 6, 7, -1, -999, i32::MIN, i32::MAX] {
        calls.push(c("undo_if", ArgSpec::I32(x)));
    }
    for x in [0, 1, -1, 0x1234_5678, -0x7654_3210, 0x80, 0xff, 0x180] {
        calls.push(c("mem_fwd", ArgSpec::I32(x)));
    }
    for d in [0, 1, 2, 7, 13, 100] {
        calls.push(c("guarded_div", ArgSpec::I32(d)));
    }
    for s in [0, 1, 7, 31, 32, 33, 63, 64, -1, i32::MIN, i32::MAX] {
        calls.push(c("bmi2_shifts", ArgSpec::I32(s)));
    }
    assert_oracle_matrix("stage3_features", &wasm, &calls, &[], &[(0, 48)]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage3_council_round1_regressions() {
    // f0(x: i32) -> i32: conditional infinite self-loop (`if x == 0 { loop { br 0 } } else { x + 41 }`)
    let b0 = vec![
        0x20, 0, 0x45, 0x04, 0x40, 0x03, 0x40, 0x0c, 0, 0x0b, 0x0b, 0x20, 0, 0x41, 41, 0x6a,
    ];

    // f1(v64: i64) -> i32: i64.store32 with non-zero high 32 bits followed by i32.load
    // used in BOTH arithmetic AND as a linear memory address (catches dirty upper 32 bits!).
    let b1 = vec![
        // seed memory[16] = i32.wrap(v64 >> 32) ^ i32.wrap(v64)
        0x41, 16, 0x20, 0, 0x42, 32, 0x88, 0xa7, 0x20, 0, 0xa7, 0x73, 0x36, 2, 0,
        // i64.store32 [0], v64
        0x41, 0, 0x20, 0, 0x3e, 2, 0,
        // reload as i32 -> local 1 (must NOT forward Ty::I64 `v64`!)
        0x41, 0, 0x28, 2, 0, 0x22, 1, // tee local 1
        // use local 1 as address `(local1 & 12) + 4` to load + combine with local 1
        0x41, 12, 0x71, 0x28, 2, 4, // i32.load [(local1 & 12) + 4]
        0x20, 1, 0x6a,
    ];

    // f2(d: i32) -> i32: guarded-preheader LICM where `exit_bb` directly uses `hdr.params` (`i` and `x`)
    let mut b2 = vec![0x20, 0, 0x41, 7, 0x6c, 0x41, 3, 0x6a, 0x21, 2];
    b2.push(0x41);
    leb_i32(&mut b2, 0x1234);
    b2.extend_from_slice(&[0x21, 1, 0x02, 0x40, 0x03, 0x40]);
    b2.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]);
    b2.extend_from_slice(&[0x20, 2, 0x41]);
    leb_i32(&mut b2, 1000);
    b2.extend_from_slice(&[0x20, 0, 0x6e, 0x6a, 0x21, 2]);
    b2.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b]);
    b2.extend_from_slice(&[0x20, 1, 0x41]);
    leb_i32(&mut b2, 1000);
    b2.extend_from_slice(&[0x6c, 0x20, 2, 0x6a]);

    // f3(packed: i32) -> i32: unpack d1 = packed & 0xff, d2 = (packed >> 8) & 0xff;
    // nested guarded-preheader LICM loops (`(10000u / d1) / d2`)
    let mut b3 = Vec::new();
    // local 1 = d1, local 2 = d2, local 3 = i2, local 4 = i1, local 5 = acc
    b3.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b3, 0xff);
    b3.extend_from_slice(&[0x71, 0x21, 1]);
    b3.extend_from_slice(&[0x20, 0, 0x41, 8, 0x76, 0x41]);
    leb_i32(&mut b3, 0xff);
    b3.extend_from_slice(&[0x71, 0x21, 2]);
    b3.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b3.extend_from_slice(&[0x20, 3, 0x20, 2, 0x4f, 0x0d, 1]); // if i2 >=u d2 break outer
    b3.extend_from_slice(&[0x41, 0, 0x21, 4]); // i1 = 0
    b3.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b3.extend_from_slice(&[0x20, 4, 0x20, 1, 0x4f, 0x0d, 1]); // if i1 >=u d1 break inner
    b3.extend_from_slice(&[0x20, 5, 0x41]);
    leb_i32(&mut b3, 10000);
    b3.extend_from_slice(&[0x20, 1, 0x6e, 0x20, 2, 0x6e, 0x6a, 0x21, 5]);
    b3.extend_from_slice(&[0x20, 4, 0x41, 1, 0x6a, 0x21, 4, 0x0c, 0, 0x0b, 0x0b]);
    b3.extend_from_slice(&[0x20, 3, 0x41, 1, 0x6a, 0x21, 3, 0x0c, 0, 0x0b, 0x0b]);
    b3.extend_from_slice(&[0x20, 5]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7f])]),
        func_sec(&[0, 1, 0, 0]),
        section(5, &[1, 0x01, 1, 2]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("spin_if", 0, 0),
            ("store32_load32", 0, 1),
            ("licm_exit_hdr", 0, 2),
            ("licm_nested", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[(1, 0x7f)], &b1),
            enc_body(&[(2, 0x7f)], &b2),
            enc_body(&[(5, 0x7f)], &b3),
        ]),
    ]);

    let mut calls = Vec::new();
    for x in [1, -1, 42, i32::MAX] {
        calls.push(c("spin_if", ArgSpec::I32(x)));
    }
    for v64 in [
        0i64,
        0x1234_5678_0000_000c,
        -1i64,
        (i32::MIN as i64) << 32 | 16,
        0x7fff_ffff_8000_000c,
    ] {
        calls.push(c("store32_load32", ArgSpec::I64(v64)));
    }
    for d in [0, 1, 0x1233, 0x1234, 0x1235, 0x1240] {
        calls.push(c("licm_exit_hdr", ArgSpec::I32(d)));
    }
    for (d1, d2) in [(0, 0), (0, 3), (3, 0), (1, 1), (3, 5), (7, 4)] {
        calls.push(c("licm_nested", ArgSpec::I32(d1 | (d2 << 8))));
    }
    assert_oracle_matrix("stage3_r1_regressions", &wasm, &calls, &[], &[(0, 32)]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage4_hills_minmax_short_circuit_and_loop_bce() {
    // f0(bits_a: i64, bits_b: i64) -> i64: tests bit-exact FP `CmpSelect` lowering across:
    //  1) strict `f64.lt` / `f64.gt` (SSE2 `minsd`/`maxsd`, including ±0.0 & distinct NaN payloads)
    //  2) non-strict `f64.ge` / `f64.le` against non-zero constants `-100.0` / `100.0` (SSE2 clamp)
    //  3) non-strict `f64.ge` / `f64.le` against `+0.0` / `-0.0` (must NOT lose sign of zero!)
    //  4) `f32` strict & non-strict equivalents (reinterpreted as bits and mixed into return value).
    // Packed as 1-arg wrapper via memory[0..16] so `assert_oracle_matrix` can drive arbitrary pairs.
    let mut b_seed = Vec::new(); // f0(idx: i32) -> i64
    // Read (a: f64 @ [0], b: f64 @ [8]) set by caller via `store_pair`, OR derive directly from `idx`!
    // Even cleaner: derive 10 edge-case f64/f32 bit patterns from a selector `idx = 0..100` (a_idx = idx / 10, b_idx = idx % 10)
    // stored into memory[0..80] in `init_fp`.
    b_seed.extend_from_slice(&[
        // local 1 = a (f64), local 2 = b (f64), local 3 = acc (i64), local 4 = fa (f32), local 5 = fb (f32)
        // a = f64.load[(idx / 10) * 8]
        0x20, 0, 0x41, 10, 0x6e, 0x41, 3, 0x74, 0x2b, 3, 0, 0x21, 1,
        // b = f64.load[(idx % 10) * 8]
        0x20, 0, 0x41, 10, 0x70, 0x41, 3, 0x74, 0x2b, 3, 0, 0x21, 2,
        // fa = f32.load[128 + (idx / 10) * 4]
        0x20, 0, 0x41, 10, 0x6e, 0x41, 2, 0x74, 0x2a, 2, 0x80, 0x01, 0x21, 4,
        // fb = f32.load[128 + (idx % 10) * 4]
        0x20, 0, 0x41, 10, 0x70, 0x41, 2, 0x74, 0x2a, 2, 0x80, 0x01, 0x21, 5,
    ]);
    let push_mix_f64 = |v: &mut Vec<u8>| {
        // acc = rotl(acc, 7) ^ i64.reinterpret_f64(top)
        v.extend_from_slice(&[0xbd, 0x20, 3, 0x42, 7, 0x89, 0x85, 0x21, 3]);
    };
    let push_mix_f32 = |v: &mut Vec<u8>| {
        // acc = rotl(acc, 5) ^ i64.extend_i32_u(i32.reinterpret_f32(top))
        v.extend_from_slice(&[0xbc, 0xad, 0x20, 3, 0x42, 5, 0x89, 0x85, 0x21, 3]);
    };
    // 1. select(a, b, a < b), select(a, b, b > a), select(a, b, a > b), select(a, b, b < a)
    for (op, swap_cmp) in [(0x63u8, false), (0x64, true), (0x64, false), (0x63, true)] {
        b_seed.extend_from_slice(&[0x20, 1, 0x20, 2]);
        if swap_cmp {
            b_seed.extend_from_slice(&[0x20, 2, 0x20, 1, op, 0x1b]);
        } else {
            b_seed.extend_from_slice(&[0x20, 1, 0x20, 2, op, 0x1b]);
        }
        push_mix_f64(&mut b_seed);
    }
    // 2. select(a, -100.0, a >= -100.0), select(a, 100.0, a <= 100.0), and swapped operand order
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(-100.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(-100.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x66, 0x1b]); // f64.ge
    push_mix_f64(&mut b_seed);
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(100.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(100.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x65, 0x1b]); // f64.le
    push_mix_f64(&mut b_seed);
    // 3. Non-strict `a >= +0.0` and `a <= -0.0` (must preserve exact `-0.0` vs `+0.0` sign bit!)
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(0.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(0.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x66, 0x1b]); // f64.ge
    push_mix_f64(&mut b_seed);
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(-0.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 1, 0x44]);
    b_seed.extend_from_slice(&(-0.0f64).to_le_bytes());
    b_seed.extend_from_slice(&[0x65, 0x1b]); // f64.le
    push_mix_f64(&mut b_seed);
    // 4. Same for f32: strict lt/gt + clamp [-100.0f32, 100.0f32] + non-strict zero
    for op in [0x5du8, 0x5e] {
        b_seed.extend_from_slice(&[0x20, 4, 0x20, 5, 0x20, 4, 0x20, 5, op, 0x1b]);
        push_mix_f32(&mut b_seed);
    }
    b_seed.extend_from_slice(&[0x20, 4, 0x43]);
    b_seed.extend_from_slice(&(-100.0f32).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 4, 0x43]);
    b_seed.extend_from_slice(&(-100.0f32).to_le_bytes());
    b_seed.extend_from_slice(&[0x60, 0x1b]); // f32.ge
    push_mix_f32(&mut b_seed);
    b_seed.extend_from_slice(&[0x20, 4, 0x43]);
    b_seed.extend_from_slice(&(0.0f32).to_le_bytes());
    b_seed.extend_from_slice(&[0x20, 4, 0x43]);
    b_seed.extend_from_slice(&(0.0f32).to_le_bytes());
    b_seed.extend_from_slice(&[0x60, 0x1b]); // f32.ge against +0.0
    push_mix_f32(&mut b_seed);
    b_seed.extend_from_slice(&[0x20, 3]);

    // f1(packed: i32) -> i32: short-circuit `Or`/`And` on pure boolean cmps AND non-bool bitwise `And`
    // a = packed & 0xf, b = (packed >> 4) & 0xf
    // acc += ((a == 3 || b == 7) ? 1 : 0) + ((a == 3 && b == 7) ? 2 : 0) + (((a & b) != 0) ? 4 : 0)
    let mut b_sc = Vec::new();
    b_sc.extend_from_slice(&[
        0x20, 0, 0x41, 15, 0x71, 0x21, 1, // a
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // b
        // if (a == 3 || b == 7) acc += 1
        0x20, 1, 0x41, 3, 0x46, 0x20, 2, 0x41, 7, 0x46, 0x72, 0x04, 0x40, 0x20, 3, 0x41, 1, 0x6a,
        0x21, 3, 0x0b, // if (a == 3 && b == 7) acc += 2
        0x20, 1, 0x41, 3, 0x46, 0x20, 2, 0x41, 7, 0x46, 0x71, 0x04, 0x40, 0x20, 3, 0x41, 2, 0x6a,
        0x21, 3, 0x0b, // if (a & b) acc += 4  (MUST NOT be mis-split when a=1, b=2!)
        0x20, 1, 0x20, 2, 0x71, 0x04, 0x40, 0x20, 3, 0x41, 4, 0x6a, 0x21, 3, 0x0b, 0x20, 3,
    ]);

    // f2(n: i32) -> i32: unsigned counted loop writing `mem[i + 4096] = (i * 3 + 1) & 0xff` and returning sum.
    // On 1-page (65536 B) memory, `max_safe_limit = 61440`:
    //   n = 0 -> 0 iters; n = 61440 -> fast path exact boundary; n = 61441 -> slow path writes 0..61440 then traps OOB!
    let mut b_u32 = Vec::new();
    b_u32.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_u32.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // if i >=u n break
    b_u32.extend_from_slice(&[0x20, 1, 0x41]);
    leb_i32(&mut b_u32, 4096);
    b_u32.extend_from_slice(&[
        0x6a, 0x20, 1, 0x41, 3, 0x6c, 0x41, 1, 0x6a, 0x22, 3, 0x3a, 0, 0,
    ]);
    b_u32.extend_from_slice(&[0x20, 2, 0x20, 3, 0x41]);
    leb_i32(&mut b_u32, 0xff);
    b_u32.extend_from_slice(&[0x71, 0x6a, 0x21, 2]);
    b_u32.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f3(n: i32) -> i32: signed counted loop `for (i = 0; i <s n; i++)` reading `mem[i + 4096]` + writing `mem[i + 512]`.
    let mut b_s32 = Vec::new();
    b_s32.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_s32.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4e, 0x0d, 1]); // if i >=s n break
    b_s32.extend_from_slice(&[0x20, 1, 0x41]);
    leb_i32(&mut b_s32, 4096);
    b_s32.extend_from_slice(&[0x6a, 0x2d, 0, 0, 0x22, 3, 0x20, 2, 0x6a, 0x21, 2]);
    b_s32.extend_from_slice(&[0x20, 1, 0x41]);
    leb_i32(&mut b_s32, 512);
    b_s32.extend_from_slice(&[0x6a, 0x20, 3, 0x3a, 0, 0]);
    b_s32.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f4(n: i32) -> i32: 3-tap clamped stencil (`left = i==0?0:i-1`, `right = i+1>=n?n-1:i+1`)
    // + 32-byte SIB `memory.copy` at `(i << 5) + 32768 <- (i << 5)` (capped at `n <= 1024`).
    let mut b_sten = Vec::new();
    b_sten.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_sten.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]);
    // left
    b_sten.extend_from_slice(&[
        0x20, 1, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 1, 0x41, 1, 0x6b, 0x0b, 0x21, 3,
    ]);
    // right
    b_sten.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x20, 0, 0x4f, 0x04, 0x7f, 0x20, 0, 0x41, 1, 0x6b, 0x05, 0x20, 1,
        0x41, 1, 0x6a, 0x0b, 0x21, 4,
    ]);
    // acc += load8_u[left] + load8_u[i] + load8_u[right]
    b_sten.extend_from_slice(&[
        0x20, 2, 0x20, 3, 0x2d, 0, 0, 0x20, 1, 0x2d, 0, 0, 0x6a, 0x20, 4, 0x2d, 0, 0, 0x6a, 0x6a,
        0x21, 2,
    ]);
    // mem.copy dst=(i<<5)+32768, src=(i<<5), len=32
    b_sten.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x22, 5, 0x41]);
    leb_i32(&mut b_sten, 32768);
    b_sten.extend_from_slice(&[0x6a, 0x20, 5, 0x41, 32, 0xfc, 0x0a, 0, 0]);
    b_sten.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // Active data segment seeding 10 edge-case f64s at [0..80] and 10 edge-case f32s at [128..168].
    let f64_vals: [u64; 10] = [
        0.0f64.to_bits(),
        (-0.0f64).to_bits(),
        1.0f64.to_bits(),
        (-100.0f64).to_bits(),
        100.0f64.to_bits(),
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        0x7ff8_0000_0000_0001, // qNaN payload 1
        0x7ff8_0000_0000_0042, // qNaN payload 0x42
        0x7ff0_0000_0000_0007, // sNaN payload 7
    ];
    let f32_vals: [u32; 10] = [
        0.0f32.to_bits(),
        (-0.0f32).to_bits(),
        1.0f32.to_bits(),
        (-100.0f32).to_bits(),
        100.0f32.to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        0x7fc0_0001, // qNaN payload 1
        0x7fc0_0042, // qNaN payload 0x42
        0x7f80_0007, // sNaN payload 7
    ];
    let mut d64 = Vec::with_capacity(80);
    for bits in f64_vals {
        d64.extend_from_slice(&bits.to_le_bytes());
    }
    let mut d32 = Vec::with_capacity(40);
    for bits in f32_vals {
        d32.extend_from_slice(&bits.to_le_bytes());
    }
    let mut data_sec = vec![2u8]; // 2 active segments
    data_sec.extend_from_slice(&[0x00, 0x41, 0, 0x0b]);
    leb_u32(&mut data_sec, d64.len() as u32);
    data_sec.extend_from_slice(&d64);
    data_sec.extend_from_slice(&[0x00, 0x41]);
    leb_i32(&mut data_sec, 128);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, d32.len() as u32);
    data_sec.extend_from_slice(&d32);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7e]), (&[0x7f], &[0x7f])]),
        func_sec(&[0, 1, 1, 1, 1]),
        section(5, &[1, 0x01, 1, 1]), // fixed 64 KiB memory
        export_sec(&[
            ("blot:memory", 2, 0),
            ("fp_minmax", 0, 0),
            ("sc_or_and", 0, 1),
            ("loop_ver_u32", 0, 2),
            ("loop_ver_s32", 0, 3),
            ("loop_ver_stencil", 0, 4),
        ]),
        code_sec(&[
            enc_body(&[(2, 0x7c), (1, 0x7e), (2, 0x7d)], &b_seed),
            enc_body(&[(3, 0x7f)], &b_sc),
            enc_body(&[(3, 0x7f)], &b_u32),
            enc_body(&[(3, 0x7f)], &b_s32),
            enc_body(&[(5, 0x7f)], &b_sten),
        ]),
        section(11, &data_sec),
    ]);

    let mut calls = Vec::new();
    // Exhaustive 10x10 = 100 FP pairs (including +0/-0, ±100, ±inf, qNaN1, qNaN2, sNaN)!
    for idx in 0..100 {
        calls.push(c("fp_minmax", ArgSpec::I32(idx)));
    }
    for (a, b) in [(0, 0), (1, 2), (3, 0), (0, 7), (3, 7), (3, 2), (5, 7)] {
        calls.push(c("sc_or_and", ArgSpec::I32(a | (b << 4))));
    }
    for n in [0, 1, 16, 61440, 61441] {
        calls.push(c("loop_ver_u32", ArgSpec::I32(n)));
    }
    for n in [-5, -1, 0, 1, 32, 61440, 61441] {
        calls.push(c("loop_ver_s32", ArgSpec::I32(n)));
    }
    for n in [0, 1, 2, 16, 1024, 1025] {
        calls.push(c("loop_ver_stencil", ArgSpec::I32(n)));
    }
    // Verify exact memory state across both in-bounds fast paths AND post-trap partial writes!
    assert_oracle_matrix(
        "stage4_hills",
        &wasm,
        &calls,
        &[],
        &[(512, 64), (4096, 64), (32768, 128), (65472, 64)],
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage5_wave2_rotation_memcopy_forward_and_layout() {
    // f0(n: i32) -> i32: rotated loop with escaping accumulator (`sum = sum_0^n-1 (i * 7 + 3)`).
    // Tests 0-iteration guard bypass, 1-iteration, and multi-iteration rotated latches.
    let mut b_acc = Vec::new();
    b_acc.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_acc.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // if i >=u n break
    b_acc.extend_from_slice(&[
        0x20, 2, 0x20, 1, 0x41, 7, 0x6c, 0x41, 3, 0x6a, 0x6a, 0x21, 2, // sum += i*7 + 3
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f1(n: i32) -> i32: rotated loop with parallel-copy backedge swap (`a, b = b, a + b`).
    let mut b_swap = Vec::new();
    b_swap.extend_from_slice(&[0x41, 0, 0x21, 2, 0x41, 1, 0x21, 3]); // a=0, b=1
    b_swap.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_swap.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // if i >=u n break
    b_swap.extend_from_slice(&[
        0x20, 2, 0x20, 3, 0x6a, 0x21, 4, // tmp = a + b
        0x20, 3, 0x21, 2, // a = b
        0x20, 4, 0x21, 3, // b = tmp
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f2(n: i32) -> i32: rotated loop writing `mem[65530 + i] = i + 1` (traps OOB cleanly at iter 6
    // when n > 6, or succeeds when n <= 6, verifying exact partial writes before trap!).
    let mut b_trap = Vec::new();
    b_trap.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
    b_trap.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]);
    b_trap.extend_from_slice(&[0x20, 1, 0x41]);
    leb_i32(&mut b_trap, 65530);
    b_trap.extend_from_slice(&[0x6a, 0x20, 1, 0x41, 1, 0x6a, 0x3a, 0, 0]);
    b_trap.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 1,
    ]);

    // f3(packed: i64) -> i64: unpacks `dst = packed as i32`, `src = (packed >> 32) as i32`,
    // loads 4 f64 lanes from `[src + 0/8/16/24]` (including sNaN and -0.0!), stores updated
    // `[src + 0/8]`, then `memory.copy(dst, src, 32)` (forwarded to BoundsCheck + 4 checked f64 stores!).
    let mut b_mc32 = Vec::new();
    b_mc32.extend_from_slice(&[
        0x20, 0, 0xa7, 0x21, 1, // local 1 = dst
        0x20, 0, 0x42, 32, 0x88, 0xa7, 0x21, 2, // local 2 = src
        0x20, 2, 0x2b, 3, 0, 0x21, 3, // f0
        0x20, 2, 0x2b, 3, 8, 0x21, 4, // f1
        0x20, 2, 0x2b, 3, 16, 0x21, 5, // f2 (sNaN preserved bit-exact!)
        0x20, 2, 0x2b, 3, 24, 0x21, 6, // f3 (-0.0 preserved bit-exact!)
        0x20, 2, 0x20, 3, 0x44,
    ]);
    b_mc32.extend_from_slice(&(1.5f64).to_le_bytes());
    b_mc32.extend_from_slice(&[
        0xa0, 0x39, 3, 0, // store [src+0]
        0x20, 2, 0x20, 4, 0x39, 3, 8, // store [src+8]
        0x20, 1, 0x20, 2, 0x41, 32, 0xfc, 0x0a, 0, 0, // memory.copy(dst, src, 32)
        0x20, 1, 0x29, 3, 0, 0x20, 1, 0x29, 3, 8, 0x85, 0x20, 1, 0x29, 3, 16, 0x85, 0x20, 1, 0x29,
        3, 24, 0x85,
    ]);

    // f4(packed: i64) -> i32: mixed 8B i64 + 4B i32 + 4B f32 `memory.copy(dst, src, 16)`.
    let b_mc16 = vec![
        0x20, 0, 0xa7, 0x21, 1, // local 1 = dst
        0x20, 0, 0x42, 32, 0x88, 0xa7, 0x21, 2, // local 2 = src
        0x20, 2, 0x29, 3, 0, 0x21, 3, // local 3 = i64 at +0
        0x20, 2, 0x28, 2, 8, 0x21, 4, // local 4 = i32 at +8
        0x20, 2, 0x2a, 2, 12, 0x21, 5, // local 5 = f32 at +12
        0x20, 2, 0x20, 3, 0x42, 1, 0x7c, 0x37, 3, 0, // store [src+0] = i64 + 1
        0x20, 2, 0x20, 4, 0x41, 5, 0x6a, 0x36, 2, 8, // store [src+8] = i32 + 5
        0x20, 2, 0x20, 5, 0x38, 2, 12, // store [src+12] = f32
        0x20, 1, 0x20, 2, 0x41, 16, 0xfc, 0x0a, 0, 0, // memory.copy(dst, src, 16)
        0x20, 1, 0x28, 2, 0, 0x20, 1, 0x28, 2, 4, 0x73, 0x20, 1, 0x28, 2, 8, 0x73, 0x20, 1, 0x28,
        2, 12, 0x73,
    ];

    // Seed initial data at [64..128] (including sNaN payload 7 and -0.0!) and [65504..65536]
    // (sentinel bytes to prove OOB `memory.copy` writes ZERO bytes before trapping!).
    let seed_lanes: [u64; 8] = [
        2.25f64.to_bits(),
        (-42.5f64).to_bits(),
        0x7ff0_0000_0000_0007, // sNaN payload 7
        (-0.0f64).to_bits(),
        0x0123_4567_89ab_cdef,
        0xdead_beef_cafe_babe,
        0x7f80_0003_1122_3344, // f32 sNaN in upper 32 bits
        0x5566_7788_99aa_bbcc,
    ];
    let mut seed_bytes = Vec::with_capacity(64);
    for w in seed_lanes {
        seed_bytes.extend_from_slice(&w.to_le_bytes());
    }
    let sentinel = [0xA5u8; 32];
    let mut data_sec = vec![2u8];
    data_sec.extend_from_slice(&[0x00, 0x41]);
    leb_i32(&mut data_sec, 64);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, seed_bytes.len() as u32);
    data_sec.extend_from_slice(&seed_bytes);
    data_sec.extend_from_slice(&[0x00, 0x41]);
    leb_i32(&mut data_sec, 65504);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, sentinel.len() as u32);
    data_sec.extend_from_slice(&sentinel);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e]), (&[0x7e], &[0x7f])]),
        func_sec(&[0, 0, 0, 1, 2]),
        section(5, &[1, 0x01, 1, 1]), // fixed 64 KiB memory
        export_sec(&[
            ("blot:memory", 2, 0),
            ("rot_acc", 0, 0),
            ("rot_swap", 0, 1),
            ("rot_trap", 0, 2),
            ("memcpy_fwd_32", 0, 3),
            ("memcpy_fwd_16", 0, 4),
        ]),
        code_sec(&[
            enc_body(&[(2, 0x7f)], &b_acc),
            enc_body(&[(4, 0x7f)], &b_swap),
            enc_body(&[(1, 0x7f)], &b_trap),
            enc_body(&[(2, 0x7f), (4, 0x7c)], &b_mc32),
            enc_body(&[(2, 0x7f), (1, 0x7e), (1, 0x7f), (1, 0x7d)], &b_mc16),
        ]),
        section(11, &data_sec),
    ]);

    let pack = |dst: i32, src: i32| -> ArgSpec {
        ArgSpec::I64((u64::from(dst as u32) | (u64::from(src as u32) << 32)) as i64)
    };
    let mut calls = Vec::new();
    for n in [0, 1, 2, 17, 100] {
        calls.push(c("rot_acc", ArgSpec::I32(n)));
    }
    for n in [0, 1, 2, 10, 25] {
        calls.push(c("rot_swap", ArgSpec::I32(n)));
    }
    for (dst, src) in [
        (256, 64),     // disjoint
        (72, 64),      // overlap dst > src (+8)
        (80, 64),      // overlap dst > src (+16)
        (56, 64),      // overlap dst < src (-8)
        (48, 64),      // overlap dst < src (-16)
        (64, 64),      // exact self-overlap
        (65504, 64),   // exact in-bounds end boundary (65504 + 32 == 65536)
        (65520, 64),   // OOB dst -> MUST trap with ZERO bytes modified at [65520..65536]!
        (1024, 65520), // OOB src -> MUST trap with ZERO bytes modified at [1024..1056]!
    ] {
        calls.push(c("memcpy_fwd_32", pack(dst, src)));
    }
    for (dst, src) in [
        (512, 96),
        (104, 96),
        (88, 96),
        (96, 96),
        (65520, 96),
        (65528, 96),
    ] {
        calls.push(c("memcpy_fwd_16", pack(dst, src)));
    }
    for n in [0, 1, 6, 10] {
        calls.push(c("rot_trap", ArgSpec::I32(n)));
    }
    assert_oracle_matrix(
        "stage5_wave2",
        &wasm,
        &calls,
        &[],
        &[(48, 80), (256, 32), (512, 16), (1024, 32), (65504, 32)],
    );
}

fn push_simd_op(out: &mut Vec<u8>, op: u16) {
    assert_eq!(op >> 8, 0xfd);
    out.push(0xfd);
    leb_u32(out, u32::from(op & 0xff));
}

fn push_v128_const(out: &mut Vec<u8>, bytes: [u8; 16]) {
    out.push(0xfd);
    leb_u32(out, 12);
    out.extend_from_slice(&bytes);
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn oracle_hill1_all_236_simd_opcodes() {
    // Track EVERY valid 0xfd00..=0xfdff opcode we emit and assert total == 236!
    let covered = std::cell::RefCell::new(std::collections::BTreeSet::<u16>::new());
    let emit_op = |out: &mut Vec<u8>, op: u16| {
        covered.borrow_mut().insert(op);
        push_simd_op(out, op);
    };

    // Seed linear memory [0..256] with rich edge-case bytes (int boundaries, floats, NaNs, +/-0, +/-Inf).
    let mut seed = [0u8; 256];
    // [0..16]: signed/unsigned interesting bytes
    seed[0..16].copy_from_slice(&[
        0x00, 0x01, 0x7f, 0x80, 0x81, 0xff, 0x10, 0x20, 0xfe, 0x02, 0x55, 0xaa, 0x0f, 0xf0, 0x33,
        0xcc,
    ]);
    // [16..32]: second byte vector (includes -32768 in i16 lane 0 paired with -32768 at [0..2] -> put -32768 in both!)
    seed[0..2].copy_from_slice(&(-32768i16).to_le_bytes());
    seed[16..32].copy_from_slice(&[
        0x00, 0x80, 0x7f, 0x80, 0xff, 0x01, 0x00, 0x7f, 0x03, 0x11, 0x10, 0x20, 0xff, 0x00, 0x07,
        0x08,
    ]);
    // [32..48]: f32x4 vector A: [-0.0, 3.75, NaN, -2147483904.0]
    for (i, v) in [
        (-0.0f32).to_bits(),
        3.75f32.to_bits(),
        0x7fc0_1234u32, // non-canonical qNaN
        (-2147483904.0f32).to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[32 + i * 4..36 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    // [48..64]: f32x4 vector B: [+0.0, -3.75, 2147483648.0, 4294967296.0]
    for (i, v) in [
        0.0f32.to_bits(),
        (-3.75f32).to_bits(),
        2147483648.0f32.to_bits(),
        4294967296.0f32.to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[48 + i * 4..52 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    // [64..80]: f32x4 vector C: [f32::INFINITY, f32::NEG_INFINITY, 3000000000.0, -0.5]
    for (i, v) in [
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        3000000000.0f32.to_bits(),
        (-0.5f32).to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[64 + i * 4..68 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    // [80..96]: f64x2 vector A: [-0.0, 3.75]
    seed[80..88].copy_from_slice(&(-0.0f64).to_bits().to_le_bytes());
    seed[88..96].copy_from_slice(&3.75f64.to_bits().to_le_bytes());
    // [96..112]: f64x2 vector B: [+0.0, NaN]
    seed[96..104].copy_from_slice(&0.0f64.to_bits().to_le_bytes());
    seed[104..112].copy_from_slice(&0x7ff8_0000_0000_0042u64.to_le_bytes());
    // [112..128]: f64x2 vector C: [-3e9, 5e9]
    seed[112..120].copy_from_slice(&(-3_000_000_000.0f64).to_bits().to_le_bytes());
    seed[120..128].copy_from_slice(&5_000_000_000.0f64.to_bits().to_le_bytes());
    // [128..144]: f64x2 vector D: [f64::NEG_INFINITY, 3_000_000_000.5]
    seed[128..136].copy_from_slice(&f64::NEG_INFINITY.to_bits().to_le_bytes());
    seed[136..144].copy_from_slice(&3_000_000_000.5f64.to_bits().to_le_bytes());
    // [144..160]: i64x2 extremes
    seed[144..152].copy_from_slice(&i64::MIN.to_le_bytes());
    seed[152..160].copy_from_slice(&i64::MAX.to_le_bytes());
    // [160..176]: i64x2 second
    seed[160..168].copy_from_slice(&(-1i64).to_le_bytes());
    seed[168..176].copy_from_slice(&0x0123_4567_89ab_cdefi64.to_le_bytes());

    // Exact unsigned conversion inputs and expected f32 bits. Include ties on
    // both sides of 2^31, ties rounding down/up to even, and adjacent non-ties.
    let unsigned_to_f32: [([u32; 4], [u32; 4]); 4] = [
        (
            [0, 1, 0x00ff_ffff, 0x7fff_ffff],
            [0, 0x3f80_0000, 0x4b7f_ffff, 0x4f00_0000],
        ),
        (
            [0x8000_0000, 0xffff_ffff, 0x0100_0001, 0x0100_0003],
            [0x4f00_0000, 0x4f80_0000, 0x4b80_0000, 0x4b80_0002],
        ),
        (
            [0x8000_0080, 0x8000_0180, 0x8000_0081, 0xffff_ff7f],
            [0x4f00_0000, 0x4f00_0002, 0x4f00_0001, 0x4f7f_ffff],
        ),
        (
            [0xffff_ff80, 0x0100_0002, 0x0200_0002, 0x0200_0006],
            [0x4f80_0000, 0x4b80_0001, 0x4c00_0000, 0x4c00_0002],
        ),
    ];
    for (i, (inputs, _)) in unsigned_to_f32.iter().enumerate() {
        let off = 176 + 16 * i;
        seed[off..off + 16].copy_from_slice(&inputs.map(u32::to_le_bytes).concat());
    }

    // Output cursor in memory starts at offset 512 (each result is 16 bytes -> fits easily in 1 page!).
    let mut out_off: i32 = 512;
    let store_v128 =
        |b: &mut Vec<u8>,
         cursor: &mut i32,
         cov: &std::cell::RefCell<std::collections::BTreeSet<u16>>| {
            // Stack has [v128]; save into local 1, then store at cursor!
            b.extend_from_slice(&[0x21, 1]); // local.set 1
            b.push(0x41);
            leb_i32(b, *cursor);
            b.extend_from_slice(&[0x20, 1]); // local.get 1
            cov.borrow_mut().insert(0xfd0b);
            push_simd_op(b, 0xfd0b); // v128.store
            b.extend_from_slice(&[0, 0]);
            *cursor += 16;
        };
    let load_at =
        |b: &mut Vec<u8>, off: i32, cov: &std::cell::RefCell<std::collections::BTreeSet<u16>>| {
            b.push(0x41);
            leb_i32(b, off);
            cov.borrow_mut().insert(0xfd00);
            push_simd_op(b, 0xfd00);
            b.extend_from_slice(&[0, 0]);
        };

    // f0: (i32) -> (i32): memory loads, lane loads/stores, shuffle, swizzle, splats, extract/replace, to_i32
    // locals: param 0 = i32 (shift/selector), local 1 = v128 tmp, local 2 = i32 hash acc
    let mut b0 = Vec::new();
    // All 12 SIMD load variants (0xfd00..=0xfd0a, 0xfd5c, 0xfd5d)
    for op in [
        0xfd00, 0xfd01, 0xfd02, 0xfd03, 0xfd04, 0xfd05, 0xfd06, 0xfd07, 0xfd08, 0xfd09, 0xfd0a,
        0xfd5c, 0xfd5d,
    ] {
        b0.extend_from_slice(&[0x41, 0]); // addr = 0
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0]); // align=0, offset=0
        store_v128(&mut b0, &mut out_off, &covered);
    }
    // v128.const (0xfd0c)
    covered.borrow_mut().insert(0xfd0c);
    push_v128_const(
        &mut b0,
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    );
    store_v128(&mut b0, &mut out_off, &covered);
    // i8x16.shuffle (0xfd0d): both inputs, a-only, b-only
    for lanes in [
        [0, 16, 1, 17, 2, 18, 15, 31, 30, 14, 7, 23, 8, 24, 0, 31],
        [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
        [
            16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    ] {
        load_at(&mut b0, 0, &covered);
        load_at(&mut b0, 16, &covered);
        emit_op(&mut b0, 0xfd0d);
        b0.extend_from_slice(&lanes);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    // i8x16.swizzle (0xfd0e)
    load_at(&mut b0, 0, &covered);
    load_at(&mut b0, 16, &covered);
    emit_op(&mut b0, 0xfd0e);
    store_v128(&mut b0, &mut out_off, &covered);
    // Splats (0xfd0f..=0xfd14)
    for (op, push_arg) in [
        (0xfd0fu16, vec![0x20, 0]),
        (0xfd10, vec![0x20, 0]),
        (0xfd11, vec![0x20, 0]),
        (0xfd12, vec![0x20, 0, 0xac]), // i64.extend_i32_s
        (0xfd13, vec![0x20, 0, 0xb2]), // f32.convert_i32_s
        (0xfd14, vec![0x20, 0, 0xb7]), // f64.convert_i32_s
    ] {
        b0.extend_from_slice(&push_arg);
        emit_op(&mut b0, op);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    // Extract & replace lanes (0xfd15..=0xfd22)
    let mix_i32 = |b: &mut Vec<u8>| {
        // stack has [i32]: acc = acc * 31 ^ val
        b.extend_from_slice(&[0x20, 2, 0x41, 31, 0x6c, 0x73, 0x21, 2]);
    };
    for (op, lane, conv) in [
        (0xfd15u16, 3u8, vec![]),      // i8x16.extract_lane_s
        (0xfd16, 3, vec![]),           // i8x16.extract_lane_u
        (0xfd18, 1, vec![]),           // i16x8.extract_lane_s
        (0xfd19, 1, vec![]),           // i16x8.extract_lane_u
        (0xfd1b, 2, vec![]),           // i32x4.extract_lane
        (0xfd1d, 1, vec![0xa7]),       // i64x2.extract_lane -> wrap i32
        (0xfd1f, 1, vec![0xbc]),       // f32x4.extract_lane -> reinterpret i32
        (0xfd21, 1, vec![0xbd, 0xa7]), // f64x2.extract_lane -> reinterpret i64 -> wrap
    ] {
        load_at(&mut b0, if op >= 0xfd1f { 32 } else { 0 }, &covered);
        emit_op(&mut b0, op);
        b0.push(lane);
        b0.extend_from_slice(&conv);
        mix_i32(&mut b0);
    }
    for (op, lane, val_ops) in [
        (0xfd17u16, 5u8, vec![0x20, 0]),
        (0xfd1a, 3, vec![0x20, 0]),
        (0xfd1c, 2, vec![0x20, 0]),
        (0xfd1e, 1, vec![0x20, 0, 0xac]),
        (0xfd20, 2, vec![0x20, 0, 0xb2]),
        (0xfd22, 1, vec![0x20, 0, 0xb7]),
    ] {
        load_at(&mut b0, 0, &covered);
        b0.extend_from_slice(&val_ops);
        emit_op(&mut b0, op);
        b0.push(lane);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    // Load/Store lane (0xfd54..=0xfd5b)
    for (op, lane) in [(0xfd54u16, 7u8), (0xfd55, 5), (0xfd56, 3), (0xfd57, 1)] {
        b0.extend_from_slice(&[0x41, 4]); // mem addr = 4
        load_at(&mut b0, 0, &covered);
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0, lane]);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    for (op, lane) in [(0xfd58u16, 7u8), (0xfd59, 5), (0xfd5a, 3), (0xfd5b, 1)] {
        b0.push(0x41);
        leb_i32(&mut b0, out_off);
        load_at(&mut b0, 0, &covered);
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0, lane]);
        out_off += 16;
    }
    // V128ToI32: any_true, all_true, bitmask (test on both non-zero and containing-zero vectors!)
    for op in [
        0xfd53u16, 0xfd63, 0xfd83, 0xfda3, 0xfdc3, 0xfd64, 0xfd84, 0xfda4, 0xfdc4,
    ] {
        for off in [0, 16, 144] {
            load_at(&mut b0, off, &covered);
            emit_op(&mut b0, op);
            mix_i32(&mut b0);
        }
        // All-ones vector so all_true returns 1!
        push_v128_const(&mut b0, [0xff; 16]);
        emit_op(&mut b0, op);
        mix_i32(&mut b0);
    }
    // v128.bitselect (0xfd52)
    load_at(&mut b0, 0, &covered);
    load_at(&mut b0, 16, &covered);
    load_at(&mut b0, 144, &covered);
    emit_op(&mut b0, 0xfd52);
    store_v128(&mut b0, &mut out_off, &covered);
    b0.extend_from_slice(&[0x20, 2]); // return acc

    // f1: (i32) -> (i32): ALL unary V128 -> V128 ops + ALL 12 shift ops!
    let mut b1 = Vec::new();
    let unary_ops: &[(u16, &[i32])] = &[
        (0xfd4d, &[0]),
        (0xfd5e, &[80, 96, 112]),
        (0xfd5f, &[32, 48, 64]),
        (0xfd60, &[0, 16]),
        (0xfd61, &[0, 16]),
        (0xfd62, &[0, 16]),
        (0xfd67, &[32, 48, 64]),
        (0xfd68, &[32, 48, 64]),
        (0xfd69, &[32, 48, 64]),
        (0xfd6a, &[32, 48, 64]),
        (0xfd74, &[80, 96, 112, 128]),
        (0xfd75, &[80, 96, 112, 128]),
        (0xfd7a, &[80, 96, 112, 128]),
        (0xfd94, &[80, 96, 112, 128]),
        (0xfd7c, &[0, 16]),
        (0xfd7d, &[0, 16]),
        (0xfd7e, &[0, 16]),
        (0xfd7f, &[0, 16]),
        (0xfd80, &[0, 16]),
        (0xfd81, &[0, 16]),
        (0xfd87, &[0]),
        (0xfd88, &[0]),
        (0xfd89, &[0]),
        (0xfd8a, &[0]),
        (0xfda0, &[0, 144]),
        (0xfda1, &[0, 144]),
        (0xfda7, &[0]),
        (0xfda8, &[0]),
        (0xfda9, &[0]),
        (0xfdaa, &[0]),
        (0xfdc0, &[144, 160]),
        (0xfdc1, &[144, 160]),
        (0xfdc7, &[0, 144]),
        (0xfdc8, &[0, 144]),
        (0xfdc9, &[0, 144]),
        (0xfdca, &[0, 144]),
        (0xfde0, &[32, 48, 64]),
        (0xfde1, &[32, 48, 64]),
        (0xfde3, &[32, 48, 64]),
        (0xfdec, &[80, 96, 112]),
        (0xfded, &[80, 96, 112]),
        (0xfdef, &[80, 96, 112]),
        (0xfdf8, &[32, 48, 64]),
        (0xfdf9, &[32, 48, 64]),
        (0xfdfa, &[0, 144]),
        (0xfdfb, &[0, 144]),
        (0xfdfc, &[80, 96, 112, 128]),
        (0xfdfd, &[80, 96, 112, 128]),
        (0xfdfe, &[0, 144]),
        (0xfdff, &[0, 144]),
    ];
    for &(op, offs) in unary_ops {
        for &off in offs {
            load_at(&mut b1, off, &covered);
            emit_op(&mut b1, op);
            store_v128(&mut b1, &mut out_off, &covered);
        }
    }
    let u32_to_f32_output = out_off;
    for off in [176, 192, 208, 224] {
        load_at(&mut b1, off, &covered);
        emit_op(&mut b1, 0xfdfb);
        store_v128(&mut b1, &mut out_off, &covered);
    }
    // All 12 shifts using parameter 0 (dynamic shift count!)
    for op in [
        0xfd6bu16, 0xfd6c, 0xfd6d, 0xfd8b, 0xfd8c, 0xfd8d, 0xfdab, 0xfdac, 0xfdad, 0xfdcb, 0xfdcc,
        0xfdcd,
    ] {
        for off in [0, 144] {
            load_at(&mut b1, off, &covered);
            b1.extend_from_slice(&[0x20, 0]);
            emit_op(&mut b1, op);
            store_v128(&mut b1, &mut out_off, &covered);
        }
    }
    b1.push(0x41);
    leb_i32(&mut b1, out_off);

    // f2: (i32) -> (i32): ALL binary (V128, V128) -> V128 ops!
    let mut b2 = Vec::new();
    let int_bin_ops: &[u16] = &[
        // i8x16 cmp
        0xfd23, 0xfd24, 0xfd25, 0xfd26, 0xfd27, 0xfd28, 0xfd29, 0xfd2a, 0xfd2b, 0xfd2c,
        // i16x8 cmp
        0xfd2d, 0xfd2e, 0xfd2f, 0xfd30, 0xfd31, 0xfd32, 0xfd33, 0xfd34, 0xfd35, 0xfd36,
        // i32x4 cmp
        0xfd37, 0xfd38, 0xfd39, 0xfd3a, 0xfd3b, 0xfd3c, 0xfd3d, 0xfd3e, 0xfd3f, 0xfd40,
        // bitwise
        0xfd4e, 0xfd4f, 0xfd50, 0xfd51, // narrow
        0xfd65, 0xfd66, 0xfd85, 0xfd86, // i8x16 arith
        0xfd6e, 0xfd6f, 0xfd70, 0xfd71, 0xfd72, 0xfd73, 0xfd76, 0xfd77, 0xfd78, 0xfd79, 0xfd7b,
        // i16x8 arith & extmul
        0xfd82, 0xfd8e, 0xfd8f, 0xfd90, 0xfd91, 0xfd92, 0xfd93, 0xfd95, 0xfd96, 0xfd97, 0xfd98,
        0xfd99, 0xfd9b, 0xfd9c, 0xfd9d, 0xfd9e, 0xfd9f, // i32x4 arith, dot, extmul
        0xfdae, 0xfdb1, 0xfdb5, 0xfdb6, 0xfdb7, 0xfdb8, 0xfdb9, 0xfdba, 0xfdbc, 0xfdbd, 0xfdbe,
        0xfdbf, // i64x2 arith, cmp, extmul
        0xfdce, 0xfdd1, 0xfdd5, 0xfdd6, 0xfdd7, 0xfdd8, 0xfdd9, 0xfdda, 0xfddb, 0xfddc, 0xfddd,
        0xfdde, 0xfddf,
    ];
    for &op in int_bin_ops {
        let (a_off, b_off) = if op >= 0xfdce { (144, 160) } else { (0, 16) };
        load_at(&mut b2, a_off, &covered);
        load_at(&mut b2, b_off, &covered);
        emit_op(&mut b2, op);
        store_v128(&mut b2, &mut out_off, &covered);
    }
    // f32x4 cmp & arith
    for op in [
        0xfd41u16, 0xfd42, 0xfd43, 0xfd44, 0xfd45, 0xfd46, 0xfde4, 0xfde5, 0xfde6, 0xfde7, 0xfde8,
        0xfde9, 0xfdea, 0xfdeb,
    ] {
        for (a_off, b_off) in [(32, 48), (48, 32), (32, 64)] {
            load_at(&mut b2, a_off, &covered);
            load_at(&mut b2, b_off, &covered);
            emit_op(&mut b2, op);
            store_v128(&mut b2, &mut out_off, &covered);
        }
    }
    // f64x2 cmp & arith
    for op in [
        0xfd47u16, 0xfd48, 0xfd49, 0xfd4a, 0xfd4b, 0xfd4c, 0xfdf0, 0xfdf1, 0xfdf2, 0xfdf3, 0xfdf4,
        0xfdf5, 0xfdf6, 0xfdf7,
    ] {
        for (a_off, b_off) in [(80, 96), (96, 80), (112, 128)] {
            load_at(&mut b2, a_off, &covered);
            load_at(&mut b2, b_off, &covered);
            emit_op(&mut b2, op);
            store_v128(&mut b2, &mut out_off, &covered);
        }
    }
    b2.push(0x41);
    leb_i32(&mut b2, out_off);

    // Assert we covered ALL 236 standard Wasm 1.0 fixed-width SIMD opcodes!
    assert_eq!(
        covered.borrow().len(),
        236,
        "expected all 236 fixed-width Wasm SIMD opcodes covered, got {}",
        covered.borrow().len()
    );
    assert!(
        out_off < 65_536,
        "test output exceeded 1 memory page: {out_off}"
    );

    // f3: (i32) -> (i32): OOB trap test for v128.load32_lane at dynamic addr
    let mut b3 = vec![0x20, 0];
    push_v128_const(&mut b3, [0; 16]);
    push_simd_op(&mut b3, 0xfd56);
    b3.extend_from_slice(&[0, 1, 0]); // align=0, offset=1, lane=0
    push_simd_op(&mut b3, 0xfd1b);
    b3.push(0);

    let mut data_sec = vec![1, 0x00, 0x41, 0, 0x0b];
    leb_u32(&mut data_sec, seed.len() as u32);
    data_sec.extend_from_slice(&seed);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0, 0, 0]),
        section(5, &[1, 0x01, 1, 2]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("simd_misc", 0, 0),
            ("simd_un_shift", 0, 1),
            ("simd_bin", 0, 2),
            ("simd_lane_oob", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[(1, 0x7b), (1, 0x7f)], &b0),
            enc_body(&[(1, 0x7b)], &b1),
            enc_body(&[(1, 0x7b)], &b2),
            enc_body(&[], &b3),
        ]),
        section(11, &data_sec),
    ]);

    let mut calls = vec![
        c("simd_misc", ArgSpec::I32(-42)),
        c("simd_bin", ArgSpec::I32(0)),
        c("simd_lane_oob", ArgSpec::I32(65_531)), // exact in-bounds
        c("simd_lane_oob", ArgSpec::I32(65_532)), // OOB trap!
    ];
    for shift in [0, 1, 7, 8, 15, 31, 63, 65, -1] {
        calls.push(c("simd_un_shift", ArgSpec::I32(shift)));
    }
    #[cfg(target_arch = "x86_64")]
    assert_oracle_matrix(
        "hill1_simd_236",
        &wasm,
        &calls,
        &[],
        &[(512, (out_off - 512) as usize)],
    );

    let Some(driver) = get_oracle_driver() else {
        return;
    };
    let root = crate::build::create_fresh_test_root("wtime-aot-c-simd-oracle");
    let baseline = if cfg!(target_arch = "x86_64") {
        "x86-64"
    } else {
        "generic"
    };
    for (cpu, opt) in [
        (baseline, crate::cgen::COpt::O2),
        ("native", crate::cgen::COpt::Auto),
    ] {
        let so = root.0.join(format!("{cpu}.so"));
        match crate::cgen::detect_cc_with_deadline(cpu, None, None) {
            Ok(_) => {}
            Err(err)
                if std::env::var_os("WTIME_AOT_REQUIRE_NATIVE_TESTS").is_some_and(|v| v == "1") =>
            {
                panic!("C compiler probe failed ({cpu}): {err}");
            }
            Err(_) => return,
        }
        crate::cgen::build_c_blot_cdylib(
            &wasm,
            &root.0.join("cache"),
            cpu,
            Some(2),
            opt,
            Some(std::time::Duration::from_secs(120)),
            &so,
        )
        .unwrap_or_else(|e| panic!("C SIMD compile failed ({cpu}): {e}"));
        let expected_bytes: Vec<u8> = unsigned_to_f32
            .iter()
            .flat_map(|(_, bits)| bits.iter().flat_map(|bits| bits.to_le_bytes()))
            .collect();
        let check_mem = [
            (u32_to_f32_output as u64, expected_bytes.len()),
            (512, (out_off - 512) as usize),
        ];
        // Compare after each shift so later calls cannot overwrite a bad result.
        for shift in [0, 1, 7, 8, 15, 31, 63, 65, -1] {
            let mut case_calls = calls[..4].to_vec();
            case_calls.push(c("simd_un_shift", ArgSpec::I32(shift)));
            let actual = run_blot_so(driver, &so, &case_calls, &[], &check_mem, true);
            let expected = run_wtime_oracle(&wasm, &case_calls, &[], &check_mem);
            assert_eq!(
                actual.outcomes, expected.outcomes,
                "C SIMD outcomes ({cpu}, shift={shift})"
            );
            assert_eq!(
                actual.mem_slices[0].1, expected_bytes,
                "f32x4.convert_i32x4_u ({cpu})"
            );
            let mismatch = actual.mem_slices[1]
                .1
                .iter()
                .zip(&expected.mem_slices[1].1)
                .position(|(actual, expected)| actual != expected);
            assert_eq!(
                mismatch, None,
                "C SIMD output differs at the indicated byte relative to 512 ({cpu}, shift={shift})"
            );
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_hill1_memory64() {
    // Active data segment with i64.const offset = 16
    let mut data_sec = vec![1, 0x00, 0x42, 16, 0x0b, 8];
    data_sec.extend_from_slice(&0x0123_4567_89ab_cdefu64.to_le_bytes());

    // f0: (i64) -> (i64): i64.load align=0 offset=0 at addr
    let b0 = vec![0x20, 0, 0x29, 0, 0];
    // f1: (i64) -> (i64): i64.load align=0 offset=0x1_0000_0000 (4 GiB offset -> always OOB on 4-page mem!)
    let mut b1 = vec![0x20, 0, 0x29, 0];
    // LEB128 u64 for 0x1_0000_0000 (4294967296): [0x80, 0x80, 0x80, 0x80, 0x10]
    b1.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x10]);
    // f2: (i64) -> (i64): memory.grow(delta) -> returns old pages or -1i64
    let b2 = vec![0x20, 0, 0x40, 0];
    // f3: (i64) -> (i64): memory.size
    let b3 = vec![0x3f, 0];
    // f4: (i64) -> (i64): memory.fill(dst=addr, val=0x5a, len=32) + memory.copy(dst=addr+16, src=addr, len=32) + i64.load(addr+24)
    let b4 = vec![
        0x20, 0, 0x41, 0x5a, 0x42, 32, 0xfc, 11, 0, // memory.fill
        0x20, 0, 0x42, 16, 0x7c, 0x20, 0, 0x42, 32, 0xfc, 10, 0, 0, // memory.copy
        0x20, 0, 0x29, 0, 24, // i64.load offset=24
    ];
    // f5: (i64) -> (i64): v128.load(addr) -> i64x2.add with v128.const -> v128.store(addr + 32) -> i64.load(addr + 40)
    let mut b5 = vec![0x20, 0, 0x42, 32, 0x7c, 0x20, 0];
    push_simd_op(&mut b5, 0xfd00); // v128.load
    b5.extend_from_slice(&[0, 0]);
    push_v128_const(&mut b5, [1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    push_simd_op(&mut b5, 0xfdce); // i64x2.add
    push_simd_op(&mut b5, 0xfd0b); // v128.store
    b5.extend_from_slice(&[0, 0]);
    b5.extend_from_slice(&[0x20, 0, 0x29, 0, 40]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7e], &[0x7e])]),
        func_sec(&[0, 0, 0, 0, 0, 0]),
        // memory64 with min=1, max=3 (flags = 0x05)
        section(5, &[1, 0x05, 1, 3]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("load64", 0, 0),
            ("load_hi_off", 0, 1),
            ("grow64", 0, 2),
            ("size64", 0, 3),
            ("copyfill64", 0, 4),
            ("simd64", 0, 5),
        ]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[], &b1),
            enc_body(&[], &b2),
            enc_body(&[], &b3),
            enc_body(&[], &b4),
            enc_body(&[], &b5),
        ]),
        section(11, &data_sec),
    ]);

    let calls = vec![
        c("size64", ArgSpec::I64(0)),
        c("load64", ArgSpec::I64(16)),
        c("load64", ArgSpec::I64(65_528)), // exact in-bounds end
        c("load64", ArgSpec::I64(65_529)), // OOB trap
        c("load64", ArgSpec::I64(-1)),     // u64::MAX + 8 overflow OOB trap!
        c("load_hi_off", ArgSpec::I64(0)), // offset 2^32 OOB trap!
        c("simd64", ArgSpec::I64(16)),
        c("copyfill64", ArgSpec::I64(128)),
        c("copyfill64", ArgSpec::I64(-4)), // u64 overflow in memory.fill -> trap!
        c("grow64", ArgSpec::I64(2)),      // 1 -> 3 (ok, returns 1)
        c("size64", ArgSpec::I64(0)),      // 3
        c("load64", ArgSpec::I64(131_072)), // now in bounds on page 2!
        c("grow64", ArgSpec::I64(1)),      // exceeds max=3 -> returns -1i64!
        c("grow64", ArgSpec::I64(1 << 40)), // huge delta -> returns -1i64!
    ];
    assert_oracle_matrix("hill1_memory64", &wasm, &calls, &[], &[(0, 256)]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_hill3_autovec_psadbw_slp_f64x2_and_boundary_peel() {
    // 1. Byte-sum reduction (`psadbw` + scalar tail) with non-zero init, non-zero offset,
    //    exact mod-2^32 wrap, and `do-while` edge cases (`limit = 0, 1, 15, 16, 17, 31, 32, 33, 64`).
    // 2. Boundary-clamped 1D 3-tap stencil (`i == 0` and `i + 1 >= n` clamps) across `n = 0, 1, 2, 3, 4, 17, 64`.
    // 3. `f64x2` SLP (paired loads, arithmetic, clamp(-100, 100), loop-carried accumulators, paired stores)
    //    including NaN, -0.0, +0.0, +/-inf, and an aliasing store test.
    let mut mem_init = vec![0u8; 512];
    for (i, b) in mem_init.iter_mut().enumerate() {
        *b = ((i.wrapping_mul(37).wrapping_add(13)) & 0xff) as u8;
    }
    let mut data_sec = vec![1, 0, 0x41, 0, 0x0b];
    leb_u32(&mut data_sec, mem_init.len() as u32);
    data_sec.extend_from_slice(&mem_init);

    // f0(n: i32) -> i32: byte sum over [0..n) at offset 16, starting acc = 0xffff_ff00 (tests mod-2^32 wrap!)
    let mut b0 = vec![
        0x41, 0, 0x21, 1, // i = 0
        0x41, 0x80, 0x7e, 0x21, 2, // acc = -256 (0xffff_ff00)
        0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, // if i >=u n break
        0x20, 2, 0x20, 1, 0x2d, 0, 16, 0x6a, 0x21, 2, // acc += mem.u8[i + 16]
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, // i += 1
        0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ];
    let _ = &mut b0;

    // f1(n: i32) -> i32: boundary-clamped 3-tap stencil [0..n) -> out at +256, then byte sum of out[0..n)
    let b1 = vec![
        0x41, 0, 0x21, 1, // i = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, // if i >=u n break
        // left = i == 0 ? 0 : i - 1
        0x41, 0, 0x20, 1, 0x41, 1, 0x6b, 0x20, 1, 0x45, 0x1b, 0x21, 2,
        // right = i + 1 >=u n ? n - 1 : i + 1
        0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x41, 1, 0x6a, 0x20, 1, 0x41, 1, 0x6a, 0x20, 0, 0x4f, 0x1b,
        0x21, 3, // out[i + 256] = (in[left] + in[i] + in[right]) /u 3
        0x20, 1, 0x20, 2, 0x2d, 0, 0, 0x20, 1, 0x2d, 0, 0, 0x6a, 0x20, 3, 0x2d, 0, 0, 0x6a, 0x41,
        3, 0x6e, 0x3a, 0, 0x80, 0x02, // i32.store8 offset=256
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, // sum out[256..256+n)
        0x41, 0, 0x21, 1, 0x41, 0, 0x21, 2, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d,
        1, 0x20, 2, 0x20, 1, 0x2d, 0, 0x80, 0x02, 0x6a, 0x21, 2, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1,
        0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ];

    // f2(mode: i32) -> i64: writes special IEEE-754 f64 pairs (NaN, -0.0, +0.0, +/-inf, finite)
    // into memory at +1024, runs an f64x2 SLP-eligible loop with clamp(-100.0, 100.0), paired stores,
    // and loop-carried accumulators, and returns raw f64 bits as i64!
    let mut b2: Vec<u8> = Vec::new();
    let pairs: [(f64, f64, f64, f64); 4] = [
        (0.0, -0.0, 1.5, -2.5),
        (f64::NAN, 99.5, 10.0, 5.0),
        (f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.0),
        (-100.0, 100.0, -0.5, 0.5),
    ];
    for (idx, &(x, y, vx, vy)) in pairs.iter().enumerate() {
        let base = 1024 + (idx as u32) * 32;
        for (k, val) in [x, y, vx, vy].iter().enumerate() {
            b2.push(0x41);
            leb_i32(&mut b2, (base + (k as u32) * 8) as i32);
            b2.push(0x44);
            b2.extend_from_slice(&val.to_le_bytes());
            b2.extend_from_slice(&[0x39, 3, 0]);
        }
    }
    // Loop i = 0..mode (clamped to 4):
    // locals: 0=n(i32), 1=i(i32), 2=ptr(i32), 3=ax(f64), 4=ay(f64), 5=x(f64), 6=y(f64), 7=vx(f64), 8=vy(f64)
    b2.extend_from_slice(&[
        0x41, 0, 0x21, 1, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, 0x20, 1, 0x41,
        5, 0x74, 0x41, 0x80, 0x08, 0x6a, 0x22, 2, // ptr = 1024 + (i << 5)
        0x2b, 3, 0, 0x21, 5, 0x20, 2, 0x2b, 3, 8, 0x21, 6, 0x20, 2, 0x2b, 3, 16, 0x21, 7, 0x20, 2,
        0x2b, 3, 24, 0x21, 8,
    ]);
    // x' = clamp(x + vx * 0.5, -100.0, 100.0)
    b2.extend_from_slice(&[0x20, 5, 0x20, 7, 0x44]);
    b2.extend_from_slice(&0.5f64.to_le_bytes());
    b2.extend_from_slice(&[0xa2, 0xa0, 0x22, 5]);
    b2.push(0x44);
    b2.extend_from_slice(&(-100.0f64).to_le_bytes());
    b2.extend_from_slice(&[0x20, 5, 0x44]);
    b2.extend_from_slice(&(-100.0f64).to_le_bytes());
    b2.extend_from_slice(&[0x66, 0x1b, 0x22, 5]);
    b2.push(0x44);
    b2.extend_from_slice(&100.0f64.to_le_bytes());
    b2.extend_from_slice(&[0x20, 5, 0x44]);
    b2.extend_from_slice(&100.0f64.to_le_bytes());
    b2.extend_from_slice(&[0x65, 0x1b, 0x21, 5]);
    // y' = clamp(y + vy * 0.5, -100.0, 100.0)
    b2.extend_from_slice(&[0x20, 6, 0x20, 8, 0x44]);
    b2.extend_from_slice(&0.5f64.to_le_bytes());
    b2.extend_from_slice(&[0xa2, 0xa0, 0x22, 6]);
    b2.push(0x44);
    b2.extend_from_slice(&(-100.0f64).to_le_bytes());
    b2.extend_from_slice(&[0x20, 6, 0x44]);
    b2.extend_from_slice(&(-100.0f64).to_le_bytes());
    b2.extend_from_slice(&[0x66, 0x1b, 0x22, 6]);
    b2.push(0x44);
    b2.extend_from_slice(&100.0f64.to_le_bytes());
    b2.extend_from_slice(&[0x20, 6, 0x44]);
    b2.extend_from_slice(&100.0f64.to_le_bytes());
    b2.extend_from_slice(&[0x65, 0x1b, 0x21, 6]);
    // acc_x += x', acc_y += y', store x', y' back
    b2.extend_from_slice(&[
        0x20, 3, 0x20, 5, 0xa0, 0x21, 3, 0x20, 4, 0x20, 6, 0xa0, 0x21, 4, 0x20, 2, 0x20, 5, 0x39,
        3, 0, 0x20, 2, 0x20, 6, 0x39, 3, 8, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b,
        0x20, 3, 0xbd, 0x20, 4, 0xbd, 0x85, // i64.reinterpret(ax) ^ i64.reinterpret(ay)
    ]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7f], &[0x7e])]),
        func_sec(&[0, 0, 1]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("bytesum", 0, 0),
            ("stencil", 0, 1),
            ("slp_f64", 0, 2),
        ]),
        code_sec(&[
            enc_body(&[(2, 0x7f)], &b0),
            enc_body(&[(3, 0x7f)], &b1),
            enc_body(&[(2, 0x7f), (6, 0x7c)], &b2),
        ]),
        section(11, &data_sec),
    ]);

    let calls = vec![
        c("bytesum", ArgSpec::I32(0)),
        c("bytesum", ArgSpec::I32(1)),
        c("bytesum", ArgSpec::I32(15)),
        c("bytesum", ArgSpec::I32(16)),
        c("bytesum", ArgSpec::I32(17)),
        c("bytesum", ArgSpec::I32(31)),
        c("bytesum", ArgSpec::I32(32)),
        c("bytesum", ArgSpec::I32(33)),
        c("bytesum", ArgSpec::I32(64)),
        c("bytesum", ArgSpec::I32(255)),
        c("stencil", ArgSpec::I32(0)),
        c("stencil", ArgSpec::I32(1)),
        c("stencil", ArgSpec::I32(2)),
        c("stencil", ArgSpec::I32(3)),
        c("stencil", ArgSpec::I32(4)),
        c("stencil", ArgSpec::I32(17)),
        c("stencil", ArgSpec::I32(64)),
        c("slp_f64", ArgSpec::I32(0)),
        c("slp_f64", ArgSpec::I32(1)),
        c("slp_f64", ArgSpec::I32(2)),
        c("slp_f64", ArgSpec::I32(3)),
        c("slp_f64", ArgSpec::I32(4)),
    ];
    assert_oracle_matrix(
        "hill3_autovec",
        &wasm,
        &calls,
        &[],
        &[(256, 320), (1024, 1152)],
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_step3_avx2_blur3_magic_hoist_and_f64x4() {
    // Seed 0..256 with bytes including [255, 255, 255] extremes, plus f64 test vectors at
    // 4096 (vals_a), 4128 (vals_b), and 4224 (vals_splat) via active data segments so
    // store-to-load forwarding does not constant-fold away the memory loads we want SLP-fused!
    let mut seed = vec![0u8; 256];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = if i < 6 {
            255
        } else {
            ((i.wrapping_mul(37).wrapping_add(11)) & 0xff) as u8
        };
    }
    let vals_a: [f64; 4] = [1.5, -0.0, f64::INFINITY, f64::NAN];
    let vals_b: [f64; 4] = [2.25, 0.0, -3.5, 4.0];
    let vals_splat: [f64; 4] = [1.25, -2.5, 0.0, -0.0];
    let mut f64_seg = Vec::new();
    for v in vals_a.iter().chain(vals_b.iter()) {
        f64_seg.extend_from_slice(&v.to_le_bytes());
    }
    let mut splat_seg = Vec::new();
    for v in &vals_splat {
        splat_seg.extend_from_slice(&v.to_le_bytes());
    }
    let mut data_sec = vec![3]; // 3 active segments
    // seg 0: @0
    data_sec.extend_from_slice(&[0, 0x41, 0, 0x0b]);
    leb_u32(&mut data_sec, seed.len() as u32);
    data_sec.extend_from_slice(&seed);
    // seg 1: @4096 (vals_a @4096, vals_b @4128)
    data_sec.extend_from_slice(&[0, 0x41]);
    leb_i32(&mut data_sec, 4096);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, f64_seg.len() as u32);
    data_sec.extend_from_slice(&f64_seg);
    // seg 2: @4224 (vals_splat @4224)
    data_sec.extend_from_slice(&[0, 0x41]);
    leb_i32(&mut data_sec, 4224);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, splat_seg.len() as u32);
    data_sec.extend_from_slice(&splat_seg);

    // Helper: emit a boundary-clamped 3-tap stencil loop `[0..n)` writing to `dst_off`
    // with `(b0 + b1 + b2)` divided by `div_op` (`0x6e` DivU / `0x76` ShrU) and `div_imm`,
    // then returning a rolling hash of `dst[0..n)`.
    let make_blur3 = |dst_off: u32, div_op: u8, div_imm: i32| -> Vec<u8> {
        let mut b = vec![
            0x41, 0, 0x21, 1, // i = 0
            0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, // if i >=u n break
            // left = select(i - 1, 0, i)  (direct Op::Select on iv!)
            0x20, 1, 0x41, 1, 0x6b, 0x41, 0, 0x20, 1, 0x1b, 0x21, 2,
            // right = select(n - 1, i + 1, i + 1 >=u n)
            0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x41, 1, 0x6a, 0x20, 1, 0x41, 1, 0x6a, 0x20, 0, 0x4f,
            0x1b, 0x21, 3,
            // dst[i + dst_off] = (src[left] + src[i] + src[right]) <div_op> <div_imm>
            0x20, 1, 0x20, 2, 0x2d, 0, 0, 0x20, 1, 0x2d, 0, 0, 0x6a, 0x20, 3, 0x2d, 0, 0, 0x6a,
            0x41,
        ];
        leb_i32(&mut b, div_imm);
        b.extend_from_slice(&[div_op, 0x3a, 0]);
        leb_u32(&mut b, dst_off);
        b.extend_from_slice(&[
            0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b,
            // rolling hash h = h * 31 + dst[i + dst_off]
            0x41, 0, 0x21, 1, 0x41, 0, 0x21, 2, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f,
            0x0d, 1, 0x20, 2, 0x41, 31, 0x6c, 0x20, 1, 0x2d, 0,
        ]);
        leb_u32(&mut b, dst_off);
        b.extend_from_slice(&[
            0x6a, 0x21, 2, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
        ]);
        b
    };

    let b_blur3_d3 = make_blur3(512, 0x6e, 3); // triggers MemByteBlur3Div (d=3)
    let b_blur3_shr2 = make_blur3(1024, 0x76, 2); // triggers MemByteBlur3Div (pow2 d=4 via ShrU 2)
    let b_blur3_d2 = make_blur3(1536, 0x6e, 2); // MUST stay scalar (765/2 = 382 -> 126, NOT 255!)
    let b_blur3_overlap = make_blur3(10, 0x6e, 3); // overlapping dst=10 (disjoint for n<=10, scalar for n>10)

    // f4(n: i32) -> i32: loop exercising hoisted magic DivU & RemU for d = 3, 5, 7, 10, 13
    // across edge values (0, 1, d-1, d, 0x7fffffff, 0x80000000, 0xffffffff).
    let mut b_magic = vec![
        0x41, 0, 0x21, 1, // i = 0
        0x41, 0, 0x21, 2, // acc = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, // if i >=u n break
        // x = i * 0x1234567 + 0x7ffffffc (hits wrap around 0x80000000 and 0xffffffff!)
        0x20, 1, 0x41,
    ];
    leb_i32(&mut b_magic, 0x0123_4567);
    b_magic.extend_from_slice(&[0x6c, 0x41]);
    leb_i32(&mut b_magic, 0x7fff_fffc);
    b_magic.extend_from_slice(&[0x6a, 0x21, 3]); // local 3 = x
    for &(op, d) in &[
        (0x6e, 3),
        (0x70, 3),
        (0x6e, 5),
        (0x70, 5),
        (0x6e, 7),
        (0x70, 7),
        (0x6e, 10),
        (0x70, 10),
        (0x6e, 13),
        (0x70, 13),
    ] {
        b_magic.extend_from_slice(&[0x20, 2, 0x41, 31, 0x6c, 0x20, 3, 0x41]);
        leb_i32(&mut b_magic, d);
        b_magic.extend_from_slice(&[op, 0x73, 0x21, 2]);
    }
    b_magic.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ]);

    // f5(mode: i32) -> i64: exercises 256-bit `Op::F64x4BinMem` (memory rhs, both overlapping
    // `dst = 4104 = lhs + 8` when mode==0 and disjoint `dst = 4168` when mode==1, with NaN/-0.0/+0.0/+Inf)
    // and returns XOR of 4 result f64 bits.
    let mut b_f64x4_mem = Vec::new();
    // `dst = ((mode & 1) << 6) + 4104`: proven in-bounds (`<= 4168 + 32 <= 65536`) by dominator BCE!
    b_f64x4_mem.extend_from_slice(&[0x20, 0, 0x41, 1, 0x71, 0x41, 6, 0x74, 0x41]);
    leb_i32(&mut b_f64x4_mem, 4104);
    b_f64x4_mem.extend_from_slice(&[0x6a, 0x21, 1]); // local 1 = dst
    // Load all 4 from A (local 10 = 4096) and all 4 from B (local 11 = 4128), add, store 4 to dst.
    b_f64x4_mem.push(0x41);
    leb_i32(&mut b_f64x4_mem, 4096);
    b_f64x4_mem.extend_from_slice(&[0x21, 10, 0x41]);
    leb_i32(&mut b_f64x4_mem, 4128);
    b_f64x4_mem.extend_from_slice(&[0x21, 11]);
    for k in 0..4usize {
        b_f64x4_mem.extend_from_slice(&[0x20, 10, 0x2b, 3, (k * 8) as u8, 0x21, (2 + k) as u8]);
    }
    for k in 0..4usize {
        b_f64x4_mem.extend_from_slice(&[0x20, 11, 0x2b, 3, (k * 8) as u8, 0x21, (6 + k) as u8]);
    }
    for k in 0..4usize {
        b_f64x4_mem.extend_from_slice(&[
            0x20,
            1,
            0x20,
            (2 + k) as u8,
            0x20,
            (6 + k) as u8,
            0xa0, // f64.add
            0x39,
            3,
            (k * 8) as u8,
        ]);
    }
    // Return XOR of all 4 stored f64 bit patterns.
    b_f64x4_mem.extend_from_slice(&[0x20, 1, 0x29, 3, 0]);
    for k in 1..4usize {
        b_f64x4_mem.extend_from_slice(&[0x20, 1, 0x29, 3, (k * 8) as u8, 0x85]);
    }

    // f6(scale: f64) -> i64: `scale` enters in `Xmm0`! In-place interleaved 4-wide `a[0..4] *= scale`,
    // exercising `Op::F64x4BinMem { rhs_is_splat: true }` and verifying `Xmm0` preservation on SSE2!
    let mut b_f64x4_splat = Vec::new();
    // Interleaved in-place `a[k] = a[k] * scale` (k = 0..4) using local 1 = base (4224).
    b_f64x4_splat.push(0x41);
    leb_i32(&mut b_f64x4_splat, 4224);
    b_f64x4_splat.extend_from_slice(&[0x21, 1]);
    for k in 0..4usize {
        b_f64x4_splat.extend_from_slice(&[
            0x20,
            1,
            0x20,
            1,
            0x2b,
            3,
            (k * 8) as u8,
            0x20,
            0,    // scale (in Xmm0!)
            0xa2, // f64.mul
            0x39,
            3,
            (k * 8) as u8,
        ]);
    }
    // Also use `scale` AFTER the F64x4BinMem to verify Xmm0 was restored intact!
    b_f64x4_splat.extend_from_slice(&[0x20, 0, 0xbd]); // i64.reinterpret(scale)
    for k in 0..4usize {
        b_f64x4_splat.extend_from_slice(&[0x20, 1, 0x29, 3, (k * 8) as u8, 0x85]);
    }

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7f], &[0x7e]), (&[0x7c], &[0x7e])]),
        func_sec(&[0, 0, 0, 0, 0, 1, 2]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("blur3_d3", 0, 0),
            ("blur3_shr2", 0, 1),
            ("blur3_d2", 0, 2),
            ("blur3_overlap", 0, 3),
            ("magic_div_loop", 0, 4),
            ("f64x4_mem", 0, 5),
            ("f64x4_splat", 0, 6),
        ]),
        code_sec(&[
            enc_body(&[(3, 0x7f)], &b_blur3_d3),
            enc_body(&[(3, 0x7f)], &b_blur3_shr2),
            enc_body(&[(3, 0x7f)], &b_blur3_d2),
            enc_body(&[(3, 0x7f)], &b_blur3_overlap),
            enc_body(&[(3, 0x7f)], &b_magic),
            enc_body(&[(1, 0x7f), (8, 0x7c), (2, 0x7f)], &b_f64x4_mem),
            enc_body(&[(1, 0x7f)], &b_f64x4_splat),
        ]),
        section(11, &data_sec),
    ]);

    // Verify IR passes actually emitted `MemByteBlur3Div` (for d=3 & shr2, NOT d=2) and `F64x4BinMem`!
    {
        let validated = wtime_validate::validate(
            &wasm,
            &super::module::x64_validate_features(),
            &super::module::x64_validate_limits(),
        )
        .unwrap();
        let decoded = validated.decoded();
        let module = super::module::admit_x64_module(decoded).unwrap();
        let mut irs = Vec::new();
        for di in 0..decoded.code.len() {
            irs.push(super::ir::lower_wasm_to_ir(decoded, &module, di).unwrap());
        }
        let effects = super::opt::compute_module_effects(&irs, &module, None).unwrap();
        for ir in &mut irs {
            super::opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
            super::verify::verify_ir(ir, &module).unwrap();
        }
        let has_blur = |fi: usize| {
            irs[fi].blocks.iter().any(|bb| {
                bb.insts
                    .iter()
                    .any(|ins| matches!(ins.op, super::ir::Op::MemByteBlur3Div { .. }))
            })
        };
        let has_f64x4 = |fi: usize| {
            irs[fi].blocks.iter().any(|bb| {
                bb.insts
                    .iter()
                    .any(|ins| matches!(ins.op, super::ir::Op::F64x4BinMem { .. }))
            })
        };
        assert!(has_blur(0), "blur3_d3 should emit Op::MemByteBlur3Div");
        assert!(has_blur(1), "blur3_shr2 should emit Op::MemByteBlur3Div");
        assert!(
            !has_blur(2),
            "blur3_d2 must NOT emit Op::MemByteBlur3Div (saturation hazard)"
        );
        let fmt_ops = |fi: usize| {
            irs[fi]
                .blocks
                .iter()
                .map(|b| b.insts.iter().map(|i| (i.res, i.op)).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert!(
            has_f64x4(5),
            "f64x4_mem should emit Op::F64x4BinMem; ir={:?}",
            fmt_ops(5)
        );
        assert!(
            has_f64x4(6),
            "f64x4_splat should emit Op::F64x4BinMem; ir={:?}",
            fmt_ops(6)
        );
    }

    let calls = vec![
        c("blur3_d3", ArgSpec::I32(0)),
        c("blur3_d3", ArgSpec::I32(1)),
        c("blur3_d3", ArgSpec::I32(2)),
        c("blur3_d3", ArgSpec::I32(3)),
        c("blur3_d3", ArgSpec::I32(16)),
        c("blur3_d3", ArgSpec::I32(17)),
        c("blur3_d3", ArgSpec::I32(18)),
        c("blur3_d3", ArgSpec::I32(31)),
        c("blur3_d3", ArgSpec::I32(32)),
        c("blur3_d3", ArgSpec::I32(67)),
        c("blur3_d3", ArgSpec::I32(130)),
        c("blur3_shr2", ArgSpec::I32(67)),
        c("blur3_d2", ArgSpec::I32(64)),
        c("blur3_overlap", ArgSpec::I32(8)),
        c("blur3_overlap", ArgSpec::I32(40)),
        c("magic_div_loop", ArgSpec::I32(0)),
        c("magic_div_loop", ArgSpec::I32(64)),
        c("f64x4_mem", ArgSpec::I32(0)),
        c("f64x4_mem", ArgSpec::I32(1)),
        c("f64x4_splat", ArgSpec::F64((-3.5f64).to_bits())),
        c("f64x4_splat", ArgSpec::F64((-0.0f64).to_bits())),
    ];
    assert_oracle_matrix(
        "step3_avx2_blur3_magic_f64x4",
        &wasm,
        &calls,
        &[],
        &[(512, 130), (1024, 67), (1536, 64), (4096, 160)],
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage6_narrow_eq_loops_mul_strength_and_upper32_zero() {
    // 1. `narrow_equality_filtered_loops` exhaustive edge-case matrix:
    //    - f0(packed): 1-key loop `for v in init..limit { if v == k1 { acc = acc * 31 + v + 7; mem[64 + v] = v + 1 } }`
    //      where init = packed & 0xff, limit = (packed >> 8) & 0xff, k1 = (packed >> 16) & 0xff.
    //    - f1(packed): 2-key loop `for v in init..limit { if v == k1 || v == k2 { acc = acc * 131 + v + 3; mem[320 + cnt] = v; cnt += 1 } }`
    //      where init = packed & 0xf, limit = (packed >> 4) & 0xf, k1 = (packed >> 8) & 0xf, k2 = (packed >> 12) & 0xf.
    //      Strictly tests ascending visit order (`k1 > k2`), deduplication (`k1 == k2`), out-of-range keys (`< init` or `>= limit`),
    //      empty range (`limit <= init`), and 1-element range (`limit == init + 1`)!
    //    - f2(packed): `do-while` loop (NO entry guard!) `v = init; do { if v == k1 { acc += v + 100 }; v += 1 } while v <u limit`
    //      with `init >= limit` (must STILL execute iteration `v = init` when `k1 == init`, MUST NOT be mis-narrowed!).
    //    - f3(packed): side-effecting + trapping 2-key loop writing `mem[65530 + v] = 0x55`:
    //      when `k1 = 8` (OOB!) and `k2 = 2` (in bounds!), ascending order MUST write `mem[65532] = 0x55` first then trap on `v = 8`;
    //      when `k1 = 2` and `k2 = 8`, same exact partial write + OOB trap!
    // 2. Strength-reduced constant multiplies (`emit_mul_imm`) for all 24 small constants in both i32 and i64.
    // 3. `val_upper32_zero` elision safety on `i32.wrap_i64` / `i64.extend_i32_u` with both clean & dirty upper 32 bits.

    // f0(packed: i32) -> i32: 1-key filtered loop
    let mut b0 = Vec::new();
    // l1=init, l2=limit, l3=k1, l4=v, l5=acc
    b0.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b0, 0xff);
    b0.extend_from_slice(&[0x71, 0x22, 1, 0x21, 4]); // init = v = packed & 0xff
    b0.extend_from_slice(&[0x20, 0, 0x41, 8, 0x76, 0x41]);
    leb_i32(&mut b0, 0xff);
    b0.extend_from_slice(&[0x71, 0x21, 2]); // limit
    b0.extend_from_slice(&[0x20, 0, 0x41, 16, 0x76, 0x41]);
    leb_i32(&mut b0, 0xff);
    b0.extend_from_slice(&[0x71, 0x21, 3]); // k1
    b0.extend_from_slice(&[0x41, 1, 0x21, 5]); // acc = 1
    b0.extend_from_slice(&[
        0x02, 0x40, 0x03, 0x40, 0x20, 4, 0x20, 2, 0x4f, 0x0d, 1, // if v >=u limit break
        0x20, 4, 0x20, 3, 0x46, 0x04, 0x40, // if v == k1
        0x20, 5, 0x41, 31, 0x6c, 0x20, 4, 0x6a, 0x41, 7, 0x6a, 0x21,
        5, // acc = acc*31 + v + 7
        0x20, 4, 0x41, 64, 0x6a, 0x20, 4, 0x41, 1, 0x6a, 0x3a, 0, 0,    // mem[64+v] = v+1
        0x0b, // end if
        0x20, 4, 0x41, 1, 0x6a, 0x21, 4, 0x0c, 0, 0x0b, 0x0b, 0x20, 5,
    ]);

    // f1(packed: i32) -> i32: 2-key filtered loop
    let mut b1 = Vec::new();
    // l1=init, l2=limit, l3=k1, l4=k2, l5=v, l6=acc, l7=cnt
    b1.extend_from_slice(&[
        0x20, 0, 0x41, 15, 0x71, 0x22, 1, 0x21, 5, // init = v
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // limit
        0x20, 0, 0x41, 8, 0x76, 0x41, 15, 0x71, 0x21, 3, // k1
        0x20, 0, 0x41, 12, 0x76, 0x41, 15, 0x71, 0x21, 4, // k2
        0x41, 5, 0x21, 6, // acc = 5
        0x41, 0, 0x21, 7, // cnt = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 5, 0x20, 2, 0x4f, 0x0d, 1, // if v >=u limit break
        0x20, 5, 0x20, 3, 0x46, 0x20, 5, 0x20, 4, 0x46, 0x72, // (v == k1) | (v == k2)
        0x04, 0x40, // if
        0x20, 6, 0x41,
    ]);
    leb_i32(&mut b1, 131);
    b1.extend_from_slice(&[0x6c, 0x20, 5, 0x6a, 0x41, 3, 0x6a, 0x21, 6]); // acc = acc*131 + v + 3
    b1.extend_from_slice(&[0x20, 7, 0x41]);
    leb_i32(&mut b1, 320);
    b1.extend_from_slice(&[0x6a, 0x20, 5, 0x3a, 0, 0]); // mem[320 + cnt] = v
    b1.extend_from_slice(&[
        0x20, 7, 0x41, 1, 0x6a, 0x21, 7,    // cnt += 1
        0x0b, // end if
        0x20, 5, 0x41, 1, 0x6a, 0x21, 5, 0x0c, 0, 0x0b, 0x0b, 0x20, 6, 0x20, 7, 0x41, 24, 0x74,
        0x73,
    ]);

    // f2(packed: i32) -> i32: unguarded do-while loop (init = packed & 0xf, limit = (packed>>4)&0xf, k1 = (packed>>8)&0xf)
    let b2 = vec![
        0x20, 0, 0x41, 15, 0x71, 0x21, 1, // v = init
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // limit
        0x20, 0, 0x41, 8, 0x76, 0x41, 15, 0x71, 0x21, 3, // k1
        0x41, 0, 0x21, 4, // acc = 0
        0x03, 0x40, // loop (do-while: body runs BEFORE check!)
        0x20, 1, 0x20, 3, 0x46, 0x04, 0x40, 0x20, 4, 0x20, 1, 0x6a, 0x41, 0xe4, 0x00, 0x6a, 0x21,
        4, // acc += v + 100
        0x0b, 0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x20, 2, 0x49, 0x0d, 0, // br_if (v <u limit) 0
        0x0b, 0x20, 4,
    ];

    // f3(packed: i32) -> i32: 2-key loop writing mem[65530 + v] = v + 0x50 (traps OOB for v >= 6!)
    // k1 = packed & 0xf, k2 = (packed >> 4) & 0xf, limit = 12
    let mut b3 = vec![
        0x20, 0, 0x41, 15, 0x71, 0x21, 1, // k1
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // k2
        0x41, 0, 0x21, 3, // v = 0
        0x41, 0, 0x21, 4, // acc = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 3, 0x41, 12, 0x4f, 0x0d, 1, // if v >=u 12 break
        0x20, 3, 0x20, 1, 0x46, 0x20, 3, 0x20, 2, 0x46, 0x72, // v == k1 || v == k2
        0x04, 0x40, 0x20, 3, 0x41,
    ];
    leb_i32(&mut b3, 65530);
    b3.extend_from_slice(&[0x6a, 0x20, 3, 0x41]);
    leb_i32(&mut b3, 0x50);
    b3.extend_from_slice(&[0x6a, 0x3a, 0, 0]); // mem[65530 + v] = v + 0x50
    b3.extend_from_slice(&[
        0x20, 4, 0x41, 17, 0x6c, 0x20, 3, 0x6a, 0x21, 4, 0x0b, 0x20, 3, 0x41, 1, 0x6a, 0x21, 3,
        0x0c, 0, 0x0b, 0x0b, 0x20, 4,
    ]);

    // f4(x: i32) -> i32: strength-reduced constant i32 multiplies across all 24 small multipliers
    // (both `dst == src` chain and `dst != src` from live `x`)
    let mults = [
        3i32, 5, 6, 7, 9, 10, 12, 15, 17, 18, 20, 24, 25, 27, 31, 33, 36, 40, 45, 48, 63, 65, 72,
        81,
    ];
    let mut b4 = vec![0x20, 0, 0x21, 1]; // acc = x
    for &m in &mults {
        // acc = (acc * m) ^ (x * m)
        b4.extend_from_slice(&[0x20, 1, 0x41]);
        leb_i32(&mut b4, m);
        b4.extend_from_slice(&[0x6c, 0x20, 0, 0x41]);
        leb_i32(&mut b4, m);
        b4.extend_from_slice(&[0x6c, 0x73, 0x21, 1]);
    }
    b4.extend_from_slice(&[0x20, 1]);

    // f5(x: i64) -> i64: strength-reduced constant i64 multiplies + `val_upper32_zero` wrap/extend checks!
    // Tests:
    //  a) all 24 small i64 multipliers with wraparound;
    //  b) `i64.extend_i32_u(i32.wrap_i64(x))` where `x` has dirty high 32 bits (MUST zero upper 32 bits!);
    //  c) `i64.extend_i32_u(i32.wrap_i64(x >>u 32))` where input IS already upper32_zero (elidable!);
    //  d) `i64.extend_i32_u(i32.load[0])` + `i32.wrap_i64(i64.extend_i32_u(y))` used as address.
    let mut b5 = vec![0x20, 0, 0x21, 1]; // acc = x
    for &m in &mults {
        b5.extend_from_slice(&[0x20, 1, 0x42]);
        leb_i32(&mut b5, m);
        b5.extend_from_slice(&[0x7e, 0x20, 0, 0x42]);
        leb_i32(&mut b5, m);
        b5.extend_from_slice(&[0x7e, 0x85, 0x21, 1]);
    }
    // b) dirty high bits wrap -> extend_u (must clear high 32 bits!)
    b5.extend_from_slice(&[0x20, 1, 0x20, 0, 0xa7, 0xad, 0x85, 0x21, 1]);
    // c) clean upper32_zero (`x >>u 32`) wrap -> extend_u
    b5.extend_from_slice(&[0x20, 1, 0x20, 0, 0x42, 32, 0x88, 0xa7, 0xad, 0x7c, 0x21, 1]);
    // d) negative i32 sign-extended to i64 (`0xffff_ffff_8000_0001`), wrapped then extended_u (MUST NOT elide!)
    b5.extend_from_slice(&[
        0x20, 1, 0x20, 0, 0xa7,
        0xac, // i64.extend_i32_s -> dirty upper 32 bits if bit 31 set!
        0xa7, // i32.wrap_i64
        0xad, // i64.extend_i32_u -> upper 32 bits MUST be 0!
        0x85, 0x21, 1, 0x20, 1,
    ]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e])]),
        func_sec(&[0, 0, 0, 0, 0, 1]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("narrow_1k", 0, 0),
            ("narrow_2k", 0, 1),
            ("dowhile_1k", 0, 2),
            ("narrow_2k_trap", 0, 3),
            ("mul_i32", 0, 4),
            ("mul_and_u32z_i64", 0, 5),
        ]),
        code_sec(&[
            enc_body(&[(5, 0x7f)], &b0),
            enc_body(&[(7, 0x7f)], &b1),
            enc_body(&[(4, 0x7f)], &b2),
            enc_body(&[(4, 0x7f)], &b3),
            enc_body(&[(1, 0x7f)], &b4),
            enc_body(&[(1, 0x7e)], &b5),
        ]),
    ]);

    let mut calls = Vec::new();
    // f0: (init, limit, k1)
    for (init, limit, k1) in [
        (0, 0, 0),
        (0, 1, 0),
        (0, 1, 1),
        (0, 10, 0),
        (0, 10, 5),
        (0, 10, 9),
        (0, 10, 10),
        (5, 12, 4),
        (5, 12, 5),
        (5, 12, 8),
        (5, 12, 11),
        (5, 12, 12),
        (10, 5, 10),
    ] {
        calls.push(c(
            "narrow_1k",
            ArgSpec::I32(init | (limit << 8) | (k1 << 16)),
        ));
    }
    // f1: (init, limit, k1, k2)
    for (init, limit, k1, k2) in [
        (0, 0, 0, 0),
        (0, 1, 0, 0),
        (0, 1, 0, 1),
        (0, 10, 2, 7),  // k1 < k2
        (0, 10, 7, 2),  // k1 > k2 (ascending order!)
        (0, 10, 5, 5),  // k1 == k2 (dedup!)
        (3, 10, 1, 8),  // k1 < init, k2 in range
        (3, 10, 8, 1),  // k1 in range, k2 < init
        (3, 10, 4, 12), // k1 in range, k2 >= limit
        (3, 10, 12, 4), // k1 >= limit, k2 in range
        (3, 10, 1, 12), // both out of range
        (3, 10, 3, 9),  // both exact boundaries
        (3, 10, 9, 3),  // reversed exact boundaries
        (8, 3, 8, 9),   // init > limit -> 0 iters
    ] {
        calls.push(c(
            "narrow_2k",
            ArgSpec::I32(init | (limit << 4) | (k1 << 8) | (k2 << 12)),
        ));
    }
    // f2: unguarded do-while with init >= limit AND init < limit
    for (init, limit, k1) in [
        (5, 0, 5),
        (5, 3, 5),
        (5, 5, 5),
        (5, 5, 4),
        (2, 7, 4),
        (2, 7, 2),
    ] {
        calls.push(c(
            "dowhile_1k",
            ArgSpec::I32(init | (limit << 4) | (k1 << 8)),
        ));
    }
    // f3: trapping 2-key ordering (`k1 > k2` where larger traps -> smaller MUST commit first!)
    for (k1, k2) in [(1, 4), (4, 1), (3, 3), (8, 2), (2, 8), (9, 7)] {
        calls.push(c("narrow_2k_trap", ArgSpec::I32(k1 | (k2 << 4))));
    }
    // f4: i32 multiplies
    for x in [0, 1, -1, 7, -13, 0x1234_5678, i32::MIN, i32::MAX] {
        calls.push(c("mul_i32", ArgSpec::I32(x)));
    }
    // f5: i64 multiplies + upper32_zero wrap/extend
    for x in [
        0i64,
        1,
        -1,
        0x1234_5678_9abc_def0,
        (1i64 << 32) | 0x8000_0001,
        i64::MIN,
        i64::MAX,
    ] {
        calls.push(c("mul_and_u32z_i64", ArgSpec::I64(x)));
    }
    assert_oracle_matrix(
        "stage6_narrow_mul_u32z",
        &wasm,
        &calls,
        &[],
        &[(64, 32), (320, 16), (65530, 6)],
    );

    // Also verify `fixtures/graph_bfs.wasm` end-to-end against `wtime_runtime` interpreter on
    // BOTH cold initial memory (full BFS) and repeated calls!
    let bfs_wasm = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/graph_bfs.wasm"),
    )
    .expect("read fixtures/graph_bfs.wasm");
    let bfs_calls = vec![
        c("run", ArgSpec::I32(0)),
        c("run", ArgSpec::I32(1)),
        c("run", ArgSpec::I32(16)),
        c("run", ArgSpec::I32(64)),
        c("run", ArgSpec::I32(128)),
        c("run", ArgSpec::I32(1024)),
        c("run", ArgSpec::I32(1024)),
    ];
    assert_oracle_matrix("stage6_graph_bfs_fixture", &bfs_wasm, &bfs_calls, &[], &[]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage7_relaxed_simd_tail_call_axpy_mem64_and_fixtures() {
    use super::tests_unit::{
        build_axpy_edge_test_wasm, build_mem64_1branch_test_wasm, build_relaxed_simd_test_wasm,
        build_tail_call_test_wasm,
    };

    // 1. ALL 20 relaxed SIMD opcodes (`0xfd 0x100..=0x113`) bit-exact across all 4 cells.
    let relaxed_wasm = build_relaxed_simd_test_wasm();
    assert_oracle_matrix(
        "stage7_relaxed_simd_all20",
        &relaxed_wasm,
        &[
            c("test_relaxed", ArgSpec::I32(0)),
            c("test_relaxed", ArgSpec::I32(1)),
            c("test_relaxed", ArgSpec::I32(2)),
        ],
        &[],
        &[(64, 320)],
    );

    // 2. `tail_call` (`return_call` + `return_call_indirect`): deep O(1) mutual recursion (200k
    //    steps proves constant stack space vs 8 MiB guest stack!), high-arity stack args (same
    //    and growing), overflow multi-value return buffers, and indirect traps.
    let tail_wasm = build_tail_call_test_wasm();
    assert_oracle_matrix(
        "stage7_tail_call",
        &tail_wasm,
        &[
            c("even", ArgSpec::I32(0)),
            c("even", ArgSpec::I32(1)),
            c("even", ArgSpec::I32(200_000)),
            c("even", ArgSpec::I32(200_001)),
            c("even_ind", ArgSpec::I32(0)),
            c("even_ind", ArgSpec::I32(50_000)),
            c("even_ind", ArgSpec::I32(50_001)),
            c("grow_arity", ArgSpec::I32(0)),
            c("grow_arity", ArgSpec::I32(5)),
            c("grow_arity", ArgSpec::I32(20_000)),
            c("check_multiret", ArgSpec::I64(100)),
            c("check_multiret", ArgSpec::I64(-42)),
            c("grow_multiret", ArgSpec::I32(0)),
            c("grow_multiret", ArgSpec::I32(10_000)),
            c("ind_trap", ArgSpec::I32(5)),  // null slot
            c("ind_trap", ArgSpec::I32(4)),  // bad signature
            c("ind_trap", ArgSpec::I32(99)), // table OOB
        ],
        &[],
        &[],
    );

    // 3. `Op::F64AxpyLoop` edge cases (add/sub, counts 0..17, dst==src, forward/backward overlap, OOB partial store trap).
    let axpy_wasm = build_axpy_edge_test_wasm();
    let mut axpy_calls: Vec<OracleCall> = (0..=10)
        .chain([20, 21, 22, 23, 99])
        .map(|k| c("run_case", ArgSpec::I32(k)))
        .collect();
    assert_oracle_matrix(
        "stage7_f64_axpy_edges",
        &axpy_wasm,
        &axpy_calls,
        &[],
        &[(0, 512), (65520, 16)],
    );
    axpy_calls.clear();

    // 4. Growable `memory64` 1-branch bounds checks (`mem_min_bytes >= end_off`) + 2-branch fallback + wrap traps.
    let mem64_wasm = build_mem64_1branch_test_wasm();
    assert_oracle_matrix(
        "stage7_mem64_1branch",
        &mem64_wasm,
        &[
            c("load_off0", ArgSpec::I64(0)),
            c("load_off0", ArgSpec::I64(65528)),
            c("load_off0", ArgSpec::I64(65529)),
            c("load_off0", ArgSpec::I64(-1)),
            c("store_load_off8", ArgSpec::I64(0)),
            c("store_load_off8", ArgSpec::I64(65520)),
            c("store_load_off8", ArgSpec::I64(65521)),
            c("store_load_off8", ArgSpec::I64(-8)),
            c("load_off_big", ArgSpec::I64(0)),
            c("grow_store_load_off8", ArgSpec::I64(70000)),
            c("grow_store_load_off8", ArgSpec::I64(131056)),
            c("load_off_big", ArgSpec::I64(0)),
            c("load_off_big", ArgSpec::I64(-1)),
        ],
        &[],
        &[(0, 64)],
    );

    // 5. New execution benchmark fixtures (`matmul_f64.wasm`, `heapsort_i32.wasm`) bit-exact parity.
    for (name, arg) in [("matmul_f64", 32), ("heapsort_i32", 256)] {
        let wasm = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("fixtures/{name}.wasm")),
        )
        .unwrap_or_else(|e| panic!("read fixtures/{name}.wasm: {e}"));
        assert_oracle_matrix(
            &format!("stage7_fixture_{name}"),
            &wasm,
            &[
                c("run", ArgSpec::I32(0)),
                c("run", ArgSpec::I32(1)),
                c("run", ArgSpec::I32(arg)),
            ],
            &[],
            &[],
        );
    }
}

/// Shared Wasm 3.0 `wide-arithmetic` (`0xfc13..=0xfc16`) test module for x64 and AArch64 oracles.
pub(crate) fn build_wide_arithmetic_test_wasm() -> Vec<u8> {
    let mut b_add = Vec::new();
    // Mixes multiple i64.add128 cases: (arg, 0) + (arg, 1), (-1, arg) + (1, -1), (u64::MAX, -1) + (arg, 0)
    // local 0: arg, local 1: lo, local 2: hi, local 3: acc
    // Case 1: dynamic carry
    b_add.extend_from_slice(&[
        0x20, 0, // a_lo = arg
        0x20, 0, 0x42, 7, 0x7c, // a_hi = arg + 7
        0x42, 0x7f, // b_lo = -1 (0xffff...)
        0x42, 1, // b_hi = 1
        0xfc, 0x13, // i64.add128 -> (lo, hi)
        0x21, 2, 0x21, 1, // hi, lo
        0x20, 1, 0x20, 2, 0x42, 13, 0x89, 0x85, // lo ^ rotl(hi, 13)
        0x21, 3,
    ]);
    // Case 2: constant-folded carry cascade (-1, -1) + (1, 0) = (0, 0)
    b_add.extend_from_slice(&[
        0x42, 0x7f, 0x42, 0x7f, 0x42, 1, 0x42, 0, 0xfc, 0x13, 0x21, 2, 0x21, 1, 0x20, 3, 0x20, 1,
        0x85, 0x20, 2, 0x7c, 0x21, 3,
    ]);
    // Case 3: dynamic (arg, arg ^ -1) + (arg * 3, arg >> 1)
    b_add.extend_from_slice(&[
        0x20, 0, 0x20, 0, 0x42, 0x7f, 0x85, 0x20, 0, 0x42, 3, 0x7e, 0x20, 0, 0x42, 1, 0x87, 0xfc,
        0x13, 0x21, 2, 0x21, 1, 0x20, 3, 0x20, 1, 0x7c, 0x20, 2, 0x42, 7, 0x89, 0x85,
    ]);

    let mut b_sub = Vec::new();
    // Case 1: dynamic borrow (arg, 0) - (-1, 1)
    b_sub.extend_from_slice(&[
        0x20, 0, 0x42, 0, 0x42, 0x7f, 0x42, 1, 0xfc, 0x14, 0x21, 2, 0x21, 1, 0x20, 1, 0x20, 2,
        0x42, 17, 0x89, 0x85, 0x21, 3,
    ]);
    // Case 2: (0, 0) - (1, 0) = (-1, -1)
    b_sub.extend_from_slice(&[
        0x42, 0, 0x42, 0, 0x42, 1, 0x42, 0, 0xfc, 0x14, 0x21, 2, 0x21, 1, 0x20, 3, 0x20, 1, 0x85,
        0x20, 2, 0x7c, 0x21, 3,
    ]);
    // Case 3: (arg, arg) - (arg + 1, 0) -> borrows whenever arg != u64::MAX
    b_sub.extend_from_slice(&[
        0x20, 0, 0x20, 0, 0x20, 0, 0x42, 1, 0x7c, 0x42, 0, 0xfc, 0x14, 0x21, 2, 0x21, 1, 0x20, 3,
        0x20, 1, 0x7c, 0x20, 2, 0x42, 11, 0x89, 0x85,
    ]);

    // Loop stressing RAX/RDX clobber preservation across `i64.mul_wide_s` (0xfc15) and `i64.mul_wide_u` (0xfc16)
    let make_mul_wide = |sub_op: u8| -> Vec<u8> {
        // locals: 0=arg, 1=i(i32), 2=acc(i64), 3=a(i64), 4=b(i64), 5=lo(i64), 6=hi(i64)
        let mut b = vec![
            0x20, 0, 0x21, 3, // a = arg
            0x20, 0, 0x42, 0x7f, 0x85, 0x21, 4, // b = ~arg
            0x41, 0, 0x21, 1, // i = 0
            0x42, 0, 0x21, 2, // acc = 0
            0x03, 0x40, // loop
            0x20, 3, 0x20, 4, 0xfc, sub_op, // mul_wide -> (lo, hi)
            0x21, 6, 0x21, 5, // hi, lo
            0x20, 2, 0x20, 5, 0x7c, 0x20, 6, 0x42, 19, 0x89, 0x85, 0x21,
            2, // acc = (acc+lo) ^ rotl(hi, 19)
            // Keep a and b live across mul_wide and update them
            0x20, 3, 0x42, 0x93, 0x05, 0x7c, 0x21, 3, // a += 659
            0x20, 4, 0x42, 0x7f, 0x7e, 0x42, 5, 0x7c, 0x21, 4, // b = -b + 5
            0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x41, 8, 0x49, 0x0d,
            0,    // i += 1; br_if 0 (i <u 8)
            0x0b, // end loop
            // Also test exact i64::MIN * -1 constant & dynamic path
            0x20, 0, 0x42, 0x7f, 0xfc, sub_op, 0x21, 6, 0x21, 5, 0x20, 2, 0x20, 5, 0x85, 0x20, 6,
            0x42, 5, 0x89, 0x7c,
        ];
        // Constant-fold check: i64::MIN * -1
        b.push(0x42);
        leb_i64(&mut b, i64::MIN);
        b.extend_from_slice(&[0x42, 0x7f, 0xfc, sub_op, 0x21, 6, 0x21, 5]);
        b.extend_from_slice(&[0x20, 5, 0x85, 0x20, 6, 0x7c]);
        b
    };

    wasm_module(&[
        type_sec(&[(&[0x7e], &[0x7e])]),
        func_sec(&[0, 0, 0, 0]),
        export_sec(&[
            ("add128", 0, 0),
            ("sub128", 0, 1),
            ("mul_wide_s", 0, 2),
            ("mul_wide_u", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[(3, 0x7e)], &b_add),
            enc_body(&[(3, 0x7e)], &b_sub),
            enc_body(&[(1, 0x7f), (5, 0x7e)], &make_mul_wide(0x15)),
            enc_body(&[(1, 0x7f), (5, 0x7e)], &make_mul_wide(0x16)),
        ]),
    ])
}

/// Shared Wasm 3.0 `multi-memory` (mem 0: mem32, mem 1: mem32, mem 2: mem64) test module.
pub(crate) fn build_multi_memory_test_wasm() -> Vec<u8> {
    // 3 memories:
    //   mem 0: i32, min=1, max=3 (flags=0x01)
    //   mem 1: i32, min=1, max=3 (flags=0x01)
    //   mem 2: i64, min=1, max=3 (flags=0x05)
    let mem_sec = section(5, &[3, 0x01, 1, 3, 0x01, 1, 3, 0x05, 1, 3]);

    // Active data segments for mem 0 (off 0), mem 1 (off 16), mem 2 (off 32, i64 offset!).
    let mut data_payload = vec![3];
    // seg 0: active mem 0 (flags=0), off=i32(0), "MEM0-INIT-DATA!!"
    let s0 = b"MEM0-INIT-DATA!!";
    data_payload.extend_from_slice(&[0x00, 0x41, 0, 0x0b, s0.len() as u8]);
    data_payload.extend_from_slice(s0);
    // seg 1: active mem 1 (flags=2, mem_idx=1), off=i32(16), "MEM1-HELLO-WORLD"
    let s1 = b"MEM1-HELLO-WORLD";
    data_payload.extend_from_slice(&[0x02, 1, 0x41, 16, 0x0b, s1.len() as u8]);
    data_payload.extend_from_slice(s1);
    // seg 2: active mem 2 (flags=2, mem_idx=2), off=i64(32), "MEM2-64BIT-WASM!"
    let s2 = b"MEM2-64BIT-WASM!";
    data_payload.extend_from_slice(&[0x02, 2, 0x42, 32, 0x0b, s2.len() as u8]);
    data_payload.extend_from_slice(s2);

    // f0: check_init(arg: i32) -> i64
    // Loads i64 from mem0[0], mem1[16], mem2[32] (memarg flags = 0x40 | align, mem_idx, offset)
    let f0 = vec![
        0x41, 0, 0x29, 0x03, 0, // i64.load mem0 off=0
        0x41, 16, 0x29, 0x43, 1, 0, // i64.load mem1 off=0
        0x85, 0x42, 32, 0x29, 0x43, 2, 0, // i64.load mem2 off=0
        0x7c, 0x20, 0, 0xad, 0x85,
    ];

    // f1: rw_and_simd(arg: i32) -> i64
    // Stores i32/i64/v128 and v128.load32_lane / store32_lane on mem 1 and mem 2; checks isolation from mem 0.
    let f1 = vec![
        // mem1[128] = arg (i32); mem1[136] = i64(arg)*7
        0x41, 128, 0x01, 0x20, 0, 0x36, 0x42, 1, 0, // i32.store mem1 addr=128
        0x41, 136, 0x01, 0x20, 0, 0xac, 0x42, 7, 0x7e, 0x37, 0x43, 1,
        0, // i64.store mem1 addr=136
        // mem2[256] = v128.splat(arg)
        0x42, 0x80, 0x02, 0x20, 0, 0xfd, 0x11, 0xfd, 0x0b, 0x40, 2,
        0, // v128.store mem2 addr=256
        // v128.load32_lane lane 2 from mem1[128] into mem2[256], then store32_lane lane 2 into mem2[288]
        0x42, 0x80, 0x02, 0xfd, 0x00, 0x40, 2, 0, // load v128 from mem2[256]
        0x21, 1, // local 1 (v128)
        0x41, 128, 0x01, 0x20, 1, 0xfd, 0x56, 0x42, 1, 0, 2, // v128.load32_lane mem1 lane=2
        0x21, 1, 0x42, 0xa0, 0x02, 0x20, 1, 0xfd, 0x5a, 0x42, 2, 0,
        2, // v128.store32_lane mem2 addr=288 lane=2
        // Read back mem1[136] ^ extend_u(mem2[288]) ^ mem0[0]
        0x41, 136, 0x01, 0x29, 0x43, 1, 0, 0x42, 0xa0, 0x02, 0x28, 0x42, 2, 0, 0xad, 0x85, 0x41, 0,
        0x29, 0x03, 0, 0x85,
    ];

    // f2: grow_and_bulk(arg: i32) -> i64
    // Tests memory.size 1/2, memory.grow 1/2 (ok & fail), memory.fill 1/2, cross-memory copy 1->0, 0->2, 2->1, overlap 1->1
    let f2 = vec![
        // old1 = memory.grow 1 (1) -> 1; size1 = memory.size 1 -> 2; fail1 = memory.grow 1 (5) -> -1
        0x41, 1, 0x40, 1, 0xad, // old1
        0x3f, 1, 0xad, 0x42, 4, 0x86, 0x84, // | (size1 << 4)
        0x41, 5, 0x40, 1, 0x41, 0x7f, 0x46, 0xad, 0x42, 8, 0x86,
        0x84, // | ((fail1 == -1) << 8)
        // old2 = memory.grow 2 (i64 1) -> 1; size2 = memory.size 2 -> 2; fail2 = memory.grow 2 (i64 5) -> -1
        0x42, 1, 0x40, 2, 0x42, 12, 0x86, 0x84, 0x3f, 2, 0x42, 16, 0x86, 0x84, 0x42, 5, 0x40, 2,
        0x42, 0x7f, 0x51, 0xad, 0x42, 20, 0x86, 0x84, 0x21, 1, // local 1 (i64 acc)
        // Fill mem1[65536..+64] with (arg & 0xff)
        0x41, 0x80, 0x80, 0x04, 0x20, 0, 0x41, 64, 0xfc, 0x0b, 1,
        // Copy mem1[65536..+32] -> mem0[256..+32] (dst=0, src=1)
        0x41, 0x80, 0x02, 0x41, 0x80, 0x80, 0x04, 0x41, 32, 0xfc, 0x0a, 0, 1,
        // Copy mem0[0..+16] -> mem2[65536..+16] (dst=2 [i64], src=0 [i32], len [i32])
        0x42, 0x80, 0x80, 0x04, 0x41, 0, 0x41, 16, 0xfc, 0x0a, 2, 0,
        // Copy mem2[65536..+16] -> mem1[65600..+16] (dst=1 [i32], src=2 [i64], len [i32])
        0x41, 0xc0, 0x80, 0x04, 0x42, 0x80, 0x80, 0x04, 0x41, 16, 0xfc, 0x0a, 1, 2,
        // Backward overlapping copy on mem1: mem1[65604..+12] <- mem1[65600..+12]
        0x41, 0xc4, 0x80, 0x04, 0x41, 0xc0, 0x80, 0x04, 0x41, 12, 0xfc, 0x0a, 1, 1,
        // Read back mem1[65604] and mem0[256] into acc
        0x20, 1, 0x41, 0xc4, 0x80, 0x04, 0x29, 0x43, 1, 0, 0x85, 0x41, 0x80, 0x02, 0x29, 0x03, 0,
        0x7c,
    ];

    // f3: oob_mem1(addr: i32) -> i32
    let f3 = vec![0x20, 0, 0x28, 0x42, 1, 0];

    // f4: oob_copy12(len: i32) -> i32 (copies from mem1[65530..+len] -> mem2[0..+len])
    let f4 = vec![
        0x42, 0, 0x41, 0xfa, 0xff, 0x03, 0x20, 0, 0xfc, 0x0a, 2, 1, 0x41, 1,
    ];

    wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7e]), (&[0x7f], &[0x7f])]),
        func_sec(&[0, 0, 0, 1, 1]),
        mem_sec,
        export_sec(&[
            ("blot:memory", 2, 0),
            ("check_init", 0, 0),
            ("rw_and_simd", 0, 1),
            ("grow_and_bulk", 0, 2),
            ("oob_mem1", 0, 3),
            ("oob_copy12", 0, 4),
        ]),
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[(1, 0x7b)], &f1),
            enc_body(&[(1, 0x7e)], &f2),
            enc_body(&[], &f3),
            enc_body(&[], &f4),
        ]),
        section(11, &data_payload),
    ])
}

/// Shared Wasm 3.0 `exception_handling` (`try_table`, `throw`, `throw_ref`, `exnref`) test module.
pub(crate) fn build_exception_handling_test_wasm() -> Vec<u8> {
    // Types:
    //   0: () -> ()                   (tag 0)
    //   1: (i32) -> ()                (tag 1)
    //   2: (i32, i64, f64, v128) -> () (tag 2)
    //   3: (i32) -> (i32)             (callees)
    //   4: (i32) -> (i64)             (dispatch export)
    //   5: () -> (i32, i64, f64, v128) (block type for catch 2)
    //   6: () -> (i32, exnref)        (block type for catch_ref 1)
    //   7: () -> (exnref)             (block type for catch_all_ref)
    //   8: (i32) -> (i32)             (same as 3; reuse 3)
    let types = type_sec(&[
        (&[], &[]),
        (&[0x7f], &[]),
        (&[0x7f, 0x7e, 0x7c, 0x7b], &[]),
        (&[0x7f], &[0x7f]),
        (&[0x7f], &[0x7e]),
        (&[], &[0x7f, 0x7e, 0x7c, 0x7b]),
        (&[], &[0x7f, 0x69]),
        (&[], &[0x69]),
    ]);
    // Tag section (id=13): 3 tags referencing type 0, 1, 2
    let tag_sec = section(13, &[3, 0x00, 0, 0x00, 1, 0x00, 2]);
    // Table 0: funcref, min=2, max=2; elem [0, 1] -> [f0 (f_throw_tag1), f1 (f_throw_tag2)]
    let tbl_sec = section(4, &[1, 0x70, 0x01, 2, 2]);
    let elem_sec = section(9, &[1, 0x00, 0x41, 0, 0x0b, 2, 0, 1]);

    // f0: f_throw_tag1(x: i32) -> i32: if x < 0 { throw 1 (x * 10) } else { x + 1 }
    let f0 = vec![
        0x20, 0, 0x41, 0, 0x48, // x <s 0
        0x04, 0x40, 0x20, 0, 0x41, 10, 0x6c, 0x08, 1, // throw 1 (x * 10)
        0x0b, 0x20, 0, 0x41, 1, 0x6a,
    ];
    // f1: f_throw_tag2(x: i32) -> i32: throws tag 2 (x, i64(x)*1000, f64(x)+0.5, i32x4.splat(x^0x55))
    let mut f1 = vec![
        0x20, 0, // i32: x
        0x20, 0, 0xac, 0x42, 0xe8, 0x07, 0x7e, // i64: x * 1000
        0x20, 0, 0xb7, // f64.convert_i32_s(x)
        0x44,
    ];
    f1.extend_from_slice(&0.5f64.to_le_bytes());
    f1.extend_from_slice(&[
        0xa0, // f64.add
        0x20, 0, 0x41, 0xd5, 0x00, 0x73, 0xfd, 0x11, // i32x4.splat(x ^ 0x55)
        0x08, 2, // throw 2
    ]);
    // f2: f_tail_throw(x: i32) -> i32: inside try_table(catch_all -> returns 999), executes return_call f0(x)!
    // Per Wasm 3.0 spec, return_call pops try_table first -> f2 MUST NOT catch f0's throw!
    let f2 = vec![
        0x02, 0x40, // block $c
        0x1f, 0x7f, 1, 0x02, 0, // try_table (result i32) [catch_all $c]
        0x20, 0, 0x12, 0,    // return_call f0(x)
        0x0b, // end try_table
        0x0f, // return
        0x0b, // end $c
        0x41, 0xe7, 0x07, // 999 (must NOT be reached when f0 throws!)
    ];
    // f3: f_catch_to_fn(x: i32) -> i32: catch 1 targets function frame directly (label 0)!
    let f3 = vec![
        0x1f, 0x40, 1, 0x00, 1, 0, // try_table () [catch tag=1 label=0 (Function)]
        0x20, 0, 0x10, 0, 0x1a, // call f0(x); drop
        0x0b, 0x41, 77, // reached if f0 did not throw
    ];

    // f4: dispatch(case: i32) -> i64
    // Locals: 0=case(i32), 1=tmp(i32), 2=i64tmp(i64), 3=f64tmp(f64), 4=vtmp(v128), 5=exn(exnref)
    let mut f4 = Vec::new();
    let case_branch = |buf: &mut Vec<u8>, id: i32, body: &[u8]| {
        buf.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(buf, id);
        buf.extend_from_slice(&[0x46, 0x04, 0x40]); // if case == id
        buf.extend_from_slice(body);
        buf.push(0x0f); // return
        buf.push(0x0b); // end if
    };
    // Case 0: direct throw 1 + local mutation preserved into catch
    case_branch(
        &mut f4,
        0,
        &[
            0x41, 10, 0x21, 1, // tmp = 10
            0x02, 0x7f, // block $b (result i32)
            0x1f, 0x40, 1, 0x00, 1, 0, // try_table () [catch 1 $b]
            0x41, 33, 0x21, 1, // tmp = 33
            0x41, 55, 0x08, 1, // throw 1 (55)
            0x0b, 0x41, 0, 0x0b, 0x20, 1, 0x6a, 0xac, // (55 + 33) as i64 = 88
        ],
    );
    // Case 1: cross-function direct call throw + local mutation preserved across call clobbers
    case_branch(
        &mut f4,
        1,
        &[
            0x41, 1, 0x21, 1, 0x02, 0x7f, // block $b (result i32)
            0x1f, 0x40, 1, 0x00, 1, 0, // try_table () [catch 1 $b]
            0x41, 99, 0x21, 1, // tmp = 99
            0x41, 0x79, 0x10, 0, 0x1a, // call f0(-7) -> throws -70
            0x0b, 0x41, 0, 0x0b, 0x20, 1, 0x6a, 0xac, // (-70 + 99) as i64 = 29
        ],
    );
    // Case 2: non-throwing call inside try_table
    case_branch(
        &mut f4,
        2,
        &[
            0x02, 0x7f, // block $b (result i32)
            0x1f, 0x7f, 1, 0x00, 1, 0, // try_table (result i32) [catch 1 $b]
            0x41, 10, 0x10, 0, // call f0(10) -> 11
            0x0b, 0x0b, 0xac,
        ],
    );
    // Case 3: call_indirect throwing inside try_table
    case_branch(
        &mut f4,
        3,
        &[
            0x02, 0x7f, 0x1f, 0x40, 1, 0x00, 1, 0, 0x41, 0x7b, 0x41, 0, 0x11, 3, 0,
            0x1a, // call_indirect type 3 tbl 0 (idx=0 -> f0(-5))
            0x0b, 0x41, 0, 0x0b, 0xac, // -50
        ],
    );
    // Case 4: multi-value tag 2 (i32, i64, f64, v128) across call_indirect
    case_branch(
        &mut f4,
        4,
        &[
            0x02, 5, // block $b (result i32 i64 f64 v128)
            0x1f, 0x40, 1, 0x00, 2, 0, // try_table () [catch 2 $b]
            0x41, 17, 0x41, 1, 0x11, 3, 0, 0x1a, // call_indirect idx=1 -> f1(17)
            0x0b, 0x00, // unreachable
            0x0b, // stack: [i32, i64, f64, v128]
            0x21, 4, // vtmp
            0x21, 3, // f64tmp
            0x21, 2, // i64tmp
            0x21, 1, // tmp
            0x20, 1, 0xac, 0x20, 2, 0x7c, 0x20, 3, 0xbd,
            0x85, // ^ i64.reinterpret_f64(f64tmp)
            0x20, 4, 0xfd, 0x1d, 1, 0x85, // ^ i64x2.extract_lane 1 (vtmp)
        ],
    );
    // Case 5: nested try_table (inner misses tag 1 -> outer catches tag 1, preserving inner local mutation)
    case_branch(
        &mut f4,
        5,
        &[
            0x41, 5, 0x21, 1, 0x02, 0x7f, // outer $b1 (result i32)
            0x1f, 0x40, 1, 0x00, 1, 0, // outer try_table [catch 1 $b1]
            0x02, 0x40, // inner $b0 ()
            0x1f, 0x40, 1, 0x00, 0, 0, // inner try_table [catch 0 $b0]
            0x41, 40, 0x21, 1, // tmp = 40
            0x41, 0x7c, 0x10, 0, 0x1a, // call f0(-4) -> throws tag 1 (-40)
            0x0b, // end inner try_table
            0x0b, // end $b0
            0x0b, // end outer try_table
            0x41, 0, 0x0b, // end $b1
            0x20, 1, 0x6a, 0xac, // (-40 + 40) = 0
        ],
    );
    // Case 6: catch_ref 1 -> ref.is_null == 0 -> rethrow via throw_ref -> outer catch 1
    case_branch(
        &mut f4,
        6,
        &[
            0x02, 0x7f, // outer $outer (result i32)
            0x1f, 0x40, 1, 0x00, 1, 0, // outer try_table [catch 1 $outer]
            0x02, 6, // inner $inner (result i32 exnref)
            0x1f, 0x40, 1, 0x01, 1, 0, // inner try_table [catch_ref 1 $inner]
            0x41, 0x77, 0x10, 0, 0x1a, // call f0(-9) -> throws -90
            0x0b, 0x00, 0x0b, // stack: [i32(-90), exnref]
            0x21, 5, // exn = exnref
            0x21, 1, // tmp = -90
            // check ref.is_null(exn) == 0; add 1 if null
            0x20, 5, 0xd1, 0x20, 1, 0x6a, 0x21, 1, 0x20, 5,
            0x0a, // throw_ref(exn) -> caught by outer!
            0x0b, 0x41, 0, 0x0b, // stack: [i32(-90)]
            0x20, 1, 0x6a, 0xac, // -90 + -90 = -180
        ],
    );
    // Case 7: catch_all_ref -> rethrow via throw_ref -> outer catch 0
    case_branch(
        &mut f4,
        7,
        &[
            0x02, 0x40, // $outer0
            0x1f, 0x40, 1, 0x00, 0, 0, // outer [catch 0 $outer0]
            0x02, 7, // $inner (result exnref)
            0x1f, 0x40, 1, 0x03, 0, // inner [catch_all_ref $inner]
            0x08, 0, // throw 0
            0x0b, 0x00, 0x0b, // stack: [exnref]
            0x0a, // throw_ref
            0x0b, 0x42, 0, 0x0f, // should not reach
            0x0b, 0x42, 0x9b, 0x01, // 155
        ],
    );
    // Case 8: catch targeting Function frame directly
    case_branch(
        &mut f4,
        8,
        &[
            0x41, 0x7d, 0x10, 3, // f3(-3) -> catches -30 and returns directly!
            0x41, 5, 0x10, 3, // f3(5) -> 77
            0x6a, 0xac, // 47
        ],
    );
    // Case 9: catch targeting enclosing loop (re-enters loop 5 times via throw!)
    case_branch(
        &mut f4,
        9,
        &[
            0x41, 0, 0x21, 1, // tmp = 0
            0x03, 0x40, // loop $L
            0x1f, 0x40, 1, 0x00, 0, 0, // try_table [catch 0 $L]
            0x20, 1, 0x41, 1, 0x6a, 0x22, 1, // tmp += 1
            0x41, 5, 0x48, // tmp <s 5
            0x04, 0x40, 0x08, 0, 0x0b, // if tmp < 5 { throw 0 } -> jumps to $L!
            0x0b, // end try_table
            0x0b, // end loop
            0x20, 1, 0xac, // 5
        ],
    );
    // Case 10: return_call inside f2's try_table escapes f2 and is caught here!
    case_branch(
        &mut f4,
        10,
        &[
            0x02, 0x7f, // $b (result i32)
            0x1f, 0x7f, 1, 0x00, 1, 0, // try_table (result i32) [catch 1 $b]
            0x41, 0x7a, 0x10, 2, // call f2(-6) -> return_call f0(-6) -> throws -60!
            0x0b, 0x0b, 0xac, // -60 (NOT 999!)
        ],
    );
    // Case 11: throw_ref(ref.null exn) inside try_table(catch_all) -> MUST TRAP (not caught!)
    case_branch(
        &mut f4,
        11,
        &[
            0x02, 0x40, 0x1f, 0x40, 1, 0x02, 0, // catch_all $b
            0xd0, 0x69, 0x0a, // ref.null exn; throw_ref
            0x0b, 0x0b, 0x42, 42,
        ],
    );
    // Case 12: regular trap (div by 0) inside try_table(catch_all) -> MUST TRAP (not caught!)
    case_branch(
        &mut f4,
        12,
        &[
            0x02, 0x40, 0x1f, 0x40, 1, 0x02, 0, // catch_all $b
            0x41, 1, 0x41, 0, 0x6d, 0x1a, // i32.div_s(1, 0)
            0x0b, 0x0b, 0x42, 42,
        ],
    );
    // Case 13: uncaught top-level throw -> traps cleanly
    case_branch(&mut f4, 13, &[0x41, 99, 0x08, 1]);
    f4.extend_from_slice(&[0x42, 0]);

    wasm_module(&[
        types,
        func_sec(&[3, 3, 3, 3, 4]),
        tbl_sec,
        tag_sec,
        export_sec(&[("dispatch", 0, 4)]),
        elem_sec,
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[], &f1),
            enc_body(&[], &f2),
            enc_body(&[], &f3),
            enc_body(
                &[(1, 0x7f), (1, 0x7e), (1, 0x7c), (1, 0x7b), (1, 0x69)],
                &f4,
            ),
        ]),
    ])
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_wasm3_wide_arith_multi_mem_and_exceptions() {
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };

    // 1. Wasm 3.0 `wide-arithmetic` (0xfc13..=0xfc16)
    let wa_wasm = build_wide_arithmetic_test_wasm();
    let wa_inputs = [
        0i64,
        1,
        -1,
        7,
        -7,
        i64::MIN,
        i64::MAX,
        0x0123_4567_89ab_cdef,
    ];
    let mut wa_calls = Vec::new();
    for &x in &wa_inputs {
        for exp in ["add128", "sub128", "mul_wide_s", "mul_wide_u"] {
            wa_calls.push(c(exp, ArgSpec::I64(x)));
        }
    }
    assert_oracle_matrix("wasm3_wide_arithmetic", &wa_wasm, &wa_calls, &[], &[]);

    // 2. Wasm 3.0 `multi-memory` (3 memories: mem32 + mem32 + mem64, active data, SIMD lanes, grow, copy, fill, traps)
    let mm_wasm = build_multi_memory_test_wasm();
    assert_oracle_matrix(
        "wasm3_multi_memory",
        &mm_wasm,
        &[
            c("check_init", ArgSpec::I32(12345)),
            c("rw_and_simd", ArgSpec::I32(0x1357_9bdf)),
            c("grow_and_bulk", ArgSpec::I32(0x42)),
            c("oob_mem1", ArgSpec::I32(131068)), // in-bounds after grow to 2 pages
            c("oob_mem1", ArgSpec::I32(131072)), // OOB trap on mem 1
            c("oob_copy12", ArgSpec::I32(4)),    // ok
            c("oob_copy12", ArgSpec::I32(200_000)), // OOB trap on cross-mem copy
        ],
        &[],
        &[(0, 16), (256, 32)],
    );

    // 3. Wasm 3.0 `exception_handling` (try_table, throw, throw_ref, exnref, tail-call + EH, trap non-catch)
    let eh_wasm = build_exception_handling_test_wasm();
    let eh_calls: Vec<OracleCall> = (0..=13).map(|k| c("dispatch", ArgSpec::I32(k))).collect();
    assert_oracle_matrix("wasm3_exception_handling", &eh_wasm, &eh_calls, &[], &[]);
}

pub(crate) fn build_stage9_loop_opt_test_wasm() -> Vec<u8> {
    // f0(x: i32) -> i32: Euclidean mod 64 + mod 2 + mod 1 + bool-mask checks
    let mut f0 = vec![
        // r1 = euclid(x, 64) via select
        0x20, 0, 0x41, 0xc0, 0x00, 0x6f, 0x21, 1, 0x20, 1, 0x41, 0xc0, 0x00, 0x6a, 0x20, 1, 0x20, 1,
        0x41, 0, 0x48, 0x1b, 0x21, 2,
        // r2 = euclid(x, 2) via if/else (diamond -> CmpSelect -> And 1)
        0x20, 0, 0x41, 2, 0x6f, 0x22, 1, 0x41, 0, 0x48, 0x04, 0x7f, 0x20, 1, 0x41, 2, 0x6a, 0x05,
        0x20, 1, 0x0b, 0x21, 3, // b1 = (x == 7) & 0xff  (redundant mask -> elided)
        0x20, 0, 0x41, 7, 0x46, 0x41, 0xff, 0x01, 0x71,
        // b2 = (x != 0) & 2     (mask bit 0 is 0 -> MUST NOT be elided, always 0!)
        0x20, 0, 0x41, 0, 0x47, 0x41, 2, 0x71, 0x6a,
        // combine: (r1 << 8) ^ (r2 << 4) ^ (b1 + b2)
        0x20, 2, 0x41, 8, 0x74, 0x73, 0x20, 3, 0x41, 4, 0x74, 0x73,
    ];
    let _ = &mut f0;

    // f1(x: i64) -> i64: Euclidean mod 32 + mod 64 (reversed select arm order `r >= 0 ? r : r + 64`)
    let f1 = vec![
        0x20, 0, 0x42, 32, 0x81, 0x21, 1, 0x20, 1, 0x42, 32, 0x7c, 0x20, 1, 0x20, 1, 0x42, 0, 0x53,
        0x1b, 0x21, 2, // r1 = euclid64(x, 32)
        0x20, 0, 0x42, 0xc0, 0x00, 0x81, 0x21, 1, 0x20, 1, 0x20, 1, 0x42, 0xc0, 0x00, 0x7c, 0x20,
        1, 0x42, 0, 0x57, 0x1b, // r2 = (r >= 0) ? r : r + 64
        0x20, 2, 0x42, 16, 0x86, 0x85,
    ];

    // f2(seed: i32) -> i32: seeds a 32x64 grid in memory[0..2048] and runs a 2D 3x3 nested
    // constant-trip stencil loop (`dy in -1..=1`, `dx in -1..=1`) with Euclidean wraparound
    // `(y + dy) mod 32`, `(x + dx) mod 64` (identical shape to Conway's Game of Life!).
    let mut f2 = Vec::new();
    // Seed memory[0..2048]: for i in 0..2048: mem[i] = ((i * 13 + seed) >> 2) & 1
    f2.extend_from_slice(&[0x41, 0, 0x21, 1, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x41]);
    leb_i32(&mut f2, 2048);
    f2.extend_from_slice(&[
        0x4f, 0x0d, 1, 0x20, 1, 0x20, 1, 0x41, 13, 0x6c, 0x20, 0, 0x6a, 0x41, 2, 0x76, 0x41, 1,
        0x71, 0x3a, 0, 0, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b,
    ]);
    // Sum 3x3 stencil around (y = seed & 31, x = (seed >> 5) & 63):
    // local 2 = y, local 3 = x, local 4 = dy, local 5 = dx, local 6 = acc, local 7 = ny, local 8 = nx
    f2.extend_from_slice(&[
        0x20, 0, 0x41, 31, 0x71, 0x21, 2, // y
        0x20, 0, 0x41, 5, 0x76, 0x41, 63, 0x71, 0x21, 3, // x
        0x41, 0, 0x21, 6, // acc = 0
        0x41, 0x7f, 0x21, 4, // dy = -1
        0x03, 0x40, // loop dy
        // ny = euclid(y + dy, 32)
        0x20, 2, 0x20, 4, 0x6a, 0x41, 32, 0x6f, 0x21, 7, 0x20, 7, 0x41, 32, 0x6a, 0x20, 7, 0x20, 7,
        0x41, 0, 0x48, 0x1b, 0x21, 7, // dx = -1
        0x41, 0x7f, 0x21, 5, 0x03, 0x40, // loop dx
        // nx = euclid(x + dx, 64)
        0x20, 3, 0x20, 5, 0x6a, 0x41, 0xc0, 0x00, 0x6f, 0x21, 8, 0x20, 8, 0x41, 0xc0, 0x00, 0x6a,
        0x20, 8, 0x20, 8, 0x41, 0, 0x48, 0x1b, 0x21, 8,
        // acc = acc * 3 + mem[(ny << 6) + nx] + ((dy == 0 && dx == 0) & 0xff)
        0x20, 6, 0x41, 3, 0x6c, 0x20, 7, 0x41, 6, 0x74, 0x20, 8, 0x6a, 0x2d, 0, 0, 0x6a, 0x20, 4,
        0x45, 0x20, 5, 0x45, 0x71, 0x41, 0xff, 0x01, 0x71, 0x6a, 0x21,
        6, // dx += 1; while dx <= 1
        0x20, 5, 0x41, 1, 0x6a, 0x22, 5, 0x41, 1, 0x4c, 0x0d, 0, 0x0b, // end dx
        // dy += 1; while dy <= 1
        0x20, 4, 0x41, 1, 0x6a, 0x22, 4, 0x41, 1, 0x4c, 0x0d, 0, 0x0b, // end dy
        0x20, 6,
    ]);

    // f3(x: i32) -> i32: countdown 4-iter loop (`i = 4; do { ...; i -= 1; } while i > 0`)
    // PLUS a 4-iter loop with conditional early exit (`if x == i { break }`).
    let f3 = vec![
        0x41, 0, 0x21, 1, // acc = 0
        0x41, 4, 0x21, 2, // i = 4
        0x03, 0x40, // loop 1
        0x20, 1, 0x41, 5, 0x6c, 0x20, 0, 0x20, 2, 0x6a, 0x73, 0x21, 1, // acc = acc*5 ^ (x+i)
        0x20, 2, 0x41, 1, 0x6b, 0x22, 2, // i -= 1
        0x41, 0, 0x4a, 0x0d, 0, 0x0b, // while i >s 0
        // loop 2 with early exit
        0x41, 0, 0x21, 2, // i = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 2, 0x20, 0, 0x46, 0x0d, 1, // if i == x break
        0x20, 1, 0x20, 2, 0x41, 17, 0x6c, 0x6a, 0x21, 1, // acc += i * 17
        0x20, 2, 0x41, 1, 0x6a, 0x22, 2, 0x41, 4, 0x48, 0x0d, 0, 0x0b, 0x0b, // while i <s 4
        0x20, 1,
    ];

    wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e])]),
        func_sec(&[0, 1, 0, 0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("euclid32", 0, 0),
            ("euclid64", 0, 1),
            ("stencil3x3", 0, 2),
            ("unroll_edge", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[(3, 0x7f)], &f0),
            enc_body(&[(2, 0x7e)], &f1),
            enc_body(&[(8, 0x7f)], &f2),
            enc_body(&[(2, 0x7f)], &f3),
        ]),
    ])
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_stage9_euclid_rem_bool_mask_and_loop_unroll() {
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    let wasm = build_stage9_loop_opt_test_wasm();
    let mut calls = Vec::new();
    for x in [
        0,
        1,
        -1,
        2,
        -2,
        7,
        63,
        64,
        65,
        -63,
        -64,
        -65,
        -128,
        i32::MIN,
        i32::MAX,
    ] {
        calls.push(c("euclid32", ArgSpec::I32(x)));
    }
    for x in [0i64, 1, -1, 31, 32, -32, -33, 64, -64, i64::MIN, i64::MAX] {
        calls.push(c("euclid64", ArgSpec::I64(x)));
    }
    for seed in [0, 1, 31, 63, 12345, -777] {
        calls.push(c("stencil3x3", ArgSpec::I32(seed)));
    }
    for x in [-1, 0, 1, 2, 3, 4, 99] {
        calls.push(c("unroll_edge", ArgSpec::I32(x)));
    }
    assert_oracle_matrix("stage9_loop_opt", &wasm, &calls, &[], &[]);
}

pub(crate) fn build_bulk_memory_and_table_ops_test_wasm() -> Vec<u8> {
    // Types:
    //   0: (i32) -> (i32)
    //   1: (i32, i32) -> (i32)   (mismatched signature for call_indirect trap check)
    // Functions:
    //   f0 (type 0): x + 10
    //   f1 (type 0): x * 3
    //   f2 (type 0): x - 7
    //   f3 (type 1): a + b
    //   f4 (type 0): dispatch(case: i32) -> i32
    let f0 = vec![0x20, 0, 0x41, 10, 0x6a];
    let f1 = vec![0x20, 0, 0x41, 3, 0x6c];
    let f2 = vec![0x20, 0, 0x41, 7, 0x6b];
    let f3 = vec![0x20, 0, 0x20, 1, 0x6a];

    // Memories:
    //   mem 0: mem32 min=1 max=1
    //   mem 1: mem64 min=1 max=1
    let mem_sec = section(5, &[2, 0x01, 1, 1, 0x05, 1, 1]);
    // Tables:
    //   tbl 0: funcref min=2 max=8
    //   tbl 1: funcref min=3 max=6
    //   tbl 2: externref min=1 max=4
    let tbl_sec = section(
        4,
        &[
            3, 0x70, 0x01, 2, 8, // tbl 0
            0x70, 0x01, 3, 6, // tbl 1
            0x6f, 0x01, 1, 4, // tbl 2
        ],
    );
    // DataCount = 3 (data 0 = active on mem0, data 1 = passive [0x11..0x18], data 2 = passive [0x21..0x24])
    let datacount_sec = section(12, &[3]);
    let data_sec = section(
        11,
        &[
            3, // active seg 0 on mem0 @ 0: [0xaa, 0xbb]
            0x00, 0x41, 0, 0x0b, 2, 0xaa, 0xbb, // passive seg 1 (8 bytes)
            0x01, 8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            // passive seg 2 (4 bytes)
            0x01, 4, 0x21, 0x22, 0x23, 0x24,
        ],
    );
    // Element segments:
    //   elem 0: active tbl 0 @ 0 = [f0, f1]
    //   elem 1: passive funcref = [f2, f0, f3, f1] (using flags=0x05 with exprs so we can include ref.null!)
    //   elem 2: declarative funcref = [f0, f1, f2, f3]
    let elem_sec = section(
        9,
        &[
            3, // elem 0: active tbl 0 @ 0: [f0, f1]
            0x00, 0x41, 0, 0x0b, 2, 0, 1,
            // elem 1: passive (flags=5, elemtype=0x70, 5 exprs: f2, f0, null, f3, f1)
            0x05, 0x70, 5, 0xd2, 2, 0x0b, 0xd2, 0, 0x0b, 0xd0, 0x70, 0x0b, 0xd2, 3, 0x0b, 0xd2, 1,
            0x0b, // elem 2: declarative (flags=3, elemkind=0x00, [0, 1, 2, 3])
            0x03, 0x00, 4, 0, 1, 2, 3,
        ],
    );

    let mut f4 = Vec::new();
    let case_branch = |out: &mut Vec<u8>, k: i32, body: &[u8]| {
        out.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(out, k);
        out.extend_from_slice(&[0x46, 0x04, 0x40]);
        out.extend_from_slice(body);
        out.extend_from_slice(&[0x0f, 0x0b]);
    };
    // Case 0: passive memory.init into mem0 & mem1 (mem64), check exact bytes + len==0 at exact end.
    case_branch(
        &mut f4,
        0,
        &[
            // memory.init data=1 mem=0 (dst=16, src=2, len=4) -> [0x33, 0x44, 0x55, 0x66]
            0x41, 16, 0x41, 2, 0x41, 4, 0xfc, 0x08, 1, 0,
            // memory.init data=1 mem=1 (mem64: dst=32i64, src=0, len=8)
            0x42, 32, 0x41, 0, 0x41, 8, 0xfc, 0x08, 1, 1,
            // zero-len at exact end of seg 1 (src=8, len=0) -> ok!
            0x41, 0, 0x41, 8, 0x41, 0, 0xfc, 0x08, 1, 0,
            // zero-len on active seg 0 (implicitly dropped!) at src=0 -> ok!
            0x41, 0, 0x41, 0, 0x41, 0, 0xfc, 0x08, 0, 0,
            // return i32.load[mem0:16] ^ i32.load[mem1:36] ^ i32.load8_u[mem0:0]
            0x41, 16, 0x28, 2, 0, 0x42, 36, 0x28, 0x42, 1, 0, 0x73, 0x41, 0, 0x2d, 0, 0, 0x73,
        ],
    );
    // Case 1: memory.init on active seg 0 with len=1 -> MUST TRAP OOB!
    case_branch(
        &mut f4,
        1,
        &[0x41, 0, 0x41, 0, 0x41, 1, 0xfc, 0x08, 0, 0, 0x41, 1],
    );
    // Case 2: data.drop 2 (twice, idempotent), then len==0 ok, return 42.
    case_branch(
        &mut f4,
        2,
        &[
            0xfc, 0x09, 2, 0xfc, 0x09, 2, 0x41, 0, 0x41, 0, 0x41, 0, 0xfc, 0x08, 2, 0, 0x41, 42,
        ],
    );
    // Case 3: memory.init on dropped seg 2 with len=1 -> MUST TRAP OOB!
    case_branch(
        &mut f4,
        3,
        &[0x41, 0, 0x41, 0, 0x41, 1, 0xfc, 0x08, 2, 0, 0x41, 1],
    );
    // Case 4: memory.init src OOB (src=9, len=0 on 8-byte seg 1) -> MUST TRAP OOB!
    case_branch(
        &mut f4,
        4,
        &[0x41, 0, 0x41, 9, 0x41, 0, 0xfc, 0x08, 1, 0, 0x41, 1],
    );
    // Case 5: table.size, table.grow (success + overflow -1), table.init, table.copy (overlap fwd & bwd),
    //         table.fill, table.get/set, ref.is_null, and dynamic call_indirect!
    case_branch(
        &mut f4,
        5,
        &[
            // local 1 = acc
            // Initial call_indirect on tbl 0 slots 0 (f0: 5+10=15) and 1 (f1: 5*3=15) + table.size 0 (=2)
            0x41, 5, 0x41, 0, 0x11, 0, 0, 0x41, 5, 0x41, 1, 0x11, 0, 0, 0x6a, 0xfc, 0x10, 0, 0x6a,
            0x21, 1, // grow tbl 0 by 4 with ref.func f2 -> old_len=2, new_len=6
            0xd2, 2, 0x41, 4, 0xfc, 0x0f, 0, 0x20, 1, 0x6a, 0x21, 1,
            // grow tbl 0 by 5 (exceeds max 8!) -> -1, size stays 6
            0xd0, 0x70, 0x41, 5, 0xfc, 0x0f, 0, 0x20, 1, 0x6a, 0xfc, 0x10, 0, 0x6a, 0x21, 1,
            // call_indirect tbl 0 slot 4 (f2: 20-7=13)
            0x41, 20, 0x41, 4, 0x11, 0, 0, 0x20, 1, 0x6a, 0x21, 1,
            // table.init elem=1 tbl=0 (dst=1, src=0, len=5): tbl0 = [f0, f2, f0, null, f3, f1]
            0x41, 1, 0x41, 0, 0x41, 5, 0xfc, 0x0c, 1, 0,
            // check ref.is_null(table.get 0 (3)) == 1 and ref.is_null(table.get 0 (1)) == 0
            0x41, 3, 0x25, 0, 0xd1, 0x41, 1, 0x25, 0, 0xd1, 0x6a, 0x20, 1, 0x6a, 0x21, 1,
            // table.copy tbl1 <- tbl0 (dst=0, src=0, len=3): tbl1 = [f0, f2, f0]
            0x41, 0, 0x41, 0, 0x41, 3, 0xfc, 0x0e, 1, 0,
            // overlapping backward copy on tbl0: dst=2, src=0, len=3 -> tbl0 = [f0, f2, f0, f2, f0, f1]
            0x41, 2, 0x41, 0, 0x41, 3, 0xfc, 0x0e, 0, 0,
            // overlapping forward copy on tbl0: dst=0, src=1, len=4 -> tbl0 = [f2, f0, f2, f0, f0, f1]
            0x41, 0, 0x41, 1, 0x41, 4, 0xfc, 0x0e, 0, 0,
            // call_indirect across tbl0[0..6] and tbl1[1]:
            0x41, 10, 0x41, 0, 0x11, 0, 0, // f2(10)=3
            0x41, 10, 0x41, 1, 0x11, 0, 0, 0x6a, // f0(10)=20
            0x41, 10, 0x41, 5, 0x11, 0, 0, 0x6a, // f1(10)=30
            0x41, 10, 0x41, 1, 0x11, 0, 1, 0x6a, // tbl1[1]=f2(10)=3
            0x20, 1, 0x6a, 0x21, 1, // table.fill tbl0 (dst=2, val=ref.func f1, len=2)
            0x41, 2, 0xd2, 1, 0x41, 2, 0xfc, 0x11, 0,
            // call_indirect tbl0[3] -> f1(7)=21
            0x41, 7, 0x41, 3, 0x11, 0, 0, 0x20, 1, 0x6a, 0x21, 1,
            // table.set tbl0[4] = ref.func f3 (type 1), call_indirect type 1 (11, 22) -> 33!
            0x41, 4, 0xd2, 3, 0x26, 0, 0x41, 11, 0x41, 22, 0x41, 4, 0x11, 1, 0, 0x20, 1, 0x6a, 0x21,
            1, // externref tbl 2: grow(null, 2) -> 1, size -> 3, grow(null, 5) -> -1, fill & copy
            0xd0, 0x6f, 0x41, 2, 0xfc, 0x0f, 2, 0xfc, 0x10, 2, 0x6a, 0xd0, 0x6f, 0x41, 5, 0xfc,
            0x0f, 2, 0x6a, 0x41, 0, 0xd0, 0x6f, 0x41, 3, 0xfc, 0x11, 2, 0x41, 1, 0x41, 0, 0x41, 2,
            0xfc, 0x0e, 2, 2, 0x41, 2, 0x25, 2, 0xd1, 0x6a, 0x20, 1, 0x6a,
        ],
    );
    // Case 6: call_indirect type mismatch on dynamic table (tbl0[4] = f3 (type 1), called as type 0) -> TRAP!
    case_branch(&mut f4, 6, &[0x41, 1, 0x41, 4, 0x11, 0, 0]);
    // Case 7: call_indirect on ref.null slot in dynamic table -> TRAP!
    case_branch(
        &mut f4,
        7,
        &[0x41, 0, 0xd0, 0x70, 0x26, 0, 0x41, 1, 0x41, 0, 0x11, 0, 0],
    );
    // Case 8: elem.drop 1 then table.init len=1 -> TRAP TABLE OOB!
    case_branch(
        &mut f4,
        8,
        &[
            0xfc, 0x0d, 1, 0x41, 0, 0x41, 0, 0x41, 1, 0xfc, 0x0c, 1, 0, 0x41, 1,
        ],
    );
    // Case 9: table.fill OOB -> TRAP TABLE OOB!
    case_branch(
        &mut f4,
        9,
        &[0x41, 5, 0xd0, 0x70, 0x41, 2, 0xfc, 0x11, 0, 0x41, 1],
    );
    // Case 10: table.copy OOB -> TRAP TABLE OOB!
    case_branch(
        &mut f4,
        10,
        &[0x41, 0, 0x41, 2, 0x41, 2, 0xfc, 0x0e, 0, 1, 0x41, 1],
    );
    f4.extend_from_slice(&[0x41, 0]);

    wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f]), (&[0x7f, 0x7f], &[0x7f])]),
        func_sec(&[0, 0, 0, 1, 0]),
        tbl_sec,
        mem_sec,
        export_sec(&[("blot:memory", 2, 0), ("dispatch", 0, 4)]),
        elem_sec,
        datacount_sec,
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[], &f1),
            enc_body(&[], &f2),
            enc_body(&[], &f3),
            enc_body(&[(1, 0x7f)], &f4),
        ]),
        data_sec,
    ])
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_bulk_memory_and_table_ops() {
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    let wasm = build_bulk_memory_and_table_ops_test_wasm();
    let calls: Vec<OracleCall> = (0..=10).map(|k| c("dispatch", ArgSpec::I32(k))).collect();
    assert_oracle_matrix("bulk_mem_table_ops", &wasm, &calls, &[], &[(0, 32)]);
}

/// Shared Wasm 3.0 `table64` + `function-references` test module (used by both x64 and AArch64 oracles).
pub(crate) fn build_table64_and_funcref_test_wasm() -> Vec<u8> {
    let push_i64 = |out: &mut Vec<u8>, v: i64| {
        out.push(0x42);
        leb_i64(out, v);
    };
    let enc_body_raw = |locals: &[(u32, &[u8])], body: &[u8]| -> Vec<u8> {
        let mut b = Vec::new();
        leb_u32(&mut b, locals.len() as u32);
        for &(cnt, ty) in locals {
            leb_u32(&mut b, cnt);
            b.extend_from_slice(ty);
        }
        b.extend_from_slice(body);
        b.push(0x0b);
        let mut out = Vec::new();
        leb_u32(&mut out, b.len() as u32);
        out.extend_from_slice(&b);
        out
    };

    // Types:
    //   0: (i32) -> (i32)
    //   1: (i32, i32) -> (i32)
    //   2: ((ref 0), i32) -> (i32)
    //   3: ((ref null 0), i32) -> (i32)
    //   4: (i32) -> ()                      [tag 0]
    //   5: () -> (i32, (ref 0))             [multi-result block for br_on_non_null]
    let types = section(
        1,
        &[
            6, // count
            0x60, 1, 0x7f, 1, 0x7f, // 0
            0x60, 2, 0x7f, 0x7f, 1, 0x7f, // 1
            0x60, 2, 0x64, 0x00, 0x7f, 1, 0x7f, // 2
            0x60, 2, 0x63, 0x00, 0x7f, 1, 0x7f, // 3
            0x60, 1, 0x7f, 0, // 4
            0x60, 0, 2, 0x7f, 0x64, 0x00, // 5
        ],
    );
    // Tables:
    //   0: dynamic table64 funcref (min=2, max=6), active elem 0 @ 0i64 = [f0, f1]
    //   1: static  table64 (ref 0) (min=3, max=3) explicit init = (ref.func 0)
    //   2: dynamic table64 (ref 0) (min=2, max=5) explicit init = (ref.func 1)
    //   3: dynamic table32 funcref (min=3, max=6)
    let tbl_sec = section(
        4,
        &[
            4, // table count
            0x70, 0x05, 2, 6, // tbl 0
            0x40, 0x00, 0x64, 0x00, 0x05, 3, 3, 0xd2, 0, 0x0b, // tbl 1
            0x40, 0x00, 0x64, 0x00, 0x05, 2, 5, 0xd2, 1, 0x0b, // tbl 2
            0x70, 0x01, 3, 6, // tbl 3
        ],
    );
    let tag_sec = section(13, &[1, 0x00, 4]);
    // Globals:
    //   0: mut (ref null 0) = (ref.func 0)
    //   1: const (ref 0)    = (ref.func 1)
    let global_sec = section(
        6,
        &[
            2, // count
            0x63, 0x00, 0x01, 0xd2, 0, 0x0b, // g0
            0x64, 0x00, 0x00, 0xd2, 1, 0x0b, // g1
        ],
    );
    // Elements:
    //   0: active tbl 0 @ i64.const 0: [f0, f1]
    //   1: passive funcref: [f1, f0, ref.null func]
    //   2: declarative: [f0, f1, f2, f3]
    let elem_sec = section(
        9,
        &[
            3, // count
            0x00, 0x42, 0, 0x0b, 2, 0, 1, // elem 0
            0x05, 0x70, 3, 0xd2, 1, 0x0b, 0xd2, 0, 0x0b, 0xd0, 0x70, 0x0b, // elem 1
            0x03, 0x00, 4, 0, 1, 2, 3, // elem 2
        ],
    );

    // f0(x): x + 10
    let f0 = vec![0x20, 0, 0x41, 10, 0x6a];
    // f1(x): x * 3
    let f1 = vec![0x20, 0, 0x41, 3, 0x6c];
    // f2(x): throw tag 0 (x + 5); 0
    let f2 = vec![0x20, 0, 0x41, 5, 0x6a, 0x08, 0, 0x41, 0];
    // f3(n, acc): if n == 0 { return acc } else { return_call_ref 1 (n - 1, acc + 3, ref.func 3) }
    let f3 = vec![
        0x20, 0, 0x45, 0x04, 0x40, 0x20, 1, 0x0f, 0x0b, 0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x41, 3,
        0x6a, 0xd2, 3, 0x15, 1,
    ];
    // f4(r: (ref 0), x: i32) -> i32: call_ref 0 (x, r)
    let f4 = vec![0x20, 1, 0x20, 0, 0x14, 0];
    // f5(r: (ref null 0), x: i32) -> i32: exercises br_on_null (func target) & br_on_non_null (multi-result block)
    let mut f5 = Vec::new();
    // if x < 0: push 999, local.get 0, br_on_null 1 (returns 999 if null), call_ref 0, return
    f5.extend_from_slice(&[0x20, 1, 0x41, 0, 0x48, 0x04, 0x40, 0x41]);
    leb_i32(&mut f5, 999);
    f5.extend_from_slice(&[0x20, 0, 0xd5, 1, 0x14, 0, 0x0f, 0x0b]);
    // block (type 5 -> (i32, (ref 0))): x, r, br_on_non_null 0, x + 100, ref.func 0 end; call_ref 0
    f5.extend_from_slice(&[0x02, 5, 0x20, 1, 0x20, 0, 0xd6, 0, 0x41]);
    leb_i32(&mut f5, 100);
    f5.extend_from_slice(&[0x6a, 0xd2, 0, 0x0b, 0x14, 0]);

    // f6(case: i32) -> i32: local 1 = acc (i32), local 2 = tmp_ref ((ref null 0))
    let mut f6 = Vec::new();
    let case_branch = |out: &mut Vec<u8>, k: i32, body: &[u8]| {
        out.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(out, k);
        out.extend_from_slice(&[0x46, 0x04, 0x40]);
        out.extend_from_slice(body);
        out.extend_from_slice(&[0x0f, 0x0b]);
    };

    // Case 0: static & dynamic table64 with explicit non-null (ref.func) initializers + table.grow (ok, max-fail, >u32::MAX fail)
    let mut c0 = Vec::new();
    // static tbl 1: call_indirect slot 0i64 (f0(7)=17) + slot 2i64 (f0(8)=18) + table.size 1 (3i64)
    c0.extend_from_slice(&[
        0x41, 7, 0x42, 0, 0x11, 0, 1, 0x41, 8, 0x42, 2, 0x11, 0, 1, 0x6a, 0xfc, 0x10, 1, 0xa7,
        0x6a, 0x21, 1,
    ]);
    // dynamic tbl 2 (init f1=mul3): table.get 2 (1i64) -> call_ref(9)=27
    c0.extend_from_slice(&[0x41, 9, 0x42, 1, 0x25, 2, 0x14, 0, 0x20, 1, 0x6a, 0x21, 1]);
    // table.grow 2 (ref.func 0, 2i64) -> old=2i64; new size=4i64; tbl2[3] is f0(11)=21
    c0.extend_from_slice(&[
        0xd2, 0, 0x42, 2, 0xfc, 0x0f, 2, 0xa7, 0xfc, 0x10, 2, 0xa7, 0x6a, 0x41, 11, 0x42, 3, 0x25,
        2, 0x14, 0, 0x6a, 0x20, 1, 0x6a, 0x21, 1,
    ]);
    // failed grow (>max 5): delta=5i64 -> -1i64 (check == -1i64 -> +100)
    c0.extend_from_slice(&[0xd2, 0, 0x42, 5, 0xfc, 0x0f, 2]);
    push_i64(&mut c0, -1);
    c0.extend_from_slice(&[0x51, 0x41]);
    leb_i32(&mut c0, 100);
    c0.extend_from_slice(&[0x6c, 0x20, 1, 0x6a, 0x21, 1]);
    // failed grow (delta = 0x1_0000_0001i64, low 32 bits == 1 would fit if truncated!) -> MUST return -1i64!
    c0.push(0xd2);
    c0.push(0);
    push_i64(&mut c0, 0x1_0000_0001);
    c0.extend_from_slice(&[0xfc, 0x0f, 2]);
    push_i64(&mut c0, -1);
    c0.extend_from_slice(&[0x51, 0x41]);
    leb_i32(&mut c0, 200);
    c0.extend_from_slice(&[0x6c, 0x20, 1, 0x6a]);
    case_branch(&mut f6, 0, &c0);

    // Case 1: table64 grow + init + fill + mixed copy (64->32, 32->64, 64->64) + set/get + call_indirect
    let mut c1 = Vec::new();
    // Initial tbl0 = [f0, f1]; grow by 3i64 with f1 -> old=2i64, tbl0 = [f0, f1, f1, f1, f1]
    c1.extend_from_slice(&[0xd2, 1, 0x42, 3, 0xfc, 0x0f, 0, 0xa7, 0x21, 1]);
    // table.init elem=1 tbl=0 (dst=2i64, src=0i32, len=3i32): tbl0 = [f0, f1, f1, f0, null]
    c1.extend_from_slice(&[0x42, 2, 0x41, 0, 0x41, 3, 0xfc, 0x0c, 1, 0]);
    // check tbl0[4] is null (1) and tbl0[3] is non-null (0)
    c1.extend_from_slice(&[
        0x42, 4, 0x25, 0, 0xd1, 0x42, 3, 0x25, 0, 0xd1, 0x6a, 0x20, 1, 0x6a, 0x21, 1,
    ]);
    // table.fill tbl=0 (dst=3i64, val=ref.func 1, len=2i64): tbl0 = [f0, f1, f1, f1, f1]
    c1.extend_from_slice(&[0x42, 3, 0xd2, 1, 0x42, 2, 0xfc, 0x11, 0]);
    // table.set tbl=0 (4i64, ref.func 0): tbl0 = [f0, f1, f1, f1, f0]
    c1.extend_from_slice(&[0x42, 4, 0xd2, 0, 0x26, 0]);
    // table.copy tbl3(32) <- tbl0(64) (dst=0i32, src=3i64, len=2i32): tbl3 = [f1, f0, null]
    c1.extend_from_slice(&[0x41, 0, 0x42, 3, 0x41, 2, 0xfc, 0x0e, 3, 0]);
    // table.copy tbl0(64) <- tbl3(32) (dst=0i64, src=0i32, len=2i32): tbl0 = [f1, f0, f1, f1, f0]
    c1.extend_from_slice(&[0x42, 0, 0x41, 0, 0x41, 2, 0xfc, 0x0e, 0, 3]);
    // overlapping table.copy tbl0(64) <- tbl0(64) (dst=1i64, src=0i64, len=3i64): tbl0 = [f1, f1, f0, f1, f0]
    c1.extend_from_slice(&[0x42, 1, 0x42, 0, 0x42, 3, 0xfc, 0x0e, 0, 0]);
    // call_indirect tbl0[1](5)=15 + tbl0[2](5)=15 + tbl3[1](6)=16 + tbl0[4](7)=17
    c1.extend_from_slice(&[
        0x41, 5, 0x42, 1, 0x11, 0, 0, 0x41, 5, 0x42, 2, 0x11, 0, 0, 0x6a, 0x41, 6, 0x41, 1, 0x11,
        0, 3, 0x6a, 0x41, 7, 0x42, 4, 0x11, 0, 0, 0x6a, 0x20, 1, 0x6a,
    ]);
    case_branch(&mut f6, 1, &c1);

    // Case 2: function-references (globals, ref.as_non_null, call_ref, br_on_null, br_on_non_null, 100k return_call_ref)
    let mut c2 = Vec::new();
    // g0 (f0) via ref.as_non_null -> f4(g0!, 5)=15; g1 (non-null f1) -> f4(g1, 6)=18
    c2.extend_from_slice(&[
        0x23, 0, 0xd4, 0x41, 5, 0x10, 4, 0x23, 1, 0x41, 6, 0x10, 4, 0x6a, 0x21, 1,
    ]);
    // mutate g0 = ref.null 0; test f5(g0, -1)=999, f5(g0, 7)=117, f5(g1, -1)=2997, f5(g1, 7)=21
    c2.extend_from_slice(&[
        0xd0, 0, 0x24, 0, 0x23, 0, 0x41, 0x7f, 0x10, 5, 0x23, 0, 0x41, 7, 0x10, 5, 0x6a, 0x23, 1,
        0x41, 0x7f, 0x10, 5, 0x6a, 0x23, 1, 0x41, 7, 0x10, 5, 0x6a, 0x20, 1, 0x6a, 0x21, 1,
    ]);
    // store ref in local 2, call_ref via local 2
    c2.extend_from_slice(&[
        0xd2, 0, 0x21, 2, 0x41, 20, 0x20, 2, 0x14, 0, 0x20, 1, 0x6a, 0x21, 1,
    ]);
    // 100,000-deep return_call_ref tail recursion: f3(100000, 7) = 300007
    c2.push(0x41);
    leb_i32(&mut c2, 100_000);
    c2.extend_from_slice(&[0x41, 7, 0x10, 3, 0x20, 1, 0x6a]);
    case_branch(&mut f6, 2, &c2);

    // Case 3: call_ref inside try_table catching callee throw (tag 0)!
    case_branch(
        &mut f6,
        3,
        &[
            0x02, 0x7f, // block (result i32)
            0x1f, 0x7f, 1, 0x00, 0, 0, // try_table (result i32) (catch 0 0)
            0x41, 37, 0xd2, 2, 0x14, 0, // call_ref 0 (37, ref.func 2) -> throws 42
            0x0b, 0x0b,
        ],
    );
    // Case 4: call_ref on ref.null 0 -> TRAP_INDIRECT_CALL (4)
    case_branch(&mut f6, 4, &[0x41, 1, 0xd0, 0, 0x14, 0]);
    // Case 5: call_ref on ref.null 0 inside try_table (catch_all) -> MUST STILL TRAP (not caught!)
    case_branch(
        &mut f6,
        5,
        &[
            0x02, 0x40, // block
            0x1f, 0x40, 1, 0x02, 0, // try_table (catch_all 0)
            0x41, 1, 0xd0, 0, 0x14, 0, 0x1a, 0x0b, 0x0b, 0x41, 77,
        ],
    );
    // Case 6: return_call_ref on ref.null 0 -> TRAP_INDIRECT_CALL (4)
    case_branch(&mut f6, 6, &[0x41, 1, 0xd0, 0, 0x15, 0]);
    // Case 7: ref.as_non_null on ref.null 0 -> TRAP_INDIRECT_CALL (4)
    case_branch(&mut f6, 7, &[0x41, 1, 0xd0, 0, 0xd4, 0x14, 0]);

    // Cases 8..=15: 64-bit high-word non-zero (0x1_0000_0000i64, low 32 bits == 0!) MUST TRAP OOB across all table64 ops!
    let mut c8 = vec![0x41, 1];
    push_i64(&mut c8, 0x1_0000_0000);
    c8.extend_from_slice(&[0x11, 0, 1]); // static table64 call_indirect
    case_branch(&mut f6, 8, &c8);

    let mut c9 = vec![0x41, 1];
    push_i64(&mut c9, 0x1_0000_0000);
    c9.extend_from_slice(&[0x11, 0, 0]); // dynamic table64 call_indirect
    case_branch(&mut f6, 9, &c9);

    let mut c10 = vec![0x41, 1];
    push_i64(&mut c10, 0x1_0000_0000);
    c10.extend_from_slice(&[0x13, 0, 1]); // static table64 return_call_indirect
    case_branch(&mut f6, 10, &c10);

    let mut c11 = Vec::new();
    push_i64(&mut c11, 0x1_0000_0000);
    c11.extend_from_slice(&[0x25, 0, 0xd1]); // table.get 0
    case_branch(&mut f6, 11, &c11);

    let mut c12 = Vec::new();
    push_i64(&mut c12, 0x1_0000_0000);
    c12.extend_from_slice(&[0xd0, 0x70, 0x26, 0, 0x41, 0]); // table.set 0
    case_branch(&mut f6, 12, &c12);

    let mut c13 = Vec::new();
    push_i64(&mut c13, 0x1_0000_0000);
    c13.extend_from_slice(&[0xd0, 0x70, 0x42, 1, 0xfc, 0x11, 0, 0x41, 0]); // table.fill 0
    case_branch(&mut f6, 13, &c13);

    let mut c14 = vec![0x42, 0, 0x42, 0];
    push_i64(&mut c14, 0x1_0000_0000);
    c14.extend_from_slice(&[0xfc, 0x0e, 0, 0, 0x41, 0]); // table.copy 0 <- 0 (len = 2^32)
    case_branch(&mut f6, 14, &c14);

    let mut c15 = Vec::new();
    push_i64(&mut c15, 0x1_0000_0000);
    c15.extend_from_slice(&[0x41, 0, 0x41, 1, 0xfc, 0x0c, 1, 0, 0x41, 0]); // table.init elem=1 tbl=0
    case_branch(&mut f6, 15, &c15);

    // Case 16: in-bounds table64 return_call_indirect on static tbl 1 (slot 1i64 -> f0(55) = 65)
    case_branch(&mut f6, 16, &[0x41, 55, 0x42, 1, 0x13, 0, 1]);

    f6.extend_from_slice(&[0x41, 0]);

    wasm_module(&[
        types,
        func_sec(&[0, 0, 0, 1, 2, 3, 0]),
        tbl_sec,
        tag_sec,
        global_sec,
        export_sec(&[("dispatch", 0, 6)]),
        elem_sec,
        code_sec(&[
            enc_body_raw(&[], &f0),
            enc_body_raw(&[], &f1),
            enc_body_raw(&[], &f2),
            enc_body_raw(&[], &f3),
            enc_body_raw(&[], &f4),
            enc_body_raw(&[], &f5),
            enc_body_raw(&[(1, &[0x7f]), (1, &[0x63, 0x00])], &f6),
        ]),
    ])
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_wasm3_table64_and_function_references() {
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    let wasm = build_table64_and_funcref_test_wasm();
    let calls: Vec<OracleCall> = (0..=16).map(|k| c("dispatch", ArgSpec::I32(k))).collect();
    assert_oracle_matrix("wasm3_table64_funcref", &wasm, &calls, &[], &[]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_try_table_call_catch_gvn_and_licm() {
    // Tests GVN and LICM invalidation across `Term::CallCatch`:
    //   global 0 (mut i32) = 1
    //   memory 0 (min 1, max 8)
    //   tag 0: () -> ()
    //   func 0 (mut_ok):    g0 += 10; mem[0] += 100; drop(memory.grow(1))
    //   func 1 (mut_throw): g0 += 20; mem[0] += 200; drop(memory.grow(1)); throw 0
    //   func 2 (gvn_ok):    reads (g0, mem[0], mem.size) BEFORE & AFTER try_table(call 0), returns delta
    //   func 3 (gvn_catch): reads (g0, mem[0], mem.size) BEFORE & AFTER try_table(catch 0 -> call 1), returns delta
    //   func 4 (licm_loop): loops `n` times reading g0 + mem.size and calling `try_table(call 0/1)`, returns sum
    let types = type_sec(&[(&[], &[]), (&[0x7f], &[0x7f])]);
    let mem = section(5, &[1, 0x01, 1, 8]);
    let tag = section(13, &[1, 0x00, 0]);
    let globals = section(6, &[1, 0x7f, 0x01, 0x41, 1, 0x0b]);

    let f0 = enc_body(
        &[],
        &[
            0x23, 0, 0x41, 10, 0x6a, 0x24, 0, // g0 += 10
            0x41, 0, 0x41, 0, 0x28, 2, 0, 0x41, 0xe4, 0x00, 0x6a, 0x36, 2, 0, // mem[0] += 100
            0x41, 1, 0x40, 0, 0x1a, // drop(memory.grow(1))
        ],
    );
    let f1 = enc_body(
        &[],
        &[
            0x23, 0, 0x41, 20, 0x6a, 0x24, 0, // g0 += 20
            0x41, 0, 0x41, 0, 0x28, 2, 0, 0x41, 0xc8, 0x01, 0x6a, 0x36, 2, 0, // mem[0] += 200
            0x41, 1, 0x40, 0, 0x1a, // drop(memory.grow(1))
            0x08, 0, // throw 0
        ],
    );
    // f2(x): before = g0 + mem[0] + mem.size*1000; block $c { try_table (catch 0 $c) { call 0 } }; after = g0 + mem[0] + mem.size*1000; return after - before
    let f2 = enc_body(
        &[(1, 0x7f)],
        &[
            0x23, 0, 0x41, 0, 0x28, 2, 0, 0x6a, 0x3f, 0, 0x41, 0xe8, 0x07, 0x6c, 0x6a, 0x21,
            1, // local 1 = before
            0x02, 0x40, // block $c
            0x1f, 0x40, 1, 0x00, 0, 0, // try_table () (catch 0 0)
            0x10, 0,    // call 0 (returns normally -> Term::CallCatch)
            0x0b, // end try_table
            0x0b, // end block
            0x23, 0, 0x41, 0, 0x28, 2, 0, 0x6a, 0x3f, 0, 0x41, 0xe8, 0x07, 0x6c, 0x6a, 0x20, 1,
            0x6b, // after - before
            0x20, 0, 0x6a, // + x
        ],
    );
    // f3(x): same but calls f1 which throws into $c!
    let f3 = enc_body(
        &[(1, 0x7f)],
        &[
            0x23, 0, 0x41, 0, 0x28, 2, 0, 0x6a, 0x3f, 0, 0x41, 0xe8, 0x07, 0x6c, 0x6a, 0x21, 1,
            0x02, 0x40, 0x1f, 0x40, 1, 0x00, 0, 0, 0x10, 1, 0x0b, 0x0b, 0x23, 0, 0x41, 0, 0x28, 2,
            0, 0x6a, 0x3f, 0, 0x41, 0xe8, 0x07, 0x6c, 0x6a, 0x20, 1, 0x6b, 0x20, 0, 0x6a,
        ],
    );
    // f4(n): block $done { loop $lp { if i >= n { br $done }; acc += g0 + mem.size*1000; try_table (catch 0) { call 1 }; i += 1; br $lp } }; return acc
    let f4 = enc_body(
        &[(2, 0x7f)],
        &[
            0x02, 0x40, // block $done
            0x03, 0x40, // loop $lp
            0x20, 1, 0x20, 0, 0x4e, 0x04, 0x40, 0x0c, 2, 0x0b, // if i >= n { br $done }
            0x20, 2, 0x23, 0, 0x3f, 0, 0x41, 0xe8, 0x07, 0x6c, 0x6a, 0x6a, 0x21,
            2, // acc += g0 + size*1000
            0x02, 0x40, 0x1f, 0x40, 1, 0x00, 0, 0, 0x10, 1, 0x0b, 0x0b, 0x20, 1, 0x41, 1, 0x6a,
            0x21, 1, 0x0c, 0,    // i += 1; br $lp
            0x0b, // end loop
            0x0b, // end block
            0x20, 2,
        ],
    );
    let wasm = wasm_module(&[
        types,
        func_sec(&[0, 0, 1, 1, 1]),
        mem,
        tag,
        globals,
        export_sec(&[("gvn_ok", 0, 2), ("gvn_catch", 0, 3), ("licm_loop", 0, 4)]),
        code_sec(&[f0, f1, f2, f3, f4]),
    ]);
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    assert_oracle_matrix(
        "try_table_call_catch_gvn_licm",
        &wasm,
        &[
            c("gvn_ok", ArgSpec::I32(5)),
            c("gvn_catch", ArgSpec::I32(7)),
            c("licm_loop", ArgSpec::I32(3)),
        ],
        &[],
        &[],
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_standalone_x64_cli_parse_u64_rejects_invalid_and_overflow() {
    use std::os::unix::fs::PermissionsExt as _;
    let wasm = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        section(5, &[1, 0x00, 1]),
        export_sec(&[("_start", 0, 0)]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    let opts = X64CompileOptions {
        target: super::module::X64Target::StandaloneExec,
        ..Default::default()
    };
    let elf = compile_wasm_to_x64_so(&wasm, &opts).expect("compile standalone x64");
    let bin_path = std::env::temp_dir().join(format!(
        "wtime-x64-cli-hardening-{}.bin",
        std::process::id()
    ));
    std::fs::write(&bin_path, &elf).unwrap();
    std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o700)).unwrap();

    // Valid flags must exit 0.
    let ok_st = std::process::Command::new(&bin_path)
        .args([
            "--memory-limit-pages",
            "64",
            "--output-limit",
            "18446744073709551615",
        ])
        .status()
        .unwrap();
    assert!(
        ok_st.success(),
        "valid numeric flags should succeed: {ok_st}"
    );

    // Empty, non-numeric, trailing-garbage, and u64-overflowing values must fail closed.
    for args in [
        &["--memory-limit-pages", ""][..],
        &["--memory-limit-pages", "abc"][..],
        &["--memory-limit-pages", "12abc"][..],
        &["--memory-limit-pages", "-1"][..],
        &["--memory-limit-pages", "18446744073709551616"][..],
        &["--memory-limit-pages", "99999999999999999999"][..],
        &["--output-limit", ""][..],
        &["--output-limit", "xyz"][..],
        &["--output-limit", "100x"][..],
        &["--output-limit", "18446744073709551616"][..],
    ] {
        let out = std::process::Command::new(&bin_path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "malformed standalone flag {args:?} must fail closed"
        );
    }
    let _ = std::fs::remove_file(&bin_path);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_x64_simd_spill_swap_and_self_alias_hardening() {
    fn push_sleb_i32(out: &mut Vec<u8>, mut v: i32) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if (v == 0 && (byte & 0x40) == 0) || (v == -1 && (byte & 0x40) != 0) {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    fn push_simd(code: &mut Vec<u8>, sub: u8) {
        code.push(0xfd);
        if sub < 0x80 {
            code.push(sub);
        } else {
            code.push((sub & 0x7f) | 0x80);
            code.push(sub >> 7);
        }
    }
    fn push_v128_const(code: &mut Vec<u8>, bytes: [u8; 16]) {
        push_simd(code, 0x0c);
        code.extend_from_slice(&bytes);
    }
    fn push_fold_v128_to_i32(code: &mut Vec<u8>, v_local: u8) {
        // Fold all 4 i32 lanes of `v_local` via prime multipliers so every byte matters.
        for (lane, mult) in [(0u8, 1i32), (1, 31), (2, 131), (3, 521)] {
            code.extend_from_slice(&[0x20, v_local]);
            push_simd(code, 0x1b); // i32x4.extract_lane
            code.push(lane);
            if mult != 1 {
                code.push(0x41);
                push_sleb_i32(code, mult);
                code.push(0x6c); // i32.mul
            }
            if lane > 0 {
                code.push(0x6a); // i32.add
            }
        }
    }

    // Export 0 (`simd_spill_ops(arg: i32) -> i32`):
    // Keeps 22 live `v128` locals (`local 1..=22`) across the function so regalloc spills many
    // of them (forcing `b == SFP2` with `dst == a == SFP1` and `dst == a` reg + `b == SFP2`).
    // Exercises every swapped/unsigned-compare/andnot/pmin/pmax binary SIMD op category.
    let mut spill_body = Vec::new();
    // Initialize locals 1..=22 from `arg` so nothing constant-folds.
    for i in 1u8..=22 {
        spill_body.extend_from_slice(&[0x20, 0, 0x41]);
        push_sleb_i32(&mut spill_body, i32::from(i) * 17 - 9);
        spill_body.push(0x6a); // arg + (i*17 - 9)
        push_simd(&mut spill_body, 0x11); // i32x4.splat
        let mut pat = [0u8; 16];
        for (k, slot) in pat.iter_mut().enumerate() {
            *slot = i.wrapping_mul(29).wrapping_add((k as u8).wrapping_mul(13));
        }
        push_v128_const(&mut spill_body, pat);
        push_simd(&mut spill_body, 0x51); // v128.xor
        // Clamp exponent bits so every lane is simultaneously a finite non-NaN f32 & f64 AND
        // diverse signed/unsigned i8/i16/i32/i64 bytes (preserving sign bit 31/63).
        push_v128_const(
            &mut spill_body,
            [
                0xff, 0xff, 0xff, 0xbf, 0xff, 0xff, 0xff, 0xbf, 0xff, 0xff, 0xff, 0xbf, 0xff, 0xff,
                0xff, 0xbf,
            ],
        );
        push_simd(&mut spill_body, 0x4e); // v128.and
        push_v128_const(
            &mut spill_body,
            [
                0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00,
                0x80, 0x3e,
            ],
        );
        push_simd(&mut spill_body, 0x50); // v128.or
        spill_body.extend_from_slice(&[0x21, i]); // local.set i
    }
    // `local 23` = accumulator v128 = local 1
    spill_body.extend_from_slice(&[0x20, 1, 0x21, 23]);

    // All 29 swapped/ucmp/andnot/pmin/pmax opcodes, run with BOTH `(spilled_a, spilled_b)` and
    // `(early_a, spilled_b)` into a live accumulator so every `(dst, a, b)` spill combination hits.
    let swap_ops: &[u8] = &[
        0x25, 0x26, 0x28, 0x2a, 0x2b, 0x2c, // i8x16: lt_s, lt_u, gt_u, le_u, ge_s, ge_u
        0x2f, 0x30, 0x32, 0x34, 0x35, 0x36, // i16x8: lt_s, lt_u, gt_u, le_u, ge_s, ge_u
        0x39, 0x3a, 0x3c, 0x3e, 0x3f, 0x40, // i32x4: lt_s, lt_u, gt_u, le_u, ge_s, ge_u
        0x44, 0x46, // f32x4: gt, ge
        0x4a, 0x4c, // f64x2: gt, ge
        0x4f, // v128.andnot
        0xd8, 0xdb, // i64x2: lt_s, ge_s
        0xea, 0xeb, // f32x4: pmin, pmax
        0xf6, 0xf7, // f64x2: pmin, pmax
    ];
    for (idx, &sub) in swap_ops.iter().enumerate() {
        let a_loc = 18u8 + ((idx as u8) % 5); // high locals -> spilled (SFP1)
        let b_loc = 17u8 - ((idx as u8) % 4); // distinct high locals -> spilled (SFP2)
        // Result feeds straight into `local 24` (spilled) then XOR-folded into `local 23`!
        spill_body.extend_from_slice(&[0x20, a_loc, 0x20, b_loc]);
        push_simd(&mut spill_body, sub);
        spill_body.extend_from_slice(&[0x21, 24]);
        spill_body.extend_from_slice(&[0x20, 23, 0x20, 24]);
        push_simd(&mut spill_body, 0xae); // i32x4.add
        spill_body.extend_from_slice(&[0x21, 23]);

        // Also test early (likely-register) `a` with spilled `b` where `dst` can reuse `a`'s reg!
        let reg_loc = 1u8 + ((idx as u8) % 4);
        spill_body.extend_from_slice(&[0x20, reg_loc, 0x20, a_loc]);
        push_simd(&mut spill_body, sub);
        spill_body.extend_from_slice(&[0x20, 23]);
        push_simd(&mut spill_body, 0x51); // v128.xor
        spill_body.extend_from_slice(&[0x21, 23]);
    }
    // Keep ALL 22 locals live until the end of the function so regalloc cannot free them early!
    for i in 1u8..=22 {
        spill_body.extend_from_slice(&[0x20, 23, 0x20, i]);
        push_simd(&mut spill_body, 0x51); // v128.xor
        spill_body.extend_from_slice(&[0x21, 23]);
    }
    push_fold_v128_to_i32(&mut spill_body, 23);

    // Export 1 (`simd_self_alias(arg: i32) -> i32`):
    // Exercises in-place squaring `op(x, x)` with zero register pressure (single live `v128`
    // dead immediately at `op`, forcing `dst == a == b` in regalloc) for all 12 `extmul` ops
    // AND `i64x2.mul`!
    let self_alias_ops: &[u8] = &[
        0x9c, 0x9d, 0x9e, 0x9f, // i16x8.extmul_{low,high}_i8x16_{s,u}
        0xbc, 0xbd, 0xbe, 0xbf, // i32x4.extmul_{low,high}_i16x8_{s,u}
        0xdc, 0xdd, 0xde, 0xdf, // i64x2.extmul_{low,high}_i32x4_{s,u}
        0xd5, // i64x2.mul
    ];
    let mut self_body = Vec::new();
    // local 1 (i32) = acc, local 2 (v128) = tmp
    self_body.extend_from_slice(&[0x41, 0, 0x21, 1]);
    for (idx, &sub) in self_alias_ops.iter().enumerate() {
        // Compute a fresh `x` from `arg` so `x`'s live range starts here and ends at `op(x, x)`.
        self_body.extend_from_slice(&[0x20, 0, 0x41]);
        push_sleb_i32(&mut self_body, (idx as i32) * 37 + 5);
        self_body.push(0x6a);
        push_simd(&mut self_body, 0x11); // i32x4.splat
        let mut pat = [0u8; 16];
        for (k, slot) in pat.iter_mut().enumerate() {
            *slot = ((idx as u8).wrapping_mul(19))
                .wrapping_add((k as u8).wrapping_mul(23))
                .wrapping_add(0x81);
        }
        push_v128_const(&mut self_body, pat);
        push_simd(&mut self_body, 0x51); // v128.xor
        self_body.extend_from_slice(&[0x22, 2, 0x20, 2]); // local.tee 2; local.get 2
        push_simd(&mut self_body, sub); // op(x, x) -> dst aliases x!
        self_body.extend_from_slice(&[0x21, 2]);
        // Fold result into `acc` (`local 1`): `acc = acc * 131 + fold(local 2)`
        self_body.extend_from_slice(&[0x20, 1, 0x41]);
        push_sleb_i32(&mut self_body, 131);
        self_body.push(0x6c);
        push_fold_v128_to_i32(&mut self_body, 2);
        self_body.extend_from_slice(&[0x6a, 0x21, 1]);
    }
    self_body.extend_from_slice(&[0x20, 1]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        export_sec(&[("simd_spill_ops", 0, 0), ("simd_self_alias", 0, 1)]),
        code_sec(&[
            enc_body(&[(24, 0x7b)], &spill_body),
            enc_body(&[(1, 0x7f), (1, 0x7b)], &self_body),
        ]),
    ]);
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    assert_oracle_matrix(
        "x64_simd_spill_swap_and_self_alias",
        &wasm,
        &[
            c("simd_spill_ops", ArgSpec::I32(0)),
            c("simd_spill_ops", ArgSpec::I32(1234567)),
            c("simd_spill_ops", ArgSpec::I32(-9876543)),
            c("simd_self_alias", ArgSpec::I32(0)),
            c("simd_self_alias", ArgSpec::I32(42)),
            c("simd_self_alias", ArgSpec::I32(-777777)),
        ],
        &[],
        &[],
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn oracle_x64_extra_memory_store_spilled_and_const_hardening() {
    // Regression for `Op::StoreMem` + `emit_store_to_mem`: when storing a spilled integer
    // or non-inline rematerialized constant (`i32.store16`, large `i64.store`) into an extra
    // memory (`mem >= 1`), `SCRATCH_GP1` holds an absolute host pointer and must be preserved
    // as `[SCRATCH_GP2]` rather than `[R14 + SCRATCH_GP2]`.
    fn push_sleb_i32(out: &mut Vec<u8>, mut v: i32) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if (v == 0 && (byte & 0x40) == 0) || (v == -1 && (byte & 0x40) != 0) {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    fn push_sleb_i64(out: &mut Vec<u8>, mut v: i64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if (v == 0 && (byte & 0x40) == 0) || (v == -1 && (byte & 0x40) != 0) {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    // Export 0 (`const_stores(arg: i32) -> i32`):
    // Stores rematerialized constants (`i32.store16` and large 64-bit `i64.store`) into memory 1
    // (32-bit extra memory) and memory 2 (64-bit extra memory), then reads them back.
    let mut f0 = Vec::new();
    // mem1[0] = (i16)0x5a3c via i32.store16 (align=0x41 -> memidx=1, offset=0)
    f0.extend_from_slice(&[0x41, 0, 0x41]);
    push_sleb_i32(&mut f0, 0x5a3c);
    f0.extend_from_slice(&[0x3b, 0x41, 1, 0]);
    // mem1[8] = 0x1122_3344_5566_7788 via i64.store (align=0x43 -> memidx=1, offset=8)
    f0.extend_from_slice(&[0x41, 0, 0x42]);
    push_sleb_i64(&mut f0, 0x1122_3344_5566_7788);
    f0.extend_from_slice(&[0x37, 0x43, 1, 8]);
    // mem2[16] = 0x0fed_cba9_8765_4321 via i64.store (memory64 memidx=2, offset=16)
    f0.extend_from_slice(&[0x42, 0, 0x42]);
    push_sleb_i64(&mut f0, 0x0fed_cba9_8765_4321);
    f0.extend_from_slice(&[0x37, 0x43, 2, 16]);
    // Return (i32.load16_u mem1[0]) + wrap(i64.load mem1[8] ^ i64.load mem2[16]) + arg
    f0.extend_from_slice(&[
        0x41, 0, 0x2f, 0x41, 1, 0, // i32.load16_u mem=1 off=0
        0x41, 0, 0x29, 0x43, 1, 8, // i64.load mem=1 off=8
        0x42, 0, 0x29, 0x43, 2, 16,   // i64.load mem=2 off=16
        0x85, // i64.xor
        0xa7, // i32.wrap_i64
        0x6a, // i32.add
        0x20, 0, 0x6a, // + arg
    ]);

    // Export 1 (`spilled_stores(arg: i32) -> i32`):
    // Keeps 20 live GP locals (`local 1..=20`) across the function so several are spilled,
    // then stores every local into memory 1 (`i32.store`, `i32.store16`, `i32.store8`) and
    // sums both the stored values from memory 1 and all live locals.
    let mut f1 = Vec::new();
    for i in 1u8..=20 {
        f1.extend_from_slice(&[0x20, 0, 0x41]);
        push_sleb_i32(&mut f1, i32::from(i) * 131 + 7);
        f1.extend_from_slice(&[0x6a, 0x21, i]);
    }
    for i in 1u8..=20 {
        // mem1[i*8] = local i (i32.store mem=1)
        f1.push(0x41);
        push_sleb_i32(&mut f1, i32::from(i) * 8);
        f1.extend_from_slice(&[0x20, i, 0x36, 0x42, 1, 0]);
        // mem1[256 + i*4] = local i (i32.store16 mem=1)
        f1.push(0x41);
        push_sleb_i32(&mut f1, 256 + i32::from(i) * 4);
        f1.extend_from_slice(&[0x20, i, 0x3b, 0x41, 1, 0]);
        // mem1[512 + i] = local i (i32.store8 mem=1)
        f1.push(0x41);
        push_sleb_i32(&mut f1, 512 + i32::from(i));
        f1.extend_from_slice(&[0x20, i, 0x3a, 0x40, 1, 0]);
    }
    f1.extend_from_slice(&[0x41, 0]); // acc = 0
    for i in 1u8..=20 {
        f1.push(0x41);
        push_sleb_i32(&mut f1, i32::from(i) * 8);
        f1.extend_from_slice(&[0x28, 0x42, 1, 0, 0x6a]);
        f1.push(0x41);
        push_sleb_i32(&mut f1, 256 + i32::from(i) * 4);
        f1.extend_from_slice(&[0x2f, 0x41, 1, 0, 0x6a]);
        f1.push(0x41);
        push_sleb_i32(&mut f1, 512 + i32::from(i));
        f1.extend_from_slice(&[0x2d, 0x40, 1, 0, 0x6a]);
        // Keep all 20 locals live past all stores!
        f1.extend_from_slice(&[0x20, i, 0x6a]);
    }

    // 3 memories: mem0 (32-bit, 1..2), mem1 (32-bit, 1..2), mem2 (64-bit, 1..2)
    let mems = section(5, &[3, 0x01, 1, 2, 0x01, 1, 2, 0x05, 1, 2]);
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        mems,
        export_sec(&[("const_stores", 0, 0), ("spilled_stores", 0, 1)]),
        code_sec(&[enc_body(&[], &f0), enc_body(&[(20, 0x7f)], &f1)]),
    ]);
    let c = |export: &str, arg: ArgSpec| OracleCall {
        export: export.into(),
        arg,
        ret_mode: RetMode::Exact,
    };
    assert_oracle_matrix(
        "x64_extra_memory_store_spilled_and_const",
        &wasm,
        &[
            c("const_stores", ArgSpec::I32(11)),
            c("spilled_stores", ArgSpec::I32(19)),
            c("spilled_stores", ArgSpec::I32(-42)),
        ],
        &[],
        &[],
    );
}
