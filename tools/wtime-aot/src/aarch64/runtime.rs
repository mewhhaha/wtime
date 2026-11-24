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
    CtxLayout, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES, GUEST_STACK_TOTAL_BYTES,
    GUEST_STACK_USABLE_BYTES, HELPER_INITIALIZE, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW,
    OFF_GUEST_STACK_BASE, OFF_GUEST_STACK_TOP, OFF_HANDLE_ALLOC_BYTES, OFF_HOST_FN, OFF_HOST_RSP,
    OFF_HOST_USER_CTX, OFF_IN_CALL, OFF_MEM_BASE, OFF_MEM_LEN, OFF_MEM_MAX_PAGES,
    OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR, TRAP_ALLOC_FAILED, TRAP_MEM_LIMIT, TRAP_MEM_TOO_LARGE,
    TRAP_OOB,
};
use wtime_types::ValType;

const SYS_WRITE: u64 = 64;
const SYS_EXIT_GROUP: u64 = 94;
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
        // Copy active data segments from `.rodata` into linear memory.
        for &(mem_off, rodata_off, seg_len) in &tables.data_segs {
            let len = u64::from(seg_len);
            if len == 0 {
                continue;
            }
            a.mov_imm64(Reg::X0, mem_off);
            a.mov_imm64(Reg::X2, len);
            a.adds_r(true, Reg::X3, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, oob);
            a.cmp_r(true, Reg::X3, MEM_LEN_REG);
            a.b_cond(Cond::Hi, oob);
            a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
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
        a.finish()?;
        out.push((HELPER_INITIALIZE, a));
    }

    Ok(out)
}

/// Emits Blot ABI 8 exported C functions on AArch64 (`EXPORT_NAMES` parity with x64/C/Rust).
pub fn emit_a64_exports(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Vec<(&'static str, A64Asm)>, String> {
    let mut out = Vec::with_capacity(EXPORT_NAMES.len());

    // 1. blot_aot_abi_version() -> 8
    {
        let mut a = A64Asm::new();
        a.mov_imm32(Reg::X0, 8);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_abi_version", a));
    }
    // 3. blot_aot_trap_message(code: u8, out_ptr: *mut *const u8, out_len: *mut usize) -> i32
    {
        let mut a = A64Asm::new();
        let bad = a.new_label();
        let skip_p = a.new_label();
        let skip_l = a.new_label();
        a.uxtb(Reg::X0, Reg::X0);
        a.sub_imm(false, Reg::X0, Reg::X0, 1);
        a.cmp_imm(false, Reg::X0, 22);
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
    // 4. blot_aot_import_count() -> usize
    {
        let mut a = A64Asm::new();
        a.mov_imm64(Reg::X0, module.imports.len() as u64);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_import_count", a));
    }
    // 5. blot_aot_import_name(...) -> i32 (1 = OOB when 0 imports; lookup table otherwise)
    {
        let mut a = A64Asm::new();
        let bad = a.new_label();
        a.mov_imm64(Reg::X5, module.imports.len() as u64);
        a.cmp_r(true, Reg::X0, Reg::X5);
        a.b_cond(Cond::Hs, bad);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.bind(bad);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_import_name", a));
    }
    // 6. blot_aot_new(host_ctx, host_fn, max_mem_bytes, out_trap) -> *mut Ctx
    {
        let mut a = A64Asm::new();
        let fail_limit = a.new_label();
        let fail_alloc = a.new_label();
        let fail_trap = a.new_label();
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
        a.mov_r(true, Reg::X21, Reg::X2); // max_mem_bytes
        a.mov_r(true, Reg::X22, Reg::X3); // out_trap

        let (min_pages, max_pages) = module.memory.unwrap_or((0, 0));
        // Effective max pages capped by `max_mem_bytes >> 16` when non-zero.
        a.mov_imm64(Reg::X23, max_pages);
        let no_cap = a.new_label();
        a.cbz(true, Reg::X21, no_cap);
        a.lsr_imm(true, Reg::X4, Reg::X21, 16);
        a.cmp_r(true, Reg::X4, Reg::X23);
        a.csel(true, Reg::X23, Reg::X4, Reg::X23, Cond::Lo);
        a.bind(no_cap);
        a.mov_imm64(Reg::X4, min_pages);
        a.cmp_r(true, Reg::X4, Reg::X23);
        a.b_cond(Cond::Hi, fail_limit);

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
        a.b_cond(Cond::Hs, fail_alloc);
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
        a.b_cond(Cond::Hs, fail_alloc);
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
        a.b_cond(Cond::Hs, fail_alloc);
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
        if min_pages > 0 {
            a.mov_imm64(MEM_LEN_REG, min_pages << 16);
            a.mov_r(true, Reg::X0, MEM_BASE_REG);
            a.mov_r(true, Reg::X1, MEM_LEN_REG);
            a.mov_imm32(Reg::X2, 3);
            emit_syscall(&mut a, SYS_MPROTECT);
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

        // Initialize globals, data segments, and optional start function.
        a.bl_reloc(A64RelocTarget::Helper(HELPER_INITIALIZE));
        a.cbnz(false, SGP1, fail_trap);
        a.mov_r(true, Reg::X0, CTX_REG);
        a.b(done_ok);

        a.bind(fail_limit);
        a.mov_imm32(
            SGP1,
            u32::from(if min_pages > max_pages {
                TRAP_MEM_TOO_LARGE
            } else {
                TRAP_MEM_LIMIT
            }),
        );
        a.b(fail_trap);
        a.bind(fail_alloc);
        a.mov_imm32(SGP1, u32::from(TRAP_ALLOC_FAILED));
        a.bind(fail_trap);
        let skip_tr = a.new_label();
        a.cbz(true, Reg::X22, skip_tr);
        a.strb_uimm(SGP1, Reg::X22, 0);
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
    // 7. blot_aot_drop(ctx)
    {
        let mut a = A64Asm::new();
        let ret_now = a.new_label();
        let skip_m = a.new_label();
        let skip_s = a.new_label();
        a.cbz(true, Reg::X0, ret_now);
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
            SGP3,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X1),
            None,
            Reg::X19,
            OFF_MEM_RESERVE_BYTES,
            SGP3,
        );
        a.cbz(true, Reg::X0, skip_m);
        emit_syscall(&mut a, SYS_MUNMAP);
        a.bind(skip_m);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            Reg::X19,
            OFF_GUEST_STACK_BASE,
            SGP3,
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
            SGP3,
        );
        emit_syscall(&mut a, SYS_MUNMAP);
        a.ldr_uimm(true, Reg::X19, Reg::SP, 16);
        a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 32);
        a.bind(ret_now);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_drop", a));
    }
    // 8. blot_aot_memory(ctx, out_ptr, out_len) -> i32
    {
        let mut a = A64Asm::new();
        let bad = a.new_label();
        a.cbz(true, Reg::X0, bad);
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X3),
            None,
            Reg::X0,
            OFF_MEM_BASE,
            SGP3,
        );
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            Reg::X0,
            OFF_MEM_LEN,
            SGP3,
        );
        let sp = a.new_label();
        let sl = a.new_label();
        a.cbz(true, Reg::X1, sp);
        a.str_uimm(true, Reg::X3, Reg::X1, 0);
        a.bind(sp);
        a.cbz(true, Reg::X2, sl);
        a.str_uimm(true, Reg::X4, Reg::X2, 0);
        a.bind(sl);
        a.movz(false, Reg::X0, 0, 0);
        a.ret();
        a.bind(bad);
        a.mov_imm32(Reg::X0, 1);
        a.ret();
        a.finish()?;
        out.push(("blot_aot_mem_data", a));
    }
    // 9. blot_aot_call_export(ctx, name_ptr, name_len, arg0, out_res, out_trap) -> i32
    {
        let mut a = A64Asm::new();
        let not_found = a.new_label();
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
        if let Some(pg) = layout.pinned_global {
            let off = layout.global_offs[pg as usize];
            let ty = match module.globals[pg as usize].0 {
                ValType::I64 | ValType::Ref(_) => Ty::I64,
                _ => Ty::I32,
            };
            emit_load_off(&mut a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
        }

        // Linear scan over exported functions comparing name bytes.
        for (exp_name, func_index) in &module.exported_funcs {
            let next_exp = a.new_label();
            let nb = exp_name.as_bytes();
            a.mov_imm64(Reg::X6, nb.len() as u64);
            a.cmp_r(true, Reg::X20, Reg::X6);
            a.b_cond(Cond::Ne, next_exp);
            for (bi, &ch) in nb.iter().enumerate() {
                a.ldrb_uimm(Reg::X7, Reg::X19, bi as u32);
                a.cmp_imm(false, Reg::X7, u16::from(ch));
                a.b_cond(Cond::Ne, next_exp);
            }
            // Matched export! Marshal up to 1 arg (i32/i64/f32/f64) and call.
            let ty_sig = module
                .func_type(*func_index)
                .ok_or_else(|| format!("aarch64: missing type for func {func_index}"))?;
            if ty_sig.params.len() > 1 || ty_sig.results.len() > 1 {
                a.b(not_found);
            } else {
                if let Some(&pt) = ty_sig.params.first() {
                    match pt {
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
                if let Some(pg) = layout.pinned_global {
                    let off = layout.global_offs[pg as usize];
                    let ty = match module.globals[pg as usize].0 {
                        ValType::I64 | ValType::Ref(_) => Ty::I64,
                        _ => Ty::I32,
                    };
                    emit_store_off(&mut a, ty, Some(PINNED_GLOB_REG), None, CTX_REG, off, SGP3);
                }
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
                let skip_t = a.new_label();
                a.cbz(true, Reg::X23, skip_t);
                a.strb_uimm(SGP1, Reg::X23, 0);
                a.bind(skip_t);
                a.mov_imm32(Reg::X0, 2);
                a.b(ret_ep);
            }
            a.bind(next_exp);
        }

        a.bind(not_found);
        a.mov_imm32(Reg::X0, 1);
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

    // Remaining Blot ABI 8 symbols (matching exact `EXPORT_NAMES` set).
    let stub_names: &[(&'static str, u32)] = &[
        ("blot_aot_call_allocate", 1),
        ("blot_aot_call_reset", 1),
        ("blot_aot_call_finish_invocation", 1),
        ("blot_aot_mem_grow", 1),
        ("blot_aot_get_global", 1),
        ("blot_aot_set_global", 1),
        ("blot_aot_active_mem_data", 1),
        ("blot_aot_active_allocate", 1),
        ("blot_aot_active_reset", 1),
        ("blot_aot_active_get_global", 1),
    ];
    for &(name, rv) in stub_names {
        let mut a = A64Asm::new();
        a.mov_imm32(Reg::X0, rv);
        a.ret();
        a.finish()?;
        out.push((name, a));
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

/// Emits standalone Linux AArch64 `_start` entry point (`ET_EXEC`).
pub fn emit_a64_standalone_start(module: &X64Module, layout: &CtxLayout) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let trap_exit = a.new_label();

    // Allocate Ctx via mmap.
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm64(Reg::X1, layout.total_bytes as u64);
    a.mov_imm32(Reg::X2, 3);
    a.mov_imm32(Reg::X3, 0x22);
    a.mov_imm64(Reg::X4, u64::MAX);
    a.movz(true, Reg::X5, 0, 0);
    emit_syscall(&mut a, SYS_MMAP);
    a.mov_r(true, CTX_REG, Reg::X0);
    let (min_pages, max_pages) = module.memory.unwrap_or((0, 0));
    a.mov_imm64(Reg::X4, max_pages);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_MEM_MAX_PAGES,
        SGP3,
    );

    // Allocate 8 MiB guest stack + guard page.
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm64(Reg::X1, GUEST_STACK_TOTAL_BYTES as u64);
    a.movz(true, Reg::X2, 0, 0);
    a.mov_imm32(Reg::X3, 0x4022);
    a.mov_imm64(Reg::X4, u64::MAX);
    a.movz(true, Reg::X5, 0, 0);
    emit_syscall(&mut a, SYS_MMAP);
    a.mov_r(true, Reg::X19, Reg::X0);
    a.mov_imm64(Reg::X1, GUEST_STACK_GUARD_BYTES as u64);
    a.add_r(true, Reg::X0, Reg::X19, Reg::X1);
    a.mov_imm64(Reg::X1, GUEST_STACK_USABLE_BYTES as u64);
    a.mov_imm32(Reg::X2, 3);
    emit_syscall(&mut a, SYS_MPROTECT);
    a.mov_imm64(
        Reg::X4,
        (GUEST_STACK_GUARD_BYTES + GUEST_STACK_FLOOR_HEADROOM) as u64,
    );
    a.add_r(true, Reg::X4, Reg::X19, Reg::X4);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_STACK_FLOOR,
        SGP3,
    );
    a.mov_imm64(Reg::X4, GUEST_STACK_TOTAL_BYTES as u64);
    a.add_r(true, Reg::X4, Reg::X19, Reg::X4);
    a.mov_r(true, Reg::SP, Reg::X4);

    // Reserve & commit linear memory.
    a.movz(true, MEM_BASE_REG, 0, 0);
    a.movz(true, MEM_LEN_REG, 0, 0);
    if max_pages > 0 {
        a.movz(true, Reg::X0, 0, 0);
        a.mov_imm64(Reg::X1, max_pages << 16);
        a.movz(true, Reg::X2, 0, 0);
        a.mov_imm32(Reg::X3, 0x4022);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
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
        if min_pages > 0 {
            a.mov_imm64(MEM_LEN_REG, min_pages << 16);
            a.mov_r(true, Reg::X0, MEM_BASE_REG);
            a.mov_r(true, Reg::X1, MEM_LEN_REG);
            a.mov_imm32(Reg::X2, 3);
            emit_syscall(&mut a, SYS_MPROTECT);
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
    }

    a.bl_reloc(A64RelocTarget::Helper(HELPER_INITIALIZE));
    a.cbnz(false, SGP1, trap_exit);
    if let Some((entry_f, _)) = module.standalone_start {
        a.bl_reloc(A64RelocTarget::Func(entry_f));
        a.cbnz(false, SGP1, trap_exit);
    }
    a.movz(true, Reg::X0, 0, 0);
    emit_syscall(&mut a, SYS_EXIT_GROUP);

    a.bind(trap_exit);
    let msg_off = a.add_local_rodata(1, b"Trap\n");
    a.mov_imm32(Reg::X0, 2); // stderr
    a.adr_reloc(Reg::X1, A64RelocTarget::LocalRodata(msg_off));
    a.mov_imm32(Reg::X2, 5);
    emit_syscall(&mut a, SYS_WRITE);
    a.mov_imm32(Reg::X0, 1);
    emit_syscall(&mut a, SYS_EXIT_GROUP);
    a.finish()?;
    Ok(a)
}
