#![cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_pointer_width = "64"
))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const X64_ADMISSION_REJECTED_EXIT_CODE: i32 = 4;

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

#[test]
fn cli_and_cached_build_x64_exit_codes_immutability_and_cache_keys() {
    let bin = Path::new(env!("CARGO_BIN_EXE_wtime-aot"));
    let root = TempRoot::new("wtime-aot-x64-cli");
    let cache = root.path("cache");
    let ok_wasm = root.path("ok.wasm");
    let rej_wasm = root.path("rej.wasm");
    let bad_wasm = root.path("bad.wasm");
    let direct_so = root.path("direct.so");

    // Simple valid module: export "inc" (i32) -> (i32) { local.get 0; i32.const 1; i32.add }
    let ok_bytes = wasm_module(&[
        section(1, &[1, 0x60, 1, 0x7f, 1, 0x7f]),
        section(3, &[1, 0]),
        section(7, &[1, 3, b'i', b'n', b'c', 0, 0]),
        section(10, &[1, 7, 0, 0x20, 0, 0x41, 1, 0x6a, 0x0b]),
    ]);
    // Admission-rejected module (multi-memory -> exit code 4!).
    let rej_bytes = wasm_module(&[section(5, &[2, 0x00, 1, 0x00, 1])]);
    fs::write(&ok_wasm, &ok_bytes).unwrap();
    fs::write(&rej_wasm, &rej_bytes).unwrap();
    fs::write(&bad_wasm, b"corrupt-not-wasm").unwrap();

    // 1. Direct CLI `-o direct.so --blot-cdylib --backend x64` with WTIME_AOT_TIMINGS=1
    let direct = Command::new(bin)
        .env("WTIME_AOT_TIMINGS", "1")
        .arg(&ok_wasm)
        .args(["-o"])
        .arg(&direct_so)
        .args([
            "--blot-cdylib",
            "--backend",
            "x64",
            "--target-cpu",
            "x86-64-v2",
            "--jobs",
            "2",
            "--time-limit",
            "30",
        ])
        .output()
        .unwrap();
    assert!(
        direct.status.success(),
        "direct x64 CLI failed: {}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert!(direct_so.is_file());
    let timings_stderr = String::from_utf8_lossy(&direct.stderr);
    for phase in [
        "validate=",
        "admit=",
        "ssa_opt=",
        "regalloc=",
        "emit=",
        "link=",
        "write=",
        "total=",
    ] {
        assert!(
            timings_stderr.contains(phase),
            "missing timing phase `{phase}` in stderr: {timings_stderr}"
        );
    }

    // 2. Cached `build --backend x64` cold -> immutable 0o555 library.so + 0o444 .sha256 -> warm hit!
    let cold = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args([
            "--blot-cdylib",
            "--backend",
            "x64",
            "--target-cpu",
            "native",
            "--time-limit",
            "30",
        ])
        .output()
        .unwrap();
    assert!(
        cold.status.success(),
        "cold build x64 failed: {}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let so_path = Path::new(String::from_utf8_lossy(&cold.stdout).trim()).to_path_buf();
    let sha_path = so_path.with_extension("so.sha256");
    assert!(
        so_path.starts_with(&cache),
        "so_path escaped cache: {so_path:?}"
    );
    let so_meta = fs::symlink_metadata(&so_path).unwrap();
    assert!(so_meta.file_type().is_file());
    assert_eq!(so_meta.permissions().mode() & 0o777, 0o555);
    let sha_meta = fs::symlink_metadata(&sha_path).unwrap();
    assert!(sha_meta.file_type().is_file());
    assert_eq!(sha_meta.permissions().mode() & 0o777, 0o444);
    assert_eq!(fs::read_to_string(&sha_path).unwrap().trim().len(), 64);

    // Verify `native` maps to the exact same cache key as the resolved `x86-64-v3` or `x86-64-v2` profile!
    let resolved_flag = if std::is_x86_feature_detected!("lzcnt")
        && std::is_x86_feature_detected!("bmi1")
        && std::is_x86_feature_detected!("bmi2")
        && std::is_x86_feature_detected!("avx2")
    {
        "x86-64-v3"
    } else {
        "x86-64-v2"
    };
    let warm = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args([
            "--blot-cdylib",
            "--backend",
            "x64",
            "--target-cpu",
            resolved_flag,
            "--time-limit",
            "30",
        ])
        .output()
        .unwrap();
    assert!(warm.status.success());
    assert_eq!(
        Path::new(String::from_utf8_lossy(&warm.stdout).trim()),
        so_path.as_path(),
        "native and resolved profile must share the exact same cache entry"
    );

    // 3. Machine-readable exit code 4 on admission rejection (both direct & build)!
    let rej_build = Command::new(bin)
        .arg("build")
        .arg(&rej_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "x64"])
        .output()
        .unwrap();
    assert_eq!(
        rej_build.status.code(),
        Some(X64_ADMISSION_REJECTED_EXIT_CODE),
        "expected exit code 4 on admission rejection, got {:?}: {}",
        rej_build.status.code(),
        String::from_utf8_lossy(&rej_build.stderr)
    );
    let rej_direct = Command::new(bin)
        .arg(&rej_wasm)
        .args(["-o"])
        .arg(root.path("rej.so"))
        .args(["--blot-cdylib", "--backend", "x64"])
        .output()
        .unwrap();
    assert_eq!(
        rej_direct.status.code(),
        Some(X64_ADMISSION_REJECTED_EXIT_CODE)
    );

    // 4. Malformed Wasm exits with code 1 (NEVER 4!).
    let bad_build = Command::new(bin)
        .arg("build")
        .arg(&bad_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "x64"])
        .output()
        .unwrap();
    assert_eq!(bad_build.status.code(), Some(1));

    // 5. Invalid --target-cpu and --train with --backend x64 exit with code 1 (NEVER 4!).
    let bad_cpu = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args([
            "--blot-cdylib",
            "--backend",
            "x64",
            "--target-cpu",
            "invalid-cpu-xyz",
        ])
        .output()
        .unwrap();
    assert_eq!(bad_cpu.status.code(), Some(1));

    let bad_train = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "x64", "--train", "echo"])
        .output()
        .unwrap();
    assert_eq!(bad_train.status.code(), Some(1));
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

fn assert_static_elf64_exec(path: &Path) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() > 64);
    assert_eq!(&bytes[0..4], b"\x7fELF");
    assert_eq!(bytes[4], 2, "must be ELFCLASS64");
    let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
    assert_eq!(e_type, 2, "must be ET_EXEC (2), got {e_type}");
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

#[test]
fn standalone_x64_static_exec_wasi_goose_simd_memory64_and_traps() {
    use std::io::Write as _;
    use std::process::Stdio;

    let bin = Path::new(env!("CARGO_BIN_EXE_wtime-aot"));
    let root = TempRoot::new("wtime-aot-x64-exec");
    let cache = root.path("cache");

    // 1. WASI standalone executable:
    //    - multi-iov fd_write to stdout ("hello " + "wasi\n")
    //    - fd_read from stdin into memory[64..96], echoes back to stdout
    //    - args_sizes_get + args_get -> echoes arg[0] + "\n" to stdout
    //    - clock_time_get(monotonic) -> checks returns 0
    //    - proc_exit(42)
    let mut imp_wasi = vec![5];
    // 0: fd_write(i32,i32,i32,i32)->i32 (type 0)
    // 1: fd_read(i32,i32,i32,i32)->i32 (type 0)
    // 2: args_sizes_get(i32,i32)->i32 (type 1)
    // 3: args_get(i32,i32)->i32 (type 1)
    // 4: proc_exit(i32)->() (type 2)
    for (name, ty) in [
        ("fd_write", 0u32),
        ("fd_read", 0),
        ("args_sizes_get", 1),
        ("args_get", 1),
        ("proc_exit", 2),
    ] {
        enc_str(&mut imp_wasi, "wasi_snapshot_preview1");
        enc_str(&mut imp_wasi, name);
        imp_wasi.push(0x00);
        leb_u32(&mut imp_wasi, ty);
    }
    // Data at 0: "hello wasi\n" (11 B)
    let mut data_wasi = vec![1, 0x00, 0x41, 0, 0x0b, 11];
    data_wasi.extend_from_slice(b"hello wasi\n");
    let push_i32 = |b: &mut Vec<u8>, v: i32| {
        b.push(0x41);
        leb_i64(b, i64::from(v));
    };
    let mut wasi_body = Vec::new();
    // iov[0] at 16 = (0, 6), iov[1] at 24 = (6, 5)
    for (addr, val) in [(16, 0), (20, 6), (24, 6), (28, 5)] {
        push_i32(&mut wasi_body, addr);
        push_i32(&mut wasi_body, val);
        wasi_body.extend_from_slice(&[0x36, 2, 0]);
    }
    // fd_write(1, 16, 2, 32)
    for v in [1, 16, 2, 32] {
        push_i32(&mut wasi_body, v);
    }
    wasi_body.extend_from_slice(&[0x10, 0, 0x1a]);
    // Read stdin into 64 (max 32): iov at 40 = (64, 32), nread at 48
    for (addr, val) in [(40, 64), (44, 32)] {
        push_i32(&mut wasi_body, addr);
        push_i32(&mut wasi_body, val);
        wasi_body.extend_from_slice(&[0x36, 2, 0]);
    }
    for v in [0, 40, 1, 48] {
        push_i32(&mut wasi_body, v);
    }
    wasi_body.extend_from_slice(&[0x10, 1, 0x1a]);
    // Write stdin echo: iov at 40 = (64, mem[48])
    push_i32(&mut wasi_body, 44);
    push_i32(&mut wasi_body, 48);
    wasi_body.extend_from_slice(&[0x28, 2, 0, 0x36, 2, 0]);
    for v in [1, 40, 1, 52] {
        push_i32(&mut wasi_body, v);
    }
    wasi_body.extend_from_slice(&[0x10, 0, 0x1a]);
    // args_sizes_get(100, 104), args_get(128, 256)
    push_i32(&mut wasi_body, 100);
    push_i32(&mut wasi_body, 104);
    wasi_body.extend_from_slice(&[0x10, 2, 0x1a]);
    push_i32(&mut wasi_body, 128);
    push_i32(&mut wasi_body, 256);
    wasi_body.extend_from_slice(&[0x10, 3, 0x1a]);
    // Write first 3 bytes of arg[0] ("foo")
    push_i32(&mut wasi_body, 40);
    push_i32(&mut wasi_body, 128);
    wasi_body.extend_from_slice(&[0x28, 2, 0, 0x36, 2, 0]);
    push_i32(&mut wasi_body, 44);
    push_i32(&mut wasi_body, 3);
    wasi_body.extend_from_slice(&[0x36, 2, 0]);
    for v in [1, 40, 1, 52] {
        push_i32(&mut wasi_body, v);
    }
    wasi_body.extend_from_slice(&[0x10, 0, 0x1a]);
    // proc_exit(42)
    push_i32(&mut wasi_body, 42);
    wasi_body.extend_from_slice(&[0x10, 4]);
    let wasi_wasm = wasm_module(&[
        section(
            1,
            &[
                4, // 4 types
                0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f, // 0
                0x60, 2, 0x7f, 0x7f, 1, 0x7f, // 1
                0x60, 1, 0x7f, 0, // 2
                0x60, 0, 0, // 3: _start
            ],
        ),
        section(2, &imp_wasi),
        section(3, &[1, 3]),
        section(5, &[1, 0x01, 1, 1]),
        section(
            7,
            &[
                2, 6, b'_', b's', b't', b'a', b'r', b't', 0, 5, 6, b'm', b'e', b'm', b'o', b'r',
                b'y', 2, 0,
            ],
        ),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[], &wasi_body))
                .collect::<Vec<_>>(),
        ),
        section(11, &data_wasi),
    ]);
    let wasi_path = root.path("wasi.wasm");
    let wasi_exe = root.path("wasi_exe");
    let timing_json = root.path("timing.json");
    fs::write(&wasi_path, &wasi_wasm).unwrap();

    // Direct compilation: wtime-aot wasi.wasm -o wasi_exe --backend x64
    let st = Command::new(bin)
        .arg(&wasi_path)
        .args(["-o"])
        .arg(&wasi_exe)
        .args(["--backend", "x64"])
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "direct standalone x64 build failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    assert_static_elf64_exec(&wasi_exe);

    let mut child = Command::new(&wasi_exe)
        .args(["--timing-json"])
        .arg(&timing_json)
        .args(["--", "foo", "bar"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"PING:").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(42),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello wasi\nPING:foo");
    let tj = fs::read_to_string(&timing_json).unwrap();
    assert!(
        tj.contains("\"engine\": \"wtime-aot\""),
        "bad timing json: {tj}"
    );
    assert!(
        tj.contains("\"start_function_ns\": "),
        "bad timing json: {tj}"
    );

    // 2. Cached build (`wtime-aot build --backend x64` defaults to GooseBin standalone executable!)
    //    Goose + memory64 + 128-bit SIMD standalone binary!
    let mut imp_g = vec![5];
    for (name, ty) in [
        ("print_i64", 0u32),
        ("print_u64", 0),
        ("print_bool", 1),
        ("newline", 2),
        ("exit", 1),
    ] {
        enc_str(&mut imp_g, "goose");
        enc_str(&mut imp_g, name);
        imp_g.push(0x00);
        leb_u32(&mut imp_g, ty);
    }
    let mut g_body = Vec::new();
    // Store two v128 vectors into memory64 at 0 and 16, add i64x2, extract lane 0 + print_i64!
    g_body.extend_from_slice(&[0x42, 0, 0xfd, 12]);
    g_body.extend_from_slice(&(-1000i64).to_le_bytes());
    g_body.extend_from_slice(&42i64.to_le_bytes());
    g_body.extend_from_slice(&[0xfd, 11, 0, 0]); // v128.store at 0
    g_body.extend_from_slice(&[0x42, 0, 0xfd, 0, 0, 0]); // v128.load(0)
    g_body.extend_from_slice(&[0xfd, 12]);
    g_body.extend_from_slice(&(-234i64).to_le_bytes());
    g_body.extend_from_slice(&58i64.to_le_bytes());
    g_body.extend_from_slice(&[0xfd, 0xce, 0x01]); // i64x2.add -> (-1234, 100)
    g_body.extend_from_slice(&[0xfd, 0x1d, 0]); // i64x2.extract_lane 0 -> -1234
    g_body.extend_from_slice(&[0x10, 0, 0x10, 3]); // print_i64(-1234); newline()
    // print_i64(i64::MIN); newline()
    g_body.push(0x42);
    leb_i64(&mut g_body, i64::MIN);
    g_body.extend_from_slice(&[0x10, 0, 0x10, 3]);
    // print_u64(u64::MAX); newline()
    g_body.push(0x42);
    leb_i64(&mut g_body, -1);
    g_body.extend_from_slice(&[0x10, 1, 0x10, 3]);
    // print_bool(1); newline(); exit(7)
    g_body.extend_from_slice(&[0x41, 1, 0x10, 2, 0x10, 3, 0x41, 7, 0x10, 4]);

    let goose_wasm = wasm_module(&[
        section(
            1,
            &[
                3, 0x60, 1, 0x7e, 0, // 0: (i64)->()
                0x60, 1, 0x7f, 0, // 1: (i32)->()
                0x60, 0, 0, // 2: ()->()
            ],
        ),
        section(2, &imp_g),
        section(3, &[1, 2]),
        section(5, &[1, 0x05, 1, 2]), // memory64!
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 5]),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[], &g_body))
                .collect::<Vec<_>>(),
        ),
    ]);
    let goose_path = root.path("goose.wasm");
    fs::write(&goose_path, &goose_wasm).unwrap();

    let built = Command::new(bin)
        .arg("build")
        .arg(&goose_path)
        .args(["--cache"])
        .arg(&cache)
        .args(["--backend", "x64"])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "cached standalone build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let prog = PathBuf::from(String::from_utf8_lossy(&built.stdout).trim());
    assert_static_elf64_exec(&prog);
    let gout = Command::new(&prog).output().unwrap();
    assert_eq!(gout.status.code(), Some(7));
    assert_eq!(
        String::from_utf8_lossy(&gout.stdout),
        "-1234\n-9223372036854775808\n18446744073709551615\ntrue\n"
    );

    // 3. Standalone trap reporting: i32.div_s(1, 0) -> stderr "integer divide by zero\n", exit code 1!
    let trap_wasm = wasm_module(&[
        section(1, &[1, 0x60, 0, 0]),
        section(3, &[1, 0]),
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 0]),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[], &[0x41, 1, 0x41, 0, 0x6d, 0x1a]))
                .collect::<Vec<_>>(),
        ),
    ]);
    let trap_path = root.path("trap.wasm");
    let trap_exe = root.path("trap_exe");
    fs::write(&trap_path, &trap_wasm).unwrap();
    let st = Command::new(bin)
        .arg(&trap_path)
        .args(["-o"])
        .arg(&trap_exe)
        .args(["--backend", "x64"])
        .status()
        .unwrap();
    assert!(st.success());
    let tout = Command::new(&trap_exe).output().unwrap();
    assert_eq!(tout.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&tout.stderr),
        "Trap: integer divide by zero\n"
    );

    // 4. Goose `abort(ptr, len, line)` prints message + line to stderr and exits 1.
    let mut imp_ab = vec![1];
    enc_str(&mut imp_ab, "goose");
    enc_str(&mut imp_ab, "abort");
    imp_ab.push(0x00);
    leb_u32(&mut imp_ab, 0);
    let mut data_ab = vec![1, 0x00, 0x41, 0, 0x0b, 4];
    data_ab.extend_from_slice(b"boom");
    let mut ab_body = Vec::new();
    push_i32(&mut ab_body, 0);
    push_i32(&mut ab_body, 4);
    push_i32(&mut ab_body, 99);
    ab_body.extend_from_slice(&[0x10, 0]);
    let abort_wasm = wasm_module(&[
        section(1, &[2, 0x60, 3, 0x7f, 0x7f, 0x7f, 0, 0x60, 0, 0]),
        section(2, &imp_ab),
        section(3, &[1, 1]),
        section(5, &[1, 0x01, 1, 1]),
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 1]),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[], &ab_body))
                .collect::<Vec<_>>(),
        ),
        section(11, &data_ab),
    ]);
    let ab_path = root.path("abort.wasm");
    let ab_exe = root.path("abort_exe");
    fs::write(&ab_path, &abort_wasm).unwrap();
    assert!(
        Command::new(bin)
            .arg(&ab_path)
            .args(["-o"])
            .arg(&ab_exe)
            .args(["--backend", "x64"])
            .status()
            .unwrap()
            .success()
    );
    let aout = Command::new(&ab_exe).output().unwrap();
    assert_eq!(aout.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&aout.stderr),
        "goose runtime error: boom (line 99)\n"
    );

    // 5. Standalone guard-page OOB trap (both 1-page and 0-page memory32) + `--timing-json` still emitted!
    for (label, min_pages, addr) in [("1page", 1u8, 65_536i32), ("0page", 0u8, 0i32)] {
        let mut oob_body = Vec::new();
        push_i32(&mut oob_body, addr);
        oob_body.extend_from_slice(&[0x28, 2, 0, 0x1a]); // i32.load offset=0; drop
        let oob_wasm = wasm_module(&[
            section(1, &[1, 0x60, 0, 0]),
            section(3, &[1, 0]),
            section(5, &[1, 0x01, min_pages, 2]),
            section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 0]),
            section(
                10,
                &[1].into_iter()
                    .chain(enc_body(&[], &oob_body))
                    .collect::<Vec<_>>(),
            ),
        ]);
        let p = root.path(&format!("oob_{label}.wasm"));
        let e = root.path(&format!("oob_{label}_exe"));
        let tj = root.path(&format!("oob_{label}_timing.json"));
        fs::write(&p, &oob_wasm).unwrap();
        assert!(
            Command::new(bin)
                .arg(&p)
                .args(["-o"])
                .arg(&e)
                .args(["--backend", "x64"])
                .status()
                .unwrap()
                .success()
        );
        let out = Command::new(&e)
            .args(["--timing-json"])
            .arg(&tj)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "oob_{label} stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "Trap: out of bounds\n",
            "oob_{label}"
        );
        let tj_txt = fs::read_to_string(&tj).unwrap();
        assert!(
            tj_txt.contains("\"start_function_ns\": "),
            "oob_{label} bad timing json: {tj_txt}"
        );
    }

    // 6. Goose float formatting (`print_f64`, `print_f32`, `format_f64`, `format_f32`) +
    //    `env::bench_input` / `env::bench_input_f64` exact round-trip parity with `wtime-aot-rt`.
    let mut imp_f = vec![8];
    for (m, n, ty) in [
        ("goose", "print_f64", 0u32),  // 0: (f64)->()
        ("goose", "print_f32", 1),     // 1: (f32)->()
        ("goose", "format_f64", 2),    // 2: (f64, i32)->i32
        ("goose", "format_f32", 3),    // 3: (f32, i32)->i32
        ("goose", "write", 4),         // 4: (i32, i32)->()
        ("goose", "newline", 5),       // 5: ()->()
        ("env", "bench_input", 6),     // 6: ()->i64
        ("env", "bench_input_f64", 7), // 7: ()->f64
    ] {
        enc_str(&mut imp_f, m);
        enc_str(&mut imp_f, n);
        imp_f.push(0x00);
        leb_u32(&mut imp_f, ty);
    }
    #[allow(clippy::excessive_precision)]
    let f64_cases: &[(f64, &str)] = &[
        (0.0, "0.0"),
        (-0.0, "-0.0"),
        (f64::NAN, "nan"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (1.0, "1.0"),
        (-1.5, "-1.5"),
        (0.1, "0.1"),
        (123.456, "123.456"),
        (1e-4, "0.0001"),
        (1e-5, "1e-05"),
        (1.5e-7, "1.5e-07"),
        (1e14, "100000000000000.0"),
        (1e15, "1e+15"),
        (123456789012345680.0, "1.2345678901234568e+17"),
        (1234567890123456.0, "1234567890123456.0"),
        (1e100, "1e+100"),
        (5e-324, "5e-324"),
        (f64::MAX, "1.7976931348623157e+308"),
        (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
        (0.30000000000000004, "0.30000000000000004"),
        (2.0f64.powi(-25), "2.9802322387695312e-08"),
        (12345678901234.5625, "12345678901234.562"),
        (597659464700.90625, "597659464700.9062"),
        (9007199254740993.0 * 4.0, "3.602879701896397e+16"),
    ];
    let f32_cases: &[(f32, &str)] = &[
        (0.0f32, "0.0"),
        (-0.0, "-0.0"),
        (f32::NAN, "nan"),
        (f32::INFINITY, "inf"),
        (f32::NEG_INFINITY, "-inf"),
        (1.0, "1.0"),
        (0.1, "0.1"),
        (-2.5, "-2.5"),
        (16777216.0, "16777216.0"),
        (1e10, "10000000000.0"),
        (1e15, "1e+15"),
        (1e-5, "1e-05"),
        (3.4028235e38, "3.4028235e+38"),
        (1.1754944e-38, "1.1754944e-38"),
        (1e-45, "1e-45"),
        (0.3, "0.3"),
        (9.999999e-5, "9.999999e-05"),
    ];
    let mut f_body = Vec::new();
    let mut expected_f = String::new();
    for &(v, txt) in f64_cases {
        f_body.push(0x44);
        f_body.extend_from_slice(&v.to_bits().to_le_bytes());
        f_body.extend_from_slice(&[0x10, 0, 0x10, 5]); // print_f64; newline
        expected_f.push_str(txt);
        expected_f.push('\n');
    }
    // Also exercise format_f64(ptr, val) -> write into memory at offset 64
    push_i32(&mut f_body, 64);
    f_body.push(0x44);
    f_body.extend_from_slice(&(-123.456f64).to_bits().to_le_bytes());
    f_body.extend_from_slice(&[0x10, 2, 0x21, 0]); // len = format_f64(64, -123.456)
    push_i32(&mut f_body, 64);
    f_body.extend_from_slice(&[0x20, 0, 0x10, 4, 0x10, 5]); // write(64, len); newline
    expected_f.push_str("-123.456\n");
    for &(v, txt) in f32_cases {
        f_body.push(0x43);
        f_body.extend_from_slice(&v.to_bits().to_le_bytes());
        f_body.extend_from_slice(&[0x10, 1, 0x10, 5]); // print_f32; newline
        expected_f.push_str(txt);
        expected_f.push('\n');
    }
    // Exercise format_f32(ptr, val) -> write
    push_i32(&mut f_body, 128);
    f_body.push(0x43);
    f_body.extend_from_slice(&0.1f32.to_bits().to_le_bytes());
    f_body.extend_from_slice(&[0x10, 3, 0x21, 0]);
    push_i32(&mut f_body, 128);
    f_body.extend_from_slice(&[0x20, 0, 0x10, 4, 0x10, 5]);
    expected_f.push_str("0.1\n");
    // Exercise bench_input (reads -42) -> convert to f64 & print, then bench_input_f64 (reads -3.25e2) & print
    f_body.extend_from_slice(&[0x10, 6, 0xb9, 0x10, 0, 0x10, 5]); // f64.convert_i64_s(bench_input()); print_f64; newline
    f_body.extend_from_slice(&[0x10, 7, 0x10, 0, 0x10, 5]); // print_f64(bench_input_f64()); newline
    expected_f.push_str("-42.0\n-325.0\n");

    let flt_wasm = wasm_module(&[
        section(
            1,
            &[
                8, 0x60, 1, 0x7c, 0, // 0: (f64)->()
                0x60, 1, 0x7d, 0, // 1: (f32)->()
                0x60, 2, 0x7f, 0x7c, 1, 0x7f, // 2: (i32, f64)->i32
                0x60, 2, 0x7f, 0x7d, 1, 0x7f, // 3: (i32, f32)->i32
                0x60, 2, 0x7f, 0x7f, 0, // 4: (i32, i32)->()
                0x60, 0, 0, // 5: ()->()
                0x60, 0, 1, 0x7e, // 6: ()->i64
                0x60, 0, 1, 0x7c, // 7: ()->f64
            ],
        ),
        section(2, &imp_f),
        section(3, &[1, 5]),
        section(5, &[1, 0x01, 1, 1]),
        section(7, &[1, 6, b'_', b's', b't', b'a', b'r', b't', 0, 8]),
        section(
            10,
            &[1].into_iter()
                .chain(enc_body(&[(1, 0x7f)], &f_body))
                .collect::<Vec<_>>(),
        ),
    ]);
    let flt_path = root.path("goose_floats.wasm");
    let flt_exe = root.path("goose_floats_exe");
    fs::write(&flt_path, &flt_wasm).unwrap();
    let st_flt = Command::new(bin)
        .arg(&flt_path)
        .args(["-o"])
        .arg(&flt_exe)
        .args(["--backend", "x64"])
        .output()
        .unwrap();
    assert!(
        st_flt.status.success(),
        "goose floats build failed: {}",
        String::from_utf8_lossy(&st_flt.stderr)
    );
    let mut child = Command::new(&flt_exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"  -42 \t -3.25e2 \n")
            .unwrap();
    }
    let fout = child.wait_with_output().unwrap();
    assert!(
        fout.status.success(),
        "goose floats run failed: code={:?} stderr={}",
        fout.status.code(),
        String::from_utf8_lossy(&fout.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&fout.stdout), expected_f);
}

#[test]
fn cli_and_cached_build_aarch64_cdylib_and_standalone_exec() {
    let bin = Path::new(env!("CARGO_BIN_EXE_wtime-aot"));
    let root = TempRoot::new("wtime-aot-a64-cli");
    let cache = root.path("cache");
    let ok_wasm = root.path("ok.wasm");
    let direct_so = root.path("direct_a64.so");
    let direct_exe = root.path("direct_a64_exe");

    let ok_bytes = wasm_module(&[
        section(1, &[2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 0, 0]),
        section(3, &[2, 0, 1]),
        section(
            7,
            &[
                2, 3, b'i', b'n', b'c', 0, 0, 6, b'_', b's', b't', b'a', b'r', b't', 0, 1,
            ],
        ),
        section(10, &[2, 7, 0, 0x20, 0, 0x41, 1, 0x6a, 0x0b, 2, 0, 0x0b]),
    ]);
    fs::write(&ok_wasm, &ok_bytes).unwrap();

    // 1. Direct Blot cdylib (--backend aarch64) -> ELF64 ET_DYN EM_AARCH64 (183)
    let st = Command::new(bin)
        .arg(&ok_wasm)
        .args(["-o"])
        .arg(&direct_so)
        .args(["--blot-cdylib", "--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "aarch64 cdylib build failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    let so_bytes = fs::read(&direct_so).unwrap();
    assert_eq!(&so_bytes[0..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([so_bytes[16], so_bytes[17]]), 3); // ET_DYN
    assert_eq!(u16::from_le_bytes([so_bytes[18], so_bytes[19]]), 183); // EM_AARCH64

    // 2. Cached build (--backend aarch64 --blot-cdylib) -> immutable 0o555 + .sha256 + warm hit
    let cold = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        cold.status.success(),
        "aarch64 cached cdylib build failed: {}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let cached_so = PathBuf::from(String::from_utf8_lossy(&cold.stdout).trim());
    assert_eq!(
        fs::symlink_metadata(&cached_so)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    let warm = Command::new(bin)
        .arg("build")
        .arg(&ok_wasm)
        .args(["--cache"])
        .arg(&cache)
        .args(["--blot-cdylib", "--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(warm.status.success());
    assert_eq!(
        PathBuf::from(String::from_utf8_lossy(&warm.stdout).trim()),
        cached_so
    );

    // 3. Direct standalone static executable (--backend aarch64) -> ELF64 ET_EXEC EM_AARCH64
    let st_exec = Command::new(bin)
        .arg(&ok_wasm)
        .args(["-o"])
        .arg(&direct_exe)
        .args(["--backend", "aarch64"])
        .output()
        .unwrap();
    assert!(
        st_exec.status.success(),
        "aarch64 standalone build failed: {}",
        String::from_utf8_lossy(&st_exec.stderr)
    );
    assert_static_elf64_exec(&direct_exe);
    let exe_bytes = fs::read(&direct_exe).unwrap();
    assert_eq!(u16::from_le_bytes([exe_bytes[18], exe_bytes[19]]), 183); // EM_AARCH64
}
