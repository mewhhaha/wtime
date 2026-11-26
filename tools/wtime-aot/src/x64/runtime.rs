//! Shared runtime helpers, `wt_import{k}` trampolines, `wt_initialize`, and the
//! complete 18-export Library Protocol v1 (Blot Guest ABI 8) C ABI wrappers.

use wtime_types::ValType;

use super::asm::{Asm, Cond, Gp, Mem, RelocTarget, Xmm};
use super::ir::TruncOp;
use super::lower::TargetIsa;
use super::module::{X64ConstVal, X64Module};
use super::{
    CtxLayout, EXN_SLAB_BYTES, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES,
    GUEST_STACK_TOTAL_BYTES, GUEST_STACK_USABLE_BYTES, HELPER_EXTRA_MEM_COPY,
    HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW, HELPER_GROW_CORE, HELPER_HOST_CALL,
    HELPER_INITIALIZE, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, HELPER_MEM_INIT,
    HELPER_READ_GLOBAL, HELPER_TABLE_COPY, HELPER_TABLE_FILL, HELPER_TABLE_GROW, HELPER_TABLE_INIT,
    HELPER_TRUNC_BASE, INTERNAL_EXN_SIGNAL, MAX_ACTIVE_REENTRY_DEPTH, OFF_ACTIVE_DEPTH,
    OFF_ACTIVE_GUEST_RSP, OFF_CPU_HAS_CET_SS, OFF_GUEST_STACK_BASE, OFF_GUEST_STACK_TOP,
    OFF_HANDLE_ALLOC_BYTES, OFF_HOST_FN, OFF_HOST_RSP, OFF_HOST_USER_CTX, OFF_IN_CALL,
    OFF_MEM_BASE, OFF_MEM_LEN, OFF_MEM_MAX_PAGES, OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR,
    RodataConsts, TRAP_ALLOC_FAILED, TRAP_CALL_STACK, TRAP_HOST, TRAP_INT_OVERFLOW,
    TRAP_INVALID_CONV, TRAP_MEM_LIMIT, TRAP_MEM_TOO_LARGE, TRAP_OOB, TRAP_TABLE_OOB,
    TRAP_UNREACHABLE, TRAP_UNSUPPORTED_ISA,
};

const SYS_MMAP: u32 = 9;
const SYS_MPROTECT: u32 = 10;
const SYS_MUNMAP: u32 = 11;

const PROT_NONE: u32 = 0;
const PROT_RW: u32 = 3; // PROT_READ | PROT_WRITE
const MAP_PRIVATE_ANON: u32 = 0x22;
const MAP_PRIVATE_ANON_NORESERVE: u32 = 0x4022;

/// Canonical trap messages for codes `1..=22` (matching `cgen.rs` verbatim + x64 code `22`).
pub const TRAP_MESSAGES: [&str; 22] = [
    "Trap: unreachable",
    "Trap: out of bounds",
    "Trap: table out of bounds",
    "Trap: indirect call type mismatch",
    "Trap: integer divide by zero",
    "Trap: integer overflow",
    "Trap: invalid integer conversion",
    "ResourceLimit: call depth exhausted",
    "ResourceLimit: memory too large",
    "ResourceLimit: memory allocation overflow",
    "ResourceLimit: memory limit",
    "ResourceLimit: memory allocation failed",
    "module does not export _start",
    "Host: function value count mismatch",
    "'_start' is not a function",
    "Host: argument type mismatch",
    "host import result does not fit its declared type",
    "callback produced results when it shouldn't",
    "Host: host failure",
    "program exit",
    "Host: generator raised StopIteration",
    "Unsupported CPU ISA",
];

/// Exact ordered list of all 18 exported Library Protocol v1 (Blot Guest ABI 8) symbols.
pub const EXPORT_NAMES: [&str; 18] = [
    "blot_aot_abi_version",
    "blot_aot_trap_message",
    "blot_aot_import_count",
    "blot_aot_import_name",
    "blot_aot_new",
    "blot_aot_drop",
    "blot_aot_call_export",
    "blot_aot_call_allocate",
    "blot_aot_call_reset",
    "blot_aot_call_finish_invocation",
    "blot_aot_mem_data",
    "blot_aot_mem_grow",
    "blot_aot_get_global",
    "blot_aot_set_global",
    "blot_aot_active_mem_data",
    "blot_aot_active_allocate",
    "blot_aot_active_reset",
    "blot_aot_active_get_global",
];

/// Fault-injection hooks for CPUID verification in tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub enum CpuidSource {
    Real,
    #[cfg(test)]
    Mock {
        max_leaf: u32,
        ecx1: u32,
        ext_max: u32,
        ext_ecx: u32,
        ebx7: u32,
        ecx7: u32,
        xcr0: u32,
    },
}

/// Fault-injection hooks for CET `RDSSPQ` verification in tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub enum RdsspSource {
    Real,
    #[cfg(test)]
    Mock {
        ssp: u64,
    },
}

/// Fault-injection hooks for per-site `mprotect` failure verification in tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub enum SyscallFault {
    None,
    #[cfg(test)]
    FailMprotectAt(MprotectSite),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MprotectSite {
    GuestStack,
    InitialMemory,
    GrowMemory,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeTestHooks {
    pub cpuid: CpuidSource,
    pub rdssp: RdsspSource,
    #[allow(dead_code)]
    pub syscall_fault: SyscallFault,
}

impl Default for RuntimeTestHooks {
    fn default() -> Self {
        Self {
            cpuid: CpuidSource::Real,
            rdssp: RdsspSource::Real,
            syscall_fault: SyscallFault::None,
        }
    }
}

/// Shared `.rodata` descriptor table offsets used by runtime helpers and C exports.
#[derive(Default)]
pub struct RuntimeTables {
    pub trap_table_off: u32,
    pub imp_table_off: u32,
    pub imp_count: u32,
    pub call_export_table_off: u32,
    pub call_export_count: u32,
    pub get_global_table_off: u32,
    pub get_global_count: u32,
    pub set_global_table_off: u32,
    pub set_global_count: u32,
    /// Active data segments `(mem_idx, mem_off, rodata_off, len)` in declaration order.
    pub data_segs: Vec<(u32, u64, u32, u32)>,
    /// V128 non-zero global initializers `(global_index, rodata_off)`.
    pub v128_global_inits: Vec<(usize, u32)>,
    /// `.rodata` descriptor table of `data_seg_count` × 8B `(i32 rel_off, u32 byte_len)` for `memory.init`.
    pub passive_data_table_off: u32,
    /// `.rodata` descriptor table of `elem_segs.len()` × 8B `(i32 rel_off, u32 elem_len)` for `table.init`.
    pub elem_seg_table_off: u32,
    /// Active element initializers targeting dynamic tables `(table_idx, dst_off, rodata_off, elem_len)`.
    pub active_dyn_elem_inits: Vec<(u32, u64, u32, u32)>,
    /// `.rodata` offset of `u32 func_canon_sig[n_total]` for dynamic `call_indirect`.
    pub func_sig_ro_off: u32,
    /// `.rodata` offset of `i32 func_addr_rel[n_total]` for dynamic `call_indirect`.
    pub func_addr_ro_off: u32,
}

/// Scalar/ref marshalling kind for `blot_aot_call_export` `.rodata` descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExportValKind {
    Void = 0,
    I32 = 1,
    I64OrRef = 2,
    F32 = 3,
    F64 = 4,
}

pub fn export_val_kind(tys: &[ValType]) -> Option<ExportValKind> {
    match tys {
        [] => Some(ExportValKind::Void),
        [ValType::I32] => Some(ExportValKind::I32),
        [ValType::I64] | [ValType::Ref(_)] => Some(ExportValKind::I64OrRef),
        [ValType::F32] => Some(ExportValKind::F32),
        [ValType::F64] => Some(ExportValKind::F64),
        _ => None,
    }
}

pub fn global_kind_code(vt: ValType) -> Option<u8> {
    match vt {
        ValType::I32 => Some(0),
        ValType::I64 => Some(1),
        ValType::F32 => Some(2),
        ValType::F64 => Some(3),
        ValType::Ref(_) => Some(4),
        ValType::V128 => None,
    }
}

fn emit_pinned_load(asm: &mut Asm, module: &X64Module, layout: &CtxLayout) {
    if let Some(g) = layout.pinned_global {
        let is64 = module.globals[g as usize].0 == ValType::I64;
        let off = layout.global_offs[g as usize];
        asm.mov_rm(is64, Gp::R12, Mem::base_disp(Gp::R15, off));
    }
    if let Some(g2) = layout.pinned_global_2 {
        let is64 = module.globals[g2 as usize].0 == ValType::I64;
        let off = layout.global_offs[g2 as usize];
        asm.mov_rm(is64, Gp::R13, Mem::base_disp(Gp::R15, off));
    }
}

fn emit_pinned_store(asm: &mut Asm, module: &X64Module, layout: &CtxLayout) {
    if let Some(g) = layout.pinned_global {
        let is64 = module.globals[g as usize].0 == ValType::I64;
        let off = layout.global_offs[g as usize];
        asm.mov_mr(is64, Mem::base_disp(Gp::R15, off), Gp::R12);
    }
    if let Some(g2) = layout.pinned_global_2 {
        let is64 = module.globals[g2 as usize].0 == ValType::I64;
        let off = layout.global_offs[g2 as usize];
        asm.mov_mr(is64, Mem::base_disp(Gp::R15, off), Gp::R13);
    }
}

fn emit_rdsspq(asm: &mut Asm, dst: Gp, source: RdsspSource) {
    match source {
        RdsspSource::Real => {
            asm.xor_rr(false, dst, dst);
            asm.rdsspq(dst);
        }
        #[cfg(test)]
        RdsspSource::Mock { ssp } => {
            asm.mov_ri64(dst, ssp);
        }
    }
}

fn emit_mprotect_syscall(
    asm: &mut Asm,
    hooks: RuntimeTestHooks,
    #[allow(unused)] site: MprotectSiteMarker,
) {
    #[cfg(test)]
    if let SyscallFault::FailMprotectAt(target) = hooks.syscall_fault
        && target == site.0
    {
        asm.mov_ri64(Gp::Rax, (-12i64) as u64); // -ENOMEM
        return;
    }
    let _ = hooks;
    asm.mov_ri32(Gp::Rax, SYS_MPROTECT);
    asm.syscall();
}

#[derive(Clone, Copy)]
struct MprotectSiteMarker(#[cfg(test)] MprotectSite, #[cfg(not(test))] ());

impl MprotectSiteMarker {
    fn guest_stack() -> Self {
        #[cfg(test)]
        {
            Self(MprotectSite::GuestStack)
        }
        #[cfg(not(test))]
        {
            Self(())
        }
    }
    fn initial_memory() -> Self {
        #[cfg(test)]
        {
            Self(MprotectSite::InitialMemory)
        }
        #[cfg(not(test))]
        {
            Self(())
        }
    }
    fn grow_memory() -> Self {
        #[cfg(test)]
        {
            Self(MprotectSite::GrowMemory)
        }
        #[cfg(not(test))]
        {
            Self(())
        }
    }
}

/// Emits all 16 float-to-integer truncation helpers (`HELPER_TRUNC_BASE + 0..=15`).
///
/// ABI:
/// - Input: `Xmm15`
/// - Output: `Rax` (32-bit zero-extended for `I32*`, 64-bit for `I64*`), `R11d = 0 | 6 | 7`
/// - Clobbers ONLY `Rax, R10, R11, Xmm14, Xmm15` (preserves all other GP & `Xmm0..Xmm13`).
pub fn emit_trunc_helper(top: TruncOp, ro: &RodataConsts) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    let is_sat = !top.can_trap();
    let is_f64 = matches!(
        top,
        TruncOp::I32F64S
            | TruncOp::I32F64U
            | TruncOp::I64F64S
            | TruncOp::I64F64U
            | TruncOp::I32F64SatS
            | TruncOp::I32F64SatU
            | TruncOp::I64F64SatS
            | TruncOp::I64F64SatU
    );
    let is_i64 = matches!(
        top,
        TruncOp::I64F32S
            | TruncOp::I64F32U
            | TruncOp::I64F64S
            | TruncOp::I64F64U
            | TruncOp::I64F32SatS
            | TruncOp::I64F32SatU
            | TruncOp::I64F64SatS
            | TruncOp::I64F64SatU
    );
    let is_unsigned = matches!(
        top,
        TruncOp::I32F32U
            | TruncOp::I32F64U
            | TruncOp::I64F32U
            | TruncOp::I64F64U
            | TruncOp::I32F32SatU
            | TruncOp::I32F64SatU
            | TruncOp::I64F32SatU
            | TruncOp::I64F64SatU
    );

    let lbl_nan = a.new_label();
    let lbl_under = a.new_label();
    let lbl_over = a.new_label();

    let load_const = |a: &mut Asm, dst: Xmm, off: u32| {
        if is_f64 {
            a.movsd_rip(dst, RelocTarget::Rodata(off));
        } else {
            a.movss_rip(dst, RelocTarget::Rodata(off));
        }
    };
    let ucomi = |a: &mut Asm, x: Xmm, y: Xmm| {
        if is_f64 {
            a.ucomisd(x, y);
        } else {
            a.ucomiss(x, y);
        }
    };

    // 1. Check NaN first so all subsequent `ucomis*` comparisons are strictly ordered.
    ucomi(&mut a, Xmm::Xmm15, Xmm::Xmm15);
    a.jcc(Cond::P, lbl_nan);

    // 2. Check lower & upper bounds.
    // - Signed I32 from F32: valid iff x >= -2^31 && x < +2^31
    // - Signed I32 from F64: valid iff x > -2^31 - 1 (-2147483649.0) && x < +2^31
    // - Unsigned I32 from F32/F64: valid iff x > -1.0 && x < +2^32
    // - Signed I64 from F32/F64: valid iff x >= -2^63 && x < +2^63
    // - Unsigned I64 from F32/F64: valid iff x > -1.0 && x < +2^64
    let (lo_off, lo_cond, hi_off) = match (is_i64, is_unsigned, is_f64) {
        (false, false, false) => (ro.f32_neg_2p31, Cond::B, ro.f32_2p31),
        (false, false, true) => (ro.f64_neg_2p31_m1, Cond::Be, ro.f64_2p31),
        (false, true, false) => (ro.f32_neg_one, Cond::Be, ro.f32_2p32),
        (false, true, true) => (ro.f64_neg_one, Cond::Be, ro.f64_2p32),
        (true, false, false) => (ro.f32_neg_2p63, Cond::B, ro.f32_2p63),
        (true, false, true) => (ro.f64_neg_2p63, Cond::B, ro.f64_2p63),
        (true, true, false) => (ro.f32_neg_one, Cond::Be, ro.f32_2p64),
        (true, true, true) => (ro.f64_neg_one, Cond::Be, ro.f64_2p64),
    };

    load_const(&mut a, Xmm::Xmm14, lo_off);
    ucomi(&mut a, Xmm::Xmm15, Xmm::Xmm14);
    a.jcc(lo_cond, lbl_under);

    load_const(&mut a, Xmm::Xmm14, hi_off);
    ucomi(&mut a, Xmm::Xmm15, Xmm::Xmm14);
    a.jcc(Cond::Ae, lbl_over);

    // 3. In-range conversion.
    match (is_i64, is_unsigned) {
        (false, false) => {
            if is_f64 {
                a.cvttsd2si(false, Gp::Rax, Xmm::Xmm15);
            } else {
                a.cvttss2si(false, Gp::Rax, Xmm::Xmm15);
            }
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        (false, true) => {
            // Range (-1, 2^32) fits cleanly in signed 64-bit integer; then zero-extend to 32-bit.
            if is_f64 {
                a.cvttsd2si(true, Gp::Rax, Xmm::Xmm15);
            } else {
                a.cvttss2si(true, Gp::Rax, Xmm::Xmm15);
            }
            a.mov_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        (true, false) => {
            if is_f64 {
                a.cvttsd2si(true, Gp::Rax, Xmm::Xmm15);
            } else {
                a.cvttss2si(true, Gp::Rax, Xmm::Xmm15);
            }
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        (true, true) => {
            let lbl_hi = a.new_label();
            let p63 = if is_f64 { ro.f64_2p63 } else { ro.f32_2p63 };
            load_const(&mut a, Xmm::Xmm14, p63);
            ucomi(&mut a, Xmm::Xmm15, Xmm::Xmm14);
            a.jcc(Cond::Ae, lbl_hi);
            if is_f64 {
                a.cvttsd2si(true, Gp::Rax, Xmm::Xmm15);
            } else {
                a.cvttss2si(true, Gp::Rax, Xmm::Xmm15);
            }
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_hi);
            if is_f64 {
                a.subsd(Xmm::Xmm15, Xmm::Xmm14);
                a.cvttsd2si(true, Gp::Rax, Xmm::Xmm15);
            } else {
                a.subss(Xmm::Xmm15, Xmm::Xmm14);
                a.cvttss2si(true, Gp::Rax, Xmm::Xmm15);
            }
            a.mov_ri64(Gp::R10, 0x8000_0000_0000_0000);
            a.or_rr(true, Gp::Rax, Gp::R10);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
    }

    // 4. Out-of-range / NaN paths.
    if is_sat {
        a.bind(lbl_nan);
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_under);
        match (is_i64, is_unsigned) {
            (_, true) => a.xor_rr(false, Gp::Rax, Gp::Rax),
            (false, false) => a.mov_ri32(Gp::Rax, 0x8000_0000), // i32::MIN zero-extended
            (true, false) => a.mov_ri64(Gp::Rax, 0x8000_0000_0000_0000),
        }
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_over);
        match (is_i64, is_unsigned) {
            (false, false) => a.mov_ri32(Gp::Rax, 0x7fff_ffff),
            (false, true) => a.mov_ri32(Gp::Rax, 0xffff_ffff),
            (true, false) => a.mov_ri64(Gp::Rax, 0x7fff_ffff_ffff_ffff),
            (true, true) => a.mov_ri64(Gp::Rax, 0xffff_ffff_ffff_ffff),
        }
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();
    } else {
        a.bind(lbl_nan);
        a.mov_ri32(Gp::R11, u32::from(TRAP_INVALID_CONV));
        a.ret();

        a.bind(lbl_under);
        a.bind(lbl_over);
        a.mov_ri32(Gp::R11, u32::from(TRAP_INT_OVERFLOW));
        a.ret();
    }

    a.finish_labels()?;
    Ok(a)
}

/// Shared syscall-only `grow_core(R15 = Ctx*, Rdi = delta: u64) -> Rax (old_pages | -1i64)`.
/// Updates `ctx->mem_len` (and `R13` when `!fixed_mem`) ONLY after `mprotect` succeeds;
/// never touches `R14`, `Xmm*`, or (when `fixed_mem`) `R13`.
pub fn emit_helper_grow_core(fixed_mem: bool, hooks: RuntimeTestHooks) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    let lbl_ok = a.new_label();
    let lbl_fail = a.new_label();

    // old_pages = ctx->mem_len >> 16
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    a.shr_imm(true, Gp::Rax, 16);

    // delta == 0 -> return old_pages immediately without mprotect.
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_ok);

    if fixed_mem {
        // Fixed-size memory (`min == max`): any non-zero grow always fails without touching R13.
        a.jmp(lbl_fail);
    } else {
        // Wrap-proof unsigned 64-bit check: delta <= max_pages && old_pages <= max_pages - delta
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_MEM_MAX_PAGES));
        a.cmp_rr(true, Gp::Rdi, Gp::Rcx);
        a.jcc(Cond::A, lbl_fail);
        a.sub_rr(true, Gp::Rcx, Gp::Rdi);
        a.cmp_rr(true, Gp::Rax, Gp::Rcx);
        a.jcc(Cond::A, lbl_fail);

        // Save old_pages in R8, delta_bytes = delta << 16 in R9.
        a.mov_rr(true, Gp::R8, Gp::Rax);
        a.mov_rr(true, Gp::R9, Gp::Rdi);
        a.shl_imm(true, Gp::R9, 16);

        // mprotect(ctx->mem_base + old_len, delta_bytes, PROT_READ | PROT_WRITE)
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
        a.add_rr(true, Gp::Rdi, Gp::R13);
        a.mov_rr(true, Gp::Rsi, Gp::R9);
        a.mov_ri32(Gp::Rdx, PROT_RW);
        emit_mprotect_syscall(&mut a, hooks, MprotectSiteMarker::grow_memory());
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail);

        // Commit new mem_len to both R13 and ctx->mem_len.
        a.add_rr(true, Gp::R13, Gp::R9);
        a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_LEN), Gp::R13);
        a.mov_rr(true, Gp::Rax, Gp::R8);
    }
    a.bind(lbl_ok);
    a.ret();

    a.bind(lbl_fail);
    a.mov_ri64(Gp::Rax, u64::MAX);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Wasm `memory.grow(Rdi = delta) -> Rax (old_pages | -1)`.
pub fn emit_helper_mem_grow(memory64: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(32);
    if !memory64 {
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
    }
    a.sub_ri(true, Gp::Rsp, 8);
    a.call_reloc(RelocTarget::Helper(HELPER_GROW_CORE));
    a.add_ri(true, Gp::Rsp, 8);
    if !memory64 {
        // Preserve I32 zero-extension invariant: failure becomes 0x0000_0000_ffff_ffff.
        a.mov_rr(false, Gp::Rax, Gp::Rax);
    }
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Wasm `memory.copy(Rdi = dst, Rsi = src, Rdx = len) -> R11d = 0 | TRAP_OOB`.
pub fn emit_helper_mem_copy(fixed_mem: bool, memory64: bool, spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();

    if !memory64 {
        // Defensive 32-bit zero extension.
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(false, Gp::Rsi, Gp::Rsi);
        a.mov_rr(false, Gp::Rdx, Gp::Rdx);
    }

    let len_reg = if fixed_mem {
        a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
        Gp::R10
    } else {
        Gp::R13
    };

    // Bounds checks FIRST (even when len == 0!).
    if memory64 {
        a.mov_rr(true, Gp::Rax, Gp::Rdi);
        a.add_rr(true, Gp::Rax, Gp::Rdx);
        a.jcc(Cond::B, lbl_oob);
        a.cmp_rr(true, Gp::Rax, len_reg);
        a.jcc(Cond::A, lbl_oob);
        a.mov_rr(true, Gp::Rcx, Gp::Rsi);
        a.add_rr(true, Gp::Rcx, Gp::Rdx);
        a.jcc(Cond::B, lbl_oob);
        a.cmp_rr(true, Gp::Rcx, len_reg);
        a.jcc(Cond::A, lbl_oob);
    } else {
        a.lea(true, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, 0));
        a.cmp_rr(true, Gp::Rax, len_reg);
        a.jcc(Cond::A, lbl_oob);
        a.lea(true, Gp::Rcx, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, 0));
        a.cmp_rr(true, Gp::Rcx, len_reg);
        a.jcc(Cond::A, lbl_oob);
    }

    // Early return on len == 0 or dst == src BEFORE touching R14.
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::E, lbl_ok);

    if spectre {
        // Serialize bounds and len == 0 checks before any memory access, and avoid
        // negative-offset head/tail size-dispatch tricks that could speculate past edges.
        let lbl_fwd_rep = a.new_label();
        let lbl_bwd1 = a.new_label();
        a.lfence();
        a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
        a.jcc(Cond::B, lbl_fwd_rep);
        a.cmp_rr(true, Gp::Rdi, Gp::Rcx);
        a.jcc(Cond::Ae, lbl_fwd_rep);
        // Overlapping backward copy (fenced against loop-exit underflow speculation).
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.add_rr(true, Gp::Rsi, Gp::R14);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.bind(lbl_bwd1);
        a.lfence();
        a.sub_ri(true, Gp::Rcx, 1);
        a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
        a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::Ne, lbl_bwd1);
        a.jmp(lbl_ok);

        a.bind(lbl_fwd_rep);
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.add_rr(true, Gp::Rsi, Gp::R14);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.rep_movsb();
    } else {
        let lbl_le15 = a.new_label();
        let lbl_le7 = a.new_label();
        let lbl_le3 = a.new_label();
        let lbl_one = a.new_label();
        let lbl_large = a.new_label();
        let lbl_fwd = a.new_label();
        let lbl_bwd16 = a.new_label();
        let lbl_bwd1_check = a.new_label();
        let lbl_bwd1 = a.new_label();

        // Fast overlap-safe small copies (`len <= 32`): load both head and tail BEFORE storing either!
        a.cmp_ri(true, Gp::Rdx, 32);
        a.jcc(Cond::A, lbl_large);
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.add_rr(true, Gp::Rsi, Gp::R14);
        a.cmp_ri(true, Gp::Rdx, 16);
        a.jcc(Cond::B, lbl_le15);
        // 16..=32 bytes: overlap-safe 2x XMM load then 2x store.
        a.movdqu_xm(Xmm::Xmm14, Mem::base(Gp::Rsi));
        a.movdqu_xm(Xmm::Xmm15, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -16));
        a.movdqu_mx(Mem::base(Gp::Rdi), Xmm::Xmm14);
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -16), Xmm::Xmm15);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le15);
        a.cmp_ri(true, Gp::Rdx, 8);
        a.jcc(Cond::B, lbl_le7);
        // 8..=15 bytes: overlap-safe 2x 64-bit load then 2x store.
        a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rsi));
        a.mov_rm(true, Gp::Rcx, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -8));
        a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -8), Gp::Rcx);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le7);
        a.cmp_ri(true, Gp::Rdx, 4);
        a.jcc(Cond::B, lbl_le3);
        // 4..=7 bytes: overlap-safe 2x 32-bit load then 2x store.
        a.mov_rm(false, Gp::Rax, Mem::base(Gp::Rsi));
        a.mov_rm(false, Gp::Rcx, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -4));
        a.mov_mr(false, Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr(false, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -4), Gp::Rcx);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le3);
        a.cmp_ri(true, Gp::Rdx, 1);
        a.jcc(Cond::E, lbl_one);
        // 2..=3 bytes: overlap-safe 16-bit head + 8-bit tail.
        a.movzx_rm16(false, Gp::Rax, Mem::base(Gp::Rsi));
        a.movzx_rm8(false, Gp::Rcx, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -1));
        a.mov_mr16(Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -1), Gp::Rcx);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_one);
        a.movzx_rm8(false, Gp::Rax, Mem::base(Gp::Rsi));
        a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        // Large copy (`len > 32`): forward if `dst < src || dst >= src + len` (`rcx == src + len`).
        a.bind(lbl_large);
        a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
        a.jcc(Cond::B, lbl_fwd);
        a.cmp_rr(true, Gp::Rdi, Gp::Rcx);
        a.jcc(Cond::Ae, lbl_fwd);

        // Large overlapping backward copy: 16-byte XMM14 chunks then byte tail.
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.add_rr(true, Gp::Rsi, Gp::R14);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.bind(lbl_bwd16);
        a.sub_ri(true, Gp::Rcx, 16);
        a.movdqu_xm(Xmm::Xmm14, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Xmm::Xmm14);
        a.cmp_ri(true, Gp::Rcx, 16);
        a.jcc(Cond::Ae, lbl_bwd16);
        a.bind(lbl_bwd1_check);
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::E, lbl_ok);
        a.bind(lbl_bwd1);
        a.sub_ri(true, Gp::Rcx, 1);
        a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
        a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::Ne, lbl_bwd1);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_fwd);
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.add_rr(true, Gp::Rsi, Gp::R14);
        // For medium forward copies (`33..=1024` B), a 32-byte XMM14/XMM15 loop avoids `rep movsb`'s
        // microcode startup latency; larger copies or tight forward overlaps (`0 < src - dst < 32`)
        // use `rep movsb`.
        let lbl_fwd_rep = a.new_label();
        let lbl_fwd32 = a.new_label();
        let lbl_fwd_tail = a.new_label();
        a.cmp_ri(true, Gp::Rdx, 1024);
        a.jcc(Cond::A, lbl_fwd_rep);
        // `lbl_fwd_tail` loads `[src + len - 32 .. src + len]` after `lbl_fwd32` may have written up
        // to `dst + len - 2`. That is overlap-safe iff the ranges are disjoint (`dst >= src + len`, so
        // `(src - dst) as u64 >= 2^64 - 4 GiB >= 32`) or `src - dst >= 32`.
        a.mov_rr(true, Gp::Rax, Gp::Rsi);
        a.sub_rr(true, Gp::Rax, Gp::Rdi);
        a.cmp_ri(true, Gp::Rax, 32);
        a.jcc(Cond::B, lbl_fwd_rep);
        a.xor_rr(false, Gp::Rcx, Gp::Rcx);
        a.lea(true, Gp::R8, Mem::base_disp(Gp::Rdx, -32)); // len > 32, so r8 >= 1
        a.bind(lbl_fwd32);
        a.movdqu_xm(Xmm::Xmm14, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
        a.movdqu_xm(Xmm::Xmm15, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 16));
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Xmm::Xmm14);
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 16), Xmm::Xmm15);
        a.add_ri(true, Gp::Rcx, 32);
        a.cmp_rr(true, Gp::Rcx, Gp::R8);
        a.jcc(Cond::B, lbl_fwd32);
        a.bind(lbl_fwd_tail);
        // Final overlapping 32-byte tail `[len - 32 .. len]` finishes any remainder `1..=32` in 4 insns.
        a.movdqu_xm(Xmm::Xmm14, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -32));
        a.movdqu_xm(Xmm::Xmm15, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, -16));
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -32), Xmm::Xmm14);
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -16), Xmm::Xmm15);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_fwd_rep);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.rep_movsb();
    }

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Wasm `memory.fill(Rdi = dst, Rsi = val: u32, Rdx = len) -> R11d = 0 | TRAP_OOB`.
pub fn emit_helper_mem_fill(fixed_mem: bool, memory64: bool, spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();

    if !memory64 {
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(false, Gp::Rdx, Gp::Rdx);
    }

    let len_reg = if fixed_mem {
        a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
        Gp::R10
    } else {
        Gp::R13
    };

    // Bounds check FIRST (even when len == 0!).
    if memory64 {
        a.mov_rr(true, Gp::Rax, Gp::Rdi);
        a.add_rr(true, Gp::Rax, Gp::Rdx);
        a.jcc(Cond::B, lbl_oob);
        a.cmp_rr(true, Gp::Rax, len_reg);
        a.jcc(Cond::A, lbl_oob);
    } else {
        a.lea(true, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, 0));
        a.cmp_rr(true, Gp::Rax, len_reg);
        a.jcc(Cond::A, lbl_oob);
    }

    // Early return on len == 0 BEFORE touching R14.
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);

    if spectre {
        a.lfence();
        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.mov_rr(false, Gp::Rax, Gp::Rsi);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.rep_stosb();
    } else {
        let lbl_large = a.new_label();
        let lbl_le15 = a.new_label();
        let lbl_le7 = a.new_label();
        let lbl_le3 = a.new_label();
        let lbl_one = a.new_label();

        a.add_rr(true, Gp::Rdi, Gp::R14);
        a.cmp_ri(true, Gp::Rdx, 32);
        a.jcc(Cond::A, lbl_large);

        // Fast small fill (`len <= 32`): broadcast byte into 64-bit Rax.
        a.movzx_rr8(Gp::Rax, Gp::Rsi);
        a.mov_ri64(Gp::Rcx, 0x0101_0101_0101_0101);
        a.imul_rr(true, Gp::Rax, Gp::Rcx);

        a.cmp_ri(true, Gp::Rdx, 16);
        a.jcc(Cond::B, lbl_le15);
        a.movq_xr(Xmm::Xmm14, Gp::Rax);
        a.punpcklqdq(Xmm::Xmm14, Xmm::Xmm14);
        a.movdqu_mx(Mem::base(Gp::Rdi), Xmm::Xmm14);
        a.movdqu_mx(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -16), Xmm::Xmm14);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le15);
        a.cmp_ri(true, Gp::Rdx, 8);
        a.jcc(Cond::B, lbl_le7);
        a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -8), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le7);
        a.cmp_ri(true, Gp::Rdx, 4);
        a.jcc(Cond::B, lbl_le3);
        a.mov_mr(false, Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr(false, Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -4), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_le3);
        a.cmp_ri(true, Gp::Rdx, 1);
        a.jcc(Cond::E, lbl_one);
        a.mov_mr16(Mem::base(Gp::Rdi), Gp::Rax);
        a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rdx, 1, -1), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_one);
        a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rax);
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();

        a.bind(lbl_large);
        a.mov_rr(false, Gp::Rax, Gp::Rsi);
        a.mov_rr(true, Gp::Rcx, Gp::Rdx);
        a.rep_stosb();
    }

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Shared host callback trampoline `HELPER_HOST_CALL(Edi = idx, Rsi = tok, Rdx = arg_bits)`.
/// Runs on guest stack (`rsp % 16 == 8`), switches back to `ctx->host_rsp`, restores host `MXCSR`,
/// invokes `ctx->host_fn(user_ctx, &ac, idx, tok, arg_bits, &res)`, switches back to guest stack,
/// reloads Wasm `MXCSR`, `R13 = mem_len`, and `R12 = g_pinned`, returning `Rax = res` & `R11d = 0 | TRAP_HOST`.
pub fn emit_helper_host_call(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let lbl_null_host = a.new_label();
    let lbl_fail_host = a.new_label();

    // Check host_fn != NULL before allocating guest link slots.
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_HOST_FN));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_null_host);

    // Guest stack frame: `sub rsp, 120` (`8 + 120 = 128 % 16 == 0`):
    //   [rsp + 0..96] = saved callee-saved Xmm8..Xmm13 (Stage 2 internal convention)
    //   [rsp + 96]    = prev_active_guest_rsp
    //   [rsp + 104]   = prev_host_rsp
    //   [rsp + 112]   = pad
    a.sub_ri(true, Gp::Rsp, 120);
    for (i, xmm) in [
        Xmm::Xmm8,
        Xmm::Xmm9,
        Xmm::Xmm10,
        Xmm::Xmm11,
        Xmm::Xmm12,
        Xmm::Xmm13,
    ]
    .into_iter()
    .enumerate()
    {
        a.movdqu_mx(Mem::base_disp(Gp::Rsp, (i as i32) * 16), xmm);
    }
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_ACTIVE_GUEST_RSP));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 96), Gp::R10);
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_HOST_RSP));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 104), Gp::R10);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_ACTIVE_GUEST_RSP), Gp::Rsp);

    // Publish pinned global to Ctx before leaving guest.
    emit_pinned_store(&mut a, module, layout);

    // Marshal SysV args 2, 3, 4 BEFORE switching stacks:
    //   Edx = idx (was Edi), Rcx = tok (was Rsi), R8 = arg_bits (was Rdx).
    a.mov_rr(true, Gp::R8, Gp::Rdx);
    a.mov_rr(true, Gp::Rcx, Gp::Rsi);
    a.mov_rr(false, Gp::Rdx, Gp::Rdi);

    // Switch to outer host stack (`rsp % 16 == 0`, `[rsp + 0]` = host MXCSR).
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::R15, OFF_HOST_RSP));
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));

    // Allocate 32 bytes on host stack (`rsp % 16 == 0`):
    //   [rsp + 0]  = ac.ctx = R15
    //   [rsp + 8]  = ac.mem = R14
    //   [rsp + 16] = u64 res = 0
    a.sub_ri(true, Gp::Rsp, 32);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::R15);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::R14);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);

    // Finish SysV 6-arg setup: Rdi = user_ctx, Rsi = &ac, R9 = &res.
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_HOST_USER_CTX));
    a.lea(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, 0));
    a.lea(true, Gp::R9, Mem::base_disp(Gp::Rsp, 16));
    a.call_mem(Mem::base_disp(Gp::R15, OFF_HOST_FN));

    // Capture status (`eax`) and `res` (`[rsp + 16]`) before switching back to guest stack.
    a.mov_rr(false, Gp::Rcx, Gp::Rax);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));

    // Switch back to guest stack and restore guest state.
    // Note: when `module.fixed32.is_some()`, `R13` is a callee-saved GP register preserved by SysV ABI
    // and `mem_len` is immutable, so we must NOT overwrite `R13`!
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::R15, OFF_ACTIVE_GUEST_RSP));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    if module.fixed32.is_none() {
        a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    }
    emit_pinned_load(&mut a, module, layout);

    // Restore saved callee-saved Xmm8..Xmm13, nesting pointers, and pop 120-byte link frame.
    for (i, xmm) in [
        Xmm::Xmm8,
        Xmm::Xmm9,
        Xmm::Xmm10,
        Xmm::Xmm11,
        Xmm::Xmm12,
        Xmm::Xmm13,
    ]
    .into_iter()
    .enumerate()
    {
        a.movdqu_xm(xmm, Mem::base_disp(Gp::Rsp, (i as i32) * 16));
    }
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rsp, 96));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_ACTIVE_GUEST_RSP), Gp::R10);
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rsp, 104));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::R10);
    a.add_ri(true, Gp::Rsp, 120);

    a.test_rr(false, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::Ne, lbl_fail_host);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();

    a.bind(lbl_null_host);
    a.bind(lbl_fail_host);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits internal Wasm ABI trampoline `wt_import{k}` for import `k`.
pub fn emit_import_trampoline(module: &X64Module, k: usize) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(64);
    let ty = module
        .func_type(k)
        .ok_or("import type index out of range")?;

    // Marshal internal Wasm ABI args into (Edi = k, Rsi = tok, Rdx = arg_bits).
    match ty.params.as_slice() {
        [] => {
            a.xor_rr(false, Gp::Rsi, Gp::Rsi);
            a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        }
        [ValType::Ref(_)] => {
            a.mov_rr(true, Gp::Rsi, Gp::Rdi);
            a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        }
        [ValType::I32] => {
            a.mov_rr(false, Gp::Rdx, Gp::Rdi);
            a.xor_rr(false, Gp::Rsi, Gp::Rsi);
        }
        [ValType::F32] => {
            a.movd_rx(Gp::Rdx, Xmm::Xmm0);
            a.xor_rr(false, Gp::Rsi, Gp::Rsi);
        }
        [ValType::Ref(_), ValType::I32] => {
            a.mov_rr(false, Gp::Rdx, Gp::Rsi);
            a.mov_rr(true, Gp::Rsi, Gp::Rdi);
        }
        [ValType::Ref(_), ValType::I64] | [ValType::Ref(_), ValType::Ref(_)] => {
            a.mov_rr(true, Gp::Rdx, Gp::Rsi);
            a.mov_rr(true, Gp::Rsi, Gp::Rdi);
        }
        [ValType::Ref(_), ValType::F32] => {
            a.movd_rx(Gp::Rdx, Xmm::Xmm0);
            a.mov_rr(true, Gp::Rsi, Gp::Rdi);
        }
        [ValType::Ref(_), ValType::F64] => {
            a.movq_rx(Gp::Rdx, Xmm::Xmm0);
            a.mov_rr(true, Gp::Rsi, Gp::Rdi);
        }
        other => return Err(format!("x64: unsupported import {k} params {other:?}")),
    }

    a.mov_ri32(
        Gp::Rdi,
        u32::try_from(k).map_err(|_| "import index overflow")?,
    );
    a.sub_ri(true, Gp::Rsp, 8);
    a.call_reloc(RelocTarget::Helper(HELPER_HOST_CALL));

    // Convert `Rax` (u64) to internal Wasm ABI return register (preserving R11d!).
    match ty.results.as_slice() {
        [] | [ValType::I64] | [ValType::Ref(_)] => {}
        [ValType::I32] => a.mov_rr(false, Gp::Rax, Gp::Rax), // zero-extend 32-bit
        [ValType::F32] => a.movd_xr(Xmm::Xmm0, Gp::Rax),
        [ValType::F64] => a.movq_xr(Xmm::Xmm0, Gp::Rax),
        other => return Err(format!("x64: unsupported import {k} results {other:?}")),
    }

    a.add_ri(true, Gp::Rsp, 8);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Emits `wt_initialize` (`HELPER_INITIALIZE`), entered on guest stack (`rsp % 16 == 8`) with
/// `R15 = Ctx*`, `R14 = mem_base`, `R13 = mem_len`.
pub fn emit_helper_initialize(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let lbl_trap_table_oob = a.new_label();
    let lbl_trap_oob = a.new_label();
    let mut need_table_oob = false;
    let mut need_oob = false;

    // 1. Initialize all non-folded globals in Ctx.
    let mut v128_iter = tables.v128_global_inits.iter().peekable();
    for (k, &init) in module.global_inits.iter().enumerate() {
        if module.global_consts[k].is_some() {
            continue;
        }
        let off = layout.global_offs[k];
        match init {
            X64ConstVal::I32(v) => {
                a.mov_ri32(Gp::Rax, v as u32);
                a.mov_mr(false, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::I64(v) => {
                a.mov_ri64(Gp::Rax, v as u64);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::F32(bits) => {
                a.mov_ri32(Gp::Rax, bits);
                a.mov_mr(false, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::F64(bits) => {
                a.mov_ri64(Gp::Rax, bits);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::RefNull => {
                a.xor_rr(false, Gp::Rax, Gp::Rax);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::FuncRef(f) => {
                a.mov_ri64(Gp::Rax, u64::from(f) + 1);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::V128(bytes) => {
                if bytes == [0u8; 16] {
                    a.xorps(Xmm::Xmm15, Xmm::Xmm15);
                } else {
                    let &(gk, ro_off) = v128_iter.next().expect("v128_global_inits mismatch");
                    debug_assert_eq!(gk, k);
                    a.movdqu_rip(Xmm::Xmm15, RelocTarget::Rodata(ro_off));
                }
                a.movdqu_mx(Mem::base_disp(Gp::R15, off), Xmm::Xmm15);
            }
        }
    }

    // 3. Explicit non-null initializers for dynamic tables (`table_inits` from function-references).
    for (&tbl, meta) in &module.dyn_tables {
        if meta.init_val != 0 && meta.min_len > 0 {
            let (desc_off, _, _) = layout.dyn_tables[&tbl];
            let lp = a.new_label();
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, desc_off));
            a.mov_ri64(Gp::Rax, meta.init_val);
            a.xor_rr(false, Gp::Rcx, Gp::Rcx);
            a.mov_ri32(Gp::Rdx, meta.min_len);
            a.bind(lp);
            a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
            a.add_ri(true, Gp::Rcx, 1);
            a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
            a.jcc(Cond::B, lp);
        }
    }

    // 3a. Active element bounds checks using 64-bit unsigned arithmetic `(base + len) > table_len`.
    for &(table_len, base_u64, elem_len) in &module.active_elem_checks {
        if base_u64
            .checked_add(u64::from(elem_len))
            .is_none_or(|end| end > u64::from(table_len))
        {
            need_table_oob = true;
            a.jmp(lbl_trap_table_oob);
        }
    }
    // 3b. Active element segments targeting dynamic tables (bounds-checked + copied into `Ctx` table buffer).
    for &(tbl, dst_off, ro_off, elem_len) in &tables.active_dyn_elem_inits {
        let (desc_off, min_len, _) = layout.dyn_tables[&tbl];
        if dst_off
            .checked_add(u64::from(elem_len))
            .is_none_or(|end| end > u64::from(min_len))
        {
            need_table_oob = true;
            a.jmp(lbl_trap_table_oob);
            continue;
        }
        if elem_len > 0 {
            let lp = a.new_label();
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, desc_off));
            if dst_off > 0 {
                a.mov_ri64(Gp::Rax, dst_off * 8);
                a.add_rr(true, Gp::Rdi, Gp::Rax);
            }
            a.lea_rip(Gp::Rsi, RelocTarget::Rodata(ro_off));
            a.xor_rr(false, Gp::Rcx, Gp::Rcx);
            a.mov_ri32(Gp::Rdx, elem_len);
            a.bind(lp);
            a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 4, 0));
            a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
            a.add_ri(true, Gp::Rcx, 1);
            a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
            a.jcc(Cond::B, lp);
        }
    }
    // 3c. Mark active/declarative element & active data segments implicitly dropped per Wasm spec.
    if let Some(elem_drop_off) = layout.elem_dropped_off {
        for (ei, seg) in module.elem_segs.iter().enumerate() {
            if !seg.is_passive {
                a.mov_mi8(Mem::base_disp(Gp::R15, elem_drop_off + (ei as i32)), 1);
            }
        }
    }
    if let Some(data_drop_off) = layout.data_dropped_off {
        for &(_, _, di) in &module.active_data_segs {
            a.mov_mi8(Mem::base_disp(Gp::R15, data_drop_off + (di as i32)), 1);
        }
    }

    // 4. Active data segments in declaration order:
    //    Check `mem_off + (len as u64) > mem_len` FIRST (even when `len == 0`!),
    //    then copy ONLY if `len > 0` so `base == NULL` is never dereferenced.
    for &(mem_idx, mem_off, ro_off, seg_len) in &tables.data_segs {
        need_oob = true;
        let Some(end) = mem_off.checked_add(u64::from(seg_len)) else {
            a.jmp(lbl_trap_oob);
            continue;
        };
        if mem_idx == 0 {
            a.mov_ri64(Gp::Rax, end);
            a.cmp_rr(true, Gp::Rax, Gp::R13);
            a.jcc(Cond::A, lbl_trap_oob);
            if seg_len > 0 {
                a.mov_ri64(Gp::Rdi, mem_off);
                a.add_rr(true, Gp::Rdi, Gp::R14);
                a.lea_rip(Gp::Rsi, RelocTarget::Rodata(ro_off));
                a.mov_ri64(Gp::Rcx, u64::from(seg_len));
                a.rep_movsb();
            }
        } else {
            let desc_off = layout.mem_desc_off(mem_idx);
            a.mov_ri64(Gp::Rax, end);
            a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, desc_off + 8));
            a.jcc(Cond::A, lbl_trap_oob);
            if seg_len > 0 {
                a.mov_ri64(Gp::Rdi, mem_off);
                a.add_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, desc_off));
                a.lea_rip(Gp::Rsi, RelocTarget::Rodata(ro_off));
                a.mov_ri64(Gp::Rcx, u64::from(seg_len));
                a.rep_movsb();
            }
        }
    }

    // 4b. Load pinned global(s) into R12 (and R13 when `pinned_global_2` is set on fixed-size
    //     memories) AFTER active data segment bounds checks have consumed `R13 = mem_len`.
    emit_pinned_load(&mut a, module, layout);

    // 5. Optional start function (defined or imported `wt_import{k}` via `RelocTarget::Func`).
    if let Some(start_idx) = module.start {
        a.sub_ri(true, Gp::Rsp, 8);
        a.call_reloc(RelocTarget::Func(start_idx));
        a.add_ri(true, Gp::Rsp, 8);
        a.ret();
    } else {
        a.xor_rr(false, Gp::R11, Gp::R11);
        a.ret();
    }

    if need_table_oob {
        a.bind(lbl_trap_table_oob);
        a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
        a.ret();
    }
    if need_oob {
        a.bind(lbl_trap_oob);
        a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
        a.ret();
    }

    a.finish_labels()?;
    Ok(a)
}

/// Shared helper `HELPER_READ_GLOBAL(ctx [rdi], np [rsi], nl [rdx], val_out [rcx], kind_out [r8]) -> i32`.
/// Table-driven over `.rodata` 24-byte descriptors:
///   `+0: i32 name_rel_off, +4: u32 name_len, +8: u8 kind, +9: u8 is_const, +12: i32 ctx_off, +16: u64 const_bits`.
pub fn emit_helper_read_global(tables: &RuntimeTables) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    if tables.get_global_count == 0 {
        a.mov_ri32(Gp::Rax, 1);
        a.ret();
        a.finish_labels()?;
        return Ok(a);
    }

    let lbl_missing = a.new_label();
    let lbl_np_ok = a.new_label();
    let lbl_loop = a.new_label();
    let lbl_cmp = a.new_label();
    let lbl_next = a.new_label();
    let lbl_found = a.new_label();
    let lbl_live = a.new_label();
    let lbl_load64 = a.new_label();
    let lbl_have_val = a.new_label();
    let lbl_skip_val = a.new_label();
    let lbl_skip_kind = a.new_label();

    // (np == NULL && nl > 0) -> 1; (np == NULL && nl == 0) legally matches "" without deref!
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_np_ok);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_missing);
    a.bind(lbl_np_ok);

    // Save Rbx, Rbp, R12 (`24 B` — leaf, no calls).
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);

    a.lea_rip(Gp::Rbx, RelocTarget::Rodata(tables.get_global_table_off));
    a.mov_rr(true, Gp::Rbp, Gp::Rbx);
    a.mov_ri32(Gp::R9, tables.get_global_count);

    a.bind(lbl_loop);
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::Rbp, 4)); // name_len (zero-ext to 64-bit)
    a.cmp_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_next);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_found);

    a.movsxd_rm(Gp::R11, Mem::base_disp(Gp::Rbp, 0));
    a.add_rr(true, Gp::R11, Gp::Rbx);
    a.xor_rr(false, Gp::R10, Gp::R10);
    a.bind(lbl_cmp);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::R10, 1, 0));
    a.movzx_rm8(false, Gp::R12, Mem::indexed(Gp::R11, Gp::R10, 1, 0));
    a.cmp_rr(false, Gp::Rax, Gp::R12);
    a.jcc(Cond::Ne, lbl_next);
    a.add_ri(true, Gp::R10, 1);
    a.cmp_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_cmp);
    a.jmp(lbl_found);

    a.bind(lbl_next);
    a.add_ri(true, Gp::Rbp, 24);
    a.sub_ri(false, Gp::R9, 1);
    a.jcc(Cond::Ne, lbl_loop);

    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.bind(lbl_missing);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.bind(lbl_found);
    // r9d = kind, r10d = is_const
    a.movzx_rm8(false, Gp::R9, Mem::base_disp(Gp::Rbp, 8));
    a.movzx_rm8(false, Gp::R10, Mem::base_disp(Gp::Rbp, 9));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_live);
    // Folded constant (already stored 32-bit zero-extended for I32/F32, 0 for RefNull).
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rbp, 16));
    a.jmp(lbl_have_val);

    a.bind(lbl_live);
    a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::Rbp, 12)); // ctx_off
    // kind == 0 (I32) or kind == 2 (F32) => 32-bit zero-extended load; else 64-bit load.
    let lbl_load32 = a.new_label();
    a.cmp_ri(false, Gp::R9, 0);
    a.jcc(Cond::E, lbl_load32);
    a.cmp_ri(false, Gp::R9, 2);
    a.jcc(Cond::Ne, lbl_load64);
    a.bind(lbl_load32);
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));
    a.jmp(lbl_have_val);
    a.bind(lbl_load64);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0));

    a.bind(lbl_have_val);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_skip_val);
    a.mov_mr(true, Mem::base(Gp::Rcx), Gp::Rax);
    a.bind(lbl_skip_val);
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_skip_kind);
    a.mov_mr8(Mem::base(Gp::R8), Gp::R9);
    a.bind(lbl_skip_kind);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

// ---- The 18 Library Protocol v1 (Blot Guest ABI 8) C ABI Exports ----

fn emit_export_abi_version() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(16);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

fn emit_export_trap_message(tables: &RuntimeTables) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(96);
    let lbl_invalid = a.new_label();
    let lbl_skip_ptr = a.new_label();
    let lbl_skip_len = a.new_label();

    // Zero-extend `dil` defensively (caller may pass dirty upper bits in `edi`).
    a.movzx_rr8(Gp::Rax, Gp::Rdi);
    a.sub_ri(false, Gp::Rax, 1);
    a.cmp_ri(false, Gp::Rax, TRAP_MESSAGES.len() as i32);
    a.jcc(Cond::Ae, lbl_invalid);

    a.lea_rip(Gp::R8, RelocTarget::Rodata(tables.trap_table_off));
    a.movsxd_rm(Gp::R10, Mem::indexed(Gp::R8, Gp::Rax, 8, 0));
    a.add_rr(true, Gp::R10, Gp::R8);
    a.mov_rm(false, Gp::Rcx, Mem::indexed(Gp::R8, Gp::Rax, 8, 4));

    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_skip_ptr);
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::R10);
    a.bind(lbl_skip_ptr);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_skip_len);
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::Rcx);
    a.bind(lbl_skip_len);

    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.ret();

    a.bind(lbl_invalid);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_import_count(tables: &RuntimeTables) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(16);
    a.mov_ri32(Gp::Rax, tables.imp_count);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

fn emit_export_import_name(tables: &RuntimeTables) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    if tables.imp_count == 0 {
        a.mov_ri32(Gp::Rax, 1);
        a.ret();
        a.finish_labels()?;
        return Ok(a);
    }
    let lbl_invalid = a.new_label();
    let lbl_s1 = a.new_label();
    let lbl_s2 = a.new_label();
    let lbl_s3 = a.new_label();
    let lbl_s4 = a.new_label();

    a.mov_rr(false, Gp::Rax, Gp::Rdi); // zero-extend u32 idx
    a.cmp_ri(false, Gp::Rax, tables.imp_count as i32);
    a.jcc(Cond::Ae, lbl_invalid);

    a.lea_rip(Gp::R9, RelocTarget::Rodata(tables.imp_table_off));
    a.shl_imm(true, Gp::Rax, 4); // idx * 16
    a.add_rr(true, Gp::Rax, Gp::R9);

    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_s1);
    a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::Rax, 0));
    a.add_rr(true, Gp::R10, Gp::R9);
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::R10);
    a.bind(lbl_s1);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_s2);
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rax, 4));
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::R10);
    a.bind(lbl_s2);

    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_s3);
    a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::Rax, 8));
    a.add_rr(true, Gp::R10, Gp::R9);
    a.mov_mr(true, Mem::base(Gp::Rcx), Gp::R10);
    a.bind(lbl_s3);

    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_s4);
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::Rax, 12));
    a.mov_mr(true, Mem::base(Gp::R8), Gp::R10);
    a.bind(lbl_s4);

    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.ret();

    a.bind(lbl_invalid);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_new(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    isa: TargetIsa,
    hooks: RuntimeTestHooks,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(512);
    let lbl_fail_isa = a.new_label();
    let lbl_fail_mem_limit = a.new_label();
    let lbl_fail_mem_too_large = a.new_label();
    let lbl_fail_alloc_none = a.new_label();
    let lbl_fail_alloc_cleanup = a.new_label();
    let lbl_fail_init_cleanup = a.new_label();
    let lbl_cleanup_ctx = a.new_label();
    let lbl_fail_no_ctx = a.new_label();
    let lbl_ret_null = a.new_label();
    let lbl_cet_done = a.new_label();

    // SysV entry: rsp % 16 == 8. Push 6 callee-saved regs (48 B) + sub rsp, 56 => rsp % 16 == 0.
    // Strictly disjoint stack slots:
    //   [rsp + 0]  = host_mxcsr
    //   [rsp + 8]  = user_ctx
    //   [rsp + 16] = host_fn
    //   [rsp + 24] = mem_limit_pages
    //   [rsp + 32] = trap_out
    //   [rsp + 40] = saved_trap_byte
    //   [rsp + 48] = cpu_has_cet_ss
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 56);

    a.stmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rdi);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rsi);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rdx);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rcx);
    a.mov_mi32(false, Mem::base_disp(Gp::Rsp, 48), 0);

    // ---- Step A: Mandatory CPUID & zero-syscall CET SHSTK gate ----
    // 1. Leaf 0 -> preserve max_leaf in ebp (survives leaves 1, 0x8000_0000, 0x8000_0001).
    match hooks.cpuid {
        CpuidSource::Real => {
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.cpuid();
        }
        #[cfg(test)]
        CpuidSource::Mock { max_leaf, .. } => {
            a.mov_ri32(Gp::Rax, max_leaf);
        }
    }
    a.mov_rr(false, Gp::Rbp, Gp::Rax);
    a.cmp_ri(false, Gp::Rbp, 1);
    a.jcc(Cond::B, lbl_fail_isa);
    if isa.has_abm_bmi1 || isa.has_bmi2 || isa.has_avx2 {
        a.cmp_ri(false, Gp::Rbp, 7);
        a.jcc(Cond::B, lbl_fail_isa);
    }

    // 2. Leaf 1 -> require x86-64-v2 baseline: SSSE3 (bit 9), SSE4.1 (bit 19), SSE4.2 (bit 20), POPCNT (bit 23),
    //    plus OSXSAVE (bit 27) & AVX (bit 28) when AVX2 codegen is enabled.
    match hooks.cpuid {
        CpuidSource::Real => {
            a.mov_ri32(Gp::Rax, 1);
            a.cpuid();
        }
        #[cfg(test)]
        CpuidSource::Mock { ecx1, .. } => {
            a.mov_ri32(Gp::Rcx, ecx1);
        }
    }
    let mut baseline_mask: i32 = (1 << 9) | (1 << 19) | (1 << 20) | (1 << 23);
    if isa.has_avx2 {
        baseline_mask |= (1 << 27) | (1 << 28); // OSXSAVE + AVX
    }
    a.and_ri(false, Gp::Rcx, baseline_mask);
    a.cmp_ri(false, Gp::Rcx, baseline_mask);
    a.jcc(Cond::Ne, lbl_fail_isa);

    // 2b. Verify OS enabled XMM + YMM state saving via XGETBV(0) when AVX2 is enabled!
    if isa.has_avx2 {
        match hooks.cpuid {
            CpuidSource::Real => {
                a.xor_rr(false, Gp::Rcx, Gp::Rcx);
                a.xgetbv();
            }
            #[cfg(test)]
            CpuidSource::Mock { xcr0, .. } => {
                a.mov_ri32(Gp::Rax, xcr0);
            }
        }
        a.and_ri(false, Gp::Rax, 0x6);
        a.cmp_ri(false, Gp::Rax, 0x6);
        a.jcc(Cond::Ne, lbl_fail_isa);
    }

    // 3. Optional ABM (LZCNT) + BMI1 (TZCNT) + BMI2 (SHLX/SHRX/SARX/RORX) + AVX2 when compiled for x64-v3.
    if isa.has_abm_bmi1 {
        match hooks.cpuid {
            CpuidSource::Real => {
                a.mov_ri32(Gp::Rax, 0x8000_0000);
                a.cpuid();
            }
            #[cfg(test)]
            CpuidSource::Mock { ext_max, .. } => {
                a.mov_ri32(Gp::Rax, ext_max);
            }
        }
        // Unsigned comparison! Never signed `jl` because bit 31 is set.
        a.cmp_ri(false, Gp::Rax, 0x8000_0001u32 as i32);
        a.jcc(Cond::B, lbl_fail_isa);

        match hooks.cpuid {
            CpuidSource::Real => {
                a.mov_ri32(Gp::Rax, 0x8000_0001);
                a.cpuid();
            }
            #[cfg(test)]
            CpuidSource::Mock { ext_ecx, .. } => {
                a.mov_ri32(Gp::Rcx, ext_ecx);
            }
        }
        a.and_ri(false, Gp::Rcx, 1 << 5);
        a.jcc(Cond::E, lbl_fail_isa);
    }
    let mut need_ebx7: i32 = 0;
    if isa.has_abm_bmi1 {
        need_ebx7 |= 1 << 3; // BMI1
    }
    if isa.has_avx2 {
        need_ebx7 |= 1 << 5; // AVX2
    }
    if isa.has_bmi2 {
        need_ebx7 |= 1 << 8; // BMI2
    }
    if need_ebx7 != 0 {
        match hooks.cpuid {
            CpuidSource::Real => {
                a.mov_ri32(Gp::Rax, 7);
                a.xor_rr(false, Gp::Rcx, Gp::Rcx);
                a.cpuid();
            }
            #[cfg(test)]
            CpuidSource::Mock { ebx7, .. } => {
                a.mov_ri32(Gp::Rbx, ebx7);
            }
        }
        a.and_ri(false, Gp::Rbx, need_ebx7);
        a.cmp_ri(false, Gp::Rbx, need_ebx7);
        a.jcc(Cond::Ne, lbl_fail_isa);
    }

    // 4. Zero-syscall calling-thread CET SHSTK check (using preserved max_leaf in ebp even on x64-v2!).
    a.cmp_ri(false, Gp::Rbp, 7);
    a.jcc(Cond::B, lbl_cet_done);
    match hooks.cpuid {
        CpuidSource::Real => {
            a.mov_ri32(Gp::Rax, 7);
            a.xor_rr(false, Gp::Rcx, Gp::Rcx);
            a.cpuid();
        }
        #[cfg(test)]
        CpuidSource::Mock { ecx7, .. } => {
            a.mov_ri32(Gp::Rcx, ecx7);
        }
    }
    a.and_ri(false, Gp::Rcx, 1 << 7); // CET_SS
    a.jcc(Cond::E, lbl_cet_done);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 48), 1);
    emit_rdsspq(&mut a, Gp::Rax, hooks.rdssp);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_fail_isa);
    a.bind(lbl_cet_done);

    // ---- Step B: Memory limit validation BEFORE any mmap ----
    if let Some((mem_min, declared_max)) = module.memory {
        let lbl_have_lim = a.new_label();
        let default_lim = if module.memory64 {
            mem_min.max(131_072) // 8 GiB default reservation ceiling for memory64
        } else {
            65_536
        };
        let hard_cap = if module.memory64 { 1u64 << 28 } else { 65_536 };
        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 24)); // mem_limit_pages
        a.test_rr(true, Gp::Rbx, Gp::Rbx);
        a.jcc(Cond::Ne, lbl_have_lim);
        a.mov_ri64(Gp::Rbx, default_lim);
        a.bind(lbl_have_lim);

        a.mov_ri64(Gp::Rax, mem_min);
        a.cmp_rr(true, Gp::Rax, Gp::Rbx);
        a.jcc(Cond::A, lbl_fail_mem_limit);

        let clamped_decl = declared_max.min(hard_cap);
        a.mov_ri64(Gp::Rbp, clamped_decl);
        a.cmp_rr(true, Gp::Rbx, Gp::Rbp);
        a.cmovcc(Cond::B, true, Gp::Rbp, Gp::Rbx); // rbp = max_p = min(declared_max, limit, hard_cap)

        a.cmp_rr(true, Gp::Rax, Gp::Rbp);
        a.jcc(Cond::A, lbl_fail_mem_too_large);
    } else {
        a.xor_rr(false, Gp::Rbp, Gp::Rbp);
    }
    for em in &module.extra_memories {
        let hard_cap = if em.memory64 { 1u64 << 28 } else { 65_536 };
        if em.min_pages > hard_cap {
            a.jmp(lbl_fail_mem_too_large);
        }
    }

    // ---- Step C1: mmap Ctx ----
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri64(Gp::Rsi, layout.total_bytes as u64);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc_none);
    a.mov_rr(true, Gp::R15, Gp::Rax);

    a.mov_ri64(Gp::Rax, layout.total_bytes as u64);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, OFF_HANDLE_ALLOC_BYTES),
        Gp::Rax,
    );
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 8));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_USER_CTX), Gp::Rax);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_FN), Gp::Rax);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
    a.mov_mr8(Mem::base_disp(Gp::R15, OFF_CPU_HAS_CET_SS), Gp::Rax);

    // ---- Step C2: mmap dedicated guest stack (64 KiB PROT_NONE guard + 8 MiB RW) ----
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri64(Gp::Rsi, GUEST_STACK_TOTAL_BYTES as u64);
    a.mov_ri32(Gp::Rdx, PROT_NONE);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON_NORESERVE);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
    a.mov_rr(true, Gp::Rbx, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_BASE), Gp::Rbx);

    // mprotect(stack_base + 64 KiB, 8 MiB, PROT_RW)
    a.lea(
        true,
        Gp::Rdi,
        Mem::base_disp(Gp::Rbx, GUEST_STACK_GUARD_BYTES as i32),
    );
    a.mov_ri64(Gp::Rsi, GUEST_STACK_USABLE_BYTES as u64);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    emit_mprotect_syscall(&mut a, hooks, MprotectSiteMarker::guest_stack());
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);

    a.lea(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rbx, GUEST_STACK_TOTAL_BYTES as i32),
    );
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP), Gp::Rax);
    a.lea(
        true,
        Gp::Rax,
        Mem::base_disp(
            Gp::Rbx,
            (GUEST_STACK_GUARD_BYTES + GUEST_STACK_FLOOR_HEADROOM) as i32,
        ),
    );
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_STACK_FLOOR), Gp::Rax);

    // ---- Step C3: Linear memory reservation & initial mprotect ----
    a.xor_rr(false, Gp::R14, Gp::R14);
    a.xor_rr(false, Gp::R13, Gp::R13);
    if let Some((mem_min, _)) = module.memory {
        let lbl_mem_done = a.new_label();
        a.test_rr(true, Gp::Rbp, Gp::Rbp);
        a.jcc(Cond::E, lbl_mem_done);

        a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_MAX_PAGES), Gp::Rbp);
        // 64-bit shift so 65_536 << 16 == 4 GiB never truncates!
        a.mov_rr(true, Gp::Rbx, Gp::Rbp);
        a.shl_imm(true, Gp::Rbx, 16);

        a.xor_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(true, Gp::Rsi, Gp::Rbx);
        a.mov_ri32(Gp::Rdx, PROT_NONE);
        a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON_NORESERVE);
        a.mov_ri64(Gp::R8, u64::MAX);
        a.xor_rr(false, Gp::R9, Gp::R9);
        a.mov_ri32(Gp::Rax, SYS_MMAP);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);

        a.mov_rr(true, Gp::R14, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_BASE), Gp::R14);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
            Gp::Rbx,
        );

        let mem0_hard_cap = if module.memory64 { 1u64 << 28 } else { 65_536 };
        if mem_min > 0 && mem_min <= mem0_hard_cap {
            a.mov_ri64(Gp::R13, mem_min << 16);
            a.mov_rr(true, Gp::Rdi, Gp::R14);
            a.mov_rr(true, Gp::Rsi, Gp::R13);
            a.mov_ri32(Gp::Rdx, PROT_RW);
            emit_mprotect_syscall(&mut a, hooks, MprotectSiteMarker::initial_memory());
            a.cmp_ri(true, Gp::Rax, -4095);
            a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_LEN), Gp::R13);
        }
        a.bind(lbl_mem_done);
    }

    // ---- Step C3b: Additional linear memories `1..N` (`multi-memory`) ----
    for (mi, em) in module.extra_memories.iter().enumerate() {
        let desc_off = layout.extra_mem_offs[mi];
        let hard_cap = if em.memory64 { 1u64 << 28 } else { 65_536 };
        if em.min_pages > hard_cap {
            // Already rejected unconditionally in Step B above; skip dead compile-time shift.
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
        a.mov_ri64(Gp::Rax, eff_max);
        a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off + 16), Gp::Rax);
        let reserve_bytes = eff_max << 16;
        a.mov_ri64(Gp::Rbx, reserve_bytes);
        a.xor_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(true, Gp::Rsi, Gp::Rbx);
        a.mov_ri32(Gp::Rdx, PROT_NONE);
        a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON_NORESERVE);
        a.mov_ri64(Gp::R8, u64::MAX);
        a.xor_rr(false, Gp::R9, Gp::R9);
        a.mov_ri32(Gp::Rax, SYS_MMAP);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
        a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off + 24), Gp::Rbx);
        if em.min_pages > 0 {
            let init_len = em.min_pages << 16;
            a.mov_rr(true, Gp::Rdi, Gp::Rax);
            a.mov_ri64(Gp::Rsi, init_len);
            a.mov_ri32(Gp::Rdx, PROT_RW);
            a.mov_ri32(Gp::Rax, SYS_MPROTECT);
            a.syscall();
            a.cmp_ri(true, Gp::Rax, -4095);
            a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
            a.mov_ri64(Gp::Rax, init_len);
            a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off + 8), Gp::Rax);
        }
    }

    // ---- Step C3c: Exception arena slab (demand-paged RW, allocated ONLY for EH modules) ----
    if let Some(exn_off) = layout.exn_state_off {
        a.xor_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_ri64(Gp::Rsi, EXN_SLAB_BYTES as u64);
        a.mov_ri32(Gp::Rdx, PROT_RW);
        a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
        a.mov_ri64(Gp::R8, u64::MAX);
        a.xor_rr(false, Gp::R9, Gp::R9);
        a.mov_ri32(Gp::Rax, SYS_MMAP);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
        a.mov_mr(true, Mem::base_disp(Gp::R15, exn_off + 8), Gp::Rax);
        a.mov_ri64(Gp::Rax, 16);
        a.mov_mr(true, Mem::base_disp(Gp::R15, exn_off + 16), Gp::Rax);
        a.mov_ri64(Gp::Rax, EXN_SLAB_BYTES as u64);
        a.mov_mr(true, Mem::base_disp(Gp::R15, exn_off + 24), Gp::Rax);
    }

    // ---- Step C3d: Dynamic tables (demand-paged RW mmap, allocated ONLY when `dyn_tables` non-empty) ----
    for &(desc_off, min_len, max_len) in layout.dyn_tables.values() {
        let reserve_bytes = if max_len > 0 {
            ((u64::from(max_len) * 8) + 4095) & !4095
        } else {
            0
        };
        if reserve_bytes > 0 {
            a.xor_rr(false, Gp::Rdi, Gp::Rdi);
            a.mov_ri64(Gp::Rsi, reserve_bytes);
            a.mov_ri32(Gp::Rdx, PROT_RW);
            a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
            a.mov_ri64(Gp::R8, u64::MAX);
            a.xor_rr(false, Gp::R9, Gp::R9);
            a.mov_ri32(Gp::Rax, SYS_MMAP);
            a.syscall();
            a.cmp_ri(true, Gp::Rax, -4095);
            a.jcc(Cond::Ae, lbl_fail_alloc_cleanup);
            a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off), Gp::Rax);
            a.mov_ri32(Gp::Rax, min_len);
            a.mov_mr(false, Mem::base_disp(Gp::R15, desc_off + 8), Gp::Rax);
            a.mov_ri32(Gp::Rax, max_len);
            a.mov_mr(false, Mem::base_disp(Gp::R15, desc_off + 12), Gp::Rax);
            a.mov_ri64(Gp::Rax, reserve_bytes);
            a.mov_mr(true, Mem::base_disp(Gp::R15, desc_off + 16), Gp::Rax);
        }
    }

    // ---- Step D: Switch to guest stack and run wt_initialize ----
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 1);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::Rsp);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_mr(true, Mem::base_disp(Gp::Rax, -8), Gp::Rsp);
    a.lea(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -16));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    a.call_reloc(RelocTarget::Helper(HELPER_INITIALIZE));

    // Restore host stack, sync pinned global, clear in_call, restore host MXCSR.
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -8));
    emit_pinned_store(&mut a, module, layout);
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 0);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));

    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_fail_init_cleanup);

    a.mov_rr(true, Gp::Rax, Gp::R15);
    a.add_ri(true, Gp::Rsp, 56);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    // ---- Step E: Cleanup & error paths (trap byte saved BEFORE munmap syscalls!) ----
    a.bind(lbl_fail_isa);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 40), TRAP_UNSUPPORTED_ISA);
    a.jmp(lbl_fail_no_ctx);

    a.bind(lbl_fail_mem_limit);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 40), TRAP_MEM_LIMIT);
    a.jmp(lbl_fail_no_ctx);

    a.bind(lbl_fail_mem_too_large);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 40), TRAP_MEM_TOO_LARGE);
    a.jmp(lbl_fail_no_ctx);

    a.bind(lbl_fail_alloc_none);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 40), TRAP_ALLOC_FAILED);
    a.jmp(lbl_fail_no_ctx);

    a.bind(lbl_fail_alloc_cleanup);
    a.mov_mi8(Mem::base_disp(Gp::Rsp, 40), TRAP_ALLOC_FAILED);
    a.jmp(lbl_cleanup_ctx);

    a.bind(lbl_fail_init_cleanup);
    a.mov_ri32(Gp::Rax, u32::from(TRAP_UNREACHABLE));
    a.cmp_ri(false, Gp::R11, i32::from(INTERNAL_EXN_SIGNAL));
    a.cmovcc(Cond::E, false, Gp::R11, Gp::Rax);
    a.mov_mr8(Mem::base_disp(Gp::Rsp, 40), Gp::R11);

    a.bind(lbl_cleanup_ctx);
    emit_x64_cleanup_ctx(&mut a, Gp::R15, layout);

    a.bind(lbl_fail_no_ctx);
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 32));
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ret_null);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rsp, 40));
    a.mov_mr8(Mem::base(Gp::Rcx), Gp::Rax);
    a.bind(lbl_ret_null);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.add_ri(true, Gp::Rsp, 56);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

/// Zeroes `[Rdi .. Rdi + Rcx)` when `Rcx != 0`, then emits `lfence`.
/// Clobbers ONLY `Rdi`, `Rcx`, `Rax` (preserves `Rsi` and callee-saved registers).
fn emit_x64_scrub_rdi_rcx(a: &mut Asm) {
    let lbl_done = a.new_label();
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_done);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.rep_stosb();
    a.lfence();
    a.bind(lbl_done);
}

/// Unmaps all handle-owned allocations (and scrubs committed linear memory, active dynamic tables,
/// used exception slab, and `Ctx` before `munmap` when `layout.spectre_mitigations` is enabled).
fn emit_x64_cleanup_ctx(a: &mut Asm, ctx_reg: Gp, layout: &CtxLayout) {
    let spectre = layout.spectre_mitigations;
    let lbl_no_stack = a.new_label();
    let lbl_no_mem = a.new_label();

    a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, OFF_GUEST_STACK_BASE));
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_no_stack);
    a.mov_ri64(Gp::Rsi, GUEST_STACK_TOTAL_BYTES as u64);
    a.mov_ri32(Gp::Rax, SYS_MUNMAP);
    a.syscall();
    a.bind(lbl_no_stack);

    a.mov_rm(
        true,
        Gp::Rsi,
        Mem::base_disp(ctx_reg, OFF_MEM_RESERVE_BYTES),
    );
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_no_mem);
    if spectre {
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, OFF_MEM_BASE));
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(ctx_reg, OFF_MEM_LEN));
        emit_x64_scrub_rdi_rcx(a);
    }
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, OFF_MEM_BASE));
    a.mov_ri32(Gp::Rax, SYS_MUNMAP);
    a.syscall();
    a.bind(lbl_no_mem);

    for &desc_off in &layout.extra_mem_offs {
        let lbl_skip = a.new_label();
        a.mov_rm(true, Gp::Rsi, Mem::base_disp(ctx_reg, desc_off + 24));
        a.test_rr(true, Gp::Rsi, Gp::Rsi);
        a.jcc(Cond::E, lbl_skip);
        if spectre {
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, desc_off));
            a.mov_rm(true, Gp::Rcx, Mem::base_disp(ctx_reg, desc_off + 8));
            emit_x64_scrub_rdi_rcx(a);
        }
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, desc_off));
        a.mov_ri32(Gp::Rax, SYS_MUNMAP);
        a.syscall();
        a.bind(lbl_skip);
    }
    if let Some(exn_off) = layout.exn_state_off {
        let lbl_skip = a.new_label();
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, exn_off + 8));
        a.test_rr(true, Gp::Rdi, Gp::Rdi);
        a.jcc(Cond::E, lbl_skip);
        a.mov_ri64(Gp::Rsi, EXN_SLAB_BYTES as u64);
        if spectre {
            a.mov_rm(true, Gp::Rcx, Mem::base_disp(ctx_reg, exn_off + 16));
            a.cmp_rr(true, Gp::Rcx, Gp::Rsi);
            a.cmovcc(Cond::A, true, Gp::Rcx, Gp::Rsi);
            emit_x64_scrub_rdi_rcx(a);
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, exn_off + 8));
        }
        a.mov_ri32(Gp::Rax, SYS_MUNMAP);
        a.syscall();
        a.bind(lbl_skip);
    }
    for &(desc_off, _, _) in layout.dyn_tables.values() {
        let lbl_skip = a.new_label();
        a.mov_rm(true, Gp::Rsi, Mem::base_disp(ctx_reg, desc_off + 16));
        a.test_rr(true, Gp::Rsi, Gp::Rsi);
        a.jcc(Cond::E, lbl_skip);
        if spectre {
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, desc_off));
            a.mov_rm(false, Gp::Rcx, Mem::base_disp(ctx_reg, desc_off + 8));
            a.shl_imm(true, Gp::Rcx, 3);
            emit_x64_scrub_rdi_rcx(a);
        }
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(ctx_reg, desc_off));
        a.mov_ri32(Gp::Rax, SYS_MUNMAP);
        a.syscall();
        a.bind(lbl_skip);
    }

    // Load `OFF_HANDLE_ALLOC_BYTES` BEFORE scrubbing/unmapping `ctx_reg`!
    a.mov_rm(
        true,
        Gp::Rsi,
        Mem::base_disp(ctx_reg, OFF_HANDLE_ALLOC_BYTES),
    );
    if spectre {
        a.mov_rr(true, Gp::Rdi, ctx_reg);
        a.mov_rr(true, Gp::Rcx, Gp::Rsi);
        emit_x64_scrub_rdi_rcx(a);
    }
    a.mov_rr(true, Gp::Rdi, ctx_reg);
    a.mov_ri32(Gp::Rax, SYS_MUNMAP);
    a.syscall();
}

fn emit_export_drop(layout: &CtxLayout) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    let lbl_done = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_done);
    // Defense-in-depth: refuse to unmap while a call is active on this handle!
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_done);

    a.push(Gp::Rbx);
    a.mov_rr(true, Gp::Rbx, Gp::Rdi);
    emit_x64_cleanup_ctx(&mut a, Gp::Rbx, layout);
    a.pop(Gp::Rbx);
    a.bind(lbl_done);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_call_export(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
    hooks: RuntimeTestHooks,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(512);
    let lbl_ret_missing = a.new_label();
    let lbl_ret_in_call = a.new_label();
    let lbl_in_call_rc = a.new_label();
    let lbl_np_ok = a.new_label();

    // Precedence (1): h == NULL -> return 3 (outs untouched).
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_ret_missing);

    // Precedence (2): ctx->in_call != 0 -> *trap_out = TRAP_HOST (19), return 2.
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_ret_in_call);

    // Precedence (3a): (np == NULL && nl > 0) -> return 3 (outs untouched).
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_np_ok);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_ret_missing);
    a.bind(lbl_np_ok);

    if tables.call_export_count == 0 {
        a.bind(lbl_ret_missing);
        a.mov_ri32(Gp::Rax, 3);
        a.ret();

        a.bind(lbl_ret_in_call);
        a.test_rr(true, Gp::R9, Gp::R9);
        a.jcc(Cond::E, lbl_in_call_rc);
        a.mov_mi8(Mem::base(Gp::R9), TRAP_HOST);
        a.bind(lbl_in_call_rc);
        a.mov_ri32(Gp::Rax, 2);
        a.ret();

        a.finish_labels()?;
        return Ok(a);
    }

    // Push 6 callee-saved regs (48 B) + sub rsp, 56 => rsp % 16 == 0:
    //   [rsp + 0]  = host_mxcsr
    //   [rsp + 8]  = res_out (r8)
    //   [rsp + 16] = trap_out (r9)
    //   [rsp + 24] = h (rdi)
    //   [rsp + 32] = arg_bits (rcx)
    //   [rsp + 40] = ret_kind
    //   [rsp + 48] = pad
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 56);

    a.stmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::R8);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::R9);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rdi);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rcx);

    let lbl_loop = a.new_label();
    let lbl_cmp = a.new_label();
    let lbl_next = a.new_label();
    let lbl_found = a.new_label();
    let lbl_not_found_pop = a.new_label();
    let lbl_ret_pop = a.new_label();
    let lbl_no_cet = a.new_label();
    let lbl_trap_isa_pop = a.new_label();
    let lbl_isa_rc = a.new_label();

    // Precedence (3b): scan `.rodata` call_export_table (16 B entries):
    //   +0: i32 name_rel_off, +4: u32 name_len, +8: i32 fn_rel_off, +12: u8 arg_kind, +13: u8 ret_kind
    a.lea_rip(Gp::Rbx, RelocTarget::Rodata(tables.call_export_table_off));
    a.mov_rr(true, Gp::Rbp, Gp::Rbx);
    a.mov_ri32(Gp::R10, tables.call_export_count);

    a.bind(lbl_loop);
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::Rbp, 4));
    a.cmp_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_next);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_found);

    a.movsxd_rm(Gp::R11, Mem::base_disp(Gp::Rbp, 0));
    a.add_rr(true, Gp::R11, Gp::Rbx);
    a.xor_rr(false, Gp::R8, Gp::R8);
    a.bind(lbl_cmp);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::R8, 1, 0));
    a.movzx_rm8(false, Gp::Rcx, Mem::indexed(Gp::R11, Gp::R8, 1, 0));
    a.cmp_rr(false, Gp::Rax, Gp::Rcx);
    a.jcc(Cond::Ne, lbl_next);
    a.add_ri(true, Gp::R8, 1);
    a.cmp_rr(true, Gp::R8, Gp::Rdx);
    a.jcc(Cond::B, lbl_cmp);
    a.jmp(lbl_found);

    a.bind(lbl_next);
    a.add_ri(true, Gp::Rbp, 16);
    a.sub_ri(false, Gp::R10, 1);
    a.jcc(Cond::Ne, lbl_loop);
    a.jmp(lbl_not_found_pop);

    // Export matched at [rbp]!
    a.bind(lbl_found);
    a.mov_rm(true, Gp::R15, Mem::base_disp(Gp::Rsp, 24)); // Ctx*
    a.movzx_rm8(false, Gp::R10, Mem::base_disp(Gp::Rbp, 12)); // arg_kind
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rbp, 13)); // ret_kind
    a.mov_mr(false, Mem::base_disp(Gp::Rsp, 40), Gp::Rax);
    a.movsxd_rm(Gp::Rax, Mem::base_disp(Gp::Rbp, 8));
    a.add_rr(true, Gp::Rbx, Gp::Rax); // rbx = target function vaddr

    // Precedence (4): CET SHSTK check STRICTLY AFTER export resolution!
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::R15, OFF_CPU_HAS_CET_SS));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_no_cet);
    emit_rdsspq(&mut a, Gp::Rax, hooks.rdssp);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_trap_isa_pop);
    a.bind(lbl_no_cet);

    // Enter guest context.
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 1);
    a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    emit_pinned_load(&mut a, module, layout);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::Rsp);

    // Marshal arg_bits from [rsp + 32] BEFORE switching rsp!
    let lbl_arg_i32 = a.new_label();
    let lbl_arg_i64 = a.new_label();
    let lbl_arg_f32 = a.new_label();
    let lbl_arg_f64 = a.new_label();
    let lbl_arg_done = a.new_label();
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 32));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_arg_done);
    a.cmp_ri(false, Gp::R10, ExportValKind::I32 as i32);
    a.jcc(Cond::E, lbl_arg_i32);
    a.cmp_ri(false, Gp::R10, ExportValKind::I64OrRef as i32);
    a.jcc(Cond::E, lbl_arg_i64);
    a.cmp_ri(false, Gp::R10, ExportValKind::F32 as i32);
    a.jcc(Cond::E, lbl_arg_f32);
    a.jmp(lbl_arg_f64);

    a.bind(lbl_arg_i32);
    a.mov_rr(false, Gp::Rdi, Gp::Rcx); // 32-bit zero-ext
    a.jmp(lbl_arg_done);

    a.bind(lbl_arg_i64);
    a.mov_rr(true, Gp::Rdi, Gp::Rcx);
    a.jmp(lbl_arg_done);

    a.bind(lbl_arg_f32);
    a.movd_xr(Xmm::Xmm0, Gp::Rcx);
    a.jmp(lbl_arg_done);

    a.bind(lbl_arg_f64);
    a.movq_xr(Xmm::Xmm0, Gp::Rcx);

    a.bind(lbl_arg_done);
    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_mr(true, Mem::base_disp(Gp::Rdx, -8), Gp::Rsp);
    a.lea(true, Gp::Rsp, Mem::base_disp(Gp::Rdx, -16));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    a.call_reg(Gp::Rbx);

    // Return to host stack, sync pinned global, clear in_call, restore host MXCSR (preserve Rax, Xmm0, R11d!).
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rcx, -8));
    emit_pinned_store(&mut a, module, layout);
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 0);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));

    let lbl_trapped = a.new_label();
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_trapped);

    // Unpack return value based on ret_kind in [rsp + 40].
    let lbl_ret_void = a.new_label();
    let lbl_ret_i32 = a.new_label();
    let lbl_ret_f32 = a.new_label();
    let lbl_ret_f64 = a.new_label();
    let lbl_ret_store = a.new_label();
    let lbl_ret_ok = a.new_label();
    a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::Rsp, 40));
    a.test_rr(false, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_ret_void);
    a.cmp_ri(false, Gp::Rcx, ExportValKind::I32 as i32);
    a.jcc(Cond::E, lbl_ret_i32);
    a.cmp_ri(false, Gp::Rcx, ExportValKind::I64OrRef as i32);
    a.jcc(Cond::E, lbl_ret_store);
    a.cmp_ri(false, Gp::Rcx, ExportValKind::F32 as i32);
    a.jcc(Cond::E, lbl_ret_f32);
    a.jmp(lbl_ret_f64);

    a.bind(lbl_ret_void);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.jmp(lbl_ret_store);

    a.bind(lbl_ret_i32);
    a.mov_rr(false, Gp::Rax, Gp::Rax); // 32-bit zero-ext
    a.jmp(lbl_ret_store);

    a.bind(lbl_ret_f32);
    a.movd_rx(Gp::Rax, Xmm::Xmm0);
    a.jmp(lbl_ret_store);

    a.bind(lbl_ret_f64);
    a.movq_rx(Gp::Rax, Xmm::Xmm0);

    a.bind(lbl_ret_store);
    a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_ret_ok);
    a.mov_mr(true, Mem::base(Gp::R8), Gp::Rax);
    a.bind(lbl_ret_ok);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.jmp(lbl_ret_pop);

    // Trap path (res_out left untouched!).
    let lbl_trap_rc = a.new_label();
    a.bind(lbl_trapped);
    a.mov_ri32(Gp::Rax, u32::from(TRAP_UNREACHABLE));
    a.cmp_ri(false, Gp::R11, i32::from(INTERNAL_EXN_SIGNAL));
    a.cmovcc(Cond::E, false, Gp::R11, Gp::Rax);
    a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::Rsp, 16));
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_trap_rc);
    a.mov_mr8(Mem::base(Gp::R9), Gp::R11);
    a.bind(lbl_trap_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.mov_ri32(Gp::Rcx, 2);
    a.cmp_ri(false, Gp::R11, i32::from(TRAP_HOST));
    a.cmovcc(Cond::E, false, Gp::Rax, Gp::Rcx);
    a.jmp(lbl_ret_pop);

    a.bind(lbl_trap_isa_pop);
    a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::Rsp, 16));
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_isa_rc);
    a.mov_mi8(Mem::base(Gp::R9), TRAP_UNSUPPORTED_ISA);
    a.bind(lbl_isa_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.jmp(lbl_ret_pop);

    a.bind(lbl_not_found_pop);
    a.mov_ri32(Gp::Rax, 3);

    a.bind(lbl_ret_pop);
    a.add_ri(true, Gp::Rsp, 56);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    // Early-return stubs (before push).
    a.bind(lbl_ret_missing);
    a.mov_ri32(Gp::Rax, 3);
    a.ret();

    a.bind(lbl_ret_in_call);
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_in_call_rc);
    a.mov_mi8(Mem::base(Gp::R9), TRAP_HOST);
    a.bind(lbl_in_call_rc);
    a.mov_ri32(Gp::Rax, 2);
    a.ret();

    a.finish_labels()?;
    Ok(a)
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

/// Emits top-level `blot_aot_call_allocate`, `blot_aot_call_reset`, or `blot_aot_call_finish_invocation`.
fn emit_export_call_fixed(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    export_name: &str,
    has_i32_arg: bool,
    hooks: RuntimeTestHooks,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let expect_param = has_i32_arg.then_some(ValType::I32);
    // Precedence (1): missing/incompatible export -> return 3 (outs untouched).
    let Some(target_fn) = find_fixed_export(module, export_name, expect_param) else {
        a.mov_ri32(Gp::Rax, 3);
        a.ret();
        a.finish_labels()?;
        return Ok(a);
    };

    // SysV args:
    //   has_i32_arg == true:  (h: Rdi, arg: Esi, res_out: Rdx, trap_out: Rcx)
    //   has_i32_arg == false: (h: Rdi, res_out: Rsi, trap_out: Rdx)
    let (res_reg, trap_reg) = if has_i32_arg {
        (Gp::Rdx, Gp::Rcx)
    } else {
        (Gp::Rsi, Gp::Rdx)
    };

    let lbl_fail_host = a.new_label();
    let lbl_fail_host_rc = a.new_label();
    let lbl_no_cet = a.new_label();
    let lbl_fail_isa = a.new_label();
    let lbl_fail_isa_rc = a.new_label();
    let lbl_trapped = a.new_label();
    let lbl_trap_rc = a.new_label();
    let lbl_ret_ok = a.new_label();
    let lbl_ret_pop = a.new_label();

    // Precedence (2): h == NULL or ctx->in_call != 0 -> *trap_out = TRAP_HOST (19), return 2.
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_fail_host);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_fail_host);

    // Precedence (3): CET SHSTK active -> *trap_out = TRAP_UNSUPPORTED_ISA (22), return 1.
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_CPU_HAS_CET_SS));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_no_cet);
    emit_rdsspq(&mut a, Gp::Rax, hooks.rdssp);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_fail_isa);
    a.bind(lbl_no_cet);

    // Push 6 callee-saved regs (48 B) + sub rsp, 56 => rsp % 16 == 0:
    //   [rsp + 0]  = host_mxcsr
    //   [rsp + 8]  = res_out
    //   [rsp + 16] = trap_out
    //   [rsp + 24] = h
    //   [rsp + 32] = arg (if has_i32_arg)
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 56);

    a.stmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), res_reg);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), trap_reg);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rdi);
    if has_i32_arg {
        a.mov_mr(false, Mem::base_disp(Gp::Rsp, 32), Gp::Rsi);
    }

    a.mov_rr(true, Gp::R15, Gp::Rdi);
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 1);
    a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    emit_pinned_load(&mut a, module, layout);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::Rsp);

    if has_i32_arg {
        a.mov_rr(false, Gp::Rdi, Gp::Rsi); // zero-ext 32-bit arg BEFORE switching rsp
    }
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_mr(true, Mem::base_disp(Gp::Rax, -8), Gp::Rsp);
    a.lea(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -16));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    a.call_reloc(RelocTarget::Func(target_fn));

    // Return to host stack, sync pinned global, clear in_call, restore host MXCSR (preserve Rax & R11d!).
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rcx, -8));
    emit_pinned_store(&mut a, module, layout);
    a.mov_mi8(Mem::base_disp(Gp::R15, OFF_IN_CALL), 0);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));

    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_trapped);

    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ret_ok);
    a.mov_mr(false, Mem::base(Gp::Rdx), Gp::Rax); // 32-bit *res_out = eax
    a.bind(lbl_ret_ok);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.jmp(lbl_ret_pop);

    a.bind(lbl_trapped);
    a.mov_ri32(Gp::Rax, u32::from(TRAP_UNREACHABLE));
    a.cmp_ri(false, Gp::R11, i32::from(INTERNAL_EXN_SIGNAL));
    a.cmovcc(Cond::E, false, Gp::R11, Gp::Rax);
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 16));
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_trap_rc);
    a.mov_mr8(Mem::base(Gp::Rcx), Gp::R11);
    a.bind(lbl_trap_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.mov_ri32(Gp::Rcx, 2);
    a.cmp_ri(false, Gp::R11, i32::from(TRAP_HOST));
    a.cmovcc(Cond::E, false, Gp::Rax, Gp::Rcx);

    a.bind(lbl_ret_pop);
    a.add_ri(true, Gp::Rsp, 56);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    // Pre-push error stubs.
    a.bind(lbl_fail_host);
    a.test_rr(true, trap_reg, trap_reg);
    a.jcc(Cond::E, lbl_fail_host_rc);
    a.mov_mi8(Mem::base(trap_reg), TRAP_HOST);
    a.bind(lbl_fail_host_rc);
    a.mov_ri32(Gp::Rax, 2);
    a.ret();

    a.bind(lbl_fail_isa);
    a.test_rr(true, trap_reg, trap_reg);
    a.jcc(Cond::E, lbl_fail_isa_rc);
    a.mov_mi8(Mem::base(trap_reg), TRAP_UNSUPPORTED_ISA);
    a.bind(lbl_fail_isa_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_mem_data() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(64);
    let lbl_null = a.new_label();
    let lbl_null_s1 = a.new_label();
    let lbl_done = a.new_label();
    let lbl_s1 = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_null);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_s1);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_MEM_BASE));
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::Rax);
    a.bind(lbl_s1);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_MEM_LEN));
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::Rax);
    a.jmp(lbl_done);

    a.bind(lbl_null);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_null_s1);
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::Rax);
    a.bind(lbl_null_s1);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::Rax);

    a.bind(lbl_done);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_mem_grow() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(64);
    let lbl_fail = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_fail);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_fail);

    // Save R13 & R15 (16 B) + sub rsp, 8 => rsp % 16 == 0; leaves R14 completely untouched!
    a.push(Gp::R13);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 8);
    a.mov_rr(true, Gp::R15, Gp::Rdi);
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    a.mov_rr(true, Gp::Rdi, Gp::Rsi); // full 64-bit delta!
    a.call_reloc(RelocTarget::Helper(HELPER_GROW_CORE));
    a.add_ri(true, Gp::Rsp, 8);
    a.pop(Gp::R15);
    a.pop(Gp::R13);
    a.ret();

    a.bind(lbl_fail);
    a.mov_ri64(Gp::Rax, u64::MAX);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_get_global() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(48);
    let lbl_missing = a.new_label();
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_missing);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_missing);
    a.jmp_reloc(RelocTarget::Helper(HELPER_READ_GLOBAL));
    a.bind(lbl_missing);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

fn emit_export_set_global(tables: &RuntimeTables) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    if tables.set_global_count == 0 {
        a.mov_ri32(Gp::Rax, 1);
        a.ret();
        a.finish_labels()?;
        return Ok(a);
    }

    let lbl_fail = a.new_label();
    let lbl_np_ok = a.new_label();
    let lbl_loop = a.new_label();
    let lbl_cmp = a.new_label();
    let lbl_next = a.new_label();
    let lbl_found = a.new_label();
    let lbl_store64 = a.new_label();
    let lbl_done = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_fail);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rdi, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_fail);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_np_ok);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_fail);
    a.bind(lbl_np_ok);

    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);

    // Table entry (16 B): +0: i32 name_rel_off, +4: u32 name_len, +8: i32 ctx_off, +12: u8 is64
    a.lea_rip(Gp::Rbx, RelocTarget::Rodata(tables.set_global_table_off));
    a.mov_rr(true, Gp::Rbp, Gp::Rbx);
    a.mov_ri32(Gp::R9, tables.set_global_count);

    a.bind(lbl_loop);
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::Rbp, 4));
    a.cmp_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_next);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_found);

    a.movsxd_rm(Gp::R11, Mem::base_disp(Gp::Rbp, 0));
    a.add_rr(true, Gp::R11, Gp::Rbx);
    a.xor_rr(false, Gp::R10, Gp::R10);
    a.bind(lbl_cmp);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::R10, 1, 0));
    a.movzx_rm8(false, Gp::R12, Mem::indexed(Gp::R11, Gp::R10, 1, 0));
    a.cmp_rr(false, Gp::Rax, Gp::R12);
    a.jcc(Cond::Ne, lbl_next);
    a.add_ri(true, Gp::R10, 1);
    a.cmp_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_cmp);
    a.jmp(lbl_found);

    a.bind(lbl_next);
    a.add_ri(true, Gp::Rbp, 16);
    a.sub_ri(false, Gp::R9, 1);
    a.jcc(Cond::Ne, lbl_loop);

    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.bind(lbl_fail);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.bind(lbl_found);
    a.movsxd_rm(Gp::R10, Mem::base_disp(Gp::Rbp, 8)); // ctx_off
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rbp, 12)); // is64
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_store64);
    a.mov_mr(false, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0), Gp::Rcx); // 32-bit store for I32/F32
    a.jmp(lbl_done);
    a.bind(lbl_store64);
    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::R10, 1, 0), Gp::Rcx); // 64-bit store for I64/F64/Ref

    a.bind(lbl_done);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_active_mem_data() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(80);
    let lbl_null = a.new_label();
    let lbl_null_s1 = a.new_label();
    let lbl_done = a.new_label();
    let lbl_s1 = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rdi, 0)); // ac->ctx
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_null);

    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_s1);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rdi, 8)); // ac->mem
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::Rax);
    a.bind(lbl_s1);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rcx, OFF_MEM_LEN));
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::Rax);
    a.jmp(lbl_done);

    a.bind(lbl_null);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_null_s1);
    a.mov_mr(true, Mem::base(Gp::Rsi), Gp::Rax);
    a.bind(lbl_null_s1);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_mr(true, Mem::base(Gp::Rdx), Gp::Rax);

    a.bind(lbl_done);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_active_fixed(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    export_name: &str,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    // Precedence (1): missing/incompatible export -> return 3 (outs untouched).
    let Some(target_fn) = find_fixed_export(module, export_name, Some(ValType::I32)) else {
        a.mov_ri32(Gp::Rax, 3);
        a.ret();
        a.finish_labels()?;
        return Ok(a);
    };

    // SysV args: (ac: Rdi, arg: Esi, res_out: Rdx, trap_out: Rcx)
    let lbl_fail_host = a.new_label();
    let lbl_fail_host_rc = a.new_label();
    let lbl_fail_stack = a.new_label();
    let lbl_fail_stack_rc = a.new_label();
    let lbl_trapped = a.new_label();
    let lbl_trap_rc = a.new_label();
    let lbl_ret_ok = a.new_label();
    let lbl_ret_pop = a.new_label();

    // Precedence (2): ac == NULL || ac->ctx == NULL || ctx->in_call == 0 -> TRAP_HOST (19), return 2.
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_fail_host);
    a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::Rdi, 0)); // ctx
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_fail_host);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::R8, OFF_IN_CALL));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_fail_host);

    // Precedence (3): active_depth >= 16 || suspended < stack_floor + 64 || suspended > guest_stack_top - 16
    // -> TRAP_CALL_STACK (8), return 1.
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R8, OFF_ACTIVE_DEPTH));
    a.cmp_ri(false, Gp::Rax, MAX_ACTIVE_REENTRY_DEPTH as i32);
    a.jcc(Cond::Ae, lbl_fail_stack);

    a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R8, OFF_ACTIVE_GUEST_RSP));
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R8, OFF_STACK_FLOOR));
    a.add_ri(true, Gp::R10, 64);
    a.cmp_rr(true, Gp::R9, Gp::R10);
    a.jcc(Cond::B, lbl_fail_stack);

    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R8, OFF_GUEST_STACK_TOP));
    a.sub_ri(true, Gp::R10, 16);
    a.cmp_rr(true, Gp::R9, Gp::R10);
    a.jcc(Cond::A, lbl_fail_stack);

    // Push 6 callee-saved regs (48 B) + sub rsp, 40 => rsp % 16 == 0:
    //   [rsp + 0]  = host_mxcsr
    //   [rsp + 8]  = res_out (rdx)
    //   [rsp + 16] = trap_out (rcx)
    //   [rsp + 24] = arg (esi)
    //   [rsp + 32] = pad
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 40);

    a.stmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rdx);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rcx);
    a.mov_mr(false, Mem::base_disp(Gp::Rsp, 24), Gp::Rsi);

    // Explicitly reload R15 = ac->ctx and R14 = ac->mem (callback may have repurposed R14/R15!).
    a.mov_rm(true, Gp::R15, Mem::base_disp(Gp::Rdi, 0));
    a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::Rdi, 8));
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
    emit_pinned_load(&mut a, module, layout);

    // new_guest_rsp = (suspended & !15) - 32 (guaranteed >= stack_floor, <= guest_stack_top - 48, % 16 == 0).
    a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::R15, OFF_ACTIVE_GUEST_RSP));
    a.and_ri(true, Gp::Rbx, -16);
    a.sub_ri(true, Gp::Rbx, 32);

    // Link record at [rbx + 0..16]: [0] = prev ctx->host_rsp, [8] = current host rsp.
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_HOST_RSP));
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, 0), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, 8), Gp::Rsp);

    // active_depth += 1; ctx->host_rsp = current host rsp.
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R15, OFF_ACTIVE_DEPTH));
    a.add_ri(false, Gp::Rax, 1);
    a.mov_mr(false, Mem::base_disp(Gp::R15, OFF_ACTIVE_DEPTH), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::Rsp);

    // Marshal arg BEFORE switching rsp!
    a.mov_rr(false, Gp::Rdi, Gp::Rsi);
    a.mov_rr(true, Gp::Rsp, Gp::Rbx);
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    a.call_reloc(RelocTarget::Func(target_fn));

    // Restore ctx->host_rsp, host rsp, active_depth -= 1, pinned global, host MXCSR (preserve Rax & R11d!).
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 0));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_HOST_RSP), Gp::Rcx);
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rsp, 8));
    a.mov_rm(false, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_ACTIVE_DEPTH));
    a.sub_ri(false, Gp::Rcx, 1);
    a.mov_mr(false, Mem::base_disp(Gp::R15, OFF_ACTIVE_DEPTH), Gp::Rcx);
    emit_pinned_store(&mut a, module, layout);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));

    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_trapped);

    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ret_ok);
    a.mov_mr(false, Mem::base(Gp::Rdx), Gp::Rax); // 32-bit *res_out = eax
    a.bind(lbl_ret_ok);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.jmp(lbl_ret_pop);

    a.bind(lbl_trapped);
    a.mov_ri32(Gp::Rax, u32::from(TRAP_UNREACHABLE));
    a.cmp_ri(false, Gp::R11, i32::from(INTERNAL_EXN_SIGNAL));
    a.cmovcc(Cond::E, false, Gp::R11, Gp::Rax);
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 16));
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_trap_rc);
    a.mov_mr8(Mem::base(Gp::Rcx), Gp::R11);
    a.bind(lbl_trap_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.mov_ri32(Gp::Rcx, 2);
    a.cmp_ri(false, Gp::R11, i32::from(TRAP_HOST));
    a.cmovcc(Cond::E, false, Gp::Rax, Gp::Rcx);

    a.bind(lbl_ret_pop);
    a.add_ri(true, Gp::Rsp, 40);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    // Pre-push error stubs.
    a.bind(lbl_fail_host);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_fail_host_rc);
    a.mov_mi8(Mem::base(Gp::Rcx), TRAP_HOST);
    a.bind(lbl_fail_host_rc);
    a.mov_ri32(Gp::Rax, 2);
    a.ret();

    a.bind(lbl_fail_stack);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_fail_stack_rc);
    a.mov_mi8(Mem::base(Gp::Rcx), TRAP_CALL_STACK);
    a.bind(lbl_fail_stack_rc);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}

fn emit_export_active_get_global() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(40);
    let lbl_missing = a.new_label();
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_missing);
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, 0)); // ac->ctx
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_missing);
    a.jmp_reloc(RelocTarget::Helper(HELPER_READ_GLOBAL));
    a.bind(lbl_missing);
    a.mov_ri32(Gp::Rax, 1);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Emits all 18 exported Library Protocol v1 functions in `EXPORT_NAMES` order.
pub fn emit_all_exports(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
    isa: TargetIsa,
    hooks: RuntimeTestHooks,
) -> Result<Vec<Asm>, String> {
    Ok(vec![
        emit_export_abi_version()?,
        emit_export_trap_message(tables)?,
        emit_export_import_count(tables)?,
        emit_export_import_name(tables)?,
        emit_export_new(module, layout, ro, isa, hooks)?,
        emit_export_drop(layout)?,
        emit_export_call_export(module, layout, ro, tables, hooks)?,
        emit_export_call_fixed(module, layout, ro, "blot:allocate", true, hooks)?,
        emit_export_call_fixed(module, layout, ro, "blot:reset", true, hooks)?,
        emit_export_call_fixed(module, layout, ro, "blot:finish_invocation", false, hooks)?,
        emit_export_mem_data()?,
        emit_export_mem_grow()?,
        emit_export_get_global()?,
        emit_export_set_global(tables)?,
        emit_export_active_mem_data()?,
        emit_export_active_fixed(module, layout, ro, "blot:allocate")?,
        emit_export_active_fixed(module, layout, ro, "blot:reset")?,
        emit_export_active_get_global()?,
    ])
}

/// Emits all `NUM_HELPERS` shared runtime helpers in helper-ID order (`0..NUM_HELPERS`).
pub fn emit_all_helpers(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
    hooks: RuntimeTestHooks,
) -> Result<Vec<Asm>, String> {
    let trunc_ops = [
        TruncOp::I32F32S,
        TruncOp::I32F32U,
        TruncOp::I32F64S,
        TruncOp::I32F64U,
        TruncOp::I64F32S,
        TruncOp::I64F32U,
        TruncOp::I64F64S,
        TruncOp::I64F64U,
        TruncOp::I32F32SatS,
        TruncOp::I32F32SatU,
        TruncOp::I32F64SatS,
        TruncOp::I32F64SatU,
        TruncOp::I64F32SatS,
        TruncOp::I64F32SatU,
        TruncOp::I64F64SatS,
        TruncOp::I64F64SatU,
    ];
    let mut helpers = Vec::with_capacity(super::NUM_HELPERS);
    for (i, &op) in trunc_ops.iter().enumerate() {
        debug_assert_eq!(HELPER_TRUNC_BASE + i as u32, op as u32);
        helpers.push(emit_trunc_helper(op, ro)?);
    }
    let fixed_mem = module.fixed32.is_some();
    let memory64 = module.memory64;
    let spectre = layout.spectre_mitigations;
    debug_assert_eq!(helpers.len(), HELPER_MEM_GROW as usize);
    helpers.push(emit_helper_mem_grow(memory64)?);
    debug_assert_eq!(helpers.len(), HELPER_MEM_COPY as usize);
    helpers.push(emit_helper_mem_copy(fixed_mem, memory64, spectre)?);
    debug_assert_eq!(helpers.len(), HELPER_MEM_FILL as usize);
    helpers.push(emit_helper_mem_fill(fixed_mem, memory64, spectre)?);
    debug_assert_eq!(helpers.len(), HELPER_HOST_CALL as usize);
    helpers.push(emit_helper_host_call(module, layout, ro)?);
    debug_assert_eq!(helpers.len(), HELPER_GROW_CORE as usize);
    helpers.push(emit_helper_grow_core(fixed_mem, hooks)?);
    debug_assert_eq!(helpers.len(), HELPER_INITIALIZE as usize);
    helpers.push(emit_helper_initialize(module, layout, tables)?);
    debug_assert_eq!(helpers.len(), HELPER_READ_GLOBAL as usize);
    helpers.push(emit_helper_read_global(tables)?);
    debug_assert_eq!(helpers.len(), super::HELPER_GOOSE_FMT_FLOAT as usize);
    let need_goose_float = module.target == super::module::X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "goose"
                && matches!(
                    f.as_str(),
                    "print_f64" | "print_f32" | "format_f64" | "format_f32"
                )
        });
    if need_goose_float {
        helpers.push(super::standalone_rt::emit_helper_goose_fmt_float()?);
    } else {
        let mut stub = Asm::with_capacity(4);
        stub.ret();
        helpers.push(stub);
    }
    debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_GROW as usize);
    if !module.extra_memories.is_empty() {
        helpers.push(emit_helper_extra_mem_grow()?);
        debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_COPY as usize);
        helpers.push(emit_helper_extra_mem_copy(spectre)?);
        debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_FILL as usize);
        helpers.push(emit_helper_extra_mem_fill(spectre)?);
    } else {
        for _ in 0..3 {
            let mut stub = Asm::with_capacity(4);
            stub.ret();
            helpers.push(stub);
        }
    }
    debug_assert_eq!(helpers.len(), super::HELPER_STANDALONE_FS as usize);
    let need_standalone_fs = module.target == super::module::X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "env"
                && matches!(
                    f.as_str(),
                    "gs_os_file_exists"
                        | "gs_os_is_dir"
                        | "gs_os_read_file"
                        | "gs_os_write_file"
                        | "gs_os_append_file"
                )
        });
    if need_standalone_fs {
        helpers.push(super::standalone_rt::emit_helper_standalone_fs()?);
    } else {
        let mut stub = Asm::with_capacity(4);
        stub.ret();
        helpers.push(stub);
    }
    debug_assert_eq!(
        helpers.len(),
        super::HELPER_STANDALONE_RESERVE_COPY as usize
    );
    let need_standalone_reserve = module.target == super::module::X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "env" && matches!(f.as_str(), "gs_os_arg" | "gs_os_getenv" | "gs_os_read_file")
        });
    if need_standalone_reserve {
        helpers.push(super::standalone_rt::emit_helper_standalone_reserve_copy(
            module,
        )?);
    } else {
        let mut stub = Asm::with_capacity(4);
        stub.ret();
        helpers.push(stub);
    }
    debug_assert_eq!(helpers.len(), HELPER_MEM_INIT as usize);
    if let Some(drop_off) = layout.data_dropped_off {
        helpers.push(emit_helper_mem_init(
            tables.passive_data_table_off,
            drop_off,
            spectre,
        )?);
    } else {
        let mut stub = Asm::with_capacity(4);
        stub.ret();
        helpers.push(stub);
    }
    debug_assert_eq!(helpers.len(), HELPER_TABLE_INIT as usize);
    if let Some(drop_off) = layout.elem_dropped_off {
        helpers.push(emit_helper_table_init(
            tables.elem_seg_table_off,
            drop_off,
            spectre,
        )?);
    } else {
        let mut stub = Asm::with_capacity(4);
        stub.ret();
        helpers.push(stub);
    }
    debug_assert_eq!(helpers.len(), HELPER_TABLE_COPY as usize);
    if !layout.dyn_tables.is_empty() {
        helpers.push(emit_helper_table_copy(spectre)?);
        debug_assert_eq!(helpers.len(), HELPER_TABLE_GROW as usize);
        helpers.push(emit_helper_table_grow(spectre)?);
        debug_assert_eq!(helpers.len(), HELPER_TABLE_FILL as usize);
        helpers.push(emit_helper_table_fill(spectre)?);
    } else {
        for _ in 0..3 {
            let mut stub = Asm::with_capacity(4);
            stub.ret();
            helpers.push(stub);
        }
    }
    debug_assert_eq!(helpers.len(), super::NUM_HELPERS);
    Ok(helpers)
}

/// Multi-memory `memory.grow(Rdi = delta, Rsi = desc_off) -> Rax (old_pages | u64::MAX)`.
fn emit_helper_extra_mem_grow() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    let lbl_ok = a.new_label();
    let lbl_fail = a.new_label();
    a.mov_rm(true, Gp::R10, Mem::indexed(Gp::R15, Gp::Rsi, 1, 8)); // old_len
    a.mov_rr(true, Gp::Rax, Gp::R10);
    a.shr_imm(true, Gp::Rax, 16); // old_pages
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_ok);
    a.mov_rm(true, Gp::Rcx, Mem::indexed(Gp::R15, Gp::Rsi, 1, 16)); // max_pages
    a.cmp_rr(true, Gp::Rdi, Gp::Rcx);
    a.jcc(Cond::A, lbl_fail);
    a.sub_rr(true, Gp::Rcx, Gp::Rdi);
    a.cmp_rr(true, Gp::Rax, Gp::Rcx);
    a.jcc(Cond::A, lbl_fail);
    // Preserve across Linux syscall: R8 = old_pages, R9 = delta_bytes, R10 = desc_off!
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.mov_rr(true, Gp::R9, Gp::Rdi);
    a.shl_imm(true, Gp::R9, 16);
    a.mov_rr(true, Gp::Rdi, Gp::R10); // old_len
    a.mov_rr(true, Gp::R10, Gp::Rsi); // desc_off
    a.add_rm(true, Gp::Rdi, Mem::indexed(Gp::R15, Gp::R10, 1, 0)); // base + old_len
    a.mov_rr(true, Gp::Rsi, Gp::R9);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::Rax, SYS_MPROTECT);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail);
    a.mov_rm(true, Gp::Rcx, Mem::indexed(Gp::R15, Gp::R10, 1, 8));
    a.add_rr(true, Gp::Rcx, Gp::R9);
    a.mov_mr(true, Mem::indexed(Gp::R15, Gp::R10, 1, 8), Gp::Rcx);
    a.mov_rr(true, Gp::Rax, Gp::R8);
    a.bind(lbl_ok);
    a.ret();
    a.bind(lbl_fail);
    a.mov_ri64(Gp::Rax, u64::MAX);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Multi-memory `memory.copy(Rdi = dst, Rsi = src, Rdx = len, Rcx = dst_desc_off, R8 = src_desc_off) -> R11d = 0 | TRAP_OOB`.
fn emit_helper_extra_mem_copy(spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(160);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_bwd = a.new_label();
    // Bounds check dst: dst + len <= dst_len
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::indexed(Gp::R15, Gp::Rcx, 1, 8));
    a.jcc(Cond::A, lbl_oob);
    // Bounds check src: src + len <= src_len
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::indexed(Gp::R15, Gp::R8, 1, 8));
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }
    a.add_rm(true, Gp::Rdi, Mem::indexed(Gp::R15, Gp::Rcx, 1, 0));
    a.add_rm(true, Gp::Rsi, Mem::indexed(Gp::R15, Gp::R8, 1, 0));
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::A, lbl_bwd);
    a.rep_movsb();
    a.jmp(lbl_ok);
    a.bind(lbl_bwd);
    if spectre {
        a.lfence();
    }
    a.sub_ri(true, Gp::Rcx, 1);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.mov_mr8(Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0), Gp::Rax);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::Ne, lbl_bwd);
    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Multi-memory `memory.fill(Rdi = dst, Rsi = val, Rdx = len, Rcx = desc_off) -> R11d = 0 | TRAP_OOB`.
fn emit_helper_extra_mem_fill(spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(96);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::indexed(Gp::R15, Gp::Rcx, 1, 8));
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }
    a.add_rm(true, Gp::Rdi, Mem::indexed(Gp::R15, Gp::Rcx, 1, 0));
    a.mov_rr(false, Gp::Rax, Gp::Rsi);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.rep_stosb();
    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Bulk `memory.init(Rdi = dst, Rsi = src, Rdx = len, Rcx = data_idx, R8 = mem_desc_off) -> R11d`.
fn emit_helper_mem_init(
    passive_table_off: u32,
    drop_off: i32,
    spectre: bool,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    let lbl_not_dropped = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();

    a.lea_rip(Gp::R9, RelocTarget::Rodata(passive_table_off));
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R9, Gp::Rcx, 8, 4)); // seg_len
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::R15, Gp::Rcx, 1, drop_off));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(lbl_not_dropped);

    // Check `src + len <= seg_len` (no u64 wrap).
    a.mov_rr(true, Gp::R10, Gp::Rsi);
    a.add_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::R10, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    // Check `dst + len <= mem_len` (no u64 wrap).
    a.mov_rr(true, Gp::R10, Gp::Rdi);
    a.add_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::R10, Mem::indexed(Gp::R15, Gp::R8, 1, 8));
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }

    a.movsxd_rm(Gp::R10, Mem::indexed(Gp::R9, Gp::Rcx, 8, 0));
    a.add_rr(true, Gp::R9, Gp::R10);
    a.add_rr(true, Gp::Rsi, Gp::R9);
    a.add_rm(true, Gp::Rdi, Mem::indexed(Gp::R15, Gp::R8, 1, 0));
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.rep_movsb();

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Bulk `table.init(Rdi = dst, Rsi = src, Rdx = len, Rcx = elem_idx, R8 = tbl_desc_off) -> R11d`.
fn emit_helper_table_init(
    elem_table_off: u32,
    drop_off: i32,
    spectre: bool,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(144);
    let lbl_not_dropped = a.new_label();
    let lbl_loop = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();

    a.lea_rip(Gp::R9, RelocTarget::Rodata(elem_table_off));
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R9, Gp::Rcx, 8, 4)); // elem_len
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::R15, Gp::Rcx, 1, drop_off));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(lbl_not_dropped);

    // Check `src + len <= elem_len`.
    a.mov_rr(true, Gp::R10, Gp::Rsi);
    a.add_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::R10, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    // Check `dst + len <= tbl_cur_len`.
    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R15, Gp::R8, 1, 8));
    a.mov_rr(true, Gp::R10, Gp::Rdi);
    a.add_rr(true, Gp::R10, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::R10, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }

    a.movsxd_rm(Gp::R10, Mem::indexed(Gp::R9, Gp::Rcx, 8, 0));
    a.add_rr(true, Gp::R9, Gp::R10); // seg_u32_base
    a.lea(true, Gp::R9, Mem::indexed(Gp::R9, Gp::Rsi, 4, 0));
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R15, Gp::R8, 1, 0)); // tbl_u64_base
    a.lea(true, Gp::Rax, Mem::indexed(Gp::Rax, Gp::Rdi, 8, 0));
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(lbl_loop);
    if spectre {
        a.lfence();
    }
    a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R9, Gp::Rcx, 4, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rax, Gp::Rcx, 8, 0), Gp::R10);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
    a.jcc(Cond::B, lbl_loop);

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Bulk `table.copy(Rdi = dst, Rsi = src, Rdx = len, Rcx = dst_desc_off, R8 = src_desc_off) -> R11d`.
fn emit_helper_table_copy(spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(144);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_fwd = a.new_label();

    a.mov_rm(false, Gp::R9, Mem::indexed(Gp::R15, Gp::Rcx, 1, 8));
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::A, lbl_oob);

    a.mov_rm(false, Gp::R9, Mem::indexed(Gp::R15, Gp::R8, 1, 8));
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }

    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R15, Gp::Rcx, 1, 0));
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::Rdi, 8, 0));
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R15, Gp::R8, 1, 0));
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R9, Gp::Rsi, 8, 0));

    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::A, lbl_bwd);
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(lbl_fwd);
    if spectre {
        a.lfence();
    }
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 8, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
    a.jcc(Cond::B, lbl_fwd);
    a.jmp(lbl_ok);

    a.bind(lbl_bwd);
    if spectre {
        a.lfence();
    }
    a.sub_ri(true, Gp::Rdx, 1);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rdx, 8, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rdx, 8, 0), Gp::Rax);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_bwd);

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Bulk `table.grow(Rdi = init_val, Rsi = delta, Rdx = tbl_desc_off) -> Rax (old_len | 0xffff_ffff)`.
fn emit_helper_table_grow(spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(96);
    let lbl_fail = a.new_label();
    let lbl_done = a.new_label();
    let lbl_loop = a.new_label();

    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R15, Gp::Rdx, 1, 8)); // old_len
    a.mov_rm(false, Gp::Rcx, Mem::indexed(Gp::R15, Gp::Rdx, 1, 12)); // max_len
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.add_rr(true, Gp::R8, Gp::Rsi);
    a.jcc(Cond::B, lbl_fail);
    a.cmp_rr(true, Gp::R8, Gp::Rcx);
    a.jcc(Cond::A, lbl_fail);

    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_done);
    if spectre {
        a.lfence();
    }

    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R15, Gp::Rdx, 1, 0)); // tbl_ptr
    a.mov_rr(true, Gp::Rcx, Gp::Rax);
    a.bind(lbl_loop);
    if spectre {
        a.lfence();
    }
    a.mov_mr(true, Mem::indexed(Gp::R9, Gp::Rcx, 8, 0), Gp::Rdi);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_rr(true, Gp::Rcx, Gp::R8);
    a.jcc(Cond::B, lbl_loop);
    a.mov_mr(false, Mem::indexed(Gp::R15, Gp::Rdx, 1, 8), Gp::R8);

    a.bind(lbl_done);
    a.ret();
    a.bind(lbl_fail);
    a.mov_ri64(Gp::Rax, u64::MAX);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Bulk `table.fill(Rdi = dst, Rsi = val, Rdx = len, Rcx = tbl_desc_off) -> R11d`.
fn emit_helper_table_fill(spectre: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(96);
    let lbl_oob = a.new_label();
    let lbl_ok = a.new_label();
    let lbl_loop = a.new_label();

    a.mov_rm(false, Gp::Rax, Mem::indexed(Gp::R15, Gp::Rcx, 1, 8)); // cur_len
    a.mov_rr(true, Gp::R8, Gp::Rdi);
    a.add_rr(true, Gp::R8, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::R8, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);

    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_ok);
    if spectre {
        a.lfence();
    }

    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R15, Gp::Rcx, 1, 0));
    a.lea(true, Gp::R9, Mem::indexed(Gp::R9, Gp::Rdi, 8, 0));
    a.xor_rr(false, Gp::R8, Gp::R8);
    a.bind(lbl_loop);
    if spectre {
        a.lfence();
    }
    a.mov_mr(true, Mem::indexed(Gp::R9, Gp::R8, 8, 0), Gp::Rsi);
    a.add_ri(true, Gp::R8, 1);
    a.cmp_rr(true, Gp::R8, Gp::Rdx);
    a.jcc(Cond::B, lbl_loop);

    a.bind(lbl_ok);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}
