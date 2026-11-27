//! Wasm GC runtime metadata, Cheney semi-space copying garbage collector, array bulk helpers,
//! and `wt_initialize` GC constant-expression evaluator for `--backend x64`.
//!
//! All machine code emitted here is 100% pure safe Rust (`#![forbid(unsafe_code)]`) with zero
//! external dependencies, zero shadow stacks, and zero overhead for non-GC modules.

use wtime_types::ValType;

use super::asm::{Asm, Cond, Gp, Label, Mem, RelocTarget, SafepointRecord, Xmm};
use super::module::{
    GC_DISPLAY_LEN, GC_EXT_I31_TAG_HI, GC_HDR_KIND_INTERNAL_EXTERN, GcCompositeInfo, GcConstExpr,
    GcStorageKind, X64ConstVal, X64Module,
};
use super::{
    CtxLayout, HELPER_GC_ARRAY_FILL_RAW, HELPER_GC_COLLECT, OFF_STORE_CANON_MAP,
    OFF_STORE_ELEM_BUF_PTR, OFF_STORE_FUNC_REFS, OFF_STORE_PTR, OFF_STORE_TABLE_DESCS,
    TRAP_ALLOC_FAILED, TRAP_INDIRECT_CALL, TRAP_OOB,
};

const SYS_MMAP: u32 = 9;
const SYS_MUNMAP: u32 = 11;
const SYS_MADVISE: u32 = 28;
const PROT_RW: u32 = 3;
const MAP_PRIVATE_ANON: u32 = 0x22;
const MADV_DONTNEED: u32 = 4;

// ---- `GcState` byte offsets relative to `gc.gc_state_off` inside `Ctx` (112 bytes total) ----
pub const GC_OFF_ALLOC_PTR: i32 = 0;
pub const GC_OFF_ALLOC_LIMIT: i32 = 8;
pub const GC_OFF_FROM_BASE: i32 = 16;
pub const GC_OFF_FROM_CAP: i32 = 24;
pub const GC_OFF_TO_BASE: i32 = 32;
pub const GC_OFF_TO_CAP: i32 = 40;
pub const GC_OFF_HEAP_CAP: i32 = 48;
pub const GC_OFF_GC_COUNT: i32 = 56;
pub const GC_OFF_WALK_STOP_RSP: i32 = 64;
pub const GC_OFF_STACK_CHAIN: i32 = 72;
pub const GC_OFF_INIT_ROOTS_PTR: i32 = 80;
pub const GC_OFF_INIT_ROOTS_LEN: i32 = 88;
pub const GC_OFF_ELEM_BUF_PTR: i32 = 96;
pub const GC_OFF_ELEM_BUF_BYTES: i32 = 104;
pub const GC_STATE_BYTES: usize = 112;

/// `sizeof(TypeDesc)` in `.rodata` (power of 2 for `shl reg, 6` indexing).
pub const TYPE_DESC_BYTES: usize = 64;

/// GC metadata embedded in `CtxLayout` when `module.uses_gc == true`.
#[derive(Clone, Debug)]
pub struct GcCtxLayout {
    /// Byte offset of the 112-byte `GcState` block inside `Ctx` (16-byte aligned).
    pub gc_state_off: i32,
    /// Initial semi-space size in bytes (4 KiB-aligned, `>= 4096`).
    pub init_heap_bytes: u64,
    /// Maximum semi-space size in bytes before trapping `TRAP_ALLOC_FAILED` (4 KiB-aligned).
    pub max_heap_bytes: u64,
    /// Page-aligned size of the passive GC element buffer (`0` if no passive GC elem segs).
    pub gc_elem_buf_bytes: u64,
    /// `.rodata` offset of `TypeDesc[n_canon_types]` (64 bytes each).
    pub type_desc_ro_off: u32,
    /// `.rodata` offset of `u32 func_canon_id[n_total_funcs]`.
    pub func_canon_ro_off: u32,
    /// `.rodata` offset of `GcElemDesc[elem_segs.len()]` (16 bytes each; `0` if empty).
    pub gc_elem_desc_ro_off: u32,
    /// `.rodata` offset of `i32` `Ctx` offsets for traced GC/Extern globals.
    pub gc_root_globals_ro_off: u32,
    pub gc_root_globals_count: u32,
    /// `.rodata` offset of `(i32 ctx_off, u32 len)` pairs for traced inline `extern_tables`.
    pub gc_root_inline_tables_ro_off: u32,
    pub gc_root_inline_tables_count: u32,
    /// `.rodata` offset of `i32` `Ctx` descriptor offsets for traced `dyn_tables`.
    pub gc_root_dyn_tables_ro_off: u32,
    pub gc_root_dyn_tables_count: u32,
    /// `.rodata` offset of per-tag exception root descriptors (16 bytes/tag; `0` if no traced tags).
    pub gc_root_exn_tags_ro_off: u32,
    pub has_traced_exn_tags: bool,
    /// Per-element-segment byte offset into `gc_elem_buf_ptr` (`-1` if not a passive GC elem seg).
    pub passive_gc_elem_offs: Vec<i32>,
}

/// Output from building GC `.rodata` tables during `build_shared_rodata`.
#[derive(Clone, Debug)]
pub struct GcRodataInfo {
    pub gc_elem_buf_bytes: u64,
    pub type_desc_ro_off: u32,
    pub func_canon_ro_off: u32,
    pub gc_elem_desc_ro_off: u32,
    pub gc_root_globals_ro_off: u32,
    pub gc_root_globals_count: u32,
    pub gc_root_inline_tables_ro_off: u32,
    pub gc_root_inline_tables_count: u32,
    pub gc_root_dyn_tables_ro_off: u32,
    pub gc_root_dyn_tables_count: u32,
    pub gc_root_exn_tags_ro_off: u32,
    pub has_traced_exn_tags: bool,
    pub passive_gc_elem_offs: Vec<i32>,
}

/// Helper to align and push bytes into a `.rodata` buffer with `MAX_TOTAL_RODATA_BYTES` enforcement.
fn ro_align(buf: &mut Vec<u8>, align: usize) -> Result<u32, String> {
    let rem = buf.len() & (align - 1);
    if rem != 0 {
        buf.resize(buf.len() + (align - rem), 0);
    }
    if buf.len() > super::module::MAX_TOTAL_RODATA_BYTES {
        return Err(format!(
            "x64: GC .rodata ({} B) exceeds cap {} B",
            buf.len(),
            super::module::MAX_TOTAL_RODATA_BYTES
        ));
    }
    u32::try_from(buf.len()).map_err(|_| "x64: GC .rodata offset exceeds u32".into())
}

fn ro_push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn ro_push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn ro_push_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Builds all GC `.rodata` descriptors (type table, subtyping displays, ref-offset maps,
/// function canonical IDs, GC element descriptors, and root descriptor tables).
pub fn build_gc_rodata(
    rodata: &mut Vec<u8>,
    module: &X64Module,
    layout: &CtxLayout,
    passive_elem_ro_entries: &[(u32, u32)],
) -> Result<GcRodataInfo, String> {
    let n_canon = module.gc_canon_types.len();

    // 1. Emit per-struct traced-field `[u16]` lists and deep supertype overflow `[u32]` lists FIRST
    //    so we can store exact signed relative offsets inside each 64-byte `TypeDesc`.
    let mut struct_ref_offs: Vec<(u32, u32)> = vec![(0, 0); n_canon];
    let mut super_overflow_offs: Vec<u32> = vec![0; n_canon];
    for (cid, gt) in module.gc_canon_types.iter().enumerate() {
        if let GcCompositeInfo::Struct { fields, .. } = &gt.composite {
            let mut traced: Vec<u16> = Vec::new();
            for f in fields {
                if X64Module::storage_ref_kind(f.storage).is_traced() {
                    let off_u16 = u16::try_from(f.offset)
                        .map_err(|_| "x64: GC struct field offset exceeds u16")?;
                    traced.push(off_u16);
                }
            }
            if !traced.is_empty() {
                let off = ro_align(rodata, 2)?;
                for fo in &traced {
                    ro_push_u16(rodata, *fo);
                }
                struct_ref_offs[cid] = (off, traced.len() as u32);
            }
        }
        if gt.depth as usize >= GC_DISPLAY_LEN {
            let off = ro_align(rodata, 4)?;
            for &sup in &gt.supertypes_all {
                ro_push_u32(rodata, sup);
            }
            super_overflow_offs[cid] = off;
        }
    }

    // 2. Emit `TypeDesc[n_canon]` (64 bytes each, 8-byte aligned).
    //    Layout:
    //      +0 : kind: u8 (0=Func, 1=Struct, 2=Array)
    //      +1 : elem_ref_kind: u8 (Array: 0=None, 1=GcRef, 2=ExternRef)
    //      +2 : is_final: u8
    //      +3 : pad: u8
    //      +4 : size_or_elem_sz: u32
    //      +8 : ref_offs_rel: i32 (relative to `type_desc_ro_off`)
    //      +12: num_traced_fields: u32
    //      +16: super_overflow_rel: i32 (relative to `type_desc_ro_off`)
    //      +20: depth: u32
    //      +24..+64: display: [u32; 10]
    let type_desc_ro_off = ro_align(rodata, 8)?;
    for (cid, gt) in module.gc_canon_types.iter().enumerate() {
        let (kind, elem_ref_kind, size_or_elem_sz) = match &gt.composite {
            GcCompositeInfo::Func => (0u8, 0u8, 0u32),
            GcCompositeInfo::Struct { total_bytes, .. } => (1u8, 0u8, *total_bytes),
            GcCompositeInfo::Array { elem, .. } => (
                2u8,
                X64Module::storage_ref_kind(*elem) as u8,
                elem.byte_size(),
            ),
        };
        rodata.push(kind);
        rodata.push(elem_ref_kind);
        rodata.push(u8::from(gt.is_final));
        rodata.push(0);
        ro_push_u32(rodata, size_or_elem_sz);
        let (ref_off, num_refs) = struct_ref_offs[cid];
        let ref_rel = if num_refs > 0 {
            i32::try_from(i64::from(ref_off) - i64::from(type_desc_ro_off))
                .map_err(|_| "x64: GC ref_offs rel32 overflow")?
        } else {
            0
        };
        ro_push_i32(rodata, ref_rel);
        ro_push_u32(rodata, num_refs);
        let ov_off = super_overflow_offs[cid];
        let ov_rel = if gt.depth as usize >= GC_DISPLAY_LEN {
            i32::try_from(i64::from(ov_off) - i64::from(type_desc_ro_off))
                .map_err(|_| "x64: GC super_overflow rel32 overflow")?
        } else {
            0
        };
        ro_push_i32(rodata, ov_rel);
        ro_push_u32(rodata, gt.depth);
        for &d_cid in &gt.display {
            ro_push_u32(rodata, d_cid);
        }
    }
    debug_assert_eq!(
        rodata.len() - type_desc_ro_off as usize,
        n_canon * TYPE_DESC_BYTES
    );

    // 3. Emit `func_canon_id[n_total_funcs]` (`u32` canonical type ID for every function).
    let func_canon_ro_off = ro_align(rodata, 4)?;
    for &ti in &module.func_type_idx {
        let cid = module.type_canon[ti as usize];
        ro_push_u32(rodata, cid);
    }

    // 4. Emit `GcElemDesc[elem_segs.len()]` (16 bytes each) and compute passive GC elem buffer layout.
    let mut passive_gc_elem_offs = vec![-1i32; module.elem_segs.len()];
    let mut gc_elem_cursor_bytes: usize = 0;
    for (ei, seg) in module.elem_segs.iter().enumerate() {
        if seg.is_passive
            && let Some(ref ge) = seg.gc_entries
        {
            let off_i32 = i32::try_from(gc_elem_cursor_bytes)
                .map_err(|_| "x64: GC element buffer exceeds i32")?;
            passive_gc_elem_offs[ei] = off_i32;
            gc_elem_cursor_bytes = gc_elem_cursor_bytes
                .checked_add(
                    ge.len()
                        .checked_mul(8)
                        .ok_or("x64: GC element buffer size overflow")?,
                )
                .ok_or("x64: GC element buffer size overflow")?;
        }
    }
    let gc_elem_buf_bytes = if gc_elem_cursor_bytes == 0 {
        0u64
    } else {
        ((gc_elem_cursor_bytes as u64) + 4095) & !4095
    };
    let gc_elem_desc_ro_off = if !module.elem_segs.is_empty() {
        let tbl_off = ro_align(rodata, 8)?;
        for (ei, seg) in module.elem_segs.iter().enumerate() {
            let (ro_off, static_len) = passive_elem_ro_entries.get(ei).copied().unwrap_or((0, 0));
            let gc_buf_off = passive_gc_elem_offs[ei];
            let (ro_rel, elem_len, is_traced) = if gc_buf_off >= 0 {
                let len = seg.gc_entries.as_ref().map_or(0, Vec::len) as u32;
                let traced = u32::from(seg.ref_family.is_traced());
                (0i32, len, traced)
            } else {
                let rel = if static_len > 0 {
                    i32::try_from(i64::from(ro_off) - i64::from(tbl_off))
                        .map_err(|_| "x64: GC elem rel32 overflow")?
                } else {
                    0
                };
                (rel, static_len, 0u32)
            };
            ro_push_i32(rodata, ro_rel);
            ro_push_u32(rodata, elem_len);
            ro_push_i32(rodata, gc_buf_off);
            ro_push_u32(rodata, is_traced);
        }
        tbl_off
    } else {
        0
    };

    // 5. Traced global root offsets (`i32` offsets inside `Ctx`).
    let mut root_globals: Vec<i32> = Vec::new();
    for (k, &(vt, _)) in module.globals.iter().enumerate() {
        if module.global_consts[k].is_some() {
            continue;
        }
        if module.val_ref_kind(vt).is_traced() {
            root_globals.push(layout.global_offs[k]);
        }
    }
    let gc_root_globals_ro_off = if !root_globals.is_empty() {
        let off = ro_align(rodata, 4)?;
        for &goff in &root_globals {
            ro_push_i32(rodata, goff);
        }
        off
    } else {
        0
    };
    let gc_root_globals_count = root_globals.len() as u32;

    // 6. Traced inline `extern_tables` `(i32 ctx_off, u32 elem_len)`.
    let mut root_inline_tables: Vec<(i32, u32)> = Vec::new();
    for (&tbl, &(ctx_off, len)) in &layout.extern_tables {
        // Inline `extern_tables` are fixed-size mutable `externref` tables (always traced).
        let _ = tbl;
        if len > 0 {
            root_inline_tables.push((ctx_off, len));
        }
    }
    let gc_root_inline_tables_ro_off = if !root_inline_tables.is_empty() {
        let off = ro_align(rodata, 4)?;
        for &(ctx_off, len) in &root_inline_tables {
            ro_push_i32(rodata, ctx_off);
            ro_push_u32(rodata, len);
        }
        off
    } else {
        0
    };
    let gc_root_inline_tables_count = root_inline_tables.len() as u32;

    // 7. Traced dynamic tables (`i32 ctx_desc_off`).
    let mut root_dyn_tables: Vec<i32> = Vec::new();
    for (&tbl, &(desc_off, _, _)) in &layout.dyn_tables {
        if module
            .dyn_tables
            .get(&tbl)
            .is_some_and(|m| m.ref_family.is_traced())
        {
            root_dyn_tables.push(desc_off);
        }
    }
    let gc_root_dyn_tables_ro_off = if !root_dyn_tables.is_empty() {
        let off = ro_align(rodata, 4)?;
        for &doff in &root_dyn_tables {
            ro_push_i32(rodata, doff);
        }
        off
    } else {
        0
    };
    let gc_root_dyn_tables_count = root_dyn_tables.len() as u32;

    // 8. Exception-tag root descriptors (when `module.uses_exceptions`).
    let mut has_traced_exn_tags = false;
    let gc_root_exn_tags_ro_off = if module.uses_exceptions && !module.tags.is_empty() {
        let mut tag_ref_lists: Vec<(u32, u32)> = Vec::with_capacity(module.tags.len());
        for (ti, &type_idx) in module.tags.iter().enumerate() {
            let fty = &module.module_types[type_idx as usize];
            let (ref fields, _) = layout.tag_layouts[ti];
            let mut offs: Vec<i32> = Vec::new();
            for (&vt, &(_, foff)) in fty.params.iter().zip(fields.iter()) {
                if module.val_ref_kind(vt).is_traced() {
                    offs.push(foff);
                    has_traced_exn_tags = true;
                }
            }
            if !offs.is_empty() {
                let off = ro_align(rodata, 4)?;
                for fo in &offs {
                    ro_push_i32(rodata, *fo);
                }
                tag_ref_lists.push((off, offs.len() as u32));
            } else {
                tag_ref_lists.push((0, 0));
            }
        }
        if has_traced_exn_tags {
            let tbl_off = ro_align(rodata, 8)?;
            for (ti, &(ref_off, n_refs)) in tag_ref_lists.iter().enumerate() {
                let rec_size = layout.tag_layouts[ti].1;
                let ref_rel = if n_refs > 0 {
                    i32::try_from(i64::from(ref_off) - i64::from(tbl_off))
                        .map_err(|_| "x64: GC exn tag rel32 overflow")?
                } else {
                    0
                };
                ro_push_u32(rodata, rec_size);
                ro_push_i32(rodata, ref_rel);
                ro_push_u32(rodata, n_refs);
                ro_push_u32(rodata, 0);
            }
            tbl_off
        } else {
            0
        }
    } else {
        0
    };

    if rodata.len() > super::module::MAX_TOTAL_RODATA_BYTES {
        return Err(format!(
            "x64: GC .rodata ({} B) exceeds cap {} B",
            rodata.len(),
            super::module::MAX_TOTAL_RODATA_BYTES
        ));
    }

    Ok(GcRodataInfo {
        gc_elem_buf_bytes,
        type_desc_ro_off,
        func_canon_ro_off,
        gc_elem_desc_ro_off,
        gc_root_globals_ro_off,
        gc_root_globals_count,
        gc_root_inline_tables_ro_off,
        gc_root_inline_tables_count,
        gc_root_dyn_tables_ro_off,
        gc_root_dyn_tables_count,
        gc_root_exn_tags_ro_off,
        has_traced_exn_tags,
        passive_gc_elem_offs,
    })
}

/// Computes exact byte size of the post-link sorted `SafepointHeader + entries + slots` table.
pub fn planned_safepoint_table_bytes(def_asms: &[Asm]) -> Result<usize, String> {
    let mut n_entries: usize = 0;
    let mut n_slots: usize = 0;
    for asm in def_asms {
        n_entries = n_entries
            .checked_add(asm.safepoints.len())
            .ok_or("x64: safepoint count overflow")?;
        for sp in &asm.safepoints {
            n_slots = n_slots
                .checked_add(sp.gc_slots.len())
                .and_then(|v| v.checked_add(sp.ext_slots.len()))
                .ok_or("x64: safepoint slot count overflow")?;
        }
    }
    16usize
        .checked_add(
            n_entries
                .checked_mul(16)
                .ok_or("x64: safepoint table size overflow")?,
        )
        .and_then(|v| {
            v.checked_add(
                n_slots
                    .checked_mul(4)
                    .ok_or("x64: safepoint slots size overflow")
                    .ok()?,
            )
        })
        .ok_or_else(|| "x64: safepoint table size overflow".into())
}

/// Appends the post-link sorted `SafepointHeader + entries + slots` table into `rodata` at
/// `expected_off` (8-byte aligned), returning its `.rodata` offset.
pub fn emit_safepoint_table_rodata(
    rodata: &mut Vec<u8>,
    expected_off: usize,
    def_asms: &[Asm],
    func_text_offs: &[u64],
    n_imports: usize,
    defs_start_off: usize,
) -> Result<(), String> {
    debug_assert!(expected_off >= rodata.len());
    rodata.resize(expected_off, 0);

    let mut flat: Vec<(u32, u32, &SafepointRecord)> = Vec::new();
    for (bi, asm) in def_asms.iter().enumerate() {
        let fn_off = func_text_offs[n_imports + bi];
        for sp in &asm.safepoints {
            let abs_off = fn_off
                .checked_add(u64::from(sp.code_off))
                .ok_or("x64: safepoint code offset overflow")?;
            let rel = abs_off
                .checked_sub(defs_start_off as u64)
                .ok_or("x64: safepoint precedes GuestCodeStart")?;
            let rel_u32 = u32::try_from(rel)
                .map_err(|_| "x64: safepoint GuestCodeStart-relative offset exceeds u32")?;
            flat.push((rel_u32, sp.frame_caller_ret_off, sp));
        }
    }
    flat.sort_unstable_by_key(|&(pc_rel, _, _)| pc_rel);
    if flat.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("x64: duplicate safepoint return address detected".into());
    }

    let num_entries = u32::try_from(flat.len()).map_err(|_| "x64: safepoint count exceeds u32")?;
    let entries_rel_off: i32 = 16;
    let slots_rel_off = i32::try_from(16usize + flat.len() * 16)
        .map_err(|_| "x64: safepoint slots offset exceeds i32")?;

    // Header (16 bytes): num_entries, pad, entries_rel_off, slots_rel_off.
    ro_push_u32(rodata, num_entries);
    ro_push_u32(rodata, 0);
    ro_push_i32(rodata, entries_rel_off);
    ro_push_i32(rodata, slots_rel_off);

    // Entries (16 bytes each): ret_pc_rel, frame_caller_ret_off, first_slot_idx, num_slots.
    let mut slot_cursor: u32 = 0;
    for &(pc_rel, frame_ret_off, sp) in &flat {
        let n_sp_slots = u32::try_from(sp.gc_slots.len() + sp.ext_slots.len())
            .map_err(|_| "x64: safepoint slot count exceeds u32")?;
        ro_push_u32(rodata, pc_rel);
        ro_push_u32(rodata, frame_ret_off);
        ro_push_u32(rodata, slot_cursor);
        ro_push_u32(rodata, n_sp_slots);
        slot_cursor = slot_cursor
            .checked_add(n_sp_slots)
            .ok_or("x64: safepoint total slot index overflow")?;
    }

    // Slots (`[i32]`): `gc_slots` followed by `ext_slots` for each entry.
    for &(_, _, sp) in &flat {
        for &off in &sp.gc_slots {
            ro_push_i32(rodata, off);
        }
        for &off in &sp.ext_slots {
            ro_push_i32(rodata, off);
        }
    }
    Ok(())
}

/// Emits `HELPER_GC_COLLECT`: preserve-all Cheney semi-space copying garbage collector.
///
/// ABI:
/// - Inputs: `R15 = Ctx*`, `R10 = req_bytes` (8-byte aligned requested allocation bytes).
/// - Clobbers: ONLY `R11` (`R11d = 0` on success with `alloc_ptr + req_bytes <= alloc_limit`,
///   or `R11d = TRAP_ALLOC_FAILED` on OOM/heap-cap exceeded).
/// - Preserves: ALL other GP registers (`Rax..R15` except `R11`) and ALL XMM/YMM/ZMM registers
///   (executes zero vector/FP instructions).
pub fn emit_helper_gc_collect(module: &X64Module, layout: &CtxLayout) -> Result<Asm, String> {
    let gc = layout.gc.as_ref().ok_or("x64: missing GcCtxLayout")?;
    let gc_off = gc.gc_state_off;
    let mut a = Asm::with_capacity(2048);

    // Save all 13 GP registers other than R15 (Ctx*) and R11 (return trap code).
    // 13 pushes (104 B) + 8 B return address = 112 B (16-byte aligned!).
    let saved_regs = [
        Gp::Rax,
        Gp::Rbx,
        Gp::Rcx,
        Gp::Rdx,
        Gp::Rsi,
        Gp::Rdi,
        Gp::Rbp,
        Gp::R8,
        Gp::R9,
        Gp::R10,
        Gp::R12,
        Gp::R13,
        Gp::R14,
    ];
    for &r in &saved_regs {
        a.push(r);
    }
    // Local 64-byte scratch frame (keeps `rsp % 16 == 0`):
    //   [rsp + 0]  = caller_ret_rsp (pointer to caller's return address: `rsp + 168`)
    //   [rsp + 8]  = req_bytes (u64)
    //   [rsp + 16] = gc_pass (0 = first pass, 1 = post-growth second pass)
    //   [rsp + 24] = requested target_cap for pass 1
    //   [rsp + 32] = active to_base
    //   [rsp + 40] = active to_cap
    //   [rsp + 48] = scratch 0 (outer loop cursor / stack chain ptr)
    //   [rsp + 56] = scratch 1 (outer loop end / stack stop_rsp)
    a.sub_ri(true, Gp::Rsp, 64);
    a.cld();

    a.lea(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 168));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::R10);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);

    let lbl_oom = a.new_label();
    let lbl_done_ok = a.new_label();
    let lbl_ret_common = a.new_label();
    let lbl_gc_pass = a.new_label();
    let lbl_evacuate = a.new_label();

    // Upfront check: `req_bytes <= heap_cap_bytes`.
    a.cmp_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_HEAP_CAP),
    );
    a.jcc(Cond::A, lbl_oom);

    // =========================================================================
    // Cheney semi-space collection pass (`gc_pass` = 0 or 1).
    // =========================================================================
    a.bind(lbl_gc_pass);
    // Load `from_base` (R12) and `from_cap` (Rcx), compute `from_end` (R13 = R12 + Rcx).
    a.mov_rm(
        true,
        Gp::R12,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_BASE),
    );
    a.mov_rm(
        true,
        Gp::Rcx,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_CAP),
    );
    a.lea(true, Gp::R13, Mem::indexed(Gp::R12, Gp::Rcx, 1, 0));

    // Determine `want_to_cap` in Rbx:
    //   pass 0 -> `from_cap`
    //   pass 1 -> `[rsp + 24]` (target_cap, always >= from_cap and <= heap_cap)
    a.mov_rr(true, Gp::Rbx, Gp::Rcx);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    let lbl_have_want_cap = a.new_label();
    a.jcc(Cond::E, lbl_have_want_cap);
    a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 24));
    a.bind(lbl_have_want_cap);

    // Check existing `to_space`: reuse if `to_base != 0 && to_cap >= want_to_cap`, else unmap & mmap.
    let lbl_have_to_space = a.new_label();
    let lbl_mmap_to = a.new_label();
    a.mov_rm(
        true,
        Gp::Rdi,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_BASE),
    );
    a.mov_rm(
        true,
        Gp::Rsi,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_CAP),
    );
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_mmap_to);
    a.cmp_rr(true, Gp::Rsi, Gp::Rbx);
    a.jcc(Cond::Ae, lbl_have_to_space);
    // Existing `to_space` is too small -> munmap(to_base, to_cap) and clear.
    a.mov_ri32(Gp::Rax, SYS_MUNMAP);
    a.syscall();
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_BASE),
        Gp::Rax,
    );
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_CAP),
        Gp::Rax,
    );

    a.bind(lbl_mmap_to);
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_rr(true, Gp::Rsi, Gp::Rbx);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_oom);
    a.mov_rr(true, Gp::Rdi, Gp::Rax);
    a.mov_rr(true, Gp::Rsi, Gp::Rbx);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_BASE),
        Gp::Rdi,
    );
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_CAP),
        Gp::Rsi,
    );

    a.bind(lbl_have_to_space);
    // Pin Cheney copy invariants:
    //   R12 = from_base, R13 = from_end, R14 = to_alloc (= to_base), Rbp = TypeDesc base, R15 = Ctx*
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rdi); // to_base
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 40), Gp::Rsi); // to_cap
    a.mov_rr(true, Gp::R14, Gp::Rdi); // to_alloc
    a.lea_rip(Gp::Rbp, RelocTarget::Rodata(gc.type_desc_ro_off));

    // ---- Root Set 1: Traced GC/Extern Globals ----
    if gc.gc_root_globals_count > 0 {
        let lp = a.new_label();
        a.lea_rip(Gp::R9, RelocTarget::Rodata(gc.gc_root_globals_ro_off));
        a.mov_ri32(Gp::R10, gc.gc_root_globals_count);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp);
        a.movsxd_rm(Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 4, 0));
        a.add_rr(true, Gp::Rdi, Gp::R15);
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp);
    }

    // ---- Root Set 2: Traced Inline Extern Tables ----
    if gc.gc_root_inline_tables_count > 0 {
        let lp_outer = a.new_label();
        let lp_inner = a.new_label();
        a.lea_rip(
            Gp::Rax,
            RelocTarget::Rodata(gc.gc_root_inline_tables_ro_off),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
        a.mov_ri64(Gp::Rax, u64::from(gc.gc_root_inline_tables_count));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
        a.bind(lp_outer);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
        a.movsxd_rm(Gp::R9, Mem::base_disp(Gp::Rax, 0));
        a.add_rr(true, Gp::R9, Gp::R15); // elem 0 ptr
        a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rax, 4)); // elem len
        a.add_ri(true, Gp::Rax, 8);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp_inner);
        a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 8, 0));
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp_inner);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 56));
        a.sub_ri(true, Gp::Rax, 1);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
        a.jcc(Cond::Ne, lp_outer);
    }

    // ---- Root Set 3: Traced Dynamic Tables ----
    if gc.gc_root_dyn_tables_count > 0 {
        let lp_outer = a.new_label();
        let lp_inner = a.new_label();
        let lbl_skip_tbl = a.new_label();
        a.lea_rip(Gp::Rax, RelocTarget::Rodata(gc.gc_root_dyn_tables_ro_off));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
        a.mov_ri64(Gp::Rax, u64::from(gc.gc_root_dyn_tables_count));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
        a.bind(lp_outer);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
        a.movsxd_rm(Gp::Rcx, Mem::base_disp(Gp::Rax, 0)); // ctx_desc_off
        a.add_ri(true, Gp::Rax, 4);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
        a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R15, Gp::Rcx, 1, 0)); // tbl_ptr
        a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R15, Gp::Rcx, 1, 8)); // cur_len
        a.test_rr(true, Gp::R9, Gp::R9);
        a.jcc(Cond::E, lbl_skip_tbl);
        a.test_rr(true, Gp::R10, Gp::R10);
        a.jcc(Cond::E, lbl_skip_tbl);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp_inner);
        a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 8, 0));
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp_inner);
        a.bind(lbl_skip_tbl);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 56));
        a.sub_ri(true, Gp::Rax, 1);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
        a.jcc(Cond::Ne, lp_outer);
    }

    // ---- Root Set 4: Non-Dropped Traced Passive GC Element Segments ----
    if gc.gc_elem_buf_bytes > 0 && !module.elem_segs.is_empty() {
        let lbl_no_elem_buf = a.new_label();
        let lp_seg = a.new_label();
        let lbl_next_seg = a.new_label();
        let lp_elem = a.new_label();
        a.mov_rm(
            true,
            Gp::Rax,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_ELEM_BUF_PTR),
        );
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::E, lbl_no_elem_buf);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax); // gc_elem_buf_ptr
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax); // ei = 0

        a.bind(lp_seg);
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 48)); // ei
        a.lea_rip(Gp::Rdx, RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
        a.mov_rr(true, Gp::Rax, Gp::Rcx);
        a.shl_imm(true, Gp::Rax, 4); // ei * 16
        a.mov_rm(false, Gp::R8, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 12)); // is_traced
        a.test_rr(false, Gp::R8, Gp::R8);
        a.jcc(Cond::E, lbl_next_seg);
        if let Some(drop_off) = layout.elem_dropped_off {
            a.movzx_rm8(false, Gp::R8, Mem::indexed(Gp::R15, Gp::Rcx, 1, drop_off));
            a.test_rr(false, Gp::R8, Gp::R8);
            a.jcc(Cond::Ne, lbl_next_seg);
        }
        a.mov_rm(false, Gp::R10, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 4)); // elem_len
        a.test_rr(true, Gp::R10, Gp::R10);
        a.jcc(Cond::E, lbl_next_seg);
        a.movsxd_rm(Gp::R9, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 8)); // gc_buf_byte_off
        a.add_rm(true, Gp::R9, Mem::base_disp(Gp::Rsp, 56)); // seg ptr
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp_elem);
        a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 8, 0));
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp_elem);

        a.bind(lbl_next_seg);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
        a.add_ri(true, Gp::Rax, 1);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
        a.cmp_ri(true, Gp::Rax, module.elem_segs.len() as i32);
        a.jcc(Cond::B, lp_seg);
        a.bind(lbl_no_elem_buf);
    }

    // ---- Root Set 5: Live Exception Slab Records ----
    if let Some(exn_off) = layout.exn_state_off
        && gc.has_traced_exn_tags
    {
        let lbl_exn_done = a.new_label();
        let lp_rec = a.new_label();
        let lp_fld = a.new_label();
        let lbl_next_rec = a.new_label();
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, exn_off + 8)); // slab_ptr
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::E, lbl_exn_done);
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, exn_off + 16)); // slab_cursor
        a.lea(true, Gp::Rdx, Mem::indexed(Gp::Rax, Gp::Rcx, 1, 0)); // slab_end
        a.add_ri(true, Gp::Rax, 16); // cur_rec = slab_ptr + 16
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rdx); // slab_end
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax); // cur_rec

        a.bind(lp_rec);
        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 56)); // cur_rec
        a.cmp_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 48));
        a.jcc(Cond::Ae, lbl_exn_done);
        a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::Rbx, 0)); // tag_idx
        a.lea_rip(Gp::Rdx, RelocTarget::Rodata(gc.gc_root_exn_tags_ro_off));
        a.shl_imm(true, Gp::Rax, 4); // tag_idx * 16
        a.mov_rm(false, Gp::R9, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 0)); // rec_size
        a.mov_rm(false, Gp::R10, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 8)); // num_refs
        a.test_rr(true, Gp::R10, Gp::R10);
        a.jcc(Cond::E, lbl_next_rec);
        a.movsxd_rm(Gp::Rcx, Mem::indexed(Gp::Rdx, Gp::Rax, 1, 4)); // ref_offs_rel
        a.add_rr(true, Gp::Rdx, Gp::Rcx); // ref_offs_ptr
        // Save ref_offs_ptr in R9? No, advance [rsp+56] = cur_rec + rec_size now, freeing R9!
        a.lea(true, Gp::Rax, Mem::indexed(Gp::Rbx, Gp::R9, 1, 0));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
        a.mov_rr(true, Gp::R9, Gp::Rdx); // ref_offs_ptr
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp_fld);
        a.movsxd_rm(Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 4, 0));
        a.add_rr(true, Gp::Rdi, Gp::Rbx);
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp_fld);
        a.jmp(lp_rec);

        a.bind(lbl_next_rec);
        a.add_rr(true, Gp::Rbx, Gp::R9);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rbx);
        a.jmp(lp_rec);
        a.bind(lbl_exn_done);
    }

    // ---- Root Set 6: `wt_initialize` Temporary Const-Expr Init Roots ----
    {
        let lbl_no_init_roots = a.new_label();
        let lp_init = a.new_label();
        a.mov_rm(
            true,
            Gp::R9,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_PTR),
        );
        a.mov_rm(
            true,
            Gp::R10,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_LEN),
        );
        a.test_rr(true, Gp::R9, Gp::R9);
        a.jcc(Cond::E, lbl_no_init_roots);
        a.test_rr(true, Gp::R10, Gp::R10);
        a.jcc(Cond::E, lbl_no_init_roots);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.bind(lp_init);
        a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::R11, 8, 0));
        a.call_label(lbl_evacuate);
        a.add_ri(true, Gp::R11, 1);
        a.cmp_rr(true, Gp::R11, Gp::R10);
        a.jcc(Cond::B, lp_init);
        a.bind(lbl_no_init_roots);
    }

    // ---- Root Set 7: Guest Call Stack Walk via Precise Return-Address SafepointTable ----
    {
        let lbl_walk_seg = a.new_label();
        let lbl_walk_frame = a.new_label();
        let lbl_bsearch_lp = a.new_label();
        let lbl_bsearch_lt = a.new_label();
        let lbl_bsearch_found = a.new_label();
        let lbl_slot_lp = a.new_label();
        let lbl_slot_done = a.new_label();
        let lbl_next_chain = a.new_label();
        let lbl_walk_done = a.new_label();
        let lbl_bsearch_miss = a.new_label();

        a.mov_rm(
            true,
            Gp::Rax,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_STACK_CHAIN),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax); // cur_chain
        a.mov_rm(
            true,
            Gp::Rax,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_WALK_STOP_RSP),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax); // stop_rsp
        a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 0)); // cur_ret_rsp

        a.bind(lbl_walk_seg);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 56));
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::E, lbl_next_chain);

        a.bind(lbl_walk_frame);
        // Rdx = cur_ret_rsp; stop when cur_ret_rsp >= stop_rsp.
        a.cmp_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 56));
        a.jcc(Cond::Ae, lbl_next_chain);

        // Compute `key = [cur_ret_rsp] - GuestCodeStart` (u32 in Rax).
        a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rdx));
        a.lea_rip(Gp::Rcx, RelocTarget::GuestCodeStart);
        a.sub_rr(true, Gp::Rax, Gp::Rcx);
        // Save frame_rsp = cur_ret_rsp + 8 in Rbx.
        a.lea(true, Gp::Rbx, Mem::base_disp(Gp::Rdx, 8));

        // Binary search SafepointHeader entries for `ret_pc_rel == eax`.
        a.lea_rip(Gp::Rsi, RelocTarget::GcSafepointHeader);
        a.mov_rm(false, Gp::Rdx, Mem::base_disp(Gp::Rsi, 0)); // hi = num_entries
        a.test_rr(false, Gp::Rdx, Gp::Rdx);
        a.jcc(Cond::E, lbl_bsearch_miss);
        a.movsxd_rm(Gp::R8, Mem::base_disp(Gp::Rsi, 8));
        a.add_rr(true, Gp::R8, Gp::Rsi); // R8 = entries_ptr
        a.xor_rr(false, Gp::Rcx, Gp::Rcx); // lo = 0

        a.bind(lbl_bsearch_lp);
        a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
        a.jcc(Cond::Ae, lbl_bsearch_miss);
        a.lea(true, Gp::R9, Mem::indexed(Gp::Rcx, Gp::Rdx, 1, 0));
        a.shr_imm(true, Gp::R9, 1); // mid = (lo + hi) >> 1
        a.mov_rr(true, Gp::R10, Gp::R9);
        a.shl_imm(true, Gp::R10, 4); // mid * 16
        a.mov_rm(false, Gp::R11, Mem::indexed(Gp::R8, Gp::R10, 1, 0));
        a.cmp_rr(false, Gp::R11, Gp::Rax);
        a.jcc(Cond::E, lbl_bsearch_found);
        a.jcc(Cond::B, lbl_bsearch_lt);
        a.mov_rr(true, Gp::Rdx, Gp::R9); // hi = mid
        a.jmp(lbl_bsearch_lp);
        a.bind(lbl_bsearch_lt);
        a.lea(true, Gp::Rcx, Mem::base_disp(Gp::R9, 1)); // lo = mid + 1
        a.jmp(lbl_bsearch_lp);

        a.bind(lbl_bsearch_miss);
        a.ud2();

        a.bind(lbl_bsearch_found);
        // R9 = entry_ptr = R8 + R10 (preserved across lbl_evacuate).
        a.lea(true, Gp::R9, Mem::indexed(Gp::R8, Gp::R10, 1, 0));
        a.mov_rm(false, Gp::R11, Mem::base_disp(Gp::R9, 12)); // num_slots
        a.test_rr(true, Gp::R11, Gp::R11);
        a.jcc(Cond::E, lbl_slot_done);
        // R10 = &slots[first_slot_idx] (preserved across lbl_evacuate).
        a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::Rsi, 12));
        a.add_rr(true, Gp::R10, Gp::Rsi); // slots_base
        a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R9, 8)); // first_slot_idx
        a.lea(true, Gp::R10, Mem::indexed(Gp::R10, Gp::Rax, 4, 0));
        // Count down R11 from num_slots to 0 (R11 preserved across lbl_evacuate!).
        a.bind(lbl_slot_lp);
        a.sub_ri(true, Gp::R11, 1);
        a.movsxd_rm(Gp::Rdi, Mem::indexed(Gp::R10, Gp::R11, 4, 0));
        a.add_rr(true, Gp::Rdi, Gp::Rbx); // frame_rsp + slot_off
        a.call_label(lbl_evacuate);
        a.test_rr(true, Gp::R11, Gp::R11);
        a.jcc(Cond::Ne, lbl_slot_lp);

        a.bind(lbl_slot_done);
        // Advance cur_ret_rsp = frame_rsp + entry.frame_caller_ret_off.
        a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R9, 4));
        a.lea(true, Gp::Rdx, Mem::indexed(Gp::Rbx, Gp::Rax, 1, 0));
        a.jmp(lbl_walk_frame);

        a.bind(lbl_next_chain);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::E, lbl_walk_done);
        // Chain node layout: +0: prev_chain, +8: outer_stop_rsp, +16: outer_paused_ret_rsp
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rax, 0));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rcx);
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rax, 8));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rcx);
        a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rax, 16));
        a.jmp(lbl_walk_seg);

        a.bind(lbl_walk_done);
    }

    // =========================================================================
    // Cheney BFS Scan Loop: `scan` (Rbx) from `to_base` up to `to_alloc` (R14).
    // =========================================================================
    {
        let lbl_scan_lp = a.new_label();
        let lbl_scan_done = a.new_label();
        let lbl_scan_array = a.new_label();
        let lbl_scan_box = a.new_label();
        let lbl_struct_fld_lp = a.new_label();
        let lbl_struct_adv = a.new_label();
        let lbl_arr_elem_lp = a.new_label();
        let lbl_arr_adv = a.new_label();

        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 32)); // scan = to_base
        a.bind(lbl_scan_lp);
        a.cmp_rr(true, Gp::Rbx, Gp::R14);
        a.jcc(Cond::Ae, lbl_scan_done);

        a.mov_rm(false, Gp::Rax, Mem::base(Gp::Rbx)); // low 32b header
        a.mov_rr(false, Gp::Rcx, Gp::Rax);
        a.and_ri(false, Gp::Rcx, 6);
        a.cmp_ri(false, Gp::Rcx, 2);
        a.jcc(Cond::E, lbl_scan_array);
        a.jcc(Cond::A, lbl_scan_box);

        // ---- Scan Struct ----
        a.shr_imm(false, Gp::Rax, 3); // canon_id
        a.shl_imm(true, Gp::Rax, 6); // * 64
        a.lea(true, Gp::R9, Mem::indexed(Gp::Rbp, Gp::Rax, 1, 0)); // TypeDesc*
        a.mov_rm(false, Gp::R11, Mem::base_disp(Gp::R9, 12)); // num_traced_fields
        a.test_rr(true, Gp::R11, Gp::R11);
        a.jcc(Cond::E, lbl_struct_adv);
        a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::R9, 8));
        a.add_rr(true, Gp::R10, Gp::Rbp); // &[u16] field offsets
        a.bind(lbl_struct_fld_lp);
        a.sub_ri(true, Gp::R11, 1);
        a.movzx_rm16(true, Gp::Rdi, Mem::indexed(Gp::R10, Gp::R11, 2, 0));
        a.add_rr(true, Gp::Rdi, Gp::Rbx);
        a.call_label(lbl_evacuate);
        a.test_rr(true, Gp::R11, Gp::R11);
        a.jcc(Cond::Ne, lbl_struct_fld_lp);
        a.bind(lbl_struct_adv);
        a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R9, 4)); // struct size
        a.add_rr(true, Gp::Rbx, Gp::Rax);
        a.jmp(lbl_scan_lp);

        // ---- Scan Array ----
        a.bind(lbl_scan_array);
        a.shr_imm(false, Gp::Rax, 3);
        a.shl_imm(true, Gp::Rax, 6);
        a.lea(true, Gp::R9, Mem::indexed(Gp::Rbp, Gp::Rax, 1, 0)); // TypeDesc*
        a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rbx, 4)); // arr_len
        a.movzx_rm8(false, Gp::Rcx, Mem::base_disp(Gp::R9, 1)); // elem_ref_kind
        a.test_rr(false, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::E, lbl_arr_adv);
        a.test_rr(true, Gp::R10, Gp::R10);
        a.jcc(Cond::E, lbl_arr_adv);
        a.mov_rr(true, Gp::R11, Gp::R10);
        a.bind(lbl_arr_elem_lp);
        a.sub_ri(true, Gp::R11, 1);
        a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rbx, Gp::R11, 8, 8));
        a.call_label(lbl_evacuate);
        a.test_rr(true, Gp::R11, Gp::R11);
        a.jcc(Cond::Ne, lbl_arr_elem_lp);
        a.bind(lbl_arr_adv);
        a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R9, 4)); // elem_sz
        a.imul_rr(true, Gp::Rax, Gp::R10);
        a.add_ri(true, Gp::Rax, 15);
        a.and_ri(true, Gp::Rax, -8);
        a.add_rr(true, Gp::Rbx, Gp::Rax);
        a.jmp(lbl_scan_lp);

        // ---- Scan InternalExternBox (16B, holds opaque host externref at +8) ----
        a.bind(lbl_scan_box);
        a.add_ri(true, Gp::Rbx, 16);
        a.jmp(lbl_scan_lp);

        a.bind(lbl_scan_done);
    }

    // =========================================================================
    // Flip semi-spaces, reclaim old `from_space`, and check growth / OOM.
    // =========================================================================
    {
        // R12 = old_from_base, R13 = old_from_end, R14 = new_alloc_ptr
        // [rsp+32] = new_from_base, [rsp+40] = new_from_cap
        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 32)); // new_from_base
        a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::Rsp, 40)); // new_from_cap
        a.lea(true, Gp::Rax, Mem::indexed(Gp::Rbx, Gp::Rbp, 1, 0)); // new_alloc_limit
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_ALLOC_PTR),
            Gp::R14,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_ALLOC_LIMIT),
            Gp::Rax,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_BASE),
            Gp::Rbx,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_CAP),
            Gp::Rbp,
        );
        a.mov_rm(
            true,
            Gp::Rcx,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_GC_COUNT),
        );
        a.add_ri(true, Gp::Rcx, 1);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_GC_COUNT),
            Gp::Rcx,
        );

        // Compute `old_from_cap = R13 - R12` in Rsi; if `old_from_cap < new_from_cap`, munmap it;
        // else keep it as `to_space` and `madvise(MADV_DONTNEED)` to drop RSS pages immediately.
        let lbl_keep_old = a.new_label();
        let lbl_reclaimed = a.new_label();
        a.mov_rr(true, Gp::Rdi, Gp::R12);
        a.mov_rr(true, Gp::Rsi, Gp::R13);
        a.sub_rr(true, Gp::Rsi, Gp::R12);
        a.cmp_rr(true, Gp::Rsi, Gp::Rbp);
        a.jcc(Cond::Ae, lbl_keep_old);
        // Old from-space was smaller -> unmap and zero to_space fields.
        a.mov_ri32(Gp::Rax, SYS_MUNMAP);
        a.syscall();
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_BASE),
            Gp::Rax,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_CAP),
            Gp::Rax,
        );
        a.jmp(lbl_reclaimed);

        a.bind(lbl_keep_old);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_BASE),
            Gp::Rdi,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_TO_CAP),
            Gp::Rsi,
        );
        a.mov_ri32(Gp::Rdx, MADV_DONTNEED);
        a.mov_ri32(Gp::Rax, SYS_MADVISE);
        a.syscall();

        a.bind(lbl_reclaimed);
        // Check if allocation fits AND whether amortized 2x growth is needed.
        // R14 = new_alloc_ptr, Rbx = new_from_base, Rbp = new_from_cap.
        a.mov_rr(true, Gp::R8, Gp::R14);
        a.sub_rr(true, Gp::R8, Gp::Rbx); // R8 = live_bytes
        a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::Rsp, 8)); // R9 = req_bytes
        a.mov_rr(true, Gp::R10, Gp::R8);
        a.add_rr(true, Gp::R10, Gp::R9); // R10 = need_bytes = live_bytes + req_bytes
        a.jcc(Cond::B, lbl_oom); // u64 overflow guard

        let lbl_need_grow = a.new_label();
        a.cmp_rr(true, Gp::R10, Gp::Rbp);
        a.jcc(Cond::A, lbl_need_grow);

        // Fits in `new_from_cap`! Also grow on pass 0 if `live_bytes * 2 > new_from_cap` and `new_from_cap < heap_cap`.
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16)); // gc_pass
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::Ne, lbl_done_ok);
        a.mov_rm(
            true,
            Gp::Rdx,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_HEAP_CAP),
        );
        a.cmp_rr(true, Gp::Rbp, Gp::Rdx);
        a.jcc(Cond::Ae, lbl_done_ok);
        a.lea(true, Gp::Rcx, Mem::indexed(Gp::R8, Gp::R8, 1, 0)); // live_bytes * 2
        a.cmp_rr(true, Gp::Rcx, Gp::Rbp);
        a.jcc(Cond::Be, lbl_done_ok);

        a.bind(lbl_need_grow);
        // Can only grow if `gc_pass == 0` AND `need_bytes <= heap_cap_bytes`.
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::Ne, lbl_oom);
        a.mov_rm(
            true,
            Gp::Rdx,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_HEAP_CAP),
        );
        a.cmp_rr(true, Gp::R10, Gp::Rdx);
        a.jcc(Cond::A, lbl_oom);
        // target_cap = min(heap_cap, max(new_from_cap * 2, align_up_4k(need_bytes)))
        a.mov_rr(true, Gp::Rcx, Gp::R10);
        a.add_ri(true, Gp::Rcx, 4095);
        a.and_ri(true, Gp::Rcx, -4096); // align_up_4k(need_bytes)
        // Prefer 2x headroom (`align_up_4k(need_bytes) * 2`) when it fits in heap_cap.
        a.lea(true, Gp::Rax, Mem::indexed(Gp::Rbp, Gp::Rbp, 1, 0)); // new_from_cap * 2
        a.cmp_rr(true, Gp::Rax, Gp::Rcx);
        a.cmovcc(Cond::B, true, Gp::Rax, Gp::Rcx);
        a.cmp_rr(true, Gp::Rax, Gp::Rdx);
        a.cmovcc(Cond::A, true, Gp::Rax, Gp::Rdx); // clamp to heap_cap
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
        a.mov_ri32(Gp::Rax, 1);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax); // gc_pass = 1
        a.jmp(lbl_gc_pass);
    }

    // ---- Return paths ----
    a.bind(lbl_done_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(lbl_ret_common);

    a.bind(lbl_oom);
    a.mov_ri32(Gp::R11, u32::from(TRAP_ALLOC_FAILED));

    a.bind(lbl_ret_common);
    a.add_ri(true, Gp::Rsp, 64);
    for &r in saved_regs.iter().rev() {
        a.pop(r);
    }
    a.ret();

    // =========================================================================
    // Local subroutine `lbl_evacuate(Rdi = slot_ptr)`
    //   Preserves: Rbx, Rbp, R9, R10, R11, R12, R13, R15 (advances R14 = to_alloc).
    //   Clobbers : Rax, Rcx, Rdx, Rsi, Rdi, R8.
    // =========================================================================
    {
        let lbl_ev_ret = a.new_label();
        let lbl_ev_fwd = a.new_label();
        let lbl_ev_array = a.new_label();
        let lbl_ev_box = a.new_label();
        let lbl_ev_copy = a.new_label();

        a.bind(lbl_evacuate);
        a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rdi));
        a.test_ri(true, Gp::Rax, 7);
        a.jcc(Cond::Ne, lbl_ev_ret);
        a.cmp_rr(true, Gp::Rax, Gp::R12);
        a.jcc(Cond::B, lbl_ev_ret);
        a.cmp_rr(true, Gp::Rax, Gp::R13);
        a.jcc(Cond::Ae, lbl_ev_ret);

        a.mov_rm(true, Gp::Rdx, Mem::base(Gp::Rax)); // 8B header
        a.test_ri(false, Gp::Rdx, 1);
        a.jcc(Cond::Ne, lbl_ev_fwd);

        // Live object in from-space: compute 8-byte-aligned object size in Rcx.
        a.mov_rr(false, Gp::R8, Gp::Rdx);
        a.and_ri(false, Gp::R8, 6);
        a.cmp_ri(false, Gp::R8, 2);
        a.jcc(Cond::E, lbl_ev_array);
        a.jcc(Cond::A, lbl_ev_box);

        // Struct: size = TypeDesc[hdr >> 3].size_or_elem_sz
        a.mov_rr(false, Gp::Rcx, Gp::Rdx);
        a.shr_imm(false, Gp::Rcx, 3);
        a.shl_imm(true, Gp::Rcx, 6);
        a.mov_rm(false, Gp::Rcx, Mem::indexed(Gp::Rbp, Gp::Rcx, 1, 4));
        a.jmp(lbl_ev_copy);

        // Array: size = align8(8 + len * elem_sz)
        a.bind(lbl_ev_array);
        a.mov_rr(false, Gp::Rcx, Gp::Rdx);
        a.shr_imm(false, Gp::Rcx, 3);
        a.shl_imm(true, Gp::Rcx, 6);
        a.mov_rm(false, Gp::Rcx, Mem::indexed(Gp::Rbp, Gp::Rcx, 1, 4));
        a.mov_rm(false, Gp::R8, Mem::base_disp(Gp::Rax, 4)); // len (u32 zero-ext)
        a.imul_rr(true, Gp::Rcx, Gp::R8);
        a.add_ri(true, Gp::Rcx, 15);
        a.and_ri(true, Gp::Rcx, -8);
        a.jmp(lbl_ev_copy);

        // InternalExternBox: 16 bytes
        a.bind(lbl_ev_box);
        a.mov_ri32(Gp::Rcx, 16);

        a.bind(lbl_ev_copy);
        a.mov_rr(true, Gp::R8, Gp::Rdi); // save slot_ptr
        a.mov_rr(true, Gp::Rsi, Gp::Rax); // src = old_obj
        a.mov_rr(true, Gp::Rdi, Gp::R14); // dst = new_obj
        a.mov_rr(true, Gp::Rdx, Gp::R14); // save new_obj
        a.add_rr(true, Gp::R14, Gp::Rcx); // advance to_alloc
        a.shr_imm(true, Gp::Rcx, 3); // qword count
        a.rep_movsq();
        // Store forwarding pointer (new_obj | 1) in [old_obj + 0], update *slot_ptr = new_obj.
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.or_ri(true, Gp::Rcx, 1);
        a.mov_mr(true, Mem::base(Gp::Rax), Gp::Rcx);
        a.mov_mr(true, Mem::base(Gp::R8), Gp::Rdx);
        a.ret();

        a.bind(lbl_ev_fwd);
        a.and_ri(true, Gp::Rdx, -2);
        a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rdx);
        a.bind(lbl_ev_ret);
        a.ret();
    }

    a.finish_labels()?;
    Ok(a)
}

/// Emits `HELPER_GC_ARRAY_FILL_RAW`: fills `[obj_ptr + 8 .. + 8 + len * elem_sz]` with `fill`.
///
/// Inputs: `Rdi = obj_ptr`, `Rsi = len: u32`, `Rdx = fill_u64` (or `Xmm15` for `elem_sz == 16`),
/// `Rcx = elem_sz: u32` (`1, 2, 4, 8, 16`).
/// Output: `Rax = obj_ptr`. Clobbers caller-saved GP only (preserves `Rbx, Rbp, R12..R15`, all XMM).
pub fn emit_helper_gc_array_fill_raw() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    let lbl_done = a.new_label();
    let lbl_sz1 = a.new_label();
    let lbl_sz2 = a.new_label();
    let lbl_sz4 = a.new_label();
    let lbl_sz8 = a.new_label();
    let lbl_sz16 = a.new_label();

    a.mov_rr(true, Gp::R8, Gp::Rdi); // preserve obj_ptr in R8
    a.mov_rr(false, Gp::Rsi, Gp::Rsi); // zero-ext len
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_done);
    a.add_ri(true, Gp::Rdi, 8); // dst = obj_ptr + 8

    a.cmp_ri(false, Gp::Rcx, 8);
    a.jcc(Cond::E, lbl_sz8);
    a.jcc(Cond::A, lbl_sz16);
    a.cmp_ri(false, Gp::Rcx, 4);
    a.jcc(Cond::E, lbl_sz4);
    a.cmp_ri(false, Gp::Rcx, 2);
    a.jcc(Cond::E, lbl_sz2);

    // elem_sz == 1
    a.bind(lbl_sz1);
    a.mov_rr(false, Gp::Rax, Gp::Rdx);
    a.mov_rr(true, Gp::Rcx, Gp::Rsi);
    a.rep_stosb();
    a.jmp(lbl_done);

    // elem_sz == 2
    a.bind(lbl_sz2);
    let lp2 = a.new_label();
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(lp2);
    a.mov_mr16(Mem::indexed(Gp::Rdi, Gp::Rcx, 2, 0), Gp::Rdx);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::Rsi);
    a.jcc(Cond::B, lp2);
    a.jmp(lbl_done);

    // elem_sz == 4
    a.bind(lbl_sz4);
    let lp4 = a.new_label();
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(lp4);
    a.mov_mr(false, Mem::indexed(Gp::Rdi, Gp::Rcx, 4, 0), Gp::Rdx);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::Rsi);
    a.jcc(Cond::B, lp4);
    a.jmp(lbl_done);

    // elem_sz == 8
    a.bind(lbl_sz8);
    a.mov_rr(true, Gp::Rax, Gp::Rdx);
    a.mov_rr(true, Gp::Rcx, Gp::Rsi);
    a.rep_stosq();
    a.jmp(lbl_done);

    // elem_sz == 16 (`v128` in `Xmm15`)
    a.bind(lbl_sz16);
    let lp16 = a.new_label();
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(lp16);
    a.movdqu_mx(Mem::base(Gp::Rdi), Xmm::Xmm15);
    a.add_ri(true, Gp::Rdi, 16);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::Rsi);
    a.jcc(Cond::B, lp16);

    a.bind(lbl_done);
    a.mov_rr(true, Gp::Rax, Gp::R8);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits `HELPER_GC_ARRAY_FILL` (`array.fill`):
/// Inputs: `Rdi = arr_ptr`, `Rsi = dst_off: u32`, `Rdx = fill_u64` (or `Xmm15`), `Rcx = len: u32`,
/// `R8 = elem_sz: u32`. Returns `R11d = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
pub fn emit_helper_gc_array_fill() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(224);
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_sz2 = a.new_label();
    let lbl_sz4 = a.new_label();
    let lbl_sz8 = a.new_label();
    let lbl_sz16 = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);
    a.mov_rr(false, Gp::Rsi, Gp::Rsi); // dst_off u64
    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // len u64
    a.mov_rm(false, Gp::R9, Mem::base_disp(Gp::Rdi, 4)); // arr.len u64
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0)); // dst_off + len (<= 2^33, cannot wrap u64)
    a.cmp_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ok);

    // Compute dst_byte_ptr = arr_ptr + 8 + dst_off * elem_sz in Rdi.
    a.mov_rr(false, Gp::R8, Gp::R8);
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.imul_rr(true, Gp::Rax, Gp::R8);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rax, 1, 8));

    a.cmp_ri(false, Gp::R8, 8);
    a.jcc(Cond::E, lbl_sz8);
    a.jcc(Cond::A, lbl_sz16);
    a.cmp_ri(false, Gp::R8, 4);
    a.jcc(Cond::E, lbl_sz4);
    a.cmp_ri(false, Gp::R8, 2);
    a.jcc(Cond::E, lbl_sz2);

    // elem_sz == 1
    a.mov_rr(false, Gp::Rax, Gp::Rdx);
    a.rep_stosb();
    a.jmp(lbl_ok);

    // elem_sz == 2
    a.bind(lbl_sz2);
    let lp2 = a.new_label();
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.bind(lp2);
    a.mov_mr16(Mem::indexed(Gp::Rdi, Gp::R9, 2, 0), Gp::Rdx);
    a.add_ri(true, Gp::R9, 1);
    a.cmp_rr(true, Gp::R9, Gp::Rcx);
    a.jcc(Cond::B, lp2);
    a.jmp(lbl_ok);

    // elem_sz == 4
    a.bind(lbl_sz4);
    let lp4 = a.new_label();
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.bind(lp4);
    a.mov_mr(false, Mem::indexed(Gp::Rdi, Gp::R9, 4, 0), Gp::Rdx);
    a.add_ri(true, Gp::R9, 1);
    a.cmp_rr(true, Gp::R9, Gp::Rcx);
    a.jcc(Cond::B, lp4);
    a.jmp(lbl_ok);

    // elem_sz == 8
    a.bind(lbl_sz8);
    a.mov_rr(true, Gp::Rax, Gp::Rdx);
    a.rep_stosq();
    a.jmp(lbl_ok);

    // elem_sz == 16
    a.bind(lbl_sz16);
    let lp16 = a.new_label();
    a.bind(lp16);
    a.movdqu_mx(Mem::base(Gp::Rdi), Xmm::Xmm15);
    a.add_ri(true, Gp::Rdi, 16);
    a.sub_ri(true, Gp::Rcx, 1);
    a.jcc(Cond::Ne, lp16);

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_null);
    a.mov_ri32(Gp::R11, u32::from(TRAP_INDIRECT_CALL));
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits `HELPER_GC_ARRAY_COPY` (`array.copy`):
/// Inputs: `Rdi = dst_arr`, `Rsi = dst_off: u32`, `Rdx = src_arr`, `Rcx = src_off: u32`,
/// `R8 = len: u32`, `R9 = elem_sz: u32`. Returns `R11d = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
pub fn emit_helper_gc_array_copy() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(224);
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_bwd_q = a.new_label();
    let lbl_bwd_b = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_null);

    a.mov_rr(false, Gp::Rsi, Gp::Rsi); // dst_off
    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // src_off
    a.mov_rr(false, Gp::R8, Gp::R8); // len
    a.mov_rr(false, Gp::R9, Gp::R9); // elem_sz

    // Bounds checks: dst_off + len <= dst.len && src_off + len <= src.len
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rdi, 4));
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::R8, 1, 0));
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);

    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rdx, 4));
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rcx, Gp::R8, 1, 0));
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_ok);

    // dst_ptr = Rdi + 8 + dst_off * elem_sz; src_ptr = Rdx + 8 + src_off * elem_sz; byte_len = len * elem_sz.
    a.imul_rr(true, Gp::Rsi, Gp::R9);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rsi, 1, 8));
    a.imul_rr(true, Gp::Rcx, Gp::R9);
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::Rdx, Gp::Rcx, 1, 8));
    a.mov_rr(true, Gp::Rcx, Gp::R8);
    a.imul_rr(true, Gp::Rcx, Gp::R9); // byte_len

    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::E, lbl_ok);
    a.jcc(Cond::A, lbl_bwd);

    // Forward copy: qwords + tail bytes.
    a.mov_rr(true, Gp::Rdx, Gp::Rcx);
    a.shr_imm(true, Gp::Rcx, 3);
    a.rep_movsq();
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.and_ri(true, Gp::Rcx, 7);
    a.rep_movsb();
    a.jmp(lbl_ok);

    // Backward overlap-safe copy: qwords from top down, then tail bytes.
    a.bind(lbl_bwd);
    a.bind(lbl_bwd_q);
    a.cmp_ri(true, Gp::Rcx, 8);
    a.jcc(Cond::B, lbl_bwd_b);
    a.sub_ri(true, Gp::Rcx, 8);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
    a.jmp(lbl_bwd_q);
    a.bind(lbl_bwd_b);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ok);
    a.sub_ri(true, Gp::Rcx, 1);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
    a.jmp(lbl_bwd_b);

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_null);
    a.mov_ri32(Gp::R11, u32::from(TRAP_INDIRECT_CALL));
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits `HELPER_GC_ARRAY_INIT_DATA` (`array.init_data` & fill half of `array.new_data`):
/// Inputs: `Rdi = arr_ptr`, `Rsi = dst_elem_off: u32`, `Rdx = src_byte_off: u32`,
/// `Rcx = elem_len: u32`, `R8 = (data_idx << 32) | elem_sz`.
/// Returns `R11d = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
pub fn emit_helper_gc_array_init_data(
    passive_data_table_off: u32,
    data_dropped_off: i32,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(224);
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_not_dropped = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);

    a.mov_rr(false, Gp::Rsi, Gp::Rsi); // dst_elem_off
    a.mov_rr(false, Gp::Rdx, Gp::Rdx); // src_byte_off
    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // elem_len

    // Unpack R9 = data_idx, R8 = elem_sz.
    a.mov_rr(true, Gp::R9, Gp::R8);
    a.shr_imm(true, Gp::R9, 32);
    a.mov_rr(false, Gp::R8, Gp::R8);

    // Check `dst_elem_off + elem_len <= arr.len`.
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rdi, 4));
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);

    // Load data segment byte length (0 if dropped).
    a.lea_rip(Gp::R10, RelocTarget::Rodata(passive_data_table_off));
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R10, Gp::R9, 8, 4)); // seg_len
    a.movzx_rm8(
        false,
        Gp::R11,
        Mem::indexed(Gp::R15, Gp::R9, 1, data_dropped_off),
    );
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(lbl_not_dropped);

    // Compute `byte_len = elem_len * elem_sz` (u64, <= 16 * 2^32, cannot overflow u64).
    // Check `src_byte_off + byte_len <= seg_len`.
    a.imul_rr(true, Gp::Rcx, Gp::R8); // byte_len
    a.lea(true, Gp::R11, Mem::indexed(Gp::Rdx, Gp::Rcx, 1, 0));
    a.cmp_rr(true, Gp::R11, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ok);

    // dst = arr_ptr + 8 + dst_elem_off * elem_sz; src = seg_base + src_byte_off.
    a.imul_rr(true, Gp::Rsi, Gp::R8);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rsi, 1, 8));
    a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R10, Gp::R9, 8, 0));
    a.add_rr(true, Gp::R10, Gp::Rax); // seg_base
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R10, Gp::Rdx, 1, 0));
    a.rep_movsb();

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_null);
    a.mov_ri32(Gp::R11, u32::from(TRAP_INDIRECT_CALL));
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits `HELPER_GC_ARRAY_INIT_ELEM` (`array.init_elem` & fill half of `array.new_elem`):
/// Inputs: `Rdi = arr_ptr`, `Rsi = dst_elem_off: u32`, `Rdx = src_elem_off: u32`,
/// `Rcx = elem_len: u32`, `R8 = elem_idx: u32`.
/// Returns `R11d = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
pub fn emit_helper_gc_array_init_elem(
    gc: &GcCtxLayout,
    elem_dropped_off: i32,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(224);
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_dynamic = a.new_label();
    let lbl_lp_static = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);

    a.mov_rr(false, Gp::Rsi, Gp::Rsi); // dst_elem_off
    a.mov_rr(false, Gp::Rdx, Gp::Rdx); // src_elem_off
    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // elem_len
    a.mov_rr(false, Gp::R8, Gp::R8); // elem_idx

    // Check `dst_elem_off + elem_len <= arr.len`.
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rdi, 4));
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);

    // Load `GcElemDesc[elem_idx]` (16 bytes each).
    a.lea_rip(Gp::R9, RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
    a.mov_rr(true, Gp::R10, Gp::R8);
    a.shl_imm(true, Gp::R10, 4); // elem_idx * 16
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R9, Gp::R10, 1, 4)); // elem_len
    a.movzx_rm8(
        false,
        Gp::R11,
        Mem::indexed(Gp::R15, Gp::R8, 1, elem_dropped_off),
    );
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(lbl_not_dropped);

    // Check `src_elem_off + elem_len <= seg_len`.
    a.lea(true, Gp::R11, Mem::indexed(Gp::Rdx, Gp::Rcx, 1, 0));
    a.cmp_rr(true, Gp::R11, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ok);

    // dst = arr_ptr + 8 + dst_elem_off * 8.
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rsi, 8, 8));
    // Check `gc_buf_byte_off` (`i32` at +8): >= 0 means dynamic `u64` buffer, < 0 means static `u32` .rodata.
    a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R9, Gp::R10, 1, 8));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ge, lbl_dynamic);

    // Static `u32` entries in `.rodata`: zero-extend each `u32` into `[Rdi + i*8]`.
    a.movsxd_rm(Gp::R8, Mem::indexed(Gp::R9, Gp::R10, 1, 0));
    a.add_rr(true, Gp::R9, Gp::R8); // static u32 base
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R9, Gp::Rdx, 4, 0));
    a.xor_rr(false, Gp::R8, Gp::R8);
    a.bind(lbl_lp_static);
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::R8, 4, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::R8, 8, 0), Gp::Rax);
    a.add_ri(true, Gp::R8, 1);
    a.cmp_rr(true, Gp::R8, Gp::Rcx);
    a.jcc(Cond::B, lbl_lp_static);
    a.jmp(lbl_ok);

    // Dynamic `u64` entries in `Ctx.gc_elem_buf_ptr + gc_buf_byte_off + src_elem_off * 8`.
    a.bind(lbl_dynamic);
    a.add_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::R15, gc.gc_state_off + GC_OFF_ELEM_BUF_PTR),
    );
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::Rax, Gp::Rdx, 8, 0));
    a.rep_movsq();

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_null);
    a.mov_ri32(Gp::R11, u32::from(TRAP_INDIRECT_CALL));
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

// =============================================================================
// `wt_initialize` GC Constant-Expression Evaluator
// =============================================================================

/// Computes `(max_traced_root_slots, max_scalar_bytes)` needed on `wt_initialize`'s stack frame
/// to evaluate `expr` safely with full GC rooting of intermediate references.
fn const_expr_frame_need(expr: &GcConstExpr, module: &X64Module) -> (usize, usize) {
    match expr {
        GcConstExpr::Scalar(_) | GcConstExpr::GlobalGet(_) | GcConstExpr::StructNewDefault(_) => {
            (0, 0)
        }
        GcConstExpr::RefI31(inner)
        | GcConstExpr::AnyConvertExtern(inner)
        | GcConstExpr::ExternConvertAny(inner)
        | GcConstExpr::ArrayNewDefault { len: inner, .. } => const_expr_frame_need(inner, module),
        GcConstExpr::I32Bin(_, a, b) | GcConstExpr::I64Bin(_, a, b) => {
            let (ra, sa) = const_expr_frame_need(a, module);
            let (rb, sb) = const_expr_frame_need(b, module);
            (ra.max(rb), sa.max(8 + sb))
        }
        GcConstExpr::ArrayNew { ty, init, len } => {
            let gt = module.gc_type(*ty);
            let elem_storage = match gt.composite {
                GcCompositeInfo::Array { elem, .. } => elem,
                _ => GcStorageKind::I64,
            };
            let is_traced = X64Module::storage_ref_kind(elem_storage).is_traced();
            let elem_slot_bytes = if elem_storage == GcStorageKind::V128 {
                16
            } else {
                8
            };
            let (ri, si) = const_expr_frame_need(init, module);
            let (rl, sl) = const_expr_frame_need(len, module);
            if is_traced {
                (ri.max(1 + rl), si.max(sl))
            } else {
                (ri.max(rl), si.max(elem_slot_bytes + sl))
            }
        }
        GcConstExpr::StructNew { ty, fields } => {
            let gt = module.gc_type(*ty);
            let field_defs = match &gt.composite {
                GcCompositeInfo::Struct { fields, .. } => fields.as_slice(),
                _ => &[],
            };
            let (mut r_cur, mut s_cur) = (0usize, 0usize);
            let (mut r_max, mut s_max) = (0usize, 0usize);
            for (fe, fd) in fields.iter().zip(field_defs.iter()) {
                let (rf, sf) = const_expr_frame_need(fe, module);
                r_max = r_max.max(r_cur + rf);
                s_max = s_max.max(s_cur + sf);
                if X64Module::storage_ref_kind(fd.storage).is_traced() {
                    r_cur += 1;
                } else if fd.storage == GcStorageKind::V128 {
                    s_cur = (s_cur + 15) & !15;
                    s_cur += 16;
                } else {
                    s_cur += 8;
                }
            }
            (r_max.max(r_cur), s_max.max(s_cur))
        }
        GcConstExpr::ArrayNewFixed { ty, elems } => {
            let gt = module.gc_type(*ty);
            let elem_storage = match gt.composite {
                GcCompositeInfo::Array { elem, .. } => elem,
                _ => GcStorageKind::I64,
            };
            let is_traced = X64Module::storage_ref_kind(elem_storage).is_traced();
            let (mut r_cur, mut s_cur) = (0usize, 0usize);
            let (mut r_max, mut s_max) = (0usize, 0usize);
            for ee in elems {
                let (re, se) = const_expr_frame_need(ee, module);
                r_max = r_max.max(r_cur + re);
                s_max = s_max.max(s_cur + se);
                if is_traced {
                    r_cur += 1;
                } else if elem_storage == GcStorageKind::V128 {
                    s_cur = (s_cur + 15) & !15;
                    s_cur += 16;
                } else {
                    s_cur += 8;
                }
            }
            (r_max.max(r_cur), s_max.max(s_cur))
        }
    }
}

struct InitExprCtx<'a> {
    module: &'a X64Module,
    layout: &'a CtxLayout,
    gc_off: i32,
    scalar_base: i32,
    lbl_trap_alloc: Label,
}

impl<'a> InitExprCtx<'a> {
    fn emit_set_init_roots_len(&self, a: &mut Asm, n: usize) {
        a.mov_ri64(Gp::R10, n as u64);
        if self.layout.is_store {
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_LEN),
                Gp::R10,
            );
        } else {
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_INIT_ROOTS_LEN),
                Gp::R10,
            );
        }
    }

    /// Writes the 32-bit (or zero-extended 64-bit for structs) GC object header word to `[obj_reg + 0]`.
    /// Clobbers `Rcx` and `R11`.
    fn emit_write_header(
        &self,
        a: &mut Asm,
        obj_reg: Gp,
        gt: &super::module::GcCanonType,
        is_struct_64: bool,
    ) {
        if self.layout.is_store {
            let kind_bits = match gt.composite {
                GcCompositeInfo::Struct { .. } => super::module::GC_HDR_KIND_STRUCT,
                GcCompositeInfo::Array { .. } => super::module::GC_HDR_KIND_ARRAY,
                GcCompositeInfo::Func => unreachable!(),
            };
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_CANON_MAP));
            a.mov_rm(
                false,
                Gp::Rcx,
                Mem::base_disp(Gp::R11, (gt.canon_id as i32) * 4),
            );
            a.shl_imm(false, Gp::Rcx, 3);
            a.or_ri(false, Gp::Rcx, kind_bits as i32);
            a.mov_mr(is_struct_64, Mem::base_disp(obj_reg, 0), Gp::Rcx);
        } else if is_struct_64 {
            a.mov_ri64(Gp::Rcx, u64::from(gt.header_word()));
            a.mov_mr(true, Mem::base_disp(obj_reg, 0), Gp::Rcx);
        } else {
            a.mov_mi32(false, Mem::base_disp(obj_reg, 0), gt.header_word() as i32);
        }
    }

    /// Reserves `req_bytes` (constant) in GC heap via bump check + cold `HELPER_GC_COLLECT`,
    /// bumping `alloc_ptr` and returning `new_obj` in `Rax`. Clobbers `Rax, Rcx, R10, R11`.
    fn emit_alloc_fixed(&self, a: &mut Asm, size_bytes: u32) {
        let lbl_ok = a.new_label();
        if self.layout.is_store {
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR));
            a.lea(true, Gp::Rcx, Mem::base_disp(Gp::Rax, size_bytes as i32));
            a.cmp_rm(true, Gp::Rcx, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_LIMIT));
            a.jcc(Cond::Be, lbl_ok);
            a.mov_ri64(Gp::R10, u64::from(size_bytes));
            a.call_reloc(RelocTarget::Helper(HELPER_GC_COLLECT));
            a.test_rr(false, Gp::R11, Gp::R11);
            a.jcc(Cond::Ne, self.lbl_trap_alloc);
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR));
            a.lea(true, Gp::Rcx, Mem::base_disp(Gp::Rax, size_bytes as i32));
            a.bind(lbl_ok);
            a.mov_mr(true, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR), Gp::Rcx);
        } else {
            a.mov_rm(
                true,
                Gp::Rax,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
            );
            a.lea(true, Gp::Rcx, Mem::base_disp(Gp::Rax, size_bytes as i32));
            a.cmp_rm(
                true,
                Gp::Rcx,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_LIMIT),
            );
            a.jcc(Cond::Be, lbl_ok);
            a.mov_ri64(Gp::R10, u64::from(size_bytes));
            a.call_reloc(RelocTarget::Helper(HELPER_GC_COLLECT));
            a.test_rr(false, Gp::R11, Gp::R11);
            a.jcc(Cond::Ne, self.lbl_trap_alloc);
            a.mov_rm(
                true,
                Gp::Rax,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
            );
            a.lea(true, Gp::Rcx, Mem::base_disp(Gp::Rax, size_bytes as i32));
            a.bind(lbl_ok);
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
                Gp::Rcx,
            );
        }
    }

    /// Reserves dynamic `R10` bytes in GC heap, returning `new_obj` in `Rdi`.
    /// Preserves `Rsi` and `R10`; clobbers `Rcx`, `R11`.
    fn emit_alloc_dyn_r10_into_rdi(&self, a: &mut Asm) {
        let lbl_have = a.new_label();
        if self.layout.is_store {
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR));
            a.lea(true, Gp::Rcx, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));
            a.cmp_rr(true, Gp::Rcx, Gp::Rdi);
            a.jcc(Cond::B, self.lbl_trap_alloc);
            a.cmp_rm(true, Gp::Rcx, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_LIMIT));
            a.jcc(Cond::Be, lbl_have);
            a.call_reloc(RelocTarget::Helper(HELPER_GC_COLLECT));
            a.test_rr(false, Gp::R11, Gp::R11);
            a.jcc(Cond::Ne, self.lbl_trap_alloc);
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR));
            a.lea(true, Gp::Rcx, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));
            a.bind(lbl_have);
            a.mov_mr(true, Mem::base_disp(Gp::R11, GC_OFF_ALLOC_PTR), Gp::Rcx);
        } else {
            a.mov_rm(
                true,
                Gp::Rdi,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
            );
            a.lea(true, Gp::Rcx, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));
            a.cmp_rr(true, Gp::Rcx, Gp::Rdi);
            a.jcc(Cond::B, self.lbl_trap_alloc);
            a.cmp_rm(
                true,
                Gp::Rcx,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_LIMIT),
            );
            a.jcc(Cond::Be, lbl_have);
            a.call_reloc(RelocTarget::Helper(HELPER_GC_COLLECT));
            a.test_rr(false, Gp::R11, Gp::R11);
            a.jcc(Cond::Ne, self.lbl_trap_alloc);
            a.mov_rm(
                true,
                Gp::Rdi,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
            );
            a.lea(true, Gp::Rcx, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));
            a.bind(lbl_have);
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_ALLOC_PTR),
                Gp::Rcx,
            );
        }
    }

    /// Evaluates `expr` into `Rax` (or `Xmm15` when `expect_v128`).
    /// Active traced root slots live at `[rsp + 0 .. r_cur * 8]`.
    fn emit_eval(
        &self,
        a: &mut Asm,
        expr: &GcConstExpr,
        expect_v128: bool,
        r_cur: usize,
        s_cur: usize,
    ) -> Result<(), String> {
        match expr {
            GcConstExpr::Scalar(c) => match *c {
                X64ConstVal::I32(v) => a.mov_ri32(Gp::Rax, v as u32),
                X64ConstVal::I64(v) => a.mov_ri64(Gp::Rax, v as u64),
                X64ConstVal::F32(bits) => a.mov_ri32(Gp::Rax, bits),
                X64ConstVal::F64(bits) => a.mov_ri64(Gp::Rax, bits),
                X64ConstVal::RefNull => a.xor_rr(false, Gp::Rax, Gp::Rax),
                X64ConstVal::FuncRef(f) => {
                    if self.layout.is_store {
                        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
                        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rcx, (f as i32) * 8));
                    } else {
                        a.mov_ri64(Gp::Rax, u64::from(f) + 1);
                    }
                }
                X64ConstVal::I31(enc) => a.mov_ri64(Gp::Rax, u64::from(enc)),
                X64ConstVal::ExternI31(enc) => {
                    a.mov_ri64(Gp::Rax, GC_EXT_I31_TAG_HI | u64::from(enc));
                }
                X64ConstVal::V128(bytes) => {
                    if bytes == [0u8; 16] {
                        a.xorps(Xmm::Xmm15, Xmm::Xmm15);
                    } else {
                        let off = a.add_local_rodata(16, &bytes);
                        a.movdqu_rip(Xmm::Xmm15, RelocTarget::LocalRodata(off));
                    }
                }
                X64ConstVal::RuntimeGc => {
                    return Err("x64: unexpected RuntimeGc placeholder in GcConstExpr".into());
                }
            },
            GcConstExpr::GlobalGet(g) => {
                let gi = *g as usize;
                if let Some(c) = self.module.global_consts[gi] {
                    self.emit_eval(a, &GcConstExpr::Scalar(c), expect_v128, r_cur, s_cur)?;
                } else {
                    let off = self.layout.global_offs[gi];
                    let mem = if self.layout.is_store && (*g) < self.layout.imported_global_count {
                        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, off));
                        Mem::base(Gp::Rcx)
                    } else {
                        Mem::base_disp(Gp::R15, off)
                    };
                    if expect_v128 || self.module.globals[gi].0 == ValType::V128 {
                        a.movdqu_xm(Xmm::Xmm15, mem);
                    } else {
                        a.mov_rm(true, Gp::Rax, mem);
                    }
                }
            }
            GcConstExpr::RefI31(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                a.lea(false, Gp::Rax, Mem::indexed(Gp::Rax, Gp::Rax, 1, 1));
            }
            GcConstExpr::ExternConvertAny(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                let lbl_done = a.new_label();
                let lbl_i31 = a.new_label();
                a.test_rr(true, Gp::Rax, Gp::Rax);
                a.jcc(Cond::E, lbl_done);
                a.test_ri(false, Gp::Rax, 1);
                a.jcc(Cond::Ne, lbl_i31);
                a.mov_rm(false, Gp::Rcx, Mem::base(Gp::Rax));
                a.and_ri(false, Gp::Rcx, 6);
                a.cmp_ri(false, Gp::Rcx, GC_HDR_KIND_INTERNAL_EXTERN as i32);
                a.jcc(Cond::Ne, lbl_done);
                a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rax, 8));
                a.jmp(lbl_done);
                a.bind(lbl_i31);
                a.mov_ri64(Gp::Rcx, GC_EXT_I31_TAG_HI);
                a.or_rr(true, Gp::Rax, Gp::Rcx);
                a.bind(lbl_done);
            }
            GcConstExpr::AnyConvertExtern(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                let lbl_done = a.new_label();
                let lbl_untag_i31 = a.new_label();
                let lbl_box = a.new_label();
                a.test_rr(true, Gp::Rax, Gp::Rax);
                a.jcc(Cond::E, lbl_done);
                a.mov_rr(true, Gp::Rcx, Gp::Rax);
                a.shr_imm(true, Gp::Rcx, 32);
                a.cmp_ri(false, Gp::Rcx, (GC_EXT_I31_TAG_HI >> 32) as i32);
                a.jcc(Cond::E, lbl_untag_i31);
                a.test_ri(true, Gp::Rax, 7);
                a.jcc(Cond::Ne, lbl_box);
                if self.layout.is_store {
                    a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
                    a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R11, GC_OFF_FROM_BASE));
                    a.jcc(Cond::B, lbl_box);
                    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R11, GC_OFF_FROM_BASE));
                    a.add_rm(true, Gp::Rcx, Mem::base_disp(Gp::R11, GC_OFF_FROM_CAP));
                    a.cmp_rr(true, Gp::Rax, Gp::Rcx);
                    a.jcc(Cond::B, lbl_done);
                } else {
                    a.cmp_rm(
                        true,
                        Gp::Rax,
                        Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_FROM_BASE),
                    );
                    a.jcc(Cond::B, lbl_box);
                    a.mov_rm(
                        true,
                        Gp::Rcx,
                        Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_FROM_BASE),
                    );
                    a.add_rm(
                        true,
                        Gp::Rcx,
                        Mem::base_disp(Gp::R15, self.gc_off + GC_OFF_FROM_CAP),
                    );
                    a.cmp_rr(true, Gp::Rax, Gp::Rcx);
                    a.jcc(Cond::B, lbl_done);
                }

                a.bind(lbl_box);
                a.mov_rr(true, Gp::Rdx, Gp::Rax); // preserve opaque host externref across preserve-all GC
                self.emit_alloc_fixed(a, 16);
                a.mov_ri64(Gp::Rcx, u64::from(GC_HDR_KIND_INTERNAL_EXTERN));
                a.mov_mr(true, Mem::base_disp(Gp::Rax, 0), Gp::Rcx);
                a.mov_mr(true, Mem::base_disp(Gp::Rax, 8), Gp::Rdx);
                a.jmp(lbl_done);

                a.bind(lbl_untag_i31);
                a.mov_rr(false, Gp::Rax, Gp::Rax); // zero-ext low 32b encoded i31
                a.bind(lbl_done);
            }
            GcConstExpr::I32Bin(op, lhs, rhs) => {
                self.emit_eval(a, lhs, false, r_cur, s_cur)?;
                let slot = self.scalar_base + s_cur as i32;
                a.mov_mr(false, Mem::base_disp(Gp::Rsp, slot), Gp::Rax);
                self.emit_eval(a, rhs, false, r_cur, s_cur + 8)?;
                a.mov_rr(false, Gp::Rcx, Gp::Rax);
                a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::Rsp, slot));
                match *op {
                    0x6a => a.add_rr(false, Gp::Rax, Gp::Rcx),
                    0x6b => a.sub_rr(false, Gp::Rax, Gp::Rcx),
                    0x6c => a.imul_rr(false, Gp::Rax, Gp::Rcx),
                    other => return Err(format!("x64: unsupported i32 const op 0x{other:x}")),
                }
            }
            GcConstExpr::I64Bin(op, lhs, rhs) => {
                self.emit_eval(a, lhs, false, r_cur, s_cur)?;
                let slot = self.scalar_base + s_cur as i32;
                a.mov_mr(true, Mem::base_disp(Gp::Rsp, slot), Gp::Rax);
                self.emit_eval(a, rhs, false, r_cur, s_cur + 8)?;
                a.mov_rr(true, Gp::Rcx, Gp::Rax);
                a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, slot));
                match *op {
                    0x7c => a.add_rr(true, Gp::Rax, Gp::Rcx),
                    0x7d => a.sub_rr(true, Gp::Rax, Gp::Rcx),
                    0x7e => a.imul_rr(true, Gp::Rax, Gp::Rcx),
                    other => return Err(format!("x64: unsupported i64 const op 0x{other:x}")),
                }
            }
            GcConstExpr::StructNewDefault(ty) => {
                let gt = self.module.gc_type(*ty);
                let size = match gt.composite {
                    GcCompositeInfo::Struct { total_bytes, .. } => total_bytes,
                    _ => return Err("x64: StructNewDefault on non-struct".into()),
                };
                self.emit_alloc_fixed(a, size);
                self.emit_write_header(a, Gp::Rax, gt, true);
                let zero_qwords = (size - 8) / 8;
                if zero_qwords > 0 {
                    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
                    for q in 0..zero_qwords {
                        a.mov_mr(true, Mem::base_disp(Gp::Rax, 8 + (q as i32) * 8), Gp::Rcx);
                    }
                }
            }
            GcConstExpr::StructNew { ty, fields } => {
                let gt = self.module.gc_type(*ty);
                let (field_defs, size) = match &gt.composite {
                    GcCompositeInfo::Struct {
                        fields,
                        total_bytes,
                        ..
                    } => (fields.as_slice(), *total_bytes),
                    _ => return Err("x64: StructNew on non-struct".into()),
                };
                let mut r_next = r_cur;
                let mut s_next = s_cur;
                let mut saved_locs: Vec<(i32, GcStorageKind)> = Vec::with_capacity(fields.len());
                for (fe, fd) in fields.iter().zip(field_defs.iter()) {
                    let is_v128 = fd.storage == GcStorageKind::V128;
                    self.emit_eval(a, fe, is_v128, r_next, s_next)?;
                    if X64Module::storage_ref_kind(fd.storage).is_traced() {
                        let off = (r_next as i32) * 8;
                        a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                        r_next += 1;
                        self.emit_set_init_roots_len(a, r_next);
                        saved_locs.push((off, fd.storage));
                    } else if is_v128 {
                        s_next = (s_next + 15) & !15;
                        let off = self.scalar_base + s_next as i32;
                        a.movdqu_mx(Mem::base_disp(Gp::Rsp, off), Xmm::Xmm15);
                        s_next += 16;
                        saved_locs.push((off, fd.storage));
                    } else {
                        let off = self.scalar_base + s_next as i32;
                        a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                        s_next += 8;
                        saved_locs.push((off, fd.storage));
                    }
                }
                self.emit_alloc_fixed(a, size);
                self.emit_write_header(a, Gp::Rax, gt, true);
                for (&(src_off, st), fd) in saved_locs.iter().zip(field_defs.iter()) {
                    let dst = Mem::base_disp(Gp::Rax, fd.offset as i32);
                    match st {
                        GcStorageKind::I8 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr8(dst, Gp::Rcx);
                        }
                        GcStorageKind::I16 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr16(dst, Gp::Rcx);
                        }
                        GcStorageKind::I32 | GcStorageKind::F32 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr(false, dst, Gp::Rcx);
                        }
                        GcStorageKind::I64
                        | GcStorageKind::F64
                        | GcStorageKind::GcRef
                        | GcStorageKind::ExternRef
                        | GcStorageKind::OtherRef => {
                            a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr(true, dst, Gp::Rcx);
                        }
                        GcStorageKind::V128 => {
                            a.movdqu_xm(Xmm::Xmm15, Mem::base_disp(Gp::Rsp, src_off));
                            a.movdqu_mx(dst, Xmm::Xmm15);
                        }
                    }
                }
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
            GcConstExpr::ArrayNewDefault { ty, len } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("x64: ArrayNewDefault on non-array".into()),
                };
                let elem_sz = elem_storage.byte_size();
                self.emit_eval(a, len, false, r_cur, s_cur)?;
                // Eax = len (u32 zero-ext -> Rsi); compute req_bytes = align8(8 + len * elem_sz) in R10.
                a.mov_rr(false, Gp::Rsi, Gp::Rax);
                a.mov_ri64(Gp::R10, u64::from(elem_sz));
                a.imul_rr(true, Gp::R10, Gp::Rsi);
                a.add_ri(true, Gp::R10, 15);
                a.and_ri(true, Gp::R10, -8);
                self.emit_alloc_dyn_r10_into_rdi(a);
                self.emit_write_header(a, Gp::Rdi, gt, false);
                a.mov_mr(false, Mem::base_disp(Gp::Rdi, 4), Gp::Rsi);
                if elem_sz == 16 {
                    a.xorps(Xmm::Xmm15, Xmm::Xmm15);
                } else {
                    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
                }
                a.mov_ri32(Gp::Rcx, elem_sz);
                a.call_reloc(RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
            }
            GcConstExpr::ArrayNew { ty, init, len } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("x64: ArrayNew on non-array".into()),
                };
                let elem_sz = elem_storage.byte_size();
                let is_traced = X64Module::storage_ref_kind(elem_storage).is_traced();
                let is_v128 = elem_storage == GcStorageKind::V128;
                self.emit_eval(a, init, is_v128, r_cur, s_cur)?;
                let (init_slot, r_next, s_next) = if is_traced {
                    let off = (r_cur as i32) * 8;
                    a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                    self.emit_set_init_roots_len(a, r_cur + 1);
                    (off, r_cur + 1, s_cur)
                } else if is_v128 {
                    let s_al = (s_cur + 15) & !15;
                    let off = self.scalar_base + s_al as i32;
                    a.movdqu_mx(Mem::base_disp(Gp::Rsp, off), Xmm::Xmm15);
                    (off, r_cur, s_al + 16)
                } else {
                    let off = self.scalar_base + s_cur as i32;
                    a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                    (off, r_cur, s_cur + 8)
                };
                self.emit_eval(a, len, false, r_next, s_next)?;
                a.mov_rr(false, Gp::Rsi, Gp::Rax);
                a.mov_ri64(Gp::R10, u64::from(elem_sz));
                a.imul_rr(true, Gp::R10, Gp::Rsi);
                a.add_ri(true, Gp::R10, 15);
                a.and_ri(true, Gp::R10, -8);
                self.emit_alloc_dyn_r10_into_rdi(a);
                self.emit_write_header(a, Gp::Rdi, gt, false);
                a.mov_mr(false, Mem::base_disp(Gp::Rdi, 4), Gp::Rsi);
                if is_v128 {
                    a.movdqu_xm(Xmm::Xmm15, Mem::base_disp(Gp::Rsp, init_slot));
                } else {
                    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, init_slot));
                }
                a.mov_ri32(Gp::Rcx, elem_sz);
                a.call_reloc(RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
            GcConstExpr::ArrayNewFixed { ty, elems } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("x64: ArrayNewFixed on non-array".into()),
                };
                let elem_sz = elem_storage.byte_size();
                let is_traced = X64Module::storage_ref_kind(elem_storage).is_traced();
                let is_v128 = elem_storage == GcStorageKind::V128;
                let mut r_next = r_cur;
                let mut s_next = s_cur;
                let mut saved_offs: Vec<i32> = Vec::with_capacity(elems.len());
                for ee in elems {
                    self.emit_eval(a, ee, is_v128, r_next, s_next)?;
                    if is_traced {
                        let off = (r_next as i32) * 8;
                        a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                        r_next += 1;
                        self.emit_set_init_roots_len(a, r_next);
                        saved_offs.push(off);
                    } else if is_v128 {
                        s_next = (s_next + 15) & !15;
                        let off = self.scalar_base + s_next as i32;
                        a.movdqu_mx(Mem::base_disp(Gp::Rsp, off), Xmm::Xmm15);
                        s_next += 16;
                        saved_offs.push(off);
                    } else {
                        let off = self.scalar_base + s_next as i32;
                        a.mov_mr(true, Mem::base_disp(Gp::Rsp, off), Gp::Rax);
                        s_next += 8;
                        saved_offs.push(off);
                    }
                }
                let arr_len = u32::try_from(elems.len())
                    .map_err(|_| "x64: ArrayNewFixed length exceeds u32")?;
                let byte_size = u32::try_from((8usize + elems.len() * (elem_sz as usize) + 7) & !7)
                    .map_err(|_| "x64: ArrayNewFixed byte size exceeds u32")?;
                self.emit_alloc_fixed(a, byte_size);
                self.emit_write_header(a, Gp::Rax, gt, false);
                a.mov_mi32(false, Mem::base_disp(Gp::Rax, 4), arr_len as i32);
                for (i, &src_off) in saved_offs.iter().enumerate() {
                    let dst = Mem::base_disp(Gp::Rax, 8 + (i as i32) * (elem_sz as i32));
                    match elem_storage {
                        GcStorageKind::I8 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr8(dst, Gp::Rcx);
                        }
                        GcStorageKind::I16 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr16(dst, Gp::Rcx);
                        }
                        GcStorageKind::I32 | GcStorageKind::F32 => {
                            a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr(false, dst, Gp::Rcx);
                        }
                        GcStorageKind::I64
                        | GcStorageKind::F64
                        | GcStorageKind::GcRef
                        | GcStorageKind::ExternRef
                        | GcStorageKind::OtherRef => {
                            a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, src_off));
                            a.mov_mr(true, dst, Gp::Rcx);
                        }
                        GcStorageKind::V128 => {
                            a.movdqu_xm(Xmm::Xmm15, Mem::base_disp(Gp::Rsp, src_off));
                            a.movdqu_mx(dst, Xmm::Xmm15);
                        }
                    }
                }
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
        }
        Ok(())
    }
}

/// Emits GC constant-expression initializers inside `wt_initialize` (runtime GC globals,
/// runtime GC table initializers, and active/passive GC element segments).
///
/// Called with `rsp % 16 == 8` (directly inside `wt_initialize` before `start`).
/// On allocation failure, jumps to `lbl_trap_alloc`; on active table OOB, jumps to `lbl_trap_table_oob`.
pub fn emit_gc_const_inits(
    a: &mut Asm,
    module: &X64Module,
    layout: &CtxLayout,
    lbl_trap_table_oob: Label,
    lbl_trap_alloc: Label,
    need_table_oob: &mut bool,
    need_alloc_trap: &mut bool,
) -> Result<(), String> {
    let Some(ref gc) = layout.gc else {
        return Ok(());
    };
    let has_gc_inits = !module.runtime_global_inits.is_empty()
        || !module.runtime_table_inits.is_empty()
        || module.elem_segs.iter().any(|s| s.gc_entries.is_some());
    if !has_gc_inits {
        return Ok(());
    }
    *need_alloc_trap = true;

    // Compute exact stack frame size needed for rooted intermediate values across all GC const exprs.
    let mut max_roots = 0usize;
    let mut max_scalar_bytes = 0usize;
    for (_, expr) in &module.runtime_global_inits {
        let (r, s) = const_expr_frame_need(expr, module);
        max_roots = max_roots.max(r);
        max_scalar_bytes = max_scalar_bytes.max(s);
    }
    for (_, expr) in &module.runtime_table_inits {
        let (r, s) = const_expr_frame_need(expr, module);
        max_roots = max_roots.max(r);
        max_scalar_bytes = max_scalar_bytes.max(s);
    }
    for seg in &module.elem_segs {
        if let Some(ref ge) = seg.gc_entries {
            for expr in ge {
                let (r, s) = const_expr_frame_need(expr, module);
                max_roots = max_roots.max(r);
                max_scalar_bytes = max_scalar_bytes.max(s);
            }
        }
    }
    let root_bytes = (max_roots * 8 + 15) & !15;
    let scalar_bytes = (max_scalar_bytes + 15) & !15;
    // In `StoreCdylib`, reserve 24 extra bytes at top of frame to save/restore `init_roots_ptr/len` & `walk_stop_rsp`
    // in case `init_instance` runs while another Wasm frame is active on the stack.
    let save_bytes = if layout.is_store { 32usize } else { 0usize };
    // Entry `rsp % 16 == 8`; add 8 bytes so `sub rsp, frame_bytes` achieves `rsp % 16 == 0`.
    let frame_bytes = root_bytes + scalar_bytes + save_bytes + 8;
    if frame_bytes > (2 << 20) {
        return Err(format!(
            "x64: GC const-expr init frame ({frame_bytes} B) exceeds 2 MiB cap"
        ));
    }
    let frame_i32 = frame_bytes as i32;
    let save_off = (root_bytes + scalar_bytes) as i32;
    let gc_off = gc.gc_state_off;

    let lbl_local_alloc_trap = a.new_label();
    let lbl_local_table_oob = a.new_label();
    let lbl_init_done = a.new_label();

    a.sub_ri(true, Gp::Rsp, frame_i32);
    // Point `gc_init_roots_ptr` at `[rsp + 0]` with `len = 0` and `gc_walk_stop_rsp = 0`.
    if layout.is_store {
        a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
        a.mov_rm(
            true,
            Gp::Rax,
            Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_PTR),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, save_off), Gp::Rax);
        a.mov_rm(
            true,
            Gp::Rax,
            Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_LEN),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, save_off + 8), Gp::Rax);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R11, GC_OFF_WALK_STOP_RSP));
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, save_off + 16), Gp::Rax);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_PTR),
            Gp::Rsp,
        );
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_LEN),
            Gp::Rax,
        );
        a.mov_mr(true, Mem::base_disp(Gp::R11, GC_OFF_WALK_STOP_RSP), Gp::Rax);
    } else {
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_PTR),
            Gp::Rsp,
        );
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_LEN),
            Gp::Rax,
        );
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_WALK_STOP_RSP),
            Gp::Rax,
        );
    }

    let ictx = InitExprCtx {
        module,
        layout,
        gc_off,
        scalar_base: root_bytes as i32,
        lbl_trap_alloc: lbl_local_alloc_trap,
    };

    // 1. Runtime GC globals in declaration order (defined globals only; imported globals are already initialized).
    for (gi, expr) in &module.runtime_global_inits {
        let is_v128 = module.globals[*gi].0 == ValType::V128;
        ictx.emit_eval(a, expr, is_v128, 0, 0)?;
        let goff = layout.global_offs[*gi];
        if is_v128 {
            a.movdqu_mx(Mem::base_disp(Gp::R15, goff), Xmm::Xmm15);
        } else {
            a.mov_mr(true, Mem::base_disp(Gp::R15, goff), Gp::Rax);
        }
    }

    // 2. Runtime GC table initializers (`table_inits` with GC const expr).
    for (tbl, expr) in &module.runtime_table_inits {
        ictx.emit_eval(a, expr, false, 0, 0)?;
        if let Some(&(desc_off, min_len, _)) = layout.dyn_tables.get(tbl) {
            if min_len > 0 {
                let lp = a.new_label();
                if layout.is_store {
                    a.mov_rm(
                        true,
                        Gp::Rdi,
                        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
                    );
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, (*tbl as i32) * 8));
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, 0));
                } else {
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, desc_off));
                }
                a.xor_rr(false, Gp::Rcx, Gp::Rcx);
                a.mov_ri32(Gp::Rdx, min_len);
                a.bind(lp);
                a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
                a.add_ri(true, Gp::Rcx, 1);
                a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
                a.jcc(Cond::B, lp);
            }
        } else if let Some(&(ctx_off, len)) = layout.extern_tables.get(tbl)
            && len > 0
        {
            let lp = a.new_label();
            a.lea(true, Gp::Rdi, Mem::base_disp(Gp::R15, ctx_off));
            a.xor_rr(false, Gp::Rcx, Gp::Rcx);
            a.mov_ri32(Gp::Rdx, len);
            a.bind(lp);
            a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
            a.add_ri(true, Gp::Rcx, 1);
            a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
            a.jcc(Cond::B, lp);
        }
    }

    // 3. Active & passive GC element segments (`gc_entries.is_some()`).
    let mut local_table_oob = false;
    for (ei, seg) in module.elem_segs.iter().enumerate() {
        let Some(ref ge) = seg.gc_entries else {
            continue;
        };
        if let Some((tbl, dst_off)) = seg.active_target {
            if layout.is_store {
                local_table_oob = true;
                *need_table_oob = true;
                // Runtime bounds check against `table_desc->cur_len` (imported tables may have grown).
                a.mov_rm(
                    true,
                    Gp::Rdi,
                    Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
                );
                a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, (tbl as i32) * 8));
                a.mov_rm(false, Gp::Rdx, Mem::base_disp(Gp::Rdi, 8)); // cur_len zero-ext to u64
                if let Some(end) = dst_off.checked_add(ge.len() as u64) {
                    a.mov_ri64(Gp::Rcx, end);
                    a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
                    a.jcc(Cond::A, lbl_local_table_oob);
                } else {
                    a.jmp(lbl_local_table_oob);
                    continue;
                }
            } else {
                let tbl_len = module.table_lens[tbl as usize];
                if dst_off
                    .checked_add(ge.len() as u64)
                    .is_none_or(|end| end > u64::from(tbl_len))
                {
                    local_table_oob = true;
                    *need_table_oob = true;
                    a.jmp(lbl_local_table_oob);
                    continue;
                }
            }
            for (i, expr) in ge.iter().enumerate() {
                ictx.emit_eval(a, expr, false, 0, 0)?;
                let slot_idx = dst_off + i as u64;
                if layout.is_store {
                    a.mov_rm(
                        true,
                        Gp::Rdi,
                        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
                    );
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, (tbl as i32) * 8));
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, 0));
                    a.mov_ri64(Gp::Rcx, slot_idx);
                    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
                } else if let Some(&(desc_off, _, _)) = layout.dyn_tables.get(&tbl) {
                    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, desc_off));
                    a.mov_ri64(Gp::Rcx, slot_idx);
                    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
                } else if let Some(&(ctx_off, _)) = layout.extern_tables.get(&tbl) {
                    let off = ctx_off + (slot_idx as i32) * 8;
                    a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
                }
            }
        } else if seg.is_passive {
            let buf_base_off = gc.passive_gc_elem_offs[ei];
            debug_assert!(buf_base_off >= 0);
            for (i, expr) in ge.iter().enumerate() {
                ictx.emit_eval(a, expr, false, 0, 0)?;
                if layout.is_store {
                    a.mov_rm(
                        true,
                        Gp::Rdi,
                        Mem::base_disp(Gp::R15, OFF_STORE_ELEM_BUF_PTR),
                    );
                } else {
                    a.mov_rm(
                        true,
                        Gp::Rdi,
                        Mem::base_disp(Gp::R15, gc_off + GC_OFF_ELEM_BUF_PTR),
                    );
                }
                let byte_off = (buf_base_off as u64) + (i as u64) * 8;
                a.mov_ri64(Gp::Rcx, byte_off);
                a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
            }
        }
    }

    let emit_teardown = |a: &mut Asm| {
        if layout.is_store {
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, save_off));
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_PTR),
                Gp::Rax,
            );
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, save_off + 8));
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R11, GC_OFF_INIT_ROOTS_LEN),
                Gp::Rax,
            );
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, save_off + 16));
            a.mov_mr(true, Mem::base_disp(Gp::R11, GC_OFF_WALK_STOP_RSP), Gp::Rax);
        } else {
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_PTR),
                Gp::Rax,
            );
            a.mov_mr(
                true,
                Mem::base_disp(Gp::R15, gc_off + GC_OFF_INIT_ROOTS_LEN),
                Gp::Rax,
            );
        }
        a.add_ri(true, Gp::Rsp, frame_i32);
    };

    // Cleanly tear down init-roots frame before continuing `wt_initialize`.
    emit_teardown(a);
    a.jmp(lbl_init_done);

    a.bind(lbl_local_alloc_trap);
    emit_teardown(a);
    a.jmp(lbl_trap_alloc);

    if local_table_oob {
        a.bind(lbl_local_table_oob);
        emit_teardown(a);
        a.jmp(lbl_trap_table_oob);
    }

    a.bind(lbl_init_done);
    Ok(())
}

/// Emits GC heap `mmap` initialization inside `blot_aot_new` and `emit_standalone_entry`.
/// Clobbers caller-saved GP regs; on `mmap` failure jumps to `lbl_fail_alloc`.
pub fn emit_gc_heap_init(a: &mut Asm, layout: &CtxLayout, lbl_fail_alloc: Label) {
    let Some(ref gc) = layout.gc else {
        return;
    };
    let gc_off = gc.gc_state_off;
    // Map initial `from_space` (demand-paged RW anonymous mmap).
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri64(Gp::Rsi, gc.init_heap_bytes);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_ALLOC_PTR),
        Gp::Rax,
    );
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_BASE),
        Gp::Rax,
    );
    a.mov_ri64(Gp::Rcx, gc.init_heap_bytes);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_FROM_CAP),
        Gp::Rcx,
    );
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_ALLOC_LIMIT),
        Gp::Rax,
    );
    a.mov_ri64(Gp::Rcx, gc.max_heap_bytes);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, gc_off + GC_OFF_HEAP_CAP),
        Gp::Rcx,
    );

    if gc.gc_elem_buf_bytes > 0 {
        a.xor_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_ri64(Gp::Rsi, gc.gc_elem_buf_bytes);
        a.mov_ri32(Gp::Rdx, PROT_RW);
        a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
        a.mov_ri64(Gp::R8, u64::MAX);
        a.xor_rr(false, Gp::R9, Gp::R9);
        a.mov_ri32(Gp::Rax, SYS_MMAP);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_ELEM_BUF_PTR),
            Gp::Rax,
        );
        a.mov_ri64(Gp::Rcx, gc.gc_elem_buf_bytes);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, gc_off + GC_OFF_ELEM_BUF_BYTES),
            Gp::Rcx,
        );
    }
}

/// Emits GC heap + element buffer `munmap` cleanup inside `emit_x64_cleanup_ctx`.
pub fn emit_gc_heap_cleanup(a: &mut Asm, ctx_reg: Gp, layout: &CtxLayout) {
    let Some(ref gc) = layout.gc else {
        return;
    };
    let gc_off = gc.gc_state_off;
    for (ptr_off, cap_off) in [
        (GC_OFF_FROM_BASE, GC_OFF_FROM_CAP),
        (GC_OFF_TO_BASE, GC_OFF_TO_CAP),
        (GC_OFF_ELEM_BUF_PTR, GC_OFF_ELEM_BUF_BYTES),
    ] {
        let lbl_skip = a.new_label();
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, gc_off + ptr_off));
        a.test_rr(true, Gp::Rdi, Gp::Rdi);
        a.jcc(Cond::E, lbl_skip);
        a.mov_rm(true, Gp::Rsi, Mem::base_disp(ctx_reg, gc_off + cap_off));
        a.test_rr(true, Gp::Rsi, Gp::Rsi);
        a.jcc(Cond::E, lbl_skip);
        a.mov_ri32(Gp::Rax, SYS_MUNMAP);
        a.syscall();
        a.bind(lbl_skip);
    }
}
