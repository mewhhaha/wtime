//! Content-addressed per-function incremental compilation cache for `--backend x64`.
//!
//! Stores per-function pre-link machine code (`code`, `relocs`, unpatched `local_rodata`,
//! resolved jump-table offsets) plus exact interprocedural summaries (inlining cone,
//! own memory/global/table effects, stack/trap elision inputs, and dependency fingerprints)
//! inside a single SHA-256 verified binary file under `<cache>/x64-inc/`.
//!
//! Safety & determinism guarantees:
//! - Strictly `#![forbid(unsafe_code)]` with bounded, checked binary decoding.
//! - Any missing, truncated, corrupted, oversized, or ABI-mismatched cache file silently
//!   falls back to full cold compilation.
//! - Produces bit-for-bit identical `.so` output to a cold compile for every admitted module.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use wtime_types::{FuncType, HeapType, ValType};

use crate::sha256::{Sha256, to_hex};

use super::asm::{Asm, JumpTablePatch, Reloc, RelocTarget};
use super::lower::TargetIsa;
use super::module::{
    MAX_FN_CODE_BYTES, MAX_FN_LOCAL_RODATA_BYTES, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES,
    X64ConstVal, X64Module,
};
use super::opt::{FnElideInput, FnOwnEffects, ModuleEffects};
use super::{CtxLayout, NUM_HELPERS, RodataConsts};

const MAGIC: &[u8; 8] = b"WTX64INC";
const FORMAT_VERSION: u32 = 12;
const DELTA_MAGIC: &[u8; 8] = b"WTX64DLT";
const DELTA_VERSION: u32 = 1;
const DELTA_HEADER_BYTES: usize = 89;
const MAX_DELTA_ENTRIES: usize = 64;
const MAX_DELTA_PAYLOAD_BYTES: usize = 256 << 10; // 256 KiB write threshold before compacting
const MAX_DELTA_READ_BYTES: usize = 1 << 20; // 1 MiB read cap
const PARALLEL_INC_MIN_FUNCS: usize = 256;
const MAX_CACHE_FILE_BYTES: usize = 128 << 20; // 128 MiB cap on read

/// Computes a deterministic SHA-256 Merkle root over 64 KiB chunks in parallel across worker
/// threads using dynamic work-stealing. Uses fallible allocation and `spawn_scoped` with inline
/// fallback so optional cache hashing never aborts.
pub(crate) fn parallel_payload_digest(payload: &[u8]) -> Option<[u8; 32]> {
    crate::sha256::parallel_merkle_digest_with_chunk(b"WTX64INC-ROOT-2", payload, 64 << 10)
}

static NEXT_INC_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn try_vec_with_capacity<T>(cap: usize) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(cap).ok()?;
    Some(v)
}

pub fn try_copy_slice<T: Copy>(src: &[T]) -> Option<Vec<T>> {
    let mut v = try_vec_with_capacity(src.len())?;
    v.extend_from_slice(src);
    Some(v)
}

/// Zero-copy byte buffer for cached machine code and local rodata: either newly emitted
/// (`Owned`) or borrowed by byte range from `IncCacheData::backing` (`Slice`).
#[derive(Clone)]
pub enum CachedBytes {
    Owned(Vec<u8>),
    Slice { start: u32, len: u32 },
}

impl CachedBytes {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            CachedBytes::Owned(v) => v.len(),
            CachedBytes::Slice { len, .. } => *len as usize,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_slice<'a>(&'a self, backing: Option<&'a [u8]>) -> &'a [u8] {
        match self {
            CachedBytes::Owned(v) => v.as_slice(),
            CachedBytes::Slice { start, len } => {
                let s = *start as usize;
                let e = s + (*len as usize);
                &backing.expect("inc_cache backing missing for Slice")[s..e]
            }
        }
    }

    pub fn into_vec(self, backing: Option<&[u8]>) -> Vec<u8> {
        match self {
            CachedBytes::Owned(v) => v,
            CachedBytes::Slice { start, len } => {
                let s = start as usize;
                let e = s + (len as usize);
                backing.expect("inc_cache backing missing for Slice")[s..e].to_vec()
            }
        }
    }
}

#[derive(Clone)]
pub struct CachedFn {
    pub raw_body_hash: [u8; 32],
    pub was_inlinable: bool,
    pub pre_inline_callees: Vec<(u32, u32)>,
    pub inlined_callees: Vec<u32>,
    pub inline_added_insts: u32,
    pub own_effects: FnOwnEffects,
    pub elide_input: FnElideInput,
    pub callee_effects_hash: [u8; 32],
    pub needs_stack_check: bool,
    pub elided_callee_stack_bytes: i32,
    pub callee_trap_hash: [u8; 32],
    /// Pre-relaxation byte count charged to `CompileBudget` during emission.
    pub pre_relax_code_len: u32,
    pub code: CachedBytes,
    pub relocs: Vec<Reloc>,
    pub local_rodata: CachedBytes,
    pub jt_entries: Vec<(u32, u32, u32)>,
    /// Pre-built jump-table metadata so `into_asm` moves in O(1) with zero heap allocations.
    pub jt_labels: Vec<Option<u32>>,
    pub jt_patches: Vec<JumpTablePatch>,
}

impl CachedFn {
    pub fn into_asm(self, backing: Option<&[u8]>) -> Asm {
        let (jt_labels, jt_patches) = if self.jt_labels.is_empty() && !self.jt_entries.is_empty() {
            Asm::try_build_cached_jt(&self.jt_entries).unwrap_or_default()
        } else {
            (self.jt_labels, self.jt_patches)
        };
        Asm::from_cached_parts(
            self.code.into_vec(backing),
            self.pre_relax_code_len,
            self.relocs,
            self.local_rodata.into_vec(backing),
            jt_labels,
            jt_patches,
        )
    }
}

pub struct IncCacheData {
    pub abi_hash: [u8; 32],
    pub global_words: usize,
    pub hit_inline_cap: bool,
    pub funcs: Vec<CachedFn>,
    /// Shared read-only backing buffer for `CachedBytes::Slice` entries loaded from disk.
    pub backing: Option<std::sync::Arc<Vec<u8>>>,
    /// Trailing 32-byte Merkle checksum of the on-disk base `<abi_hash>.bin` this cache descends from.
    pub base_checksum: Option<[u8; 32]>,
    /// Sorted, deduplicated indices modified relative to the on-disk base `<abi_hash>.bin`.
    pub delta_dirty: Vec<usize>,
}

/// Verified raw base `.bin` buffer plus optional matching `.delta` override buffer.
pub struct RawIncCache {
    pub base: Vec<u8>,
    pub delta: Option<Vec<u8>>,
}

fn hash_u32(h: &mut Sha256, v: u32) {
    h.update(&v.to_le_bytes());
}

fn hash_i32(h: &mut Sha256, v: i32) {
    h.update(&v.to_le_bytes());
}

fn hash_u64(h: &mut Sha256, v: u64) {
    h.update(&v.to_le_bytes());
}

fn hash_val_type(h: &mut Sha256, vt: ValType) {
    let tag: u8 = match vt {
        ValType::I32 => 1,
        ValType::I64 => 2,
        ValType::F32 => 3,
        ValType::F64 => 4,
        ValType::V128 => 5,
        ValType::Ref(rt) => {
            h.update(&[6, u8::from(rt.nullable)]);
            match rt.heap {
                HeapType::Func => h.update(&[1]),
                HeapType::Extern => h.update(&[2]),
                other => {
                    h.update(&[3]);
                    let dbg = format!("{other:?}");
                    hash_u32(h, dbg.len() as u32);
                    h.update(dbg.as_bytes());
                }
            }
            return;
        }
    };
    h.update(&[tag]);
}

fn hash_func_type(h: &mut Sha256, ft: &FuncType) {
    hash_u32(h, ft.params.len() as u32);
    for &p in &ft.params {
        hash_val_type(h, p);
    }
    hash_u32(h, ft.results.len() as u32);
    for &r in &ft.results {
        hash_val_type(h, r);
    }
}

/// Computes the exact module-wide ABI & layout fingerprint that every cached function depends on.
#[allow(clippy::too_many_arguments)]
pub fn compute_abi_hash(
    salt: &[u8; 32],
    isa: TargetIsa,
    opt: bool,
    verify: bool,
    spectre_mitigations: bool,
    n_defs: usize,
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(MAGIC);
    hash_u32(&mut h, FORMAT_VERSION);
    h.update(salt);
    h.update(&[
        u8::from(isa.has_abm_bmi1),
        u8::from(isa.has_bmi2),
        u8::from(isa.has_avx2),
        u8::from(opt),
        u8::from(verify),
        u8::from(spectre_mitigations),
        u8::from(module.memory64),
        match module.target {
            super::module::X64Target::BlotCdylib => 0,
            super::module::X64Target::StandaloneExec => 1,
        },
    ]);
    hash_u32(&mut h, module.imports.len() as u32);
    hash_u32(&mut h, n_defs as u32);

    match module.memory {
        None => h.update(&[0]),
        Some((min_p, max_p)) => {
            h.update(&[1]);
            hash_u64(&mut h, min_p);
            hash_u64(&mut h, max_p);
        }
    }
    match module.fixed32 {
        None => h.update(&[0]),
        Some(b) => {
            h.update(&[1]);
            hash_u64(&mut h, b);
        }
    }
    hash_u32(&mut h, module.extra_memories.len() as u32);
    for em in &module.extra_memories {
        hash_u64(&mut h, em.min_pages);
        hash_u64(&mut h, em.max_pages);
        h.update(&[u8::from(em.memory64)]);
    }
    h.update(&[u8::from(module.uses_exceptions)]);
    hash_u32(&mut h, module.tags.len() as u32);
    for &t in &module.tags {
        hash_u32(&mut h, t);
    }
    match layout.exn_state_off {
        None => h.update(&[0]),
        Some(off) => {
            h.update(&[1]);
            hash_i32(&mut h, off);
        }
    }

    hash_u32(&mut h, module.module_types.len() as u32);
    for ft in &module.module_types {
        hash_func_type(&mut h, ft);
    }
    hash_u32(&mut h, module.func_type_idx.len() as u32);
    for &ti in &module.func_type_idx {
        hash_u32(&mut h, ti);
    }

    hash_u32(&mut h, module.globals.len() as u32);
    for (i, &(vt, mutable)) in module.globals.iter().enumerate() {
        hash_val_type(&mut h, vt);
        h.update(&[u8::from(mutable)]);
        hash_i32(&mut h, layout.global_offs[i]);
        match module.global_consts[i] {
            None => h.update(&[0]),
            Some(X64ConstVal::I32(v)) => {
                h.update(&[1]);
                hash_i32(&mut h, v);
            }
            Some(X64ConstVal::I64(v)) => {
                h.update(&[2]);
                hash_u64(&mut h, v as u64);
            }
            Some(X64ConstVal::F32(b)) => {
                h.update(&[3]);
                hash_u32(&mut h, b);
            }
            Some(X64ConstVal::F64(b)) => {
                h.update(&[4]);
                hash_u64(&mut h, b);
            }
            Some(X64ConstVal::RefNull) => h.update(&[5]),
            Some(X64ConstVal::V128(bytes)) => {
                h.update(&[6]);
                h.update(&bytes);
            }
            Some(X64ConstVal::FuncRef(f)) => {
                h.update(&[7]);
                hash_u32(&mut h, f);
            }
        }
    }
    match layout.pinned_global {
        None => h.update(&[0]),
        Some(g) => {
            h.update(&[1]);
            hash_u32(&mut h, g);
        }
    }

    hash_u32(&mut h, layout.table_is64.len() as u32);
    for &t64 in &layout.table_is64 {
        h.update(&[u8::from(t64)]);
    }
    hash_u32(&mut h, layout.extern_tables.len() as u32);
    for (&tbl, &(off, len)) in &layout.extern_tables {
        hash_u32(&mut h, tbl);
        hash_i32(&mut h, off);
        hash_u32(&mut h, len);
    }
    hash_u32(&mut h, layout.dyn_tables.len() as u32);
    for (&tbl, &(off, min_l, max_l)) in &layout.dyn_tables {
        hash_u32(&mut h, tbl);
        hash_i32(&mut h, off);
        hash_u32(&mut h, min_l);
        hash_u32(&mut h, max_l);
    }
    hash_i32(&mut h, layout.data_dropped_off.unwrap_or(-1));
    hash_i32(&mut h, layout.elem_dropped_off.unwrap_or(-1));
    hash_u32(&mut h, layout.func_sig_ro_off);
    hash_u32(&mut h, layout.func_addr_ro_off);

    hash_u32(&mut h, layout.ci_tables.len() as u32);
    for (ci_sig, ci_lay) in module.ci_sigs.iter().zip(&layout.ci_tables) {
        hash_u32(&mut h, ci_sig.table);
        hash_u32(&mut h, ci_sig.type_idx);
        h.update(&[u8::from(ci_lay.is_call_ref), u8::from(ci_lay.table_is64)]);
        hash_u32(&mut h, ci_lay.table_len);
        hash_u32(&mut h, ci_lay.min_slot);
        hash_u32(&mut h, ci_lay.span);
        hash_u32(&mut h, ci_lay.rodata_off);
        hash_i32(&mut h, ci_lay.dyn_table_desc_off);
        hash_u32(&mut h, ci_lay.canon_sig);
        hash_u32(&mut h, ci_sig.targets.len() as u32);
        for &(slot, fi) in &ci_sig.targets {
            hash_u32(&mut h, slot);
            hash_u32(&mut h, fi);
        }
    }
    hash_u32(&mut h, module.ci_index.len() as u32);
    for (&(tbl, ti), &ci_id) in &module.ci_index {
        hash_u32(&mut h, tbl);
        hash_u32(&mut h, ti);
        hash_u32(&mut h, ci_id as u32);
    }

    hash_u32(&mut h, module.exported_funcs.len() as u32);
    for &(_, fi) in &module.exported_funcs {
        hash_u32(&mut h, fi as u32);
    }
    match module.start {
        None => h.update(&[0]),
        Some(fi) => {
            h.update(&[1]);
            hash_u32(&mut h, fi);
        }
    }

    // Shared `.rodata` constant offsets referenced via `RelocTarget::Rodata`.
    for off in [
        ro.f32_abs,
        ro.f32_neg,
        ro.f64_abs,
        ro.f64_neg,
        ro.f32_qnan,
        ro.f64_qnan,
        ro.mxcsr_wasm,
        ro.f32_neg_one,
        ro.f32_neg_2p31,
        ro.f32_2p31,
        ro.f32_2p32,
        ro.f32_neg_2p63,
        ro.f32_2p63,
        ro.f32_2p64,
        ro.f64_neg_one,
        ro.f64_neg_2p31_m1,
        ro.f64_2p31,
        ro.f64_2p32,
        ro.f64_neg_2p63,
        ro.f64_2p63,
        ro.f64_2p64,
    ] {
        hash_u32(&mut h, off);
    }

    h.finish()
}

/// Computes the exact dependency digest of transitive `ModuleEffects` for a function's `eff_callees`.
pub fn hash_callee_effects(callees: &[u32], effects: &ModuleEffects) -> [u8; 32] {
    let mut h = Sha256::new();
    hash_u32(&mut h, callees.len() as u32);
    for &fi in callees {
        let f = fi as usize;
        hash_u32(&mut h, fi);
        let flags = u8::from(effects.writes_mem[f])
            | (u8::from(effects.grows_mem[f]) << 1)
            | (u8::from(effects.writes_all_globals[f]) << 2)
            | (u8::from(effects.writes_all_tables[f]) << 3);
        h.update(&[flags]);
        for &w in effects.written_globals_slice(f) {
            hash_u64(&mut h, w);
        }
        hash_u64(&mut h, effects.written_tables[f]);
    }
    h.finish()
}

/// Computes the exact dependency digest of `fn_can_trap` for a function's direct defined `elide_callees`.
pub fn hash_callee_traps(
    elide_callees: &[u32],
    n_imports: usize,
    fn_can_trap: &[bool],
) -> [u8; 32] {
    let mut h = Sha256::new();
    hash_u32(&mut h, elide_callees.len() as u32);
    for &c in elide_callees {
        hash_u32(&mut h, c);
        h.update(&[u8::from(fn_can_trap[n_imports + c as usize])]);
    }
    h.finish()
}

fn cache_file_name(abi_hash: &[u8; 32]) -> String {
    let hex = to_hex(abi_hash);
    format!("{}.bin", &hex[..32])
}

fn delta_file_name(abi_hash: &[u8; 32]) -> String {
    let hex = to_hex(abi_hash);
    format!("{}.delta", &hex[..32])
}

/// Fast O(1) existence check for `<cache_dir>/x64-inc/<abi_hash>.bin` so cold builds avoid
/// hashing all function bodies synchronously before validation/compilation.
pub fn has_inc_cache(cache_dir: &Path, abi_hash: &[u8; 32]) -> bool {
    let path = cache_dir.join("x64-inc").join(cache_file_name(abi_hash));
    fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file())
}

#[cfg(unix)]
fn unix_nofollow_flag() -> i32 {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "x86", target_arch = "riscv64")
    ))]
    {
        0o400_000
    }
    #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
    {
        0o100_000
    }
    #[cfg(target_os = "macos")]
    {
        0x0100
    }
    #[cfg(not(any(
        all(
            target_os = "linux",
            any(
                target_arch = "x86_64",
                target_arch = "x86",
                target_arch = "riscv64",
                target_arch = "aarch64",
                target_arch = "arm"
            )
        ),
        target_os = "macos"
    )))]
    {
        0
    }
}

/// Opens `path` strictly as an owner-private, non-symlink regular file without blocking on FIFOs
/// and reads at most `MAX_CACHE_FILE_BYTES` bytes.
pub(crate) fn read_private_cache_file(path: &Path) -> Option<Vec<u8>> {
    let smeta = fs::symlink_metadata(path).ok()?;
    if smeta.file_type().is_symlink() || !smeta.file_type().is_file() {
        return None;
    }
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        #[cfg(target_os = "linux")]
        const O_NONBLOCK_NOCTTY: i32 = 0o4000 | 0o400;
        #[cfg(not(target_os = "linux"))]
        const O_NONBLOCK_NOCTTY: i32 = 0x0004 | 0x20000;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK_NOCTTY | unix_nofollow_flag())
            .open(path)
            .ok()?
    };
    #[cfg(not(unix))]
    let file = fs::File::open(path).ok()?;

    let fmeta = file.metadata().ok()?;
    if !fmeta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let euid = crate::build::current_euid().ok()?;
        if fmeta.dev() != smeta.dev()
            || fmeta.ino() != smeta.ino()
            || fmeta.uid() != euid
            || (fmeta.permissions().mode() & 0o022) != 0
        {
            return None;
        }
    }
    if fmeta.len() < 80 || fmeta.len() > MAX_CACHE_FILE_BYTES as u64 {
        return None;
    }
    let init_len = (fmeta.len() as usize).min(MAX_CACHE_FILE_BYTES);
    let mut bytes = try_vec_with_capacity(init_len)?;
    bytes.resize(init_len, 0u8);
    let mut reader = file.take((MAX_CACHE_FILE_BYTES + 1) as u64);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let end = pos.saturating_add(1 << 20).min(bytes.len());
        match reader.read(&mut bytes[pos..end]) {
            Ok(0) => {
                bytes.truncate(pos);
                return (bytes.len() >= 80 && bytes.len() <= MAX_CACHE_FILE_BYTES).then_some(bytes);
            }
            Ok(n) => pos += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    let mut chunk = [0u8; 65_536];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let new_len = bytes.len().saturating_add(n);
                if new_len > MAX_CACHE_FILE_BYTES {
                    return None;
                }
                if bytes.capacity() < new_len {
                    let target = bytes
                        .capacity()
                        .saturating_mul(2)
                        .max(new_len)
                        .min(MAX_CACHE_FILE_BYTES);
                    bytes.try_reserve_exact(target - bytes.len()).ok()?;
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    if bytes.len() < 80 || bytes.len() > MAX_CACHE_FILE_BYTES {
        return None;
    }
    Some(bytes)
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn check_remaining(&self, count: usize, item_bytes: usize) -> Option<()> {
        let need = count.checked_mul(item_bytes)?;
        if need > self.buf.len().saturating_sub(self.pos) {
            return None;
        }
        Some(())
    }

    fn read_bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    fn read_u8(&mut self) -> Option<u8> {
        Some(self.read_bytes(1)?[0])
    }

    fn read_u32(&mut self) -> Option<u32> {
        let s = self.read_bytes(4)?;
        Some(u32::from_le_bytes(s.try_into().ok()?))
    }

    fn read_i32(&mut self) -> Option<i32> {
        let s = self.read_bytes(4)?;
        Some(i32::from_le_bytes(s.try_into().ok()?))
    }

    fn read_u64(&mut self) -> Option<u64> {
        let s = self.read_bytes(8)?;
        Some(u64::from_le_bytes(s.try_into().ok()?))
    }

    fn read_hash(&mut self) -> Option<[u8; 32]> {
        let s = self.read_bytes(32)?;
        s.try_into().ok()
    }

    fn read_u32_vec(&mut self, max_len: usize) -> Option<Vec<u32>> {
        let len = self.read_u32()? as usize;
        if len > max_len {
            return None;
        }
        let raw = self.read_bytes(len.checked_mul(4)?)?;
        let mut out = try_vec_with_capacity(len)?;
        for &chunk in raw.as_chunks::<4>().0 {
            out.push(u32::from_le_bytes(chunk));
        }
        Some(out)
    }
}

/// Decodes and validates a single `CachedFn` entry slice, returning `(entry, code_len,
/// pre_relax_code_len, ro_len)`. Requires exact slice consumption (`r.pos == slice.len()`).
/// When `entry_abs_base` is `Some`, borrows `code` and `local_rodata` zero-copy via
/// `CachedBytes::Slice` into `backing`; when `None` (delta override), copies into `Owned`.
fn decode_cached_fn(
    slice: &[u8],
    entry_abs_base: Option<usize>,
    expected_bh: &[u8; 32],
    n_defs: usize,
    n_total: usize,
    global_words: usize,
    expected_rodata_len: usize,
) -> Option<(CachedFn, usize, usize, usize)> {
    let mut r = Reader { buf: slice, pos: 0 };
    let raw_body_hash = r.read_hash()?;
    let is_changed = &raw_body_hash != expected_bh;
    let flags = r.read_u8()?;
    let was_inlinable = (flags & 1) != 0;
    let writes_mem = (flags & 2) != 0;
    let grows_mem = (flags & 4) != 0;
    let writes_all_globals = (flags & 8) != 0;
    let writes_all_tables = (flags & 16) != 0;
    let local_may_trap = (flags & 32) != 0;
    let calls_import_or_indirect = (flags & 64) != 0;
    let needs_stack_check = (flags & 128) != 0;

    let n_pre = r.read_u32()? as usize;
    if n_pre > n_defs {
        return None;
    }
    r.check_remaining(n_pre, 8)?;
    let mut pre_inline_callees = try_vec_with_capacity(n_pre)?;
    for _ in 0..n_pre {
        let c = r.read_u32()?;
        let cnt = r.read_u32()?;
        if (c as usize) >= n_defs {
            return None;
        }
        pre_inline_callees.push((c, cnt));
    }

    let inlined_callees = r.read_u32_vec(n_defs)?;
    if inlined_callees.iter().any(|&c| (c as usize) >= n_defs) {
        return None;
    }
    let inline_added_insts = r.read_u32()?;
    let eff_callees = r.read_u32_vec(n_total)?;
    if eff_callees.iter().any(|&fi| (fi as usize) >= n_total) {
        return None;
    }
    let elide_callees = r.read_u32_vec(n_defs)?;
    if elide_callees.iter().any(|&c| (c as usize) >= n_defs) {
        return None;
    }

    r.check_remaining(global_words, 8)?;
    let mut written_globals = try_vec_with_capacity(global_words)?;
    for _ in 0..global_words {
        written_globals.push(r.read_u64()?);
    }
    let written_tables = r.read_u64()?;
    let max_helper_bytes = r.read_i32()?;
    let own_stack_bytes = r.read_i32()?;
    let elided_callee_stack_bytes = r.read_i32()?;
    if max_helper_bytes < 0 || own_stack_bytes < 0 || elided_callee_stack_bytes < 0 {
        return None;
    }
    let callee_effects_hash = r.read_hash()?;
    let callee_trap_hash = r.read_hash()?;

    let pre_relax_code_len = r.read_u32()?;
    let code_len = r.read_u32()? as usize;
    if code_len > MAX_FN_CODE_BYTES
        || (pre_relax_code_len as usize) < code_len
        || (pre_relax_code_len as usize) > MAX_FN_CODE_BYTES
    {
        return None;
    }
    let code_rel_pos = r.pos;
    let code_slice = r.read_bytes(code_len)?;
    let code = if is_changed || code_len == 0 {
        CachedBytes::Owned(Vec::new())
    } else if let Some(abs_base) = entry_abs_base {
        let abs_start = u32::try_from(abs_base.checked_add(code_rel_pos)?).ok()?;
        let len_u32 = u32::try_from(code_len).ok()?;
        CachedBytes::Slice {
            start: abs_start,
            len: len_u32,
        }
    } else {
        CachedBytes::Owned(try_copy_slice(code_slice)?)
    };

    let n_relocs = r.read_u32()? as usize;
    if n_relocs > code_len.saturating_add(16) {
        return None;
    }
    r.check_remaining(n_relocs, 13)?;
    let mut relocs = if is_changed {
        Vec::new()
    } else {
        try_vec_with_capacity(n_relocs)?
    };
    let mut max_local_rodata_off: Option<u32> = None;
    for _ in 0..n_relocs {
        let patch_off = r.read_u32()?;
        let next_ip_off = r.read_u32()?;
        let kind = r.read_u8()?;
        let val = r.read_u32()?;
        let patch_end = (patch_off as usize).checked_add(4)?;
        if patch_end > code_len || (next_ip_off as usize) > code_len {
            return None;
        }
        let target = match kind {
            0 => {
                if (val as usize) >= n_total {
                    return None;
                }
                RelocTarget::Func(val)
            }
            1 => {
                if (val as usize) >= NUM_HELPERS {
                    return None;
                }
                RelocTarget::Helper(val)
            }
            2 => {
                if expected_rodata_len == 0 || (val as usize) >= expected_rodata_len {
                    return None;
                }
                RelocTarget::Rodata(val)
            }
            3 => {
                max_local_rodata_off = Some(max_local_rodata_off.map_or(val, |m| m.max(val)));
                RelocTarget::LocalRodata(val)
            }
            _ => return None,
        };
        if !is_changed {
            relocs.push(Reloc {
                patch_off,
                next_ip_off,
                target,
            });
        }
    }

    let ro_len = r.read_u32()? as usize;
    if ro_len > MAX_FN_LOCAL_RODATA_BYTES {
        return None;
    }
    let ro_rel_pos = r.pos;
    let ro_slice = r.read_bytes(ro_len)?;
    if let Some(max_off) = max_local_rodata_off
        && (ro_len == 0 || (max_off as usize) >= ro_len)
    {
        return None;
    }
    let local_rodata = if is_changed || ro_len == 0 {
        CachedBytes::Owned(Vec::new())
    } else if let Some(abs_base) = entry_abs_base {
        let abs_start = u32::try_from(abs_base.checked_add(ro_rel_pos)?).ok()?;
        let len_u32 = u32::try_from(ro_len).ok()?;
        CachedBytes::Slice {
            start: abs_start,
            len: len_u32,
        }
    } else {
        CachedBytes::Owned(try_copy_slice(ro_slice)?)
    };

    let n_jt = r.read_u32()? as usize;
    if n_jt > ro_len / 4 + 1 {
        return None;
    }
    r.check_remaining(n_jt, 12)?;
    let mut jt_entries = if is_changed {
        Vec::new()
    } else {
        try_vec_with_capacity(n_jt)?
    };
    for _ in 0..n_jt {
        let ro_off = r.read_u32()?;
        let tbl_off = r.read_u32()?;
        let lbl_off = r.read_u32()?;
        let ro_end = (ro_off as usize).checked_add(4)?;
        if ro_end > ro_len || (tbl_off as usize) > ro_len || (lbl_off as usize) > code_len {
            return None;
        }
        if !is_changed {
            jt_entries.push((ro_off, tbl_off, lbl_off));
        }
    }

    if r.pos != slice.len() {
        return None;
    }

    // Leave `jt_labels`/`jt_patches` empty on load: the normal `link_cache` path patches jump
    // tables directly from `jt_entries`, and `CachedFn::into_asm` builds them lazily if needed.
    let (jt_labels, jt_patches) = (Vec::new(), Vec::new());

    Some((
        CachedFn {
            raw_body_hash,
            was_inlinable,
            pre_inline_callees,
            inlined_callees,
            inline_added_insts,
            own_effects: FnOwnEffects {
                writes_mem,
                grows_mem,
                writes_all_globals,
                written_globals,
                writes_all_tables,
                written_tables,
                direct_callees: eff_callees,
            },
            elide_input: FnElideInput {
                own_stack_bytes,
                local_may_trap,
                calls_import_or_indirect,
                max_helper_bytes,
                direct_def_callees: elide_callees,
            },
            callee_effects_hash,
            needs_stack_check,
            elided_callee_stack_bytes,
            callee_trap_hash,
            pre_relax_code_len,
            code,
            relocs,
            local_rodata,
            jt_entries,
            jt_labels,
            jt_patches,
        },
        code_len,
        pre_relax_code_len as usize,
        ro_len,
    ))
}

/// Reads and Merkle-verifies raw incremental cache file bytes (plus optional matching `.delta`
/// sidecar), independent of `body_hashes` so callers can overlap I/O + verification with body hashing.
pub fn load_inc_cache_raw(cache_dir: &Path, expected_abi_hash: &[u8; 32]) -> Option<RawIncCache> {
    let inc_dir = cache_dir.join("x64-inc");
    // Do not create `x64-inc/` on a read miss; validate it strictly when present.
    let dir_meta = fs::symlink_metadata(&inc_dir).ok()?;
    if dir_meta.file_type().is_symlink() || !dir_meta.file_type().is_dir() {
        return None;
    }
    let canon_dir = crate::build::ensure_private_dir(&inc_dir).ok()?;
    let path = canon_dir.join(cache_file_name(expected_abi_hash));
    let bytes = read_private_cache_file(&path)?;
    if bytes.len() < 80 {
        return None;
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - 32);
    let expected_sum = parallel_payload_digest(payload)?;
    if trailer != expected_sum {
        return None;
    }
    // Opportunistically load and cryptographically verify `<abi_hash>.delta` bound to `expected_sum`.
    let delta_path = canon_dir.join(delta_file_name(expected_abi_hash));
    let delta = read_private_cache_file(&delta_path).and_then(|dbytes| {
        if dbytes.len() < DELTA_HEADER_BYTES + 32 || dbytes.len() > MAX_DELTA_READ_BYTES {
            return None;
        }
        let (d_payload, d_trailer) = dbytes.split_at(dbytes.len() - 32);
        if &d_payload[..8] != DELTA_MAGIC
            || u32::from_le_bytes(d_payload[8..12].try_into().ok()?) != DELTA_VERSION
            || &d_payload[12..44] != expected_abi_hash
            || &d_payload[44..76] != expected_sum.as_slice()
        {
            return None;
        }
        let actual = crate::sha256::digest(d_payload);
        (d_trailer == actual.as_slice()).then_some(dbytes)
    });
    Some(RawIncCache { base: bytes, delta })
}

/// Loads and integrity-checks (SHA-256 Merkle trailer) the incremental function cache for `abi_hash`.
/// Uses fallible allocations, offset-table early body-hash filtering, and parallel entry decoding
/// (up to 16 threads for large modules) so low memory or >50% edited modules cleanly fall back.
#[cfg(test)]
pub fn load_inc_cache(
    cache_dir: &Path,
    expected_abi_hash: &[u8; 32],
    expected_n_imports: usize,
    expected_n_defs: usize,
    expected_global_words: usize,
    expected_rodata_len: usize,
    expected_body_hashes: &[[u8; 32]],
) -> Option<IncCacheData> {
    if expected_body_hashes.len() != expected_n_defs {
        return None;
    }
    let raw = load_inc_cache_raw(cache_dir, expected_abi_hash)?;
    decode_verified_inc_cache(
        std::sync::Arc::new(raw.base),
        raw.delta.as_deref(),
        expected_abi_hash,
        expected_n_imports,
        expected_n_defs,
        expected_global_words,
        expected_rodata_len,
        expected_body_hashes,
    )
}

/// Parses verified `.delta` sidecar entries into sorted `(bi, record_slice)` overrides.
fn parse_verified_delta<'a>(
    delta_bytes: &'a [u8],
    expected_abi_hash: &[u8; 32],
    expected_base_checksum: &[u8; 32],
    expected_n_defs: usize,
    expected_global_words: usize,
    min_fn_wire: usize,
) -> Option<Vec<(usize, &'a [u8])>> {
    if delta_bytes.len() < DELTA_HEADER_BYTES + 32 || delta_bytes.len() > MAX_DELTA_READ_BYTES {
        return None;
    }
    let (payload, _trailer) = delta_bytes.split_at(delta_bytes.len() - 32);
    let mut r = Reader {
        buf: payload,
        pos: 0,
    };
    if r.read_bytes(8)? != DELTA_MAGIC {
        return None;
    }
    if r.read_u32()? != DELTA_VERSION {
        return None;
    }
    if &r.read_hash()? != expected_abi_hash {
        return None;
    }
    if &r.read_hash()? != expected_base_checksum {
        return None;
    }
    if (r.read_u32()? as usize) != expected_n_defs {
        return None;
    }
    if (r.read_u32()? as usize) != expected_global_words {
        return None;
    }
    if r.read_u8()? != 0 {
        return None;
    }
    let n_entries = r.read_u32()? as usize;
    if n_entries == 0 || n_entries > MAX_DELTA_ENTRIES {
        return None;
    }
    let mut out = try_vec_with_capacity::<(usize, &'a [u8])>(n_entries)?;
    let mut prev_bi: Option<usize> = None;
    for _ in 0..n_entries {
        let bi = r.read_u32()? as usize;
        let rec_len = r.read_u32()? as usize;
        if bi >= expected_n_defs || rec_len < min_fn_wire {
            return None;
        }
        if prev_bi.is_some_and(|p| bi <= p) {
            return None;
        }
        let rec = r.read_bytes(rec_len)?;
        out.push((bi, rec));
        prev_bi = Some(bi);
    }
    if r.pos != payload.len() {
        return None;
    }
    Some(out)
}

/// Decodes an already Merkle-verified incremental cache buffer from `load_inc_cache_raw`,
/// overlaying any matching verified `.delta` overrides cleanly on top of the base `.bin`.
/// Retains `bytes` as an immutable `Arc<Vec<u8>>` backing for zero-copy `CachedBytes::Slice`s.
#[allow(clippy::too_many_arguments)]
pub fn decode_verified_inc_cache(
    bytes: std::sync::Arc<Vec<u8>>,
    delta_bytes: Option<&[u8]>,
    expected_abi_hash: &[u8; 32],
    expected_n_imports: usize,
    expected_n_defs: usize,
    expected_global_words: usize,
    expected_rodata_len: usize,
    expected_body_hashes: &[[u8; 32]],
) -> Option<IncCacheData> {
    if expected_body_hashes.len() != expected_n_defs || bytes.len() < 80 {
        return None;
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - 32);
    let base_checksum: [u8; 32] = trailer.try_into().ok()?;

    let mut r = Reader {
        buf: payload,
        pos: 0,
    };
    if r.read_bytes(8)? != MAGIC {
        return None;
    }
    if r.read_u32()? != FORMAT_VERSION {
        return None;
    }
    let abi_hash = r.read_hash()?;
    if &abi_hash != expected_abi_hash {
        return None;
    }
    let n_defs = r.read_u32()? as usize;
    if n_defs != expected_n_defs {
        return None;
    }
    let global_words = r.read_u32()? as usize;
    if global_words != expected_global_words {
        return None;
    }
    let hit_inline_cap = r.read_u8()? != 0;
    if hit_inline_cap {
        return None;
    }
    let n_total = expected_n_imports.checked_add(n_defs)?;
    // Minimum fixed wire size per CachedFn entry (with empty vectors/code/rodata):
    // 32 + 1 + 4 + 4 + 4 + 4 + 4 + global_words*8 + 8 + 12 + 64 + 4 (pre_relax) + 4 + 4 + 4 + 4 = 157 + global_words*8.
    let min_fn_wire = 157usize.checked_add(global_words.checked_mul(8)?)?;

    // Format v6/v7 offset table: `(n_defs + 1)` little-endian `u32` byte offsets relative to `entries`.
    let n_offs = n_defs.checked_add(1)?;
    let off_raw = r.read_bytes(n_offs.checked_mul(4)?)?;
    let entries_abs_base = r.pos;
    let entries = &payload[entries_abs_base..];
    if entries.len() < n_defs.checked_mul(min_fn_wire)? {
        return None;
    }
    let mut offsets = try_vec_with_capacity::<u32>(n_offs)?;
    for &chunk in off_raw.as_chunks::<4>().0 {
        offsets.push(u32::from_le_bytes(chunk));
    }
    if offsets[0] != 0 || (offsets[n_defs] as usize) != entries.len() {
        return None;
    }
    for i in 0..n_defs {
        let start = offsets[i] as usize;
        let end = offsets[i + 1] as usize;
        if end < start || end - start < min_fn_wire {
            return None;
        }
    }

    let delta_overrides: Vec<(usize, &[u8])> = delta_bytes
        .and_then(|db| {
            parse_verified_delta(
                db,
                expected_abi_hash,
                &base_checksum,
                n_defs,
                global_words,
                min_fn_wire,
            )
        })
        .unwrap_or_default();
    let delta_lookup = |bi: usize| -> Option<&[u8]> {
        if delta_overrides.is_empty() {
            return None;
        }
        delta_overrides
            .binary_search_by_key(&bi, |&(idx, _)| idx)
            .ok()
            .map(|pos| delta_overrides[pos].1)
    };

    // Fast O(n_defs) early-bail check using the 32-byte `raw_body_hash` at the start of each entry.
    let mut changed_count = 0usize;
    for (i, expected_bh) in expected_body_hashes.iter().enumerate() {
        let bh_slice = if let Some(ds) = delta_lookup(i) {
            &ds[..32]
        } else {
            let start = offsets[i] as usize;
            &entries[start..start + 32]
        };
        if bh_slice != expected_bh {
            changed_count += 1;
            if changed_count > n_defs / 2 {
                // Incremental path would bail to cold compile anyway — abort before materializing!
                return None;
            }
        }
    }

    let mut funcs = try_vec_with_capacity::<CachedFn>(n_defs)?;
    let mut total_code = 0usize;
    let mut total_pre_relax_code = 0usize;
    let mut total_rodata = 0usize;

    if n_defs < PARALLEL_INC_MIN_FUNCS {
        for (i, expected_bh) in expected_body_hashes.iter().enumerate() {
            let (slice, abs_base) = if let Some(ds) = delta_lookup(i) {
                (ds, None)
            } else {
                let off_i = offsets[i] as usize;
                (
                    &entries[off_i..offsets[i + 1] as usize],
                    Some(entries_abs_base + off_i),
                )
            };
            let (cfn, code_len, pre_len, ro_len) = decode_cached_fn(
                slice,
                abs_base,
                expected_bh,
                n_defs,
                n_total,
                global_words,
                expected_rodata_len,
            )?;
            total_code = total_code.checked_add(code_len)?;
            total_pre_relax_code = total_pre_relax_code.checked_add(pre_len)?;
            total_rodata = total_rodata.checked_add(ro_len)?;
            if total_code > MAX_TOTAL_CODE_BYTES
                || total_pre_relax_code > MAX_TOTAL_CODE_BYTES
                || total_rodata > MAX_TOTAL_RODATA_BYTES
            {
                return None;
            }
            funcs.push(cfn);
        }
    } else {
        type DecodedSlot = Option<(CachedFn, usize, usize, usize)>;
        let mut decoded_slots = try_vec_with_capacity::<DecodedSlot>(n_defs)?;
        for _ in 0..n_defs {
            decoded_slots.push(None);
        }
        let n_threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16)
            .min(n_defs);
        let per_worker = n_defs.div_ceil(n_threads);
        let failed = AtomicBool::new(false);
        let offsets_ref = &offsets;
        let delta_ref = delta_overrides.as_slice();
        let decode_range = |start_idx: usize, slots: &mut [DecodedSlot]| {
            for (j, slot) in slots.iter_mut().enumerate() {
                if failed.load(Ordering::Acquire) {
                    return;
                }
                let i = start_idx + j;
                let (slice, abs_base) = if !delta_ref.is_empty()
                    && let Ok(pos) = delta_ref.binary_search_by_key(&i, |&(idx, _)| idx)
                {
                    (delta_ref[pos].1, None)
                } else {
                    let off_i = offsets_ref[i] as usize;
                    (
                        &entries[off_i..offsets_ref[i + 1] as usize],
                        Some(entries_abs_base + off_i),
                    )
                };
                match decode_cached_fn(
                    slice,
                    abs_base,
                    &expected_body_hashes[i],
                    n_defs,
                    n_total,
                    global_words,
                    expected_rodata_len,
                ) {
                    Some(v) => *slot = Some(v),
                    None => {
                        failed.store(true, Ordering::Release);
                        return;
                    }
                }
            }
        };
        let mut unspawned: [Option<(usize, usize)>; 16] = [None; 16];
        let mut n_unspawned = 0usize;
        std::thread::scope(|s| {
            let mut rem = decoded_slots.as_mut_slice();
            let mut idx = 0usize;
            while !rem.is_empty() {
                let take = per_worker.min(rem.len());
                let (chunk, rest) = rem.split_at_mut(take);
                let start_idx = idx;
                rem = rest;
                idx += take;
                if rem.is_empty() {
                    decode_range(start_idx, chunk);
                } else if std::thread::Builder::new()
                    .spawn_scoped(s, move || decode_range(start_idx, chunk))
                    .is_err()
                {
                    unspawned[n_unspawned] = Some((start_idx, take));
                    n_unspawned += 1;
                }
            }
        });
        for &(start_idx, take) in unspawned[..n_unspawned].iter().flatten() {
            decode_range(start_idx, &mut decoded_slots[start_idx..start_idx + take]);
        }
        if failed.load(Ordering::Acquire) {
            return None;
        }
        for slot in decoded_slots {
            let (cfn, code_len, pre_len, ro_len) = slot?;
            total_code = total_code.checked_add(code_len)?;
            total_pre_relax_code = total_pre_relax_code.checked_add(pre_len)?;
            total_rodata = total_rodata.checked_add(ro_len)?;
            if total_code > MAX_TOTAL_CODE_BYTES
                || total_pre_relax_code > MAX_TOTAL_CODE_BYTES
                || total_rodata > MAX_TOTAL_RODATA_BYTES
            {
                return None;
            }
            funcs.push(cfn);
        }
    }

    let delta_dirty: Vec<usize> = delta_overrides.iter().map(|&(bi, _)| bi).collect();
    Some(IncCacheData {
        abi_hash,
        global_words,
        hit_inline_cap,
        funcs,
        backing: Some(bytes),
        base_checksum: Some(base_checksum),
        delta_dirty,
    })
}

/// Returns true strictly for staging names produced by `save_inc_cache` (`tmp.<pid>.<seq>`).
fn is_tmp_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("tmp.") else {
        return false;
    };
    let Some((pid, seq)) = rest.split_once('.') else {
        return false;
    };
    !pid.is_empty()
        && !seq.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && seq.bytes().all(|b| b.is_ascii_digit())
}

/// Best-effort cleanup of abandoned `tmp.<pid>.<seq>` staging files older than `cutoff` owned by this user.
pub(crate) fn sweep_stale_tmp_files_with_cutoff(dir: &Path, cutoff: std::time::Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    #[cfg(unix)]
    let euid = crate::build::current_euid().ok();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_tmp_staging_name) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if euid.is_none_or(|u| meta.uid() != u) {
                continue;
            }
        }
        if meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age >= cutoff)
        {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Best-effort cleanup of abandoned `tmp.<pid>.<seq>` staging files older than 60s owned by this user.
fn sweep_stale_tmp_files(dir: &Path) {
    sweep_stale_tmp_files_with_cutoff(dir, std::time::Duration::from_secs(60));
}

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> SliceWriter<'a> {
    #[inline]
    fn write_bytes(&mut self, s: &[u8]) {
        let end = self.pos + s.len();
        self.buf[self.pos..end].copy_from_slice(s);
        self.pos = end;
    }

    #[inline]
    fn write_u8(&mut self, v: u8) {
        self.buf[self.pos] = v;
        self.pos += 1;
    }

    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }
}

fn fn_wire_len(f: &CachedFn, fixed_fn: usize) -> Option<usize> {
    fixed_fn
        .checked_add(f.pre_inline_callees.len().checked_mul(8)?)?
        .checked_add(f.inlined_callees.len().checked_mul(4)?)?
        .checked_add(f.own_effects.direct_callees.len().checked_mul(4)?)?
        .checked_add(f.elide_input.direct_def_callees.len().checked_mul(4)?)?
        .checked_add(f.code.len())?
        .checked_add(f.relocs.len().checked_mul(13)?)?
        .checked_add(f.local_rodata.len())?
        .checked_add(f.jt_entries.len().checked_mul(12)?)
}

fn write_cached_fn(
    w: &mut SliceWriter<'_>,
    f: &CachedFn,
    global_words: usize,
    backing: Option<&[u8]>,
) {
    w.write_bytes(&f.raw_body_hash);
    let flags = u8::from(f.was_inlinable)
        | (u8::from(f.own_effects.writes_mem) << 1)
        | (u8::from(f.own_effects.grows_mem) << 2)
        | (u8::from(f.own_effects.writes_all_globals) << 3)
        | (u8::from(f.own_effects.writes_all_tables) << 4)
        | (u8::from(f.elide_input.local_may_trap) << 5)
        | (u8::from(f.elide_input.calls_import_or_indirect) << 6)
        | (u8::from(f.needs_stack_check) << 7);
    w.write_u8(flags);

    w.write_u32(f.pre_inline_callees.len() as u32);
    for &(c, cnt) in &f.pre_inline_callees {
        w.write_u32(c);
        w.write_u32(cnt);
    }

    w.write_u32(f.inlined_callees.len() as u32);
    for &c in &f.inlined_callees {
        w.write_u32(c);
    }
    w.write_u32(f.inline_added_insts);

    w.write_u32(f.own_effects.direct_callees.len() as u32);
    for &c in &f.own_effects.direct_callees {
        w.write_u32(c);
    }

    w.write_u32(f.elide_input.direct_def_callees.len() as u32);
    for &c in &f.elide_input.direct_def_callees {
        w.write_u32(c);
    }

    for gw in 0..global_words {
        let val = f.own_effects.written_globals.get(gw).copied().unwrap_or(0);
        w.write_u64(val);
    }
    w.write_u64(f.own_effects.written_tables);
    w.write_i32(f.elide_input.max_helper_bytes);
    w.write_i32(f.elide_input.own_stack_bytes);
    w.write_i32(f.elided_callee_stack_bytes);
    w.write_bytes(&f.callee_effects_hash);
    w.write_bytes(&f.callee_trap_hash);

    w.write_u32(f.pre_relax_code_len);
    let code_bytes = f.code.as_slice(backing);
    w.write_u32(code_bytes.len() as u32);
    w.write_bytes(code_bytes);

    w.write_u32(f.relocs.len() as u32);
    for r in &f.relocs {
        w.write_u32(r.patch_off);
        w.write_u32(r.next_ip_off);
        let (kind, val) = match r.target {
            RelocTarget::Func(v) => (0u8, v),
            RelocTarget::Helper(v) => (1u8, v),
            RelocTarget::Rodata(v) => (2u8, v),
            RelocTarget::LocalRodata(v) => (3u8, v),
        };
        w.write_u8(kind);
        w.write_u32(val);
    }

    let ro_bytes = f.local_rodata.as_slice(backing);
    w.write_u32(ro_bytes.len() as u32);
    w.write_bytes(ro_bytes);

    w.write_u32(f.jt_entries.len() as u32);
    for &(ro_off, tbl_off, lbl_off) in &f.jt_entries {
        w.write_u32(ro_off);
        w.write_u32(tbl_off);
        w.write_u32(lbl_off);
    }
}

/// Atomically writes `data` to `<cache_dir>/x64-inc/<abi_hash>.bin` (or a compact `.delta`
/// sidecar when `<= 64` functions changed relative to an intact base `.bin`, eliminating multi-MiB
/// re-serialization & hashing on small incremental edits). Best-effort: silently ignores I/O,
/// permission, or OOM failures so caching can never break compilation.
pub fn save_inc_cache(cache_dir: &Path, data: &IncCacheData) {
    let Ok(dir) = crate::build::ensure_private_dir(&cache_dir.join("x64-inc")) else {
        return;
    };
    sweep_stale_tmp_files(&dir);

    let n_defs = data.funcs.len();
    let Some(fixed_fn) = data
        .global_words
        .checked_mul(8)
        .and_then(|g| 157usize.checked_add(g))
    else {
        return;
    };

    let target = dir.join(cache_file_name(&data.abi_hash));
    let delta_target = dir.join(delta_file_name(&data.abi_hash));

    // Fast path: small incremental update against an intact base `.bin` on disk -> write compact
    // `<abi_hash>.delta` (~2–16 KiB, <0.1 ms) instead of rewriting the full multi-MiB `.bin`!
    if let Some(base_ck) = data.base_checksum
        && !data.hit_inline_cap
        && data.delta_dirty.len() <= MAX_DELTA_ENTRIES
        && fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_file())
    {
        if data.delta_dirty.is_empty() {
            return;
        }
        let mut delta_total = DELTA_HEADER_BYTES;
        let mut delta_ok = true;
        let mut prev_bi: Option<usize> = None;
        for &bi in &data.delta_dirty {
            if bi >= n_defs || prev_bi.is_some_and(|p| bi <= p) {
                delta_ok = false;
                break;
            }
            prev_bi = Some(bi);
            let Some(sz) = fn_wire_len(&data.funcs[bi], fixed_fn) else {
                delta_ok = false;
                break;
            };
            let Some(next) = delta_total.checked_add(8).and_then(|n| n.checked_add(sz)) else {
                delta_ok = false;
                break;
            };
            if next > MAX_DELTA_PAYLOAD_BYTES {
                delta_ok = false;
                break;
            }
            delta_total = next;
        }
        if delta_ok && let Some(mut dout) = try_vec_with_capacity::<u8>(delta_total + 32) {
            dout.extend_from_slice(DELTA_MAGIC);
            dout.extend_from_slice(&DELTA_VERSION.to_le_bytes());
            dout.extend_from_slice(&data.abi_hash);
            dout.extend_from_slice(&base_ck);
            dout.extend_from_slice(&(n_defs as u32).to_le_bytes());
            dout.extend_from_slice(&(data.global_words as u32).to_le_bytes());
            dout.push(u8::from(data.hit_inline_cap));
            dout.extend_from_slice(&(data.delta_dirty.len() as u32).to_le_bytes());
            debug_assert_eq!(dout.len(), DELTA_HEADER_BYTES);
            let backing = data.backing.as_deref().map(Vec::as_slice);
            for &bi in &data.delta_dirty {
                let f = &data.funcs[bi];
                let sz = fn_wire_len(f, fixed_fn).unwrap();
                dout.extend_from_slice(&(bi as u32).to_le_bytes());
                dout.extend_from_slice(&(sz as u32).to_le_bytes());
                let start = dout.len();
                dout.resize(start + sz, 0);
                let mut w = SliceWriter {
                    buf: &mut dout[start..start + sz],
                    pos: 0,
                };
                write_cached_fn(&mut w, f, data.global_words, backing);
                debug_assert_eq!(w.pos, sz);
            }
            debug_assert_eq!(dout.len(), delta_total);
            let ck = crate::sha256::digest(&dout);
            dout.extend_from_slice(&ck);
            for _ in 0..16 {
                let seq = NEXT_INC_SEQ.fetch_add(1, Ordering::Relaxed);
                let tmp = dir.join(format!("tmp.{}.{seq}", std::process::id()));
                let mut opts = fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    opts.mode(0o600);
                    let nf = unix_nofollow_flag();
                    if nf != 0 {
                        opts.custom_flags(nf);
                    }
                }
                match opts.open(&tmp) {
                    Ok(mut f) => {
                        let ok = f
                            .write_all(&dout)
                            .and_then(|()| fs::rename(&tmp, &delta_target))
                            .is_ok();
                        if !ok {
                            let _ = fs::remove_file(&tmp);
                        }
                        return;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => return,
                }
            }
            return;
        }
    }

    let Some(off_table_bytes) = n_defs.checked_add(1).and_then(|n| n.checked_mul(4)) else {
        return;
    };
    let entries_base = 53usize.saturating_add(off_table_bytes);
    let Some(mut offsets) = try_vec_with_capacity::<u32>(n_defs + 1) else {
        return;
    };
    let mut cur_entry_off = 0usize;
    offsets.push(0u32);
    for f in &data.funcs {
        let Some(sz) = fn_wire_len(f, fixed_fn) else {
            return;
        };
        let Some(next) = cur_entry_off.checked_add(sz) else {
            return;
        };
        if entries_base.saturating_add(next).saturating_add(32) > MAX_CACHE_FILE_BYTES {
            return;
        }
        let Ok(next_u32) = u32::try_from(next) else {
            return;
        };
        cur_entry_off = next;
        offsets.push(next_u32);
    }

    let payload_len = entries_base + cur_entry_off;
    let Some(mut out) = try_vec_with_capacity::<u8>(payload_len + 32) else {
        return;
    };
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&data.abi_hash);
    out.extend_from_slice(&(n_defs as u32).to_le_bytes());
    out.extend_from_slice(&(data.global_words as u32).to_le_bytes());
    out.push(u8::from(data.hit_inline_cap));
    for &off in &offsets {
        out.extend_from_slice(&off.to_le_bytes());
    }
    debug_assert_eq!(out.len(), entries_base);
    out.resize(payload_len, 0);

    let global_words = data.global_words;
    let funcs = &data.funcs;
    let backing = data.backing.as_deref().map(Vec::as_slice);
    let entries_buf = &mut out[entries_base..payload_len];
    if n_defs < PARALLEL_INC_MIN_FUNCS {
        let mut w = SliceWriter {
            buf: entries_buf,
            pos: 0,
        };
        for f in funcs {
            write_cached_fn(&mut w, f, global_words, backing);
        }
        debug_assert_eq!(w.pos, entries_buf.len());
    } else {
        let n_threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16)
            .min(n_defs);
        let per_worker = n_defs.div_ceil(n_threads);
        let write_range = |start_idx: usize, end_idx: usize, slice: &mut [u8]| {
            let mut w = SliceWriter { buf: slice, pos: 0 };
            for f in &funcs[start_idx..end_idx] {
                write_cached_fn(&mut w, f, global_words, backing);
            }
            debug_assert_eq!(w.pos, slice.len());
        };
        let mut unspawned: [Option<(usize, usize)>; 16] = [None; 16];
        let mut n_unspawned = 0usize;
        std::thread::scope(|s| {
            let mut rem_buf = entries_buf;
            let mut idx = 0usize;
            while idx < n_defs {
                let end_idx = (idx + per_worker).min(n_defs);
                let byte_len = (offsets[end_idx] - offsets[idx]) as usize;
                let (chunk_buf, rest_buf) = rem_buf.split_at_mut(byte_len);
                let start_idx = idx;
                rem_buf = rest_buf;
                idx = end_idx;
                if idx == n_defs {
                    write_range(start_idx, end_idx, chunk_buf);
                } else if std::thread::Builder::new()
                    .spawn_scoped(s, move || write_range(start_idx, end_idx, chunk_buf))
                    .is_err()
                {
                    unspawned[n_unspawned] = Some((start_idx, end_idx));
                    n_unspawned += 1;
                }
            }
        });
        for &(start_idx, end_idx) in unspawned[..n_unspawned].iter().flatten() {
            let b0 = offsets[start_idx] as usize;
            let b1 = offsets[end_idx] as usize;
            write_range(
                start_idx,
                end_idx,
                &mut out[entries_base + b0..entries_base + b1],
            );
        }
    }

    // Overlap staging-file payload write with parallel Merkle checksum computation,
    // then append the 32-byte trailer and atomically rename into place (clearing any old `.delta`).
    for _ in 0..16 {
        let seq = NEXT_INC_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("tmp.{}.{seq}", std::process::id()));
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
            let nf = unix_nofollow_flag();
            if nf != 0 {
                opts.custom_flags(nf);
            }
        }
        match opts.open(&tmp) {
            Ok(mut f) => {
                let payload_ref = out.as_slice();
                let (write_res, checksum_opt) = std::thread::scope(|s| {
                    let h_hash = std::thread::Builder::new()
                        .spawn_scoped(s, || parallel_payload_digest(payload_ref))
                        .ok();
                    let wr = f.write_all(payload_ref);
                    let ck = h_hash
                        .and_then(|h| h.join().ok())
                        .unwrap_or_else(|| parallel_payload_digest(payload_ref));
                    (wr, ck)
                });
                let ok = write_res.is_ok()
                    && checksum_opt.is_some_and(|ck| {
                        f.write_all(&ck)
                            .and_then(|()| fs::rename(&tmp, &target))
                            .is_ok()
                    });
                if ok {
                    let _ = fs::remove_file(&delta_target);
                } else {
                    let _ = fs::remove_file(&tmp);
                }
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return,
        }
    }
}

// ============================================================================
// Companion Validation & Admission Index (`v.<shell_hash>.vidx`)
// ============================================================================

const VIDX_MAGIC: &[u8; 8] = b"WTX64VID";
const VIDX_VERSION: u32 = 4;
const MAX_VIDX_FILE_BYTES: usize = 32 << 20;

/// Per-body cached validation and `x64` admission summary for incremental validation skip.
#[derive(Clone)]
pub struct ValBodyEntry {
    pub raw_body_hash: [u8; 32],
    pub total_locals: u32,
    pub insn_count: u32,
    pub imm_elems: u32,
    pub saw_data_count_op: bool,
    pub uses_exceptions: bool,
    pub uses_bulk_data: bool,
    pub uses_bulk_elem: bool,
    pub dyn_table_ops: Vec<u32>,
    pub sig_arity: u32,
    pub br_table_entries: u32,
    pub g_reads: Vec<(u32, u32)>,
    pub g_writes: Vec<(u32, u32)>,
    pub ci_pairs: Vec<(u32, u32)>,
}

/// Merkle-verified validation/admission/call-graph index keyed by `shell_hash`.
pub struct ValIndexData {
    pub shell_hash: [u8; 32],
    pub abi_hash: [u8; 32],
    pub topo_bottom_up: Vec<usize>,
    pub in_cycle: Vec<bool>,
    pub bodies: Vec<ValBodyEntry>,
}

fn read_uleb32(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let mut out = 0u32;
    let mut shift = 0u32;
    for _ in 0..5 {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        let low = u32::from(b & 0x7f);
        if shift == 28 && (b & 0xf0) != 0 {
            return None;
        }
        out |= low << shift;
        if (b & 0x80) == 0 {
            return Some(out);
        }
        shift += 7;
    }
    None
}

/// Shell digest and raw function-body byte spans `(start, end)` from [`scan_wasm_shell_and_body_spans`].
pub type WasmShellAndSpans = ([u8; 32], Vec<(usize, usize)>);

/// Quickly scans Wasm section headers to compute `shell_hash` (cryptographic hash of every
/// non-custom, non-code section + compiler salt + ISA/target/opt/verify + body count) and
/// exact raw byte spans `(start, end)` of every function body in the code section.
///
/// Returns `None` on any structural anomaly so callers fall back to full `wtime_validate::validate`.
pub fn scan_wasm_shell_and_body_spans(
    wasm: &[u8],
    salt: &[u8; 32],
    isa: TargetIsa,
    opt: bool,
    verify: bool,
    target: super::module::X64Target,
) -> Option<WasmShellAndSpans> {
    if wasm.len() < 8 || &wasm[..8] != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut h = Sha256::new();
    h.update(VIDX_MAGIC);
    hash_u32(&mut h, VIDX_VERSION);
    h.update(salt);
    h.update(&[
        u8::from(isa.has_abm_bmi1),
        u8::from(isa.has_bmi2),
        u8::from(isa.has_avx2),
        u8::from(opt),
        u8::from(verify),
        match target {
            super::module::X64Target::BlotCdylib => 0,
            super::module::X64Target::StandaloneExec => 1,
        },
    ]);
    let mut pos = 8usize;
    let mut last_rank = 0u8;
    let mut seen_mask = 0u16;
    let mut spans: Option<Vec<(usize, usize)>> = None;
    while pos < wasm.len() {
        let id = *wasm.get(pos)?;
        pos += 1;
        let len = read_uleb32(wasm, &mut pos)? as usize;
        let end = pos.checked_add(len)?;
        if end > wasm.len() {
            return None;
        }
        if id == 0 {
            pos = end;
            continue;
        }
        let rank = match id {
            1..=5 => id,
            13 => 6,
            6..=9 => id + 1,
            12 => 11,
            10 => 12,
            11 => 13,
            _ => return None,
        };
        if rank < last_rank || (seen_mask & (1u16 << id)) != 0 {
            return None;
        }
        last_rank = rank;
        seen_mask |= 1u16 << id;
        if id == 10 {
            let mut p = pos;
            let n_defs = read_uleb32(wasm, &mut p)? as usize;
            if n_defs > 200_000 {
                return None;
            }
            let mut body_spans = try_vec_with_capacity::<(usize, usize)>(n_defs)?;
            for _ in 0..n_defs {
                let sz = read_uleb32(wasm, &mut p)? as usize;
                let b_end = p.checked_add(sz)?;
                if b_end > end {
                    return None;
                }
                body_spans.push((p, b_end));
                p = b_end;
            }
            if p != end {
                return None;
            }
            h.update(&[10]);
            hash_u32(&mut h, n_defs as u32);
            spans = Some(body_spans);
        } else {
            h.update(&[id]);
            hash_u32(&mut h, len as u32);
            if len > 128 * 1024
                && let Some(sec_root) = parallel_payload_digest(&wasm[pos..end])
            {
                h.update(&[1]);
                h.update(&sec_root);
            } else {
                h.update(&[0]);
                h.update(&wasm[pos..end]);
            }
        }
        pos = end;
    }
    let body_spans = spans?;
    Some((h.finish(), body_spans))
}

fn vidx_file_name(shell_hash: &[u8; 32]) -> String {
    format!("v.{}.vidx", to_hex(shell_hash))
}

/// Fast O(1) existence check for `<cache_dir>/x64-inc/v.<shell_hash>.vidx`.
pub fn has_val_index(cache_dir: &Path, shell_hash: &[u8; 32]) -> bool {
    let path = cache_dir.join("x64-inc").join(vidx_file_name(shell_hash));
    fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file())
}

/// Loads and Merkle-verifies `<cache_dir>/x64-inc/v.<shell_hash>.vidx`.
pub fn load_val_index(
    cache_dir: &Path,
    expected_shell_hash: &[u8; 32],
    expected_n_defs: usize,
) -> Option<ValIndexData> {
    let inc_dir = cache_dir.join("x64-inc");
    let dir_meta = fs::symlink_metadata(&inc_dir).ok()?;
    if dir_meta.file_type().is_symlink() || !dir_meta.file_type().is_dir() {
        return None;
    }
    let canon_dir = crate::build::ensure_private_dir(&inc_dir).ok()?;
    let path = canon_dir.join(vidx_file_name(expected_shell_hash));
    let bytes = read_private_cache_file(&path)?;
    if bytes.len() > MAX_VIDX_FILE_BYTES || bytes.len() < 120 {
        return None;
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - 32);
    if parallel_payload_digest(payload)? != trailer {
        return None;
    }
    let mut r = Reader {
        buf: payload,
        pos: 0,
    };
    if r.read_bytes(8)? != VIDX_MAGIC || r.read_u32()? != VIDX_VERSION {
        return None;
    }
    let shell_hash = r.read_hash()?;
    if &shell_hash != expected_shell_hash {
        return None;
    }
    let abi_hash = r.read_hash()?;
    let n_defs = r.read_u32()? as usize;
    if n_defs != expected_n_defs {
        return None;
    }
    let has_topo = r.read_u8()? != 0;
    let (topo_bottom_up, in_cycle) = if has_topo {
        r.check_remaining(n_defs, 4)?;
        let mut topo = try_vec_with_capacity::<usize>(n_defs)?;
        let raw_topo = r.read_bytes(n_defs.checked_mul(4)?)?;
        for &ch in raw_topo.as_chunks::<4>().0 {
            let idx = u32::from_le_bytes(ch) as usize;
            if idx >= n_defs {
                return None;
            }
            topo.push(idx);
        }
        let cyc_bytes = r.read_bytes(n_defs.div_ceil(8))?;
        let mut cyc = try_vec_with_capacity::<bool>(n_defs)?;
        for i in 0..n_defs {
            cyc.push((cyc_bytes[i >> 3] & (1u8 << (i & 7))) != 0);
        }
        (topo, cyc)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut bodies = try_vec_with_capacity::<ValBodyEntry>(n_defs)?;
    for _ in 0..n_defs {
        let raw_body_hash = r.read_hash()?;
        let total_locals = r.read_u32()?;
        let insn_count = r.read_u32()?;
        let imm_elems = r.read_u32()?;
        let saw_data_count_op = r.read_u8()? != 0;
        let uses_exceptions = r.read_u8()? != 0;
        let uses_bulk_data = r.read_u8()? != 0;
        let uses_bulk_elem = r.read_u8()? != 0;
        let n_dyn = r.read_u32()? as usize;
        if n_dyn > 65_536 {
            return None;
        }
        r.check_remaining(n_dyn, 4)?;
        let mut dyn_table_ops = try_vec_with_capacity(n_dyn)?;
        for _ in 0..n_dyn {
            dyn_table_ops.push(r.read_u32()?);
        }
        let sig_arity = r.read_u32()?;
        let br_table_entries = r.read_u32()?;
        let read_pairs = |r: &mut Reader<'_>| -> Option<Vec<(u32, u32)>> {
            let n = r.read_u32()? as usize;
            if n > 65_536 {
                return None;
            }
            r.check_remaining(n, 8)?;
            let mut out = try_vec_with_capacity(n)?;
            for _ in 0..n {
                out.push((r.read_u32()?, r.read_u32()?));
            }
            Some(out)
        };
        let g_reads = read_pairs(&mut r)?;
        let g_writes = read_pairs(&mut r)?;
        let ci_pairs = read_pairs(&mut r)?;
        bodies.push(ValBodyEntry {
            raw_body_hash,
            total_locals,
            insn_count,
            imm_elems,
            saw_data_count_op,
            uses_exceptions,
            uses_bulk_data,
            uses_bulk_elem,
            dyn_table_ops,
            sig_arity,
            br_table_entries,
            g_reads,
            g_writes,
            ci_pairs,
        });
    }
    if r.pos != payload.len() {
        return None;
    }
    Some(ValIndexData {
        shell_hash,
        abi_hash,
        topo_bottom_up,
        in_cycle,
        bodies,
    })
}

/// Atomically saves `ValIndexData` to `<cache_dir>/x64-inc/v.<shell_hash>.vidx`. Best-effort.
pub fn save_val_index(cache_dir: &Path, data: &ValIndexData) {
    let Ok(dir) = crate::build::ensure_private_dir(&cache_dir.join("x64-inc")) else {
        return;
    };
    let n_defs = data.bodies.len();
    let has_topo = data.topo_bottom_up.len() == n_defs && data.in_cycle.len() == n_defs;
    let est =
        85 + if has_topo {
            n_defs * 4 + n_defs.div_ceil(8)
        } else {
            0
        } + n_defs * 68
            + 32;
    let Some(mut out) = try_vec_with_capacity::<u8>(est.min(MAX_VIDX_FILE_BYTES)) else {
        return;
    };
    out.extend_from_slice(VIDX_MAGIC);
    out.extend_from_slice(&VIDX_VERSION.to_le_bytes());
    out.extend_from_slice(&data.shell_hash);
    out.extend_from_slice(&data.abi_hash);
    out.extend_from_slice(&(n_defs as u32).to_le_bytes());
    out.push(u8::from(has_topo));
    if has_topo {
        for &idx in &data.topo_bottom_up {
            out.extend_from_slice(&(idx as u32).to_le_bytes());
        }
        let mut cyc_bytes = vec![0u8; n_defs.div_ceil(8)];
        for (i, &c) in data.in_cycle.iter().enumerate() {
            if c {
                cyc_bytes[i >> 3] |= 1u8 << (i & 7);
            }
        }
        out.extend_from_slice(&cyc_bytes);
    }
    for b in &data.bodies {
        out.extend_from_slice(&b.raw_body_hash);
        out.extend_from_slice(&b.total_locals.to_le_bytes());
        out.extend_from_slice(&b.insn_count.to_le_bytes());
        out.extend_from_slice(&b.imm_elems.to_le_bytes());
        out.push(u8::from(b.saw_data_count_op));
        out.push(u8::from(b.uses_exceptions));
        out.push(u8::from(b.uses_bulk_data));
        out.push(u8::from(b.uses_bulk_elem));
        out.extend_from_slice(&(b.dyn_table_ops.len() as u32).to_le_bytes());
        for &t in &b.dyn_table_ops {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out.extend_from_slice(&b.sig_arity.to_le_bytes());
        out.extend_from_slice(&b.br_table_entries.to_le_bytes());
        for pairs in [&b.g_reads, &b.g_writes, &b.ci_pairs] {
            out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for &(k, v) in pairs {
                out.extend_from_slice(&k.to_le_bytes());
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        if out.len() + 32 > MAX_VIDX_FILE_BYTES {
            return;
        }
    }
    let Some(checksum) = parallel_payload_digest(&out) else {
        return;
    };
    out.extend_from_slice(&checksum);

    let target = dir.join(vidx_file_name(&data.shell_hash));
    for _ in 0..16 {
        let seq = NEXT_INC_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("tmp.{}.{seq}", std::process::id()));
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
            let nf = unix_nofollow_flag();
            if nf != 0 {
                opts.custom_flags(nf);
            }
        }
        match opts.open(&tmp) {
            Ok(mut f) => {
                let ok = f
                    .write_all(&out)
                    .and_then(|()| fs::rename(&tmp, &target))
                    .is_ok();
                if !ok {
                    let _ = fs::remove_file(&tmp);
                }
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_tmp_staging_name;

    #[test]
    fn tmp_staging_name_matches_only_numeric_pid_seq() {
        assert!(is_tmp_staging_name("tmp.1234.0"));
        assert!(is_tmp_staging_name("tmp.999999.42"));
        assert!(!is_tmp_staging_name("tmp.notes"));
        assert!(!is_tmp_staging_name("tmp.1234"));
        assert!(!is_tmp_staging_name("tmp.1234."));
        assert!(!is_tmp_staging_name("tmp..0"));
        assert!(!is_tmp_staging_name("tmp.12a.0"));
        assert!(!is_tmp_staging_name("tmp.12.0.extra"));
        assert!(!is_tmp_staging_name("other.12.0"));
    }
}
