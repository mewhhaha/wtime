//! Multi-module `X64Target::StoreCdylib` AArch64 runtime helpers, host-import trampolines,
//! export-entry trampolines, instance initializer (with Wasm GC constant-expression evaluator),
//! single ELF descriptor (`wt_store_get_desc`), and post-link GC safepoint table builder.
//!
//! All code here is 100% safe Rust (`#![forbid(unsafe_code)]`) with zero external dependencies.

use wtime_types::ValType;

use super::asm::{A64Asm, A64RelocTarget, Cond, Label, Reg, VReg};
use super::lower::{
    CTX_REG, MEM_BASE_REG, MEM_LEN_REG, SFP1, SGP1, SGP2, SGP3, emit_load_off, emit_store_off,
};
use super::regalloc::{A64AbiSlot, layout_args, layout_rets};
use crate::x64::asm::SafepointRecord;
use crate::x64::gc_rt::{
    GC_OFF_ALLOC_LIMIT, GC_OFF_ALLOC_PTR, GC_OFF_FROM_BASE, GC_OFF_FROM_CAP, GC_OFF_INIT_ROOTS_LEN,
    GC_OFF_INIT_ROOTS_PTR, GC_OFF_STACK_CHAIN, GC_OFF_WALK_STOP_RSP,
};
use crate::x64::ir::Ty;
use crate::x64::module::{
    GC_EXT_I31_TAG_HI, GC_HDR_KIND_ARRAY, GC_HDR_KIND_INTERNAL_EXTERN, GC_HDR_KIND_STRUCT,
    GcCanonType, GcCompositeInfo, GcConstExpr, GcStorageKind, X64ConstVal, X64Module,
};
use crate::x64::runtime::RuntimeTables;
use crate::x64::store_rt::{STORE_ABI_VERSION, StoreRodataInfo};
use crate::x64::{
    CtxLayout, HELPER_ATOMIC_NOTIFY, HELPER_ATOMIC_WAIT, HELPER_EXTRA_MEM_COPY,
    HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW, HELPER_GC_ARRAY_COPY, HELPER_GC_ARRAY_FILL,
    HELPER_GC_ARRAY_FILL_RAW, HELPER_GC_ARRAY_INIT_DATA, HELPER_GC_ARRAY_INIT_ELEM,
    HELPER_GC_COLLECT, HELPER_INITIALIZE, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW,
    HELPER_MEM_INIT, HELPER_STORE_EXN_ALLOC, HELPER_TABLE_COPY, HELPER_TABLE_FILL,
    HELPER_TABLE_GROW, HELPER_TABLE_INIT, OFF_MEM_BASE, OFF_MEM_RESERVE_BYTES, OFF_STORE_CANON_MAP,
    OFF_STORE_ELEM_BUF_PTR, OFF_STORE_FUNC_REFS, OFF_STORE_MEM_DESCS, OFF_STORE_PTR,
    OFF_STORE_TABLE_DESCS, STORE_OFF_ATOMIC_NOTIFY_FN, STORE_OFF_ATOMIC_WAIT_FN,
    STORE_OFF_EXN_ALLOC_FN, STORE_OFF_GC_COLLECT_FN, STORE_OFF_HOST_CALL_FN, STORE_OFF_MEM_GROW_FN,
    STORE_OFF_RET_SPILL_PTR, STORE_OFF_TABLE_GROW_FN, TRAP_ALLOC_FAILED, TRAP_INDIRECT_CALL,
    TRAP_OOB, TRAP_TABLE_OOB,
};

const ALL_Q_REGS: [(VReg, VReg); 16] = [
    (VReg::V0, VReg::V1),
    (VReg::V2, VReg::V3),
    (VReg::V4, VReg::V5),
    (VReg::V6, VReg::V7),
    (VReg::V8, VReg::V9),
    (VReg::V10, VReg::V11),
    (VReg::V12, VReg::V13),
    (VReg::V14, VReg::V15),
    (VReg::V16, VReg::V17),
    (VReg::V18, VReg::V19),
    (VReg::V20, VReg::V21),
    (VReg::V22, VReg::V23),
    (VReg::V24, VReg::V25),
    (VReg::V26, VReg::V27),
    (VReg::V28, VReg::V29),
    (VReg::V30, VReg::V31),
];

const CALLEE_Q_REGS: [VReg; 8] = [
    VReg::V8,
    VReg::V9,
    VReg::V10,
    VReg::V11,
    VReg::V12,
    VReg::V13,
    VReg::V14,
    VReg::V15,
];

const CALLEE_GP_REGS: [Reg; 10] = [
    Reg::X19,
    Reg::X20,
    Reg::X21,
    Reg::X22,
    Reg::X23,
    Reg::X24,
    Reg::X25,
    Reg::X26,
    Reg::X27,
    Reg::X28,
];

fn emit_save_q0_31(a: &mut A64Asm, base_off: i32) {
    debug_assert!((base_off & 15) == 0 && base_off >= 0 && base_off + 480 <= 1008);
    for (i, &(v0, v1)) in ALL_Q_REGS.iter().enumerate() {
        a.stp_q(v0, v1, Reg::SP, base_off + (i as i32) * 32);
    }
}

fn emit_restore_q0_31(a: &mut A64Asm, base_off: i32) {
    debug_assert!((base_off & 15) == 0 && base_off >= 0 && base_off + 480 <= 1008);
    for (i, &(v0, v1)) in ALL_Q_REGS.iter().enumerate() {
        a.ldp_q(v0, v1, Reg::SP, base_off + (i as i32) * 32);
    }
}

fn emit_save_callee_q8_15(a: &mut A64Asm, base_off: i32) {
    for (i, &vr) in CALLEE_Q_REGS.iter().enumerate() {
        emit_store_off(
            a,
            Ty::V128,
            None,
            Some(vr),
            Reg::SP,
            base_off + (i as i32) * 16,
            SGP3,
        );
    }
}

fn emit_restore_callee_q8_15(a: &mut A64Asm, base_off: i32) {
    for (i, &vr) in CALLEE_Q_REGS.iter().enumerate() {
        emit_load_off(
            a,
            Ty::V128,
            None,
            Some(vr),
            Reg::SP,
            base_off + (i as i32) * 16,
            SGP3,
        );
    }
}

fn emit_save_fp_lr(a: &mut A64Asm, off: i32) {
    if (off & 7) == 0 && (-512..=504).contains(&off) {
        a.stp_x(Reg::FP, Reg::LR, Reg::SP, off);
    } else {
        emit_store_off(a, Ty::I64, Some(Reg::FP), None, Reg::SP, off, SGP3);
        emit_store_off(a, Ty::I64, Some(Reg::LR), None, Reg::SP, off + 8, SGP3);
    }
}

fn emit_restore_fp_lr(a: &mut A64Asm, off: i32) {
    if (off & 7) == 0 && (-512..=504).contains(&off) {
        a.ldp_x(Reg::FP, Reg::LR, Reg::SP, off);
    } else {
        emit_load_off(a, Ty::I64, Some(Reg::FP), None, Reg::SP, off, SGP3);
        emit_load_off(a, Ty::I64, Some(Reg::LR), None, Reg::SP, off + 8, SGP3);
    }
}

/// Computes exact byte size of the post-link sorted `SafepointHeader + entries + slots` table.
pub fn planned_a64_safepoint_table_bytes(def_asms: &[A64Asm]) -> Result<usize, String> {
    let mut n_entries: usize = 0;
    let mut n_slots: usize = 0;
    for asm in def_asms {
        n_entries = n_entries
            .checked_add(asm.safepoints.len())
            .ok_or("aarch64: safepoint count overflow")?;
        for sp in &asm.safepoints {
            n_slots = n_slots
                .checked_add(sp.gc_slots.len())
                .and_then(|v| v.checked_add(sp.ext_slots.len()))
                .ok_or("aarch64: safepoint slot count overflow")?;
        }
    }
    16usize
        .checked_add(
            n_entries
                .checked_mul(16)
                .ok_or("aarch64: safepoint table size overflow")?,
        )
        .and_then(|v| {
            v.checked_add(
                n_slots
                    .checked_mul(4)
                    .ok_or("aarch64: safepoint slots size overflow")
                    .ok()?,
            )
        })
        .ok_or_else(|| "aarch64: safepoint table size overflow".into())
}

/// Writes the post-link sorted `SafepointHeader + entries + slots` table into `rodata` at
/// `expected_off` (8-byte aligned), matching `wtime_aot_host::abi::{SafepointHeader, SafepointEntry}`.
pub fn emit_a64_safepoint_table_rodata(
    rodata: &mut [u8],
    expected_off: usize,
    def_asms: &[A64Asm],
    func_offs: &[usize],
    n_imports: usize,
    defs_start_off: usize,
) -> Result<(), String> {
    let mut flat: Vec<(u32, u32, &SafepointRecord)> = Vec::new();
    for (bi, asm) in def_asms.iter().enumerate() {
        let fn_off = func_offs[n_imports + bi] as u64;
        for sp in &asm.safepoints {
            let abs_off = fn_off
                .checked_add(sp.code_off as u64)
                .ok_or("aarch64: safepoint code offset overflow")?;
            let rel = abs_off
                .checked_sub(defs_start_off as u64)
                .ok_or("aarch64: safepoint precedes GuestCodeStart")?;
            let rel_u32 = u32::try_from(rel)
                .map_err(|_| "aarch64: safepoint GuestCodeStart-relative offset exceeds u32")?;
            flat.push((rel_u32, sp.frame_caller_ret_off, sp));
        }
    }
    flat.sort_unstable_by_key(|&(pc_rel, _, _)| pc_rel);
    if flat.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("aarch64: duplicate safepoint return address detected".into());
    }

    let num_entries =
        u32::try_from(flat.len()).map_err(|_| "aarch64: safepoint count exceeds u32")?;
    let entries_rel_off: i32 = 16;
    let slots_rel_off = i32::try_from(16usize + flat.len() * 16)
        .map_err(|_| "aarch64: safepoint slots offset exceeds i32")?;

    let mut pos = expected_off;
    let mut write_u32 = |buf: &mut [u8], v: u32| {
        buf[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
        pos += 4;
    };

    // Header (16 bytes): num_entries, pad, entries_rel_off, slots_rel_off.
    write_u32(rodata, num_entries);
    write_u32(rodata, 0);
    write_u32(rodata, entries_rel_off as u32);
    write_u32(rodata, slots_rel_off as u32);

    // Entries (16 bytes each): ret_pc_rel, frame_caller_ret_off, first_slot_idx, num_slots.
    let mut slot_cursor: u32 = 0;
    for &(pc_rel, frame_ret_off, sp) in &flat {
        let n_sp_slots = u32::try_from(sp.gc_slots.len() + sp.ext_slots.len())
            .map_err(|_| "aarch64: safepoint slot count exceeds u32")?;
        write_u32(rodata, pc_rel);
        write_u32(rodata, frame_ret_off);
        write_u32(rodata, slot_cursor);
        write_u32(rodata, n_sp_slots);
        slot_cursor = slot_cursor
            .checked_add(n_sp_slots)
            .ok_or("aarch64: safepoint total slot index overflow")?;
    }

    // Slots (`[i32]`): `gc_slots` followed by `ext_slots` for each entry.
    for &(_, _, sp) in &flat {
        for &off in &sp.gc_slots {
            write_u32(rodata, off as u32);
        }
        for &off in &sp.ext_slots {
            write_u32(rodata, off as u32);
        }
    }
    Ok(())
}

/// Emits the single ELF export `wt_store_get_desc(out: *mut StoreModuleDesc [X0])`.
///
/// Exact `StoreModuleDesc` layout (96 bytes, verified by `wtime-aot-host/src/abi.rs`):
///   `+0`:  abi_version: u32
///   `+4`:  flags: u32 (bit0=uses_gc, bit1=uses_exceptions, bit2=uses_threads)
///   `+8`:  ctx_total_bytes: u64
///   `+16`: code_start: *const u8
///   `+24`: code_end: *const u8
///   `+32`: safepoint_header: *const u8 (0 if !uses_gc)
///   `+40`: rodata_base: *const u8
///   `+48`: metadata_ptr: *const u8
///   `+56`: metadata_len: u64
///   `+64`: init_instance_fn: *const u8
///   `+72`: def_fn_table: *const i32
///   `+80`: host_tramp_table: *const i32
///   `+88`: export_tramp_table: *const i32
pub fn emit_a64_store_get_desc(
    module: &X64Module,
    layout: &CtxLayout,
    store_ro: &StoreRodataInfo,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let flags = u32::from(module.uses_gc)
        | (u32::from(module.uses_exceptions) << 1)
        | (u32::from(module.uses_threads) << 2);
    a.mov_imm32(Reg::X1, STORE_ABI_VERSION);
    a.str_uimm(false, Reg::X1, Reg::X0, 0);
    a.mov_imm32(Reg::X1, flags);
    a.str_uimm(false, Reg::X1, Reg::X0, 4);
    a.mov_imm64(Reg::X1, layout.total_bytes as u64);
    a.str_uimm(true, Reg::X1, Reg::X0, 8);

    a.adr_reloc(Reg::X1, A64RelocTarget::GuestCodeStart);
    a.str_uimm(true, Reg::X1, Reg::X0, 16);
    a.adr_reloc(Reg::X1, A64RelocTarget::GuestCodeEnd);
    a.str_uimm(true, Reg::X1, Reg::X0, 24);
    if module.uses_gc {
        a.adr_reloc(Reg::X1, A64RelocTarget::GcSafepointHeader);
    } else {
        a.movz(true, Reg::X1, 0, 0);
    }
    a.str_uimm(true, Reg::X1, Reg::X0, 32);
    a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(0));
    a.str_uimm(true, Reg::X1, Reg::X0, 40);
    a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(store_ro.meta_ro_off));
    a.str_uimm(true, Reg::X1, Reg::X0, 48);
    a.mov_imm64(Reg::X1, store_ro.meta_len);
    a.str_uimm(true, Reg::X1, Reg::X0, 56);
    a.adr_reloc(Reg::X1, A64RelocTarget::Helper(HELPER_INITIALIZE));
    a.str_uimm(true, Reg::X1, Reg::X0, 64);
    a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(store_ro.def_fn_rel_ro_off));
    a.str_uimm(true, Reg::X1, Reg::X0, 72);
    a.adr_reloc(
        Reg::X1,
        A64RelocTarget::Rodata(store_ro.host_tramp_rel_ro_off),
    );
    a.str_uimm(true, Reg::X1, Reg::X0, 80);
    a.adr_reloc(
        Reg::X1,
        A64RelocTarget::Rodata(store_ro.export_tramp_rel_ro_off),
    );
    a.str_uimm(true, Reg::X1, Reg::X0, 88);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Emits all `StoreCdylib` AArch64 runtime helpers (`(helper_id, A64Asm)` pairs).
pub fn emit_a64_store_helpers(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Vec<(u32, A64Asm)>, String> {
    let mut out = Vec::with_capacity(24);
    out.push((HELPER_MEM_GROW, emit_a64_store_mem_grow(module.memory64)?));
    out.push((HELPER_MEM_COPY, emit_a64_store_mem_copy(module.memory64)?));
    out.push((HELPER_MEM_FILL, emit_a64_store_mem_fill(module.memory64)?));
    out.push((
        HELPER_INITIALIZE,
        emit_a64_store_init_instance(module, layout, tables)?,
    ));
    out.push((HELPER_EXTRA_MEM_GROW, emit_a64_store_extra_mem_grow()?));
    out.push((HELPER_EXTRA_MEM_COPY, emit_a64_store_extra_mem_copy()?));
    out.push((HELPER_EXTRA_MEM_FILL, emit_a64_store_extra_mem_fill()?));
    if module.uses_bulk_data {
        out.push((HELPER_MEM_INIT, emit_a64_store_mem_init(layout, tables)?));
    }
    if module.uses_bulk_elem {
        out.push((
            HELPER_TABLE_INIT,
            emit_a64_store_table_init(layout, tables)?,
        ));
    }
    out.push((HELPER_TABLE_COPY, emit_a64_store_table_copy()?));
    out.push((HELPER_TABLE_GROW, emit_a64_store_table_grow()?));
    out.push((HELPER_TABLE_FILL, emit_a64_store_table_fill()?));

    if module.uses_gc {
        out.push((HELPER_GC_COLLECT, emit_a64_store_gc_collect_stub()?));
        out.push((HELPER_GC_ARRAY_FILL_RAW, emit_a64_gc_array_fill_raw()?));
        out.push((HELPER_GC_ARRAY_FILL, emit_a64_gc_array_fill()?));
        out.push((HELPER_GC_ARRAY_COPY, emit_a64_gc_array_copy()?));
        if let Some(drop_off) = layout.data_dropped_off {
            out.push((
                HELPER_GC_ARRAY_INIT_DATA,
                emit_a64_gc_array_init_data(tables.passive_data_table_off, drop_off)?,
            ));
        }
        if layout.elem_dropped_off.is_some()
            && layout
                .gc
                .as_ref()
                .is_some_and(|g| g.gc_elem_desc_ro_off != 0)
        {
            out.push((
                HELPER_GC_ARRAY_INIT_ELEM,
                emit_a64_store_gc_array_init_elem(layout)?,
            ));
        }
    }

    if module.uses_threads {
        out.push((HELPER_ATOMIC_WAIT, emit_a64_store_atomic_wait()?));
        out.push((HELPER_ATOMIC_NOTIFY, emit_a64_store_atomic_notify()?));
    }

    if module.uses_exceptions {
        out.push((HELPER_STORE_EXN_ALLOC, emit_a64_store_exn_alloc_stub()?));
    }

    Ok(out)
}

/// Preserve-all wrapper for `HELPER_GC_COLLECT`:
/// - Entry: `X17 (SGP2) = req_bytes`, `X28 (CTX_REG) = Ctx*`, `LR = caller Wasm return PC`.
/// - Saves `(FP, LR)` at `[SP_caller - 16]` so `[SP_caller - 8]` holds `LR` (exact match for
///   `wtime-aot-host/src/gc.rs::walk_stack_segment`), saves `X0..X15, X17, X18` and full 128-bit `Q0..Q31`.
/// - Calls `store->gc_collect_fn(store_ptr [X0], req_bytes [X1], gc_call_ret_rsp [X2]) -> u32 [W0]`.
/// - Returns trap code in `W16 (SGP1)`; preserves ALL other GP registers and ALL `Q0..Q31`.
fn emit_a64_store_gc_collect_stub() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    // Place (FP, LR) at [SP_caller - 16 .. SP_caller] so LR sits at [SP_caller - 8].
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.mov_r(true, Reg::FP, Reg::SP);
    // Reserve 656B (16B aligned): [SP+0..144] = X0..X15, X17, X18; [SP+144..656] = Q0..Q31.
    a.sub_imm(true, Reg::SP, Reg::SP, 656);
    a.stp_x(Reg::X0, Reg::X1, Reg::SP, 0);
    a.stp_x(Reg::X2, Reg::X3, Reg::SP, 16);
    a.stp_x(Reg::X4, Reg::X5, Reg::SP, 32);
    a.stp_x(Reg::X6, Reg::X7, Reg::SP, 48);
    a.stp_x(Reg::X8, Reg::X9, Reg::SP, 64);
    a.stp_x(Reg::X10, Reg::X11, Reg::SP, 80);
    a.stp_x(Reg::X12, Reg::X13, Reg::SP, 96);
    a.stp_x(Reg::X14, Reg::X15, Reg::SP, 112);
    a.stp_x(Reg::X17, Reg::X18, Reg::SP, 128);
    emit_save_q0_31(&mut a, 144);

    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.mov_r(true, Reg::X1, SGP2);
    a.add_imm(true, Reg::X2, Reg::SP, 664); // SP + 656 + 8 == SP_caller - 8 (LR slot!)
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_GC_COLLECT_FN as u32);
    a.blr(Reg::X9);
    a.mov_r(false, SGP1, Reg::X0);

    emit_restore_q0_31(&mut a, 144);
    a.ldp_x(Reg::X0, Reg::X1, Reg::SP, 0);
    a.ldp_x(Reg::X2, Reg::X3, Reg::SP, 16);
    a.ldp_x(Reg::X4, Reg::X5, Reg::SP, 32);
    a.ldp_x(Reg::X6, Reg::X7, Reg::SP, 48);
    a.ldp_x(Reg::X8, Reg::X9, Reg::SP, 64);
    a.ldp_x(Reg::X10, Reg::X11, Reg::SP, 80);
    a.ldp_x(Reg::X12, Reg::X13, Reg::SP, 96);
    a.ldp_x(Reg::X14, Reg::X15, Reg::SP, 112);
    a.ldp_x(Reg::X17, Reg::X18, Reg::SP, 128);
    a.add_imm(true, Reg::SP, Reg::SP, 656);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Preserve-all wrapper for `HELPER_STORE_EXN_ALLOC`:
/// - Entry: `W16 (SGP1) = global_tag_id`, `W17 (SGP2) = rec_size`, `X28 (CTX_REG) = Ctx*`.
/// - Calls `store->exn_alloc_fn(store_ptr [X0], global_tag_id [W1], rec_size [W2]) -> *mut u8 [X0]`.
/// - Returns `X17 (SGP2) = exn_ptr`, `W16 (SGP1) = 0 | TRAP_ALLOC_FAILED`; preserves all other GPs and `Q0..Q31`.
fn emit_a64_store_exn_alloc_stub() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_ok = a.new_label();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.mov_r(true, Reg::FP, Reg::SP);
    a.sub_imm(true, Reg::SP, Reg::SP, 656);
    a.stp_x(Reg::X0, Reg::X1, Reg::SP, 0);
    a.stp_x(Reg::X2, Reg::X3, Reg::SP, 16);
    a.stp_x(Reg::X4, Reg::X5, Reg::SP, 32);
    a.stp_x(Reg::X6, Reg::X7, Reg::SP, 48);
    a.stp_x(Reg::X8, Reg::X9, Reg::SP, 64);
    a.stp_x(Reg::X10, Reg::X11, Reg::SP, 80);
    a.stp_x(Reg::X12, Reg::X13, Reg::SP, 96);
    a.stp_x(Reg::X14, Reg::X15, Reg::SP, 112);
    a.str_uimm(true, Reg::X18, Reg::SP, 128);
    emit_save_q0_31(&mut a, 144);

    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.uxtw(Reg::X1, SGP1);
    a.uxtw(Reg::X2, SGP2);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_EXN_ALLOC_FN as u32);
    a.blr(Reg::X9);
    a.mov_r(true, SGP2, Reg::X0);
    a.movz(false, SGP1, 0, 0);
    a.cbnz(true, SGP2, lbl_ok);
    a.mov_imm32(SGP1, u32::from(TRAP_ALLOC_FAILED));
    a.bind(lbl_ok);

    emit_restore_q0_31(&mut a, 144);
    a.ldp_x(Reg::X0, Reg::X1, Reg::SP, 0);
    a.ldp_x(Reg::X2, Reg::X3, Reg::SP, 16);
    a.ldp_x(Reg::X4, Reg::X5, Reg::SP, 32);
    a.ldp_x(Reg::X6, Reg::X7, Reg::SP, 48);
    a.ldp_x(Reg::X8, Reg::X9, Reg::SP, 64);
    a.ldp_x(Reg::X10, Reg::X11, Reg::SP, 80);
    a.ldp_x(Reg::X12, Reg::X13, Reg::SP, 96);
    a.ldp_x(Reg::X14, Reg::X15, Reg::SP, 112);
    a.ldr_uimm(true, Reg::X18, Reg::SP, 128);
    a.add_imm(true, Reg::SP, Reg::SP, 656);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_MEM_GROW`: `X0 = delta_pages`, `X28 = Ctx*` -> `X0 = old_pages | -1`, reloads `MEM_LEN_REG`.
fn emit_a64_store_mem_grow(memory64: bool) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.sub_imm(true, Reg::SP, Reg::SP, 512);
    emit_save_q0_31(&mut a, 0);

    if memory64 {
        a.mov_r(true, Reg::X2, Reg::X0);
    } else {
        a.uxtw(Reg::X2, Reg::X0);
    }
    a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_MEM_GROW_FN as u32);
    a.blr(Reg::X9);

    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, Reg::X9, 8);
    if !memory64 {
        a.uxtw(Reg::X0, Reg::X0);
    }

    emit_restore_q0_31(&mut a, 0);
    a.add_imm(true, Reg::SP, Reg::SP, 512);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_GROW`: `X0 = delta_pages`, `W1 = mem_idx`, `X28 = Ctx*` -> `X0 = old_pages | -1`.
fn emit_a64_store_extra_mem_grow() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_not0 = a.new_label();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.sub_imm(true, Reg::SP, Reg::SP, 528); // [SP+0..16] = saved mem_idx; [SP+16..528] = Q0..Q31.
    emit_save_q0_31(&mut a, 16);

    a.uxtw(Reg::X1, Reg::X1);
    a.str_uimm(true, Reg::X1, Reg::SP, 0);
    a.mov_r(true, Reg::X2, Reg::X0);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_MEM_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X1, Reg::X9, Reg::X1, false);
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_MEM_GROW_FN as u32);
    a.blr(Reg::X9);

    a.ldr_uimm(true, Reg::X9, Reg::SP, 0);
    a.cbnz(false, Reg::X9, lbl_not0);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, Reg::X9, 8);
    a.bind(lbl_not0);

    emit_restore_q0_31(&mut a, 16);
    a.add_imm(true, Reg::SP, Reg::SP, 528);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Emits an overlap-safe byte copy from `src` (`X1`) to `dst` (`X0`) of length `len` (`X2` > 0),
/// using 64-bit chunks with 1-byte tails. Clobbers `X0..X4`.
fn emit_a64_memmove_x0_x1_x2(a: &mut A64Asm, lbl_done: Label) {
    let lbl_bwd = a.new_label();
    let lbl_fwd8 = a.new_label();
    let lbl_fwd1_chk = a.new_label();
    let lbl_fwd1 = a.new_label();
    let lbl_bwd8 = a.new_label();
    let lbl_bwd1 = a.new_label();

    a.cmp_r(true, Reg::X0, Reg::X1);
    a.b_cond(Cond::Eq, lbl_done);
    a.b_cond(Cond::Hi, lbl_bwd);

    // Forward copy
    a.cmp_imm(true, Reg::X2, 8);
    a.b_cond(Cond::Lo, lbl_fwd1_chk);
    a.bind(lbl_fwd8);
    a.ldr_uimm(true, Reg::X4, Reg::X1, 0);
    a.str_uimm(true, Reg::X4, Reg::X0, 0);
    a.add_imm(true, Reg::X1, Reg::X1, 8);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.sub_imm(true, Reg::X2, Reg::X2, 8);
    a.cmp_imm(true, Reg::X2, 8);
    a.b_cond(Cond::Hs, lbl_fwd8);
    a.bind(lbl_fwd1_chk);
    a.cbz(true, Reg::X2, lbl_done);
    a.bind(lbl_fwd1);
    a.ldrb_uimm(Reg::X4, Reg::X1, 0);
    a.strb_uimm(Reg::X4, Reg::X0, 0);
    a.add_imm(true, Reg::X1, Reg::X1, 1);
    a.add_imm(true, Reg::X0, Reg::X0, 1);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_fwd1);
    a.b(lbl_done);

    // Backward copy
    a.bind(lbl_bwd);
    a.bind(lbl_bwd8);
    a.cmp_imm(true, Reg::X2, 8);
    a.b_cond(Cond::Lo, lbl_bwd1);
    a.sub_imm(true, Reg::X2, Reg::X2, 8);
    a.ldr_reg(true, Reg::X4, Reg::X1, Reg::X2, false);
    a.str_reg(true, Reg::X4, Reg::X0, Reg::X2, false);
    a.b(lbl_bwd8);
    a.bind(lbl_bwd1);
    a.cbz(true, Reg::X2, lbl_done);
    a.sub_imm(true, Reg::X2, Reg::X2, 1);
    a.ldrb_reg(Reg::X4, Reg::X1, Reg::X2, false);
    a.strb_reg(Reg::X4, Reg::X0, Reg::X2, false);
    a.b(lbl_bwd1);
}

/// `HELPER_MEM_COPY`: `X0 = dst, X1 = src, X2 = len` on memory 0 -> `W16 = 0 | TRAP_OOB`.
fn emit_a64_store_mem_copy(memory64: bool) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_done = a.new_label();
    if !memory64 {
        a.uxtw(Reg::X0, Reg::X0);
        a.uxtw(Reg::X1, Reg::X1);
        a.uxtw(Reg::X2, Reg::X2);
    }
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, Reg::X9, 8);
    a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X3, MEM_LEN_REG);
    a.b_cond(Cond::Hi, lbl_oob);
    a.adds_r(true, Reg::X3, Reg::X1, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X3, MEM_LEN_REG);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
    a.add_r(true, Reg::X1, MEM_BASE_REG, Reg::X1);
    emit_a64_memmove_x0_x1_x2(&mut a, lbl_done);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_MEM_FILL`: `X0 = dst, W1 = val, X2 = len` on memory 0 -> `W16 = 0 | TRAP_OOB`.
fn emit_a64_store_mem_fill(memory64: bool) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_lp = a.new_label();
    let lbl_done = a.new_label();
    if !memory64 {
        a.uxtw(Reg::X0, Reg::X0);
        a.uxtw(Reg::X2, Reg::X2);
    }
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, Reg::X9, 8);
    a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X3, MEM_LEN_REG);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
    a.bind(lbl_lp);
    a.strb_uimm(Reg::X1, Reg::X0, 0);
    a.add_imm(true, Reg::X0, Reg::X0, 1);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_lp);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_COPY`: `X0 = dst, X1 = src, X2 = len, W3 = dst_mem_idx, W4 = src_mem_idx`.
fn emit_a64_store_extra_mem_copy() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_done = a.new_label();
    a.uxtw(Reg::X3, Reg::X3);
    a.uxtw(Reg::X4, Reg::X4);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_MEM_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X3, false); // dst_desc
    a.ldr_reg_scaled(true, Reg::X11, Reg::X9, Reg::X4, false); // src_desc
    a.ldr_uimm(true, Reg::X12, Reg::X10, 8); // dst_len
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X12);
    a.b_cond(Cond::Hi, lbl_oob);
    a.ldr_uimm(true, Reg::X12, Reg::X11, 8); // src_len
    a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X12);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.ldr_uimm(true, Reg::X12, Reg::X10, 0);
    a.add_r(true, Reg::X0, Reg::X12, Reg::X0);
    a.ldr_uimm(true, Reg::X12, Reg::X11, 0);
    a.add_r(true, Reg::X1, Reg::X12, Reg::X1);
    emit_a64_memmove_x0_x1_x2(&mut a, lbl_done);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_FILL`: `X0 = dst, W1 = val, X2 = len, W3 = mem_idx`.
fn emit_a64_store_extra_mem_fill() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_lp = a.new_label();
    let lbl_done = a.new_label();
    a.uxtw(Reg::X3, Reg::X3);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_MEM_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X3, false); // mem_desc
    a.ldr_uimm(true, Reg::X11, Reg::X10, 8);
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.ldr_uimm(true, Reg::X11, Reg::X10, 0);
    a.add_r(true, Reg::X0, Reg::X11, Reg::X0);
    a.bind(lbl_lp);
    a.strb_uimm(Reg::X1, Reg::X0, 0);
    a.add_imm(true, Reg::X0, Reg::X0, 1);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_lp);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_MEM_INIT`: `X0 = dst, X1 = src, X2 = len, W3 = data_idx, W4 = mem_idx`.
fn emit_a64_store_mem_init(layout: &CtxLayout, tables: &RuntimeTables) -> Result<A64Asm, String> {
    let drop_off = layout
        .data_dropped_off
        .ok_or("aarch64: StoreCdylib HELPER_MEM_INIT missing data_dropped_off")?;
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_done = a.new_label();

    a.uxtw(Reg::X3, Reg::X3); // data_idx
    a.uxtw(Reg::X4, Reg::X4); // mem_idx
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_MEM_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X4, false); // mem_desc
    // Check dst + len <= mem_desc->len
    a.ldr_uimm(true, Reg::X11, Reg::X10, 8);
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    // Check dropped flag & seg_len
    a.add_i32_checked(true, Reg::X11, CTX_REG, drop_off, SGP3);
    a.ldrb_reg(Reg::X11, Reg::X11, Reg::X3, false);
    a.cbz(false, Reg::X11, lbl_not_dropped);
    a.movz(true, Reg::X11, 0, 0);
    a.b(lbl_cmp_src);
    a.bind(lbl_not_dropped);
    a.adr_reloc(
        Reg::X12,
        A64RelocTarget::Rodata(tables.passive_data_table_off),
    );
    a.lsl_imm(true, Reg::X13, Reg::X3, 3);
    a.add_r(true, Reg::X13, Reg::X12, Reg::X13);
    a.ldr_uimm(false, Reg::X11, Reg::X13, 4); // seg_len
    a.bind(lbl_cmp_src);
    a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    // dst_ptr = mem_desc->base + dst; src_ptr = seg_base + src
    a.ldr_uimm(true, Reg::X11, Reg::X10, 0);
    a.add_r(true, Reg::X0, Reg::X11, Reg::X0);
    a.ldrsw_uimm(Reg::X5, Reg::X13, 0);
    a.add_r(true, Reg::X12, Reg::X12, Reg::X5);
    a.add_r(true, Reg::X1, Reg::X12, Reg::X1);
    emit_a64_memmove_x0_x1_x2(&mut a, lbl_done);

    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_TABLE_GROW`: `X0 = init_val, X1 = delta, W2 = tbl_idx` -> `X0 = old_len | -1`.
fn emit_a64_store_table_grow() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.sub_imm(true, Reg::SP, Reg::SP, 512);
    emit_save_q0_31(&mut a, 0);

    // C ABI: table_grow_fn(store_ptr [X0], tbl_desc [X1], init_val [X2], delta [X3])
    a.mov_r(true, Reg::X3, Reg::X1); // delta
    a.uxtw(Reg::X9, Reg::X2); // tbl_idx
    a.mov_r(true, Reg::X2, Reg::X0); // init_val
    a.ldr_uimm(true, Reg::X10, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X1, Reg::X10, Reg::X9, false); // tbl_desc
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_TABLE_GROW_FN as u32);
    a.blr(Reg::X9);

    emit_restore_q0_31(&mut a, 0);
    a.add_imm(true, Reg::SP, Reg::SP, 512);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_TABLE_FILL`: `X0 = dst, X1 = val, X2 = len, W3 = tbl_idx` -> `W16 = 0 | TRAP_TABLE_OOB`.
fn emit_a64_store_table_fill() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_lp = a.new_label();
    let lbl_done = a.new_label();
    a.uxtw(Reg::X3, Reg::X3);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X3, false);
    a.ldr_uimm(false, Reg::X11, Reg::X10, 8); // cur_len zero-ext u64
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.ldr_uimm(true, Reg::X11, Reg::X10, 0);
    a.add_r_lsl(true, Reg::X0, Reg::X11, Reg::X0, 3);
    a.bind(lbl_lp);
    a.str_uimm(true, Reg::X1, Reg::X0, 0);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_lp);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_TABLE_COPY`: `X0 = dst, X1 = src, X2 = len, W3 = dst_tbl, W4 = src_tbl` -> `W16`.
fn emit_a64_store_table_copy() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_fwd_lp = a.new_label();
    let lbl_bwd_lp = a.new_label();
    let lbl_done = a.new_label();
    a.uxtw(Reg::X3, Reg::X3);
    a.uxtw(Reg::X4, Reg::X4);
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X3, false); // dst_desc
    a.ldr_reg_scaled(true, Reg::X11, Reg::X9, Reg::X4, false); // src_desc
    a.ldr_uimm(false, Reg::X12, Reg::X10, 8); // dst_len
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X12);
    a.b_cond(Cond::Hi, lbl_oob);
    a.ldr_uimm(false, Reg::X12, Reg::X11, 8); // src_len
    a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X12);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);
    a.ldr_uimm(true, Reg::X12, Reg::X10, 0);
    a.add_r_lsl(true, Reg::X0, Reg::X12, Reg::X0, 3);
    a.ldr_uimm(true, Reg::X12, Reg::X11, 0);
    a.add_r_lsl(true, Reg::X1, Reg::X12, Reg::X1, 3);
    a.cmp_r(true, Reg::X0, Reg::X1);
    a.b_cond(Cond::Eq, lbl_done);
    a.b_cond(Cond::Hi, lbl_bwd);
    a.bind(lbl_fwd_lp);
    a.ldr_uimm(true, Reg::X5, Reg::X1, 0);
    a.str_uimm(true, Reg::X5, Reg::X0, 0);
    a.add_imm(true, Reg::X1, Reg::X1, 8);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_fwd_lp);
    a.b(lbl_done);
    a.bind(lbl_bwd);
    a.bind(lbl_bwd_lp);
    a.sub_imm(true, Reg::X2, Reg::X2, 1);
    a.ldr_reg_scaled(true, Reg::X5, Reg::X1, Reg::X2, false);
    a.str_reg_scaled(true, Reg::X5, Reg::X0, Reg::X2, false);
    a.cbnz(true, Reg::X2, lbl_bwd_lp);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_TABLE_INIT`: `X0 = dst, X1 = src, X2 = len, W3 = elem_idx, W4 = tbl_idx` -> `W16`.
/// Translates static `u32` funcref entries (`enc -> StoreFuncRef*`) and copies passive GC elem entries directly.
fn emit_a64_store_table_init(layout: &CtxLayout, tables: &RuntimeTables) -> Result<A64Asm, String> {
    let drop_off = layout
        .elem_dropped_off
        .ok_or("aarch64: StoreCdylib HELPER_TABLE_INIT missing elem_dropped_off")?;
    let mut a = A64Asm::new();
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_done = a.new_label();
    let lbl_gc_copy = a.new_label();
    let lbl_gc_lp = a.new_label();
    let lbl_fn_lp = a.new_label();
    let lbl_fn_store = a.new_label();

    a.uxtw(Reg::X3, Reg::X3); // elem_idx
    a.uxtw(Reg::X4, Reg::X4); // tbl_idx
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
    a.ldr_reg_scaled(true, Reg::X10, Reg::X9, Reg::X4, false); // tbl_desc
    // Check dst + len <= tbl_desc->cur_len
    a.ldr_uimm(false, Reg::X11, Reg::X10, 8);
    a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    // Check dropped flag
    a.add_i32_checked(true, Reg::X11, CTX_REG, drop_off, SGP3);
    a.ldrb_reg(Reg::X11, Reg::X11, Reg::X3, false);
    a.cbz(false, Reg::X11, lbl_not_dropped);
    a.movz(true, Reg::X11, 0, 0);
    a.b(lbl_cmp_src);

    let has_gc_elem_desc = layout
        .gc
        .as_ref()
        .is_some_and(|g| g.gc_elem_desc_ro_off != 0);

    a.bind(lbl_not_dropped);
    if let Some(ref gc) = layout.gc
        && gc.gc_elem_desc_ro_off != 0
    {
        // `GcElemDesc[elem_idx]` (16B): +0 ro_rel, +4 elem_len, +8 gc_buf_off (i32), +12 is_traced.
        a.adr_reloc(Reg::X12, A64RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
        a.lsl_imm(true, Reg::X13, Reg::X3, 4);
        a.add_r(true, Reg::X13, Reg::X12, Reg::X13);
        a.ldr_uimm(false, Reg::X11, Reg::X13, 4); // elem_len
    } else {
        a.adr_reloc(Reg::X12, A64RelocTarget::Rodata(tables.elem_seg_table_off));
        a.lsl_imm(true, Reg::X13, Reg::X3, 3);
        a.add_r(true, Reg::X13, Reg::X12, Reg::X13);
        a.ldr_uimm(false, Reg::X11, Reg::X13, 4);
    }

    a.bind(lbl_cmp_src);
    a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
    a.b_cond(Cond::Hs, lbl_oob);
    a.cmp_r(true, Reg::X5, Reg::X11);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X2, lbl_done);

    // Dst pointer = tbl_desc->ptr + dst * 8
    a.ldr_uimm(true, Reg::X10, Reg::X10, 0);
    a.add_r_lsl(true, Reg::X0, Reg::X10, Reg::X0, 3);

    if has_gc_elem_desc {
        a.ldrsw_uimm(Reg::X5, Reg::X13, 8); // gc_buf_off
        a.cmp_imm(true, Reg::X5, 0);
        a.b_cond(Cond::Ge, lbl_gc_copy);
    }
    // Static u32 funcref segment in `.rodata`
    a.ldrsw_uimm(Reg::X5, Reg::X13, 0);
    a.add_r(true, Reg::X12, Reg::X12, Reg::X5);
    a.add_r_lsl(true, Reg::X1, Reg::X12, Reg::X1, 2); // src u32 ptr
    a.ldr_uimm(true, Reg::X14, CTX_REG, OFF_STORE_FUNC_REFS as u32);
    a.bind(lbl_fn_lp);
    a.ldr_uimm(false, Reg::X5, Reg::X1, 0);
    a.cbz(false, Reg::X5, lbl_fn_store);
    a.sub_imm(true, Reg::X5, Reg::X5, 1);
    a.ldr_reg_scaled(true, Reg::X5, Reg::X14, Reg::X5, false);
    a.bind(lbl_fn_store);
    a.str_uimm(true, Reg::X5, Reg::X0, 0);
    a.add_imm(true, Reg::X1, Reg::X1, 4);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.subs_imm(true, Reg::X2, Reg::X2, 1);
    a.b_cond(Cond::Ne, lbl_fn_lp);
    a.b(lbl_done);

    if has_gc_elem_desc {
        a.bind(lbl_gc_copy);
        a.ldr_uimm(true, Reg::X14, CTX_REG, OFF_STORE_ELEM_BUF_PTR as u32);
        a.add_r(true, Reg::X14, Reg::X14, Reg::X5);
        a.add_r_lsl(true, Reg::X1, Reg::X14, Reg::X1, 3);
        a.bind(lbl_gc_lp);
        a.ldr_uimm(true, Reg::X5, Reg::X1, 0);
        a.str_uimm(true, Reg::X5, Reg::X0, 0);
        a.add_imm(true, Reg::X1, Reg::X1, 8);
        a.add_imm(true, Reg::X0, Reg::X0, 8);
        a.subs_imm(true, Reg::X2, Reg::X2, 1);
        a.b_cond(Cond::Ne, lbl_gc_lp);
    }

    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_FILL_RAW`:
/// Inputs: `X0 = arr_ptr`, `W1 = len`, `X2 = fill_u64` (or `SFP1 = V30` if `elem_sz == 16`), `W3 = elem_sz`.
/// Returns `X0 = arr_ptr`.
fn emit_a64_gc_array_fill_raw() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_done = a.new_label();
    let lbl_sz2 = a.new_label();
    let lbl_sz4 = a.new_label();
    let lbl_sz8 = a.new_label();
    let lbl_sz16 = a.new_label();

    a.mov_r(true, Reg::X8, Reg::X0); // preserve arr_ptr
    a.uxtw(Reg::X1, Reg::X1);
    a.cbz(true, Reg::X1, lbl_done);
    a.add_imm(true, Reg::X9, Reg::X0, 8);

    a.cmp_imm(false, Reg::X3, 8);
    a.b_cond(Cond::Eq, lbl_sz8);
    a.b_cond(Cond::Hi, lbl_sz16);
    a.cmp_imm(false, Reg::X3, 4);
    a.b_cond(Cond::Eq, lbl_sz4);
    a.cmp_imm(false, Reg::X3, 2);
    a.b_cond(Cond::Eq, lbl_sz2);

    // elem_sz == 1
    let lp1 = a.new_label();
    a.bind(lp1);
    a.strb_uimm(Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 1);
    a.subs_imm(true, Reg::X1, Reg::X1, 1);
    a.b_cond(Cond::Ne, lp1);
    a.b(lbl_done);

    // elem_sz == 2
    a.bind(lbl_sz2);
    let lp2 = a.new_label();
    a.bind(lp2);
    a.strh_uimm(Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 2);
    a.subs_imm(true, Reg::X1, Reg::X1, 1);
    a.b_cond(Cond::Ne, lp2);
    a.b(lbl_done);

    // elem_sz == 4
    a.bind(lbl_sz4);
    let lp4 = a.new_label();
    a.bind(lp4);
    a.str_uimm(false, Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 4);
    a.subs_imm(true, Reg::X1, Reg::X1, 1);
    a.b_cond(Cond::Ne, lp4);
    a.b(lbl_done);

    // elem_sz == 8
    a.bind(lbl_sz8);
    let lp8 = a.new_label();
    a.bind(lp8);
    a.str_uimm(true, Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 8);
    a.subs_imm(true, Reg::X1, Reg::X1, 1);
    a.b_cond(Cond::Ne, lp8);
    a.b(lbl_done);

    // elem_sz == 16 (`SFP1 = V30`)
    a.bind(lbl_sz16);
    let lp16 = a.new_label();
    a.bind(lp16);
    a.str_q_uimm(SFP1, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 16);
    a.subs_imm(true, Reg::X1, Reg::X1, 1);
    a.b_cond(Cond::Ne, lp16);

    a.bind(lbl_done);
    a.mov_r(true, Reg::X0, Reg::X8);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_FILL`:
/// Inputs: `X0 = arr_ptr`, `W1 = dst_off`, `X2 = val` (or `SFP1 = V30` if `elem_sz == 16`),
/// `W3 = len`, `W4 = elem_sz`. Returns `W16 = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
fn emit_a64_gc_array_fill() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_sz2 = a.new_label();
    let lbl_sz4 = a.new_label();
    let lbl_sz8 = a.new_label();
    let lbl_sz16 = a.new_label();

    a.cbz(true, Reg::X0, lbl_null);
    a.uxtw(Reg::X1, Reg::X1); // dst_off
    a.uxtw(Reg::X3, Reg::X3); // len
    a.uxtw(Reg::X4, Reg::X4); // elem_sz
    a.ldr_uimm(false, Reg::X5, Reg::X0, 4); // arr.len
    a.add_r(true, Reg::X6, Reg::X1, Reg::X3);
    a.cmp_r(true, Reg::X6, Reg::X5);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X3, lbl_ok);

    a.mul(true, Reg::X5, Reg::X1, Reg::X4);
    a.add_imm(true, Reg::X9, Reg::X0, 8);
    a.add_r(true, Reg::X9, Reg::X9, Reg::X5);

    a.cmp_imm(false, Reg::X4, 8);
    a.b_cond(Cond::Eq, lbl_sz8);
    a.b_cond(Cond::Hi, lbl_sz16);
    a.cmp_imm(false, Reg::X4, 4);
    a.b_cond(Cond::Eq, lbl_sz4);
    a.cmp_imm(false, Reg::X4, 2);
    a.b_cond(Cond::Eq, lbl_sz2);

    // elem_sz == 1
    let lp1 = a.new_label();
    a.bind(lp1);
    a.strb_uimm(Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 1);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lp1);
    a.b(lbl_ok);

    // elem_sz == 2
    a.bind(lbl_sz2);
    let lp2 = a.new_label();
    a.bind(lp2);
    a.strh_uimm(Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 2);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lp2);
    a.b(lbl_ok);

    // elem_sz == 4
    a.bind(lbl_sz4);
    let lp4 = a.new_label();
    a.bind(lp4);
    a.str_uimm(false, Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 4);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lp4);
    a.b(lbl_ok);

    // elem_sz == 8
    a.bind(lbl_sz8);
    let lp8 = a.new_label();
    a.bind(lp8);
    a.str_uimm(true, Reg::X2, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 8);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lp8);
    a.b(lbl_ok);

    // elem_sz == 16
    a.bind(lbl_sz16);
    let lp16 = a.new_label();
    a.bind(lp16);
    a.str_q_uimm(SFP1, Reg::X9, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 16);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lp16);

    a.bind(lbl_ok);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_null);
    a.mov_imm32(SGP1, u32::from(TRAP_INDIRECT_CALL));
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_COPY`:
/// Inputs: `X0 = dst_arr`, `W1 = dst_off`, `X2 = src_arr`, `W3 = src_off`, `W4 = len`, `W5 = elem_sz`.
/// Returns `W16 = 0 | TRAP_OOB | TRAP_INDIRECT_CALL`.
fn emit_a64_gc_array_copy() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();

    a.cbz(true, Reg::X0, lbl_null);
    a.cbz(true, Reg::X2, lbl_null);
    a.uxtw(Reg::X1, Reg::X1);
    a.uxtw(Reg::X3, Reg::X3);
    a.uxtw(Reg::X4, Reg::X4);
    a.uxtw(Reg::X5, Reg::X5);

    a.ldr_uimm(false, Reg::X6, Reg::X0, 4);
    a.add_r(true, Reg::X7, Reg::X1, Reg::X4);
    a.cmp_r(true, Reg::X7, Reg::X6);
    a.b_cond(Cond::Hi, lbl_oob);

    a.ldr_uimm(false, Reg::X6, Reg::X2, 4);
    a.add_r(true, Reg::X7, Reg::X3, Reg::X4);
    a.cmp_r(true, Reg::X7, Reg::X6);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X4, lbl_ok);

    // Compute dst_ptr -> X0, src_ptr -> X1, byte_len -> X2
    a.mul(true, Reg::X6, Reg::X1, Reg::X5);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.add_r(true, Reg::X0, Reg::X0, Reg::X6);
    a.mul(true, Reg::X6, Reg::X3, Reg::X5);
    a.add_imm(true, Reg::X1, Reg::X2, 8);
    a.add_r(true, Reg::X1, Reg::X1, Reg::X6);
    a.mul(true, Reg::X2, Reg::X4, Reg::X5);

    emit_a64_memmove_x0_x1_x2(&mut a, lbl_ok);

    a.bind(lbl_ok);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_null);
    a.mov_imm32(SGP1, u32::from(TRAP_INDIRECT_CALL));
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_INIT_DATA`:
/// Inputs: `X0 = arr_ptr`, `W1 = dst_off`, `W2 = src_byte_off`, `W3 = len`, `X4 = (data_idx<<32)|elem_sz`.
fn emit_a64_gc_array_init_data(
    passive_data_table_off: u32,
    data_dropped_off: i32,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_not_dropped = a.new_label();

    a.cbz(true, Reg::X0, lbl_null);
    a.uxtw(Reg::X1, Reg::X1); // dst_off
    a.uxtw(Reg::X2, Reg::X2); // src_byte_off
    a.uxtw(Reg::X3, Reg::X3); // elem_len
    a.lsr_imm(true, Reg::X5, Reg::X4, 32); // data_idx
    a.uxtw(Reg::X4, Reg::X4); // elem_sz

    // Check dst_off + elem_len <= arr.len
    a.ldr_uimm(false, Reg::X6, Reg::X0, 4);
    a.add_r(true, Reg::X7, Reg::X1, Reg::X3);
    a.cmp_r(true, Reg::X7, Reg::X6);
    a.b_cond(Cond::Hi, lbl_oob);

    // Load data seg byte len (0 if dropped)
    a.adr_reloc(Reg::X9, A64RelocTarget::Rodata(passive_data_table_off));
    a.lsl_imm(true, Reg::X10, Reg::X5, 3);
    a.add_r(true, Reg::X10, Reg::X9, Reg::X10);
    a.ldr_uimm(false, Reg::X6, Reg::X10, 4); // seg_len
    a.add_i32_checked(true, Reg::X7, CTX_REG, data_dropped_off, SGP3);
    a.ldrb_reg(Reg::X7, Reg::X7, Reg::X5, false);
    a.cbz(false, Reg::X7, lbl_not_dropped);
    a.movz(true, Reg::X6, 0, 0);
    a.bind(lbl_not_dropped);

    // byte_len = elem_len * elem_sz; check src_byte_off + byte_len <= seg_len
    a.mul(true, Reg::X3, Reg::X3, Reg::X4);
    a.add_r(true, Reg::X7, Reg::X2, Reg::X3);
    a.cmp_r(true, Reg::X7, Reg::X6);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X3, lbl_ok);

    // dst_ptr -> X0, src_ptr -> X1, byte_len -> X2
    a.mul(true, Reg::X1, Reg::X1, Reg::X4);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.add_r(true, Reg::X0, Reg::X0, Reg::X1);
    a.ldrsw_uimm(Reg::X7, Reg::X10, 0);
    a.add_r(true, Reg::X9, Reg::X9, Reg::X7);
    a.add_r(true, Reg::X1, Reg::X9, Reg::X2);
    a.mov_r(true, Reg::X2, Reg::X3);
    emit_a64_memmove_x0_x1_x2(&mut a, lbl_ok);

    a.bind(lbl_ok);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_null);
    a.mov_imm32(SGP1, u32::from(TRAP_INDIRECT_CALL));
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_INIT_ELEM` for `StoreCdylib`:
/// Inputs: `X0 = arr`, `W1 = dst`, `W2 = src`, `W3 = len`, `W4 = elem_idx`.
fn emit_a64_store_gc_array_init_elem(layout: &CtxLayout) -> Result<A64Asm, String> {
    let gc = layout
        .gc
        .as_ref()
        .ok_or("aarch64: HELPER_GC_ARRAY_INIT_ELEM missing GcCtxLayout")?;
    let drop_off = layout
        .elem_dropped_off
        .ok_or("aarch64: HELPER_GC_ARRAY_INIT_ELEM missing elem_dropped_off")?;
    let mut a = A64Asm::new();
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_gc_copy = a.new_label();
    let lbl_gc_lp = a.new_label();
    let lbl_fn_lp = a.new_label();
    let lbl_fn_store = a.new_label();
    let lbl_done = a.new_label();

    a.cbz(true, Reg::X0, lbl_null);
    a.uxtw(Reg::X1, Reg::X1);
    a.uxtw(Reg::X2, Reg::X2);
    a.uxtw(Reg::X3, Reg::X3);
    a.uxtw(Reg::X4, Reg::X4);

    // Check dst + len <= arr.len
    a.ldr_uimm(false, Reg::X5, Reg::X0, 4);
    a.add_r(true, Reg::X6, Reg::X1, Reg::X3);
    a.cmp_r(true, Reg::X6, Reg::X5);
    a.b_cond(Cond::Hi, lbl_oob);

    // Check dropped flag & seg_len
    a.add_i32_checked(true, Reg::X7, CTX_REG, drop_off, SGP3);
    a.ldrb_reg(Reg::X7, Reg::X7, Reg::X4, false);
    a.cbz(false, Reg::X7, lbl_not_dropped);
    a.movz(true, Reg::X6, 0, 0);
    a.b(lbl_cmp_src);
    a.bind(lbl_not_dropped);
    a.adr_reloc(Reg::X9, A64RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
    a.lsl_imm(true, Reg::X10, Reg::X4, 4);
    a.add_r(true, Reg::X10, Reg::X9, Reg::X10);
    a.ldr_uimm(false, Reg::X6, Reg::X10, 4);
    a.bind(lbl_cmp_src);
    a.add_r(true, Reg::X7, Reg::X2, Reg::X3);
    a.cmp_r(true, Reg::X7, Reg::X6);
    a.b_cond(Cond::Hi, lbl_oob);
    a.cbz(true, Reg::X3, lbl_done);

    // Dst ptr = arr + 8 + dst * 8
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.add_r_lsl(true, Reg::X0, Reg::X0, Reg::X1, 3);
    a.ldrsw_uimm(Reg::X5, Reg::X10, 8); // gc_buf_off
    a.cmp_imm(true, Reg::X5, 0);
    a.b_cond(Cond::Ge, lbl_gc_copy);

    // Static u32 funcref segment -> translate via OFF_STORE_FUNC_REFS
    a.ldrsw_uimm(Reg::X5, Reg::X10, 0);
    a.add_r(true, Reg::X9, Reg::X9, Reg::X5);
    a.add_r_lsl(true, Reg::X9, Reg::X9, Reg::X2, 2);
    a.ldr_uimm(true, Reg::X11, CTX_REG, OFF_STORE_FUNC_REFS as u32);
    a.bind(lbl_fn_lp);
    a.ldr_uimm(false, Reg::X5, Reg::X9, 0);
    a.cbz(false, Reg::X5, lbl_fn_store);
    a.sub_imm(true, Reg::X5, Reg::X5, 1);
    a.ldr_reg_scaled(true, Reg::X5, Reg::X11, Reg::X5, false);
    a.bind(lbl_fn_store);
    a.str_uimm(true, Reg::X5, Reg::X0, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 4);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lbl_fn_lp);
    a.b(lbl_done);

    a.bind(lbl_gc_copy);
    a.ldr_uimm(true, Reg::X11, CTX_REG, OFF_STORE_ELEM_BUF_PTR as u32);
    a.add_r(true, Reg::X11, Reg::X11, Reg::X5);
    a.add_r_lsl(true, Reg::X9, Reg::X11, Reg::X2, 3);
    a.bind(lbl_gc_lp);
    a.ldr_uimm(true, Reg::X5, Reg::X9, 0);
    a.str_uimm(true, Reg::X5, Reg::X0, 0);
    a.add_imm(true, Reg::X9, Reg::X9, 8);
    a.add_imm(true, Reg::X0, Reg::X0, 8);
    a.subs_imm(true, Reg::X3, Reg::X3, 1);
    a.b_cond(Cond::Ne, lbl_gc_lp);

    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.ret();
    a.bind(lbl_null);
    a.mov_imm32(SGP1, u32::from(TRAP_INDIRECT_CALL));
    a.ret();
    a.bind(lbl_oob);
    a.mov_imm32(SGP1, u32::from(TRAP_OOB));
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_ATOMIC_WAIT`: `X0 = host_ptr, X1 = expected, X2 = timeout_ns, W3 = is_64` -> `W0`.
fn emit_a64_store_atomic_wait() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.sub_imm(true, Reg::SP, Reg::SP, 512);
    emit_save_q0_31(&mut a, 0);

    // C ABI: atomic_wait_fn(store [X0], host_ptr [X1], expected [X2], timeout [X3], is_64 [W4])
    a.uxtw(Reg::X4, Reg::X3);
    a.mov_r(true, Reg::X3, Reg::X2);
    a.mov_r(true, Reg::X2, Reg::X1);
    a.mov_r(true, Reg::X1, Reg::X0);
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_ATOMIC_WAIT_FN as u32);
    a.blr(Reg::X9);

    emit_restore_q0_31(&mut a, 0);
    a.add_imm(true, Reg::SP, Reg::SP, 512);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// `HELPER_ATOMIC_NOTIFY`: `X0 = host_ptr, W1 = count` -> `W0`.
fn emit_a64_store_atomic_notify() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.sub_imm(true, Reg::SP, Reg::SP, 512);
    emit_save_q0_31(&mut a, 0);

    // C ABI: atomic_notify_fn(store [X0], host_ptr [X1], count [W2])
    a.uxtw(Reg::X2, Reg::X1);
    a.mov_r(true, Reg::X1, Reg::X0);
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, Reg::X9, Reg::X0, STORE_OFF_ATOMIC_NOTIFY_FN as u32);
    a.blr(Reg::X9);

    emit_restore_q0_31(&mut a, 0);
    a.add_imm(true, Reg::SP, Reg::SP, 512);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

// =============================================================================
// AArch64 GC Constant-Expression Evaluator (for `wt_store_init_instance`)
// =============================================================================

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

struct InitExprCtxA64<'a> {
    module: &'a X64Module,
    layout: &'a CtxLayout,
    scalar_base: i32,
    lbl_trap_alloc: Label,
}

impl<'a> InitExprCtxA64<'a> {
    fn emit_set_init_roots_len(&self, a: &mut A64Asm, n: usize) {
        a.mov_imm64(SGP2, n as u64);
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        a.str_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_LEN as u32);
    }

    /// Writes the 32-bit (or zero-extended 64-bit for structs) GC header word to `[obj_reg + 0]`.
    /// Clobbers `SGP1`, `SGP2`, `SGP3`.
    fn emit_write_header(
        &self,
        a: &mut A64Asm,
        obj_reg: Reg,
        gt: &GcCanonType,
        is_struct_64: bool,
    ) {
        let kind_bits = match gt.composite {
            GcCompositeInfo::Struct { .. } => GC_HDR_KIND_STRUCT,
            GcCompositeInfo::Array { .. } => GC_HDR_KIND_ARRAY,
            GcCompositeInfo::Func => unreachable!(),
        };
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_CANON_MAP as u32);
        emit_load_off(
            a,
            Ty::I32,
            Some(SGP2),
            None,
            SGP1,
            (gt.canon_id as i32) * 4,
            SGP3,
        );
        a.lsl_imm(false, SGP2, SGP2, 3);
        if kind_bits != 0 {
            a.orr_imm_2_w(SGP2, SGP2);
        }
        a.str_uimm(is_struct_64, SGP2, obj_reg, 0);
    }

    /// Allocates constant `size_bytes` from `StoreState`'s GC heap into `X0`.
    /// Clobbers `X0, X1, SGP1, SGP2, SGP3`. Preserves `X2`.
    fn emit_alloc_fixed(&self, a: &mut A64Asm, size_bytes: u32) {
        let lbl_ok = a.new_label();
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        a.ldr_uimm(true, Reg::X0, SGP1, GC_OFF_ALLOC_PTR as u32);
        a.add_i32_checked(true, Reg::X1, Reg::X0, size_bytes as i32, SGP3);
        a.ldr_uimm(true, SGP3, SGP1, GC_OFF_ALLOC_LIMIT as u32);
        a.cmp_r(true, Reg::X1, SGP3);
        a.b_cond(Cond::Ls, lbl_ok);
        a.mov_imm64(SGP2, u64::from(size_bytes));
        a.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
        a.cbnz(false, SGP1, self.lbl_trap_alloc);
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        a.ldr_uimm(true, Reg::X0, SGP1, GC_OFF_ALLOC_PTR as u32);
        a.add_i32_checked(true, Reg::X1, Reg::X0, size_bytes as i32, SGP3);
        a.bind(lbl_ok);
        a.str_uimm(true, Reg::X1, SGP1, GC_OFF_ALLOC_PTR as u32);
    }

    /// Allocates dynamic `SGP2 (X17)` bytes from `StoreState`'s GC heap into `X0`.
    /// Preserves `X1` (`len`) and `SGP2`; clobbers `X0, X2, SGP1, SGP3`.
    fn emit_alloc_dyn_sgp2_into_x0(&self, a: &mut A64Asm) {
        let lbl_have = a.new_label();
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        a.ldr_uimm(true, Reg::X0, SGP1, GC_OFF_ALLOC_PTR as u32);
        a.adds_r(true, Reg::X2, Reg::X0, SGP2);
        a.b_cond(Cond::Hs, self.lbl_trap_alloc);
        a.ldr_uimm(true, SGP3, SGP1, GC_OFF_ALLOC_LIMIT as u32);
        a.cmp_r(true, Reg::X2, SGP3);
        a.b_cond(Cond::Ls, lbl_have);
        a.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
        a.cbnz(false, SGP1, self.lbl_trap_alloc);
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        a.ldr_uimm(true, Reg::X0, SGP1, GC_OFF_ALLOC_PTR as u32);
        a.add_r(true, Reg::X2, Reg::X0, SGP2);
        a.bind(lbl_have);
        a.str_uimm(true, Reg::X2, SGP1, GC_OFF_ALLOC_PTR as u32);
    }

    fn emit_store_field_from_stack(
        &self,
        a: &mut A64Asm,
        obj_reg: Reg,
        field_off: i32,
        st: GcStorageKind,
        stack_off: i32,
    ) {
        match st {
            GcStorageKind::I8 => {
                emit_load_off(a, Ty::I32, Some(SGP1), None, Reg::SP, stack_off, SGP3);
                a.add_i32_checked(true, SGP3, obj_reg, field_off, SGP3);
                a.strb_uimm(SGP1, SGP3, 0);
            }
            GcStorageKind::I16 => {
                emit_load_off(a, Ty::I32, Some(SGP1), None, Reg::SP, stack_off, SGP3);
                a.add_i32_checked(true, SGP3, obj_reg, field_off, SGP3);
                a.strh_uimm(SGP1, SGP3, 0);
            }
            GcStorageKind::I32 | GcStorageKind::F32 => {
                emit_load_off(a, Ty::I32, Some(SGP1), None, Reg::SP, stack_off, SGP3);
                emit_store_off(a, Ty::I32, Some(SGP1), None, obj_reg, field_off, SGP3);
            }
            GcStorageKind::I64
            | GcStorageKind::F64
            | GcStorageKind::GcRef
            | GcStorageKind::ExternRef
            | GcStorageKind::OtherRef => {
                emit_load_off(a, Ty::I64, Some(SGP1), None, Reg::SP, stack_off, SGP3);
                emit_store_off(a, Ty::I64, Some(SGP1), None, obj_reg, field_off, SGP3);
            }
            GcStorageKind::V128 => {
                emit_load_off(a, Ty::V128, None, Some(SFP1), Reg::SP, stack_off, SGP3);
                emit_store_off(a, Ty::V128, None, Some(SFP1), obj_reg, field_off, SGP3);
            }
        }
    }

    /// Evaluates `expr` into `X0` (or `SFP1 = V30` when `expect_v128`).
    /// Active traced root slots live at `[SP + 0 .. r_cur * 8]`.
    fn emit_eval(
        &self,
        a: &mut A64Asm,
        expr: &GcConstExpr,
        expect_v128: bool,
        r_cur: usize,
        s_cur: usize,
    ) -> Result<(), String> {
        match expr {
            GcConstExpr::Scalar(c) => match *c {
                X64ConstVal::I32(v) => a.mov_imm32(Reg::X0, v as u32),
                X64ConstVal::I64(v) => a.mov_imm64(Reg::X0, v as u64),
                X64ConstVal::F32(bits) => a.mov_imm32(Reg::X0, bits),
                X64ConstVal::F64(bits) => a.mov_imm64(Reg::X0, bits),
                X64ConstVal::RefNull => a.movz(true, Reg::X0, 0, 0),
                X64ConstVal::FuncRef(f) => {
                    a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_FUNC_REFS as u32);
                    emit_load_off(a, Ty::I64, Some(Reg::X0), None, SGP1, (f as i32) * 8, SGP3);
                }
                X64ConstVal::I31(enc) => a.mov_imm64(Reg::X0, u64::from(enc)),
                X64ConstVal::ExternI31(enc) => {
                    a.mov_imm64(Reg::X0, GC_EXT_I31_TAG_HI | u64::from(enc));
                }
                X64ConstVal::V128(bytes) => {
                    if bytes == [0u8; 16] {
                        a.movi_zero_16b(SFP1);
                    } else {
                        let off = a.add_local_rodata(16, &bytes);
                        a.adr_reloc(SGP1, A64RelocTarget::LocalRodata(off));
                        a.ldr_q_uimm(SFP1, SGP1, 0);
                    }
                }
                X64ConstVal::RuntimeGc => {
                    return Err("aarch64: unexpected RuntimeGc placeholder in GcConstExpr".into());
                }
            },
            GcConstExpr::GlobalGet(g) => {
                let gi = *g as usize;
                if let Some(c) = self.module.global_consts[gi] {
                    self.emit_eval(a, &GcConstExpr::Scalar(c), expect_v128, r_cur, s_cur)?;
                } else {
                    let off = self.layout.global_offs[gi];
                    let is_v128 = expect_v128 || self.module.globals[gi].0 == ValType::V128;
                    if *g < self.layout.imported_global_count {
                        emit_load_off(a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
                        if is_v128 {
                            a.ldr_q_uimm(SFP1, SGP1, 0);
                        } else {
                            a.ldr_uimm(true, Reg::X0, SGP1, 0);
                        }
                    } else if is_v128 {
                        emit_load_off(a, Ty::V128, None, Some(SFP1), CTX_REG, off, SGP3);
                    } else {
                        emit_load_off(a, Ty::I64, Some(Reg::X0), None, CTX_REG, off, SGP3);
                    }
                }
            }
            GcConstExpr::RefI31(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                a.lsl_imm(false, Reg::X0, Reg::X0, 1);
                a.orr_imm_low_bits(false, Reg::X0, Reg::X0, 1);
            }
            GcConstExpr::ExternConvertAny(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                let lbl_done = a.new_label();
                let lbl_i31 = a.new_label();
                a.cbz(true, Reg::X0, lbl_done);
                a.tst_imm_low_bits(false, Reg::X0, 1);
                a.b_cond(Cond::Ne, lbl_i31);
                a.ldr_uimm(false, SGP1, Reg::X0, 0);
                a.and_imm_6_w(SGP1, SGP1);
                a.cmp_imm(false, SGP1, GC_HDR_KIND_INTERNAL_EXTERN as u16);
                a.b_cond(Cond::Ne, lbl_done);
                a.ldr_uimm(true, Reg::X0, Reg::X0, 8);
                a.b(lbl_done);
                a.bind(lbl_i31);
                a.mov_imm64(SGP1, GC_EXT_I31_TAG_HI);
                a.orr_r(true, Reg::X0, Reg::X0, SGP1);
                a.bind(lbl_done);
            }
            GcConstExpr::AnyConvertExtern(inner) => {
                self.emit_eval(a, inner, false, r_cur, s_cur)?;
                let lbl_done = a.new_label();
                let lbl_untag_i31 = a.new_label();
                let lbl_box = a.new_label();
                a.cbz(true, Reg::X0, lbl_done);
                a.lsr_imm(true, SGP1, Reg::X0, 32);
                a.mov_imm32(SGP2, (GC_EXT_I31_TAG_HI >> 32) as u32);
                a.cmp_r(false, SGP1, SGP2);
                a.b_cond(Cond::Eq, lbl_untag_i31);
                a.tst_imm_low_bits(true, Reg::X0, 3);
                a.b_cond(Cond::Ne, lbl_box);
                a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
                a.ldr_uimm(true, SGP2, SGP1, GC_OFF_FROM_BASE as u32);
                a.cmp_r(true, Reg::X0, SGP2);
                a.b_cond(Cond::Lo, lbl_box);
                a.ldr_uimm(true, SGP3, SGP1, GC_OFF_FROM_CAP as u32);
                a.add_r(true, SGP2, SGP2, SGP3);
                a.cmp_r(true, Reg::X0, SGP2);
                a.b_cond(Cond::Lo, lbl_done);

                a.bind(lbl_box);
                a.mov_r(true, Reg::X2, Reg::X0); // preserved across emit_alloc_fixed + HELPER_GC_COLLECT
                self.emit_alloc_fixed(a, 16);
                a.mov_imm64(SGP1, u64::from(GC_HDR_KIND_INTERNAL_EXTERN));
                a.str_uimm(true, SGP1, Reg::X0, 0);
                a.str_uimm(true, Reg::X2, Reg::X0, 8);
                a.b(lbl_done);

                a.bind(lbl_untag_i31);
                a.uxtw(Reg::X0, Reg::X0);
                a.bind(lbl_done);
            }
            GcConstExpr::I32Bin(op, lhs, rhs) => {
                self.emit_eval(a, lhs, false, r_cur, s_cur)?;
                let slot = self.scalar_base + s_cur as i32;
                emit_store_off(a, Ty::I32, Some(Reg::X0), None, Reg::SP, slot, SGP3);
                self.emit_eval(a, rhs, false, r_cur, s_cur + 8)?;
                a.mov_r(false, SGP1, Reg::X0);
                emit_load_off(a, Ty::I32, Some(Reg::X0), None, Reg::SP, slot, SGP3);
                match *op {
                    0x6a => a.add_r(false, Reg::X0, Reg::X0, SGP1),
                    0x6b => a.sub_r(false, Reg::X0, Reg::X0, SGP1),
                    0x6c => a.mul(false, Reg::X0, Reg::X0, SGP1),
                    other => return Err(format!("aarch64: unsupported i32 const op 0x{other:x}")),
                }
            }
            GcConstExpr::I64Bin(op, lhs, rhs) => {
                self.emit_eval(a, lhs, false, r_cur, s_cur)?;
                let slot = self.scalar_base + s_cur as i32;
                emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, slot, SGP3);
                self.emit_eval(a, rhs, false, r_cur, s_cur + 8)?;
                a.mov_r(true, SGP1, Reg::X0);
                emit_load_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, slot, SGP3);
                match *op {
                    0x7c => a.add_r(true, Reg::X0, Reg::X0, SGP1),
                    0x7d => a.sub_r(true, Reg::X0, Reg::X0, SGP1),
                    0x7e => a.mul(true, Reg::X0, Reg::X0, SGP1),
                    other => return Err(format!("aarch64: unsupported i64 const op 0x{other:x}")),
                }
            }
            GcConstExpr::StructNewDefault(ty) => {
                let gt = self.module.gc_type(*ty);
                let size = match gt.composite {
                    GcCompositeInfo::Struct { total_bytes, .. } => total_bytes,
                    _ => return Err("aarch64: StructNewDefault on non-struct".into()),
                };
                self.emit_alloc_fixed(a, size);
                self.emit_write_header(a, Reg::X0, gt, true);
                let zero_qwords = (size - 8) / 8;
                if zero_qwords > 0 && zero_qwords <= 8 {
                    for q in 0..zero_qwords {
                        a.str_uimm(true, Reg::XZR, Reg::X0, 8 + q * 8);
                    }
                } else if zero_qwords > 8 {
                    let lp = a.new_label();
                    a.add_imm(true, Reg::X1, Reg::X0, 8);
                    a.mov_imm32(Reg::X2, zero_qwords);
                    a.bind(lp);
                    a.str_uimm(true, Reg::XZR, Reg::X1, 0);
                    a.add_imm(true, Reg::X1, Reg::X1, 8);
                    a.subs_imm(true, Reg::X2, Reg::X2, 1);
                    a.b_cond(Cond::Ne, lp);
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
                    _ => return Err("aarch64: StructNew on non-struct".into()),
                };
                let mut r_next = r_cur;
                let mut s_next = s_cur;
                let mut saved_locs: Vec<(i32, GcStorageKind)> = Vec::with_capacity(fields.len());
                for (fe, fd) in fields.iter().zip(field_defs.iter()) {
                    let is_v128 = fd.storage == GcStorageKind::V128;
                    self.emit_eval(a, fe, is_v128, r_next, s_next)?;
                    if X64Module::storage_ref_kind(fd.storage).is_traced() {
                        let off = (r_next as i32) * 8;
                        emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                        r_next += 1;
                        self.emit_set_init_roots_len(a, r_next);
                        saved_locs.push((off, fd.storage));
                    } else if is_v128 {
                        s_next = (s_next + 15) & !15;
                        let off = self.scalar_base + s_next as i32;
                        emit_store_off(a, Ty::V128, None, Some(SFP1), Reg::SP, off, SGP3);
                        s_next += 16;
                        saved_locs.push((off, fd.storage));
                    } else {
                        let off = self.scalar_base + s_next as i32;
                        emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                        s_next += 8;
                        saved_locs.push((off, fd.storage));
                    }
                }
                self.emit_alloc_fixed(a, size);
                self.emit_write_header(a, Reg::X0, gt, true);
                for (&(src_off, st), fd) in saved_locs.iter().zip(field_defs.iter()) {
                    self.emit_store_field_from_stack(a, Reg::X0, fd.offset as i32, st, src_off);
                }
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
            GcConstExpr::ArrayNewDefault { ty, len } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNewDefault on non-array".into()),
                };
                let elem_sz = elem_storage.byte_size();
                self.emit_eval(a, len, false, r_cur, s_cur)?;
                a.uxtw(Reg::X1, Reg::X0); // len
                a.mov_imm64(SGP2, u64::from(elem_sz));
                a.mul(true, SGP2, SGP2, Reg::X1);
                a.add_imm(true, SGP2, SGP2, 15);
                a.and_imm_align8(SGP2, SGP2);
                self.emit_alloc_dyn_sgp2_into_x0(a);
                self.emit_write_header(a, Reg::X0, gt, false);
                a.str_uimm(false, Reg::X1, Reg::X0, 4);
                if elem_sz == 16 {
                    a.movi_zero_16b(SFP1);
                } else {
                    a.movz(true, Reg::X2, 0, 0);
                }
                a.mov_imm32(Reg::X3, elem_sz);
                a.bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
            }
            GcConstExpr::ArrayNew { ty, init, len } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNew on non-array".into()),
                };
                let elem_sz = elem_storage.byte_size();
                let is_traced = X64Module::storage_ref_kind(elem_storage).is_traced();
                let is_v128 = elem_storage == GcStorageKind::V128;
                self.emit_eval(a, init, is_v128, r_cur, s_cur)?;
                let (init_slot, r_next, s_next) = if is_traced {
                    let off = (r_cur as i32) * 8;
                    emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                    self.emit_set_init_roots_len(a, r_cur + 1);
                    (off, r_cur + 1, s_cur)
                } else if is_v128 {
                    let s_al = (s_cur + 15) & !15;
                    let off = self.scalar_base + s_al as i32;
                    emit_store_off(a, Ty::V128, None, Some(SFP1), Reg::SP, off, SGP3);
                    (off, r_cur, s_al + 16)
                } else {
                    let off = self.scalar_base + s_cur as i32;
                    emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                    (off, r_cur, s_cur + 8)
                };
                self.emit_eval(a, len, false, r_next, s_next)?;
                a.uxtw(Reg::X1, Reg::X0);
                a.mov_imm64(SGP2, u64::from(elem_sz));
                a.mul(true, SGP2, SGP2, Reg::X1);
                a.add_imm(true, SGP2, SGP2, 15);
                a.and_imm_align8(SGP2, SGP2);
                self.emit_alloc_dyn_sgp2_into_x0(a);
                self.emit_write_header(a, Reg::X0, gt, false);
                a.str_uimm(false, Reg::X1, Reg::X0, 4);
                if is_v128 {
                    emit_load_off(a, Ty::V128, None, Some(SFP1), Reg::SP, init_slot, SGP3);
                } else {
                    emit_load_off(a, Ty::I64, Some(Reg::X2), None, Reg::SP, init_slot, SGP3);
                }
                a.mov_imm32(Reg::X3, elem_sz);
                a.bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
            GcConstExpr::ArrayNewFixed { ty, elems } => {
                let gt = self.module.gc_type(*ty);
                let elem_storage = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNewFixed on non-array".into()),
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
                        emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                        r_next += 1;
                        self.emit_set_init_roots_len(a, r_next);
                        saved_offs.push(off);
                    } else if is_v128 {
                        s_next = (s_next + 15) & !15;
                        let off = self.scalar_base + s_next as i32;
                        emit_store_off(a, Ty::V128, None, Some(SFP1), Reg::SP, off, SGP3);
                        s_next += 16;
                        saved_offs.push(off);
                    } else {
                        let off = self.scalar_base + s_next as i32;
                        emit_store_off(a, Ty::I64, Some(Reg::X0), None, Reg::SP, off, SGP3);
                        s_next += 8;
                        saved_offs.push(off);
                    }
                }
                let arr_len = u32::try_from(elems.len())
                    .map_err(|_| "aarch64: ArrayNewFixed length exceeds u32")?;
                let byte_size = u32::try_from((8usize + elems.len() * (elem_sz as usize) + 7) & !7)
                    .map_err(|_| "aarch64: ArrayNewFixed byte size exceeds u32")?;
                self.emit_alloc_fixed(a, byte_size);
                self.emit_write_header(a, Reg::X0, gt, false);
                a.mov_imm32(SGP1, arr_len);
                a.str_uimm(false, SGP1, Reg::X0, 4);
                for (i, &src_off) in saved_offs.iter().enumerate() {
                    let elem_off = 8 + (i as i32) * (elem_sz as i32);
                    self.emit_store_field_from_stack(a, Reg::X0, elem_off, elem_storage, src_off);
                }
                if r_next != r_cur {
                    self.emit_set_init_roots_len(a, r_cur);
                }
            }
        }
        Ok(())
    }
}

fn emit_a64_gc_const_inits(
    a: &mut A64Asm,
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
    let save_bytes = 32usize;
    let frame_bytes = root_bytes + scalar_bytes + save_bytes;
    if frame_bytes > (2 << 20) {
        return Err(format!(
            "aarch64: GC const-expr init frame ({frame_bytes} B) exceeds 2 MiB cap"
        ));
    }
    let frame_i32 = frame_bytes as i32;
    let save_off = (root_bytes + scalar_bytes) as i32;

    let lbl_local_alloc_trap = a.new_label();
    let lbl_local_table_oob = a.new_label();
    let lbl_init_done = a.new_label();

    a.add_i32_checked(true, Reg::SP, Reg::SP, -frame_i32, SGP3);
    a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
    a.ldr_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_PTR as u32);
    emit_store_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off, SGP3);
    a.ldr_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_LEN as u32);
    emit_store_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off + 8, SGP3);
    a.ldr_uimm(true, SGP2, SGP1, GC_OFF_WALK_STOP_RSP as u32);
    emit_store_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off + 16, SGP3);
    a.mov_r(true, SGP2, Reg::SP);
    a.str_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_PTR as u32);
    a.str_uimm(true, Reg::XZR, SGP1, GC_OFF_INIT_ROOTS_LEN as u32);
    a.str_uimm(true, Reg::XZR, SGP1, GC_OFF_WALK_STOP_RSP as u32);

    let ictx = InitExprCtxA64 {
        module,
        layout,
        scalar_base: root_bytes as i32,
        lbl_trap_alloc: lbl_local_alloc_trap,
    };

    // 1. Runtime GC globals in declaration order.
    for (gi, expr) in &module.runtime_global_inits {
        let is_v128 = module.globals[*gi].0 == ValType::V128;
        ictx.emit_eval(a, expr, is_v128, 0, 0)?;
        let goff = layout.global_offs[*gi];
        if is_v128 {
            emit_store_off(a, Ty::V128, None, Some(SFP1), CTX_REG, goff, SGP3);
        } else {
            emit_store_off(a, Ty::I64, Some(Reg::X0), None, CTX_REG, goff, SGP3);
        }
    }

    // 2. Runtime GC table initializers.
    for (tbl, expr) in &module.runtime_table_inits {
        ictx.emit_eval(a, expr, false, 0, 0)?;
        if let Some(&(_, min_len, _)) = layout.dyn_tables.get(tbl)
            && min_len > 0
        {
            let lp = a.new_label();
            a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
            emit_load_off(
                a,
                Ty::I64,
                Some(Reg::X1),
                None,
                Reg::X1,
                (*tbl as i32) * 8,
                SGP3,
            );
            a.ldr_uimm(true, Reg::X1, Reg::X1, 0);
            a.mov_imm32(Reg::X2, min_len);
            a.bind(lp);
            a.str_uimm(true, Reg::X0, Reg::X1, 0);
            a.add_imm(true, Reg::X1, Reg::X1, 8);
            a.subs_imm(true, Reg::X2, Reg::X2, 1);
            a.b_cond(Cond::Ne, lp);
        }
    }

    // 3. Active & passive GC element segments.
    let mut local_table_oob = false;
    for (ei, seg) in module.elem_segs.iter().enumerate() {
        let Some(ref ge) = seg.gc_entries else {
            continue;
        };
        if let Some((tbl, dst_off)) = seg.active_target {
            local_table_oob = true;
            *need_table_oob = true;
            a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
            emit_load_off(
                a,
                Ty::I64,
                Some(Reg::X1),
                None,
                Reg::X1,
                (tbl as i32) * 8,
                SGP3,
            );
            a.ldr_uimm(false, Reg::X2, Reg::X1, 8); // cur_len zero-ext u64
            if let Some(end) = dst_off.checked_add(ge.len() as u64) {
                a.mov_imm64(Reg::X3, end);
                a.cmp_r(true, Reg::X3, Reg::X2);
                a.b_cond(Cond::Hi, lbl_local_table_oob);
            } else {
                a.b(lbl_local_table_oob);
                continue;
            }
            for (i, expr) in ge.iter().enumerate() {
                ictx.emit_eval(a, expr, false, 0, 0)?;
                let slot_idx = dst_off + i as u64;
                a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
                emit_load_off(
                    a,
                    Ty::I64,
                    Some(Reg::X1),
                    None,
                    Reg::X1,
                    (tbl as i32) * 8,
                    SGP3,
                );
                a.ldr_uimm(true, Reg::X1, Reg::X1, 0);
                a.mov_imm64(Reg::X2, slot_idx);
                a.str_reg_scaled(true, Reg::X0, Reg::X1, Reg::X2, false);
            }
        } else if seg.is_passive {
            let buf_base_off = gc.passive_gc_elem_offs[ei];
            debug_assert!(buf_base_off >= 0);
            for (i, expr) in ge.iter().enumerate() {
                ictx.emit_eval(a, expr, false, 0, 0)?;
                a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_STORE_ELEM_BUF_PTR as u32);
                let byte_off = (buf_base_off as u64) + (i as u64) * 8;
                a.mov_imm64(Reg::X2, byte_off);
                a.str_reg(true, Reg::X0, Reg::X1, Reg::X2, false);
            }
        }
    }

    let emit_teardown = |a: &mut A64Asm| {
        a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        emit_load_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off, SGP3);
        a.str_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_PTR as u32);
        emit_load_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off + 8, SGP3);
        a.str_uimm(true, SGP2, SGP1, GC_OFF_INIT_ROOTS_LEN as u32);
        emit_load_off(a, Ty::I64, Some(SGP2), None, Reg::SP, save_off + 16, SGP3);
        a.str_uimm(true, SGP2, SGP1, GC_OFF_WALK_STOP_RSP as u32);
        a.add_i32_checked(true, Reg::SP, Reg::SP, frame_i32, SGP3);
    };

    emit_teardown(a);
    a.b(lbl_init_done);

    a.bind(lbl_local_alloc_trap);
    emit_teardown(a);
    a.b(lbl_trap_alloc);

    if local_table_oob {
        a.bind(lbl_local_table_oob);
        emit_teardown(a);
        a.b(lbl_trap_table_oob);
    }

    a.bind(lbl_init_done);
    Ok(())
}

/// Emits `wt_store_init_instance(ctx: *mut u8 [X0]) -> u32 [W0]` (AAPCS64 C ABI, `HELPER_INITIALIZE`).
fn emit_a64_store_init_instance(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let lbl_trap_oob = a.new_label();
    let lbl_trap_table_oob = a.new_label();
    let lbl_trap_alloc = a.new_label();
    let lbl_done = a.new_label();

    // Frame layout (224B, 16B aligned):
    //   +0..128   : Q8..Q15 (128B)
    //   +128..208 : X19..X28 (80B)
    //   +208..224 : FP, LR (16B)
    const INIT_FRAME: i32 = 224;
    a.sub_imm(true, Reg::SP, Reg::SP, INIT_FRAME as u16);
    emit_save_fp_lr(&mut a, 208);
    a.add_imm(true, Reg::FP, Reg::SP, 208);
    for i in (0..CALLEE_GP_REGS.len()).step_by(2) {
        a.stp_x(
            CALLEE_GP_REGS[i],
            CALLEE_GP_REGS[i + 1],
            Reg::SP,
            128 + (i as i32) * 8,
        );
    }
    emit_save_callee_q8_15(&mut a, 0);

    a.mov_r(true, CTX_REG, Reg::X0);
    a.ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
    a.ldr_uimm(true, SGP1, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, SGP1, 8);

    // 1. Initialize defined non-folded scalar / FuncRef / I31 / ExternI31 / V128 globals.
    for (gi, &init) in module
        .global_inits
        .iter()
        .enumerate()
        .skip(module.imported_globals.len())
    {
        if module.global_consts[gi].is_some() {
            continue;
        }
        let off = layout.global_offs[gi];
        match init {
            X64ConstVal::I32(v) => {
                a.mov_imm32(SGP1, v as u32);
                emit_store_off(&mut a, Ty::I32, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::I64(v) => {
                a.mov_imm64(SGP1, v as u64);
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::F32(bits) => {
                a.mov_imm32(SGP1, bits);
                emit_store_off(&mut a, Ty::I32, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::F64(bits) => {
                a.mov_imm64(SGP1, bits);
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::RefNull => {
                emit_store_off(&mut a, Ty::I64, Some(Reg::XZR), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::FuncRef(f) => {
                a.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_FUNC_REFS as u32);
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    SGP1,
                    (f as i32) * 8,
                    SGP3,
                );
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::I31(enc) => {
                a.mov_imm64(SGP1, u64::from(enc));
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::ExternI31(enc) => {
                a.mov_imm64(SGP1, GC_EXT_I31_TAG_HI | u64::from(enc));
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
            }
            X64ConstVal::V128(bytes) => {
                let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                a.mov_imm64(SGP1, lo);
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
                a.mov_imm64(SGP1, hi);
                emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off + 8, SGP3);
            }
            X64ConstVal::RuntimeGc => {}
        }
    }

    // 2. Initialize defined tables' non-zero scalar / FuncRef defaults.
    for (&tbl, meta) in &module.dyn_tables {
        if (tbl as usize) < module.imported_tables.len() || meta.init_val == 0 || meta.min_len == 0
        {
            continue;
        }
        let lp = a.new_label();
        if meta.is_funcref {
            let f = (meta.init_val - 1) as i32;
            a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_STORE_FUNC_REFS as u32);
            emit_load_off(&mut a, Ty::I64, Some(Reg::X0), None, Reg::X0, f * 8, SGP3);
        } else {
            a.mov_imm64(Reg::X0, meta.init_val);
        }
        a.ldr_uimm(true, Reg::X1, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            Reg::X1,
            (tbl as i32) * 8,
            SGP3,
        );
        a.ldr_uimm(true, Reg::X1, Reg::X1, 0);
        a.mov_imm32(Reg::X2, meta.min_len);
        a.bind(lp);
        a.str_uimm(true, Reg::X0, Reg::X1, 0);
        a.add_imm(true, Reg::X1, Reg::X1, 8);
        a.subs_imm(true, Reg::X2, Reg::X2, 1);
        a.b_cond(Cond::Ne, lp);
    }

    // 3. Evaluate GC const expressions (globals, tables, active/passive GC elem segs).
    let mut need_table_oob = false;
    let mut need_alloc_trap = false;
    let mut need_oob = false;
    if module.uses_gc {
        emit_a64_gc_const_inits(
            &mut a,
            module,
            layout,
            lbl_trap_table_oob,
            lbl_trap_alloc,
            &mut need_table_oob,
            &mut need_alloc_trap,
        )?;
    }

    // 4. Initialize active non-GC (`funcref`) element segments into `StoreTableDesc`s.
    for &(tbl, dst_off, ro_off, elem_len) in &tables.active_dyn_elem_inits {
        need_table_oob = true;
        a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_TABLE_DESCS as u32);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X9),
            None,
            Reg::X9,
            (tbl as i32) * 8,
            SGP3,
        );
        a.ldr_uimm(false, Reg::X10, Reg::X9, 8); // cur_len zero-ext u64
        if let Some(end) = dst_off.checked_add(u64::from(elem_len)) {
            a.mov_imm64(Reg::X0, end);
            a.cmp_r(true, Reg::X0, Reg::X10);
            a.b_cond(Cond::Hi, lbl_trap_table_oob);
        } else {
            a.b(lbl_trap_table_oob);
            continue;
        }
        if elem_len > 0 {
            let lp = a.new_label();
            let st = a.new_label();
            a.ldr_uimm(true, Reg::X0, Reg::X9, 0);
            if dst_off != 0 {
                a.mov_imm64(Reg::X3, dst_off);
                a.add_r_lsl(true, Reg::X0, Reg::X0, Reg::X3, 3);
            }
            a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(ro_off));
            a.ldr_uimm(true, Reg::X11, CTX_REG, OFF_STORE_FUNC_REFS as u32);
            a.mov_imm32(Reg::X2, elem_len);
            a.bind(lp);
            a.ldr_uimm(false, Reg::X3, Reg::X1, 0);
            a.cbz(false, Reg::X3, st);
            a.sub_imm(true, Reg::X3, Reg::X3, 1);
            a.ldr_reg_scaled(true, Reg::X3, Reg::X11, Reg::X3, false);
            a.bind(st);
            a.str_uimm(true, Reg::X3, Reg::X0, 0);
            a.add_imm(true, Reg::X1, Reg::X1, 4);
            a.add_imm(true, Reg::X0, Reg::X0, 8);
            a.subs_imm(true, Reg::X2, Reg::X2, 1);
            a.b_cond(Cond::Ne, lp);
        }
    }

    // Mark active/declarative element segments dropped in `elem_dropped_off`.
    if let Some(drop_off) = layout.elem_dropped_off {
        a.mov_imm32(Reg::X4, 1);
        for (ei, seg) in module.elem_segs.iter().enumerate() {
            if !seg.is_passive {
                a.add_i32_checked(true, Reg::X5, CTX_REG, drop_off + (ei as i32), SGP3);
                a.strb_uimm(Reg::X4, Reg::X5, 0);
            }
        }
    }

    // 5. Initialize active data segments into `StoreMemDesc`s.
    for &(mem_idx, mem_off, ro_off, len) in &tables.data_segs {
        need_oob = true;
        a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_MEM_DESCS as u32);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X9),
            None,
            Reg::X9,
            (mem_idx as i32) * 8,
            SGP3,
        );
        if let Some(end) = mem_off.checked_add(u64::from(len)) {
            a.mov_imm64(Reg::X0, end);
            a.ldr_uimm(true, Reg::X10, Reg::X9, 8);
            a.cmp_r(true, Reg::X0, Reg::X10);
            a.b_cond(Cond::Hi, lbl_trap_oob);
        } else {
            a.b(lbl_trap_oob);
            continue;
        }
        if len > 0 {
            a.ldr_uimm(true, Reg::X0, Reg::X9, 0);
            if mem_off != 0 {
                a.mov_imm64(Reg::X3, mem_off);
                a.add_r(true, Reg::X0, Reg::X0, Reg::X3);
            }
            a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(ro_off));
            a.mov_imm32(Reg::X2, len);
            let lbl_copied = a.new_label();
            emit_a64_memmove_x0_x1_x2(&mut a, lbl_copied);
            a.bind(lbl_copied);
        }
    }

    // Mark active data segments dropped in `data_dropped_off`.
    if let Some(drop_off) = layout.data_dropped_off {
        a.mov_imm32(Reg::X4, 1);
        for &(_, _, di) in &module.active_data_segs {
            a.add_i32_checked(true, Reg::X5, CTX_REG, drop_off + (di as i32), SGP3);
            a.strb_uimm(Reg::X4, Reg::X5, 0);
        }
    }

    // 6. Execute `start` function `() -> ()` (if any), managing `gc_walk_stop_rsp` cleanly.
    a.movz(false, SGP1, 0, 0);
    if let Some(s) = module.start {
        a.ldr_uimm(true, Reg::X19, CTX_REG, OFF_STORE_PTR as u32);
        a.ldr_uimm(true, Reg::X20, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);
        a.sub_imm(true, Reg::X0, Reg::SP, 8);
        a.str_uimm(true, Reg::X0, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);
        if (s as usize) < module.imports.len() {
            a.mov_r(true, Reg::X21, CTX_REG);
            a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_STORE_FUNC_REFS as u32);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X9),
                None,
                Reg::X9,
                (s as i32) * 8,
                SGP3,
            );
            a.ldr_uimm(true, CTX_REG, Reg::X9, 8);
            a.ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
            a.ldr_uimm(true, Reg::X10, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
            a.ldr_uimm(true, MEM_LEN_REG, Reg::X10, 8);
            a.ldr_uimm(true, Reg::X9, Reg::X9, 0);
            a.blr(Reg::X9);
            a.mov_r(true, CTX_REG, Reg::X21);
        } else {
            a.bl_reloc(A64RelocTarget::Func(s));
        }
        a.str_uimm(true, Reg::X20, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);
    }

    a.bind(lbl_done);
    a.mov_r(false, Reg::X0, SGP1);
    emit_restore_callee_q8_15(&mut a, 0);
    for i in (0..CALLEE_GP_REGS.len()).step_by(2) {
        a.ldp_x(
            CALLEE_GP_REGS[i],
            CALLEE_GP_REGS[i + 1],
            Reg::SP,
            128 + (i as i32) * 8,
        );
    }
    emit_restore_fp_lr(&mut a, 208);
    a.add_imm(true, Reg::SP, Reg::SP, INIT_FRAME as u16);
    a.ret();

    if need_oob {
        a.bind(lbl_trap_oob);
        a.mov_imm32(SGP1, u32::from(TRAP_OOB));
        a.b(lbl_done);
    }
    if need_table_oob {
        a.bind(lbl_trap_table_oob);
        a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
        a.b(lbl_done);
    }
    if need_alloc_trap {
        a.bind(lbl_trap_alloc);
        a.mov_imm32(SGP1, u32::from(TRAP_ALLOC_FAILED));
        a.b(lbl_done);
    }

    a.finish()?;
    Ok(a)
}

/// Computes flat `[u64]` byte offset for each value in a host-marshalled args/rets buffer:
/// scalar/ref = 8 bytes (1 `u64`), `V128` = 16 bytes (2 `u64`s). Returns `(offsets, total_bytes)`.
fn flat_buf_layout(tys: &[Ty]) -> (Vec<i32>, i32) {
    let mut offs = Vec::with_capacity(tys.len());
    let mut cur = 0i32;
    for &ty in tys {
        offs.push(cur);
        cur += if ty == Ty::V128 { 16 } else { 8 };
    }
    (offs, cur)
}

/// Emits an AArch64 `StoreCdylib` host-import trampoline for machine signature `(param_tys, result_tys)`.
///
/// Called by Wasm via `StoreFuncRef`:
/// - `X28 (CTX_REG)` = `HostFuncCtx*` (`+0: 0`, `+8: host_func_id (u64)`, `+16: store_ptr`, `+24: &ZERO_MEM_DESC`)
/// - `LR` = caller's Wasm safepoint return address; saved at `[SP_caller - 8]` (`[SP + fp_lr_off + 8]`).
/// - Preserves Wasm callee-saved `X19..X28` and full 128-bit `Q8..Q15`.
/// - Pushes `GcStackChainNode` onto `store->gc_stack_chain`, calls `store->host_call_fn`,
///   unlinks the chain node, unpacks results, and returns status in `W16 (SGP1)`.
pub fn emit_a64_store_host_trampoline(
    param_tys: &[Ty],
    result_tys: &[Ty],
) -> Result<A64Asm, String> {
    let arg_layout = layout_args(param_tys)?;
    let ret_layout = layout_rets(result_tys)?;
    let (arg_flat_offs, arg_flat_bytes) = flat_buf_layout(param_tys);
    let (ret_flat_offs, ret_flat_bytes) = flat_buf_layout(result_tys);

    // Frame layout (all 16B aligned):
    //   +0 .. args_bytes_al     : flat args buffer
    //   +rets_off ..            : flat rets buffer
    //   +chain_off .. +24       : GcStackChainNode { prev_chain, outer_stop_rsp, outer_paused_ret_rsp }
    //   +ret_spill_save_off     : saved `store->ret_spill_ptr` (8B)
    //   +q_save_off .. +128     : full 128-bit Wasm callee-saved `Q8..Q15`
    //   +gp_save_off .. +16     : saved `X19, X20` (AAPCS64 preserves `X21..X28`)
    //   +fp_lr_off .. +16       : saved `(FP, LR)` at TOP of frame so LR sits at `[SP_caller - 8]`!
    let args_bytes_al = (arg_flat_bytes + 15) & !15;
    let rets_off = args_bytes_al;
    let rets_bytes_al = (ret_flat_bytes + 15) & !15;
    let chain_off = rets_off + rets_bytes_al;
    let ret_spill_save_off = chain_off + 24;
    let q_save_off = chain_off + 32;
    let gp_save_off = q_save_off + 128;
    let fp_lr_off = gp_save_off + 16;
    let frame_sz = fp_lr_off + 16;
    let entry_ret_rsp_off = fp_lr_off + 8; // == frame_sz - 8 == SP_caller - 8!
    let incoming_stack_base = frame_sz;

    let mut a = A64Asm::new();
    let lbl_trapped = a.new_label();

    a.add_i32_checked(true, Reg::SP, Reg::SP, -frame_sz, SGP3);
    emit_save_fp_lr(&mut a, fp_lr_off);
    a.add_i32_checked(true, Reg::FP, Reg::SP, fp_lr_off, SGP3);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X19),
        None,
        Reg::SP,
        gp_save_off,
        SGP3,
    );
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X20),
        None,
        Reg::SP,
        gp_save_off + 8,
        SGP3,
    );

    // X20 = HostFuncCtx*, X19 = store_ptr (preserved across C call!).
    a.mov_r(true, Reg::X20, CTX_REG);
    a.ldr_uimm(true, Reg::X19, Reg::X20, 16);
    if ret_layout.stack_bytes > 0 {
        a.ldr_uimm(true, SGP1, Reg::X19, STORE_OFF_RET_SPILL_PTR as u32);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(SGP1),
            None,
            Reg::SP,
            ret_spill_save_off,
            SGP3,
        );
    }

    // 1. Pack Wasm ABI arguments (`X0..X7`, `V0..V7`, `[SP + frame_sz + soff]`) into `[SP + 0..]`.
    for ((&ty, &slot), &foff) in param_tys.iter().zip(&arg_layout.slots).zip(&arg_flat_offs) {
        match slot {
            A64AbiSlot::Gp(r) => {
                if ty == Ty::I32 {
                    a.uxtw(SGP1, r);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, foff, SGP3);
                } else {
                    emit_store_off(&mut a, Ty::I64, Some(r), None, Reg::SP, foff, SGP3);
                }
            }
            A64AbiSlot::Fp(v) => match ty {
                Ty::F32 => {
                    emit_store_off(&mut a, Ty::I64, Some(Reg::XZR), None, Reg::SP, foff, SGP3);
                    emit_store_off(&mut a, Ty::F32, None, Some(v), Reg::SP, foff, SGP3);
                }
                Ty::F64 => {
                    emit_store_off(&mut a, Ty::F64, None, Some(v), Reg::SP, foff, SGP3);
                }
                Ty::V128 => {
                    emit_store_off(&mut a, Ty::V128, None, Some(v), Reg::SP, foff, SGP3);
                }
                _ => unreachable!(),
            },
            A64AbiSlot::Stack(soff) => {
                let src_off = incoming_stack_base + soff;
                match ty {
                    Ty::I32 | Ty::F32 => {
                        emit_load_off(&mut a, Ty::I32, Some(SGP1), None, Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, foff, SGP3);
                    }
                    Ty::I64 | Ty::F64 => {
                        emit_load_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, foff, SGP3);
                    }
                    Ty::V128 => {
                        emit_load_off(&mut a, Ty::V128, None, Some(SFP1), Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::V128, None, Some(SFP1), Reg::SP, foff, SGP3);
                    }
                }
            }
        }
    }

    // 2. Save full 128-bit Wasm callee-saved `Q8..Q15` (AAPCS64 only preserves low 64-bit `D8..D15`).
    emit_save_callee_q8_15(&mut a, q_save_off);

    // 3. Push `GcStackChainNode` onto `store->gc_stack_chain` and pause `gc_walk_stop_rsp = 0`.
    a.ldr_uimm(true, SGP1, Reg::X19, GC_OFF_STACK_CHAIN as u32);
    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, chain_off, SGP3);
    a.ldr_uimm(true, SGP1, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(SGP1),
        None,
        Reg::SP,
        chain_off + 8,
        SGP3,
    );
    a.add_i32_checked(true, SGP1, Reg::SP, entry_ret_rsp_off, SGP3);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(SGP1),
        None,
        Reg::SP,
        chain_off + 16,
        SGP3,
    );
    a.add_i32_checked(true, SGP1, Reg::SP, chain_off, SGP3);
    a.str_uimm(true, SGP1, Reg::X19, GC_OFF_STACK_CHAIN as u32);
    a.str_uimm(true, Reg::XZR, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);

    // 4. Call `store->host_call_fn(store_ptr [X0], host_func_id [W1], args_ptr [X2], rets_ptr [X3]) -> u32 [W0]`.
    a.mov_r(true, Reg::X0, Reg::X19);
    a.ldr_uimm(false, Reg::X1, Reg::X20, 8);
    a.mov_r(true, Reg::X2, Reg::SP);
    a.add_i32_checked(true, Reg::X3, Reg::SP, rets_off, SGP3);
    a.ldr_uimm(true, Reg::X9, Reg::X19, STORE_OFF_HOST_CALL_FN as u32);
    a.blr(Reg::X9);
    a.mov_r(false, SGP1, Reg::X0); // trap / exn status in W16 (SGP1)

    // 5. Unlink `GcStackChainNode`.
    emit_load_off(&mut a, Ty::I64, Some(SGP2), None, Reg::SP, chain_off, SGP3);
    a.str_uimm(true, SGP2, Reg::X19, GC_OFF_STACK_CHAIN as u32);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        Reg::SP,
        chain_off + 8,
        SGP3,
    );
    a.str_uimm(true, SGP2, Reg::X19, GC_OFF_WALK_STOP_RSP as u32);

    a.cbnz(false, SGP1, lbl_trapped);

    // 6. Unpack flat results from `[SP + rets_off..]` into Wasm ABI return regs & overflow buffer.
    if ret_layout.stack_bytes > 0 {
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(SGP2),
            None,
            Reg::SP,
            ret_spill_save_off,
            SGP3,
        );
    }
    for ((&ty, &slot), &foff) in result_tys.iter().zip(&ret_layout.slots).zip(&ret_flat_offs) {
        let src_off = rets_off + foff;
        match slot {
            A64AbiSlot::Gp(r) => {
                emit_load_off(&mut a, ty, Some(r), None, Reg::SP, src_off, SGP3);
            }
            A64AbiSlot::Fp(v) => {
                emit_load_off(&mut a, ty, None, Some(v), Reg::SP, src_off, SGP3);
            }
            A64AbiSlot::Stack(boff) => match ty {
                Ty::I32 | Ty::F32 => {
                    emit_load_off(&mut a, Ty::I32, Some(Reg::X9), None, Reg::SP, src_off, SGP3);
                    emit_store_off(&mut a, Ty::I32, Some(Reg::X9), None, SGP2, boff, SGP3);
                }
                Ty::I64 | Ty::F64 => {
                    emit_load_off(&mut a, Ty::I64, Some(Reg::X9), None, Reg::SP, src_off, SGP3);
                    emit_store_off(&mut a, Ty::I64, Some(Reg::X9), None, SGP2, boff, SGP3);
                }
                Ty::V128 => {
                    emit_load_off(&mut a, Ty::V128, None, Some(SFP1), Reg::SP, src_off, SGP3);
                    emit_store_off(&mut a, Ty::V128, None, Some(SFP1), SGP2, boff, SGP3);
                }
            },
        }
    }

    a.bind(lbl_trapped);
    emit_restore_callee_q8_15(&mut a, q_save_off);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X19),
        None,
        Reg::SP,
        gp_save_off,
        SGP3,
    );
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X20),
        None,
        Reg::SP,
        gp_save_off + 8,
        SGP3,
    );
    emit_restore_fp_lr(&mut a, fp_lr_off);
    a.add_i32_checked(true, Reg::SP, Reg::SP, frame_sz, SGP3);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Emits an AArch64 `StoreCdylib` export-entry trampoline for machine signature `(param_tys, result_tys)`.
///
/// AAPCS64 C ABI:
/// `extern "C" fn(ctx: *mut u8 [X0], fn_code: *const u8 [X1], args_ptr: *const u64 [X2], rets_ptr: *mut u64 [X3]) -> u32 [W0]`
pub fn emit_a64_store_export_entry(param_tys: &[Ty], result_tys: &[Ty]) -> Result<A64Asm, String> {
    let arg_layout = layout_args(param_tys)?;
    let ret_layout = layout_rets(result_tys)?;
    let (arg_flat_offs, _) = flat_buf_layout(param_tys);
    let (ret_flat_offs, _) = flat_buf_layout(result_tys);

    // Frame layout (all 16B aligned):
    //   +0 .. outgoing_arg_bytes : Wasm outgoing stack arguments (read by callee at `[SP_caller + soff]`)
    //   +ret_buf_off ..          : Wasm overflow return buffer (16B aligned)
    //   +prev_stop_off .. +16    : saved `store->gc_walk_stop_rsp` (8B + 8B pad)
    //   +q_save_off .. +128      : saved `Q8..Q15`
    //   +gp_save_off .. +80      : saved `X19..X28`
    //   +fp_lr_off .. +16        : saved `(FP, LR)`
    let ret_buf_off = (arg_layout.stack_bytes + 15) & !15;
    let prev_stop_off = (ret_buf_off + ret_layout.stack_bytes + 15) & !15;
    let q_save_off = prev_stop_off + 16;
    let gp_save_off = q_save_off + 128;
    let fp_lr_off = gp_save_off + 80;
    let frame_sz = fp_lr_off + 16;

    let mut a = A64Asm::new();
    let lbl_done = a.new_label();

    a.add_i32_checked(true, Reg::SP, Reg::SP, -frame_sz, SGP3);
    emit_save_fp_lr(&mut a, fp_lr_off);
    a.add_i32_checked(true, Reg::FP, Reg::SP, fp_lr_off, SGP3);
    for (i, &r) in CALLEE_GP_REGS.iter().enumerate() {
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(r),
            None,
            Reg::SP,
            gp_save_off + (i as i32) * 8,
            SGP3,
        );
    }
    emit_save_callee_q8_15(&mut a, q_save_off);

    // Pin callee-saved registers across Wasm call:
    //   X28 = ctx, X21 = fn_code, X22 = args_ptr (until unpacked), X19 = rets_ptr, X20 = store_ptr.
    a.mov_r(true, CTX_REG, Reg::X0);
    a.mov_r(true, Reg::X21, Reg::X1);
    a.mov_r(true, Reg::X22, Reg::X2);
    a.mov_r(true, Reg::X19, Reg::X3);
    a.ldr_uimm(true, Reg::X20, CTX_REG, OFF_STORE_PTR as u32);

    // Load `MEM_BASE_REG = mem0_base`, `MEM_LEN_REG = mem0_desc->len`.
    a.ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
    a.ldr_uimm(true, SGP1, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
    a.ldr_uimm(true, MEM_LEN_REG, SGP1, 8);

    // Publish overflow return buffer pointer if needed.
    if ret_layout.stack_bytes > 0 {
        a.add_i32_checked(true, SGP1, Reg::SP, ret_buf_off, SGP3);
        a.str_uimm(true, SGP1, Reg::X20, STORE_OFF_RET_SPILL_PTR as u32);
    }

    // Unpack flat `args_ptr` (`X22`) into Wasm ABI argument regs & outgoing stack area.
    for ((&ty, &slot), &foff) in param_tys.iter().zip(&arg_layout.slots).zip(&arg_flat_offs) {
        match slot {
            A64AbiSlot::Gp(r) => {
                emit_load_off(&mut a, ty, Some(r), None, Reg::X22, foff, SGP3);
            }
            A64AbiSlot::Fp(v) => {
                emit_load_off(&mut a, ty, None, Some(v), Reg::X22, foff, SGP3);
            }
            A64AbiSlot::Stack(soff) => match ty {
                Ty::I32 | Ty::F32 => {
                    emit_load_off(&mut a, Ty::I32, Some(SGP1), None, Reg::X22, foff, SGP3);
                    emit_store_off(&mut a, Ty::I32, Some(SGP1), None, Reg::SP, soff, SGP3);
                }
                Ty::I64 | Ty::F64 => {
                    emit_load_off(&mut a, Ty::I64, Some(SGP1), None, Reg::X22, foff, SGP3);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, Reg::SP, soff, SGP3);
                }
                Ty::V128 => {
                    emit_load_off(&mut a, Ty::V128, None, Some(SFP1), Reg::X22, foff, SGP3);
                    emit_store_off(&mut a, Ty::V128, None, Some(SFP1), Reg::SP, soff, SGP3);
                }
            },
        }
    }

    // Save previous `gc_walk_stop_rsp` and set `gc_walk_stop_rsp = SP - 8` right before `blr X21`.
    a.ldr_uimm(true, SGP1, Reg::X20, GC_OFF_WALK_STOP_RSP as u32);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(SGP1),
        None,
        Reg::SP,
        prev_stop_off,
        SGP3,
    );
    a.sub_imm(true, SGP1, Reg::SP, 8);
    a.str_uimm(true, SGP1, Reg::X20, GC_OFF_WALK_STOP_RSP as u32);

    a.blr(Reg::X21);

    // Restore `gc_walk_stop_rsp` (preserving `W16 = SGP1` and all Wasm return regs `X0..X7, V0..V7`!).
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        Reg::SP,
        prev_stop_off,
        SGP3,
    );
    a.str_uimm(true, SGP2, Reg::X20, GC_OFF_WALK_STOP_RSP as u32);

    a.cbnz(false, SGP1, lbl_done);

    // Pack Wasm return regs & overflow buffer into flat `rets_ptr` (`X19`), preserving `SGP1` (`0`).
    for ((&ty, &slot), &foff) in result_tys.iter().zip(&ret_layout.slots).zip(&ret_flat_offs) {
        match slot {
            A64AbiSlot::Gp(r) => {
                if ty == Ty::I32 {
                    a.uxtw(SGP2, r);
                    emit_store_off(&mut a, Ty::I64, Some(SGP2), None, Reg::X19, foff, SGP3);
                } else {
                    emit_store_off(&mut a, Ty::I64, Some(r), None, Reg::X19, foff, SGP3);
                }
            }
            A64AbiSlot::Fp(v) => match ty {
                Ty::F32 => {
                    emit_store_off(&mut a, Ty::I64, Some(Reg::XZR), None, Reg::X19, foff, SGP3);
                    emit_store_off(&mut a, Ty::F32, None, Some(v), Reg::X19, foff, SGP3);
                }
                Ty::F64 => {
                    emit_store_off(&mut a, Ty::F64, None, Some(v), Reg::X19, foff, SGP3);
                }
                Ty::V128 => {
                    emit_store_off(&mut a, Ty::V128, None, Some(v), Reg::X19, foff, SGP3);
                }
                _ => unreachable!(),
            },
            A64AbiSlot::Stack(boff) => {
                let src_off = ret_buf_off + boff;
                match ty {
                    Ty::I32 | Ty::F32 => {
                        emit_load_off(&mut a, Ty::I32, Some(SGP2), None, Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::I64, Some(SGP2), None, Reg::X19, foff, SGP3);
                    }
                    Ty::I64 | Ty::F64 => {
                        emit_load_off(&mut a, Ty::I64, Some(SGP2), None, Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::I64, Some(SGP2), None, Reg::X19, foff, SGP3);
                    }
                    Ty::V128 => {
                        emit_load_off(&mut a, Ty::V128, None, Some(SFP1), Reg::SP, src_off, SGP3);
                        emit_store_off(&mut a, Ty::V128, None, Some(SFP1), Reg::X19, foff, SGP3);
                    }
                }
            }
        }
    }

    a.bind(lbl_done);
    a.mov_r(false, Reg::X0, SGP1);
    emit_restore_callee_q8_15(&mut a, q_save_off);
    for (i, &r) in CALLEE_GP_REGS.iter().enumerate() {
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(r),
            None,
            Reg::SP,
            gp_save_off + (i as i32) * 8,
            SGP3,
        );
    }
    emit_restore_fp_lr(&mut a, fp_lr_off);
    a.add_i32_checked(true, Reg::SP, Reg::SP, frame_sz, SGP3);
    a.ret();
    a.finish()?;
    Ok(a)
}
