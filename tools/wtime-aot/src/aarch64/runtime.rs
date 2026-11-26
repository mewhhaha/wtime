//! AArch64 Blot ABI 8 runtime helpers, C-ABI export stubs, and Linux AArch64 static `_start`.
//!
//! Uses direct Linux AArch64 syscalls (`svc #0` with syscall number in `X8`, args in `X0..X5`,
//! return in `X0`):
//! - `SYS_READ = 63`, `SYS_WRITE = 64`, `SYS_EXIT_GROUP = 94`
//! - `SYS_MMAP = 222`, `SYS_MPROTECT = 226`, `SYS_MUNMAP = 215`

use super::asm::{A64Asm, A64RelocTarget, Cond, Reg, VReg};
use super::lower::{
    CTX_REG, MEM_BASE_REG, MEM_LEN_REG, PINNED_GLOB_REG, SGP1, SGP2, SGP3, emit_load_off,
    emit_store_off,
};
use crate::x64::ir::Ty;
use crate::x64::module::{X64ConstVal, X64Module};
use crate::x64::runtime::{EXPORT_NAMES, RuntimeTables};
use crate::x64::{
    CtxLayout, EXN_SLAB_BYTES, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES,
    GUEST_STACK_TOTAL_BYTES, GUEST_STACK_USABLE_BYTES, HELPER_EXTRA_MEM_COPY,
    HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW, HELPER_HOST_CALL, HELPER_INITIALIZE,
    HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, HELPER_MEM_INIT, HELPER_READ_GLOBAL,
    HELPER_TABLE_COPY, HELPER_TABLE_FILL, HELPER_TABLE_GROW, HELPER_TABLE_INIT,
    INTERNAL_EXN_SIGNAL, MAX_ACTIVE_REENTRY_DEPTH, OFF_ACTIVE_DEPTH, OFF_ACTIVE_GUEST_RSP,
    OFF_GUEST_STACK_BASE, OFF_GUEST_STACK_TOP, OFF_HANDLE_ALLOC_BYTES, OFF_HOST_FN, OFF_HOST_RSP,
    OFF_HOST_USER_CTX, OFF_IN_CALL, OFF_MEM_BASE, OFF_MEM_LEN, OFF_MEM_MAX_PAGES,
    OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR, TRAP_ALLOC_FAILED, TRAP_CALL_STACK, TRAP_HOST,
    TRAP_MEM_LIMIT, TRAP_MEM_TOO_LARGE, TRAP_OOB, TRAP_TABLE_OOB, TRAP_UNREACHABLE,
};
use wtime_types::ValType;

const SYS_MUNMAP: u64 = 215;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;

fn emit_syscall(asm: &mut A64Asm, num: u64) {
    asm.mov_imm64(Reg::X8, num);
    asm.svc(0);
}

/// Emits shared AArch64 runtime helpers (`HELPER_MEM_GROW`, `HELPER_MEM_COPY`, `HELPER_MEM_FILL`,
/// `HELPER_INITIALIZE`).
pub fn emit_a64_helpers(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Vec<(u32, A64Asm)>, String> {
    let mut out = Vec::new();

    // 1. HELPER_MEM_GROW: X0 = delta_pages -> returns old_pages or u64::MAX (-1) in X0, updates X26 & [X28+OFF_MEM_LEN].
    {
        let mut a = A64Asm::new();
        let fail = a.new_label();
        let ok_zero = a.new_label();
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
        a.lsr_imm(true, Reg::X1, MEM_LEN_REG, 16); // old_pages
        a.cbz(true, Reg::X0, ok_zero);
        // Check old_pages + delta <= max_pages (stored at [X28 + OFF_MEM_MAX_PAGES])
        a.adds_r(true, Reg::X2, Reg::X1, Reg::X0);
        a.b_cond(Cond::Hs, fail);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X3),
            None,
            CTX_REG,
            OFF_MEM_MAX_PAGES,
            SGP3,
        );
        a.cmp_r(true, Reg::X2, Reg::X3);
        a.b_cond(Cond::Hi, fail);
        // mprotect(mem_base + old_len, delta * 65536, PROT_READ | PROT_WRITE)
        a.mov_r(true, Reg::X9, Reg::X1); // save old_pages
        a.lsl_imm(true, Reg::X10, Reg::X2, 16); // new_len
        a.lsl_imm(true, Reg::X1, Reg::X0, 16); // len = delta << 16
        a.add_r(true, Reg::X0, MEM_BASE_REG, MEM_LEN_REG);
        a.mov_imm32(Reg::X2, 3); // PROT_READ | PROT_WRITE
        emit_syscall(&mut a, SYS_MPROTECT);
        a.mov_imm64(Reg::X3, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X3);
        a.b_cond(Cond::Hs, fail);
        a.mov_r(true, MEM_LEN_REG, Reg::X10);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(MEM_LEN_REG),
            None,
            CTX_REG,
            OFF_MEM_LEN,
            SGP3,
        );
        a.mov_r(true, Reg::X0, Reg::X9);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
        a.ret();

        a.bind(ok_zero);
        a.mov_r(true, Reg::X0, Reg::X1);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
        a.ret();

        a.bind(fail);
        a.mov_imm64(Reg::X0, u64::MAX);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
        a.ret();
        a.finish()?;
        out.push((HELPER_MEM_GROW, a));
    }

    // 2. HELPER_MEM_COPY: X0 = dst, X1 = src, X2 = len -> W16 = 0 or TRAP_OOB.
    {
        let mut a = A64Asm::new();
        let oob = a.new_label();
        let done = a.new_label();
        let bwd = a.new_label();
        let fwd_loop = a.new_label();
        let bwd_loop = a.new_label();
        a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X3, MEM_LEN_REG);
        a.b_cond(Cond::Hi, oob);
        a.adds_r(true, Reg::X3, Reg::X1, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X3, MEM_LEN_REG);
        a.b_cond(Cond::Hi, oob);
        a.cbz(true, Reg::X2, done);
        a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
        a.add_r(true, Reg::X1, MEM_BASE_REG, Reg::X1);
        a.cmp_r(true, Reg::X0, Reg::X1);
        a.b_cond(Cond::Hi, bwd);
        // Forward
        a.movz(true, Reg::X3, 0, 0);
        a.bind(fwd_loop);
        a.ldrb_reg(Reg::X4, Reg::X1, Reg::X3, false);
        a.strb_reg(Reg::X4, Reg::X0, Reg::X3, false);
        a.add_imm(true, Reg::X3, Reg::X3, 1);
        a.cmp_r(true, Reg::X3, Reg::X2);
        a.b_cond(Cond::Lo, fwd_loop);
        a.b(done);
        // Backward
        a.bind(bwd);
        a.bind(bwd_loop);
        a.sub_imm(true, Reg::X2, Reg::X2, 1);
        a.ldrb_reg(Reg::X4, Reg::X1, Reg::X2, false);
        a.strb_reg(Reg::X4, Reg::X0, Reg::X2, false);
        a.cbnz(true, Reg::X2, bwd_loop);
        a.bind(done);
        a.movz(false, SGP1, 0, 0);
        a.ret();
        a.bind(oob);
        a.mov_imm32(SGP1, u32::from(TRAP_OOB));
        a.ret();
        a.finish()?;
        out.push((HELPER_MEM_COPY, a));
    }

    // 3. HELPER_MEM_FILL: X0 = dst, X1 = byte_val, X2 = len -> W16 = 0 or TRAP_OOB.
    {
        let mut a = A64Asm::new();
        let oob = a.new_label();
        let done = a.new_label();
        let lp = a.new_label();
        a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X3, MEM_LEN_REG);
        a.b_cond(Cond::Hi, oob);
        a.cbz(true, Reg::X2, done);
        a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
        a.movz(true, Reg::X3, 0, 0);
        a.bind(lp);
        a.strb_reg(Reg::X1, Reg::X0, Reg::X3, false);
        a.add_imm(true, Reg::X3, Reg::X3, 1);
        a.cmp_r(true, Reg::X3, Reg::X2);
        a.b_cond(Cond::Lo, lp);
        a.bind(done);
        a.movz(false, SGP1, 0, 0);
        a.ret();
        a.bind(oob);
        a.mov_imm32(SGP1, u32::from(TRAP_OOB));
        a.ret();
        a.finish()?;
        out.push((HELPER_MEM_FILL, a));
    }

    // 4. HELPER_INITIALIZE: initializes globals, active data segments, and runs start function.
    {
        let mut a = A64Asm::new();
        let oob = a.new_label();
        let table_oob = a.new_label();
        let mut need_table_oob = false;
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
        for (idx, &init) in module.global_inits.iter().enumerate() {
            let off = layout.global_offs[idx];
            if off < 0 {
                continue;
            }
            match init {
                X64ConstVal::I32(v) => {
                    a.mov_imm32(SGP1, v as u32);
                    emit_store_off(&mut a, Ty::I32, Some(SGP1), None, CTX_REG, off, SGP3);
                }
                X64ConstVal::I64(v) => {
                    a.mov_imm64(SGP1, v as u64);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
                }
                X64ConstVal::RefNull => {
                    a.movz(true, SGP1, 0, 0);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
                }
                X64ConstVal::FuncRef(f) => {
                    a.mov_imm64(SGP1, u64::from(f) + 1);
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
                X64ConstVal::V128(bytes) => {
                    let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                    let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                    a.mov_imm64(SGP1, lo);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off, SGP3);
                    a.mov_imm64(SGP1, hi);
                    emit_store_off(&mut a, Ty::I64, Some(SGP1), None, CTX_REG, off + 8, SGP3);
                }
            }
        }
        // Explicit non-null initializers for dynamic tables (`table_inits` from function-references).
        for (&tbl, meta) in &module.dyn_tables {
            if meta.init_val != 0 && meta.min_len > 0 {
                let (desc_off, _, _) = layout.dyn_tables[&tbl];
                let lp = a.new_label();
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                a.mov_imm64(Reg::X1, meta.init_val);
                a.mov_imm32(Reg::X2, meta.min_len);
                a.movz(true, Reg::X3, 0, 0);
                a.bind(lp);
                a.str_reg_scaled(true, Reg::X1, Reg::X0, Reg::X3, true);
                a.add_imm(true, Reg::X3, Reg::X3, 1);
                a.cmp_r(true, Reg::X3, Reg::X2);
                a.b_cond(Cond::Lo, lp);
            }
        }
        // Static active element bounds checks + dynamic active element copies + implicit drops.
        for &(table_len, base_u64, elem_len) in &module.active_elem_checks {
            if base_u64
                .checked_add(u64::from(elem_len))
                .is_none_or(|end| end > u64::from(table_len))
            {
                need_table_oob = true;
                a.b(table_oob);
            }
        }
        for &(tbl, dst_off, rodata_off, elem_len) in &tables.active_dyn_elem_inits {
            let (desc_off, min_len, _) = layout.dyn_tables[&tbl];
            if dst_off
                .checked_add(u64::from(elem_len))
                .is_none_or(|end| end > u64::from(min_len))
            {
                need_table_oob = true;
                a.b(table_oob);
                continue;
            }
            if elem_len > 0 {
                let lp = a.new_label();
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                if dst_off > 0 {
                    a.mov_imm64(Reg::X5, dst_off * 8);
                    a.add_r(true, Reg::X0, Reg::X0, Reg::X5);
                }
                a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(rodata_off));
                a.mov_imm32(Reg::X2, elem_len);
                a.movz(true, Reg::X3, 0, 0);
                a.bind(lp);
                a.ldr_reg_scaled(false, Reg::X4, Reg::X1, Reg::X3, true);
                a.str_reg_scaled(true, Reg::X4, Reg::X0, Reg::X3, true);
                a.add_imm(true, Reg::X3, Reg::X3, 1);
                a.cmp_r(true, Reg::X3, Reg::X2);
                a.b_cond(Cond::Lo, lp);
            }
        }
        if let Some(elem_drop_off) = layout.elem_dropped_off {
            a.mov_imm32(Reg::X4, 1);
            for (ei, seg) in module.elem_segs.iter().enumerate() {
                if !seg.is_passive {
                    a.add_i32_checked(true, Reg::X5, CTX_REG, elem_drop_off + (ei as i32), SGP3);
                    a.strb_uimm(Reg::X4, Reg::X5, 0);
                }
            }
        }
        if let Some(data_drop_off) = layout.data_dropped_off {
            a.mov_imm32(Reg::X4, 1);
            for &(_, _, di) in &module.active_data_segs {
                a.add_i32_checked(true, Reg::X5, CTX_REG, data_drop_off + (di as i32), SGP3);
                a.strb_uimm(Reg::X4, Reg::X5, 0);
            }
        }
        // Copy active data segments from `.rodata` into target linear memory.
        for &(mem_idx, mem_off, rodata_off, seg_len) in &tables.data_segs {
            let len = u64::from(seg_len);
            a.mov_imm64(Reg::X0, mem_off);
            a.mov_imm64(Reg::X2, len);
            a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            if mem_idx == 0 {
                a.cmp_r(true, Reg::X3, MEM_LEN_REG);
                a.b_cond(Cond::Hi, oob);
                if len == 0 {
                    continue;
                }
                a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
            } else {
                let desc_off = layout.mem_desc_off(mem_idx);
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X5),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                a.cmp_r(true, Reg::X3, Reg::X5);
                a.b_cond(Cond::Hi, oob);
                if len == 0 {
                    continue;
                }
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X5),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                a.add_r(true, Reg::X0, Reg::X5, Reg::X0);
            }
            a.adr_reloc(Reg::X1, A64RelocTarget::Rodata(rodata_off));
            let lp = a.new_label();
            a.movz(true, Reg::X3, 0, 0);
            a.bind(lp);
            a.ldrb_reg(Reg::X4, Reg::X1, Reg::X3, false);
            a.strb_reg(Reg::X4, Reg::X0, Reg::X3, false);
            a.add_imm(true, Reg::X3, Reg::X3, 1);
            a.cmp_r(true, Reg::X3, Reg::X2);
            a.b_cond(Cond::Lo, lp);
        }
        if let Some(pg) = layout.pinned_global {
            let off = layout.global_offs[pg as usize];
            let ty = match module.globals[pg as usize].0 {
                ValType::I64 | ValType::Ref(_) => Ty::I64,
                _ => Ty::I32,
            };
            emit_load_off(&mut a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
        }
        if let Some(start_f) = module.start {
            a.bl_reloc(A64RelocTarget::Func(start_f));
            let done_trap = a.new_label();
            a.cbnz(false, SGP1, done_trap);
            if let Some(pg) = layout.pinned_global {
                let off = layout.global_offs[pg as usize];
                let ty = match module.globals[pg as usize].0 {
                    ValType::I64 | ValType::Ref(_) => Ty::I64,
                    _ => Ty::I32,
                };
                emit_store_off(&mut a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
            }
            a.bind(done_trap);
        } else {
            a.movz(false, SGP1, 0, 0);
        }
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
        a.ret();

        a.bind(oob);
        a.mov_imm32(SGP1, u32::from(TRAP_OOB));
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
        a.ret();
        if need_table_oob {
            a.bind(table_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
        }
        a.finish()?;
        out.push((HELPER_INITIALIZE, a));
    }

    // 5-7. Multi-memory helpers (`HELPER_EXTRA_MEM_GROW`, `HELPER_EXTRA_MEM_COPY`, `HELPER_EXTRA_MEM_FILL`).
    if !module.extra_memories.is_empty() {
        // HELPER_EXTRA_MEM_GROW: X0 = delta_pages, X1 = desc_off -> X0 = old_pages | u64::MAX.
        {
            let mut a = A64Asm::new();
            let fail = a.new_label();
            let ok_zero = a.new_label();
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
            a.add_r(true, Reg::X11, CTX_REG, Reg::X1); // desc_ptr
            a.ldr_uimm(true, Reg::X12, Reg::X11, 8); // old_len
            a.lsr_imm(true, Reg::X9, Reg::X12, 16); // old_pages
            a.cbz(true, Reg::X0, ok_zero);
            a.adds_r(true, Reg::X2, Reg::X9, Reg::X0);
            a.b_cond(Cond::Hs, fail);
            a.ldr_uimm(true, Reg::X3, Reg::X11, 16); // max_pages
            a.cmp_r(true, Reg::X2, Reg::X3);
            a.b_cond(Cond::Hi, fail);
            a.lsl_imm(true, Reg::X10, Reg::X2, 16); // new_len
            a.lsl_imm(true, Reg::X1, Reg::X0, 16); // delta_bytes
            a.ldr_uimm(true, Reg::X0, Reg::X11, 0); // base
            a.add_r(true, Reg::X0, Reg::X0, Reg::X12); // base + old_len
            a.mov_imm32(Reg::X2, 3); // PROT_RW
            emit_syscall(&mut a, SYS_MPROTECT);
            a.mov_imm64(Reg::X3, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X3);
            a.b_cond(Cond::Hs, fail);
            a.str_uimm(true, Reg::X10, Reg::X11, 8);
            a.mov_r(true, Reg::X0, Reg::X9);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
            a.bind(ok_zero);
            a.mov_r(true, Reg::X0, Reg::X9);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
            a.bind(fail);
            a.mov_imm64(Reg::X0, u64::MAX);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
            a.finish()?;
            out.push((HELPER_EXTRA_MEM_GROW, a));
        }
        // HELPER_EXTRA_MEM_COPY: X0 = dst, X1 = src, X2 = len, X3 = dst_desc_off, X4 = src_desc_off -> W16.
        {
            let mut a = A64Asm::new();
            let oob = a.new_label();
            let done = a.new_label();
            let bwd = a.new_label();
            let fwd_loop = a.new_label();
            let bwd_loop = a.new_label();
            a.add_r(true, Reg::X9, CTX_REG, Reg::X3); // dst_desc
            a.add_r(true, Reg::X10, CTX_REG, Reg::X4); // src_desc
            a.ldr_uimm(true, Reg::X11, Reg::X9, 8); // dst_len
            a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X11);
            a.b_cond(Cond::Hi, oob);
            a.ldr_uimm(true, Reg::X12, Reg::X10, 8); // src_len
            a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X12);
            a.b_cond(Cond::Hi, oob);
            a.cbz(true, Reg::X2, done);
            a.ldr_uimm(true, Reg::X11, Reg::X9, 0); // dst_base
            a.ldr_uimm(true, Reg::X12, Reg::X10, 0); // src_base
            a.add_r(true, Reg::X0, Reg::X11, Reg::X0);
            a.add_r(true, Reg::X1, Reg::X12, Reg::X1);
            a.cmp_r(true, Reg::X0, Reg::X1);
            a.b_cond(Cond::Hi, bwd);
            a.movz(true, Reg::X3, 0, 0);
            a.bind(fwd_loop);
            a.ldrb_reg(Reg::X4, Reg::X1, Reg::X3, false);
            a.strb_reg(Reg::X4, Reg::X0, Reg::X3, false);
            a.add_imm(true, Reg::X3, Reg::X3, 1);
            a.cmp_r(true, Reg::X3, Reg::X2);
            a.b_cond(Cond::Lo, fwd_loop);
            a.b(done);
            a.bind(bwd);
            a.bind(bwd_loop);
            a.sub_imm(true, Reg::X2, Reg::X2, 1);
            a.ldrb_reg(Reg::X4, Reg::X1, Reg::X2, false);
            a.strb_reg(Reg::X4, Reg::X0, Reg::X2, false);
            a.cbnz(true, Reg::X2, bwd_loop);
            a.bind(done);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(oob);
            a.mov_imm32(SGP1, u32::from(TRAP_OOB));
            a.ret();
            a.finish()?;
            out.push((HELPER_EXTRA_MEM_COPY, a));
        }
        // HELPER_EXTRA_MEM_FILL: X0 = dst, X1 = byte_val, X2 = len, X3 = desc_off -> W16.
        {
            let mut a = A64Asm::new();
            let oob = a.new_label();
            let done = a.new_label();
            let lp = a.new_label();
            a.add_r(true, Reg::X9, CTX_REG, Reg::X3);
            a.ldr_uimm(true, Reg::X10, Reg::X9, 8);
            a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X10);
            a.b_cond(Cond::Hi, oob);
            a.cbz(true, Reg::X2, done);
            a.ldr_uimm(true, Reg::X10, Reg::X9, 0);
            a.add_r(true, Reg::X0, Reg::X10, Reg::X0);
            a.movz(true, Reg::X3, 0, 0);
            a.bind(lp);
            a.strb_reg(Reg::X1, Reg::X0, Reg::X3, false);
            a.add_imm(true, Reg::X3, Reg::X3, 1);
            a.cmp_r(true, Reg::X3, Reg::X2);
            a.b_cond(Cond::Lo, lp);
            a.bind(done);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(oob);
            a.mov_imm32(SGP1, u32::from(TRAP_OOB));
            a.ret();
            a.finish()?;
            out.push((HELPER_EXTRA_MEM_FILL, a));
        }
    }

    // 8. HELPER_MEM_INIT: X0 = dst, X1 = src, X2 = len, X3 = data_idx, X4 = mem_desc_off -> W16.
    if let Some(drop_off) = layout.data_dropped_off {
        let mut a = A64Asm::new();
        let not_dropped = a.new_label();
        let oob = a.new_label();
        let done = a.new_label();
        let lp = a.new_label();
        a.adr_reloc(
            Reg::X9,
            A64RelocTarget::Rodata(tables.passive_data_table_off),
        );
        a.lsl_imm(true, Reg::X10, Reg::X3, 3);
        a.add_r(true, Reg::X10, Reg::X9, Reg::X10);
        a.ldr_uimm(false, Reg::X11, Reg::X10, 4); // seg_len
        a.add_i32_checked(true, Reg::X12, CTX_REG, drop_off, SGP3);
        a.ldrb_reg(Reg::X12, Reg::X12, Reg::X3, false);
        a.cbz(false, Reg::X12, not_dropped);
        a.movz(true, Reg::X11, 0, 0);
        a.bind(not_dropped);
        // Check src + len <= seg_len
        a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X5, Reg::X11);
        a.b_cond(Cond::Hi, oob);
        // Check dst + len <= mem_len
        a.add_r(true, Reg::X12, CTX_REG, Reg::X4);
        a.ldr_uimm(true, Reg::X11, Reg::X12, 8);
        a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X5, Reg::X11);
        a.b_cond(Cond::Hi, oob);
        a.cbz(true, Reg::X2, done);
        a.ldrsw_uimm(Reg::X5, Reg::X10, 0);
        a.add_r(true, Reg::X9, Reg::X9, Reg::X5);
        a.add_r(true, Reg::X1, Reg::X9, Reg::X1); // src_ptr
        a.ldr_uimm(true, Reg::X11, Reg::X12, 0);
        a.add_r(true, Reg::X0, Reg::X11, Reg::X0); // dst_ptr
        a.movz(true, Reg::X5, 0, 0);
        a.bind(lp);
        a.ldrb_reg(Reg::X6, Reg::X1, Reg::X5, false);
        a.strb_reg(Reg::X6, Reg::X0, Reg::X5, false);
        a.add_imm(true, Reg::X5, Reg::X5, 1);
        a.cmp_r(true, Reg::X5, Reg::X2);
        a.b_cond(Cond::Lo, lp);
        a.bind(done);
        a.movz(false, SGP1, 0, 0);
        a.ret();
        a.bind(oob);
        a.mov_imm32(SGP1, u32::from(TRAP_OOB));
        a.ret();
        a.finish()?;
        out.push((HELPER_MEM_INIT, a));
    }

    // 9. HELPER_TABLE_INIT: X0 = dst, X1 = src, X2 = len, X3 = elem_idx, X4 = tbl_desc_off -> W16.
    if let Some(drop_off) = layout.elem_dropped_off {
        let mut a = A64Asm::new();
        let not_dropped = a.new_label();
        let oob = a.new_label();
        let done = a.new_label();
        let lp = a.new_label();
        a.adr_reloc(Reg::X9, A64RelocTarget::Rodata(tables.elem_seg_table_off));
        a.lsl_imm(true, Reg::X10, Reg::X3, 3);
        a.add_r(true, Reg::X10, Reg::X9, Reg::X10);
        a.ldr_uimm(false, Reg::X11, Reg::X10, 4); // elem_len
        a.add_i32_checked(true, Reg::X12, CTX_REG, drop_off, SGP3);
        a.ldrb_reg(Reg::X12, Reg::X12, Reg::X3, false);
        a.cbz(false, Reg::X12, not_dropped);
        a.movz(true, Reg::X11, 0, 0);
        a.bind(not_dropped);
        a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X5, Reg::X11);
        a.b_cond(Cond::Hi, oob);
        a.add_r(true, Reg::X12, CTX_REG, Reg::X4);
        a.ldr_uimm(false, Reg::X11, Reg::X12, 8); // tbl_cur_len
        a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
        a.b_cond(Cond::Hs, oob);
        a.cmp_r(true, Reg::X5, Reg::X11);
        a.b_cond(Cond::Hi, oob);
        a.cbz(true, Reg::X2, done);
        a.ldrsw_uimm(Reg::X5, Reg::X10, 0);
        a.add_r(true, Reg::X9, Reg::X9, Reg::X5);
        a.lsl_imm(true, Reg::X5, Reg::X1, 2);
        a.add_r(true, Reg::X1, Reg::X9, Reg::X5); // src u32 ptr
        a.ldr_uimm(true, Reg::X11, Reg::X12, 0);
        a.lsl_imm(true, Reg::X5, Reg::X0, 3);
        a.add_r(true, Reg::X0, Reg::X11, Reg::X5); // dst u64 ptr
        a.movz(true, Reg::X5, 0, 0);
        a.bind(lp);
        a.ldr_reg_scaled(false, Reg::X6, Reg::X1, Reg::X5, true);
        a.str_reg_scaled(true, Reg::X6, Reg::X0, Reg::X5, true);
        a.add_imm(true, Reg::X5, Reg::X5, 1);
        a.cmp_r(true, Reg::X5, Reg::X2);
        a.b_cond(Cond::Lo, lp);
        a.bind(done);
        a.movz(false, SGP1, 0, 0);
        a.ret();
        a.bind(oob);
        a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
        a.ret();
        a.finish()?;
        out.push((HELPER_TABLE_INIT, a));
    }

    // 10-12. Dynamic table helpers (`HELPER_TABLE_COPY`, `HELPER_TABLE_GROW`, `HELPER_TABLE_FILL`).
    if !layout.dyn_tables.is_empty() {
        // HELPER_TABLE_COPY: X0 = dst, X1 = src, X2 = len, X3 = dst_desc_off, X4 = src_desc_off -> W16.
        {
            let mut a = A64Asm::new();
            let oob = a.new_label();
            let done = a.new_label();
            let bwd = a.new_label();
            let fwd_lp = a.new_label();
            let bwd_lp = a.new_label();
            a.add_r(true, Reg::X9, CTX_REG, Reg::X3);
            a.add_r(true, Reg::X10, CTX_REG, Reg::X4);
            a.ldr_uimm(false, Reg::X11, Reg::X9, 8);
            a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X11);
            a.b_cond(Cond::Hi, oob);
            a.ldr_uimm(false, Reg::X12, Reg::X10, 8);
            a.adds_r(true, Reg::X5, Reg::X1, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X12);
            a.b_cond(Cond::Hi, oob);
            a.cbz(true, Reg::X2, done);
            a.ldr_uimm(true, Reg::X11, Reg::X9, 0);
            a.lsl_imm(true, Reg::X5, Reg::X0, 3);
            a.add_r(true, Reg::X0, Reg::X11, Reg::X5);
            a.ldr_uimm(true, Reg::X12, Reg::X10, 0);
            a.lsl_imm(true, Reg::X5, Reg::X1, 3);
            a.add_r(true, Reg::X1, Reg::X12, Reg::X5);
            a.cmp_r(true, Reg::X0, Reg::X1);
            a.b_cond(Cond::Hi, bwd);
            a.movz(true, Reg::X5, 0, 0);
            a.bind(fwd_lp);
            a.ldr_reg_scaled(true, Reg::X6, Reg::X1, Reg::X5, true);
            a.str_reg_scaled(true, Reg::X6, Reg::X0, Reg::X5, true);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.cmp_r(true, Reg::X5, Reg::X2);
            a.b_cond(Cond::Lo, fwd_lp);
            a.b(done);
            a.bind(bwd);
            a.bind(bwd_lp);
            a.sub_imm(true, Reg::X2, Reg::X2, 1);
            a.ldr_reg_scaled(true, Reg::X6, Reg::X1, Reg::X2, true);
            a.str_reg_scaled(true, Reg::X6, Reg::X0, Reg::X2, true);
            a.cbnz(true, Reg::X2, bwd_lp);
            a.bind(done);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(oob);
            a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
            a.ret();
            a.finish()?;
            out.push((HELPER_TABLE_COPY, a));
        }
        // HELPER_TABLE_GROW: X0 = init_val, X1 = delta, X2 = tbl_desc_off -> X0 = old_len | 0xffff_ffff.
        {
            let mut a = A64Asm::new();
            let fail = a.new_label();
            let done = a.new_label();
            let lp = a.new_label();
            a.add_r(true, Reg::X9, CTX_REG, Reg::X2);
            a.ldr_uimm(false, Reg::X10, Reg::X9, 8); // old_len
            a.ldr_uimm(false, Reg::X11, Reg::X9, 12); // max_len
            a.adds_r(true, Reg::X12, Reg::X10, Reg::X1);
            a.b_cond(Cond::Hs, fail);
            a.cmp_r(true, Reg::X12, Reg::X11);
            a.b_cond(Cond::Hi, fail);
            a.cbz(true, Reg::X1, done);
            a.ldr_uimm(true, Reg::X11, Reg::X9, 0); // ptr
            a.mov_r(true, Reg::X5, Reg::X10);
            a.bind(lp);
            a.str_reg_scaled(true, Reg::X0, Reg::X11, Reg::X5, true);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.cmp_r(true, Reg::X5, Reg::X12);
            a.b_cond(Cond::Lo, lp);
            a.str_uimm(false, Reg::X12, Reg::X9, 8);
            a.bind(done);
            a.mov_r(true, Reg::X0, Reg::X10);
            a.ret();
            a.bind(fail);
            a.mov_imm64(Reg::X0, u64::MAX);
            a.ret();
            a.finish()?;
            out.push((HELPER_TABLE_GROW, a));
        }
        // HELPER_TABLE_FILL: X0 = dst, X1 = val, X2 = len, X3 = tbl_desc_off -> W16.
        {
            let mut a = A64Asm::new();
            let oob = a.new_label();
            let done = a.new_label();
            let lp = a.new_label();
            a.add_r(true, Reg::X9, CTX_REG, Reg::X3);
            a.ldr_uimm(false, Reg::X10, Reg::X9, 8);
            a.adds_r(true, Reg::X5, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X5, Reg::X10);
            a.b_cond(Cond::Hi, oob);
            a.cbz(true, Reg::X2, done);
            a.ldr_uimm(true, Reg::X10, Reg::X9, 0);
            a.lsl_imm(true, Reg::X5, Reg::X0, 3);
            a.add_r(true, Reg::X0, Reg::X10, Reg::X5);
            a.movz(true, Reg::X5, 0, 0);
            a.bind(lp);
            a.str_reg_scaled(true, Reg::X1, Reg::X0, Reg::X5, true);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.cmp_r(true, Reg::X5, Reg::X2);
            a.b_cond(Cond::Lo, lp);
            a.bind(done);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(oob);
            a.mov_imm32(SGP1, u32::from(TRAP_TABLE_OOB));
            a.ret();
            a.finish()?;
            out.push((HELPER_TABLE_FILL, a));
        }
    }

    Ok(out)
}

fn emit_a64_pinned_load(a: &mut A64Asm, module: &X64Module, layout: &CtxLayout) {
    if let Some(pg) = layout.pinned_global {
        let off = layout.global_offs[pg as usize];
        let ty = match module.globals[pg as usize].0 {
            ValType::I64 | ValType::Ref(_) => Ty::I64,
            _ => Ty::I32,
        };
        emit_load_off(a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
    }
}

fn emit_a64_pinned_store(a: &mut A64Asm, module: &X64Module, layout: &CtxLayout) {
    if let Some(pg) = layout.pinned_global {
        let off = layout.global_offs[pg as usize];
        let ty = match module.globals[pg as usize].0 {
            ValType::I64 | ValType::Ref(_) => Ty::I64,
            _ => Ty::I32,
        };
        emit_store_off(a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
    }
}

fn find_fixed_export(module: &X64Module, name: &str, expect_param: Option<ValType>) -> Option<u32> {
    let (_, fi) = module.exported_funcs.iter().find(|(n, _)| n == name)?;
    let fty = module.func_type(*fi)?;
    let ok_p = match expect_param {
        Some(p) => fty.params.as_slice() == [p],
        None => fty.params.is_empty(),
    };
    if ok_p && fty.results.as_slice() == [ValType::I32] {
        u32::try_from(*fi).ok()
    } else {
        None
    }
}

/// Shared host callback trampoline `HELPER_HOST_CALL(W0 = idx, X1 = tok, X2 = arg_bits)`.
/// Runs on guest stack, switches back to `ctx->host_rsp`, invokes
/// `ctx->host_fn(user_ctx, &ac, idx, tok, arg_bits, &res)`, switches back to guest stack,
/// reloads `MEM_LEN_REG` and `PINNED_GLOB_REG`, and returns `X0 = res`, `W16 = 0 | TRAP_HOST`.
pub fn emit_a64_helper_host_call(module: &X64Module, layout: &CtxLayout) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let null_host = a.new_label();
    let fail_host = a.new_label();

    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_HOST_FN,
        SGP3,
    );
    a.cbz(true, Reg::X9, null_host);

    // Guest stack link frame (160 B, 16-byte aligned):
    //   [SP + 0..16]   = FP, LR
    //   [SP + 16]      = prev_active_guest_rsp
    //   [SP + 24]      = prev_host_rsp
    //   [SP + 32..160] = Q8..Q15 (full 128-bit callee-saved SIMD registers; AAPCS64 only preserves D8..D15)
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -160);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X10),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X11),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.stp_x(Reg::X10, Reg::X11, Reg::SP, 16);
    a.stp_q(VReg::V8, VReg::V9, Reg::SP, 32);
    a.stp_q(VReg::V10, VReg::V11, Reg::SP, 64);
    a.stp_q(VReg::V12, VReg::V13, Reg::SP, 96);
    a.stp_q(VReg::V14, VReg::V15, Reg::SP, 128);
    a.mov_r(true, Reg::X10, Reg::SP);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X10),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    emit_a64_pinned_store(&mut a, module, layout);

    // Marshal AAPCS64 args 2, 3, 4 before switching stacks: W2 = idx, X3 = tok, X4 = arg_bits.
    a.mov_r(true, Reg::X4, Reg::X2);
    a.mov_r(true, Reg::X3, Reg::X1);
    a.uxtw(Reg::X2, Reg::X0);

    // Switch to outer host stack and allocate 32-byte ActiveCall + res frame.
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X10),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.mov_r(true, Reg::SP, Reg::X10);
    a.sub_imm(true, Reg::SP, Reg::SP, 32);
    a.stp_x(CTX_REG, MEM_BASE_REG, Reg::SP, 0); // ac.ctx, ac.mem
    a.str_uimm(true, Reg::XZR, Reg::SP, 16); // u64 res = 0

    // Finish AAPCS64 6-arg setup: X0 = user_ctx, X1 = &ac, X5 = &res.
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X0),
        None,
        CTX_REG,
        OFF_HOST_USER_CTX,
        SGP3,
    );
    a.add_imm(true, Reg::X1, Reg::SP, 0);
    a.add_imm(true, Reg::X5, Reg::SP, 16);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_HOST_FN,
        SGP3,
    );
    a.blr(Reg::X9);

    // Capture status (`W10 = W0`) and `res` (`X11 = [SP + 16]`) before switching back to guest stack.
    a.mov_r(false, Reg::X10, Reg::X0);
    a.ldr_uimm(true, Reg::X11, Reg::SP, 16);

    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X12),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    a.mov_r(true, Reg::SP, Reg::X12);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(MEM_LEN_REG),
        None,
        CTX_REG,
        OFF_MEM_LEN,
        SGP3,
    );
    emit_a64_pinned_load(&mut a, module, layout);

    a.ldp_q(VReg::V8, VReg::V9, Reg::SP, 32);
    a.ldp_q(VReg::V10, VReg::V11, Reg::SP, 64);
    a.ldp_q(VReg::V12, VReg::V13, Reg::SP, 96);
    a.ldp_q(VReg::V14, VReg::V15, Reg::SP, 128);
    a.ldp_x(Reg::X12, Reg::X13, Reg::SP, 16);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X12),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X13),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 160);

    a.cbnz(false, Reg::X10, fail_host);
    a.mov_r(true, Reg::X0, Reg::X11);
    a.movz(false, SGP1, 0, 0);
    a.ret();

    a.bind(null_host);
    a.bind(fail_host);
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm32(SGP1, u32::from(TRAP_HOST));
    a.ret();

    a.finish()?;
    Ok(a)
}

/// Emits internal AArch64 Wasm ABI trampoline for Blot cdylib import `k`.
pub fn emit_a64_blot_import_trampoline(module: &X64Module, k: usize) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let ty = module
        .func_type(k)
        .ok_or("aarch64: import type index out of range")?;

    match ty.params.as_slice() {
        [] => {
            a.movz(true, Reg::X1, 0, 0);
            a.movz(true, Reg::X2, 0, 0);
        }
        [ValType::Ref(_)] => {
            a.mov_r(true, Reg::X1, Reg::X0);
            a.movz(true, Reg::X2, 0, 0);
        }
        [ValType::I32] => {
            a.uxtw(Reg::X2, Reg::X0);
            a.movz(true, Reg::X1, 0, 0);
        }
        [ValType::I64] => {
            a.mov_r(true, Reg::X2, Reg::X0);
            a.movz(true, Reg::X1, 0, 0);
        }
        [ValType::F32] => {
            a.fmov_gp_from_fp(false, Reg::X2, VReg::V0);
            a.uxtw(Reg::X2, Reg::X2);
            a.movz(true, Reg::X1, 0, 0);
        }
        [ValType::F64] => {
            a.fmov_gp_from_fp(true, Reg::X2, VReg::V0);
            a.movz(true, Reg::X1, 0, 0);
        }
        [ValType::Ref(_), ValType::I32] => {
            a.uxtw(Reg::X2, Reg::X1);
            a.mov_r(true, Reg::X1, Reg::X0);
        }
        [ValType::Ref(_), ValType::I64] | [ValType::Ref(_), ValType::Ref(_)] => {
            a.mov_r(true, Reg::X2, Reg::X1);
            a.mov_r(true, Reg::X1, Reg::X0);
        }
        [ValType::Ref(_), ValType::F32] => {
            a.fmov_gp_from_fp(false, Reg::X2, VReg::V0);
            a.uxtw(Reg::X2, Reg::X2);
            a.mov_r(true, Reg::X1, Reg::X0);
        }
        [ValType::Ref(_), ValType::F64] => {
            a.fmov_gp_from_fp(true, Reg::X2, VReg::V0);
            a.mov_r(true, Reg::X1, Reg::X0);
        }
        other => return Err(format!("aarch64: unsupported import {k} params {other:?}")),
    }

    a.mov_imm32(
        Reg::X0,
        u32::try_from(k).map_err(|_| "aarch64: import index overflow")?,
    );
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
    a.bl_reloc(A64RelocTarget::Helper(HELPER_HOST_CALL));

    match ty.results.as_slice() {
        [] | [ValType::I64] | [ValType::Ref(_)] => {}
        [ValType::I32] => a.uxtw(Reg::X0, Reg::X0),
        [ValType::F32] => a.fmov_fp_from_gp(false, VReg::V0, Reg::X0),
        [ValType::F64] => a.fmov_fp_from_gp(true, VReg::V0, Reg::X0),
        other => return Err(format!("aarch64: unsupported import {k} results {other:?}")),
    }

    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Shared leaf helper `HELPER_READ_GLOBAL(X0 = ctx, X1 = np, X2 = nl, X3 = val_out, X4 = kind_out) -> W0`.
/// Clobbers ONLY caller-saved registers `X5..X14` so both `blot_aot_get_global` and
/// `blot_aot_active_get_global` can tail-jump directly into it.
pub fn emit_a64_helper_read_global(tables: &RuntimeTables) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    if tables.get_global_count == 0 {
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        return Ok(a);
    }

    let lbl_np_ok = a.new_label();
    let lbl_missing = a.new_label();
    let lbl_loop = a.new_label();
    let lbl_cmp = a.new_label();
    let lbl_next = a.new_label();
    let lbl_found = a.new_label();
    let lbl_live = a.new_label();
    let lbl_load32 = a.new_label();
    let lbl_load64 = a.new_label();
    let lbl_have_val = a.new_label();
    let lbl_skip_val = a.new_label();
    let lbl_skip_kind = a.new_label();

    a.cbz(true, Reg::X2, lbl_np_ok);
    a.cbz(true, Reg::X1, lbl_missing);
    a.bind(lbl_np_ok);

    // Table entry (24 B): +0: i32 name_rel, +4: u32 name_len, +8: u8 kind, +9: u8 is_const,
    // +12: i32 ctx_off, +16: u64 const_bits.
    a.adr_reloc(Reg::X5, A64RelocTarget::Rodata(tables.get_global_table_off));
    a.mov_r(true, Reg::X6, Reg::X5);
    a.mov_imm32(Reg::X7, tables.get_global_count);

    a.bind(lbl_loop);
    a.ldr_uimm(false, Reg::X8, Reg::X6, 4); // name_len (zero-ext to 64-bit)
    a.cmp_r(true, Reg::X8, Reg::X2);
    a.b_cond(Cond::Ne, lbl_next);
    a.cbz(true, Reg::X2, lbl_found);

    a.ldrsw_uimm(Reg::X9, Reg::X6, 0);
    a.add_r(true, Reg::X9, Reg::X5, Reg::X9);
    a.movz(true, Reg::X10, 0, 0);
    a.bind(lbl_cmp);
    a.ldrb_reg(Reg::X11, Reg::X1, Reg::X10, false);
    a.ldrb_reg(Reg::X12, Reg::X9, Reg::X10, false);
    a.cmp_r(false, Reg::X11, Reg::X12);
    a.b_cond(Cond::Ne, lbl_next);
    a.add_imm(true, Reg::X10, Reg::X10, 1);
    a.cmp_r(true, Reg::X10, Reg::X2);
    a.b_cond(Cond::Lo, lbl_cmp);
    a.b(lbl_found);

    a.bind(lbl_next);
    a.add_imm(true, Reg::X6, Reg::X6, 24);
    a.sub_imm(false, Reg::X7, Reg::X7, 1);
    a.cbnz(false, Reg::X7, lbl_loop);

    a.bind(lbl_missing);
    a.mov_imm32(Reg::X0, 1);
    a.ret();

    a.bind(lbl_found);
    a.ldrb_uimm(Reg::X8, Reg::X6, 8); // kind
    a.ldrb_uimm(Reg::X9, Reg::X6, 9); // is_const
    a.cbz(false, Reg::X9, lbl_live);
    a.ldr_uimm(true, Reg::X10, Reg::X6, 16); // const_bits
    a.b(lbl_have_val);

    a.bind(lbl_live);
    a.ldrsw_uimm(Reg::X11, Reg::X6, 12); // ctx_off
    a.cbz(false, Reg::X8, lbl_load32);
    a.cmp_imm(false, Reg::X8, 2);
    a.b_cond(Cond::Ne, lbl_load64);
    a.bind(lbl_load32);
    a.ldr_reg(false, Reg::X10, Reg::X0, Reg::X11, false);
    a.b(lbl_have_val);
    a.bind(lbl_load64);
    a.ldr_reg(true, Reg::X10, Reg::X0, Reg::X11, false);

    a.bind(lbl_have_val);
    a.cbz(true, Reg::X3, lbl_skip_val);
    a.str_uimm(true, Reg::X10, Reg::X3, 0);
    a.bind(lbl_skip_val);
    a.cbz(true, Reg::X4, lbl_skip_kind);
    a.strb_uimm(Reg::X8, Reg::X4, 0);
    a.bind(lbl_skip_kind);
    a.movz(false, Reg::X0, 0, 0);
    a.ret();

    a.finish()?;
    Ok(a)
}

/// Emits top-level `blot_aot_call_allocate`, `blot_aot_call_reset`, or `blot_aot_call_finish_invocation`.
fn emit_a64_export_call_fixed(
    module: &X64Module,
    layout: &CtxLayout,
    export_name: &str,
    has_arg: bool,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let expect_param = if has_arg { Some(ValType::I32) } else { None };
    let Some(target_fn) = find_fixed_export(module, export_name, expect_param) else {
        a.mov_imm32(Reg::X0, 3);
        a.ret();
        a.finish()?;
        return Ok(a);
    };

    // AAPCS64 args:
    //   has_arg=true : (h: X0, arg: W1, res_out: X2, trap_out: X3)
    //   has_arg=false: (h: X0, res_out: X1, trap_out: X2)
    let res_reg = if has_arg { Reg::X2 } else { Reg::X1 };
    let trap_reg = if has_arg { Reg::X3 } else { Reg::X2 };
    let lbl_fail_host = a.new_label();
    let lbl_fail_host_rc = a.new_label();
    let lbl_trapped = a.new_label();
    let lbl_not_exn = a.new_label();
    let lbl_skip_res = a.new_label();
    let lbl_skip_trap = a.new_label();
    let lbl_ep = a.new_label();

    a.cbz(true, Reg::X0, lbl_fail_host);
    a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
    a.cbnz(false, Reg::X9, lbl_fail_host);

    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -96);
    a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.stp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.stp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.stp_x(Reg::X25, Reg::X26, Reg::SP, 64);
    a.stp_x(Reg::X27, Reg::X28, Reg::SP, 80);

    a.mov_r(true, CTX_REG, Reg::X0);
    a.mov_r(true, Reg::X19, res_reg);
    a.mov_r(true, Reg::X20, trap_reg);
    if has_arg {
        a.uxtw(Reg::X21, Reg::X1);
    }

    emit_load_off(
        &mut a,
        Ty::I64,
        Some(MEM_BASE_REG),
        None,
        CTX_REG,
        OFF_MEM_BASE,
        SGP3,
    );
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(MEM_LEN_REG),
        None,
        CTX_REG,
        OFF_MEM_LEN,
        SGP3,
    );
    emit_a64_pinned_load(&mut a, module, layout);

    a.mov_imm32(Reg::X9, 1);
    a.strb_uimm(Reg::X9, CTX_REG, OFF_IN_CALL as u32);
    a.mov_r(true, SGP2, Reg::SP);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        CTX_REG,
        OFF_GUEST_STACK_TOP,
        SGP3,
    );
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    if has_arg {
        a.mov_r(true, Reg::X0, Reg::X21);
    }
    a.mov_r(true, Reg::SP, SGP2);
    a.bl_reloc(A64RelocTarget::Func(target_fn));

    emit_load_off(
        &mut a,
        Ty::I64,
        Some(SGP2),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.mov_r(true, Reg::SP, SGP2);
    emit_a64_pinned_store(&mut a, module, layout);
    a.strb_uimm(Reg::XZR, CTX_REG, OFF_IN_CALL as u32);

    a.cbnz(false, SGP1, lbl_trapped);
    a.cbz(true, Reg::X19, lbl_skip_res);
    a.str_uimm(false, Reg::X0, Reg::X19, 0);
    a.bind(lbl_skip_res);
    a.movz(false, Reg::X0, 0, 0);
    a.b(lbl_ep);

    a.bind(lbl_trapped);
    a.cmp_imm(false, SGP1, u16::from(INTERNAL_EXN_SIGNAL));
    a.b_cond(Cond::Ne, lbl_not_exn);
    a.mov_imm32(SGP1, u32::from(TRAP_UNREACHABLE));
    a.bind(lbl_not_exn);
    a.cbz(true, Reg::X20, lbl_skip_trap);
    a.strb_uimm(SGP1, Reg::X20, 0);
    a.bind(lbl_skip_trap);
    a.mov_imm32(Reg::X0, 1);
    a.mov_imm32(Reg::X9, 2);
    a.cmp_imm(false, SGP1, u16::from(TRAP_HOST));
    a.csel(false, Reg::X0, Reg::X9, Reg::X0, Cond::Eq);

    a.bind(lbl_ep);
    a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.ldp_x(Reg::X25, Reg::X26, Reg::SP, 64);
    a.ldp_x(Reg::X27, Reg::X28, Reg::SP, 80);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 96);
    a.ret();

    a.bind(lbl_fail_host);
    a.cbz(true, trap_reg, lbl_fail_host_rc);
    a.mov_imm32(Reg::X9, u32::from(TRAP_HOST));
    a.strb_uimm(Reg::X9, trap_reg, 0);
    a.bind(lbl_fail_host_rc);
    a.mov_imm32(Reg::X0, 2);
    a.ret();

    a.finish()?;
    Ok(a)
}

/// Emits callback-reentrant `blot_aot_active_allocate` or `blot_aot_active_reset`.
fn emit_a64_export_active_fixed(
    module: &X64Module,
    layout: &CtxLayout,
    export_name: &str,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let Some(target_fn) = find_fixed_export(module, export_name, Some(ValType::I32)) else {
        a.mov_imm32(Reg::X0, 3);
        a.ret();
        a.finish()?;
        return Ok(a);
    };

    // AAPCS64 args: (ac: X0, arg: W1, res_out: X2, trap_out: X3)
    let lbl_fail_host = a.new_label();
    let lbl_fail_host_rc = a.new_label();
    let lbl_fail_stack = a.new_label();
    let lbl_fail_stack_rc = a.new_label();
    let lbl_trapped = a.new_label();
    let lbl_not_exn = a.new_label();
    let lbl_skip_res = a.new_label();
    let lbl_skip_trap = a.new_label();
    let lbl_ep = a.new_label();

    a.cbz(true, Reg::X0, lbl_fail_host);
    a.ldr_uimm(true, Reg::X8, Reg::X0, 0); // ac->ctx
    a.cbz(true, Reg::X8, lbl_fail_host);
    a.ldrb_uimm(Reg::X9, Reg::X8, OFF_IN_CALL as u32);
    a.cbz(false, Reg::X9, lbl_fail_host);

    // Check active_depth < MAX_ACTIVE_REENTRY_DEPTH && suspended in [stack_floor+64, guest_stack_top-16]
    a.ldr_uimm(false, Reg::X9, Reg::X8, OFF_ACTIVE_DEPTH as u32);
    a.cmp_imm(false, Reg::X9, MAX_ACTIVE_REENTRY_DEPTH as u16);
    a.b_cond(Cond::Hs, lbl_fail_stack);

    a.ldr_uimm(true, Reg::X9, Reg::X8, OFF_ACTIVE_GUEST_RSP as u32);
    a.ldr_uimm(true, Reg::X10, Reg::X8, OFF_STACK_FLOOR as u32);
    a.add_imm(true, Reg::X10, Reg::X10, 64);
    a.cmp_r(true, Reg::X9, Reg::X10);
    a.b_cond(Cond::Lo, lbl_fail_stack);

    a.ldr_uimm(true, Reg::X10, Reg::X8, OFF_GUEST_STACK_TOP as u32);
    a.sub_imm(true, Reg::X10, Reg::X10, 16);
    a.cmp_r(true, Reg::X9, Reg::X10);
    a.b_cond(Cond::Hi, lbl_fail_stack);

    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -96);
    a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.stp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.stp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.stp_x(Reg::X25, Reg::X26, Reg::SP, 64);
    a.stp_x(Reg::X27, Reg::X28, Reg::SP, 80);

    a.mov_r(true, Reg::X19, Reg::X2); // res_out
    a.mov_r(true, Reg::X20, Reg::X3); // trap_out
    a.uxtw(Reg::X21, Reg::X1); // arg
    a.ldr_uimm(true, CTX_REG, Reg::X0, 0); // ac->ctx
    a.ldr_uimm(true, MEM_BASE_REG, Reg::X0, 8); // ac->mem
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(MEM_LEN_REG),
        None,
        CTX_REG,
        OFF_MEM_LEN,
        SGP3,
    );
    emit_a64_pinned_load(&mut a, module, layout);

    // new_guest_sp = (suspended & !15) - 32
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X22),
        None,
        CTX_REG,
        OFF_ACTIVE_GUEST_RSP,
        SGP3,
    );
    a.mov_imm64(Reg::X9, !15u64);
    a.and_r(true, Reg::X22, Reg::X22, Reg::X9);
    a.sub_imm(true, Reg::X22, Reg::X22, 32);

    // Link record at [X22 + 0..16]: [0] = prev ctx->host_rsp, [8] = current host SP.
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.mov_r(true, Reg::X10, Reg::SP);
    a.stp_x(Reg::X9, Reg::X10, Reg::X22, 0);

    // active_depth += 1; ctx->host_rsp = current host SP.
    emit_load_off(
        &mut a,
        Ty::I32,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_ACTIVE_DEPTH,
        SGP3,
    );
    a.add_imm(false, Reg::X9, Reg::X9, 1);
    emit_store_off(
        &mut a,
        Ty::I32,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_ACTIVE_DEPTH,
        SGP3,
    );
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X10),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );

    a.mov_r(true, Reg::X0, Reg::X21);
    a.mov_r(true, Reg::SP, Reg::X22);
    a.bl_reloc(A64RelocTarget::Func(target_fn));

    // Restore ctx->host_rsp, host SP, active_depth -= 1, pinned global (preserving X0 & W16!).
    a.ldp_x(Reg::X9, Reg::X10, Reg::SP, 0);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_HOST_RSP,
        SGP3,
    );
    a.mov_r(true, Reg::SP, Reg::X10);
    emit_load_off(
        &mut a,
        Ty::I32,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_ACTIVE_DEPTH,
        SGP3,
    );
    a.sub_imm(false, Reg::X9, Reg::X9, 1);
    emit_store_off(
        &mut a,
        Ty::I32,
        Some(Reg::X9),
        None,
        CTX_REG,
        OFF_ACTIVE_DEPTH,
        SGP3,
    );
    emit_a64_pinned_store(&mut a, module, layout);

    a.cbnz(false, SGP1, lbl_trapped);
    a.cbz(true, Reg::X19, lbl_skip_res);
    a.str_uimm(false, Reg::X0, Reg::X19, 0);
    a.bind(lbl_skip_res);
    a.movz(false, Reg::X0, 0, 0);
    a.b(lbl_ep);

    a.bind(lbl_trapped);
    a.cmp_imm(false, SGP1, u16::from(INTERNAL_EXN_SIGNAL));
    a.b_cond(Cond::Ne, lbl_not_exn);
    a.mov_imm32(SGP1, u32::from(TRAP_UNREACHABLE));
    a.bind(lbl_not_exn);
    a.cbz(true, Reg::X20, lbl_skip_trap);
    a.strb_uimm(SGP1, Reg::X20, 0);
    a.bind(lbl_skip_trap);
    a.mov_imm32(Reg::X0, 1);
    a.mov_imm32(Reg::X9, 2);
    a.cmp_imm(false, SGP1, u16::from(TRAP_HOST));
    a.csel(false, Reg::X0, Reg::X9, Reg::X0, Cond::Eq);

    a.bind(lbl_ep);
    a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.ldp_x(Reg::X25, Reg::X26, Reg::SP, 64);
    a.ldp_x(Reg::X27, Reg::X28, Reg::SP, 80);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 96);
    a.ret();

    a.bind(lbl_fail_host);
    a.cbz(true, Reg::X3, lbl_fail_host_rc);
    a.mov_imm32(Reg::X9, u32::from(TRAP_HOST));
    a.strb_uimm(Reg::X9, Reg::X3, 0);
    a.bind(lbl_fail_host_rc);
    a.mov_imm32(Reg::X0, 2);
    a.ret();

    a.bind(lbl_fail_stack);
    a.cbz(true, Reg::X3, lbl_fail_stack_rc);
    a.mov_imm32(Reg::X9, u32::from(TRAP_CALL_STACK));
    a.strb_uimm(Reg::X9, Reg::X3, 0);
    a.bind(lbl_fail_stack_rc);
    a.mov_imm32(Reg::X0, 1);
    a.ret();

    a.finish()?;
    Ok(a)
}

/// Emits Blot ABI 8 exported C functions on AArch64 (`EXPORT_NAMES` parity with x64/C/Rust).
pub fn emit_a64_exports(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Vec<(&'static str, A64Asm)>, String> {
    let mut out = Vec::with_capacity(EXPORT_NAMES.len());

    // 1. blot_aot_abi_version() -> 1 (Library Protocol v1 / Blot Guest ABI 8)
    {
        let mut a = A64Asm::new();
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_abi_version", a));
    }
    // 2. blot_aot_trap_message(code: u8, out_ptr: *mut *const u8, out_len: *mut usize) -> i32
    {
        let mut a = A64Asm::new();
        let bad = a.new_label();
        let skip_p = a.new_label();
        let skip_l = a.new_label();
        a.uxtb(Reg::X0, Reg::X0);
        a.sub_imm(false, Reg::X0, Reg::X0, 1);
        a.cmp_imm(
            false,
            Reg::X0,
            crate::x64::runtime::TRAP_MESSAGES.len() as u16,
        );
        a.b_cond(Cond::Hs, bad);
        a.adr_reloc(Reg::X3, A64RelocTarget::Rodata(tables.trap_table_off));
        a.uxtw(Reg::X4, Reg::X0);
        a.lsl_imm(true, Reg::X4, Reg::X4, 3);
        a.add_r(true, Reg::X4, Reg::X3, Reg::X4);
        a.ldrsw_uimm(Reg::X5, Reg::X4, 0);
        a.add_r(true, Reg::X5, Reg::X3, Reg::X5);
        a.ldr_uimm(false, Reg::X6, Reg::X4, 4);
        a.cbz(true, Reg::X1, skip_p);
        a.str_uimm(true, Reg::X5, Reg::X1, 0);
        a.bind(skip_p);
        a.cbz(true, Reg::X2, skip_l);
        a.str_uimm(true, Reg::X6, Reg::X2, 0);
        a.bind(skip_l);
        a.movz(false, Reg::X0, 0, 0);
        a.ret();
        a.bind(bad);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_trap_message", a));
    }
    // 3. blot_aot_import_count() -> u32
    {
        let mut a = A64Asm::new();
        a.mov_imm32(Reg::X0, tables.imp_count);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_import_count", a));
    }
    // 4. blot_aot_import_name(idx, mod_ptr, mod_len, name_ptr, name_len) -> i32
    {
        let mut a = A64Asm::new();
        if tables.imp_count == 0 {
            a.mov_imm32(Reg::X0, 1);
            a.ret();
        } else {
            let bad = a.new_label();
            let s1 = a.new_label();
            let s2 = a.new_label();
            let s3 = a.new_label();
            let s4 = a.new_label();
            a.uxtw(Reg::X5, Reg::X0);
            a.mov_imm64(Reg::X6, u64::from(tables.imp_count));
            a.cmp_r(true, Reg::X5, Reg::X6);
            a.b_cond(Cond::Hs, bad);
            a.adr_reloc(Reg::X7, A64RelocTarget::Rodata(tables.imp_table_off));
            a.lsl_imm(true, Reg::X5, Reg::X5, 4);
            a.add_r(true, Reg::X5, Reg::X7, Reg::X5);

            a.cbz(true, Reg::X1, s1);
            a.ldrsw_uimm(Reg::X8, Reg::X5, 0);
            a.add_r(true, Reg::X8, Reg::X7, Reg::X8);
            a.str_uimm(true, Reg::X8, Reg::X1, 0);
            a.bind(s1);

            a.cbz(true, Reg::X2, s2);
            a.ldr_uimm(false, Reg::X8, Reg::X5, 4);
            a.str_uimm(true, Reg::X8, Reg::X2, 0);
            a.bind(s2);

            a.cbz(true, Reg::X3, s3);
            a.ldrsw_uimm(Reg::X8, Reg::X5, 8);
            a.add_r(true, Reg::X8, Reg::X7, Reg::X8);
            a.str_uimm(true, Reg::X8, Reg::X3, 0);
            a.bind(s3);

            a.cbz(true, Reg::X4, s4);
            a.ldr_uimm(false, Reg::X8, Reg::X5, 12);
            a.str_uimm(true, Reg::X8, Reg::X4, 0);
            a.bind(s4);

            a.movz(false, Reg::X0, 0, 0);
            a.ret();
            a.bind(bad);
            a.mov_imm32(Reg::X0, 1);
            a.ret();
        }
        a.finish()?;
        out.push(("blot_aot_import_name", a));
    }
    // 5. blot_aot_new(host_ctx, host_fn, mem_limit_pages, out_trap) -> *mut Ctx
    {
        let mut a = A64Asm::new();
        let fail_mem_limit = a.new_label();
        let fail_mem_too_large = a.new_label();
        let fail_alloc_no_ctx = a.new_label();
        let fail_alloc_cleanup = a.new_label();
        let fail_trap_cleanup = a.new_label();
        let fail_no_ctx = a.new_label();
        let skip_mem = a.new_label();
        let done_ok = a.new_label();
        // Save callee-saved regs X19..X28, FP, LR (12 regs = 96 B, 16-byte aligned).
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -96);
        a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
        a.stp_x(Reg::X21, Reg::X22, Reg::SP, 32);
        a.stp_x(Reg::X23, Reg::X24, Reg::SP, 48);
        a.stp_x(Reg::X25, Reg::X26, Reg::SP, 64);
        a.stp_x(Reg::X27, Reg::X28, Reg::SP, 80);

        a.mov_r(true, Reg::X19, Reg::X0); // host_ctx
        a.mov_r(true, Reg::X20, Reg::X1); // host_fn
        a.mov_r(true, Reg::X21, Reg::X2); // mem_limit_pages (0 = default)
        a.mov_r(true, Reg::X22, Reg::X3); // out_trap

        let (min_pages, declared_max) = module.memory.unwrap_or((0, 0));
        let mem0_hard_cap = if module.memory64 { 1u64 << 28 } else { 65_536 };
        if module.memory.is_some() {
            let have_lim = a.new_label();
            let default_lim = if module.memory64 {
                min_pages.max(131_072)
            } else {
                65_536
            };
            a.cbnz(true, Reg::X21, have_lim);
            a.mov_imm64(Reg::X21, default_lim);
            a.bind(have_lim);

            a.mov_imm64(Reg::X4, min_pages);
            a.cmp_r(true, Reg::X4, Reg::X21);
            a.b_cond(Cond::Hi, fail_mem_limit);

            a.mov_imm64(Reg::X23, declared_max.min(mem0_hard_cap));
            a.cmp_r(true, Reg::X21, Reg::X23);
            a.csel(true, Reg::X23, Reg::X21, Reg::X23, Cond::Lo);

            a.cmp_r(true, Reg::X4, Reg::X23);
            a.b_cond(Cond::Hi, fail_mem_too_large);
        } else {
            a.movz(true, Reg::X23, 0, 0);
        }
        for em in &module.extra_memories {
            let hard_cap = if em.memory64 { 1u64 << 28 } else { 65_536 };
            if em.min_pages > hard_cap {
                a.b(fail_mem_too_large);
            }
        }

        // mmap Ctx (`layout.total_bytes`)
        a.movz(true, Reg::X0, 0, 0);
        a.mov_imm64(Reg::X1, layout.total_bytes as u64);
        a.mov_imm32(Reg::X2, 3); // PROT_READ | PROT_WRITE
        a.mov_imm32(Reg::X3, 0x22); // MAP_PRIVATE | MAP_ANONYMOUS
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc_no_ctx);
        a.mov_r(true, CTX_REG, Reg::X0);
        a.mov_imm64(Reg::X4, layout.total_bytes as u64);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            OFF_HANDLE_ALLOC_BYTES,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X19),
            None,
            CTX_REG,
            OFF_HOST_USER_CTX,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X20),
            None,
            CTX_REG,
            OFF_HOST_FN,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X23),
            None,
            CTX_REG,
            OFF_MEM_MAX_PAGES,
            SGP3,
        );

        // mmap guest stack (`GUEST_STACK_TOTAL_BYTES`), mprotect usable region RW.
        a.movz(true, Reg::X0, 0, 0);
        a.mov_imm64(Reg::X1, GUEST_STACK_TOTAL_BYTES as u64);
        a.movz(true, Reg::X2, 0, 0); // PROT_NONE
        a.mov_imm32(Reg::X3, 0x4022); // MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc_cleanup);
        a.mov_r(true, Reg::X24, Reg::X0);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X24),
            None,
            CTX_REG,
            OFF_GUEST_STACK_BASE,
            SGP3,
        );
        a.mov_imm64(Reg::X1, GUEST_STACK_GUARD_BYTES as u64);
        a.add_r(true, Reg::X0, Reg::X24, Reg::X1);
        a.mov_imm64(Reg::X1, GUEST_STACK_USABLE_BYTES as u64);
        a.mov_imm32(Reg::X2, 3);
        emit_syscall(&mut a, SYS_MPROTECT);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc_cleanup);
        a.mov_imm64(Reg::X4, GUEST_STACK_TOTAL_BYTES as u64);
        a.add_r(true, Reg::X4, Reg::X24, Reg::X4);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            OFF_GUEST_STACK_TOP,
            SGP3,
        );
        a.mov_imm64(
            Reg::X4,
            (GUEST_STACK_GUARD_BYTES + GUEST_STACK_FLOOR_HEADROOM) as u64,
        );
        a.add_r(true, Reg::X4, Reg::X24, Reg::X4);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            OFF_STACK_FLOOR,
            SGP3,
        );

        // Linear memory reservation & initial pages commit.
        a.movz(true, MEM_BASE_REG, 0, 0);
        a.movz(true, MEM_LEN_REG, 0, 0);
        a.cbz(true, Reg::X23, skip_mem);
        a.lsl_imm(true, Reg::X1, Reg::X23, 16); // reserve_bytes
        a.mov_r(true, Reg::X19, Reg::X1);
        a.movz(true, Reg::X0, 0, 0);
        a.movz(true, Reg::X2, 0, 0); // PROT_NONE
        a.mov_imm32(Reg::X3, 0x4022);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc_cleanup);
        a.mov_r(true, MEM_BASE_REG, Reg::X0);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(MEM_BASE_REG),
            None,
            CTX_REG,
            OFF_MEM_BASE,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X19),
            None,
            CTX_REG,
            OFF_MEM_RESERVE_BYTES,
            SGP3,
        );
        if min_pages > 0 && min_pages <= mem0_hard_cap {
            a.mov_imm64(MEM_LEN_REG, min_pages << 16);
            a.mov_r(true, Reg::X0, MEM_BASE_REG);
            a.mov_r(true, Reg::X1, MEM_LEN_REG);
            a.mov_imm32(Reg::X2, 3);
            emit_syscall(&mut a, SYS_MPROTECT);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc_cleanup);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(MEM_LEN_REG),
                None,
                CTX_REG,
                OFF_MEM_LEN,
                SGP3,
            );
        }
        a.bind(skip_mem);

        // Additional linear memories `1..N` (`multi-memory`).
        for (mi, em) in module.extra_memories.iter().enumerate() {
            let desc_off = layout.extra_mem_offs[mi];
            let hard_cap = if em.memory64 { 1u64 << 28 } else { 65_536 };
            if em.min_pages > hard_cap {
                // Already rejected unconditionally before `mmap Ctx`; skip dead compile-time shift.
                continue;
            }
            let default_cap = if em.memory64 {
                em.min_pages.max(131_072).min(hard_cap)
            } else {
                65_536
            };
            let eff_max = em.max_pages.min(default_cap);
            if eff_max == 0 {
                continue;
            }
            a.mov_imm64(Reg::X4, eff_max);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                desc_off + 16,
                SGP3,
            );
            let reserve_bytes = eff_max << 16;
            a.mov_imm64(Reg::X1, reserve_bytes);
            a.mov_r(true, Reg::X19, Reg::X1);
            a.movz(true, Reg::X0, 0, 0);
            a.movz(true, Reg::X2, 0, 0);
            a.mov_imm32(Reg::X3, 0x4022);
            a.mov_imm64(Reg::X4, u64::MAX);
            a.movz(true, Reg::X5, 0, 0);
            emit_syscall(&mut a, SYS_MMAP);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc_cleanup);
            a.mov_r(true, Reg::X20, Reg::X0);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X20),
                None,
                CTX_REG,
                desc_off,
                SGP3,
            );
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X19),
                None,
                CTX_REG,
                desc_off + 24,
                SGP3,
            );
            if em.min_pages > 0 {
                let init_len = em.min_pages << 16;
                a.mov_r(true, Reg::X0, Reg::X20);
                a.mov_imm64(Reg::X1, init_len);
                a.mov_imm32(Reg::X2, 3);
                emit_syscall(&mut a, SYS_MPROTECT);
                a.mov_imm64(Reg::X4, (-4095i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X4);
                a.b_cond(Cond::Hs, fail_alloc_cleanup);
                a.mov_imm64(Reg::X4, init_len);
                emit_store_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X4),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
            }
        }

        // Exception arena slab (demand-paged RW, EH modules only).
        if let Some(exn_off) = layout.exn_state_off {
            a.movz(true, Reg::X0, 0, 0);
            a.mov_imm64(Reg::X1, EXN_SLAB_BYTES as u64);
            a.mov_imm32(Reg::X2, 3);
            a.mov_imm32(Reg::X3, 0x22);
            a.mov_imm64(Reg::X4, u64::MAX);
            a.movz(true, Reg::X5, 0, 0);
            emit_syscall(&mut a, SYS_MMAP);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc_cleanup);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                exn_off + 8,
                SGP3,
            );
            a.mov_imm64(Reg::X4, 16);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                exn_off + 16,
                SGP3,
            );
            a.mov_imm64(Reg::X4, EXN_SLAB_BYTES as u64);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                exn_off + 24,
                SGP3,
            );
        }

        // Dynamic tables (demand-paged RW mmap, allocated ONLY when `dyn_tables` non-empty).
        for &(desc_off, min_len, max_len) in layout.dyn_tables.values() {
            let reserve_bytes = if max_len > 0 {
                ((u64::from(max_len) * 8) + 4095) & !4095
            } else {
                0
            };
            if reserve_bytes > 0 {
                a.movz(true, Reg::X0, 0, 0);
                a.mov_imm64(Reg::X1, reserve_bytes);
                a.mov_imm32(Reg::X2, 3);
                a.mov_imm32(Reg::X3, 0x22);
                a.mov_imm64(Reg::X4, u64::MAX);
                a.movz(true, Reg::X5, 0, 0);
                emit_syscall(&mut a, SYS_MMAP);
                a.mov_imm64(Reg::X4, (-4095i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X4);
                a.b_cond(Cond::Hs, fail_alloc_cleanup);
                emit_store_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                a.mov_imm32(Reg::X4, min_len);
                emit_store_off(
                    &mut a,
                    Ty::I32,
                    Some(Reg::X4),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                a.mov_imm32(Reg::X4, max_len);
                emit_store_off(
                    &mut a,
                    Ty::I32,
                    Some(Reg::X4),
                    None,
                    CTX_REG,
                    desc_off + 12,
                    SGP3,
                );
                a.mov_imm64(Reg::X4, reserve_bytes);
                emit_store_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X4),
                    None,
                    CTX_REG,
                    desc_off + 16,
                    SGP3,
                );
            }
        }

        // Switch to dedicated guest stack and run `HELPER_INITIALIZE` (globals, data, start fn).
        a.mov_imm32(Reg::X4, 1);
        a.strb_uimm(Reg::X4, CTX_REG, OFF_IN_CALL as u32);
        a.mov_r(true, SGP2, Reg::SP);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_HOST_RSP,
            SGP3,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_GUEST_STACK_TOP,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_ACTIVE_GUEST_RSP,
            SGP3,
        );
        a.mov_r(true, Reg::SP, SGP2);
        a.bl_reloc(A64RelocTarget::Helper(HELPER_INITIALIZE));
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_HOST_RSP,
            SGP3,
        );
        a.mov_r(true, Reg::SP, SGP2);
        a.strb_uimm(Reg::XZR, CTX_REG, OFF_IN_CALL as u32);
        a.cbnz(false, SGP1, fail_trap_cleanup);
        a.mov_r(true, Reg::X0, CTX_REG);
        a.b(done_ok);

        // Pre-Ctx allocation failure paths (nothing mapped yet).
        a.bind(fail_mem_limit);
        a.mov_imm32(Reg::X25, u32::from(TRAP_MEM_LIMIT));
        a.b(fail_no_ctx);
        a.bind(fail_mem_too_large);
        a.mov_imm32(Reg::X25, u32::from(TRAP_MEM_TOO_LARGE));
        a.b(fail_no_ctx);
        a.bind(fail_alloc_no_ctx);
        a.mov_imm32(Reg::X25, u32::from(TRAP_ALLOC_FAILED));
        a.b(fail_no_ctx);

        // Post-Ctx failure paths: save mapped trap byte in callee-saved `X25`, then unmap every
        // allocated mapping (mirroring `blot_aot_drop`) before returning NULL.
        a.bind(fail_alloc_cleanup);
        a.mov_imm32(SGP1, u32::from(TRAP_ALLOC_FAILED));
        a.bind(fail_trap_cleanup);
        let not_exn_init = a.new_label();
        a.cmp_imm(false, SGP1, u16::from(INTERNAL_EXN_SIGNAL));
        a.b_cond(Cond::Ne, not_exn_init);
        a.mov_imm32(SGP1, u32::from(TRAP_UNREACHABLE));
        a.bind(not_exn_init);
        a.mov_r(false, Reg::X25, SGP1);

        let skip_cl_m = a.new_label();
        let skip_cl_s = a.new_label();
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            CTX_REG,
            OFF_MEM_BASE,
            Reg::X15,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            CTX_REG,
            OFF_MEM_RESERVE_BYTES,
            Reg::X15,
        );
        a.cbz(true, Reg::X0, skip_cl_m);
        emit_syscall(&mut a, SYS_MUNMAP);
        a.bind(skip_cl_m);
        for &desc_off in &layout.extra_mem_offs {
            let skip_em = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                desc_off,
                Reg::X15,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X1),
                None,
                CTX_REG,
                desc_off + 24,
                Reg::X15,
            );
            a.cbz(true, Reg::X1, skip_em);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_em);
        }
        if let Some(exn_off) = layout.exn_state_off {
            let skip_exn = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                exn_off + 8,
                Reg::X15,
            );
            a.cbz(true, Reg::X0, skip_exn);
            a.mov_imm64(Reg::X1, EXN_SLAB_BYTES as u64);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_exn);
        }
        for &(desc_off, _, _) in layout.dyn_tables.values() {
            let skip_dt = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                desc_off,
                Reg::X15,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X1),
                None,
                CTX_REG,
                desc_off + 16,
                Reg::X15,
            );
            a.cbz(true, Reg::X1, skip_dt);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_dt);
        }
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            CTX_REG,
            OFF_GUEST_STACK_BASE,
            Reg::X15,
        );
        a.cbz(true, Reg::X0, skip_cl_s);
        a.mov_imm64(Reg::X1, GUEST_STACK_TOTAL_BYTES as u64);
        emit_syscall(&mut a, SYS_MUNMAP);
        a.bind(skip_cl_s);
        // Load `OFF_HANDLE_ALLOC_BYTES` BEFORE unmapping `CTX_REG`!
        a.mov_r(true, Reg::X0, CTX_REG);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            CTX_REG,
            OFF_HANDLE_ALLOC_BYTES,
            Reg::X15,
        );
        emit_syscall(&mut a, SYS_MUNMAP);

        a.bind(fail_no_ctx);
        let skip_tr = a.new_label();
        a.cbz(true, Reg::X22, skip_tr);
        a.strb_uimm(Reg::X25, Reg::X22, 0);
        a.bind(skip_tr);
        a.movz(true, Reg::X0, 0, 0);

        a.bind(done_ok);
        a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
        a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 32);
        a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 48);
        a.ldp_x(Reg::X25, Reg::X26, Reg::SP, 64);
        a.ldp_x(Reg::X27, Reg::X28, Reg::SP, 80);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 96);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_new", a));
    }
    // 6. blot_aot_drop(ctx)
    {
        let mut a = A64Asm::new();
        let ret_now = a.new_label();
        let skip_m = a.new_label();
        let skip_s = a.new_label();
        a.cbz(true, Reg::X0, ret_now);
        a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
        a.cbnz(false, Reg::X9, ret_now);
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -32);
        a.str_uimm(true, Reg::X19, Reg::SP, 16);
        a.mov_r(true, Reg::X19, Reg::X0);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            Reg::X19,
            OFF_MEM_BASE,
            Reg::X15,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            Reg::X19,
            OFF_MEM_RESERVE_BYTES,
            Reg::X15,
        );
        a.cbz(true, Reg::X0, skip_m);
        emit_syscall(&mut a, SYS_MUNMAP);
        a.bind(skip_m);
        for &desc_off in &layout.extra_mem_offs {
            let skip_em = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                Reg::X19,
                desc_off,
                Reg::X15,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X1),
                None,
                Reg::X19,
                desc_off + 24,
                Reg::X15,
            );
            a.cbz(true, Reg::X1, skip_em);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_em);
        }
        if let Some(exn_off) = layout.exn_state_off {
            let skip_exn = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                Reg::X19,
                exn_off + 8,
                Reg::X15,
            );
            a.cbz(true, Reg::X0, skip_exn);
            a.mov_imm64(Reg::X1, EXN_SLAB_BYTES as u64);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_exn);
        }
        for &(desc_off, _, _) in layout.dyn_tables.values() {
            let skip_dt = a.new_label();
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                Reg::X19,
                desc_off,
                Reg::X15,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X1),
                None,
                Reg::X19,
                desc_off + 16,
                Reg::X15,
            );
            a.cbz(true, Reg::X1, skip_dt);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(skip_dt);
        }
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            Reg::X19,
            OFF_GUEST_STACK_BASE,
            Reg::X15,
        );
        a.cbz(true, Reg::X0, skip_s);
        a.mov_imm64(Reg::X1, GUEST_STACK_TOTAL_BYTES as u64);
        emit_syscall(&mut a, SYS_MUNMAP);
        a.bind(skip_s);
        a.mov_r(true, Reg::X0, Reg::X19);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            Reg::X19,
            OFF_HANDLE_ALLOC_BYTES,
            Reg::X15,
        );
        emit_syscall(&mut a, SYS_MUNMAP);
        a.ldr_uimm(true, Reg::X19, Reg::SP, 16);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 32);
        a.bind(ret_now);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_drop", a));
    }
    // 7. blot_aot_mem_data(ctx, out_ptr, out_len)
    {
        let mut a = A64Asm::new();
        let zero = a.new_label();
        let zero_l = a.new_label();
        let done = a.new_label();
        let sp = a.new_label();
        a.cbz(true, Reg::X0, zero);
        a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
        a.cbnz(false, Reg::X9, zero);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X3),
            None,
            Reg::X0,
            OFF_MEM_BASE,
            Reg::X15,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            Reg::X0,
            OFF_MEM_LEN,
            Reg::X15,
        );
        a.cbz(true, Reg::X1, sp);
        a.str_uimm(true, Reg::X3, Reg::X1, 0);
        a.bind(sp);
        a.cbz(true, Reg::X2, done);
        a.str_uimm(true, Reg::X4, Reg::X2, 0);
        a.b(done);
        a.bind(zero);
        a.cbz(true, Reg::X1, zero_l);
        a.str_uimm(true, Reg::XZR, Reg::X1, 0);
        a.bind(zero_l);
        a.cbz(true, Reg::X2, done);
        a.str_uimm(true, Reg::XZR, Reg::X2, 0);
        a.bind(done);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_mem_data", a));
    }
    // 8. blot_aot_call_export(ctx, name_ptr, name_len, arg0, out_res, out_trap) -> i32
    {
        let mut a = A64Asm::new();
        let not_found = a.new_label();
        let reentry_trap = a.new_label();
        let reentry_skip = a.new_label();
        let name_ok = a.new_label();
        let ret_ep = a.new_label();
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -96);
        a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
        a.stp_x(Reg::X21, Reg::X22, Reg::SP, 32);
        a.stp_x(Reg::X23, Reg::X24, Reg::SP, 48);
        a.stp_x(Reg::X25, Reg::X26, Reg::SP, 64);
        a.stp_x(Reg::X27, Reg::X28, Reg::SP, 80);

        a.cbz(true, Reg::X0, not_found);
        a.mov_r(true, CTX_REG, Reg::X0);
        a.mov_r(true, Reg::X19, Reg::X1); // name_ptr
        a.mov_r(true, Reg::X20, Reg::X2); // name_len
        a.mov_r(true, Reg::X21, Reg::X3); // arg0
        a.mov_r(true, Reg::X22, Reg::X4); // out_res
        a.mov_r(true, Reg::X23, Reg::X5); // out_trap

        a.ldrb_uimm(Reg::X9, CTX_REG, OFF_IN_CALL as u32);
        a.cbnz(false, Reg::X9, reentry_trap);
        a.cbz(true, Reg::X20, name_ok);
        a.cbz(true, Reg::X19, not_found);
        a.bind(name_ok);

        emit_load_off(
            &mut a,
            Ty::I64,
            Some(MEM_BASE_REG),
            None,
            CTX_REG,
            OFF_MEM_BASE,
            SGP3,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(MEM_LEN_REG),
            None,
            CTX_REG,
            OFF_MEM_LEN,
            SGP3,
        );
        emit_a64_pinned_load(&mut a, module, layout);

        // Linear scan over exported functions comparing name bytes against local rodata
        // (supports arbitrary export name lengths without `ldrb_uimm` 4095-byte overflow).
        for (exp_name, func_index) in &module.exported_funcs {
            let next_exp = a.new_label();
            let nb = exp_name.as_bytes();
            a.mov_imm64(Reg::X6, nb.len() as u64);
            a.cmp_r(true, Reg::X20, Reg::X6);
            a.b_cond(Cond::Ne, next_exp);
            if !nb.is_empty() {
                let name_off = a.add_local_rodata(1, nb);
                a.adr_reloc(Reg::X6, A64RelocTarget::LocalRodata(name_off));
                a.movz(true, Reg::X8, 0, 0);
                let cmp_lp = a.new_label();
                a.bind(cmp_lp);
                a.ldrb_reg(Reg::X7, Reg::X19, Reg::X8, false);
                a.ldrb_reg(Reg::X9, Reg::X6, Reg::X8, false);
                a.cmp_r(false, Reg::X7, Reg::X9);
                a.b_cond(Cond::Ne, next_exp);
                a.add_imm(true, Reg::X8, Reg::X8, 1);
                a.cmp_r(true, Reg::X8, Reg::X20);
                a.b_cond(Cond::Lo, cmp_lp);
            }
            // Matched export! Marshal up to 1 arg (i32/i64/f32/f64/ref) and call.
            let ty_sig = module
                .func_type(*func_index)
                .ok_or_else(|| format!("aarch64: missing type for func {func_index}"))?;
            if ty_sig.params.len() > 1 || ty_sig.results.len() > 1 {
                a.b(not_found);
            } else {
                if let Some(&pt) = ty_sig.params.first() {
                    match pt {
                        ValType::I32 => a.uxtw(Reg::X0, Reg::X21),
                        ValType::F32 => a.fmov_fp_from_gp(false, VReg::V0, Reg::X21),
                        ValType::F64 => a.fmov_fp_from_gp(true, VReg::V0, Reg::X21),
                        _ => a.mov_r(true, Reg::X0, Reg::X21),
                    }
                }
                a.mov_imm32(Reg::X7, 1);
                a.strb_uimm(Reg::X7, CTX_REG, OFF_IN_CALL as u32);
                // Switch to dedicated 8 MiB guest stack (saving host SP in `[CTX + OFF_HOST_RSP]`).
                a.mov_r(true, SGP2, Reg::SP);
                emit_store_off(
                    &mut a,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_HOST_RSP,
                    SGP3,
                );
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_GUEST_STACK_TOP,
                    SGP3,
                );
                emit_store_off(
                    &mut a,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_ACTIVE_GUEST_RSP,
                    SGP3,
                );
                a.mov_r(true, Reg::SP, SGP2);
                a.bl_reloc(A64RelocTarget::Func(*func_index as u32));
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_HOST_RSP,
                    SGP3,
                );
                a.mov_r(true, Reg::SP, SGP2);
                a.strb_uimm(Reg::XZR, CTX_REG, OFF_IN_CALL as u32);
                emit_a64_pinned_store(&mut a, module, layout);
                let got_trap = a.new_label();
                a.cbnz(false, SGP1, got_trap);
                let skip_res = a.new_label();
                a.cbz(true, Reg::X22, skip_res);
                match ty_sig.results.first().copied() {
                    None => a.str_uimm(true, Reg::XZR, Reg::X22, 0),
                    Some(ValType::I32) => {
                        a.uxtw(Reg::X0, Reg::X0);
                        a.str_uimm(true, Reg::X0, Reg::X22, 0);
                    }
                    Some(ValType::F32) => {
                        a.fmov_gp_from_fp(false, Reg::X0, VReg::V0);
                        a.uxtw(Reg::X0, Reg::X0);
                        a.str_uimm(true, Reg::X0, Reg::X22, 0);
                    }
                    Some(ValType::F64) => {
                        a.fmov_gp_from_fp(true, Reg::X0, VReg::V0);
                        a.str_uimm(true, Reg::X0, Reg::X22, 0);
                    }
                    _ => a.str_uimm(true, Reg::X0, Reg::X22, 0),
                }
                a.bind(skip_res);
                a.movz(false, Reg::X0, 0, 0);
                a.b(ret_ep);

                a.bind(got_trap);
                let not_exn_call = a.new_label();
                a.cmp_imm(false, SGP1, u16::from(INTERNAL_EXN_SIGNAL));
                a.b_cond(Cond::Ne, not_exn_call);
                a.mov_imm32(SGP1, u32::from(TRAP_UNREACHABLE));
                a.bind(not_exn_call);
                let skip_t = a.new_label();
                a.cbz(true, Reg::X23, skip_t);
                a.strb_uimm(SGP1, Reg::X23, 0);
                a.bind(skip_t);
                a.mov_imm32(Reg::X0, 1);
                a.mov_imm32(Reg::X9, 2);
                a.cmp_imm(false, SGP1, u16::from(TRAP_HOST));
                a.csel(false, Reg::X0, Reg::X9, Reg::X0, Cond::Eq);
                a.b(ret_ep);
            }
            a.bind(next_exp);
        }

        a.bind(not_found);
        a.mov_imm32(Reg::X0, 3);
        a.b(ret_ep);

        a.bind(reentry_trap);
        a.cbz(true, Reg::X23, reentry_skip);
        a.mov_imm32(Reg::X9, u32::from(TRAP_HOST));
        a.strb_uimm(Reg::X9, Reg::X23, 0);
        a.bind(reentry_skip);
        a.mov_imm32(Reg::X0, 2);

        a.bind(ret_ep);
        a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
        a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 32);
        a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 48);
        a.ldp_x(Reg::X25, Reg::X26, Reg::SP, 64);
        a.ldp_x(Reg::X27, Reg::X28, Reg::SP, 80);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 96);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_call_export", a));
    }

    // 9-11. blot_aot_call_allocate, blot_aot_call_reset, blot_aot_call_finish_invocation
    out.push((
        "blot_aot_call_allocate",
        emit_a64_export_call_fixed(module, layout, "blot:allocate", true)?,
    ));
    out.push((
        "blot_aot_call_reset",
        emit_a64_export_call_fixed(module, layout, "blot:reset", true)?,
    ));
    out.push((
        "blot_aot_call_finish_invocation",
        emit_a64_export_call_fixed(module, layout, "blot:finish_invocation", false)?,
    ));

    // 12. blot_aot_mem_grow(ctx, delta_pages: u64) -> i64
    {
        let mut a = A64Asm::new();
        let fail = a.new_label();
        a.cbz(true, Reg::X0, fail);
        a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
        a.cbnz(false, Reg::X9, fail);
        a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -64);
        a.stp_x(Reg::X24, Reg::X25, Reg::SP, 16);
        a.stp_x(Reg::X26, Reg::X27, Reg::SP, 32);
        a.str_uimm(true, Reg::X28, Reg::SP, 48);
        a.mov_r(true, CTX_REG, Reg::X0);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(MEM_BASE_REG),
            None,
            CTX_REG,
            OFF_MEM_BASE,
            SGP3,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(MEM_LEN_REG),
            None,
            CTX_REG,
            OFF_MEM_LEN,
            SGP3,
        );
        a.mov_r(true, Reg::X0, Reg::X1); // full 64-bit delta_pages
        a.bl_reloc(A64RelocTarget::Helper(HELPER_MEM_GROW));
        a.ldr_uimm(true, Reg::X28, Reg::SP, 48);
        a.ldp_x(Reg::X26, Reg::X27, Reg::SP, 32);
        a.ldp_x(Reg::X24, Reg::X25, Reg::SP, 16);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 64);
        a.ret();
        a.bind(fail);
        a.mov_imm64(Reg::X0, u64::MAX);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_mem_grow", a));
    }

    // 13. blot_aot_get_global(ctx, name_ptr, name_len, val_out, kind_out) -> i32
    {
        let mut a = A64Asm::new();
        let fail = a.new_label();
        a.cbz(true, Reg::X0, fail);
        a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
        a.cbnz(false, Reg::X9, fail);
        a.b_reloc(A64RelocTarget::Helper(HELPER_READ_GLOBAL));
        a.bind(fail);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_get_global", a));
    }

    // 14. blot_aot_set_global(ctx, name_ptr, name_len, val) -> i32
    {
        let mut a = A64Asm::new();
        if tables.set_global_count == 0 {
            a.mov_imm32(Reg::X0, 1);
            a.ret();
        } else {
            let lbl_fail = a.new_label();
            let lbl_np_ok = a.new_label();
            let lbl_loop = a.new_label();
            let lbl_cmp = a.new_label();
            let lbl_next = a.new_label();
            let lbl_found = a.new_label();
            let lbl_store64 = a.new_label();
            let lbl_done = a.new_label();

            a.cbz(true, Reg::X0, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::X0, OFF_IN_CALL as u32);
            a.cbnz(false, Reg::X9, lbl_fail);
            a.cbz(true, Reg::X2, lbl_np_ok);
            a.cbz(true, Reg::X1, lbl_fail);
            a.bind(lbl_np_ok);

            // Table entry (16 B): +0: i32 name_rel, +4: u32 name_len, +8: i32 ctx_off, +12: u8 is64.
            a.adr_reloc(Reg::X5, A64RelocTarget::Rodata(tables.set_global_table_off));
            a.mov_r(true, Reg::X6, Reg::X5);
            a.mov_imm32(Reg::X7, tables.set_global_count);

            a.bind(lbl_loop);
            a.ldr_uimm(false, Reg::X8, Reg::X6, 4);
            a.cmp_r(true, Reg::X8, Reg::X2);
            a.b_cond(Cond::Ne, lbl_next);
            a.cbz(true, Reg::X2, lbl_found);

            a.ldrsw_uimm(Reg::X9, Reg::X6, 0);
            a.add_r(true, Reg::X9, Reg::X5, Reg::X9);
            a.movz(true, Reg::X10, 0, 0);
            a.bind(lbl_cmp);
            a.ldrb_reg(Reg::X11, Reg::X1, Reg::X10, false);
            a.ldrb_reg(Reg::X12, Reg::X9, Reg::X10, false);
            a.cmp_r(false, Reg::X11, Reg::X12);
            a.b_cond(Cond::Ne, lbl_next);
            a.add_imm(true, Reg::X10, Reg::X10, 1);
            a.cmp_r(true, Reg::X10, Reg::X2);
            a.b_cond(Cond::Lo, lbl_cmp);
            a.b(lbl_found);

            a.bind(lbl_next);
            a.add_imm(true, Reg::X6, Reg::X6, 16);
            a.sub_imm(false, Reg::X7, Reg::X7, 1);
            a.cbnz(false, Reg::X7, lbl_loop);

            a.bind(lbl_fail);
            a.mov_imm32(Reg::X0, 1);
            a.ret();

            a.bind(lbl_found);
            a.ldrsw_uimm(Reg::X10, Reg::X6, 8); // ctx_off
            a.ldrb_uimm(Reg::X11, Reg::X6, 12); // is64
            a.cbnz(false, Reg::X11, lbl_store64);
            a.str_reg(false, Reg::X3, Reg::X0, Reg::X10, false);
            a.b(lbl_done);
            a.bind(lbl_store64);
            a.str_reg(true, Reg::X3, Reg::X0, Reg::X10, false);
            a.bind(lbl_done);
            a.movz(false, Reg::X0, 0, 0);
            a.ret();
        }
        a.finish()?;
        out.push(("blot_aot_set_global", a));
    }

    // 15. blot_aot_active_mem_data(ac, ptr_out, len_out)
    {
        let mut a = A64Asm::new();
        let zero = a.new_label();
        let zero_l = a.new_label();
        let done = a.new_label();
        let sp = a.new_label();
        a.cbz(true, Reg::X0, zero);
        a.ldr_uimm(true, Reg::X3, Reg::X0, 0); // ac->ctx
        a.cbz(true, Reg::X3, zero);
        a.cbz(true, Reg::X1, sp);
        a.ldr_uimm(true, Reg::X4, Reg::X0, 8); // ac->mem
        a.str_uimm(true, Reg::X4, Reg::X1, 0);
        a.bind(sp);
        a.cbz(true, Reg::X2, done);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            Reg::X3,
            OFF_MEM_LEN,
            Reg::X15,
        );
        a.str_uimm(true, Reg::X4, Reg::X2, 0);
        a.b(done);
        a.bind(zero);
        a.cbz(true, Reg::X1, zero_l);
        a.str_uimm(true, Reg::XZR, Reg::X1, 0);
        a.bind(zero_l);
        a.cbz(true, Reg::X2, done);
        a.str_uimm(true, Reg::XZR, Reg::X2, 0);
        a.bind(done);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_active_mem_data", a));
    }

    // 16-17. blot_aot_active_allocate, blot_aot_active_reset
    out.push((
        "blot_aot_active_allocate",
        emit_a64_export_active_fixed(module, layout, "blot:allocate")?,
    ));
    out.push((
        "blot_aot_active_reset",
        emit_a64_export_active_fixed(module, layout, "blot:reset")?,
    ));

    // 18. blot_aot_active_get_global(ac, name_ptr, name_len, val_out, kind_out) -> i32
    {
        let mut a = A64Asm::new();
        let fail = a.new_label();
        a.cbz(true, Reg::X0, fail);
        a.ldr_uimm(true, Reg::X0, Reg::X0, 0); // ac->ctx
        a.cbz(true, Reg::X0, fail);
        a.b_reloc(A64RelocTarget::Helper(HELPER_READ_GLOBAL));
        a.bind(fail);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_active_get_global", a));
    }

    // Reorder strictly to match `EXPORT_NAMES` order expected by `A64ElfSoLayout::plan`.
    let mut ordered = Vec::with_capacity(EXPORT_NAMES.len());
    for name in EXPORT_NAMES {
        let pos = out
            .iter()
            .position(|(n, _)| *n == name)
            .ok_or_else(|| format!("aarch64: missing Blot export {name}"))?;
        ordered.push(out.swap_remove(pos));
    }
    Ok(ordered)
}
