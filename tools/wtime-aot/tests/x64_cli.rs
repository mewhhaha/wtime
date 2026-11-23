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
    // Admission-rejected module (memory64 -> exit code 4!).
    let rej_bytes = wasm_module(&[section(5, &[1, 0x04, 1])]);
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
    let resolved_flag =
        if std::is_x86_feature_detected!("lzcnt") && std::is_x86_feature_detected!("bmi1") {
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
