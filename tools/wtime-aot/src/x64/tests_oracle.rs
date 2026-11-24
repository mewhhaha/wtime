//! Stage 1 Differential Oracle (`wtime-runtime` vs `--backend x64`) across all 4 matrix cells:
//! `(opt=false, v2)`, `(opt=false, v3)`, `(opt=true, v2)`, `(opt=true, v3)`,
//! running `verify_ir` + `verify_alloc` on every single function.

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
                .args(["-ldl"])
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

fn run_x64_so(
    driver: &Path,
    so_path: &Path,
    calls: &[OracleCall],
    check_globals: &[&str],
    check_mem: &[(u64, usize)],
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

    let mut child = Command::new(driver)
        .arg(so_path)
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
                || !std::is_x86_feature_detected!("bmi2"))
        {
            continue;
        }
        let so_bytes = compile_wasm_to_x64_so(
            wasm,
            &X64CompileOptions {
                isa: TargetIsa {
                    has_abm_bmi1: v3,
                    has_bmi2: v3,
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
        let actual = run_x64_so(driver, &so_path, calls, check_globals, check_mem);
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
