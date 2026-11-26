//! Native Profile-Guided Optimization (`WTPGO1`) format and query engine for
//! `--backend x64` and `--backend aarch64`.
//!
//! Layout of a `WTPGO1` binary profile file (little-endian, strictly bounded):
//! - Header (32 bytes):
//!   - `[0..8]`:   magic `b"WTPGO1\0\0"`
//!   - `[8..12]`:  `n_defs: u32` (defined function count)
//!   - `[12..16]`: `n_ci_sites: u32` (total indirect-call sites counted in decoded Wasm)
//!   - `[16..24]`: `module_hash_lo: u64` (low 8 bytes of Wasm SHA-256)
//!   - `[24..32]`: `flags_reserved: u64` (`0`)
//! - Body:
//!   - `func_counts: [u64; n_defs]` (entry count per defined function)
//!   - `ci_sites: [CiSiteCounter; n_ci_sites]` (24 bytes per site:
//!     `total_hits: u64, top_hits: u64, top_target: u32, _pad: u32`, maintained via
//!     Boyer–Moore 1-slot heavy-hitter tracking so any >=80%-monomorphic site ends
//!     with `top_hits >= 0.6 * total_hits` and exact `top_target`).

use std::fs;
use std::io::Read as _;
use std::path::Path;

use wtime_validate::DecodedModule;

/// 8-byte `WTPGO1` header magic (`0x0000_314f_4750_5457` little-endian).
pub const WTPGO1_MAGIC: [u8; 8] = *b"WTPGO1\0\0";
pub const WTPGO1_MAGIC_U64: u64 = u64::from_le_bytes(WTPGO1_MAGIC);
pub const WTPGO1_HEADER_BYTES: usize = 32;
pub const WTPGO1_CI_SITE_BYTES: usize = 24;
/// Hard cap on native PGO buffer / profile file size (16 MiB).
pub const MAX_PGO_BYTES: usize = 16 << 20;
/// Maximum byte length of an embedded `--pgo-gen` output path.
pub const MAX_PGO_PATH_BYTES: usize = 4096;

/// Minimum total hits at a `call_indirect`/`call_ref` site before speculative devirtualization.
pub const DEVIRT_MIN_TOTAL_HITS: u64 = 64;
/// Minimum Boyer–Moore `top_hits / total_hits` ratio (`6 / 10` guarantees true frequency `>= 80%`).
pub const DEVIRT_TOP_NUM: u64 = 6;
pub const DEVIRT_TOP_DEN: u64 = 10;

/// Minimum dynamic calls for a defined function to be considered hot for PGO inlining.
pub const HOT_FUNC_MIN_CALLS: u64 = 64;

/// Per-`call_indirect`/`call_ref` Boyer–Moore heavy-hitter summary loaded from `WTPGO1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CiSiteProfile {
    pub total_hits: u64,
    pub top_hits: u64,
    pub top_target: u32,
}

/// Verified native PGO profile loaded via `--pgo-use`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgoProfile {
    pub n_defs: u32,
    pub n_ci_sites: u32,
    pub module_hash_lo: u64,
    pub func_counts: Vec<u64>,
    pub ci_sites: Vec<CiSiteProfile>,
    pub total_func_calls: u64,
    pub hot_threshold: u64,
    /// SHA-256 of the raw `WTPGO1` file bytes (bound into build & incremental cache keys).
    pub digest: [u8; 32],
}

/// Counts total `call_indirect` and `call_ref` instructions across all defined functions in
/// `decoded`, providing a deterministic compile-time upper bound on reachable IR `Op::CallIndirect`
/// sites shared identically by `--pgo-gen` and `--pgo-use`.
pub fn count_wasm_ci_sites(decoded: &DecodedModule) -> Result<u32, String> {
    let mut count: usize = 0;
    for body in &decoded.code {
        for ins in &body.instructions {
            if matches!(ins.opcode, 0x11 | 0x14) {
                count = count
                    .checked_add(1)
                    .ok_or_else(|| "pgo: indirect call site count overflow".to_string())?;
            }
        }
    }
    u32::try_from(count).map_err(|_| "pgo: indirect call site count exceeds u32".to_string())
}

/// Computes the exact `WTPGO1` payload size (`32 + n_defs*8 + n_ci_sites*24`) and page-aligned
/// `mmap` allocation size, enforcing `MAX_PGO_BYTES`.
pub fn compute_pgo_buffer_sizes(n_defs: u32, n_ci_sites: u32) -> Result<(usize, usize), String> {
    let fn_bytes = (n_defs as usize)
        .checked_mul(8)
        .ok_or("pgo: func counter table size overflow")?;
    let ci_bytes = (n_ci_sites as usize)
        .checked_mul(WTPGO1_CI_SITE_BYTES)
        .ok_or("pgo: indirect call counter table size overflow")?;
    let payload = WTPGO1_HEADER_BYTES
        .checked_add(fn_bytes)
        .and_then(|v| v.checked_add(ci_bytes))
        .ok_or("pgo: profile buffer size overflow")?;
    let alloc = payload
        .checked_add(4095)
        .map(|v| v & !4095)
        .ok_or("pgo: profile alloc size overflow")?;
    if alloc > MAX_PGO_BYTES {
        return Err(format!(
            "pgo: profile buffer ({alloc} B) exceeds cap {MAX_PGO_BYTES} B"
        ));
    }
    Ok((payload, alloc))
}

/// Computes the low 8 bytes of the Wasm module SHA-256 bound into `WTPGO1`.
pub fn module_hash_lo(wasm: &[u8]) -> u64 {
    let digest =
        crate::sha256::parallel_merkle_digest(wasm).unwrap_or_else(|| crate::sha256::digest(wasm));
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

/// Validates a `--pgo-gen` output path before embedding it into native `.rodata`.
pub fn validate_pgo_gen_path(path: &Path) -> Result<Vec<u8>, String> {
    let s = path
        .to_str()
        .ok_or_else(|| "pgo: --pgo-gen path must be valid UTF-8".to_string())?;
    if s.is_empty() {
        return Err("pgo: --pgo-gen path must not be empty".to_string());
    }
    let bytes = s.as_bytes();
    if bytes.contains(&0) {
        return Err("pgo: --pgo-gen path must not contain NUL bytes".to_string());
    }
    if bytes.len() >= MAX_PGO_PATH_BYTES {
        return Err(format!(
            "pgo: --pgo-gen path exceeds {MAX_PGO_PATH_BYTES} bytes"
        ));
    }
    let mut nul = Vec::with_capacity(bytes.len() + 1);
    nul.extend_from_slice(bytes);
    nul.push(0);
    Ok(nul)
}

impl PgoProfile {
    /// Reads and validates a `WTPGO1` profile from disk (regular file, bounded size).
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let mut file = fs::File::open(path)
            .map_err(|e| format!("pgo: failed to open {}: {e}", path.display()))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("pgo: failed to stat {}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("pgo: {} must be a regular file", path.display()));
        }
        if meta.len() as usize > MAX_PGO_BYTES {
            return Err(format!(
                "pgo: {} ({} B) exceeds cap {MAX_PGO_BYTES} B",
                path.display(),
                meta.len()
            ));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.by_ref()
            .take((MAX_PGO_BYTES as u64) + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("pgo: failed to read {}: {e}", path.display()))?;
        if bytes.len() > MAX_PGO_BYTES {
            return Err(format!(
                "pgo: {} exceeds cap {MAX_PGO_BYTES} B",
                path.display()
            ));
        }
        Self::parse(&bytes)
    }

    /// Parses and validates raw `WTPGO1` bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < WTPGO1_HEADER_BYTES {
            return Err(format!(
                "pgo: truncated WTPGO1 header ({} B < {WTPGO1_HEADER_BYTES} B)",
                bytes.len()
            ));
        }
        if bytes[0..8] != WTPGO1_MAGIC {
            return Err("pgo: invalid WTPGO1 magic header".to_string());
        }
        let n_defs = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let n_ci_sites = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let module_hash_lo = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let flags_reserved = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        if flags_reserved != 0 {
            return Err(format!(
                "pgo: unsupported WTPGO1 flags 0x{flags_reserved:x}"
            ));
        }
        let (expected_len, _) = compute_pgo_buffer_sizes(n_defs, n_ci_sites)?;
        if bytes.len() != expected_len {
            return Err(format!(
                "pgo: WTPGO1 size mismatch (got {} B, expected {expected_len} B for n_defs={n_defs}, n_ci_sites={n_ci_sites})",
                bytes.len()
            ));
        }

        let mut func_counts = Vec::with_capacity(n_defs as usize);
        let mut off = WTPGO1_HEADER_BYTES;
        let mut total_func_calls: u64 = 0;
        for _ in 0..n_defs {
            let c = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
            func_counts.push(c);
            total_func_calls = total_func_calls.saturating_add(c);
            off += 8;
        }
        let mut ci_sites = Vec::with_capacity(n_ci_sites as usize);
        for _ in 0..n_ci_sites {
            let total_hits = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
            let top_hits = u64::from_le_bytes(bytes[off + 8..off + 16].try_into().unwrap());
            let top_target = u32::from_le_bytes(bytes[off + 16..off + 20].try_into().unwrap());
            ci_sites.push(CiSiteProfile {
                total_hits,
                top_hits,
                top_target,
            });
            off += WTPGO1_CI_SITE_BYTES;
        }
        let hot_threshold = HOT_FUNC_MIN_CALLS.max(total_func_calls / 200);
        let digest = crate::sha256::digest(bytes);
        Ok(Self {
            n_defs,
            n_ci_sites,
            module_hash_lo,
            func_counts,
            ci_sites,
            total_func_calls,
            hot_threshold,
            digest,
        })
    }

    /// Serializes this profile into canonical `WTPGO1` bytes (used by tests & tools).
    #[cfg(test)]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            WTPGO1_HEADER_BYTES
                + self.func_counts.len() * 8
                + self.ci_sites.len() * WTPGO1_CI_SITE_BYTES,
        );
        out.extend_from_slice(&WTPGO1_MAGIC);
        out.extend_from_slice(&self.n_defs.to_le_bytes());
        out.extend_from_slice(&self.n_ci_sites.to_le_bytes());
        out.extend_from_slice(&self.module_hash_lo.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        for &c in &self.func_counts {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for s in &self.ci_sites {
            out.extend_from_slice(&s.total_hits.to_le_bytes());
            out.extend_from_slice(&s.top_hits.to_le_bytes());
            out.extend_from_slice(&s.top_target.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out
    }

    /// Verifies that this profile matches the current module's structure and digest.
    pub fn verify_matches_module(
        &self,
        expected_n_defs: u32,
        expected_n_ci_sites: u32,
        expected_module_hash_lo: u64,
    ) -> Result<(), String> {
        if self.module_hash_lo != expected_module_hash_lo {
            return Err(format!(
                "pgo: profile module hash 0x{:016x} does not match wasm module 0x{:016x}",
                self.module_hash_lo, expected_module_hash_lo
            ));
        }
        if self.n_defs != expected_n_defs {
            return Err(format!(
                "pgo: profile n_defs ({}) does not match wasm module ({expected_n_defs})",
                self.n_defs
            ));
        }
        if self.n_ci_sites != expected_n_ci_sites {
            return Err(format!(
                "pgo: profile n_ci_sites ({}) does not match wasm module ({expected_n_ci_sites})",
                self.n_ci_sites
            ));
        }
        Ok(())
    }

    /// Returns `true` iff defined function `bi` is hot enough for expanded PGO inlining budgets.
    #[inline]
    pub fn is_hot_func(&self, bi: usize) -> bool {
        self.func_counts
            .get(bi)
            .copied()
            .is_some_and(|c| c >= self.hot_threshold)
    }

    /// Returns `true` iff the profile has meaningful samples and defined function `bi` had 0 calls.
    #[inline]
    pub fn is_cold_func(&self, bi: usize) -> bool {
        self.total_func_calls >= HOT_FUNC_MIN_CALLS
            && self.func_counts.get(bi).copied().unwrap_or(0) == 0
    }

    /// Returns the dominant target function index for indirect-call site `site_id` if the site
    /// is hot (`total_hits >= 64`) and strongly monomorphic (`>= 80%` true frequency, proven by
    /// Boyer–Moore `top_hits * 10 >= total_hits * 6`).
    #[inline]
    pub fn hot_devirt_target(&self, site_id: usize) -> Option<u32> {
        let s = *self.ci_sites.get(site_id)?;
        if s.total_hits < DEVIRT_MIN_TOTAL_HITS || s.top_hits == 0 || s.top_hits > s.total_hits {
            return None;
        }
        if u128::from(s.top_hits) * u128::from(DEVIRT_TOP_DEN)
            >= u128::from(s.total_hits) * u128::from(DEVIRT_TOP_NUM)
        {
            Some(s.top_target)
        } else {
            None
        }
    }

    /// Returns a deterministic permutation of `0..n_defs` placing the hottest functions first in
    /// `.text` (for I-cache locality) and sinking cold functions to the end.
    pub fn text_layout_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.n_defs as usize).collect();
        order.sort_unstable_by(|&a, &b| {
            let ca = self.func_counts.get(a).copied().unwrap_or(0);
            let cb = self.func_counts.get(b).copied().unwrap_or(0);
            cb.cmp(&ca).then_with(|| a.cmp(&b))
        });
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wtpgo1_roundtrip_thresholds_and_validation() {
        let raw = PgoProfile {
            n_defs: 3,
            n_ci_sites: 3,
            module_hash_lo: 0x0123_4567_89ab_cdef,
            func_counts: vec![0, 5000, 120],
            ci_sites: vec![
                // 90% monomorphic (Boyer-Moore: 900 - 100 = 800 / 1000 >= 0.6) -> devirt!
                CiSiteProfile {
                    total_hits: 1000,
                    top_hits: 800,
                    top_target: 2,
                },
                // 50/50 polymorphic (Boyer-Moore ~ 0) -> do NOT devirt!
                CiSiteProfile {
                    total_hits: 1000,
                    top_hits: 10,
                    top_target: 1,
                },
                // Too few samples -> do NOT devirt!
                CiSiteProfile {
                    total_hits: 20,
                    top_hits: 20,
                    top_target: 2,
                },
            ],
            total_func_calls: 0,
            hot_threshold: 0,
            digest: [0; 32],
        }
        .to_bytes();

        let p = PgoProfile::parse(&raw).unwrap();
        p.verify_matches_module(3, 3, 0x0123_4567_89ab_cdef)
            .unwrap();
        assert!(p.verify_matches_module(3, 3, 0xdead_beef).is_err());
        assert!(
            p.verify_matches_module(2, 3, 0x0123_4567_89ab_cdef)
                .is_err()
        );
        assert!(p.is_cold_func(0));
        assert!(p.is_hot_func(1));
        assert!(p.is_hot_func(2));
        assert_eq!(p.hot_devirt_target(0), Some(2));
        assert_eq!(p.hot_devirt_target(1), None);
        assert_eq!(p.hot_devirt_target(2), None);
        assert_eq!(p.text_layout_order(), vec![1, 2, 0]);

        // Bad magic & truncated files must fail cleanly.
        let mut bad = raw.clone();
        bad[0] = b'X';
        assert!(PgoProfile::parse(&bad).is_err());
        assert!(PgoProfile::parse(&raw[..16]).is_err());
        assert!(PgoProfile::parse(&raw[..raw.len() - 1]).is_err());
    }
}
