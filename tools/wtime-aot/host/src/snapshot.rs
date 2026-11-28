//! Binary in-memory `StoreSnapshot` capture & sub-millisecond pointer-rebasing restore.
//!
//! Capturing a `StoreSnapshot` runs a full Major GC to compact all live GC objects into
//! a contiguous Old Space prefix `[old_base .. old_alloc_ptr)`, then precomputes flat
//! relocation tables for GC heap pointers (`+ delta`) and `StoreFuncRef` pointers
//! (`(inst_idx, def_idx)` or `Host(hf_idx)`). Restoring into a fresh `Store` with the
//! same instantiated modules performs a single `memcpy` + linear pointer rebase without
//! re-executing initialization code.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use wtime_aot::sha256::Sha256;
use wtime_aot::x64::CTX_HEADER_BYTES;
use wtime_aot::x64::module::{
    GC_HDR_KIND_ARRAY, GC_HDR_KIND_INTERNAL_EXTERN, GC_HDR_KIND_STRUCT, GcStorageKind,
};
use wtime_aot::x64::store_rt::STORE_ABI_VERSION;

const SNAPSHOT_WIRE_MAGIC: &[u8; 8] = b"WTSN0001";
const SNAPSHOT_WIRE_VERSION: u32 = 1;

fn current_arch_tag() -> u8 {
    match std::env::consts::ARCH {
        "x86_64" => 1,
        "aarch64" => 2,
        _ => 0,
    }
}

use crate::abi::{StoreFuncRef, decode_host_extern_slot};
use crate::error::{Error, GcHeapOutOfMemory, Result};
use crate::gc::{
    EphemeronEntry, EqRef, ExternArenaSlot, GcEphemeron, OwnedRooted, RootedRef, WeakAnyRef,
    WeakRefEntry, collect_store_gc,
};
use crate::memory::WASM_PAGE_BYTES;
use crate::store::StoreInner;
use crate::types::CanonComposite;

/// Context passed to host `ExternRef` decoders during [`crate::Store::restore_snapshot_with_externs`]
/// so reconstructed host payloads can bind `WeakAnyRef` / `GcEphemeron` handles to the destination Store.
pub struct SnapshotRestoreContext {
    store_id: u64,
}

impl SnapshotRestoreContext {
    /// Reconstructs a [`WeakAnyRef`] bound to the destination Store from `(slot_idx, generation)`.
    pub fn weak_ref_from_raw_parts(&self, slot_idx: u32, generation: u32) -> WeakAnyRef {
        WeakAnyRef {
            store_id: self.store_id,
            slot_idx,
            generation,
        }
    }

    /// Reconstructs a [`GcEphemeron`] bound to the destination Store from `(slot_idx, generation)`.
    pub fn ephemeron_from_raw_parts(&self, slot_idx: u32, generation: u32) -> GcEphemeron {
        GcEphemeron {
            store_id: self.store_id,
            slot_idx,
            generation,
        }
    }
}

#[inline]
fn is_immediate_traced_word(w: u64) -> bool {
    w == 0 || (w & 1) != 0 || decode_host_extern_slot(w).is_some()
}

#[derive(Clone, Copy, Debug)]
enum FuncRefTarget {
    Defined { inst_idx: u32, def_idx: u32 },
    Host(u32),
}

#[derive(Clone, Copy, Debug)]
enum SnapshotRootWord {
    Immediate(u64),
    HeapOff(u64),
}

struct SnapshotInstance {
    module_fingerprint: [u8; 32],
    local_canon_to_global: Vec<u32>,
    ctx_len: usize,
    ctx_tail: Vec<u8>,
    gc_global_offs: Vec<i32>,
    func_global_patches: Vec<(i32, FuncRefTarget)>,
    elem_buf_bytes: usize,
    elem_bytes: Vec<u8>,
    elem_gc_offs: Vec<u32>,
}

struct SnapshotMemory {
    len_bytes: u64,
    nonzero_pages: Vec<(usize, Box<[u8; 4096]>)>,
}

struct SnapshotTable {
    cur_len: u32,
    is_traced: bool,
    traced_words: Vec<u64>,
    traced_gc_indices: Vec<u32>,
    func_entries: Vec<(u32, FuncRefTarget)>,
}

struct SnapshotStandaloneGlobal {
    is_traced: bool,
    words: [u64; 2],
    is_gc_ptr: bool,
    func_target: Option<FuncRefTarget>,
}

struct SnapshotWeakRef {
    owner_off: u64,
    target_off: u64,
    generation: u32,
    live: bool,
}

struct SnapshotEphemeron {
    owner_off: u64,
    key_off: u64,
    value: SnapshotRootWord,
    generation: u32,
    live: bool,
}

/// Shareable, immutable post-bootstrap snapshot of a `Store`'s compacted GC heap,
/// module globals, linear memories, tables, passive element buffers, and weak/ephemeron state.
pub struct StoreSnapshot {
    source_old_base: u64,
    heap_bytes: Vec<u8>,
    old_card_first_obj: Vec<u16>,
    gc_ref_offsets: Vec<u32>,
    func_ref_patches: Vec<(u32, FuncRefTarget)>,
    instances: Vec<SnapshotInstance>,
    memories: Vec<SnapshotMemory>,
    shared_memories: Vec<SnapshotMemory>,
    tables: Vec<SnapshotTable>,
    standalone_globals: Vec<SnapshotStandaloneGlobal>,
    host_func_canon_types: Vec<u32>,
    weak_tables_offs: Vec<u64>,
    weak_refs: Vec<SnapshotWeakRef>,
    weak_refs_free: Vec<u32>,
    weak_retire_count: u64,
    ephemerons: Vec<SnapshotEphemeron>,
    ephemerons_free: Vec<u32>,
    extern_arena_len: u32,
    extern_free: Vec<u32>,
    extern_entries: Vec<(u32, Vec<u8>)>,
    extra_roots: Vec<SnapshotRootWord>,
}

impl StoreSnapshot {
    /// Returns the compacted GC heap size in bytes captured in this snapshot.
    pub fn heap_bytes_len(&self) -> usize {
        self.heap_bytes.len()
    }

    /// Returns the number of Wasm module instances captured in this snapshot.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Encodes this snapshot into a self-contained, SHA-256 integrity-checked binary buffer
    /// suitable for persistence across process restarts or separate `Engine` instances.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.heap_bytes.len() + 4096);
        out.extend_from_slice(SNAPSHOT_WIRE_MAGIC);
        write_u32(&mut out, SNAPSHOT_WIRE_VERSION);
        write_u32(&mut out, STORE_ABI_VERSION);
        out.push(current_arch_tag());

        write_u64(&mut out, self.source_old_base);
        write_bytes(&mut out, &self.heap_bytes);

        write_u32(&mut out, self.old_card_first_obj.len() as u32);
        for &v in &self.old_card_first_obj {
            write_u16(&mut out, v);
        }

        write_u32(&mut out, self.gc_ref_offsets.len() as u32);
        for &off in &self.gc_ref_offsets {
            write_u32(&mut out, off);
        }

        write_u32(&mut out, self.func_ref_patches.len() as u32);
        for &(off, target) in &self.func_ref_patches {
            write_u32(&mut out, off);
            write_func_target(&mut out, target);
        }

        write_u32(&mut out, self.instances.len() as u32);
        for inst in &self.instances {
            out.extend_from_slice(&inst.module_fingerprint);
            write_u32(&mut out, inst.local_canon_to_global.len() as u32);
            for &cid in &inst.local_canon_to_global {
                write_u32(&mut out, cid);
            }
            write_u32(&mut out, inst.ctx_len as u32);
            write_bytes(&mut out, &inst.ctx_tail);
            write_u32(&mut out, inst.gc_global_offs.len() as u32);
            for &off in &inst.gc_global_offs {
                write_i32(&mut out, off);
            }
            write_u32(&mut out, inst.func_global_patches.len() as u32);
            for &(off, target) in &inst.func_global_patches {
                write_i32(&mut out, off);
                write_func_target(&mut out, target);
            }
            write_u32(&mut out, inst.elem_buf_bytes as u32);
            write_bytes(&mut out, &inst.elem_bytes);
            write_u32(&mut out, inst.elem_gc_offs.len() as u32);
            for &off in &inst.elem_gc_offs {
                write_u32(&mut out, off);
            }
        }

        write_memories(&mut out, &self.memories);
        write_memories(&mut out, &self.shared_memories);

        write_u32(&mut out, self.tables.len() as u32);
        for tbl in &self.tables {
            write_u32(&mut out, tbl.cur_len);
            out.push(u8::from(tbl.is_traced));
            write_u32(&mut out, tbl.traced_words.len() as u32);
            for &w in &tbl.traced_words {
                write_u64(&mut out, w);
            }
            write_u32(&mut out, tbl.traced_gc_indices.len() as u32);
            for &idx in &tbl.traced_gc_indices {
                write_u32(&mut out, idx);
            }
            write_u32(&mut out, tbl.func_entries.len() as u32);
            for &(idx, target) in &tbl.func_entries {
                write_u32(&mut out, idx);
                write_func_target(&mut out, target);
            }
        }

        write_u32(&mut out, self.standalone_globals.len() as u32);
        for sg in &self.standalone_globals {
            out.push(u8::from(sg.is_traced));
            write_u64(&mut out, sg.words[0]);
            write_u64(&mut out, sg.words[1]);
            out.push(u8::from(sg.is_gc_ptr));
            match sg.func_target {
                None => out.push(0),
                Some(target) => {
                    out.push(1);
                    write_func_target(&mut out, target);
                }
            }
        }

        write_u32(&mut out, self.host_func_canon_types.len() as u32);
        for &cid in &self.host_func_canon_types {
            write_u32(&mut out, cid);
        }

        write_u32(&mut out, self.weak_tables_offs.len() as u32);
        for &off in &self.weak_tables_offs {
            write_u64(&mut out, off);
        }

        write_u32(&mut out, self.weak_refs.len() as u32);
        for w in &self.weak_refs {
            write_u64(&mut out, w.owner_off);
            write_u64(&mut out, w.target_off);
            write_u32(&mut out, w.generation);
            out.push(u8::from(w.live));
        }

        write_u32(&mut out, self.weak_refs_free.len() as u32);
        for &idx in &self.weak_refs_free {
            write_u32(&mut out, idx);
        }
        write_u64(&mut out, self.weak_retire_count);

        write_u32(&mut out, self.ephemerons.len() as u32);
        for e in &self.ephemerons {
            write_u64(&mut out, e.owner_off);
            write_u64(&mut out, e.key_off);
            write_root_word(&mut out, e.value);
            write_u32(&mut out, e.generation);
            out.push(u8::from(e.live));
        }

        write_u32(&mut out, self.ephemerons_free.len() as u32);
        for &idx in &self.ephemerons_free {
            write_u32(&mut out, idx);
        }

        write_u32(&mut out, self.extern_arena_len);
        write_u32(&mut out, self.extern_free.len() as u32);
        for &idx in &self.extern_free {
            write_u32(&mut out, idx);
        }
        write_u32(&mut out, self.extern_entries.len() as u32);
        for (idx, payload) in &self.extern_entries {
            write_u32(&mut out, *idx);
            write_bytes(&mut out, payload);
        }

        write_u32(&mut out, self.extra_roots.len() as u32);
        for &rw in &self.extra_roots {
            write_root_word(&mut out, rw);
        }

        let mut hasher = Sha256::new();
        hasher.update(&out);
        let digest = hasher.finish();
        out.extend_from_slice(&digest);
        out
    }

    /// Decodes and integrity-verifies a serialized [`StoreSnapshot`] produced by [`Self::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < SNAPSHOT_WIRE_MAGIC.len() + 4 + 4 + 1 + 32 {
            return Err(Error::msg("StoreSnapshot bytes too short"));
        }
        let payload_len = bytes.len() - 32;
        let payload = &bytes[..payload_len];
        let expected_digest = &bytes[payload_len..];
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let actual_digest = hasher.finish();
        if actual_digest.as_slice() != expected_digest {
            return Err(Error::msg("StoreSnapshot SHA-256 checksum mismatch"));
        }

        let mut r = SnapReader::new(payload);
        let magic = r.read_exact(8)?;
        if magic != SNAPSHOT_WIRE_MAGIC {
            return Err(Error::msg("invalid StoreSnapshot magic"));
        }
        let version = r.read_u32()?;
        if version != SNAPSHOT_WIRE_VERSION {
            return Err(Error::msg(format!(
                "unsupported StoreSnapshot wire version {version}"
            )));
        }
        let abi = r.read_u32()?;
        if abi != STORE_ABI_VERSION {
            return Err(Error::msg(format!(
                "StoreSnapshot ABI version mismatch: got {abi}, expected {STORE_ABI_VERSION}"
            )));
        }
        let arch = r.read_u8()?;
        if arch != current_arch_tag() {
            return Err(Error::msg(format!(
                "StoreSnapshot target architecture mismatch: got {arch}, expected {}",
                current_arch_tag()
            )));
        }

        let source_old_base = r.read_u64()?;
        let heap_bytes = r.read_bytes()?;

        let card_len = r.read_count(2)?;
        let mut old_card_first_obj = Vec::with_capacity(card_len);
        for _ in 0..card_len {
            old_card_first_obj.push(r.read_u16()?);
        }

        let gc_ref_len = r.read_count(4)?;
        let mut gc_ref_offsets = Vec::with_capacity(gc_ref_len);
        for _ in 0..gc_ref_len {
            gc_ref_offsets.push(r.read_u32()?);
        }

        let func_patch_len = r.read_count(9)?;
        let mut func_ref_patches = Vec::with_capacity(func_patch_len);
        for _ in 0..func_patch_len {
            let off = r.read_u32()?;
            let target = r.read_func_target()?;
            func_ref_patches.push((off, target));
        }

        let inst_len = r.read_count(32)?;
        let mut instances = Vec::with_capacity(inst_len);
        for _ in 0..inst_len {
            let mut module_fingerprint = [0u8; 32];
            module_fingerprint.copy_from_slice(r.read_exact(32)?);
            let canon_len = r.read_count(4)?;
            let mut local_canon_to_global = Vec::with_capacity(canon_len);
            for _ in 0..canon_len {
                local_canon_to_global.push(r.read_u32()?);
            }
            let ctx_len = r.read_u32()? as usize;
            let ctx_tail = r.read_bytes()?;
            let gc_g_len = r.read_count(4)?;
            let mut gc_global_offs = Vec::with_capacity(gc_g_len);
            for _ in 0..gc_g_len {
                gc_global_offs.push(r.read_i32()?);
            }
            let fg_len = r.read_count(9)?;
            let mut func_global_patches = Vec::with_capacity(fg_len);
            for _ in 0..fg_len {
                let off = r.read_i32()?;
                let target = r.read_func_target()?;
                func_global_patches.push((off, target));
            }
            let elem_buf_bytes = r.read_u32()? as usize;
            let elem_bytes = r.read_bytes()?;
            let eg_len = r.read_count(4)?;
            let mut elem_gc_offs = Vec::with_capacity(eg_len);
            for _ in 0..eg_len {
                elem_gc_offs.push(r.read_u32()?);
            }
            instances.push(SnapshotInstance {
                module_fingerprint,
                local_canon_to_global,
                ctx_len,
                ctx_tail,
                gc_global_offs,
                func_global_patches,
                elem_buf_bytes,
                elem_bytes,
                elem_gc_offs,
            });
        }

        let memories = read_memories(&mut r)?;
        let shared_memories = read_memories(&mut r)?;

        let tbl_len = r.read_count(17)?;
        let mut tables = Vec::with_capacity(tbl_len);
        for _ in 0..tbl_len {
            let cur_len = r.read_u32()?;
            let is_traced = r.read_bool()?;
            let tw_len = r.read_count(8)?;
            let mut traced_words = Vec::with_capacity(tw_len);
            for _ in 0..tw_len {
                traced_words.push(r.read_u64()?);
            }
            let ti_len = r.read_count(4)?;
            let mut traced_gc_indices = Vec::with_capacity(ti_len);
            for _ in 0..ti_len {
                traced_gc_indices.push(r.read_u32()?);
            }
            let fe_len = r.read_count(9)?;
            let mut func_entries = Vec::with_capacity(fe_len);
            for _ in 0..fe_len {
                let idx = r.read_u32()?;
                let target = r.read_func_target()?;
                func_entries.push((idx, target));
            }
            tables.push(SnapshotTable {
                cur_len,
                is_traced,
                traced_words,
                traced_gc_indices,
                func_entries,
            });
        }

        let sg_len = r.read_count(19)?;
        let mut standalone_globals = Vec::with_capacity(sg_len);
        for _ in 0..sg_len {
            let is_traced = r.read_bool()?;
            let w0 = r.read_u64()?;
            let w1 = r.read_u64()?;
            let is_gc_ptr = r.read_bool()?;
            let func_target = match r.read_u8()? {
                0 => None,
                1 => Some(r.read_func_target()?),
                other => {
                    return Err(Error::msg(format!(
                        "StoreSnapshot invalid standalone global func_target tag {other}"
                    )));
                }
            };
            standalone_globals.push(SnapshotStandaloneGlobal {
                is_traced,
                words: [w0, w1],
                is_gc_ptr,
                func_target,
            });
        }

        let hf_len = r.read_count(4)?;
        let mut host_func_canon_types = Vec::with_capacity(hf_len);
        for _ in 0..hf_len {
            host_func_canon_types.push(r.read_u32()?);
        }

        let wt_len = r.read_count(8)?;
        let mut weak_tables_offs = Vec::with_capacity(wt_len);
        for _ in 0..wt_len {
            weak_tables_offs.push(r.read_u64()?);
        }

        let wr_len = r.read_count(21)?;
        let mut weak_refs = Vec::with_capacity(wr_len);
        for _ in 0..wr_len {
            let owner_off = r.read_u64()?;
            let target_off = r.read_u64()?;
            let generation = r.read_u32()?;
            let live = r.read_bool()?;
            weak_refs.push(SnapshotWeakRef {
                owner_off,
                target_off,
                generation,
                live,
            });
        }

        let wrf_len = r.read_count(4)?;
        let mut weak_refs_free = Vec::with_capacity(wrf_len);
        for _ in 0..wrf_len {
            weak_refs_free.push(r.read_u32()?);
        }
        let weak_retire_count = r.read_u64()?;

        let eph_len = r.read_count(30)?;
        let mut ephemerons = Vec::with_capacity(eph_len);
        for _ in 0..eph_len {
            let owner_off = r.read_u64()?;
            let key_off = r.read_u64()?;
            let value = r.read_root_word()?;
            let generation = r.read_u32()?;
            let live = r.read_bool()?;
            ephemerons.push(SnapshotEphemeron {
                owner_off,
                key_off,
                value,
                generation,
                live,
            });
        }

        let ephf_len = r.read_count(4)?;
        let mut ephemerons_free = Vec::with_capacity(ephf_len);
        for _ in 0..ephf_len {
            ephemerons_free.push(r.read_u32()?);
        }

        let extern_arena_len = r.read_u32()?;
        let extf_len = r.read_count(4)?;
        let mut extern_free = Vec::with_capacity(extf_len);
        for _ in 0..extf_len {
            extern_free.push(r.read_u32()?);
        }
        let exte_len = r.read_count(8)?;
        let mut extern_entries = Vec::with_capacity(exte_len);
        for _ in 0..exte_len {
            let slot_idx = r.read_u32()?;
            let payload = r.read_bytes()?;
            extern_entries.push((slot_idx, payload));
        }

        let er_len = r.read_count(9)?;
        let mut extra_roots = Vec::with_capacity(er_len);
        for _ in 0..er_len {
            extra_roots.push(r.read_root_word()?);
        }

        if !r.is_empty() {
            return Err(Error::msg("StoreSnapshot has unexpected trailing bytes"));
        }

        Ok(Self {
            source_old_base,
            heap_bytes,
            old_card_first_obj,
            gc_ref_offsets,
            func_ref_patches,
            instances,
            memories,
            shared_memories,
            tables,
            standalone_globals,
            host_func_canon_types,
            weak_tables_offs,
            weak_refs,
            weak_refs_free,
            weak_retire_count,
            ephemerons,
            ephemerons_free,
            extern_arena_len,
            extern_free,
            extern_entries,
            extra_roots,
        })
    }
}

#[inline]
fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
fn write_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}
#[inline]
fn write_func_target(out: &mut Vec<u8>, target: FuncRefTarget) {
    match target {
        FuncRefTarget::Defined { inst_idx, def_idx } => {
            out.push(0);
            write_u32(out, inst_idx);
            write_u32(out, def_idx);
        }
        FuncRefTarget::Host(hf_idx) => {
            out.push(1);
            write_u32(out, hf_idx);
        }
    }
}
#[inline]
fn write_root_word(out: &mut Vec<u8>, rw: SnapshotRootWord) {
    match rw {
        SnapshotRootWord::Immediate(w) => {
            out.push(0);
            write_u64(out, w);
        }
        SnapshotRootWord::HeapOff(off) => {
            out.push(1);
            write_u64(out, off);
        }
    }
}
fn write_memories(out: &mut Vec<u8>, mems: &[SnapshotMemory]) {
    write_u32(out, mems.len() as u32);
    for m in mems {
        write_u64(out, m.len_bytes);
        write_u32(out, m.nonzero_pages.len() as u32);
        for (off, page) in &m.nonzero_pages {
            write_u64(out, *off as u64);
            out.extend_from_slice(&page[..]);
        }
    }
}

struct SnapReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SnapReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
    fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }
    fn read_exact(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::msg("StoreSnapshot read overflow"))?;
        if end > self.buf.len() {
            return Err(Error::msg("StoreSnapshot unexpected EOF"));
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }
    fn read_bool(&mut self) -> Result<bool> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(Error::msg(format!(
                "StoreSnapshot invalid bool byte {other}"
            ))),
        }
    }
    fn read_u16(&mut self) -> Result<u16> {
        let b = self.read_exact(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn read_u32(&mut self) -> Result<u32> {
        let b = self.read_exact(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_i32(&mut self) -> Result<i32> {
        let b = self.read_exact(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_u64(&mut self) -> Result<u64> {
        let b = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn read_count(&mut self, min_elem_bytes: usize) -> Result<usize> {
        let n = self.read_u32()? as usize;
        let needed = n
            .checked_mul(min_elem_bytes)
            .ok_or_else(|| Error::msg("StoreSnapshot count overflow"))?;
        if needed > self.remaining() {
            return Err(Error::msg("StoreSnapshot count exceeds remaining payload"));
        }
        Ok(n)
    }
    fn read_bytes(&mut self) -> Result<Vec<u8>> {
        let n = self.read_count(1)?;
        Ok(self.read_exact(n)?.to_vec())
    }
    fn read_func_target(&mut self) -> Result<FuncRefTarget> {
        match self.read_u8()? {
            0 => Ok(FuncRefTarget::Defined {
                inst_idx: self.read_u32()?,
                def_idx: self.read_u32()?,
            }),
            1 => Ok(FuncRefTarget::Host(self.read_u32()?)),
            other => Err(Error::msg(format!(
                "StoreSnapshot invalid FuncRefTarget tag {other}"
            ))),
        }
    }
    fn read_root_word(&mut self) -> Result<SnapshotRootWord> {
        match self.read_u8()? {
            0 => Ok(SnapshotRootWord::Immediate(self.read_u64()?)),
            1 => Ok(SnapshotRootWord::HeapOff(self.read_u64()?)),
            other => Err(Error::msg(format!(
                "StoreSnapshot invalid SnapshotRootWord tag {other}"
            ))),
        }
    }
}

fn read_memories(r: &mut SnapReader<'_>) -> Result<Vec<SnapshotMemory>> {
    let m_len = r.read_count(12)?;
    let mut mems = Vec::with_capacity(m_len);
    for _ in 0..m_len {
        let len_bytes = r.read_u64()?;
        let p_len = r.read_count(8 + 4096)?;
        let mut nonzero_pages = Vec::with_capacity(p_len);
        for _ in 0..p_len {
            let off = r.read_u64()? as usize;
            let page_slice = r.read_exact(4096)?;
            let mut page = Box::new([0u8; 4096]);
            page.copy_from_slice(page_slice);
            nonzero_pages.push((off, page));
        }
        mems.push(SnapshotMemory {
            len_bytes,
            nonzero_pages,
        });
    }
    Ok(mems)
}

struct FuncRefClassifier {
    def_ranges: Vec<(u64, u64, u32)>,
    host_map: HashMap<u64, u32>,
}

impl FuncRefClassifier {
    fn from_store(store: &StoreInner) -> Self {
        let mut def_ranges = Vec::with_capacity(store.instances.len());
        for (inst_idx, inst) in store.instances.iter().enumerate() {
            let base = inst._def_func_refs.as_ptr() as u64;
            let len = inst._def_func_refs.len() as u64;
            if len > 0 {
                let end = base + len * (std::mem::size_of::<StoreFuncRef>() as u64);
                def_ranges.push((base, end, inst_idx as u32));
            }
        }
        let mut host_map = HashMap::with_capacity(store.host_funcs.len());
        for (hf_idx, hf) in store.host_funcs.iter().enumerate() {
            let ptr = hf.func_ref.as_ref() as *const StoreFuncRef as u64;
            host_map.insert(ptr, hf_idx as u32);
        }
        Self {
            def_ranges,
            host_map,
        }
    }

    fn classify(&self, ptr: u64) -> Result<FuncRefTarget> {
        let stride = std::mem::size_of::<StoreFuncRef>() as u64;
        for &(base, end, inst_idx) in &self.def_ranges {
            if ptr >= base && ptr < end {
                let off = ptr - base;
                if off.is_multiple_of(stride) {
                    return Ok(FuncRefTarget::Defined {
                        inst_idx,
                        def_idx: (off / stride) as u32,
                    });
                }
            }
        }
        if let Some(&hf_idx) = self.host_map.get(&ptr) {
            return Ok(FuncRefTarget::Host(hf_idx));
        }
        Err(Error::msg(format!(
            "StoreSnapshot encountered unrecognized StoreFuncRef pointer {ptr:#x}"
        )))
    }
}

#[inline]
fn resolve_func_ref(store: &StoreInner, target: FuncRefTarget) -> Result<u64> {
    match target {
        FuncRefTarget::Defined { inst_idx, def_idx } => {
            let inst = store
                .instances
                .get(inst_idx as usize)
                .ok_or_else(|| Error::msg("StoreSnapshot instance index out of bounds"))?;
            if (def_idx as usize) >= inst._def_func_refs.len() {
                return Err(Error::msg(
                    "StoreSnapshot defined function index out of bounds",
                ));
            }
            Ok(unsafe { inst._def_func_refs.as_ptr().add(def_idx as usize) } as u64)
        }
        FuncRefTarget::Host(hf_idx) => {
            let hf = store
                .host_funcs
                .get(hf_idx as usize)
                .ok_or_else(|| Error::msg("StoreSnapshot host function index out of bounds"))?;
            Ok(hf.func_ref.as_ref() as *const StoreFuncRef as u64)
        }
    }
}

#[inline]
fn is_funcref_val_type(store: &StoreInner, vt: wtime_types::ValType) -> bool {
    match vt {
        wtime_types::ValType::Ref(rt) => match rt.heap {
            wtime_types::HeapType::Func | wtime_types::HeapType::NoFunc => true,
            wtime_types::HeapType::Defined(idx) | wtime_types::HeapType::ExactDefined(idx) => store
                .types_snapshot
                .get(idx.0 as usize)
                .is_some_and(|info| matches!(info.composite, CanonComposite::Func { .. })),
            _ => false,
        },
        _ => false,
    }
}

#[inline]
fn is_exnref_val_type(vt: wtime_types::ValType) -> bool {
    matches!(
        vt,
        wtime_types::ValType::Ref(rt)
            if matches!(rt.heap, wtime_types::HeapType::Exn | wtime_types::HeapType::NoExn)
    )
}

fn capture_memory_pages(data: &[u8]) -> SnapshotMemory {
    let mut nonzero_pages = Vec::new();
    let mut off = 0usize;
    while off < data.len() {
        let end = (off + 4096).min(data.len());
        let chunk = &data[off..end];
        let is_zero = if chunk.len() == 4096 {
            let words = unsafe { std::slice::from_raw_parts(chunk.as_ptr().cast::<u64>(), 512) };
            words.iter().all(|&w| w == 0)
        } else {
            chunk.iter().all(|&b| b == 0)
        };
        if !is_zero {
            let mut page = Box::new([0u8; 4096]);
            page[..chunk.len()].copy_from_slice(chunk);
            nonzero_pages.push((off, page));
        }
        off += 4096;
    }
    SnapshotMemory {
        len_bytes: data.len() as u64,
        nonzero_pages,
    }
}

pub(crate) fn capture_store_snapshot<R: RootedRef<EqRef>>(
    store: &mut StoreInner,
    extra_roots: &[R],
) -> Result<StoreSnapshot> {
    capture_store_snapshot_with_externs(store, extra_roots, |_| {
        Err(Error::msg(
            "cannot capture StoreSnapshot while live opaque host ExternRef payloads exist",
        ))
    })
}

pub(crate) fn capture_store_snapshot_with_externs<R, F>(
    store: &mut StoreInner,
    extra_roots: &[R],
    mut encode_extern: F,
) -> Result<StoreSnapshot>
where
    R: RootedRef<EqRef>,
    F: FnMut(&dyn Any) -> Result<Vec<u8>>,
{
    // 1. Run a full Major GC so ALL live GC objects are compacted into contiguous active Old Space,
    //    Nursery is empty, and all unreachable ExternRefs / WeakRefs / Ephemerons are swept.
    unsafe { collect_store_gc(store, 0, 0, true).map_err(Error::from)? };

    if store.state.exn_active_ptr != 0 {
        return Err(Error::msg(
            "cannot capture StoreSnapshot with an active uncaught Wasm exception",
        ));
    }
    let extern_arena_len = u32::try_from(store.gc.extern_arena.len())
        .map_err(|_| Error::msg("StoreSnapshot extern_arena length overflow"))?;
    let mut extern_entries: Vec<(u32, Vec<u8>)> = Vec::new();
    for (idx, slot) in store.gc.extern_arena.iter().enumerate() {
        if let Some(ref payload) = slot.data {
            let encoded = encode_extern(payload.as_ref())?;
            extern_entries.push((idx as u32, encoded));
        }
    }
    let extern_free = store.gc.extern_free.clone();

    store.sync_type_descs();
    let types_snap = Arc::clone(&store.types_snapshot);

    let old_base = store.gc.active_old_base();
    let old_end = store.gc.old_alloc_ptr;
    debug_assert_eq!(
        store.state.gc_alloc_ptr,
        store.gc.nursery_base(),
        "Nursery must be empty immediately after Major GC"
    );

    let live_bytes = (old_end - old_base) as usize;
    let heap_bytes = if live_bytes > 0 {
        unsafe { std::slice::from_raw_parts(old_base as *const u8, live_bytes) }.to_vec()
    } else {
        Vec::new()
    };

    let classifier = FuncRefClassifier::from_store(store);

    // 2. Scan the compacted Old Space heap once to precompute flat GC pointer and FuncRef patch lists.
    let mut gc_ref_offsets: Vec<u32> = Vec::new();
    let mut func_ref_patches: Vec<(u32, FuncRefTarget)> = Vec::new();
    let mut cur = old_base;
    while cur < old_end {
        let hdr = unsafe { *(cur as *const u64) };
        let kind = (hdr as u32) & 6;
        match kind {
            GC_HDR_KIND_STRUCT => {
                let cid = (hdr as u32) >> 3;
                let CanonComposite::Struct {
                    fields,
                    total_bytes,
                    ..
                } = &types_snap[cid as usize].composite
                else {
                    return Err(Error::msg("corrupt GC struct header during snapshot"));
                };
                for f in fields {
                    let slot_addr = cur + u64::from(f.offset);
                    match f.storage {
                        GcStorageKind::GcRef | GcStorageKind::ExternRef => {
                            let w = unsafe { *(slot_addr as *const u64) };
                            if !is_immediate_traced_word(w) {
                                if w < old_base || w >= old_end {
                                    return Err(Error::msg(format!(
                                        "StoreSnapshot saw out-of-heap traced ref {w:#x}"
                                    )));
                                }
                                gc_ref_offsets.push((slot_addr - old_base) as u32);
                            }
                        }
                        GcStorageKind::OtherRef => {
                            let w = unsafe { *(slot_addr as *const u64) };
                            if w != 0 {
                                let target = classifier.classify(w)?;
                                func_ref_patches.push(((slot_addr - old_base) as u32, target));
                            }
                        }
                        _ => {}
                    }
                }
                cur += u64::from(*total_bytes);
            }
            GC_HDR_KIND_ARRAY => {
                let cid = (hdr as u32) >> 3;
                let len = unsafe { *((cur + 4) as *const u32) } as u64;
                let CanonComposite::Array { elem, .. } = &types_snap[cid as usize].composite else {
                    return Err(Error::msg("corrupt GC array header during snapshot"));
                };
                match *elem {
                    GcStorageKind::GcRef | GcStorageKind::ExternRef => {
                        for i in 0..len {
                            let slot_addr = cur + 8 + i * 8;
                            let w = unsafe { *(slot_addr as *const u64) };
                            if !is_immediate_traced_word(w) {
                                if w < old_base || w >= old_end {
                                    return Err(Error::msg(format!(
                                        "StoreSnapshot saw out-of-heap array ref {w:#x}"
                                    )));
                                }
                                gc_ref_offsets.push((slot_addr - old_base) as u32);
                            }
                        }
                    }
                    GcStorageKind::OtherRef => {
                        for i in 0..len {
                            let slot_addr = cur + 8 + i * 8;
                            let w = unsafe { *(slot_addr as *const u64) };
                            if w != 0 {
                                let target = classifier.classify(w)?;
                                func_ref_patches.push(((slot_addr - old_base) as u32, target));
                            }
                        }
                    }
                    _ => {}
                }
                cur += ((8u64 + len * u64::from(elem.byte_size())) + 7) & !7;
            }
            GC_HDR_KIND_INTERNAL_EXTERN => {
                let payload = unsafe { *((cur + 8) as *const u64) };
                if !is_immediate_traced_word(payload) {
                    return Err(Error::msg(
                        "StoreSnapshot encountered live InternalExternBox with unknown payload",
                    ));
                }
                cur += 16;
            }
            other => {
                return Err(Error::msg(format!(
                    "corrupt GC object header kind {other} during snapshot"
                )));
            }
        }
    }

    // 3. Capture per-instance `ctx_buf` tail (defined globals + data_dropped + elem_dropped) and `elem_buf`.
    let mut instances = Vec::with_capacity(store.instances.len());
    for inst in &store.instances {
        let meta = &inst.module.inner.meta;
        let ctx_ptr = inst.ctx_buf.as_ptr();
        let ctx_tail = inst.ctx_buf[CTX_HEADER_BYTES..].to_vec();
        let mut gc_global_offs = Vec::new();
        let mut func_global_patches = Vec::new();

        for &(_gi, vt, _mutable, off, is_traced) in &meta.defined_globals {
            if off < 0 {
                continue;
            }
            let word = unsafe { *(ctx_ptr.offset(off as isize).cast::<u64>()) };
            if is_traced {
                if !is_immediate_traced_word(word) {
                    if word < old_base || word >= old_end {
                        return Err(Error::msg(format!(
                            "StoreSnapshot saw out-of-heap traced global {word:#x}"
                        )));
                    }
                    gc_global_offs.push(off);
                }
            } else if is_funcref_val_type(store, vt) {
                if word != 0 {
                    func_global_patches.push((off, classifier.classify(word)?));
                }
            } else if is_exnref_val_type(vt) && word != 0 {
                return Err(Error::msg(
                    "StoreSnapshot does not support live ExnRef globals",
                ));
            }
        }

        let (elem_bytes, elem_gc_offs) = if !inst.elem_buf.is_null() && inst.elem_buf_bytes > 0 {
            let bytes =
                unsafe { std::slice::from_raw_parts(inst.elem_buf, inst.elem_buf_bytes) }.to_vec();
            let mut offs = Vec::new();
            let elem_dropped_base = meta
                .elem_dropped_off
                .map(|off| unsafe { ctx_ptr.offset(off as isize) });
            for &(seg_idx, byte_off, count) in &meta.passive_gc_elem_ranges {
                let dropped =
                    elem_dropped_base.is_some_and(|p| unsafe { *p.add(seg_idx as usize) != 0 });
                if dropped {
                    continue;
                }
                for i in 0..count {
                    let off_bytes = (byte_off as u32) + i * 8;
                    let w = unsafe { *(inst.elem_buf.add(off_bytes as usize).cast::<u64>()) };
                    if !is_immediate_traced_word(w) {
                        if w < old_base || w >= old_end {
                            return Err(Error::msg(format!(
                                "StoreSnapshot saw out-of-heap passive elem ref {w:#x}"
                            )));
                        }
                        offs.push(off_bytes);
                    }
                }
            }
            (bytes, offs)
        } else {
            (Vec::new(), Vec::new())
        };

        instances.push(SnapshotInstance {
            module_fingerprint: inst.module.fingerprint(),
            local_canon_to_global: inst.module.inner.local_canon_to_global.to_vec(),
            ctx_len: inst.ctx_buf.len(),
            ctx_tail,
            gc_global_offs,
            func_global_patches,
            elem_buf_bytes: inst.elem_buf_bytes,
            elem_bytes,
            elem_gc_offs,
        });
    }

    // 4. Capture private & shared linear memories (sparse non-zero 4 KiB pages).
    let memories: Vec<SnapshotMemory> = store
        .memories
        .iter()
        .map(|m| capture_memory_pages(m.data()))
        .collect();
    let shared_memories: Vec<SnapshotMemory> = store
        .shared_memories
        .iter()
        .map(|sm| {
            let len = sm.desc.len_bytes.load(Ordering::Acquire) as usize;
            let data = if len == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(sm.desc.base, len) }
            };
            capture_memory_pages(data)
        })
        .collect();

    // 5. Capture dynamic tables.
    let mut tables = Vec::with_capacity(store.tables.len());
    for tbl in &store.tables {
        let cur_len = tbl.desc.cur_len;
        let words = if cur_len == 0 || tbl.desc.ptr.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(tbl.desc.ptr, cur_len as usize) }
        };
        if tbl.is_traced {
            let mut gc_indices = Vec::new();
            for (idx, &w) in words.iter().enumerate() {
                if !is_immediate_traced_word(w) {
                    if w < old_base || w >= old_end {
                        return Err(Error::msg(format!(
                            "StoreSnapshot saw out-of-heap table ref {w:#x}"
                        )));
                    }
                    gc_indices.push(idx as u32);
                }
            }
            tables.push(SnapshotTable {
                cur_len,
                is_traced: true,
                traced_words: words.to_vec(),
                traced_gc_indices: gc_indices,
                func_entries: Vec::new(),
            });
        } else {
            let wvt = wtime_types::ValType::Ref(tbl.ty.element().to_wasm());
            let is_func = is_funcref_val_type(store, wvt);
            let is_exn = is_exnref_val_type(wvt);
            let mut func_entries = Vec::new();
            for (idx, &w) in words.iter().enumerate() {
                if w != 0 {
                    if is_func {
                        func_entries.push((idx as u32, classifier.classify(w)?));
                    } else if is_exn {
                        return Err(Error::msg(
                            "StoreSnapshot does not support live ExnRef table elements",
                        ));
                    }
                }
            }
            tables.push(SnapshotTable {
                cur_len,
                is_traced: false,
                traced_words: Vec::new(),
                traced_gc_indices: Vec::new(),
                func_entries,
            });
        }
    }

    // 6. Capture standalone globals.
    let mut standalone_globals = Vec::with_capacity(store.standalone_globals.len());
    for g in &store.standalone_globals {
        let words = *g.cell;
        let mut is_gc_ptr = false;
        let mut func_target = None;
        if g.is_traced {
            let w = words[0];
            if !is_immediate_traced_word(w) {
                if w < old_base || w >= old_end {
                    return Err(Error::msg(format!(
                        "StoreSnapshot saw out-of-heap standalone global {w:#x}"
                    )));
                }
                is_gc_ptr = true;
            }
        } else {
            let wvt = g._ty.content().to_wasm();
            if is_funcref_val_type(store, wvt) && words[0] != 0 {
                func_target = Some(classifier.classify(words[0])?);
            } else if is_exnref_val_type(wvt) && words[0] != 0 {
                return Err(Error::msg(
                    "StoreSnapshot does not support live ExnRef standalone globals",
                ));
            }
        }
        standalone_globals.push(SnapshotStandaloneGlobal {
            is_traced: g.is_traced,
            words,
            is_gc_ptr,
            func_target,
        });
    }

    let host_func_canon_types: Vec<u32> = store
        .host_funcs
        .iter()
        .map(|hf| hf.func_ref.canon_type_id)
        .collect();

    // 7. Capture WeakMap tables, WeakRefs, Ephemerons, and caller's extra roots.
    let mut weak_tables_offs = Vec::with_capacity(store.gc.weak_tables.len());
    for &ptr in &store.gc.weak_tables {
        if ptr < old_base || ptr >= old_end {
            return Err(Error::msg("StoreSnapshot saw out-of-heap weak table"));
        }
        weak_tables_offs.push(ptr - old_base);
    }

    let mut weak_refs = Vec::with_capacity(store.gc.weak_refs.len());
    for w in &store.gc.weak_refs {
        if w.live {
            if w.owner < old_base
                || w.owner >= old_end
                || w.target < old_base
                || w.target >= old_end
            {
                return Err(Error::msg("StoreSnapshot saw out-of-heap WeakAnyRef"));
            }
            weak_refs.push(SnapshotWeakRef {
                owner_off: w.owner - old_base,
                target_off: w.target - old_base,
                generation: w.generation,
                live: true,
            });
        } else {
            weak_refs.push(SnapshotWeakRef {
                owner_off: 0,
                target_off: 0,
                generation: w.generation,
                live: false,
            });
        }
    }

    let mut ephemerons = Vec::with_capacity(store.gc.ephemerons.len());
    for e in &store.gc.ephemerons {
        if e.live {
            if e.owner < old_base || e.owner >= old_end || e.key < old_base || e.key >= old_end {
                return Err(Error::msg("StoreSnapshot saw out-of-heap GcEphemeron"));
            }
            let val = if is_immediate_traced_word(e.value) {
                SnapshotRootWord::Immediate(e.value)
            } else {
                if e.value < old_base || e.value >= old_end {
                    return Err(Error::msg("StoreSnapshot saw out-of-heap ephemeron value"));
                }
                SnapshotRootWord::HeapOff(e.value - old_base)
            };
            ephemerons.push(SnapshotEphemeron {
                owner_off: e.owner - old_base,
                key_off: e.key - old_base,
                value: val,
                generation: e.generation,
                live: true,
            });
        } else {
            ephemerons.push(SnapshotEphemeron {
                owner_off: 0,
                key_off: 0,
                value: SnapshotRootWord::Immediate(0),
                generation: e.generation,
                live: false,
            });
        }
    }

    let mut captured_roots = Vec::with_capacity(extra_roots.len());
    for r in extra_roots {
        let (sid, idx, gen_id) = r.root_handle();
        if sid != store.id {
            return Err(Error::msg("capture_snapshot root belongs to another Store"));
        }
        let w = store.gc.read_host_root(idx, gen_id)?;
        if is_immediate_traced_word(w) {
            captured_roots.push(SnapshotRootWord::Immediate(w));
        } else {
            if w < old_base || w >= old_end {
                return Err(Error::msg(format!(
                    "capture_snapshot root pointer {w:#x} is outside compacted Old Space"
                )));
            }
            captured_roots.push(SnapshotRootWord::HeapOff(w - old_base));
        }
    }

    Ok(StoreSnapshot {
        source_old_base: old_base,
        heap_bytes,
        old_card_first_obj: store.gc.old_card_first_obj.clone(),
        gc_ref_offsets,
        func_ref_patches,
        instances,
        memories,
        shared_memories,
        tables,
        standalone_globals,
        host_func_canon_types,
        weak_tables_offs,
        weak_refs,
        weak_refs_free: store.gc.weak_refs_free.clone(),
        weak_retire_count: store.gc.weak_retire_count,
        ephemerons,
        ephemerons_free: store.gc.ephemerons_free.clone(),
        extern_arena_len,
        extern_free,
        extern_entries,
        extra_roots: captured_roots,
    })
}

pub(crate) fn restore_store_snapshot(
    store: &mut StoreInner,
    snap: &StoreSnapshot,
) -> Result<Vec<OwnedRooted<EqRef>>> {
    restore_store_snapshot_with_externs(store, snap, |_, _| {
        Err(Error::msg(
            "StoreSnapshot contains host ExternRef entries; use restore_snapshot_with_externs",
        ))
    })
}

pub(crate) fn restore_store_snapshot_with_externs<F>(
    store: &mut StoreInner,
    snap: &StoreSnapshot,
    mut decode_extern: F,
) -> Result<Vec<OwnedRooted<EqRef>>>
where
    F: FnMut(&SnapshotRestoreContext, &[u8]) -> Result<Box<dyn Any + Send + Sync>>,
{
    store.drain_dropped_owned_roots();
    store.interrupt.restore_all_rw();

    // 1. Validate exact structural compatibility before mutating destination Store state.
    if store.instances.len() != snap.instances.len() {
        return Err(Error::msg(format!(
            "StoreSnapshot instance count mismatch: expected {}, got {}",
            snap.instances.len(),
            store.instances.len()
        )));
    }
    let mut cid_remap: Option<HashMap<u32, u32>> = None;
    for (inst, sinst) in store.instances.iter().zip(snap.instances.iter()) {
        if inst.module.fingerprint() != sinst.module_fingerprint
            || inst.ctx_buf.len() != sinst.ctx_len
            || sinst.ctx_tail.len().saturating_add(CTX_HEADER_BYTES) != sinst.ctx_len
            || inst.elem_buf_bytes != sinst.elem_buf_bytes
            || (!sinst.elem_bytes.is_empty() && sinst.elem_bytes.len() != sinst.elem_buf_bytes)
            || inst.module.inner.local_canon_to_global.len() != sinst.local_canon_to_global.len()
        {
            return Err(Error::msg(
                "StoreSnapshot module/instance layout does not match destination Store",
            ));
        }
        if inst.module.inner.local_canon_to_global.as_ref()
            != sinst.local_canon_to_global.as_slice()
        {
            let map = cid_remap.get_or_insert_with(HashMap::new);
            for (&src_cid, &dst_cid) in sinst
                .local_canon_to_global
                .iter()
                .zip(inst.module.inner.local_canon_to_global.iter())
            {
                if let Some(&prev) = map.get(&src_cid)
                    && prev != dst_cid
                {
                    return Err(Error::msg(
                        "StoreSnapshot inconsistent canonical type remapping",
                    ));
                }
                map.insert(src_cid, dst_cid);
            }
        }
    }
    if let Some(map) = cid_remap.as_mut() {
        // Ensure all identity mappings from matching instances are also present in `cid_remap`.
        for (inst, sinst) in store.instances.iter().zip(snap.instances.iter()) {
            for (&src_cid, &dst_cid) in sinst
                .local_canon_to_global
                .iter()
                .zip(inst.module.inner.local_canon_to_global.iter())
            {
                map.entry(src_cid).or_insert(dst_cid);
            }
        }
    }
    if store.memories.len() != snap.memories.len()
        || store.shared_memories.len() != snap.shared_memories.len()
        || store.tables.len() != snap.tables.len()
        || store.standalone_globals.len() != snap.standalone_globals.len()
        || store.host_funcs.len() != snap.host_func_canon_types.len()
    {
        return Err(Error::msg(
            "StoreSnapshot resource count does not match destination Store",
        ));
    }
    for (hf, &expected_cid) in store
        .host_funcs
        .iter()
        .zip(snap.host_func_canon_types.iter())
    {
        let mapped_cid = match cid_remap.as_ref() {
            Some(map) => map.get(&expected_cid).copied().unwrap_or(expected_cid),
            None => expected_cid,
        };
        if hf.func_ref.canon_type_id != mapped_cid {
            return Err(Error::msg(
                "StoreSnapshot host function signature does not match destination Store",
            ));
        }
    }

    let live_bytes = snap.heap_bytes.len();
    if live_bytes > store.gc.heap_cap {
        return Err(Error::from(GcHeapOutOfMemory::new((), live_bytes as u64)));
    }
    // Validate patch/root offsets before writing to raw heap/ctx memory.
    for &off in &snap.gc_ref_offsets {
        if (off as usize).saturating_add(8) > live_bytes {
            return Err(Error::msg("StoreSnapshot gc_ref_offset out of bounds"));
        }
    }
    for &(off, _) in &snap.func_ref_patches {
        if (off as usize).saturating_add(8) > live_bytes {
            return Err(Error::msg(
                "StoreSnapshot func_ref_patch offset out of bounds",
            ));
        }
    }
    for sinst in &snap.instances {
        for &off in &sinst.gc_global_offs {
            if off < CTX_HEADER_BYTES as i32 || (off as usize).saturating_add(8) > sinst.ctx_len {
                return Err(Error::msg("StoreSnapshot gc_global_off out of bounds"));
            }
        }
        for &(off, _) in &sinst.func_global_patches {
            if off < CTX_HEADER_BYTES as i32 || (off as usize).saturating_add(8) > sinst.ctx_len {
                return Err(Error::msg("StoreSnapshot func_global_patch out of bounds"));
            }
        }
        for &off in &sinst.elem_gc_offs {
            if (off as usize).saturating_add(8) > sinst.elem_bytes.len() {
                return Err(Error::msg("StoreSnapshot elem_gc_off out of bounds"));
            }
        }
    }
    for stbl in &snap.tables {
        if stbl.is_traced && stbl.traced_words.len() != stbl.cur_len as usize {
            return Err(Error::msg(
                "StoreSnapshot table traced_words length mismatch",
            ));
        }
        for &idx in &stbl.traced_gc_indices {
            if idx >= stbl.cur_len {
                return Err(Error::msg(
                    "StoreSnapshot table traced_gc_index out of bounds",
                ));
            }
        }
        for &(idx, _) in &stbl.func_entries {
            if idx >= stbl.cur_len {
                return Err(Error::msg(
                    "StoreSnapshot table func_entry index out of bounds",
                ));
            }
        }
    }
    for &off in &snap.weak_tables_offs {
        if off >= live_bytes as u64 {
            return Err(Error::msg("StoreSnapshot weak_table offset out of bounds"));
        }
    }
    for w in &snap.weak_refs {
        if w.live && (w.owner_off >= live_bytes as u64 || w.target_off >= live_bytes as u64) {
            return Err(Error::msg("StoreSnapshot weak_ref offset out of bounds"));
        }
    }
    for e in &snap.ephemerons {
        if e.live {
            if e.owner_off >= live_bytes as u64 || e.key_off >= live_bytes as u64 {
                return Err(Error::msg("StoreSnapshot ephemeron offset out of bounds"));
            }
            if let SnapshotRootWord::HeapOff(off) = e.value
                && off >= live_bytes as u64
            {
                return Err(Error::msg(
                    "StoreSnapshot ephemeron value offset out of bounds",
                ));
            }
        }
    }
    for &rw in &snap.extra_roots {
        if let SnapshotRootWord::HeapOff(off) = rw
            && off >= live_bytes as u64
        {
            return Err(Error::msg("StoreSnapshot extra_root offset out of bounds"));
        }
    }
    for &idx in &snap.extern_free {
        if idx >= snap.extern_arena_len {
            return Err(Error::msg("StoreSnapshot extern_free index out of bounds"));
        }
    }
    let restore_ctx = SnapshotRestoreContext { store_id: store.id };
    let mut decoded_externs: Vec<(u32, Box<dyn Any + Send + Sync>)> =
        Vec::with_capacity(snap.extern_entries.len());
    for (idx, payload) in &snap.extern_entries {
        if *idx >= snap.extern_arena_len {
            return Err(Error::msg("StoreSnapshot extern_entry index out of bounds"));
        }
        decoded_externs.push((*idx, decode_extern(&restore_ctx, payload)?));
    }

    // 2. Prepare destination `OLD_A` and copy + rebase the compacted GC heap.
    let heap_cap = store.gc.heap_cap;
    let nursery_init = store.gc.nursery_init_cap.min(heap_cap);
    let target_old_cap = ((live_bytes
        .saturating_mul(2)
        .max(live_bytes.saturating_add(nursery_init))
        .max(store.gc.init_cap)
        + 4095)
        & !4095)
        .min(heap_cap);
    if !store.gc.ensure_old_committed(false, target_old_cap) {
        return Err(Error::from(GcHeapOutOfMemory::new((), live_bytes as u64)));
    }
    // Zero `OLD_A` card table and release `OLD_B` so restored Store starts clean with minimal RSS.
    let card_a_commit = store.gc.old_a_card_commit;
    if card_a_commit > 0 {
        unsafe {
            std::ptr::write_bytes(
                store.gc.card_table_start(false) as *mut u8,
                0,
                card_a_commit,
            );
        }
    }
    store.gc.shrink_old_space(true, 0);

    let new_old_base = store.gc.old_base(false);
    let delta = new_old_base.wrapping_sub(snap.source_old_base);

    if live_bytes > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                snap.heap_bytes.as_ptr(),
                new_old_base as *mut u8,
                live_bytes,
            );
        }
        // Rebase all GC heap pointers with a tight linear pass (`*slot += delta`).
        for &off in &snap.gc_ref_offsets {
            let slot = (new_old_base + u64::from(off)) as *mut u64;
            unsafe {
                *slot = (*slot).wrapping_add(delta);
            }
        }
        // Remap all `StoreFuncRef` pointers inside GC structs/arrays.
        for &(off, target) in &snap.func_ref_patches {
            let resolved = resolve_func_ref(store, target)?;
            let slot = (new_old_base + u64::from(off)) as *mut u64;
            unsafe {
                *slot = resolved;
            }
        }
        // Cross-Engine / different load-order restore: rewrite GC object header canonical type IDs.
        if let Some(ref remap) = cid_remap {
            store.sync_type_descs();
            let types_snap = Arc::clone(&store.types_snapshot);
            let mut cur = new_old_base;
            let end = new_old_base + live_bytes as u64;
            while cur < end {
                let hdr_lo = unsafe { *(cur as *const u32) };
                let kind = hdr_lo & 6;
                match kind {
                    GC_HDR_KIND_STRUCT => {
                        let old_cid = hdr_lo >> 3;
                        let &new_cid = remap.get(&old_cid).ok_or_else(|| {
                            Error::msg(format!(
                                "StoreSnapshot missing canonical type remap for struct cid {old_cid}"
                            ))
                        })?;
                        unsafe {
                            *(cur as *mut u32) = (new_cid << 3) | (hdr_lo & 7);
                        }
                        let CanonComposite::Struct { total_bytes, .. } =
                            &types_snap[new_cid as usize].composite
                        else {
                            return Err(Error::msg(
                                "StoreSnapshot remapped struct cid is not a struct",
                            ));
                        };
                        cur += u64::from(*total_bytes);
                    }
                    GC_HDR_KIND_ARRAY => {
                        let old_cid = hdr_lo >> 3;
                        let &new_cid = remap.get(&old_cid).ok_or_else(|| {
                            Error::msg(format!(
                                "StoreSnapshot missing canonical type remap for array cid {old_cid}"
                            ))
                        })?;
                        unsafe {
                            *(cur as *mut u32) = (new_cid << 3) | (hdr_lo & 7);
                        }
                        let len = unsafe { *((cur + 4) as *const u32) } as u64;
                        let CanonComposite::Array { elem, .. } =
                            &types_snap[new_cid as usize].composite
                        else {
                            return Err(Error::msg(
                                "StoreSnapshot remapped array cid is not an array",
                            ));
                        };
                        cur += ((8u64 + len * u64::from(elem.byte_size())) + 7) & !7;
                    }
                    GC_HDR_KIND_INTERNAL_EXTERN => {
                        cur += 16;
                    }
                    other => {
                        return Err(Error::msg(format!(
                            "StoreSnapshot corrupt GC object header kind {other} during cid remap"
                        )));
                    }
                }
            }
        }
    }

    // Track any nursery pages touched by `instantiate_module`'s `init_instance_fn`, then reset Nursery.
    let nursery_base = store.gc.nursery_base();
    if store.state.gc_alloc_ptr > nursery_base {
        let used_pages =
            ((store.state.gc_alloc_ptr - store.gc.nursery_start() as u64) as usize + 4095) & !4095;
        store.gc.nursery_dirty = store
            .gc
            .nursery_dirty
            .max(used_pages)
            .min(store.gc.nursery_commit);
    }
    store.gc.old_active_is_b = false;
    store.gc.old_alloc_ptr = new_old_base + live_bytes as u64;
    store.gc.old_cap = target_old_cap;
    store.gc.old_card_first_obj = snap.old_card_first_obj.clone();
    store.state.gc_alloc_ptr = nursery_base;
    store.state.gc_alloc_limit = nursery_base + store.gc.nursery_cap as u64;

    // 3. Restore per-instance `ctx_buf` tails (preserving imported-global pointers!) and `elem_buf`.
    // Resolve `func_global_patches` before mutably borrowing `store.instances`.
    let mut resolved_inst_funcs: Vec<Vec<(i32, u64)>> = Vec::with_capacity(snap.instances.len());
    for sinst in &snap.instances {
        let mut patches = Vec::with_capacity(sinst.func_global_patches.len());
        for &(off, target) in &sinst.func_global_patches {
            patches.push((off, resolve_func_ref(store, target)?));
        }
        resolved_inst_funcs.push(patches);
    }

    for ((inst, sinst), func_patches) in store
        .instances
        .iter_mut()
        .zip(snap.instances.iter())
        .zip(resolved_inst_funcs)
    {
        let meta = &inst.module.inner.meta;
        let mut saved_imp_globals: Vec<(i32, u64)> =
            Vec::with_capacity(meta.imported_globals.len());
        for &(_, _, _, _, off) in &meta.imported_globals {
            if off >= 0 {
                let ptr = unsafe { *(inst.ctx_buf.as_ptr().offset(off as isize).cast::<u64>()) };
                saved_imp_globals.push((off, ptr));
            }
        }
        inst.ctx_buf[CTX_HEADER_BYTES..].copy_from_slice(&sinst.ctx_tail);
        let ctx_mut = inst.ctx_buf.as_mut_ptr();
        for (off, ptr) in saved_imp_globals {
            unsafe {
                *(ctx_mut.offset(off as isize).cast::<u64>()) = ptr;
            }
        }
        for &off in &sinst.gc_global_offs {
            let slot = unsafe { ctx_mut.offset(off as isize).cast::<u64>() };
            unsafe {
                *slot = (*slot).wrapping_add(delta);
            }
        }
        for (off, resolved) in func_patches {
            unsafe {
                *(ctx_mut.offset(off as isize).cast::<u64>()) = resolved;
            }
        }
        if !inst.elem_buf.is_null() && !sinst.elem_bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sinst.elem_bytes.as_ptr(),
                    inst.elem_buf,
                    sinst.elem_bytes.len(),
                );
                for &off in &sinst.elem_gc_offs {
                    let slot = inst.elem_buf.add(off as usize).cast::<u64>();
                    *slot = (*slot).wrapping_add(delta);
                }
            }
        }
    }

    // 4. Restore private and shared linear memories.
    let mem_limit = store.memory_limit_bytes;
    for (mem, smem) in store.memories.iter_mut().zip(snap.memories.iter()) {
        let cur_len = mem.desc.len_bytes.load(Ordering::Acquire);
        if cur_len < smem.len_bytes {
            let delta_pages = (smem.len_bytes - cur_len) / WASM_PAGE_BYTES;
            mem.grow(delta_pages, mem_limit).ok_or_else(|| {
                Error::msg("StoreSnapshot linear memory size exceeds destination StoreLimits")
            })?;
        }
        // Reset committed pages via `MADV_DONTNEED` so any bytes written by `init_instance_fn`
        // become guaranteed zero without faulting untouched zero pages.
        let reset_len = mem.committed_bytes;
        if reset_len > 0 {
            unsafe {
                let _ = libc::madvise(mem.desc.base.cast(), reset_len, libc::MADV_DONTNEED);
            }
        }
        let dst = mem.data_mut();
        for (off, page) in &smem.nonzero_pages {
            let end = (*off + 4096).min(dst.len());
            dst[*off..end].copy_from_slice(&page[..end - *off]);
        }
    }
    for (sm, ssm) in store
        .shared_memories
        .iter()
        .zip(snap.shared_memories.iter())
    {
        let cur_len = sm.desc.len_bytes.load(Ordering::Acquire);
        if cur_len < ssm.len_bytes {
            let delta_pages = (ssm.len_bytes - cur_len) / WASM_PAGE_BYTES;
            sm.grow(delta_pages)?;
        }
        let len = sm.desc.len_bytes.load(Ordering::Acquire) as usize;
        if len > 0 {
            // Fresh shared memory has no other holders yet; copy non-zero pages (zeroing others if needed).
            let dst = unsafe { std::slice::from_raw_parts_mut(sm.desc.base, len) };
            let mut written: HashSet<usize> = HashSet::with_capacity(ssm.nonzero_pages.len());
            for (off, page) in &ssm.nonzero_pages {
                let end = (*off + 4096).min(len);
                dst[*off..end].copy_from_slice(&page[..end - *off]);
                written.insert(*off);
            }
            let mut off = 0usize;
            while off < len {
                if !written.contains(&off) {
                    let end = (off + 4096).min(len);
                    let chunk = &mut dst[off..end];
                    let is_zero = if chunk.len() == 4096 {
                        let words = unsafe {
                            std::slice::from_raw_parts(chunk.as_ptr().cast::<u64>(), 512)
                        };
                        words.iter().all(|&w| w == 0)
                    } else {
                        chunk.iter().all(|&b| b == 0)
                    };
                    if !is_zero {
                        chunk.fill(0);
                    }
                }
                off += 4096;
            }
        }
    }

    // 5. Restore dynamic tables (resolve FuncRefs before mutably borrowing `store.tables`).
    let mut resolved_table_funcs: Vec<Vec<(u32, u64)>> = Vec::with_capacity(snap.tables.len());
    for stbl in &snap.tables {
        let mut entries = Vec::with_capacity(stbl.func_entries.len());
        for &(idx, target) in &stbl.func_entries {
            entries.push((idx, resolve_func_ref(store, target)?));
        }
        resolved_table_funcs.push(entries);
    }
    let tbl_limit = store.table_elements_limit;
    for ((tbl, stbl), func_entries) in store
        .tables
        .iter_mut()
        .zip(snap.tables.iter())
        .zip(resolved_table_funcs)
    {
        if tbl.desc.cur_len < stbl.cur_len {
            let delta_len = u64::from(stbl.cur_len - tbl.desc.cur_len);
            tbl.grow(0, delta_len, tbl_limit)
                .ok_or_else(|| Error::msg("StoreSnapshot table size exceeds StoreLimits"))?;
        }
        if stbl.is_traced {
            let dst =
                unsafe { std::slice::from_raw_parts_mut(tbl.desc.ptr, stbl.cur_len as usize) };
            dst.copy_from_slice(&stbl.traced_words);
            for &idx in &stbl.traced_gc_indices {
                dst[idx as usize] = dst[idx as usize].wrapping_add(delta);
            }
        } else {
            let dst =
                unsafe { std::slice::from_raw_parts_mut(tbl.desc.ptr, stbl.cur_len as usize) };
            dst.fill(0);
            for (idx, resolved) in func_entries {
                dst[idx as usize] = resolved;
            }
        }
    }

    // 6. Restore standalone globals.
    let mut resolved_sg_funcs: Vec<Option<u64>> = Vec::with_capacity(snap.standalone_globals.len());
    for sg in &snap.standalone_globals {
        resolved_sg_funcs.push(match sg.func_target {
            Some(target) => Some(resolve_func_ref(store, target)?),
            None => None,
        });
    }
    for ((g, sg), func_ptr) in store
        .standalone_globals
        .iter_mut()
        .zip(snap.standalone_globals.iter())
        .zip(resolved_sg_funcs)
    {
        let mut words = sg.words;
        if sg.is_traced && sg.is_gc_ptr {
            words[0] = words[0].wrapping_add(delta);
        } else if let Some(fp) = func_ptr {
            words[0] = fp;
        }
        *g.cell = words;
    }

    // 7. Restore WeakMap tables, WeakRefs, Ephemerons, and clear past caught exceptions.
    store.gc.weak_tables.clear();
    store.gc.weak_tables.reserve(snap.weak_tables_offs.len());
    for &off in &snap.weak_tables_offs {
        store.gc.weak_tables.insert(new_old_base + off);
    }

    store.gc.weak_refs = snap
        .weak_refs
        .iter()
        .map(|w| WeakRefEntry {
            owner: if w.live {
                new_old_base + w.owner_off
            } else {
                0
            },
            target: if w.live {
                new_old_base + w.target_off
            } else {
                0
            },
            generation: w.generation,
            live: w.live,
        })
        .collect();
    store.gc.weak_refs_free = snap.weak_refs_free.clone();
    store.gc.weak_retire_count = snap.weak_retire_count;

    store.gc.ephemerons = snap
        .ephemerons
        .iter()
        .map(|e| EphemeronEntry {
            owner: if e.live {
                new_old_base + e.owner_off
            } else {
                0
            },
            key: if e.live { new_old_base + e.key_off } else { 0 },
            value: if e.live {
                match e.value {
                    SnapshotRootWord::Immediate(w) => w,
                    SnapshotRootWord::HeapOff(off) => new_old_base + off,
                }
            } else {
                0
            },
            generation: e.generation,
            live: e.live,
            activated: false,
        })
        .collect();
    store.gc.ephemerons_free = snap.ephemerons_free.clone();
    store.gc.extern_arena = (0..snap.extern_arena_len)
        .map(|_| ExternArenaSlot { data: None })
        .collect();
    for (idx, payload) in decoded_externs {
        store.gc.extern_arena[idx as usize].data = Some(payload);
    }
    store.gc.extern_free = snap.extern_free.clone();
    store.gc.exn_objects.clear();
    store.gc.exn_by_ptr.clear();
    store.state.exn_active_ptr = 0;

    // 8. Reconstruct caller's `OwnedRooted<EqRef>` roots in destination Store.
    let mut out_roots = Vec::with_capacity(snap.extra_roots.len());
    for &rw in &snap.extra_roots {
        let word = match rw {
            SnapshotRootWord::Immediate(w) => w,
            SnapshotRootWord::HeapOff(off) => new_old_base + off,
        };
        out_roots.push(OwnedRooted::<EqRef>::from_abi_word(
            store, word, false, false,
        ));
    }

    Ok(out_roots)
}

pub(crate) fn rebind_ephemeron(store: &StoreInner, mut entry: GcEphemeron) -> GcEphemeron {
    entry.store_id = store.id;
    entry
}

pub(crate) fn rebind_weak_ref(store: &StoreInner, mut weak: WeakAnyRef) -> WeakAnyRef {
    weak.store_id = store.id;
    weak
}
