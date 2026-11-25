#![cfg(all(
    target_os = "linux",
    target_arch = "aarch64",
    target_pointer_width = "64"
))]

//! Native Linux AArch64 integration tests for `wtime-aot --backend aarch64`.
//!
//! Runs directly on ARM64 hardware (e.g. `ubuntu-24.04-arm` in CI) to verify:
//! 1. Blot ABI 8 `.so` compilation, immutable cache publication, `dlopen` loading,
//!    host callback dispatch, globals, linear memory, bulk memory/table ops, SIMD,
//!    exceptions, and trap isolation on real AArch64 silicon.
//! 2. Standalone static `ET_EXEC` compilation and direct kernel execution covering
//!    WASI, Goose OS/search sandbox (`--allow-files`, `--allow-env`, symlink & `..`
//!    denial), `memory64`, and trap exit codes.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const AARCH64_ADMISSION_REJECTED_EXIT_CODE: i32 = 4;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(prefix: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    fn path(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        assert!(p.starts_with(&self.0));
        p
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

fn leb_i64(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0);
        if !done {
            b |= 0x80;
        }
        out.push(b);
        if done {
            break;
        }
    }
}

fn section(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut s = vec![id];
    leb_u32(&mut s, payload.len() as u32);
    s.extend_from_slice(payload);
    s
}

fn wasm_module(sections: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    for s in sections {
        out.extend_from_slice(s);
    }
    out
}

fn enc_str(out: &mut Vec<u8>, s: &str) {
    leb_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn enc_body(locals: &[(u32, u8)], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    leb_u32(&mut b, locals.len() as u32);
    for &(cnt, ty) in locals {
        leb_u32(&mut b, cnt);
        b.push(ty);
    }
    b.extend_from_slice(body);
    b.push(0x0b);
    let mut out = Vec::new();
    leb_u32(&mut out, b.len() as u32);
    out.extend_from_slice(&b);
    out
}

fn assert_aarch64_elf64(path: &Path, expected_type: u16) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() > 64);
    assert_eq!(&bytes[0..4], b"\x7fELF");
    assert_eq!(bytes[4], 2, "must be ELFCLASS64");
    let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
    assert_eq!(e_type, expected_type, "unexpected ELF e_type {e_type}");
    let e_machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    assert_eq!(e_machine, 183, "must be EM_AARCH64 (183), got {e_machine}");
    if expected_type == 2 {
        let e_phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
        let e_phentsize = u16::from_le_bytes([bytes[54], bytes[55]]) as usize;
        let e_phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
        for i in 0..e_phnum {
            let off = e_phoff + i * e_phentsize;
            let p_type = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            assert_ne!(p_type, 3, "static ET_EXEC must have no PT_INTERP");
            assert_ne!(p_type, 2, "static ET_EXEC must have no PT_DYNAMIC");
        }
    }
}

fn detect_host_cc() -> Option<&'static str> {
    for cand in ["cc", "clang", "gcc"] {
        if Command::new(cand)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(cand);
        }
    }
    if std::env::var_os("WTIME_AOT_REQUIRE_NATIVE_TESTS").is_some_and(|v| v == "1") {
        panic!("WTIME_AOT_REQUIRE_NATIVE_TESTS=1 is set, but no host C compiler was found");
    }
    None
}

#[test]
fn native_aarch64_blot_cdylib_abi8_dlopen_and_cache() {
    let bin = Path::new(env!("CARGO_BIN_EXE_wtime-aot"));
    let root = TempRoot::new("wtime-aot-a64-cdylib");
    let cache = root.path("cache");

    // Rich Blot ABI 8 module:
    // - import 0: env::cb(i32) -> i32
    // - memory 0: min=1, max=2
    // - table 0: funcref min=2, max=4
    // - global 0: mut i32 = 10 (exported "g")
    // - passive data 0: [11, 22, 33, 44]
    // - passive elem 0: [f_leaf]
    // - tag 0: (i32) -> ()
    // Exports:
    //   "compute"(x: i32) -> i32:
    //     calls env::cb(x), exercises SIMD (i32x4.add), passive memory.init + data.drop,
    //     dynamic table.grow + table.init + elem.drop + table.copy + table.fill + call_indirect,
    //     try_table + throw catching tag 0, and mutates global 0!
    //   "div_trap"(x: i32) -> i32: 100 / x (traps when x == 0)
    let mut imp = vec![1];
    enc_str(&mut imp, "env");
    enc_str(&mut imp, "cb");
    imp.extend_from_slice(&[0x00, 0]);

    let types = vec![
        2, 0x60, 1, 0x7f, 1, 0x7f, // 0: (i32) -> (i32)
        0x60, 1, 0x7f, 0, // 1: (i32) -> ()  (tag 0)
    ];
    // Defined funcs: f1=leaf, f2=compute, f3=div_trap
    let funcs = vec![3, 0, 0, 0];
    let tables = vec![1, 0x70, 0x01, 2, 4];
    let mems = vec![1, 0x01, 1, 2];
    let tags = vec![1, 0x00, 1];
    let globals = vec![1, 0x7f, 0x01, 0x41, 10, 0x0b];
    let mut exps = vec![4];
    for (name, kind, idx) in [
        ("compute", 0u8, 2u32),
        ("div_trap", 0, 3),
        ("g", 3, 0),
        ("blot:memory", 2, 0),
    ] {
        enc_str(&mut exps, name);
        exps.push(kind);
        leb_u32(&mut exps, idx);
    }
    let elems = vec![1, 0x01, 0x00, 1, 1]; // passive elem 0: [f1]
    let data_count = vec![1];
    let data_sec = vec![1, 0x01, 4, 11, 22, 33, 44];

    // f1(x): return x + 7
    let f1 = vec![0x20, 0, 0x41, 7, 0x6a];
    // f2(x):
    //   1. local 1 = env::cb(x)                          (host callback: x * 2)
    //   2. memory.init seg=0 mem=0 (dst=16, src=0, len=4); data.drop 0
    //   3. SIMD: i32x4.splat(local 1) + i32x4.splat(mem[16] as u8 = 11) -> lane 0
    //   4. table.grow t0 (ref.null func, 1) -> old_sz (2)
    //      table.init elem=0 t0 (dst=2, src=0, len=1); elem.drop 0
    //      table.copy t0 t0 (dst=0, src=2, len=1)
    //      call_indirect type 0 table 0 (arg=lane0, slot=0) -> lane0 + 7
    //   5. try_table catching tag 0: throw 0 (40) -> caught 40
    //   6. global 0 = global 0 + old_sz; return call_res + caught + global 0
    let mut f2 = Vec::new();
    f2.extend_from_slice(&[0x20, 0, 0x10, 0, 0x21, 1]); // local 1 = cb(x)
    f2.extend_from_slice(&[0x41, 16, 0x41, 0, 0x41, 4, 0xfc, 0x08, 0, 0, 0xfc, 0x09, 0]);
    f2.extend_from_slice(&[
        0x20, 1, 0xfd, 0x11, // i32x4.splat(local 1)
        0x41, 16, 0x2d, 0, 0, // i32.load8_u [16] (= 11)
        0xfd, 0x11, // i32x4.splat(11)
        0xfd, 0xae, 0x01, // i32x4.add
        0xfd, 0x1b, 0, // i32x4.extract_lane 0
        0x21, 1, // local 1 = cb(x) + 11
    ]);
    // table.grow t0 (null, 1) -> local 2 (old size = 2)
    f2.extend_from_slice(&[0xd0, 0x70, 0x41, 1, 0xfc, 0x0f, 0, 0x21, 2]);
    // table.init elem=0 t0 (dst=2, src=0, len=1); elem.drop 0; table.copy t0 t0 (dst=0, src=2, len=1)
    f2.extend_from_slice(&[
        0x41, 2, 0x41, 0, 0x41, 1, 0xfc, 0x0c, 0, 0, 0xfc, 0x0d, 0, 0x41, 0, 0x41, 2, 0x41, 1,
        0xfc, 0x0e, 0, 0,
    ]);
    // call_indirect type 0 table 0 (local 1, slot 0) -> local 1 = cb(x) + 11 + 7
    f2.extend_from_slice(&[0x20, 1, 0x41, 0, 0x11, 0, 0, 0x21, 1]);
    // try_table catching tag 0: throws 40 -> local 3 = 40
    f2.extend_from_slice(&[
        0x02, 0x7f, 0x1f, 0x7f, 1, 0x00, 0, 0, 0x41, 40, 0x08, 0, 0x0b, 0x0b, 0x21, 3,
    ]);
    // global 0 += local 2 (10 + 2 = 12); return local 1 + local 3 + global 0
    f2.extend_from_slice(&[
        0x23, 0, 0x20, 2, 0x6a, 0x22, 2, 0x24, 0, 0x20, 1, 0x20, 3, 0x6a, 0x20, 2, 0x6a,
    ]);
    // f3(x): 100 / x
    let f3 = vec![0x41, 0xe4, 0x00, 0x20, 0, 0x6d];

    let mut code = vec![3];
    code.extend_from_slice(&enc_body(&[], &f1));
    code.extend_from_slice(&enc_body(&[(3, 0x7f)], &f2));
    code.extend_from_slice(&enc_body(&[], &f3));

    let wasm = wasm_module(&[
        section(1, &types),
        section(2, &imp),
        section(3, &funcs),
        section(4, &tables),
        section(5, &mems),
        section(13, &tags),
        section(6, &globals),
        section(7, &exps),
        section(9, &elems),
        section(12, &data_count),
        section(10, &code),
        section(11, &data_sec),
    ]);

    let wasm_path = root.path("abi8.wasm");
    let rej_path = root.path("rej.wasm");
    let bad_path = root.path("bad.wasm");
    fs::write(&wasm_path, &wasm).unwrap();
    // Admission-rejected module (exported table -> exit code 4).
    let rej_wasm = wasm_module(&[
        section(4, &[1, 0x70, 0x00, 1]),
        section(7, &[1, 1, b't', 1, 0]),
    ]);
    fs::write(&rej_path, &rej_wasm).unwrap();
    fs::write(&bad_path, b"corrupt-not-wasm").unwrap();

    // Cold build via CLI `--backend aarch64 --blot-cdylib`.
    let cold = Command::new(bin)
        .arg("build")
        .arg(&wasm_path)
        .args(["--cache"])
        .arg(&cache)
        .args([
            "--blot-cdylib",
            "--backend",
            "aarch64",
            "--time-limit",
            "60",
        ])
        .output()
        .unwrap();
    assert!(
        cold.status.success(),
        "aarch64 cold cdylib build failed: {}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let so_path = PathBuf::from(String::from_utf8_lossy(&cold.stdout).trim());
    assert!(so_path.starts_with(&cache));
    assert_aarch64_elf64(&so_path, 3); // ET_DYN
    let sha_path = so_path.with_extension("so.sha256");
    assert_eq!(
        fs::symlink_metadata(&so_path).unwrap().permissions().mode() & 0o777,
        0o555
    );
    assert_eq!(
        fs::symlink_metadata(&sha_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o444
    );

    // Warm cache hit returns exact same path.
    let warm = Command::new(bin)
        .arg("build")
        .arg(&wasm_path)
        .args(["--cache"])
        .arg(&cache)
        .args([
            "--blot-cdylib",
            "--backend",
            "aarch64",
            "--time-limit",
            "60",
        ])
        .output()
        .unwrap();
    assert!(warm.status.success());
    assert_eq!(
        PathBuf::from(String::from_utf8_lossy(&warm.stdout).trim()),
        so_path
    );

    // Exit codes: 4 on admission rejection, 1 on malformed Wasm.
    let rej = Command::new(bin)
        .arg("build")
        .arg(&rej_path)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "aarch64"])
        .output()
        .unwrap();
    assert_eq!(
        rej.status.code(),
        Some(AARCH64_ADMISSION_REJECTED_EXIT_CODE)
    );
    let bad = Command::new(bin)
        .arg("build")
        .arg(&bad_path)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "aarch64"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));

    // Native dlopen & execution test on ARM64 hardware!
    let Some(cc) = detect_host_cc() else {
        return;
    };
    let drv_c = root.path("a64_drv.c");
    let drv_bin = root.path("a64_drv");
    fs::write(
        &drv_c,
        r#"
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int32_t (*host_fn_t)(void *, void *, uint32_t, uint64_t, uint64_t, uint64_t *);
typedef uint32_t (*fn_u32)(void);
typedef int32_t (*fn_imp_name)(uint32_t, const uint8_t **, size_t *, const uint8_t **, size_t *);
typedef void *(*fn_new)(void *, host_fn_t, uint64_t, uint8_t *);
typedef void (*fn_drop)(void *);
typedef int32_t (*fn_call)(void *, const uint8_t *, size_t, uint64_t, uint64_t *, uint8_t *);
typedef int32_t (*fn_get_g)(void *, const uint8_t *, size_t, uint8_t *, uint64_t *);
typedef int32_t (*fn_set_g)(void *, const uint8_t *, size_t, uint64_t);
typedef uint8_t *(*fn_mem_ptr)(void *);
typedef size_t (*fn_mem_len)(void *);

static int32_t my_host(void *u, void *ctx, uint32_t imp, uint64_t a0, uint64_t a1, uint64_t *out) {
    (void)u; (void)ctx; (void)a1;
    if (imp != 0) return 1;
    *out = (uint64_t)((uint32_t)a0 * 2u);
    return 0;
}

#define CHECK(cond, msg) do { if (!(cond)) { fprintf(stderr, "FAIL: %s\n", msg); return 1; } } while (0)

int main(int argc, char **argv) {
    CHECK(argc == 2, "expected .so path");
    void *dl = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    CHECK(dl != NULL, dlerror());
    fn_u32 abi_ver = (fn_u32)dlsym(dl, "blot_aot_abi_version");
    fn_u32 imp_cnt = (fn_u32)dlsym(dl, "blot_aot_import_count");
    fn_imp_name imp_name = (fn_imp_name)dlsym(dl, "blot_aot_import_name");
    fn_new aot_new = (fn_new)dlsym(dl, "blot_aot_new");
    fn_drop aot_drop = (fn_drop)dlsym(dl, "blot_aot_drop");
    fn_call aot_call = (fn_call)dlsym(dl, "blot_aot_call_export");
    fn_get_g aot_get_g = (fn_get_g)dlsym(dl, "blot_aot_get_global");
    fn_set_g aot_set_g = (fn_set_g)dlsym(dl, "blot_aot_set_global");
    fn_mem_ptr mem_ptr = (fn_mem_ptr)dlsym(dl, "blot_aot_memory_ptr");
    fn_mem_len mem_len = (fn_mem_len)dlsym(dl, "blot_aot_memory_len");
    CHECK(abi_ver && imp_cnt && imp_name && aot_new && aot_drop && aot_call && aot_get_g && aot_set_g && mem_ptr && mem_len, "missing symbol");

    CHECK(abi_ver() == 1, "abi_version != 1");
    CHECK(imp_cnt() == 1, "import_count != 1");
    const uint8_t *mp = NULL, *np = NULL;
    size_t ml = 0, nl = 0;
    CHECK(imp_name(0, &mp, &ml, &np, &nl) == 0, "import_name failed");
    CHECK(ml == 3 && memcmp(mp, "env", 3) == 0 && nl == 2 && memcmp(np, "cb", 2) == 0, "import_name mismatch");

    uint8_t trap = 0;
    void *h = aot_new(NULL, my_host, 0, &trap);
    CHECK(h != NULL && trap == 0, "blot_aot_new failed");
    CHECK(mem_len(h) == 65536, "initial memory len != 65536");

    uint64_t res = 0;
    // compute(5): cb(5)=10 -> +11=21 -> +7=28 -> +40=68 -> global 10+2=12 -> 68+12=80!
    CHECK(aot_call(h, (const uint8_t *)"compute", 7, 5, &res, &trap) == 0, "compute call failed");
    CHECK((uint32_t)res == 80u, "compute(5) != 80");

    uint8_t *m = mem_ptr(h);
    CHECK(m[16] == 11 && m[17] == 22 && m[18] == 33 && m[19] == 44, "memory.init bytes mismatch");

    uint8_t kind = 0;
    uint64_t gv = 0;
    CHECK(aot_get_g(h, (const uint8_t *)"g", 1, &kind, &gv) == 0 && (uint32_t)gv == 12u, "get_global g != 12");
    CHECK(aot_set_g(h, (const uint8_t *)"g", 1, 99) == 0, "set_global failed");
    CHECK(aot_get_g(h, (const uint8_t *)"g", 1, &kind, &gv) == 0 && (uint32_t)gv == 99u, "get_global after set != 99");

    // div_trap(4) == 25; div_trap(0) traps cleanly (rc == 2, trap != 0)!
    CHECK(aot_call(h, (const uint8_t *)"div_trap", 8, 4, &res, &trap) == 0 && (uint32_t)res == 25u, "div_trap(4) != 25");
    trap = 0;
    CHECK(aot_call(h, (const uint8_t *)"div_trap", 8, 0, &res, &trap) == 2 && trap != 0, "div_trap(0) must trap");

    aot_drop(h);
    dlclose(dl);
    return 0;
}
"#,
    )
    .unwrap();
    let cc_out = Command::new(cc)
        .args(["-O1", "-o"])
        .arg(&drv_bin)
        .arg(&drv_c)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        cc_out.status.success(),
        "C driver compile failed: {}",
        String::from_utf8_lossy(&cc_out.stderr)
    );
    let run = Command::new(&drv_bin).arg(&so_path).output().unwrap();
    assert!(
        run.status.success(),
        "native ARM64 dlopen harness failed ({:?}): {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn native_aarch64_standalone_static_exec_wasi_goose_sandbox_and_traps() {
    let bin = Path::new(env!("CARGO_BIN_EXE_wtime-aot"));
    let root = TempRoot::new("wtime-aot-a64-exec");

    // 1. WASI + SIMD standalone executable: prints "arm64 ok\n" and calls proc_exit(42).
    let mut imp_wasi = vec![2];
    enc_str(&mut imp_wasi, "wasi_snapshot_preview1");
    enc_str(&mut imp_wasi, "fd_write");
    imp_wasi.extend_from_slice(&[0x00, 0]);
    enc_str(&mut imp_wasi, "wasi_snapshot_preview1");
    enc_str(&mut imp_wasi, "proc_exit");
    imp_wasi.extend_from_slice(&[0x00, 1]);

    let types_wasi = vec![
        3, 0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f, // 0: fd_write
        0x60, 1, 0x7f, 0, // 1: proc_exit
        0x60, 0, 0, // 2: _start
    ];
    let mut sb_wasi = Vec::new();
    // Write iov at offset 0: ptr=64, len=9 ("arm64 ok\n"), then fd_write(1, 0, 1, 16), proc_exit(42).
    sb_wasi.extend_from_slice(&[
        0x41, 0, 0x41, 64, 0x36, 2, 0, // [0] = 64
        0x41, 4, 0x41, 9, 0x36, 2, 0, // [4] = 9
        0x41, 1, 0x41, 0, 0x41, 1, 0x41, 16, 0x10, 0, 0x1a, // fd_write
        0x41, 42, 0x10, 1, // proc_exit(42)
    ]);
    let mut data_wasi = vec![1, 0x00, 0x41, 64, 0x0b, 9];
    data_wasi.extend_from_slice(b"arm64 ok\n");
    let mut exp_wasi = vec![2];
    enc_str(&mut exp_wasi, "_start");
    exp_wasi.extend_from_slice(&[0x00, 2]);
    enc_str(&mut exp_wasi, "memory");
    exp_wasi.extend_from_slice(&[0x02, 0]);
    let wasi_wasm = wasm_module(&[
        section(1, &types_wasi),
        section(2, &imp_wasi),
        section(3, &[1, 2]),
        section(5, &[1, 0x01, 1, 2]),
        section(7, &exp_wasi),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[], &sb_wasi))
                .collect::<Vec<_>>(),
        ),
        section(11, &data_wasi),
    ]);
    let wasi_path = root.path("wasi_a64.wasm");
    let wasi_exe = root.path("wasi_a64");
    fs::write(&wasi_path, &wasi_wasm).unwrap();
    let st = Command::new(bin)
        .arg(&wasi_path)
        .args(["-o"])
        .arg(&wasi_exe)
        .args(["--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "aarch64 standalone WASI compile failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    assert_aarch64_elf64(&wasi_exe, 2); // ET_EXEC
    let run_wasi = Command::new(&wasi_exe).output().unwrap();
    assert_eq!(run_wasi.status.code(), Some(42));
    assert_eq!(String::from_utf8_lossy(&run_wasi.stdout), "arm64 ok\n");

    // 2. Goose OS + memory64 + sandbox (`--allow-files`, `--allow-env`, symlink escape denied).
    let sandbox = root.path("sandbox");
    let outside = root.path("outside");
    fs::create_dir_all(&sandbox).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.txt");
    fs::write(&secret, b"TOP_SECRET").unwrap();
    let escape_link = sandbox.join("escape.txt");
    std::os::unix::fs::symlink(&secret, &escape_link).unwrap();

    let mut imp = vec![8];
    for (m, n, ti) in [
        ("goose", "print_i64", 0u32),
        ("goose", "write", 1),
        ("goose", "newline", 2),
        ("env", "gs_os_arg", 3),
        ("env", "gs_os_getenv", 4),
        ("env", "gs_os_write_file", 5),
        ("env", "gs_os_read_file", 4),
        ("env", "gs_find_any", 6),
    ] {
        enc_str(&mut imp, m);
        enc_str(&mut imp, n);
        imp.push(0x00);
        leb_u32(&mut imp, ti);
    }
    let types_g = vec![
        8, 0x60, 1, 0x7e, 0, // 0: print_i64(i64)
        0x60, 2, 0x7e, 0x7f, 0, // 1: write(i64, i32)
        0x60, 0, 0, // 2: newline()
        0x60, 2, 0x7e, 0x7e, 0, // 3: gs_os_arg(i64, i64)
        0x60, 3, 0x7e, 0x7e, 0x7e, 1, 0x7f, // 4: getenv/read_file -> i32
        0x60, 4, 0x7e, 0x7e, 0x7e, 0x7e, 1, 0x7f, // 5: write_file -> i32
        0x60, 3, 0x7e, 0x7e, 0x7e, 1, 0x7e, // 6: gs_find_any -> i64
        0x60, 2, 0x7e, 0x7e, 1, 0x7e, // 7: goose.reserve(i64, i64) -> i64
    ];
    let mut data_g = vec![4];
    let mut add_seg = |off: i64, bytes: &[u8]| {
        data_g.push(0x00);
        data_g.push(0x42);
        leb_i64(&mut data_g, off);
        data_g.push(0x0b);
        leb_u32(&mut data_g, bytes.len() as u32);
        data_g.extend_from_slice(bytes);
    };
    add_seg(256, b"FOO");
    add_seg(272, b"hello arm64!");
    let mut tbl_o = [0u8; 256];
    tbl_o[b'a' as usize] = 1;
    add_seg(1024, &tbl_o);
    add_seg(2048, b"xyz");

    // goose.reserve(ctrl, len): writes [ctrl+0]=old_base, [ctrl+8]=len, [ctrl+16]=len, returns base
    let mut res_b = Vec::new();
    res_b.extend_from_slice(&[
        0x20, 0, 0x29, 3, 0, 0x21, 2, // l2 = base
        0x20, 0, 0x20, 1, 0x37, 3, 8, // [ctrl+8] = len
        0x20, 0, 0x20, 1, 0x37, 3, 16, // [ctrl+16] = len
        0x20, 2,
    ]);
    let push_i64 = |b: &mut Vec<u8>, v: i64| {
        b.push(0x42);
        leb_i64(b, v);
    };
    let mut sb = Vec::new();
    // 1. gs_find_any("hello arm64!", 'a') -> 6
    push_i64(&mut sb, 272);
    push_i64(&mut sb, 12);
    push_i64(&mut sb, 1024);
    sb.extend_from_slice(&[0x10, 7, 0x10, 0, 0x10, 2]);
    // 2. Load arg 1 (hello.txt) into ctrl 16384 (buf 16400) and arg 2 (escape.txt) into ctrl 20480 (buf 20496)
    for (arg_i, ctrl, base) in [(1i64, 16384i64, 16400i64), (2, 20480, 20496)] {
        push_i64(&mut sb, ctrl);
        push_i64(&mut sb, base);
        sb.extend_from_slice(&[0x37, 3, 0]);
        push_i64(&mut sb, arg_i);
        push_i64(&mut sb, ctrl);
        sb.extend_from_slice(&[0x10, 3]);
    }
    // 3. getenv("FOO") -> 1 + print "arm_env"
    push_i64(&mut sb, 4096);
    push_i64(&mut sb, 8192);
    sb.extend_from_slice(&[0x37, 3, 0]);
    push_i64(&mut sb, 256);
    push_i64(&mut sb, 3);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x10, 4, 0xac, 0x10, 0, 0x10, 2]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x29, 3, 0]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x29, 3, 8, 0xa7, 0x10, 1, 0x10, 2]);
    // 4. write_file(hello.txt, "xyz") -> 1; read_file(hello.txt) -> 1 + "xyz"
    push_i64(&mut sb, 16384);
    sb.extend_from_slice(&[0x29, 3, 0]);
    push_i64(&mut sb, 16384);
    sb.extend_from_slice(&[0x29, 3, 8]);
    push_i64(&mut sb, 2048);
    push_i64(&mut sb, 3);
    sb.extend_from_slice(&[0x10, 5, 0xac, 0x10, 0, 0x10, 2]);
    push_i64(&mut sb, 4096);
    push_i64(&mut sb, 8192);
    sb.extend_from_slice(&[0x37, 3, 0]);
    push_i64(&mut sb, 16384);
    sb.extend_from_slice(&[0x29, 3, 0]);
    push_i64(&mut sb, 16384);
    sb.extend_from_slice(&[0x29, 3, 8]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x10, 6, 0xac, 0x10, 0, 0x10, 2]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x29, 3, 0]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x29, 3, 8, 0xa7, 0x10, 1, 0x10, 2]);
    // 5. read_file(escape.txt) -> MUST be denied (0)!
    push_i64(&mut sb, 4096);
    push_i64(&mut sb, 8192);
    sb.extend_from_slice(&[0x37, 3, 0]);
    push_i64(&mut sb, 20480);
    sb.extend_from_slice(&[0x29, 3, 0]);
    push_i64(&mut sb, 20480);
    sb.extend_from_slice(&[0x29, 3, 8]);
    push_i64(&mut sb, 4096);
    sb.extend_from_slice(&[0x10, 6, 0xac, 0x10, 0, 0x10, 2]);

    let mut exp_g = vec![3];
    enc_str(&mut exp_g, "_start");
    exp_g.extend_from_slice(&[0x00, 9]);
    enc_str(&mut exp_g, "goose.reserve");
    exp_g.extend_from_slice(&[0x00, 8]);
    enc_str(&mut exp_g, "memory");
    exp_g.extend_from_slice(&[0x02, 0]);

    let goose_wasm = wasm_module(&[
        section(1, &types_g),
        section(2, &imp),
        section(3, &[2, 7, 2]),
        section(5, &[1, 0x05, 1, 4]), // memory64!
        section(7, &exp_g),
        section(
            10,
            &[2].into_iter()
                .chain(enc_body(&[(1, 0x7e)], &res_b))
                .chain(enc_body(&[], &sb))
                .collect::<Vec<_>>(),
        ),
        section(11, &data_g),
    ]);
    let goose_path = root.path("goose_a64.wasm");
    let goose_exe = root.path("goose_a64");
    fs::write(&goose_path, &goose_wasm).unwrap();
    let st_g = Command::new(bin)
        .arg(&goose_path)
        .args(["-o"])
        .arg(&goose_exe)
        .args(["--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        st_g.status.success(),
        "aarch64 standalone goose compile failed: {}",
        String::from_utf8_lossy(&st_g.stderr)
    );
    assert_aarch64_elf64(&goose_exe, 2);

    let hello_file = sandbox.join("hello.txt");
    let run_g = Command::new(&goose_exe)
        .env("FOO", "arm_env")
        .args(["--allow-files"])
        .arg(&sandbox)
        .args(["--allow-env", "FOO", "--"])
        .arg(&hello_file)
        .arg(&escape_link)
        .output()
        .unwrap();
    assert!(
        run_g.status.success(),
        "native ARM64 goose exec failed ({:?}): {}",
        run_g.status.code(),
        String::from_utf8_lossy(&run_g.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run_g.stdout),
        "6\n1\narm_env\n1\n1\nxyz\n0\n"
    );
    assert_eq!(fs::read(&hello_file).unwrap(), b"xyz");
    assert_eq!(fs::read(&secret).unwrap(), b"TOP_SECRET");

    // 3. Unreachable trap exits non-zero cleanly on ARM64 Linux.
    let trap_wasm = wasm_module(&[
        section(1, &[1, 0x60, 0, 0]),
        section(3, &[1, 0]),
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 0]),
        section(10, &[1, 3, 0, 0x00, 0x0b]),
    ]);
    let trap_path = root.path("trap_a64.wasm");
    let trap_exe = root.path("trap_a64");
    fs::write(&trap_path, &trap_wasm).unwrap();
    let st_t = Command::new(bin)
        .arg(&trap_path)
        .args(["-o"])
        .arg(&trap_exe)
        .args(["--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(st_t.status.success());
    let run_t = Command::new(&trap_exe).output().unwrap();
    assert!(!run_t.status.success(), "unreachable must exit non-zero");

    // 4. Wasm 3.0 `table64` + `function-references` (`call_ref`, `return_call_ref`, explicit table init)
    //    executed natively on ARM64 silicon via static ET_EXEC!
    let mut imp_w3 = vec![1];
    enc_str(&mut imp_w3, "goose");
    enc_str(&mut imp_w3, "print_i64");
    imp_w3.extend_from_slice(&[0x00, 2]);
    let types_w3 = vec![
        4, // count
        0x60, 1, 0x7f, 1, 0x7f, // 0: (i32) -> (i32)
        0x60, 2, 0x7f, 0x7f, 1, 0x7f, // 1: (i32, i32) -> (i32)
        0x60, 1, 0x7e, 0, // 2: (i64) -> ()
        0x60, 0, 0, // 3: () -> ()
    ];
    // f0=print_i64 (import), f1=add10, f2=tail_rec, f3=_start
    let f1_w3 = vec![0x20, 0, 0x41, 10, 0x6a];
    let f2_w3 = vec![
        0x20, 0, 0x45, 0x04, 0x40, 0x20, 1, 0x0f, 0x0b, 0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x41, 2,
        0x6a, 0xd2, 2, 0x15, 1,
    ];
    let mut f3_w3 = Vec::new();
    // table.get 0 (1i64) -> f1 (via explicit init) -> call_ref(32) = 42; + f2(50000, 0) = 100000 -> print 100042
    f3_w3.extend_from_slice(&[0x41, 32, 0x42, 1, 0x25, 0, 0x14, 0, 0x41]);
    leb_i64(&mut f3_w3, 50_000);
    f3_w3.extend_from_slice(&[0x41, 0, 0x10, 2, 0x6a, 0xac, 0x10, 0]);
    let w3_wasm = wasm_module(&[
        section(1, &types_w3),
        section(2, &imp_w3),
        section(3, &[3, 0, 1, 3]),
        // table64 (min=2, max=4) with explicit init = (ref.func 1)
        section(4, &[1, 0x40, 0x00, 0x64, 0x00, 0x05, 2, 4, 0xd2, 1, 0x0b]),
        section(5, &[1, 0x01, 1, 1]),
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 3]),
        section(9, &[1, 0x03, 0x00, 2, 1, 2]),
        section(
            10,
            &[3].into_iter()
                .chain(enc_body(&[], &f1_w3))
                .chain(enc_body(&[], &f2_w3))
                .chain(enc_body(&[], &f3_w3))
                .collect::<Vec<_>>(),
        ),
    ]);
    let w3_path = root.path("w3_a64.wasm");
    let w3_exe = root.path("w3_a64");
    fs::write(&w3_path, &w3_wasm).unwrap();
    let st_w3 = Command::new(bin)
        .arg(&w3_path)
        .args(["-o"])
        .arg(&w3_exe)
        .args(["--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        st_w3.status.success(),
        "aarch64 standalone wasm3 compile failed: {}",
        String::from_utf8_lossy(&st_w3.stderr)
    );
    assert_aarch64_elf64(&w3_exe, 2);
    let run_w3 = Command::new(&w3_exe).output().unwrap();
    assert!(run_w3.status.success());
    assert_eq!(String::from_utf8_lossy(&run_w3.stdout), "100042");
}
