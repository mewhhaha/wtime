//! Binary in-memory `StoreSnapshot` capture & sub-millisecond pointer-rebasing restore.
//!
//! Capturing a `StoreSnapshot` runs a full Major GC to compact all live GC objects into
//! a contiguous Old Space prefix `[old_base .. old_alloc_ptr)`, then precomputes flat
//! relocation tables for GC heap pointers (`+ delta`) and `StoreFuncRef` pointers
//! (`(inst_idx, def_idx)` or `Host(hf_idx)`). Restoring into a fresh `Store` with the
//! same instantiated modules performs a single `memcpy` + linear pointer rebase without
//! re-executing initialization code.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use wtime_aot::x64::CTX_HEADER_BYTES;
use wtime_aot::x64::module::{
    GC_HDR_KIND_ARRAY, GC_HDR_KIND_INTERNAL_EXTERN, GC_HDR_KIND_STRUCT, GcStorageKind,
};

use crate::abi::StoreFuncRef;
use crate::error::{Error, GcHeapOutOfMemory, Result};
use crate::gc::{
    EphemeronEntry, EqRef, GcEphemeron, OwnedRooted, RootedRef, WeakAnyRef, WeakRefEntry,
    collect_store_gc,
};
use crate::memory::WASM_PAGE_BYTES;
use crate::store::StoreInner;
use crate::types::CanonComposite;

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
    module_identity: usize,
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
    extra_roots: Vec<SnapshotRootWord>,
}

impl StoreSnapshot {
    /// Returns the compacted GC heap size in bytes captured in this snapshot.
    pub fn heap_bytes_len(&self) -> usize {
        self.heap_bytes.len()
    }
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
    // 1. Run a full Major GC so ALL live GC objects are compacted into contiguous active Old Space,
    //    Nursery is empty, and all unreachable ExternRefs / WeakRefs / Ephemerons are swept.
    unsafe { collect_store_gc(store, 0, 0, true).map_err(Error::from)? };

    if store.state.exn_active_ptr != 0 {
        return Err(Error::msg(
            "cannot capture StoreSnapshot with an active uncaught Wasm exception",
        ));
    }
    if store.gc.extern_arena.iter().any(|s| s.data.is_some()) {
        return Err(Error::msg(
            "cannot capture StoreSnapshot while live opaque host ExternRef payloads exist",
        ));
    }

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
                            if w != 0 && (w & 1) == 0 {
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
                            if w != 0 && (w & 1) == 0 {
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
                if payload != 0 && (payload & 1) == 0 {
                    return Err(Error::msg(
                        "StoreSnapshot encountered live InternalExternBox with host payload",
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
                if word != 0 && (word & 1) == 0 {
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
                    if w != 0 && (w & 1) == 0 {
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
            module_identity: Arc::as_ptr(&inst.module.inner) as usize,
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
                if w != 0 && (w & 1) == 0 {
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
            if w != 0 && (w & 1) == 0 {
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
            let val = if e.value == 0 || (e.value & 1) != 0 {
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
        if w == 0 || (w & 1) != 0 {
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
        extra_roots: captured_roots,
    })
}

pub(crate) fn restore_store_snapshot(
    store: &mut StoreInner,
    snap: &StoreSnapshot,
) -> Result<Vec<OwnedRooted<EqRef>>> {
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
    for (inst, sinst) in store.instances.iter().zip(snap.instances.iter()) {
        if Arc::as_ptr(&inst.module.inner) as usize != sinst.module_identity
            || inst.ctx_buf.len() != sinst.ctx_len
            || inst.elem_buf_bytes != sinst.elem_buf_bytes
        {
            return Err(Error::msg(
                "StoreSnapshot module/instance layout does not match destination Store",
            ));
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
        if hf.func_ref.canon_type_id != expected_cid {
            return Err(Error::msg(
                "StoreSnapshot host function signature does not match destination Store",
            ));
        }
    }

    let live_bytes = snap.heap_bytes.len();
    if live_bytes > store.gc.heap_cap {
        return Err(Error::from(GcHeapOutOfMemory::new((), live_bytes as u64)));
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
