//! Content-addressed per-function incremental compilation cache for `--backend aarch64`.
//!
//! Mirrors `x64::inc_cache` under `<cache>/a64-inc/`, storing pre-link AArch64 machine code
//! (`code`, `relocs`, unpatched `local_rodata`, resolved jump-table label offsets) plus
//! interprocedural summaries (`pre_inline_callees`, `inlined_callees`, `own_effects`,
//! `callee_effects_hash`) inside a Merkle-SHA-256 verified binary file.
//!
//! Guarantees:
//! - Strictly `#![forbid(unsafe_code)]`, fallible allocations, bounded 128 MiB reads.
//! - Any missing, truncated, corrupted, or ABI-mismatched cache file silently falls back
//!   to cold compilation.
//! - Produces 100% byte-identical ELF64 `.so` / static executable output on 1-function edits
//!   compared to a cold compile.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::sha256::{Sha256, to_hex};
use crate::x64::inc_cache::{
    CachedBytes, compute_abi_hash, parallel_payload_digest, sweep_stale_tmp_files_with_cutoff,
    try_copy_slice, try_vec_with_capacity,
};
use crate::x64::lower::TargetIsa;
use crate::x64::module::{
    MAX_FN_CODE_BYTES, MAX_FN_LOCAL_RODATA_BYTES, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES,
    X64Module,
};
use crate::x64::opt::FnOwnEffects;
use crate::x64::{CtxLayout, NUM_HELPERS, RodataConsts};

use super::asm::{A64Asm, A64Reloc, A64RelocKind, A64RelocTarget};

const MAGIC: &[u8; 8] = b"WTA64INC";
const FORMAT_VERSION: u32 = 3;
const MAX_CACHE_FILE_BYTES: usize = 128 << 20;

static NEXT_A64_INC_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct CachedA64Fn {
    pub raw_body_hash: [u8; 32],
    pub was_inlinable: bool,
    pub pre_inline_callees: Vec<(u32, u32)>,
    pub inlined_callees: Vec<u32>,
    pub inline_added_insts: u32,
    pub own_effects: FnOwnEffects,
    pub callee_effects_hash: [u8; 32],
    pub code: CachedBytes,
    pub relocs: Vec<A64Reloc>,
    pub local_rodata: CachedBytes,
    pub jump_tables: Vec<(u32, Vec<u32>)>,
}

impl CachedA64Fn {
    pub fn into_asm(self, backing: Option<&[u8]>) -> A64Asm {
        A64Asm::from_cached_parts(
            self.code.into_vec(backing),
            self.relocs,
            self.local_rodata.into_vec(backing),
            self.jump_tables,
        )
    }
}

pub struct A64IncCacheData {
    pub abi_hash: [u8; 32],
    pub global_words: usize,
    pub hit_inline_cap: bool,
    pub funcs: Vec<CachedA64Fn>,
    pub backing: Option<std::sync::Arc<Vec<u8>>>,
}

/// Computes the exact module-wide AArch64 ABI & layout fingerprint binding every cached function.
pub fn compute_a64_abi_hash(
    gen_fingerprint: &[u8; 32],
    opt: bool,
    n_defs: usize,
    module: &X64Module,
    layout: &CtxLayout,
) -> [u8; 32] {
    let mut salt_h = Sha256::new();
    salt_h.update(MAGIC);
    salt_h.update(&FORMAT_VERSION.to_le_bytes());
    salt_h.update(gen_fingerprint);
    salt_h.update(&[u8::from(layout.use_guard_pages)]);
    let salt = salt_h.finish();
    compute_abi_hash(
        &salt,
        TargetIsa::default(),
        opt,
        true,
        n_defs,
        module,
        layout,
        &RodataConsts::default(),
    )
}

fn cache_file_path(cache_dir: &Path, abi_hash: &[u8; 32]) -> std::path::PathBuf {
    cache_dir.join(format!("{}.bin", to_hex(abi_hash)))
}

struct BinWriter {
    buf: Vec<u8>,
}

impl BinWriter {
    fn try_with_capacity(cap: usize) -> Option<Self> {
        Some(Self {
            buf: try_vec_with_capacity(cap)?,
        })
    }

    #[inline]
    fn reserve_more(&mut self, extra: usize) -> Option<()> {
        if self.buf.len().checked_add(extra)? > self.buf.capacity() {
            self.buf.try_reserve(extra).ok()?;
        }
        Some(())
    }

    #[inline]
    fn u8(&mut self, v: u8) -> Option<()> {
        self.reserve_more(1)?;
        self.buf.push(v);
        Some(())
    }

    #[inline]
    fn u32(&mut self, v: u32) -> Option<()> {
        self.bytes(&v.to_le_bytes())
    }

    #[inline]
    fn u64(&mut self, v: u64) -> Option<()> {
        self.bytes(&v.to_le_bytes())
    }

    #[inline]
    fn bytes(&mut self, b: &[u8]) -> Option<()> {
        self.reserve_more(b.len())?;
        self.buf.extend_from_slice(b);
        Some(())
    }
}

struct BinReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline]
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    #[inline]
    fn take_range(&mut self, n: usize) -> Option<CachedBytes> {
        let start = u32::try_from(self.pos).ok()?;
        let len = u32::try_from(n).ok()?;
        let _ = self.take(n)?;
        Some(CachedBytes::Slice { start, len })
    }

    #[inline]
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    #[inline]
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    #[inline]
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    #[inline]
    fn hash32(&mut self) -> Option<[u8; 32]> {
        self.take(32)?.try_into().ok()
    }
}

fn ensure_private_a64_inc_dir(dir: &Path) -> Option<()> {
    if !dir.exists() {
        fs::create_dir_all(dir).ok()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let meta = fs::symlink_metadata(dir).ok()?;
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return None;
        }
        let euid = crate::build::current_euid().ok()?;
        if meta.uid() != euid {
            return None;
        }
        let mode = meta.permissions().mode() & 0o777;
        if (mode & 0o077) != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).ok()?;
        }
    }
    Some(())
}

/// Reads and verifies the AArch64 incremental cache file on disk, returning `None` on any mismatch or I/O error.
pub fn load_a64_inc_cache(
    cache_dir: &Path,
    expected_abi_hash: &[u8; 32],
    expected_n_defs: usize,
    expected_n_total: usize,
    expected_global_words: usize,
) -> Option<A64IncCacheData> {
    let path = cache_file_path(cache_dir, expected_abi_hash);
    let mut f = fs::File::open(&path).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let file_len = usize::try_from(meta.len()).ok()?;
    if !(8 + 4 + 32 + 32..=MAX_CACHE_FILE_BYTES).contains(&file_len) {
        return None;
    }
    let mut raw = try_vec_with_capacity(file_len)?;
    let mut chunk = [0u8; 16384];
    loop {
        let n = f.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        if raw.len().saturating_add(n) > MAX_CACHE_FILE_BYTES {
            return None;
        }
        raw.extend_from_slice(&chunk[..n]);
    }
    if raw.len() < 8 + 4 + 32 + 32 {
        return None;
    }
    let payload_len = raw.len() - 32;
    let payload = &raw[..payload_len];
    let stored_digest = &raw[payload_len..];
    let computed = parallel_payload_digest(payload)?;
    if computed.as_slice() != stored_digest {
        return None;
    }

    let mut r = BinReader::new(payload);
    if r.take(8)? != MAGIC {
        return None;
    }
    if r.u32()? != FORMAT_VERSION {
        return None;
    }
    let abi_hash = r.hash32()?;
    if &abi_hash != expected_abi_hash {
        return None;
    }
    let global_words = r.u32()? as usize;
    if global_words != expected_global_words {
        return None;
    }
    let hit_inline_cap = match r.u8()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let n_funcs = r.u32()? as usize;
    if n_funcs != expected_n_defs {
        return None;
    }

    let mut funcs = try_vec_with_capacity(n_funcs)?;
    let mut total_code = 0usize;
    let mut total_ro = 0usize;

    for _ in 0..n_funcs {
        let raw_body_hash = r.hash32()?;
        let was_inlinable = match r.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let n_pre = r.u32()? as usize;
        if n_pre > expected_n_defs {
            return None;
        }
        let mut pre_inline_callees = try_vec_with_capacity(n_pre)?;
        for _ in 0..n_pre {
            let c = r.u32()?;
            let cnt = r.u32()?;
            if (c as usize) >= expected_n_defs || cnt == 0 {
                return None;
            }
            pre_inline_callees.push((c, cnt));
        }
        let n_inl = r.u32()? as usize;
        if n_inl > expected_n_defs {
            return None;
        }
        let mut inlined_callees = try_vec_with_capacity(n_inl)?;
        for _ in 0..n_inl {
            let c = r.u32()?;
            if (c as usize) >= expected_n_defs {
                return None;
            }
            inlined_callees.push(c);
        }
        let inline_added_insts = r.u32()?;

        let eff_flags = r.u8()?;
        let writes_mem = (eff_flags & 1) != 0;
        let grows_mem = (eff_flags & 2) != 0;
        let writes_all_globals = (eff_flags & 4) != 0;
        let writes_all_tables = (eff_flags & 8) != 0;
        let mut written_globals = try_vec_with_capacity(global_words)?;
        for _ in 0..global_words {
            written_globals.push(r.u64()?);
        }
        let written_tables = r.u64()?;
        let n_dc = r.u32()? as usize;
        if n_dc > expected_n_total {
            return None;
        }
        let mut direct_callees = try_vec_with_capacity(n_dc)?;
        for _ in 0..n_dc {
            let c = r.u32()?;
            if (c as usize) >= expected_n_total {
                return None;
            }
            direct_callees.push(c);
        }
        let own_effects = FnOwnEffects {
            writes_mem,
            grows_mem,
            writes_all_globals,
            written_globals,
            writes_all_tables,
            written_tables,
            direct_callees,
        };
        let callee_effects_hash = r.hash32()?;

        let code_len = r.u32()? as usize;
        if code_len > MAX_FN_CODE_BYTES || (code_len & 3) != 0 {
            return None;
        }
        total_code = total_code.checked_add(code_len)?;
        if total_code > MAX_TOTAL_CODE_BYTES {
            return None;
        }
        let code = r.take_range(code_len)?;

        let lro_len = r.u32()? as usize;
        if lro_len > MAX_FN_LOCAL_RODATA_BYTES {
            return None;
        }
        total_ro = total_ro.checked_add(lro_len)?;
        if total_ro > MAX_TOTAL_RODATA_BYTES {
            return None;
        }
        let local_rodata = r.take_range(lro_len)?;

        let n_relocs = r.u32()? as usize;
        if n_relocs > code_len / 4 + 1 {
            return None;
        }
        let mut relocs = try_vec_with_capacity(n_relocs)?;
        for _ in 0..n_relocs {
            let patch_off = r.u32()? as usize;
            let kind = match r.u8()? {
                0 => A64RelocKind::Branch26,
                1 => A64RelocKind::AdrpAdd,
                _ => return None,
            };
            let min_bytes = if kind == A64RelocKind::AdrpAdd { 8 } else { 4 };
            if (patch_off & 3) != 0 || patch_off.checked_add(min_bytes)? > code_len {
                return None;
            }
            let ttag = r.u8()?;
            let tval = r.u32()?;
            let target = match ttag {
                0 => {
                    if (tval as usize) >= expected_n_total {
                        return None;
                    }
                    A64RelocTarget::Func(tval)
                }
                1 => {
                    if (tval as usize) >= NUM_HELPERS {
                        return None;
                    }
                    A64RelocTarget::Helper(tval)
                }
                2 => A64RelocTarget::Rodata(tval),
                3 => {
                    if (tval as usize) > lro_len {
                        return None;
                    }
                    A64RelocTarget::LocalRodata(tval)
                }
                _ => return None,
            };
            relocs.push(A64Reloc {
                patch_off,
                kind,
                target,
            });
        }

        let n_jt = r.u32()? as usize;
        let mut jump_tables = try_vec_with_capacity(n_jt)?;
        for _ in 0..n_jt {
            let rodata_off = r.u32()?;
            let n_t = r.u32()? as usize;
            let jt_end = (rodata_off as usize).checked_add(n_t.checked_mul(4)?)?;
            if jt_end > lro_len {
                return None;
            }
            let mut targets = try_vec_with_capacity(n_t)?;
            for _ in 0..n_t {
                let toff = r.u32()?;
                if (toff as usize) > code_len || (toff & 3) != 0 {
                    return None;
                }
                targets.push(toff);
            }
            jump_tables.push((rodata_off, targets));
        }

        funcs.push(CachedA64Fn {
            raw_body_hash,
            was_inlinable,
            pre_inline_callees,
            inlined_callees,
            inline_added_insts,
            own_effects,
            callee_effects_hash,
            code,
            relocs,
            local_rodata,
            jump_tables,
        });
    }
    if r.pos != payload.len() {
        return None;
    }

    Some(A64IncCacheData {
        abi_hash,
        global_words,
        hit_inline_cap,
        funcs,
        backing: Some(std::sync::Arc::new(raw)),
    })
}

/// Atomically writes the AArch64 incremental cache file to `<cache_dir>/<abi_hash>.bin`.
pub fn save_a64_inc_cache(cache_dir: &Path, data: &A64IncCacheData) {
    let _ = save_a64_inc_cache_inner(cache_dir, data);
}

fn save_a64_inc_cache_inner(cache_dir: &Path, data: &A64IncCacheData) -> Option<()> {
    ensure_private_a64_inc_dir(cache_dir)?;
    sweep_stale_tmp_files_with_cutoff(cache_dir, std::time::Duration::from_secs(300));

    let backing = data.backing.as_deref().map(Vec::as_slice);
    let est_size: usize = 128
        + data
            .funcs
            .iter()
            .map(|f| 128 + f.code.len() + f.local_rodata.len() + f.relocs.len() * 10)
            .sum::<usize>();
    if est_size > MAX_CACHE_FILE_BYTES {
        return None;
    }

    let mut w = BinWriter::try_with_capacity(est_size)?;
    w.bytes(MAGIC)?;
    w.u32(FORMAT_VERSION)?;
    w.bytes(&data.abi_hash)?;
    w.u32(u32::try_from(data.global_words).ok()?)?;
    w.u8(u8::from(data.hit_inline_cap))?;
    w.u32(u32::try_from(data.funcs.len()).ok()?)?;

    for f in &data.funcs {
        w.bytes(&f.raw_body_hash)?;
        w.u8(u8::from(f.was_inlinable))?;
        w.u32(u32::try_from(f.pre_inline_callees.len()).ok()?)?;
        for &(c, cnt) in &f.pre_inline_callees {
            w.u32(c)?;
            w.u32(cnt)?;
        }
        w.u32(u32::try_from(f.inlined_callees.len()).ok()?)?;
        for &c in &f.inlined_callees {
            w.u32(c)?;
        }
        w.u32(f.inline_added_insts)?;

        let eff_flags = u8::from(f.own_effects.writes_mem)
            | (u8::from(f.own_effects.grows_mem) << 1)
            | (u8::from(f.own_effects.writes_all_globals) << 2)
            | (u8::from(f.own_effects.writes_all_tables) << 3);
        w.u8(eff_flags)?;
        for wi in 0..data.global_words {
            w.u64(f.own_effects.written_globals.get(wi).copied().unwrap_or(0))?;
        }
        w.u64(f.own_effects.written_tables)?;
        w.u32(u32::try_from(f.own_effects.direct_callees.len()).ok()?)?;
        for &c in &f.own_effects.direct_callees {
            w.u32(c)?;
        }
        w.bytes(&f.callee_effects_hash)?;

        let code_slice = f.code.as_slice(backing);
        w.u32(u32::try_from(code_slice.len()).ok()?)?;
        w.bytes(code_slice)?;

        let lro_slice = f.local_rodata.as_slice(backing);
        w.u32(u32::try_from(lro_slice.len()).ok()?)?;
        w.bytes(lro_slice)?;

        w.u32(u32::try_from(f.relocs.len()).ok()?)?;
        for rel in &f.relocs {
            w.u32(u32::try_from(rel.patch_off).ok()?)?;
            w.u8(match rel.kind {
                A64RelocKind::Branch26 => 0,
                A64RelocKind::AdrpAdd => 1,
            })?;
            let (ttag, tval) = match rel.target {
                A64RelocTarget::Func(v) => (0u8, v),
                A64RelocTarget::Helper(v) => (1u8, v),
                A64RelocTarget::Rodata(v) => (2u8, v),
                A64RelocTarget::LocalRodata(v) => (3u8, v),
            };
            w.u8(ttag)?;
            w.u32(tval)?;
        }

        w.u32(u32::try_from(f.jump_tables.len()).ok()?)?;
        for (roff, targets) in &f.jump_tables {
            w.u32(*roff)?;
            w.u32(u32::try_from(targets.len()).ok()?)?;
            for &toff in targets {
                w.u32(toff)?;
            }
        }
    }

    if w.buf.len().checked_add(32)? > MAX_CACHE_FILE_BYTES {
        return None;
    }
    let digest = parallel_payload_digest(&w.buf)?;
    w.bytes(&digest)?;

    let seq = NEXT_A64_INC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = cache_dir.join(format!(".tmp.{}.{seq}", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&tmp_path).ok()?;
    if file.write_all(&w.buf).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return None;
    }
    drop(file);
    let final_path = cache_file_path(cache_dir, &data.abi_hash);
    if fs::rename(&tmp_path, &final_path).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return None;
    }
    Some(())
}

/// Builds a `CachedA64Fn` snapshot from a freshly compiled `A64Asm` using fallible allocations.
#[allow(clippy::too_many_arguments)]
pub fn try_build_cached_a64_fn(
    raw_body_hash: [u8; 32],
    was_inlinable: bool,
    pre_inline_callees: Vec<(u32, u32)>,
    inlined_callees: Vec<u32>,
    inline_added_insts: u32,
    own_effects: FnOwnEffects,
    callee_effects_hash: [u8; 32],
    asm: &A64Asm,
) -> Option<CachedA64Fn> {
    let code = try_copy_slice(&asm.code)?;
    let local_rodata = try_copy_slice(&asm.local_rodata)?;
    let relocs = try_copy_slice(&asm.relocs)?;
    let jump_tables = asm.resolved_jump_tables().ok()?;
    Some(CachedA64Fn {
        raw_body_hash,
        was_inlinable,
        pre_inline_callees,
        inlined_callees,
        inline_added_insts,
        own_effects,
        callee_effects_hash,
        code: CachedBytes::Owned(code),
        relocs,
        local_rodata: CachedBytes::Owned(local_rodata),
        jump_tables,
    })
}
