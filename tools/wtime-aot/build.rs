#![forbid(unsafe_code)]

//! Computes a deterministic compile-time SHA-256 fingerprint of `wtime-aot`, its core
//! decoder/validator dependencies, and active build configuration so runtime `wtime-aot build`
//! never has to read and re-hash its own ~4 MiB executable on every invocation.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0u8; 64],
            block_len: 0,
            length: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.block_len > 0 {
            let take = (64 - self.block_len).min(bytes.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&bytes[..take]);
            self.block_len += take;
            bytes = &bytes[take..];
            if self.block_len == 64 {
                let b = self.block;
                self.compress(&b);
                self.block_len = 0;
            }
        }
        let (chunks, rem) = bytes.as_chunks::<64>();
        for chunk in chunks {
            self.compress(chunk);
        }
        if !rem.is_empty() {
            self.block[..rem.len()].copy_from_slice(rem);
            self.block_len = rem.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..64].fill(0);
            let b = self.block;
            self.compress(&b);
            self.block_len = 0;
        }
        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&bits.to_be_bytes());
        let b = self.block;
        self.compress(&b);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (t, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[t] = u32::from_be_bytes(*chunk);
        }
        for t in 16..64 {
            let wm15 = w[t - 15];
            let wm2 = w[t - 2];
            let s0 = wm15.rotate_right(7) ^ wm15.rotate_right(18) ^ (wm15 >> 3);
            let s1 = wm2.rotate_right(17) ^ wm2.rotate_right(19) ^ (wm2 >> 10);
            w[t] = w[t - 16]
                .wrapping_add(s0)
                .wrapping_add(w[t - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(s0.wrapping_add(maj));
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn hash_field(h: &mut Sha256, label: &str, data: &[u8]) {
    h.update(&(label.len() as u64).to_le_bytes());
    h.update(label.as_bytes());
    h.update(&(data.len() as u64).to_le_bytes());
    h.update(data);
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_files(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    println!("cargo:rerun-if-env-changed=WTIME_AOT_BUILD_SALT");

    let mut files: Vec<PathBuf> = Vec::new();
    let single_files = [
        manifest_dir.join("Cargo.toml"),
        manifest_dir.join("Cargo.lock"),
        manifest_dir.join("build.rs"),
        manifest_dir.join("rt/Cargo.toml"),
        manifest_dir.join("../../crates/wtime-types/Cargo.toml"),
        manifest_dir.join("../../crates/wtime-decode/Cargo.toml"),
        manifest_dir.join("../../crates/wtime-validate/Cargo.toml"),
    ];
    for p in &single_files {
        if p.is_file() {
            files.push(p.clone());
        }
    }
    let dirs = [
        manifest_dir.join("src"),
        manifest_dir.join("rt/src"),
        manifest_dir.join("../../crates/wtime-types/src"),
        manifest_dir.join("../../crates/wtime-decode/src"),
        manifest_dir.join("../../crates/wtime-validate/src"),
    ];
    for d in &dirs {
        collect_files(d, &mut files);
    }
    files.sort();
    files.dedup();

    let mut hasher = Sha256::new();
    hash_field(&mut hasher, "wtime-aot-generator-fingerprint-v1", &[]);
    for var in [
        "PROFILE",
        "OPT_LEVEL",
        "TARGET",
        "DEBUG",
        "CARGO_CFG_TARGET_FEATURE",
        "WTIME_AOT_BUILD_SALT",
    ] {
        let val = env::var(var).unwrap_or_default();
        hash_field(&mut hasher, var, val.as_bytes());
    }
    if let Ok(rustc) = env::var("RUSTC")
        && let Ok(out) = Command::new(rustc).arg("-vV").output()
        && out.status.success()
    {
        hash_field(&mut hasher, "rustc -vV", &out.stdout);
    }
    for p in &files {
        println!("cargo:rerun-if-changed={}", p.display());
        let rel = p
            .strip_prefix(&manifest_dir)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string_lossy().into_owned());
        let bytes = fs::read(p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
        hash_field(&mut hasher, &rel, &bytes);
    }
    let digest = hasher.finish();

    let mut code = String::from("pub const GENERATOR_BUILD_SHA256: [u8; 32] = [\n    ");
    for (i, b) in digest.iter().enumerate() {
        if i > 0 {
            code.push_str(", ");
        }
        code.push_str(&format!("0x{b:02x}"));
    }
    code.push_str("\n];\n");
    fs::write(out_dir.join("generator_fingerprint.rs"), code)
        .expect("writing generator_fingerprint.rs");
}
