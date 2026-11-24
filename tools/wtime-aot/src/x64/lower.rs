//! SSA IR -> x86-64 machine code lowering.
//!
//! Register conventions:
//! - Pinned: `R15 = Ctx*`, `R14 = mem_base`, `R13 = mem_len`, `R12 = pinned_global` (optional).
//! - Scratch (never allocated by regalloc):
//!   - `R11` / `Xmm15`: primary scratch, trap return code (`R11d`), and parallel-move cycle break.
//!   - `R10` / `Xmm14`: secondary scratch and parallel-move memory-to-memory copy temporary.
//! - Unified typed internal Wasm call ABI (`regalloc::layout_args` / `layout_rets`):
//!   - GP args: `Rdi, Rsi, Rdx, Rcx, R8, R9`, overflow on caller stack (8-byte aligned scalars, 16-byte aligned V128).
//!   - FP/V128 args: `Xmm0..Xmm7`, overflow on caller stack.
//!   - Trap return: `R11d` (`0` = ok, `1..22` = trap code).
//!   - GP results: `Rax, Rdx, Rcx, Rsi, Rdi, R8, R9`; FP/V128 results: `Xmm0..Xmm7`;
//!     overflow multi-value results stored into caller-owned stack buffer via `[r15 + OFF_RET_SPILL_PTR]`.

use std::collections::HashMap;

use super::asm::{Asm, Cond, Gp, Label, Mem, RelocTarget, Xmm};
use super::ir::{
    CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, IntBinOp, IrFunc, LoadKind, Op, StoreKind, Term,
    Ty, Val, ValRange,
};
use super::module::{
    CompileBudget, MAX_BLOCKS_PER_FN, MAX_FN_CODE_BYTES, MAX_FN_LOCAL_RODATA_BYTES,
};
use super::regalloc::{AbiSlot, AllocResult, Loc, RematConst, layout_args, layout_rets};
use super::{
    CtxLayout, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, HELPER_TRUNC_BASE,
    OFF_RET_SPILL_PTR, OFF_SCRATCH_TMP, OFF_STACK_FLOOR, RodataConsts, TRAP_CALL_STACK,
    TRAP_DIV_ZERO, TRAP_INDIRECT_CALL, TRAP_INT_OVERFLOW, TRAP_OOB, TRAP_TABLE_OOB,
    TRAP_UNREACHABLE,
};

const SCRATCH_GP1: Gp = Gp::R11;
const SCRATCH_GP2: Gp = Gp::R10;
const SCRATCH_FP1: Xmm = Xmm::Xmm15;
const SCRATCH_FP2: Xmm = Xmm::Xmm14;

/// Generalized location used by the parallel-move resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveLoc {
    Gp(Gp),
    Xmm(Xmm),
    /// `[rsp + disp]`
    Stack(i32),
    /// Pure constant source (never a destination; emitted after all reg/stack moves).
    Remat(RematConst),
}

impl From<Loc> for MoveLoc {
    fn from(l: Loc) -> Self {
        match l {
            Loc::Gp(g) => MoveLoc::Gp(g),
            Loc::Xmm(x) => MoveLoc::Xmm(x),
            Loc::Spill(off) => MoveLoc::Stack(off),
            Loc::Remat(rc) => MoveLoc::Remat(rc),
        }
    }
}

/// Emits a single move `dst <- src`.
/// Memory-to-memory copies use `SCRATCH_GP2` (`R10`) / `SCRATCH_FP2` (`Xmm14`) exclusively,
/// keeping `SCRATCH_GP1` (`R11`) / `SCRATCH_FP1` (`Xmm15`) completely disjoint for cycle breaking!
fn emit_single_move(asm: &mut Asm, dst: MoveLoc, src: MoveLoc, ty: Ty) {
    if dst == src {
        return;
    }
    let is64 = ty == Ty::I64;
    match (dst, src) {
        (MoveLoc::Gp(d), MoveLoc::Gp(s)) => asm.mov_rr(is64, d, s),
        (MoveLoc::Gp(d), MoveLoc::Stack(off)) => asm.mov_rm(is64, d, Mem::base_disp(Gp::Rsp, off)),
        (MoveLoc::Stack(off), MoveLoc::Gp(s)) => asm.mov_mr(is64, Mem::base_disp(Gp::Rsp, off), s),
        (MoveLoc::Xmm(d), MoveLoc::Xmm(s)) => asm.movaps_rr(d, s),
        (MoveLoc::Xmm(d), MoveLoc::Stack(off)) => {
            emit_fp_load(asm, d, Mem::base_disp(Gp::Rsp, off), ty)
        }
        (MoveLoc::Stack(off), MoveLoc::Xmm(s)) => {
            emit_fp_store(asm, Mem::base_disp(Gp::Rsp, off), s, ty)
        }
        (MoveLoc::Gp(d), MoveLoc::Remat(RematConst::I32(c))) => asm.mov_ri32(d, c as u32),
        (MoveLoc::Gp(d), MoveLoc::Remat(RematConst::I64(c))) => asm.mov_ri64(d, c as u64),
        (MoveLoc::Stack(off), MoveLoc::Remat(RematConst::I32(c))) => {
            asm.mov_mi32(false, Mem::base_disp(Gp::Rsp, off), c);
        }
        (MoveLoc::Stack(off), MoveLoc::Remat(RematConst::I64(c))) => {
            if let Ok(imm32) = i32::try_from(c) {
                asm.mov_mi32(true, Mem::base_disp(Gp::Rsp, off), imm32);
            } else {
                asm.mov_ri64(SCRATCH_GP2, c as u64);
                asm.mov_mr(true, Mem::base_disp(Gp::Rsp, off), SCRATCH_GP2);
            }
        }
        (MoveLoc::Xmm(d), MoveLoc::Remat(RematConst::F32(bits))) => {
            if bits == 0 {
                asm.xorps(d, d);
            } else {
                let loff = asm.intern_f32(bits);
                asm.movss_rip(d, RelocTarget::LocalRodata(loff));
            }
        }
        (MoveLoc::Xmm(d), MoveLoc::Remat(RematConst::F64(bits))) => {
            if bits == 0 {
                asm.xorps(d, d);
            } else {
                let loff = asm.intern_f64(bits);
                asm.movsd_rip(d, RelocTarget::LocalRodata(loff));
            }
        }
        (MoveLoc::Stack(off), MoveLoc::Remat(RematConst::F32(bits))) => {
            asm.mov_mi32(false, Mem::base_disp(Gp::Rsp, off), bits as i32);
        }
        (MoveLoc::Stack(off), MoveLoc::Remat(RematConst::F64(bits))) => {
            if let Ok(imm32) = i32::try_from(bits as i64) {
                asm.mov_mi32(true, Mem::base_disp(Gp::Rsp, off), imm32);
            } else {
                asm.mov_ri64(SCRATCH_GP2, bits);
                asm.mov_mr(true, Mem::base_disp(Gp::Rsp, off), SCRATCH_GP2);
            }
        }
        // Memory-to-memory through scratch2 (`R10` / `Xmm14`) so cycle-breaking scratch1 (`R11` / `Xmm15`) is never clobbered:
        (MoveLoc::Stack(doff), MoveLoc::Stack(soff)) => {
            if ty.is_fp() {
                emit_fp_load(asm, SCRATCH_FP2, Mem::base_disp(Gp::Rsp, soff), ty);
                emit_fp_store(asm, Mem::base_disp(Gp::Rsp, doff), SCRATCH_FP2, ty);
            } else {
                asm.mov_rm(is64, SCRATCH_GP2, Mem::base_disp(Gp::Rsp, soff));
                asm.mov_mr(is64, Mem::base_disp(Gp::Rsp, doff), SCRATCH_GP2);
            }
        }
        _ => unreachable!("mismatched move kinds: {dst:?} <- {src:?} ({ty:?})"),
    }
}

fn emit_fp_load(asm: &mut Asm, dst: Xmm, mem: Mem, ty: Ty) {
    match ty {
        Ty::F32 => asm.movss_xm(dst, mem),
        Ty::F64 => asm.movsd_xm(dst, mem),
        Ty::V128 => asm.movdqu_xm(dst, mem),
        _ => unreachable!(),
    }
}

fn emit_fp_store(asm: &mut Asm, mem: Mem, src: Xmm, ty: Ty) {
    match ty {
        Ty::F32 => asm.movss_mx(mem, src),
        Ty::F64 => asm.movsd_mx(mem, src),
        Ty::V128 => asm.movdqu_mx(mem, src),
        _ => unreachable!(),
    }
}

/// Verified O(M) parallel-move resolver (handles arbitrary disjoint cycles, stack swaps, and
/// duplicate sources) using source-use counts.
/// Uses `SCRATCH_GP1`/`SCRATCH_FP1` (`R11`/`Xmm15`) for cycle breaking and
/// `SCRATCH_GP2`/`SCRATCH_FP2` (`R10`/`Xmm14`) for memory-to-memory copies.
pub fn emit_parallel_moves(asm: &mut Asm, raw_moves: &[(MoveLoc, MoveLoc, Ty)]) {
    // Assert distinct destinations across ALL raw moves BEFORE filtering self-moves (`d == s`),
    // so a buggy caller passing `[(A, A), (A, B)]` trips the assertion instead of silently dropping `(A, A)`.
    if raw_moves.len() > 1 {
        let mut seen_dst: HashMap<MoveLoc, ()> = HashMap::with_capacity(raw_moves.len());
        for &(dst, _, _) in raw_moves {
            assert!(
                !matches!(dst, MoveLoc::Remat(_)),
                "parallel move destination cannot be Remat"
            );
            assert!(
                seen_dst.insert(dst, ()).is_none(),
                "parallel moves must have distinct destinations"
            );
        }
    }
    let mut moves: Vec<(MoveLoc, MoveLoc, Ty)> = Vec::with_capacity(raw_moves.len());
    let mut remat_moves: Vec<(MoveLoc, MoveLoc, Ty)> = Vec::new();
    for &(d, s, ty) in raw_moves {
        if d == s {
            continue;
        }
        if matches!(s, MoveLoc::Remat(_)) {
            remat_moves.push((d, s, ty));
        } else {
            moves.push((d, s, ty));
        }
    }
    let n = moves.len();
    if n > 0 {
        let mut active = vec![true; n];
        let mut remaining = n;
        let mut src_uses: HashMap<MoveLoc, usize> = HashMap::with_capacity(n);
        let mut dst_to_idx: HashMap<MoveLoc, usize> = HashMap::with_capacity(n);
        for (i, &(dst, src, _)) in moves.iter().enumerate() {
            *src_uses.entry(src).or_insert(0) += 1;
            let prev = dst_to_idx.insert(dst, i);
            assert!(
                prev.is_none(),
                "parallel moves must have distinct destinations"
            );
        }
        let mut ready: Vec<usize> = Vec::with_capacity(n);
        for (i, &(dst, _, _)) in moves.iter().enumerate() {
            if src_uses.get(&dst).copied().unwrap_or(0) == 0 {
                ready.push(i);
            }
        }
        let mut scan_cursor = 0usize;
        while remaining > 0 {
            while let Some(i) = ready.pop() {
                if !active[i] {
                    continue;
                }
                active[i] = false;
                remaining -= 1;
                let (dst, src, ty) = moves[i];
                emit_single_move(asm, dst, src, ty);
                if let Some(cnt) = src_uses.get_mut(&src) {
                    *cnt -= 1;
                    if *cnt == 0
                        && let Some(&producer) = dst_to_idx.get(&src)
                        && active[producer]
                    {
                        ready.push(producer);
                    }
                }
            }
            if remaining == 0 {
                break;
            }
            // All non-cycle trees have drained, so every remaining active node has `src_uses[dst] == 1`
            // (pigeonhole over distinct destinations) and forms disjoint cycles.
            while scan_cursor < n && !active[scan_cursor] {
                scan_cursor += 1;
            }
            let k = scan_cursor;
            let (_, src0, ty0) = moves[k];
            let tmp = if ty0.is_fp() {
                MoveLoc::Xmm(SCRATCH_FP1)
            } else {
                MoveLoc::Gp(SCRATCH_GP1)
            };
            debug_assert!(src0 != tmp, "parallel move cycle already in scratch");
            emit_single_move(asm, tmp, src0, ty0);
            moves[k].1 = tmp;
            *src_uses.entry(tmp).or_insert(0) += 1;
            if let Some(cnt) = src_uses.get_mut(&src0) {
                *cnt -= 1;
                if *cnt == 0
                    && let Some(&producer) = dst_to_idx.get(&src0)
                    && active[producer]
                {
                    ready.push(producer);
                }
            }
        }
    }
    // Emit constant-source moves strictly AFTER all reg/stack moves so they never overwrite a live source.
    for (dst, src, ty) in remat_moves {
        emit_single_move(asm, dst, src, ty);
    }
}

/// Exact Granlund–Montgomery unsigned division constants (Hacker's Delight Fig. 10-2)
/// for `w`-bit (`32` or `64`) unsigned `n / d` (`d >= 2`, non-power-of-two).
/// Returns `(magic, shift, add)`:
/// - `!add`: `q = mulhi_u(n, magic) >> shift`
/// - `add`:  `t = mulhi_u(n, magic); q = (((n - t) >> 1) + t) >> (shift - 1)`
pub(super) fn magic_u(w: u32, d: u64) -> (u64, u32, bool) {
    let two_w_1: u128 = 1u128 << (w - 1);
    let max_u: u128 = (two_w_1 << 1) - 1;
    let d128 = d as u128;
    let nc = max_u - ((max_u + 1) % d128);
    let mut p = w - 1;
    let mut q1 = two_w_1 / nc;
    let mut r1 = two_w_1 - q1 * nc;
    let mut q2 = (two_w_1 - 1) / d128;
    let mut r2 = (two_w_1 - 1) - q2 * d128;
    let mut add = false;
    loop {
        p += 1;
        if r1 >= nc - r1 {
            q1 = 2 * q1 + 1;
            r1 = 2 * r1 - nc;
        } else {
            q1 *= 2;
            r1 *= 2;
        }
        if r2 + 1 >= d128 - r2 {
            if q2 >= two_w_1 - 1 {
                add = true;
            }
            q2 = 2 * q2 + 1;
            r2 = 2 * r2 + 1 - d128;
        } else {
            if q2 >= two_w_1 {
                add = true;
            }
            q2 *= 2;
            r2 = 2 * r2 + 1;
        }
        let delta = d128 - 1 - r2;
        if !(p < 2 * w && (q1 < delta || (q1 == delta && r1 == 0))) {
            break;
        }
    }
    let magic = ((q2 + 1) & max_u) as u64;
    (magic, p - w, add)
}

/// Exact Granlund–Montgomery signed division constants (Hacker's Delight Fig. 10-1)
/// for `w`-bit (`32` or `64`) signed `n / d` (`|d| >= 2`, `d != MIN`).
/// Returns `(magic_sign_ext_i64, shift, add_sub)` where `add_sub` is `+1`, `-1`, or `0`.
pub(super) fn magic_s(w: u32, d: i64) -> (i64, u32, i8) {
    let two_w_1: u128 = 1u128 << (w - 1);
    let ad = d.unsigned_abs() as u128;
    let t = two_w_1 + u128::from(d < 0);
    let anc = t - 1 - (t % ad);
    let mut p = w - 1;
    let mut q1 = two_w_1 / anc;
    let mut r1 = two_w_1 - q1 * anc;
    let mut q2 = two_w_1 / ad;
    let mut r2 = two_w_1 - q2 * ad;
    loop {
        p += 1;
        q1 *= 2;
        r1 *= 2;
        if r1 >= anc {
            q1 += 1;
            r1 -= anc;
        }
        q2 *= 2;
        r2 *= 2;
        if r2 >= ad {
            q2 += 1;
            r2 -= ad;
        }
        let delta = ad - r2;
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }
    let mut mag = (q2 + 1) as u64;
    if d < 0 {
        mag = mag.wrapping_neg();
    }
    let m_signed = if w == 32 {
        i64::from(mag as u32 as i32)
    } else {
        mag as i64
    };
    let add_sub = if d > 0 && m_signed < 0 {
        1
    } else if d < 0 && m_signed > 0 {
        -1
    } else {
        0
    };
    (m_signed, p - w, add_sub)
}

fn cmp_to_cond(op: CmpOp) -> Cond {
    match op {
        CmpOp::Eq => Cond::E,
        CmpOp::Ne => Cond::Ne,
        CmpOp::LtS => Cond::L,
        CmpOp::LtU => Cond::B,
        CmpOp::GtS => Cond::G,
        CmpOp::GtU => Cond::A,
        CmpOp::LeS => Cond::Le,
        CmpOp::LeU => Cond::Be,
        CmpOp::GeS => Cond::Ge,
        CmpOp::GeU => Cond::Ae,
    }
}

/// Resolved CPU ISA feature flags enforced by both lowering and `blot_aot_new`'s CPUID gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetIsa {
    /// Emit `lzcnt`/`tzcnt` when true; emit branchless portable `bsr`/`bsf` + `cmovz` when false.
    pub has_abm_bmi1: bool,
    /// Emit BMI2 `shlx`/`shrx`/`sarx`/`rorx` when true (eliminating `RCX` shift clobbers on `x64-v3`).
    pub has_bmi2: bool,
}

struct Lower<'a> {
    ir: &'a IrFunc,
    alloc: &'a AllocResult,
    layout: &'a CtxLayout,
    ro: &'a RodataConsts,
    isa: TargetIsa,
    /// Optional per-function transitive trap-capability table (Step 8b, indexed by Wasm `func_index`).
    fn_can_trap: Option<&'a [bool]>,
    asm: Asm,
    bb_labels: Vec<Label>,
    trap_oob: Option<Label>,
    trap_table_oob: Option<Label>,
    trap_ci: Option<Label>,
    trap_div0: Option<Label>,
    trap_int_ovf: Option<Label>,
    trap_unreach: Option<Label>,
    trap_prop: Option<Label>,
    epilogue_ok: Option<Label>,
    epilogue_trap: Option<Label>,
    /// Deferred cold edge-copy trampolines `(trampoline_label, target_bb, args)` emitted after blocks.
    edge_tramps: Vec<(Label, u32, ValRange)>,
    /// Interns repeated `(target_bb, args.start, args.len)` edge trampolines (e.g. dense `br_table`).
    edge_tramp_cache: HashMap<(u32, u32, u32), Label>,
}

impl<'a> Lower<'a> {
    #[inline]
    fn loc(&self, v: Val) -> Loc {
        self.alloc.locs[v.0 as usize]
    }

    #[inline]
    fn ty(&self, v: Val) -> Ty {
        self.ir.val_tys[v.0 as usize]
    }

    fn gp_read(&mut self, v: Val, scratch: Gp) -> Gp {
        match self.loc(v) {
            Loc::Gp(g) => g,
            Loc::Spill(off) => {
                let is64 = self.ty(v) == Ty::I64;
                self.asm.mov_rm(is64, scratch, Mem::base_disp(Gp::Rsp, off));
                scratch
            }
            Loc::Remat(RematConst::I32(c)) => {
                self.asm.mov_ri32(scratch, c as u32);
                scratch
            }
            Loc::Remat(RematConst::I64(c)) => {
                self.asm.mov_ri64(scratch, c as u64);
                scratch
            }
            Loc::Xmm(_) | Loc::Remat(_) => unreachable!("expected gp val"),
        }
    }

    /// Reads GP value `v` into a register without modifying `RFLAGS` (safe between `cmp` and `cmovcc`).
    fn gp_read_preserve_flags(&mut self, v: Val, scratch: Gp) -> Gp {
        match self.loc(v) {
            Loc::Gp(g) => g,
            Loc::Spill(off) => {
                let is64 = self.ty(v) == Ty::I64;
                self.asm.mov_rm(is64, scratch, Mem::base_disp(Gp::Rsp, off));
                scratch
            }
            Loc::Remat(RematConst::I32(c)) => {
                self.asm.mov_ri32_preserve_flags(scratch, c as u32);
                scratch
            }
            Loc::Remat(RematConst::I64(c)) => {
                self.asm.mov_ri64_preserve_flags(scratch, c as u64);
                scratch
            }
            Loc::Xmm(_) | Loc::Remat(_) => unreachable!("expected gp val"),
        }
    }

    fn fp_read(&mut self, v: Val, scratch: Xmm) -> Xmm {
        let ty = self.ty(v);
        match self.loc(v) {
            Loc::Xmm(x) => x,
            Loc::Spill(off) => {
                emit_fp_load(&mut self.asm, scratch, Mem::base_disp(Gp::Rsp, off), ty);
                scratch
            }
            Loc::Remat(RematConst::F32(bits)) => {
                if bits == 0 {
                    self.asm.xorps(scratch, scratch);
                } else {
                    let loff = self.asm.intern_f32(bits);
                    self.asm.movss_rip(scratch, RelocTarget::LocalRodata(loff));
                }
                scratch
            }
            Loc::Remat(RematConst::F64(bits)) => {
                if bits == 0 {
                    self.asm.xorps(scratch, scratch);
                } else {
                    let loff = self.asm.intern_f64(bits);
                    self.asm.movsd_rip(scratch, RelocTarget::LocalRodata(loff));
                }
                scratch
            }
            Loc::Gp(_) | Loc::Remat(_) => unreachable!("expected fp val"),
        }
    }

    /// Emits `ucomiss`/`ucomisd lhs_reg, rhs_val`, folding spilled stack slots and RIP-relative
    /// non-zero FP constants directly into the memory operand.
    fn emit_ucomi_rhs(&mut self, is64: bool, lhs_reg: Xmm, rhs_val: Val) {
        match self.loc(rhs_val) {
            Loc::Xmm(rx) => {
                if is64 {
                    self.asm.ucomisd(lhs_reg, rx);
                } else {
                    self.asm.ucomiss(lhs_reg, rx);
                }
            }
            Loc::Spill(off) => {
                let m = Mem::base_disp(Gp::Rsp, off);
                if is64 {
                    self.asm.ucomisd_xm(lhs_reg, m);
                } else {
                    self.asm.ucomiss_xm(lhs_reg, m);
                }
            }
            Loc::Remat(RematConst::F32(bits)) if !is64 && bits != 0 => {
                let loff = self.asm.intern_f32(bits);
                self.asm
                    .xmm_rip_op(None, 0x2e, lhs_reg, RelocTarget::LocalRodata(loff));
            }
            Loc::Remat(RematConst::F64(bits)) if is64 && bits != 0 => {
                let loff = self.asm.intern_f64(bits);
                self.asm
                    .xmm_rip_op(Some(0x66), 0x2e, lhs_reg, RelocTarget::LocalRodata(loff));
            }
            _ => {
                let rx = self.fp_read(rhs_val, SCRATCH_FP2);
                if is64 {
                    self.asm.ucomisd(lhs_reg, rx);
                } else {
                    self.asm.ucomiss(lhs_reg, rx);
                }
            }
        }
    }

    fn gp_write_reg(&self, v: Val) -> Gp {
        match self.loc(v) {
            Loc::Gp(g) => g,
            Loc::Spill(_) => SCRATCH_GP1,
            Loc::Xmm(_) | Loc::Remat(_) => unreachable!(),
        }
    }

    fn gp_commit(&mut self, v: Val, reg: Gp) {
        match self.loc(v) {
            Loc::Gp(d) => {
                let is64 = self.ty(v) == Ty::I64;
                if d != reg {
                    self.asm.mov_rr(is64, d, reg);
                }
            }
            Loc::Spill(off) => {
                let is64 = self.ty(v) == Ty::I64;
                self.asm.mov_mr(is64, Mem::base_disp(Gp::Rsp, off), reg);
            }
            Loc::Xmm(_) | Loc::Remat(_) => unreachable!(),
        }
    }

    fn fp_write_reg(&self, v: Val) -> Xmm {
        match self.loc(v) {
            Loc::Xmm(x) => x,
            Loc::Spill(_) => SCRATCH_FP1,
            Loc::Gp(_) | Loc::Remat(_) => unreachable!(),
        }
    }

    fn fp_commit(&mut self, v: Val, reg: Xmm) {
        let ty = self.ty(v);
        match self.loc(v) {
            Loc::Xmm(d) => self.asm.movaps_rr(d, reg),
            Loc::Spill(off) => emit_fp_store(&mut self.asm, Mem::base_disp(Gp::Rsp, off), reg, ty),
            Loc::Gp(_) | Loc::Remat(_) => unreachable!(),
        }
    }

    fn trap_oob_lbl(&mut self) -> Label {
        *self.trap_oob.get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_table_oob_lbl(&mut self) -> Label {
        *self
            .trap_table_oob
            .get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_ci_lbl(&mut self) -> Label {
        *self.trap_ci.get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_div0_lbl(&mut self) -> Label {
        *self.trap_div0.get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_int_ovf_lbl(&mut self) -> Label {
        *self
            .trap_int_ovf
            .get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_unreach_lbl(&mut self) -> Label {
        *self
            .trap_unreach
            .get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_prop_lbl(&mut self) -> Label {
        *self.trap_prop.get_or_insert_with(|| self.asm.new_label())
    }
    fn epilogue_trap_lbl(&mut self) -> Label {
        *self
            .epilogue_trap
            .get_or_insert_with(|| self.asm.new_label())
    }

    fn emit_epilogue_body(&mut self) {
        for (i, &x) in self.alloc.used_callee_xmm.iter().enumerate() {
            self.asm.movdqu_xm(
                x,
                Mem::base_disp(Gp::Rsp, self.alloc.saved_xmm_base + (i as i32) * 16),
            );
        }
        if self.alloc.frame_size > 0 {
            self.asm.add_ri(true, Gp::Rsp, self.alloc.frame_size);
        }
        for &g in self.alloc.used_callee_gp.iter().rev() {
            self.asm.pop(g);
        }
        self.asm.ret();
    }

    fn emit_bounds_check(&mut self, addr: Gp, end_off: u64) {
        let oob = self.trap_oob_lbl();
        if let Some(cap) = self.layout.fixed_mem_bytes {
            // Fixed-size memory (`min == max`): `addr` is zero-extended u32, `R13` is free!
            if end_off > cap {
                self.asm.jmp(oob);
                return;
            }
            let max_addr = cap - end_off;
            if let Ok(imm32) = i32::try_from(max_addr) {
                self.asm.cmp_ri(true, addr, imm32);
            } else {
                self.asm.mov_ri64(SCRATCH_GP1, max_addr);
                self.asm.cmp_rr(true, addr, SCRATCH_GP1);
            }
            self.asm.jcc(Cond::A, oob);
            return;
        }
        if end_off == 0 {
            self.asm.cmp_rr(true, addr, Gp::R13);
            self.asm.jcc(Cond::A, oob);
            return;
        }
        if let Ok(disp) = i32::try_from(end_off) {
            self.asm.lea(true, SCRATCH_GP1, Mem::base_disp(addr, disp));
        } else {
            self.asm.mov_ri64(SCRATCH_GP1, end_off);
            self.asm.add_rr(true, SCRATCH_GP1, addr);
        }
        self.asm.cmp_rr(true, SCRATCH_GP1, Gp::R13);
        self.asm.jcc(Cond::A, oob);
    }

    fn mem_addr(&mut self, addr: Gp, offset: u32) -> Mem {
        if let Ok(disp) = i32::try_from(offset) {
            Mem::indexed(Gp::R14, addr, 1, disp)
        } else {
            // offset in 0x8000_0000..=0xffff_ffff: compute `r11 = addr + offset` (64-bit).
            self.asm.mov_ri32(SCRATCH_GP1, offset);
            self.asm.add_rr(true, SCRATCH_GP1, addr);
            Mem::indexed(Gp::R14, SCRATCH_GP1, 1, 0)
        }
    }

    fn emit_edge_moves(&mut self, target_bb: u32, args: ValRange) {
        let params = &self.ir.blocks[target_bb as usize].params;
        let arg_vals = self.ir.pool_slice(args);
        debug_assert_eq!(params.len(), arg_vals.len());
        if params.is_empty() {
            return;
        }
        let mut moves = Vec::with_capacity(params.len());
        for (&p, &a) in params.iter().zip(arg_vals) {
            moves.push((
                MoveLoc::from(self.loc(p)),
                MoveLoc::from(self.loc(a)),
                self.ty(p),
            ));
        }
        emit_parallel_moves(&mut self.asm, &moves);
    }

    fn edge_has_moves(&self, target_bb: u32, args: ValRange) -> bool {
        if args.len == 0 {
            return false;
        }
        let params = &self.ir.blocks[target_bb as usize].params;
        let arg_vals = self.ir.pool_slice(args);
        params
            .iter()
            .zip(arg_vals)
            .any(|(&p, &a)| self.loc(p) != self.loc(a))
    }

    fn edge_target_label(&mut self, target_bb: u32, args: ValRange) -> Label {
        if !self.edge_has_moves(target_bb, args) {
            self.bb_labels[target_bb as usize]
        } else {
            let key = (target_bb, args.start, args.len);
            if let Some(&lbl) = self.edge_tramp_cache.get(&key) {
                return lbl;
            }
            let lbl = self.asm.new_label();
            self.edge_tramps.push((lbl, target_bb, args));
            self.edge_tramp_cache.insert(key, lbl);
            lbl
        }
    }

    fn emit_call_abi(
        &mut self,
        args: ValRange,
        rets: ValRange,
        ci_idx: Option<(u32, Val)>,
        direct_func: Option<u32>,
    ) -> Result<(), String> {
        // For indirect call, stash `idx` in `[r15 + OFF_SCRATCH_TMP]` so `emit_parallel_moves` can freely use R10 & R11.
        if let Some((_, idx_val)) = ci_idx {
            let ir = self.gp_read(idx_val, SCRATCH_GP2);
            self.asm
                .mov_mr(false, Mem::base_disp(Gp::R15, OFF_SCRATCH_TMP), ir);
        }
        // Marshal arguments using the unified typed ABI layout shared with `regalloc`.
        let arg_vals = self.ir.pool_slice(args);
        if !arg_vals.is_empty() {
            let arg_tys: Vec<Ty> = arg_vals.iter().map(|&v| self.ty(v)).collect();
            let arg_layout = layout_args(&arg_tys)?;
            let mut moves = Vec::with_capacity(arg_vals.len());
            for ((&av, &ty), slot) in arg_vals.iter().zip(&arg_tys).zip(arg_layout.slots) {
                let dst = match slot {
                    AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    AbiSlot::Xmm(x) => MoveLoc::Xmm(x),
                    AbiSlot::Stack(off) => MoveLoc::Stack(off),
                };
                moves.push((dst, MoveLoc::from(self.loc(av)), ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }

        // If the callee has overflow return values, publish our caller-owned stack return buffer pointer.
        let ret_vals = self.ir.pool_slice(rets);
        let ret_tys: Vec<Ty> = ret_vals.iter().map(|&v| self.ty(v)).collect();
        let ret_layout = layout_rets(&ret_tys)?;
        if ret_layout.stack_bytes > 0 {
            self.asm.lea(
                true,
                SCRATCH_GP1,
                Mem::base_disp(Gp::Rsp, self.alloc.ret_buf_off),
            );
            self.asm.mov_mr(
                true,
                Mem::base_disp(Gp::R15, OFF_RET_SPILL_PTR),
                SCRATCH_GP1,
            );
        }

        if let Some(f) = direct_func {
            self.asm.call_reloc(RelocTarget::Func(f));
        } else if let Some((ci_id, _)) = ci_idx {
            let ci = &self.layout.ci_tables[ci_id as usize];
            let toob = self.trap_table_oob_lbl();
            let bad = self.trap_ci_lbl();
            self.asm
                .mov_rm(false, SCRATCH_GP2, Mem::base_disp(Gp::R15, OFF_SCRATCH_TMP));
            // Unsigned 32-bit comparisons avoid sign-extending lengths >= 2^31 in 64-bit `cmp_ri`.
            self.asm.cmp_ri(false, SCRATCH_GP2, ci.table_len as i32);
            self.asm.jcc(Cond::Ae, toob);
            if ci.span == 0 {
                self.asm.jmp(bad);
                return Ok(());
            } else {
                if ci.min_slot > 0 {
                    self.asm.sub_ri(false, SCRATCH_GP2, ci.min_slot as i32);
                }
                self.asm.cmp_ri(false, SCRATCH_GP2, ci.span as i32);
                self.asm.jcc(Cond::Ae, bad);
                self.asm
                    .lea_rip(SCRATCH_GP1, RelocTarget::Rodata(ci.rodata_off));
                self.asm
                    .movsxd_rm(SCRATCH_GP2, Mem::indexed(SCRATCH_GP1, SCRATCH_GP2, 4, 0));
                self.asm.test_rr(true, SCRATCH_GP2, SCRATCH_GP2);
                self.asm.jcc(Cond::E, bad);
                self.asm.add_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                self.asm.call_reg(SCRATCH_GP1);
            }
        }

        // Check trap in `R11d` unless Step 8b proved a direct callee cannot trap (including no stack check!).
        let need_trap_check = match direct_func {
            Some(f) => self
                .fn_can_trap
                .is_none_or(|t| t.get(f as usize).copied().unwrap_or(true)),
            None => true,
        };
        if need_trap_check {
            let prop = self.trap_prop_lbl();
            self.asm.test_rr(false, SCRATCH_GP1, SCRATCH_GP1);
            self.asm.jcc(Cond::Ne, prop);
        }

        // Unpack return values from registers and caller-owned stack return buffer.
        if !ret_vals.is_empty() {
            let mut moves = Vec::with_capacity(ret_vals.len());
            for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(ret_layout.slots) {
                let src = match slot {
                    AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    AbiSlot::Xmm(x) => MoveLoc::Xmm(x),
                    AbiSlot::Stack(off) => MoveLoc::Stack(self.alloc.ret_buf_off + off),
                };
                moves.push((MoveLoc::from(self.loc(rv)), src, ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }
        Ok(())
    }

    fn emit_int_bin(&mut self, is64: bool, op: IntBinOp, dst_val: Val, a: Val, b: Val) {
        match op {
            IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU => {
                // `regalloc` guarantees RAX & RDX are not live across this op.
                // Load `b` into R10 first, then `a` into RAX (handles `a == RDX` or `b == RAX` cleanly).
                let br = self.gp_read(b, SCRATCH_GP2);
                if br != SCRATCH_GP2 {
                    self.asm.mov_rr(is64, SCRATCH_GP2, br);
                }
                let ar = self.gp_read(a, Gp::Rax);
                if ar != Gp::Rax {
                    self.asm.mov_rr(is64, Gp::Rax, ar);
                }
                let div0 = self.trap_div0_lbl();
                self.asm.test_rr(is64, SCRATCH_GP2, SCRATCH_GP2);
                self.asm.jcc(Cond::E, div0);
                match op {
                    IntBinOp::DivS | IntBinOp::RemS => {
                        let normal = self.asm.new_label();
                        let done = self.asm.new_label();
                        self.asm.cmp_ri(is64, SCRATCH_GP2, -1);
                        self.asm.jcc(Cond::Ne, normal);
                        if is64 {
                            self.asm.mov_ri64(SCRATCH_GP1, i64::MIN as u64);
                            self.asm.cmp_rr(true, Gp::Rax, SCRATCH_GP1);
                        } else {
                            self.asm.cmp_ri(false, Gp::Rax, i32::MIN);
                        }
                        if op == IntBinOp::DivS {
                            let ovf = self.trap_int_ovf_lbl();
                            self.asm.jcc(Cond::E, ovf);
                        } else {
                            self.asm.jcc(Cond::Ne, normal);
                            self.asm.xor_rr(false, Gp::Rdx, Gp::Rdx);
                            self.asm.jmp(done);
                        }
                        self.asm.bind(normal);
                        if is64 {
                            self.asm.cqo();
                        } else {
                            self.asm.cdq();
                        }
                        self.asm.idiv_r(is64, SCRATCH_GP2);
                        self.asm.bind(done);
                        let res_reg = if op == IntBinOp::DivS {
                            Gp::Rax
                        } else {
                            Gp::Rdx
                        };
                        self.gp_commit(dst_val, res_reg);
                    }
                    IntBinOp::DivU | IntBinOp::RemU => {
                        self.asm.xor_rr(false, Gp::Rdx, Gp::Rdx);
                        self.asm.div_r(is64, SCRATCH_GP2);
                        let res_reg = if op == IntBinOp::DivU {
                            Gp::Rax
                        } else {
                            Gp::Rdx
                        };
                        self.gp_commit(dst_val, res_reg);
                    }
                    _ => unreachable!(),
                }
            }
            IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU | IntBinOp::Rotl | IntBinOp::Rotr => {
                if self.isa.has_bmi2
                    && matches!(op, IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU)
                {
                    let ar = self.gp_read(a, SCRATCH_GP1);
                    let br = self.gp_read(b, SCRATCH_GP2);
                    let d = self.gp_write_reg(dst_val);
                    match op {
                        IntBinOp::Shl => self.asm.shlx(is64, d, ar, br),
                        IntBinOp::ShrS => self.asm.sarx(is64, d, ar, br),
                        IntBinOp::ShrU => self.asm.shrx(is64, d, ar, br),
                        _ => unreachable!(),
                    }
                    self.gp_commit(dst_val, d);
                } else {
                    // `regalloc` guarantees RCX is not live across CL shifts, though `a` or `b`
                    // dying at this instruction may reside in RCX. Stage `b -> R10`, then `a -> d`
                    // (choosing `d != RCX`), then `R10 -> RCX` so neither operand can clobber the other.
                    let br = self.gp_read(b, SCRATCH_GP2);
                    if br != SCRATCH_GP2 {
                        self.asm.mov_rr(false, SCRATCH_GP2, br);
                    }
                    let mut d = self.gp_write_reg(dst_val);
                    if d == Gp::Rcx {
                        d = SCRATCH_GP1;
                    }
                    let ar = self.gp_read(a, d);
                    if d != ar {
                        self.asm.mov_rr(is64, d, ar);
                    }
                    self.asm.mov_rr(false, Gp::Rcx, SCRATCH_GP2);
                    match op {
                        IntBinOp::Shl => self.asm.shl_cl(is64, d),
                        IntBinOp::ShrS => self.asm.sar_cl(is64, d),
                        IntBinOp::ShrU => self.asm.shr_cl(is64, d),
                        IntBinOp::Rotl => self.asm.rol_cl(is64, d),
                        IntBinOp::Rotr => self.asm.ror_cl(is64, d),
                        _ => unreachable!(),
                    }
                    self.gp_commit(dst_val, d);
                }
            }
            _ => {
                let d = self.gp_write_reg(dst_val);
                // Memory-operand ALU fast path when rhs (or commutative lhs) is a spill slot and `d` is a real reg.
                let is_comm = matches!(
                    op,
                    IntBinOp::Add | IntBinOp::Mul | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor
                );
                let mem_alu = |asm: &mut Asm, op: IntBinOp, is64: bool, dst: Gp, mem: Mem| match op
                {
                    IntBinOp::Add => asm.add_rm(is64, dst, mem),
                    IntBinOp::Sub => asm.sub_rm(is64, dst, mem),
                    IntBinOp::Mul => asm.imul_rm(is64, dst, mem),
                    IntBinOp::And => asm.and_rm(is64, dst, mem),
                    IntBinOp::Or => asm.or_rm(is64, dst, mem),
                    IntBinOp::Xor => asm.xor_rm(is64, dst, mem),
                    _ => unreachable!(),
                };
                match (self.loc(a), self.loc(b)) {
                    (Loc::Gp(ag), Loc::Spill(boff)) => {
                        if d != ag {
                            self.asm.mov_rr(is64, d, ag);
                        }
                        mem_alu(&mut self.asm, op, is64, d, Mem::base_disp(Gp::Rsp, boff));
                        self.gp_commit(dst_val, d);
                        return;
                    }
                    (Loc::Spill(aoff), Loc::Gp(bg)) if is_comm => {
                        if d != bg {
                            self.asm.mov_rr(is64, d, bg);
                        }
                        mem_alu(&mut self.asm, op, is64, d, Mem::base_disp(Gp::Rsp, aoff));
                        self.gp_commit(dst_val, d);
                        return;
                    }
                    _ => {}
                }
                let ar = self.gp_read(a, SCRATCH_GP1);
                let br = self.gp_read(b, SCRATCH_GP2);
                // 3-operand `lea d, [ar + br]` avoids a `mov` whenever `Add` is non-two-address!
                if op == IntBinOp::Add && d != ar && d != br && br != Gp::Rsp {
                    self.asm.lea(is64, d, Mem::indexed(ar, br, 1, 0));
                    self.gp_commit(dst_val, d);
                    return;
                }
                // Careful if `d == br` and `d != ar`!
                let (dest, rhs) = if d == br && d != ar {
                    if is_comm {
                        // Commutative: keep `br` in `d`, op with `ar`!
                        (d, ar)
                    } else {
                        // Non-commutative (`Sub`): move `br` to `SCRATCH_GP2`, then `ar -> d`.
                        self.asm.mov_rr(is64, SCRATCH_GP2, br);
                        self.asm.mov_rr(is64, d, ar);
                        (d, SCRATCH_GP2)
                    }
                } else {
                    if d != ar {
                        self.asm.mov_rr(is64, d, ar);
                    }
                    (d, br)
                };
                match op {
                    IntBinOp::Add => self.asm.add_rr(is64, dest, rhs),
                    IntBinOp::Sub => self.asm.sub_rr(is64, dest, rhs),
                    IntBinOp::Mul => self.asm.imul_rr(is64, dest, rhs),
                    IntBinOp::And => self.asm.and_rr(is64, dest, rhs),
                    IntBinOp::Or => self.asm.or_rr(is64, dest, rhs),
                    IntBinOp::Xor => self.asm.xor_rr(is64, dest, rhs),
                    _ => unreachable!(),
                }
                self.gp_commit(dst_val, dest);
            }
        }
    }

    fn emit_magic_div_rem_i32(&mut self, op: IntBinOp, dst_val: Val, a: Val, imm: i32) {
        // Load `a` into `SCRATCH_GP1` (leaving original `a` untouched if in a real register!).
        let ar = self.gp_read(a, SCRATCH_GP1);
        if ar != SCRATCH_GP1 {
            self.asm.mov_rr(false, SCRATCH_GP1, ar);
        }
        let is_rem = matches!(op, IntBinOp::RemS | IntBinOp::RemU);
        match op {
            IntBinOp::DivU | IntBinOp::RemU => {
                let du = imm as u32;
                if du.is_power_of_two() {
                    if is_rem {
                        self.asm.and_ri(false, SCRATCH_GP1, (du - 1) as i32);
                        let d = self.gp_write_reg(dst_val);
                        if d != SCRATCH_GP1 {
                            self.asm.mov_rr(false, d, SCRATCH_GP1);
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    } else {
                        self.asm
                            .shr_imm(false, SCRATCH_GP1, du.trailing_zeros() as u8);
                    }
                } else {
                    let (magic, shift, add) = magic_u(32, u64::from(du));
                    self.asm.mov_ri32(SCRATCH_GP2, magic as u32);
                    if !add {
                        self.asm.imul_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                        self.asm.shr_imm(true, SCRATCH_GP1, (32 + shift) as u8);
                    } else {
                        self.asm.imul_rr(true, SCRATCH_GP2, SCRATCH_GP1);
                        self.asm.shr_imm(true, SCRATCH_GP2, 32);
                        self.asm.sub_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                        self.asm.shr_imm(false, SCRATCH_GP1, 1);
                        self.asm.add_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                        if shift > 1 {
                            self.asm.shr_imm(false, SCRATCH_GP1, (shift - 1) as u8);
                        }
                    }
                }
            }
            IntBinOp::DivS | IntBinOp::RemS => {
                if imm == i32::MIN {
                    if is_rem {
                        self.asm.mov_rr(false, SCRATCH_GP2, SCRATCH_GP1);
                        self.asm.xor_rr(false, SCRATCH_GP1, SCRATCH_GP1);
                        self.asm.cmp_ri(false, SCRATCH_GP2, i32::MIN);
                        self.asm.cmovcc(Cond::Ne, false, SCRATCH_GP1, SCRATCH_GP2);
                        let d = self.gp_write_reg(dst_val);
                        if d != SCRATCH_GP1 {
                            self.asm.mov_rr(false, d, SCRATCH_GP1);
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    } else {
                        self.asm.cmp_ri(false, SCRATCH_GP1, i32::MIN);
                        self.asm.setcc(Cond::E, SCRATCH_GP1);
                        self.asm.movzx_rr8(SCRATCH_GP1, SCRATCH_GP1);
                    }
                } else if imm.unsigned_abs().is_power_of_two() {
                    let k = imm.unsigned_abs().trailing_zeros() as u8;
                    self.asm.mov_rr(false, SCRATCH_GP2, SCRATCH_GP1);
                    if k > 1 {
                        self.asm.sar_imm(false, SCRATCH_GP2, k - 1);
                    }
                    self.asm.shr_imm(false, SCRATCH_GP2, 32 - k);
                    self.asm.add_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                    self.asm.sar_imm(false, SCRATCH_GP1, k);
                    if imm < 0 {
                        self.asm.neg(false, SCRATCH_GP1);
                    }
                } else {
                    let (magic, shift, add_sub) = magic_s(32, i64::from(imm));
                    self.asm.movsxd_rr(SCRATCH_GP1, SCRATCH_GP1);
                    self.asm.mov_ri64(SCRATCH_GP2, magic as u64);
                    self.asm.imul_rr(true, SCRATCH_GP2, SCRATCH_GP1);
                    self.asm.sar_imm(true, SCRATCH_GP2, 32);
                    if add_sub > 0 {
                        self.asm.add_rr(false, SCRATCH_GP2, SCRATCH_GP1);
                    } else if add_sub < 0 {
                        self.asm.sub_rr(false, SCRATCH_GP2, SCRATCH_GP1);
                    }
                    if shift > 0 {
                        self.asm.sar_imm(false, SCRATCH_GP2, shift as u8);
                    }
                    self.asm.mov_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                    self.asm.shr_imm(false, SCRATCH_GP1, 31);
                    self.asm.add_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                }
            }
            _ => unreachable!(),
        }
        if !is_rem {
            let d = self.gp_write_reg(dst_val);
            if d != SCRATCH_GP1 {
                self.asm.mov_rr(false, d, SCRATCH_GP1);
            }
            self.gp_commit(dst_val, d);
        } else {
            // `SCRATCH_GP1` holds `q`: compute `rem = a - q * imm`.
            self.asm.imul_rri(false, SCRATCH_GP1, SCRATCH_GP1, imm);
            let raw_d = self.gp_write_reg(dst_val);
            let out_reg = if raw_d == SCRATCH_GP1 {
                SCRATCH_GP2
            } else {
                raw_d
            };
            let ar = self.gp_read(a, out_reg);
            if out_reg != ar {
                self.asm.mov_rr(false, out_reg, ar);
            }
            self.asm.sub_rr(false, out_reg, SCRATCH_GP1);
            self.gp_commit(dst_val, out_reg);
        }
    }

    fn emit_magic_div_rem_i64(&mut self, op: IntBinOp, dst_val: Val, a: Val, imm: i32) {
        // `regalloc` guarantees RAX & RDX are not live across 64-bit constant div/rem.
        let is_rem = matches!(op, IntBinOp::RemS | IntBinOp::RemU);
        let d64 = if matches!(op, IntBinOp::DivU | IntBinOp::RemU) {
            u64::from(imm as u32)
        } else {
            i64::from(imm) as u64
        };
        // Load original `a` into `SCRATCH_GP1` (and into `RAX` when needed).
        let ar = self.gp_read(a, SCRATCH_GP1);
        if ar != SCRATCH_GP1 {
            self.asm.mov_rr(true, SCRATCH_GP1, ar);
        }
        // `mul`/`imul1` overwrite BOTH `Rax` and `Rdx`; if `a` dies here in `Rax`/`Rdx`, preserve
        // original `a` in unallocated `SCRATCH_FP1` (`Xmm15`) so `rem = a - q * imm` reads original `a`.
        let saved_a_in_fp = is_rem && matches!(self.loc(a), Loc::Gp(Gp::Rax | Gp::Rdx));
        if saved_a_in_fp {
            self.asm.movq_xr(SCRATCH_FP1, SCRATCH_GP1);
        }
        match op {
            IntBinOp::DivU | IntBinOp::RemU => {
                if d64.is_power_of_two() {
                    if is_rem {
                        self.asm.and_ri(true, SCRATCH_GP1, (d64 - 1) as i32);
                        let d = self.gp_write_reg(dst_val);
                        if d != SCRATCH_GP1 {
                            self.asm.mov_rr(true, d, SCRATCH_GP1);
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    } else {
                        self.asm
                            .shr_imm(true, SCRATCH_GP1, d64.trailing_zeros() as u8);
                    }
                } else {
                    let (magic, shift, add) = magic_u(64, d64);
                    self.asm.mov_rr(true, Gp::Rax, SCRATCH_GP1);
                    self.asm.mov_ri64(SCRATCH_GP2, magic);
                    self.asm.mul_r(true, SCRATCH_GP2); // RDX = mulhi_u(n, magic)
                    if !add {
                        self.asm.mov_rr(true, SCRATCH_GP1, Gp::Rdx);
                        if shift > 0 {
                            self.asm.shr_imm(true, SCRATCH_GP1, shift as u8);
                        }
                    } else {
                        // t = RDX, n = SCRATCH_GP1 -> (((n - t) >> 1) + t) >> (shift - 1)
                        self.asm.sub_rr(true, SCRATCH_GP1, Gp::Rdx);
                        self.asm.shr_imm(true, SCRATCH_GP1, 1);
                        self.asm.add_rr(true, SCRATCH_GP1, Gp::Rdx);
                        if shift > 1 {
                            self.asm.shr_imm(true, SCRATCH_GP1, (shift - 1) as u8);
                        }
                    }
                }
            }
            IntBinOp::DivS | IntBinOp::RemS => {
                let ds = i64::from(imm);
                if ds.unsigned_abs().is_power_of_two() {
                    let k = ds.unsigned_abs().trailing_zeros() as u8;
                    self.asm.mov_rr(true, SCRATCH_GP2, SCRATCH_GP1);
                    if k > 1 {
                        self.asm.sar_imm(true, SCRATCH_GP2, k - 1);
                    }
                    self.asm.shr_imm(true, SCRATCH_GP2, 64 - k);
                    self.asm.add_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                    self.asm.sar_imm(true, SCRATCH_GP1, k);
                    if ds < 0 {
                        self.asm.neg(true, SCRATCH_GP1);
                    }
                } else {
                    let (magic, shift, add_sub) = magic_s(64, ds);
                    self.asm.mov_rr(true, Gp::Rax, SCRATCH_GP1);
                    self.asm.mov_ri64(SCRATCH_GP2, magic as u64);
                    self.asm.imul1_r(true, SCRATCH_GP2); // RDX = mulhi_s(n, magic)
                    if add_sub > 0 {
                        self.asm.add_rr(true, Gp::Rdx, SCRATCH_GP1);
                    } else if add_sub < 0 {
                        self.asm.sub_rr(true, Gp::Rdx, SCRATCH_GP1);
                    }
                    if shift > 0 {
                        self.asm.sar_imm(true, Gp::Rdx, shift as u8);
                    }
                    self.asm.mov_rr(true, SCRATCH_GP1, Gp::Rdx);
                    self.asm.shr_imm(true, SCRATCH_GP1, 63);
                    self.asm.add_rr(true, SCRATCH_GP1, Gp::Rdx);
                }
            }
            _ => unreachable!(),
        }
        if !is_rem {
            let d = self.gp_write_reg(dst_val);
            if d != SCRATCH_GP1 {
                self.asm.mov_rr(true, d, SCRATCH_GP1);
            }
            self.gp_commit(dst_val, d);
        } else {
            self.asm.imul_rri(true, SCRATCH_GP1, SCRATCH_GP1, imm);
            let raw_d = self.gp_write_reg(dst_val);
            let out_reg = if raw_d == SCRATCH_GP1 {
                SCRATCH_GP2
            } else {
                raw_d
            };
            if saved_a_in_fp {
                self.asm.movq_rx(out_reg, SCRATCH_FP1);
            } else {
                let ar = self.gp_read(a, out_reg);
                if out_reg != ar {
                    self.asm.mov_rr(true, out_reg, ar);
                }
            }
            self.asm.sub_rr(true, out_reg, SCRATCH_GP1);
            self.gp_commit(dst_val, out_reg);
        }
    }

    fn emit_int_bin_imm(&mut self, is64: bool, op: IntBinOp, dst_val: Val, a: Val, imm: i32) {
        if matches!(
            op,
            IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
        ) {
            if is64 {
                self.emit_magic_div_rem_i64(op, dst_val, a, imm);
            } else {
                self.emit_magic_div_rem_i32(op, dst_val, a, imm);
            }
            return;
        }
        let ar = self.gp_read(a, SCRATCH_GP1);
        let d = self.gp_write_reg(dst_val);
        match op {
            IntBinOp::Add => {
                if d != ar {
                    self.asm.lea(is64, d, Mem::base_disp(ar, imm));
                } else {
                    self.asm.add_ri(is64, d, imm);
                }
            }
            IntBinOp::Sub => {
                if d != ar {
                    if let Some(neg) = imm.checked_neg() {
                        self.asm.lea(is64, d, Mem::base_disp(ar, neg));
                    } else {
                        self.asm.mov_rr(is64, d, ar);
                        self.asm.sub_ri(is64, d, imm);
                    }
                } else {
                    self.asm.sub_ri(is64, d, imm);
                }
            }
            IntBinOp::Mul => {
                if matches!(imm, 3 | 5 | 9) && ar != Gp::Rsp {
                    self.asm
                        .lea(is64, d, Mem::indexed(ar, ar, (imm - 1) as u8, 0));
                } else {
                    self.asm.imul_rri(is64, d, ar, imm);
                }
            }
            IntBinOp::And => {
                if d != ar {
                    self.asm.mov_rr(is64, d, ar);
                }
                self.asm.and_ri(is64, d, imm);
            }
            IntBinOp::Or => {
                if d != ar {
                    self.asm.mov_rr(is64, d, ar);
                }
                self.asm.or_ri(is64, d, imm);
            }
            IntBinOp::Xor => {
                if d != ar {
                    self.asm.mov_rr(is64, d, ar);
                }
                self.asm.xor_ri(is64, d, imm);
            }
            IntBinOp::Shl => {
                if imm == 1 && d != ar && ar != Gp::Rsp {
                    self.asm.lea(is64, d, Mem::indexed(ar, ar, 1, 0));
                } else {
                    if d != ar {
                        self.asm.mov_rr(is64, d, ar);
                    }
                    self.asm.shl_imm(is64, d, imm as u8);
                }
            }
            IntBinOp::ShrS => {
                if d != ar {
                    self.asm.mov_rr(is64, d, ar);
                }
                self.asm.sar_imm(is64, d, imm as u8);
            }
            IntBinOp::ShrU => {
                if d != ar {
                    self.asm.mov_rr(is64, d, ar);
                }
                self.asm.shr_imm(is64, d, imm as u8);
            }
            IntBinOp::Rotl => {
                if self.isa.has_bmi2 && d != ar {
                    let w: u8 = if is64 { 64 } else { 32 };
                    let r = w.wrapping_sub((imm as u8) & (w - 1)) & (w - 1);
                    self.asm.rorx(is64, d, ar, r);
                } else {
                    if d != ar {
                        self.asm.mov_rr(is64, d, ar);
                    }
                    self.asm.rol_imm(is64, d, imm as u8);
                }
            }
            IntBinOp::Rotr => {
                if self.isa.has_bmi2 && d != ar {
                    self.asm.rorx(is64, d, ar, imm as u8);
                } else {
                    if d != ar {
                        self.asm.mov_rr(is64, d, ar);
                    }
                    self.asm.ror_imm(is64, d, imm as u8);
                }
            }
            _ => unreachable!("unexpected imm binop {op:?}"),
        }
        self.gp_commit(dst_val, d);
    }

    fn emit_float_cmp(&mut self, is64: bool, op: CmpOp, dst_val: Val, a: Val, b: Val) {
        let d = self.gp_write_reg(dst_val);
        match op {
            CmpOp::Eq => {
                let ar = self.fp_read(a, SCRATCH_FP1);
                if a == b {
                    if is64 {
                        self.asm.ucomisd(ar, ar);
                    } else {
                        self.asm.ucomiss(ar, ar);
                    }
                    self.asm.setcc(Cond::Np, d);
                    self.asm.movzx_rr8(d, d);
                } else {
                    self.emit_ucomi_rhs(is64, ar, b);
                    self.asm.setcc(Cond::E, SCRATCH_GP1);
                    self.asm.setcc(Cond::Np, SCRATCH_GP2);
                    self.asm.and_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                    self.asm.movzx_rr8(d, SCRATCH_GP1);
                }
            }
            CmpOp::Ne => {
                let ar = self.fp_read(a, SCRATCH_FP1);
                if a == b {
                    if is64 {
                        self.asm.ucomisd(ar, ar);
                    } else {
                        self.asm.ucomiss(ar, ar);
                    }
                    self.asm.setcc(Cond::P, d);
                    self.asm.movzx_rr8(d, d);
                } else {
                    self.emit_ucomi_rhs(is64, ar, b);
                    self.asm.setcc(Cond::Ne, SCRATCH_GP1);
                    self.asm.setcc(Cond::P, SCRATCH_GP2);
                    self.asm.or_rr(false, SCRATCH_GP1, SCRATCH_GP2);
                    self.asm.movzx_rr8(d, SCRATCH_GP1);
                }
            }
            CmpOp::GtS => {
                let ar = self.fp_read(a, SCRATCH_FP1);
                self.emit_ucomi_rhs(is64, ar, b);
                self.asm.setcc(Cond::A, d);
                self.asm.movzx_rr8(d, d);
            }
            CmpOp::GeS => {
                let ar = self.fp_read(a, SCRATCH_FP1);
                self.emit_ucomi_rhs(is64, ar, b);
                self.asm.setcc(Cond::Ae, d);
                self.asm.movzx_rr8(d, d);
            }
            CmpOp::LtS => {
                let br = self.fp_read(b, SCRATCH_FP1);
                self.emit_ucomi_rhs(is64, br, a);
                self.asm.setcc(Cond::A, d);
                self.asm.movzx_rr8(d, d);
            }
            CmpOp::LeS => {
                let br = self.fp_read(b, SCRATCH_FP1);
                self.emit_ucomi_rhs(is64, br, a);
                self.asm.setcc(Cond::Ae, d);
                self.asm.movzx_rr8(d, d);
            }
            _ => unreachable!("invalid float cmp {op:?}"),
        }
        self.gp_commit(dst_val, d);
    }

    fn emit_float_minmax(&mut self, is64: bool, is_max: bool, dst_val: Val, a: Val, b: Val) {
        let ar = self.fp_read(a, SCRATCH_FP1);
        let br = self.fp_read(b, SCRATCH_FP2);
        let d = self.fp_write_reg(dst_val);
        let rhs = if d == br && d != ar {
            self.asm.movaps_rr(SCRATCH_FP2, br);
            self.asm.movaps_rr(d, ar);
            SCRATCH_FP2
        } else {
            if d != ar {
                self.asm.movaps_rr(d, ar);
            }
            br
        };
        let lbl_nan = self.asm.new_label();
        let lbl_eq = self.asm.new_label();
        let lbl_done = self.asm.new_label();
        if is64 {
            self.asm.ucomisd(d, rhs);
        } else {
            self.asm.ucomiss(d, rhs);
        }
        self.asm.jcc(Cond::P, lbl_nan);
        self.asm.jcc(Cond::E, lbl_eq);
        match (is64, is_max) {
            (false, false) => self.asm.minss(d, rhs),
            (false, true) => self.asm.maxss(d, rhs),
            (true, false) => self.asm.minsd(d, rhs),
            (true, true) => self.asm.maxsd(d, rhs),
        }
        self.asm.jmp(lbl_done);
        self.asm.bind(lbl_eq);
        // For equal values (notably -0.0 vs +0.0): min uses OR (sign bit 1 wins), max uses AND (sign bit 0 wins).
        if is_max {
            self.asm.andps(d, rhs);
        } else {
            self.asm.orps(d, rhs);
        }
        self.asm.jmp(lbl_done);
        self.asm.bind(lbl_nan);
        if is64 {
            self.asm.movsd_rip(d, RelocTarget::Rodata(self.ro.f64_qnan));
        } else {
            self.asm.movss_rip(d, RelocTarget::Rodata(self.ro.f32_qnan));
        }
        self.asm.bind(lbl_done);
        self.fp_commit(dst_val, d);
    }

    fn emit_inst(&mut self, res: Option<Val>, op: Op) -> Result<(), String> {
        match op {
            Op::ConstI32(c) => {
                let r = res.unwrap();
                if matches!(self.loc(r), Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.gp_write_reg(r);
                self.asm.mov_ri32(d, c as u32);
                self.gp_commit(r, d);
            }
            Op::ConstI64(c) => {
                let r = res.unwrap();
                if matches!(self.loc(r), Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.gp_write_reg(r);
                self.asm.mov_ri64(d, c as u64);
                self.gp_commit(r, d);
            }
            Op::ConstF32(bits) => {
                let r = res.unwrap();
                if matches!(self.loc(r), Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.fp_write_reg(r);
                if bits == 0 {
                    self.asm.xorps(d, d);
                } else {
                    let loff = self.asm.intern_f32(bits);
                    self.asm.movss_rip(d, RelocTarget::LocalRodata(loff));
                }
                self.fp_commit(r, d);
            }
            Op::ConstF64(bits) => {
                let r = res.unwrap();
                if matches!(self.loc(r), Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.fp_write_reg(r);
                if bits == 0 {
                    self.asm.xorps(d, d);
                } else {
                    let loff = self.asm.intern_f64(bits);
                    self.asm.movsd_rip(d, RelocTarget::LocalRodata(loff));
                }
                self.fp_commit(r, d);
            }
            Op::ConstV128(idx) => {
                let r = res.unwrap();
                let bytes = self.ir.v128_consts[idx as usize];
                let d = self.fp_write_reg(r);
                if bytes == [0u8; 16] {
                    self.asm.xorps(d, d);
                } else {
                    let loff = self.asm.add_local_rodata(16, &bytes);
                    self.asm.movdqu_rip(d, RelocTarget::LocalRodata(loff));
                }
                self.fp_commit(r, d);
            }
            Op::I32Eqz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.test_rr(false, ar, ar);
                self.asm.setcc(Cond::E, d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::I64Eqz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.test_rr(true, ar, ar);
                self.asm.setcc(Cond::E, d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::I32Clz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if self.isa.has_abm_bmi1 {
                    self.asm.lzcnt(false, d, ar);
                } else {
                    // Branchless portable CLZ32: `bsr` sets ZF=1 iff src==0; `63 ^ 31 == 32`.
                    self.asm.mov_ri32(SCRATCH_GP2, 63);
                    self.asm.bsr(false, d, ar);
                    self.asm.cmovcc(Cond::E, false, d, SCRATCH_GP2);
                    self.asm.xor_ri(false, d, 31);
                }
                self.gp_commit(r, d);
            }
            Op::I32Ctz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if self.isa.has_abm_bmi1 {
                    self.asm.tzcnt(false, d, ar);
                } else {
                    self.asm.mov_ri32(SCRATCH_GP2, 32);
                    self.asm.bsf(false, d, ar);
                    self.asm.cmovcc(Cond::E, false, d, SCRATCH_GP2);
                }
                self.gp_commit(r, d);
            }
            Op::I32Popcnt(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.popcnt(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64Clz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if self.isa.has_abm_bmi1 {
                    self.asm.lzcnt(true, d, ar);
                } else {
                    // Branchless portable CLZ64: `127 ^ 63 == 64`.
                    self.asm.mov_ri32(SCRATCH_GP2, 127);
                    self.asm.bsr(true, d, ar);
                    self.asm.cmovcc(Cond::E, true, d, SCRATCH_GP2);
                    self.asm.xor_ri(true, d, 63);
                }
                self.gp_commit(r, d);
            }
            Op::I64Ctz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if self.isa.has_abm_bmi1 {
                    self.asm.tzcnt(true, d, ar);
                } else {
                    self.asm.mov_ri32(SCRATCH_GP2, 64);
                    self.asm.bsf(true, d, ar);
                    self.asm.cmovcc(Cond::E, true, d, SCRATCH_GP2);
                }
                self.gp_commit(r, d);
            }
            Op::I64Popcnt(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.popcnt(true, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32WrapI64(a) | Op::I64ExtendI32U(a) => {
                // Both zero-extend the low 32 bits into the 64-bit register!
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.mov_rr(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64ExtendI32S(a) | Op::I64Extend32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.movsxd_rr(d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Extend8S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.movsx_rr8(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Extend16S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.movsx_rr16(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64Extend8S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.movsx_rr8(true, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64Extend16S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                self.asm.movsx_rr16(true, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Bin(bop, a, b) => self.emit_int_bin(false, bop, res.unwrap(), a, b),
            Op::I64Bin(bop, a, b) => self.emit_int_bin(true, bop, res.unwrap(), a, b),
            Op::I32BinImm(bop, a, imm) => self.emit_int_bin_imm(false, bop, res.unwrap(), a, imm),
            Op::I64BinImm(bop, a, imm) => self.emit_int_bin_imm(true, bop, res.unwrap(), a, imm),
            Op::I32Cmp(cop, a, b) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if let Loc::Spill(boff) = self.loc(b) {
                    self.asm.cmp_rm(false, ar, Mem::base_disp(Gp::Rsp, boff));
                } else {
                    let br = self.gp_read(b, SCRATCH_GP2);
                    self.asm.cmp_rr(false, ar, br);
                }
                self.asm.setcc(cmp_to_cond(cop), d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::I32CmpImm(cop, a, imm) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if imm == 0 {
                    self.asm.test_rr(false, ar, ar);
                } else {
                    self.asm.cmp_ri(false, ar, imm);
                }
                self.asm.setcc(cmp_to_cond(cop), d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::I64Cmp(cop, a, b) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if let Loc::Spill(boff) = self.loc(b) {
                    self.asm.cmp_rm(true, ar, Mem::base_disp(Gp::Rsp, boff));
                } else {
                    let br = self.gp_read(b, SCRATCH_GP2);
                    self.asm.cmp_rr(true, ar, br);
                }
                self.asm.setcc(cmp_to_cond(cop), d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::I64CmpImm(cop, a, imm) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.gp_write_reg(r);
                if imm == 0 {
                    self.asm.test_rr(true, ar, ar);
                } else {
                    self.asm.cmp_ri(true, ar, imm);
                }
                self.asm.setcc(cmp_to_cond(cop), d);
                self.asm.movzx_rr8(d, d);
                self.gp_commit(r, d);
            }
            Op::F32Cmp(cop, a, b) => self.emit_float_cmp(false, cop, res.unwrap(), a, b),
            Op::F64Cmp(cop, a, b) => self.emit_float_cmp(true, cop, res.unwrap(), a, b),
            Op::F32Un(uop, a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.fp_write_reg(r);
                match uop {
                    FloatUnOp::Abs => {
                        self.asm.movaps_rr(d, ar);
                        self.asm
                            .xmm_rip_op(None, 0x54, d, RelocTarget::Rodata(self.ro.f32_abs));
                    }
                    FloatUnOp::Neg => {
                        self.asm.movaps_rr(d, ar);
                        self.asm
                            .xmm_rip_op(None, 0x57, d, RelocTarget::Rodata(self.ro.f32_neg));
                    }
                    FloatUnOp::Nearest => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundss(d, ar, 0x08);
                    }
                    FloatUnOp::Floor => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundss(d, ar, 0x09);
                    }
                    FloatUnOp::Ceil => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundss(d, ar, 0x0a);
                    }
                    FloatUnOp::Trunc => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundss(d, ar, 0x0b);
                    }
                    FloatUnOp::Sqrt => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.sqrtss(d, ar);
                    }
                }
                self.fp_commit(r, d);
            }
            Op::F64Un(uop, a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.fp_write_reg(r);
                match uop {
                    FloatUnOp::Abs => {
                        self.asm.movaps_rr(d, ar);
                        self.asm.xmm_rip_op(
                            Some(0x66),
                            0x54,
                            d,
                            RelocTarget::Rodata(self.ro.f64_abs),
                        );
                    }
                    FloatUnOp::Neg => {
                        self.asm.movaps_rr(d, ar);
                        self.asm.xmm_rip_op(
                            Some(0x66),
                            0x57,
                            d,
                            RelocTarget::Rodata(self.ro.f64_neg),
                        );
                    }
                    FloatUnOp::Nearest => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundsd(d, ar, 0x08);
                    }
                    FloatUnOp::Floor => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundsd(d, ar, 0x09);
                    }
                    FloatUnOp::Ceil => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundsd(d, ar, 0x0a);
                    }
                    FloatUnOp::Trunc => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.roundsd(d, ar, 0x0b);
                    }
                    FloatUnOp::Sqrt => {
                        if d != ar {
                            self.asm.xorps(d, d);
                        }
                        self.asm.sqrtsd(d, ar);
                    }
                }
                self.fp_commit(r, d);
            }
            Op::F32Bin(bop, a, b) => match bop {
                FloatBinOp::Min => self.emit_float_minmax(false, false, res.unwrap(), a, b),
                FloatBinOp::Max => self.emit_float_minmax(false, true, res.unwrap(), a, b),
                FloatBinOp::Copysign => {
                    let r = res.unwrap();
                    let ar = self.fp_read(a, SCRATCH_FP1);
                    let br = self.fp_read(b, SCRATCH_FP2);
                    let d = self.fp_write_reg(r);
                    // Preserve `br` sign bit in `SCRATCH_FP2`, mag of `ar` in `SCRATCH_FP1`, then OR.
                    self.asm.movaps_rr(SCRATCH_FP2, br);
                    self.asm.xmm_rip_op(
                        None,
                        0x54,
                        SCRATCH_FP2,
                        RelocTarget::Rodata(self.ro.f32_neg),
                    );
                    self.asm.movaps_rr(SCRATCH_FP1, ar);
                    self.asm.xmm_rip_op(
                        None,
                        0x54,
                        SCRATCH_FP1,
                        RelocTarget::Rodata(self.ro.f32_abs),
                    );
                    self.asm.orps(SCRATCH_FP1, SCRATCH_FP2);
                    self.asm.movaps_rr(d, SCRATCH_FP1);
                    self.fp_commit(r, d);
                }
                _ => {
                    let r = res.unwrap();
                    let d = self.fp_write_reg(r);
                    let is_comm = matches!(bop, FloatBinOp::Add | FloatBinOp::Mul);
                    let sse_op = match bop {
                        FloatBinOp::Add => 0x58u8,
                        FloatBinOp::Sub => 0x5cu8,
                        FloatBinOp::Mul => 0x59u8,
                        FloatBinOp::Div => 0x5eu8,
                        _ => unreachable!(),
                    };
                    // Fold spilled stack RHS (or commutative LHS) and RIP-relative non-zero FP constants!
                    match (self.loc(a), self.loc(b)) {
                        (_, Loc::Spill(boff)) => {
                            let ar = self.fp_read(a, d);
                            self.asm.movaps_rr(d, ar);
                            let m = Mem::base_disp(Gp::Rsp, boff);
                            match bop {
                                FloatBinOp::Add => self.asm.addss_xm(d, m),
                                FloatBinOp::Sub => self.asm.subss_xm(d, m),
                                FloatBinOp::Mul => self.asm.mulss_xm(d, m),
                                FloatBinOp::Div => self.asm.divss_xm(d, m),
                                _ => unreachable!(),
                            }
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (Loc::Spill(aoff), Loc::Xmm(bx)) if is_comm => {
                            self.asm.movaps_rr(d, bx);
                            let m = Mem::base_disp(Gp::Rsp, aoff);
                            match bop {
                                FloatBinOp::Add => self.asm.addss_xm(d, m),
                                FloatBinOp::Mul => self.asm.mulss_xm(d, m),
                                _ => unreachable!(),
                            }
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (_, Loc::Remat(RematConst::F32(bits))) if bits != 0 => {
                            let ar = self.fp_read(a, d);
                            self.asm.movaps_rr(d, ar);
                            let loff = self.asm.intern_f32(bits);
                            self.asm.xmm_rip_op(
                                Some(0xf3),
                                sse_op,
                                d,
                                RelocTarget::LocalRodata(loff),
                            );
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (Loc::Remat(RematConst::F32(bits)), Loc::Xmm(bx))
                            if is_comm && bits != 0 =>
                        {
                            self.asm.movaps_rr(d, bx);
                            let loff = self.asm.intern_f32(bits);
                            self.asm.xmm_rip_op(
                                Some(0xf3),
                                sse_op,
                                d,
                                RelocTarget::LocalRodata(loff),
                            );
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        _ => {}
                    }
                    let ar = self.fp_read(a, SCRATCH_FP1);
                    let br = self.fp_read(b, SCRATCH_FP2);
                    let (dest, rhs) = if d == br && d != ar {
                        if is_comm {
                            (d, ar)
                        } else {
                            self.asm.movaps_rr(SCRATCH_FP2, br);
                            self.asm.movaps_rr(d, ar);
                            (d, SCRATCH_FP2)
                        }
                    } else {
                        self.asm.movaps_rr(d, ar);
                        (d, br)
                    };
                    match bop {
                        FloatBinOp::Add => self.asm.addss(dest, rhs),
                        FloatBinOp::Sub => self.asm.subss(dest, rhs),
                        FloatBinOp::Mul => self.asm.mulss(dest, rhs),
                        FloatBinOp::Div => self.asm.divss(dest, rhs),
                        _ => unreachable!(),
                    }
                    self.fp_commit(r, dest);
                }
            },
            Op::F64Bin(bop, a, b) => match bop {
                FloatBinOp::Min => self.emit_float_minmax(true, false, res.unwrap(), a, b),
                FloatBinOp::Max => self.emit_float_minmax(true, true, res.unwrap(), a, b),
                FloatBinOp::Copysign => {
                    let r = res.unwrap();
                    let ar = self.fp_read(a, SCRATCH_FP1);
                    let br = self.fp_read(b, SCRATCH_FP2);
                    let d = self.fp_write_reg(r);
                    self.asm.movaps_rr(SCRATCH_FP2, br);
                    self.asm.xmm_rip_op(
                        Some(0x66),
                        0x54,
                        SCRATCH_FP2,
                        RelocTarget::Rodata(self.ro.f64_neg),
                    );
                    self.asm.movaps_rr(SCRATCH_FP1, ar);
                    self.asm.xmm_rip_op(
                        Some(0x66),
                        0x54,
                        SCRATCH_FP1,
                        RelocTarget::Rodata(self.ro.f64_abs),
                    );
                    self.asm.orps(SCRATCH_FP1, SCRATCH_FP2);
                    self.asm.movaps_rr(d, SCRATCH_FP1);
                    self.fp_commit(r, d);
                }
                _ => {
                    let r = res.unwrap();
                    let d = self.fp_write_reg(r);
                    let is_comm = matches!(bop, FloatBinOp::Add | FloatBinOp::Mul);
                    let sse_op = match bop {
                        FloatBinOp::Add => 0x58u8,
                        FloatBinOp::Sub => 0x5cu8,
                        FloatBinOp::Mul => 0x59u8,
                        FloatBinOp::Div => 0x5eu8,
                        _ => unreachable!(),
                    };
                    match (self.loc(a), self.loc(b)) {
                        (_, Loc::Spill(boff)) => {
                            let ar = self.fp_read(a, d);
                            self.asm.movaps_rr(d, ar);
                            let m = Mem::base_disp(Gp::Rsp, boff);
                            match bop {
                                FloatBinOp::Add => self.asm.addsd_xm(d, m),
                                FloatBinOp::Sub => self.asm.subsd_xm(d, m),
                                FloatBinOp::Mul => self.asm.mulsd_xm(d, m),
                                FloatBinOp::Div => self.asm.divsd_xm(d, m),
                                _ => unreachable!(),
                            }
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (Loc::Spill(aoff), Loc::Xmm(bx)) if is_comm => {
                            self.asm.movaps_rr(d, bx);
                            let m = Mem::base_disp(Gp::Rsp, aoff);
                            match bop {
                                FloatBinOp::Add => self.asm.addsd_xm(d, m),
                                FloatBinOp::Mul => self.asm.mulsd_xm(d, m),
                                _ => unreachable!(),
                            }
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (_, Loc::Remat(RematConst::F64(bits))) if bits != 0 => {
                            let ar = self.fp_read(a, d);
                            self.asm.movaps_rr(d, ar);
                            let loff = self.asm.intern_f64(bits);
                            self.asm.xmm_rip_op(
                                Some(0xf2),
                                sse_op,
                                d,
                                RelocTarget::LocalRodata(loff),
                            );
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        (Loc::Remat(RematConst::F64(bits)), Loc::Xmm(bx))
                            if is_comm && bits != 0 =>
                        {
                            self.asm.movaps_rr(d, bx);
                            let loff = self.asm.intern_f64(bits);
                            self.asm.xmm_rip_op(
                                Some(0xf2),
                                sse_op,
                                d,
                                RelocTarget::LocalRodata(loff),
                            );
                            self.fp_commit(r, d);
                            return Ok(());
                        }
                        _ => {}
                    }
                    let ar = self.fp_read(a, SCRATCH_FP1);
                    let br = self.fp_read(b, SCRATCH_FP2);
                    let (dest, rhs) = if d == br && d != ar {
                        if is_comm {
                            (d, ar)
                        } else {
                            self.asm.movaps_rr(SCRATCH_FP2, br);
                            self.asm.movaps_rr(d, ar);
                            (d, SCRATCH_FP2)
                        }
                    } else {
                        self.asm.movaps_rr(d, ar);
                        (d, br)
                    };
                    match bop {
                        FloatBinOp::Add => self.asm.addsd(dest, rhs),
                        FloatBinOp::Sub => self.asm.subsd(dest, rhs),
                        FloatBinOp::Mul => self.asm.mulsd(dest, rhs),
                        FloatBinOp::Div => self.asm.divsd(dest, rhs),
                        _ => unreachable!(),
                    }
                    self.fp_commit(r, dest);
                }
            },
            Op::F32DemoteF64(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.fp_write_reg(r);
                if d != ar {
                    self.asm.xorps(d, d);
                }
                self.asm.cvtsd2ss(d, ar);
                self.fp_commit(r, d);
            }
            Op::F64PromoteF32(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.fp_write_reg(r);
                if d != ar {
                    self.asm.xorps(d, d);
                }
                self.asm.cvtss2sd(d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2ss(false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI32U(a) => {
                // `I32` values are zero-extended to 64-bit, so 64-bit signed cvt is exact for u32!
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2ss(true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI64S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2ss(true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI64U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                let lbl_hi = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                self.asm.xorps(d, d);
                self.asm.test_rr(true, ar, ar);
                self.asm.jcc(Cond::S, lbl_hi);
                self.asm.cvtsi2ss(true, d, ar);
                self.asm.jmp(lbl_done);
                self.asm.bind(lbl_hi);
                // Capture original bit 0 in `SCRATCH_GP2` BEFORE shifting `SCRATCH_GP1` (which may alias `ar` when spilled!).
                self.asm.mov_rr(true, SCRATCH_GP2, ar);
                self.asm.and_ri(false, SCRATCH_GP2, 1);
                self.asm.mov_rr(true, SCRATCH_GP1, ar);
                self.asm.shr_imm(true, SCRATCH_GP1, 1);
                self.asm.or_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                self.asm.cvtsi2ss(true, d, SCRATCH_GP1);
                self.asm.addss(d, d);
                self.asm.bind(lbl_done);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2sd(false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI32U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2sd(true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI64S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.xorps(d, d);
                self.asm.cvtsi2sd(true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI64U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                let lbl_hi = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                self.asm.xorps(d, d);
                self.asm.test_rr(true, ar, ar);
                self.asm.jcc(Cond::S, lbl_hi);
                self.asm.cvtsi2sd(true, d, ar);
                self.asm.jmp(lbl_done);
                self.asm.bind(lbl_hi);
                // Capture original bit 0 in `SCRATCH_GP2` BEFORE shifting `SCRATCH_GP1` (which may alias `ar` when spilled!).
                self.asm.mov_rr(true, SCRATCH_GP2, ar);
                self.asm.and_ri(false, SCRATCH_GP2, 1);
                self.asm.mov_rr(true, SCRATCH_GP1, ar);
                self.asm.shr_imm(true, SCRATCH_GP1, 1);
                self.asm.or_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                self.asm.cvtsi2sd(true, d, SCRATCH_GP1);
                self.asm.addsd(d, d);
                self.asm.bind(lbl_done);
                self.fp_commit(r, d);
            }
            Op::I32ReinterpretF32(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.gp_write_reg(r);
                self.asm.movd_rx(d, ar);
                self.gp_commit(r, d);
            }
            Op::I64ReinterpretF64(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                let d = self.gp_write_reg(r);
                self.asm.movq_rx(d, ar);
                self.gp_commit(r, d);
            }
            Op::F32ReinterpretI32(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.movd_xr(d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ReinterpretI64(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.movq_xr(d, ar);
                self.fp_commit(r, d);
            }
            Op::Trunc(top, a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SCRATCH_FP1);
                self.asm.movaps_rr(SCRATCH_FP1, ar);
                self.asm
                    .call_reloc(RelocTarget::Helper(HELPER_TRUNC_BASE + top as u32));
                if top.can_trap() {
                    let prop = self.trap_prop_lbl();
                    self.asm.test_rr(false, SCRATCH_GP1, SCRATCH_GP1);
                    self.asm.jcc(Cond::Ne, prop);
                }
                self.gp_commit(r, Gp::Rax);
            }
            Op::Select { cond, a, b } => {
                let r = res.unwrap();
                let ty = self.ty(r);
                let cr = self.gp_read(cond, SCRATCH_GP2);
                if ty.is_fp() {
                    let ar = self.fp_read(a, SCRATCH_FP1);
                    let br = self.fp_read(b, SCRATCH_FP2);
                    let d = self.fp_write_reg(r);
                    let lbl_use_a = self.asm.new_label();
                    let lbl_done = self.asm.new_label();
                    self.asm.test_rr(false, cr, cr);
                    self.asm.jcc(Cond::Ne, lbl_use_a);
                    self.asm.movaps_rr(d, br);
                    self.asm.jmp(lbl_done);
                    self.asm.bind(lbl_use_a);
                    self.asm.movaps_rr(d, ar);
                    self.asm.bind(lbl_done);
                    self.fp_commit(r, d);
                } else {
                    let is64 = ty == Ty::I64;
                    let d = self.gp_write_reg(r);
                    match (self.loc(a), self.loc(b), self.loc(cond)) {
                        (Loc::Gp(ag), Loc::Gp(bg), Loc::Gp(cg)) => {
                            self.asm.test_rr(false, cg, cg);
                            if d == ag {
                                self.asm.cmovcc(Cond::E, is64, d, bg);
                            } else if d == bg {
                                self.asm.cmovcc(Cond::Ne, is64, d, ag);
                            } else {
                                self.asm.mov_rr(is64, d, bg);
                                self.asm.cmovcc(Cond::Ne, is64, d, ag);
                            }
                        }
                        _ => {
                            let lbl_a = self.asm.new_label();
                            let lbl_done = self.asm.new_label();
                            self.asm.test_rr(false, cr, cr);
                            self.asm.jcc(Cond::Ne, lbl_a);
                            let br = self.gp_read(b, d);
                            if br != d {
                                self.asm.mov_rr(is64, d, br);
                            }
                            self.asm.jmp(lbl_done);
                            self.asm.bind(lbl_a);
                            let ar = self.gp_read(a, d);
                            if ar != d {
                                self.asm.mov_rr(is64, d, ar);
                            }
                            self.asm.bind(lbl_done);
                        }
                    }
                    self.gp_commit(r, d);
                }
            }
            Op::CmpSelect { kind, a, b, tv, fv } => {
                let r = res.unwrap();
                let ty = self.ty(r);
                // Emit comparison flags for `kind` (never modifies GP/XMM values other than scratches).
                let emit_cmp_flags = |l: &mut Lower| -> (Cond, Option<CmpOp>) {
                    match kind {
                        CmpBranchKind::I32(cop) => {
                            let ar = l.gp_read(a, SCRATCH_GP1);
                            if let Loc::Spill(boff) = l.loc(b) {
                                l.asm.cmp_rm(false, ar, Mem::base_disp(Gp::Rsp, boff));
                            } else {
                                let br = l.gp_read(b, SCRATCH_GP2);
                                l.asm.cmp_rr(false, ar, br);
                            }
                            (cmp_to_cond(cop), None)
                        }
                        CmpBranchKind::I32Imm(cop, imm) => {
                            let ar = l.gp_read(a, SCRATCH_GP1);
                            if imm == 0 {
                                l.asm.test_rr(false, ar, ar);
                            } else {
                                l.asm.cmp_ri(false, ar, imm);
                            }
                            (cmp_to_cond(cop), None)
                        }
                        CmpBranchKind::I64(cop) => {
                            let ar = l.gp_read(a, SCRATCH_GP1);
                            if let Loc::Spill(boff) = l.loc(b) {
                                l.asm.cmp_rm(true, ar, Mem::base_disp(Gp::Rsp, boff));
                            } else {
                                let br = l.gp_read(b, SCRATCH_GP2);
                                l.asm.cmp_rr(true, ar, br);
                            }
                            (cmp_to_cond(cop), None)
                        }
                        CmpBranchKind::I64Imm(cop, imm) => {
                            let ar = l.gp_read(a, SCRATCH_GP1);
                            if imm == 0 {
                                l.asm.test_rr(true, ar, ar);
                            } else {
                                l.asm.cmp_ri(true, ar, imm);
                            }
                            (cmp_to_cond(cop), None)
                        }
                        CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                            let is64 = matches!(kind, CmpBranchKind::F64(_));
                            match cop {
                                CmpOp::GtS => {
                                    let ar = l.fp_read(a, SCRATCH_FP1);
                                    l.emit_ucomi_rhs(is64, ar, b);
                                    (Cond::A, None)
                                }
                                CmpOp::GeS => {
                                    let ar = l.fp_read(a, SCRATCH_FP1);
                                    l.emit_ucomi_rhs(is64, ar, b);
                                    (Cond::Ae, None)
                                }
                                CmpOp::LtS => {
                                    let br = l.fp_read(b, SCRATCH_FP1);
                                    l.emit_ucomi_rhs(is64, br, a);
                                    (Cond::A, None)
                                }
                                CmpOp::LeS => {
                                    let br = l.fp_read(b, SCRATCH_FP1);
                                    l.emit_ucomi_rhs(is64, br, a);
                                    (Cond::Ae, None)
                                }
                                CmpOp::Eq => {
                                    let ar = l.fp_read(a, SCRATCH_FP1);
                                    if a == b {
                                        if is64 {
                                            l.asm.ucomisd(ar, ar);
                                        } else {
                                            l.asm.ucomiss(ar, ar);
                                        }
                                        (Cond::Np, None)
                                    } else {
                                        l.emit_ucomi_rhs(is64, ar, b);
                                        (Cond::E, Some(CmpOp::Eq))
                                    }
                                }
                                CmpOp::Ne => {
                                    let ar = l.fp_read(a, SCRATCH_FP1);
                                    if a == b {
                                        if is64 {
                                            l.asm.ucomisd(ar, ar);
                                        } else {
                                            l.asm.ucomiss(ar, ar);
                                        }
                                        (Cond::P, None)
                                    } else {
                                        l.emit_ucomi_rhs(is64, ar, b);
                                        (Cond::Ne, Some(CmpOp::Ne))
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                };
                if !ty.is_fp() {
                    let is64 = ty == Ty::I64;
                    let d = self.gp_write_reg(r);
                    let (cond, fp_eq_ne) = emit_cmp_flags(self);
                    // Note: `gp_read_preserve_flags` & `mov` NEVER modify EFLAGS on x86-64 (even for 0)!
                    if let Some(fp_op) = fp_eq_ne {
                        // Unordered-safe branchless FP Eq/Ne select:
                        //   Eq: true iff ZF=1 && PF=0 -> start with fv, cmove tv, cmovp fv
                        //   Ne: true iff ZF=0 || PF=1 -> start with fv, cmovne tv, cmovp tv
                        let tv_reg = if self.loc(tv) == Loc::Gp(d) {
                            self.asm.mov_rr(is64, SCRATCH_GP2, d);
                            SCRATCH_GP2
                        } else {
                            self.gp_read_preserve_flags(tv, SCRATCH_GP2)
                        };
                        let fv_reg = if self.loc(fv) == Loc::Gp(d) {
                            d
                        } else {
                            let fr = self.gp_read_preserve_flags(fv, d);
                            if fr != d {
                                self.asm.mov_rr(is64, d, fr);
                            }
                            fr
                        };
                        if fp_op == CmpOp::Eq {
                            // Need a register holding `fv` for `cmovp` that isn't `d`!
                            let fv_hold = if fv_reg != d {
                                fv_reg
                            } else {
                                // `d` originally held `fv`; save `fv` into `SCRATCH_GP1` if `d != SCRATCH_GP1`!
                                if d != SCRATCH_GP1 {
                                    self.asm.mov_rr(is64, SCRATCH_GP1, d);
                                    SCRATCH_GP1
                                } else {
                                    // `dst` is spilled (`d == SCRATCH_GP1`), reload `fv` via stack isn't clobbered by cmov.
                                    let lbl_f = self.asm.new_label();
                                    self.asm.jcc(Cond::P, lbl_f);
                                    self.asm.cmovcc(Cond::E, is64, d, tv_reg);
                                    self.asm.bind(lbl_f);
                                    self.gp_commit(r, d);
                                    return Ok(());
                                }
                            };
                            self.asm.cmovcc(Cond::E, is64, d, tv_reg);
                            self.asm.cmovcc(Cond::P, is64, d, fv_hold);
                        } else {
                            self.asm.cmovcc(Cond::Ne, is64, d, tv_reg);
                            self.asm.cmovcc(Cond::P, is64, d, tv_reg);
                        }
                    } else if self.loc(tv) == Loc::Gp(d) {
                        let fv_reg = self.gp_read_preserve_flags(fv, SCRATCH_GP2);
                        self.asm.cmovcc(cond.invert(), is64, d, fv_reg);
                    } else if self.loc(fv) == Loc::Gp(d) {
                        let tv_reg = self.gp_read_preserve_flags(tv, SCRATCH_GP2);
                        self.asm.cmovcc(cond, is64, d, tv_reg);
                    } else {
                        let fr = self.gp_read_preserve_flags(fv, d);
                        if fr != d {
                            self.asm.mov_rr(is64, d, fr);
                        }
                        let tv_reg = self.gp_read_preserve_flags(tv, SCRATCH_GP2);
                        self.asm.cmovcc(cond, is64, d, tv_reg);
                    }
                    self.gp_commit(r, d);
                } else {
                    let d = self.fp_write_reg(r);
                    let (cond, fp_eq_ne) = emit_cmp_flags(self);
                    let lbl_use_tv = self.asm.new_label();
                    let lbl_done = self.asm.new_label();
                    let tv_same = self.loc(tv) == Loc::Xmm(d);
                    let fv_same = self.loc(fv) == Loc::Xmm(d);
                    if fp_eq_ne.is_none() && fv_same {
                        // `d` already holds `fv`! Only overwrite with `tv` if condition is true.
                        self.asm.jcc(cond.invert(), lbl_done);
                        let tr = self.fp_read(tv, d);
                        if tr != d {
                            self.asm.movaps_rr(d, tr);
                        }
                        self.asm.bind(lbl_done);
                    } else if fp_eq_ne.is_none() {
                        // SSE `movaps`/`movss`/`movsd`/`xorps` NEVER modify EFLAGS on x86-64:
                        // preload `tv` into `d` (unless `tv_same`), skip `fv` with a single `jcc` (zero `jmp`!).
                        if !tv_same {
                            let tr = self.fp_read(tv, d);
                            if tr != d {
                                self.asm.movaps_rr(d, tr);
                            }
                        }
                        self.asm.jcc(cond, lbl_done);
                        let fr = self.fp_read(fv, d);
                        if fr != d {
                            self.asm.movaps_rr(d, fr);
                        }
                        self.asm.bind(lbl_done);
                    } else {
                        match fp_eq_ne {
                            Some(CmpOp::Eq) => {
                                let lbl_fv = self.asm.new_label();
                                self.asm.jcc(Cond::P, lbl_fv);
                                self.asm.jcc(Cond::E, lbl_use_tv);
                                self.asm.bind(lbl_fv);
                            }
                            Some(CmpOp::Ne) => {
                                self.asm.jcc(Cond::P, lbl_use_tv);
                                self.asm.jcc(Cond::Ne, lbl_use_tv);
                            }
                            _ => {
                                self.asm.jcc(cond, lbl_use_tv);
                            }
                        }
                        let fr = self.fp_read(fv, d);
                        if fr != d {
                            self.asm.movaps_rr(d, fr);
                        }
                        self.asm.jmp(lbl_done);
                        self.asm.bind(lbl_use_tv);
                        let tr = self.fp_read(tv, d);
                        if tr != d {
                            self.asm.movaps_rr(d, tr);
                        }
                        self.asm.bind(lbl_done);
                    }
                    self.fp_commit(r, d);
                }
            }
            Op::GlobalGet(g) => {
                let r = res.unwrap();
                let ty = self.ty(r);
                if self.layout.pinned_global == Some(g) {
                    let d = self.gp_write_reg(r);
                    self.asm.mov_rr(ty == Ty::I64, d, Gp::R12);
                    self.gp_commit(r, d);
                } else {
                    let off = self.layout.global_offs[g as usize];
                    let mem = Mem::base_disp(Gp::R15, off);
                    if ty.is_fp() {
                        let d = self.fp_write_reg(r);
                        emit_fp_load(&mut self.asm, d, mem, ty);
                        self.fp_commit(r, d);
                    } else {
                        let d = self.gp_write_reg(r);
                        self.asm.mov_rm(ty == Ty::I64, d, mem);
                        self.gp_commit(r, d);
                    }
                }
            }
            Op::GlobalSet(g, v) => {
                let ty = self.ty(v);
                if self.layout.pinned_global == Some(g) {
                    let vr = self.gp_read(v, Gp::R12);
                    if vr != Gp::R12 {
                        self.asm.mov_rr(ty == Ty::I64, Gp::R12, vr);
                    }
                } else {
                    let off = self.layout.global_offs[g as usize];
                    let mem = Mem::base_disp(Gp::R15, off);
                    match self.loc(v) {
                        Loc::Remat(RematConst::I32(c)) => {
                            self.asm.mov_mi32(false, mem, c);
                        }
                        Loc::Remat(RematConst::I64(c)) if i32::try_from(c).is_ok() => {
                            self.asm.mov_mi32(true, mem, c as i32);
                        }
                        Loc::Remat(RematConst::F32(bits)) => {
                            self.asm.mov_mi32(false, mem, bits as i32);
                        }
                        Loc::Remat(RematConst::F64(bits)) if i32::try_from(bits as i64).is_ok() => {
                            self.asm.mov_mi32(true, mem, bits as i64 as i32);
                        }
                        _ if ty.is_fp() => {
                            let vr = self.fp_read(v, SCRATCH_FP1);
                            emit_fp_store(&mut self.asm, mem, vr, ty);
                        }
                        _ => {
                            let vr = self.gp_read(v, SCRATCH_GP1);
                            self.asm.mov_mr(ty == Ty::I64, mem, vr);
                        }
                    }
                }
            }
            Op::TableGet { table, idx } => {
                let r = res.unwrap();
                let (toff, tlen) = self.layout.extern_tables[&table];
                let ir = self.gp_read(idx, SCRATCH_GP2);
                let toob = self.trap_table_oob_lbl();
                self.asm.cmp_ri(false, ir, tlen as i32);
                self.asm.jcc(Cond::Ae, toob);
                let d = self.gp_write_reg(r);
                self.asm.mov_rm(true, d, Mem::indexed(Gp::R15, ir, 8, toff));
                self.gp_commit(r, d);
            }
            Op::TableSet { table, idx, val } => {
                let (toff, tlen) = self.layout.extern_tables[&table];
                let ir = self.gp_read(idx, SCRATCH_GP2);
                let toob = self.trap_table_oob_lbl();
                self.asm.cmp_ri(false, ir, tlen as i32);
                self.asm.jcc(Cond::Ae, toob);
                let vr = self.gp_read(val, SCRATCH_GP1);
                self.asm
                    .mov_mr(true, Mem::indexed(Gp::R15, ir, 8, toff), vr);
            }
            Op::BoundsCheck { addr, end_off } => {
                let ar = self.gp_read(addr, SCRATCH_GP2);
                self.emit_bounds_check(ar, end_off);
            }
            Op::Load {
                kind,
                addr,
                offset,
                checked,
            } => {
                let r = res.unwrap();
                let ar = self.gp_read(addr, SCRATCH_GP2);
                if !checked {
                    self.emit_bounds_check(ar, (offset as u64) + kind.bytes());
                }
                let mem = self.mem_addr(ar, offset);
                if kind.ty().is_fp() {
                    let d = self.fp_write_reg(r);
                    emit_fp_load(&mut self.asm, d, mem, kind.ty());
                    self.fp_commit(r, d);
                } else {
                    let d = self.gp_write_reg(r);
                    match kind {
                        LoadKind::I32 | LoadKind::I64Load32U => self.asm.mov_rm(false, d, mem),
                        LoadKind::I64 => self.asm.mov_rm(true, d, mem),
                        LoadKind::I32Load8S => self.asm.movsx_rm8(false, d, mem),
                        LoadKind::I32Load8U | LoadKind::I64Load8U => {
                            self.asm.movzx_rm8(false, d, mem)
                        }
                        LoadKind::I32Load16S => self.asm.movsx_rm16(false, d, mem),
                        LoadKind::I32Load16U | LoadKind::I64Load16U => {
                            self.asm.movzx_rm16(false, d, mem)
                        }
                        LoadKind::I64Load8S => self.asm.movsx_rm8(true, d, mem),
                        LoadKind::I64Load16S => self.asm.movsx_rm16(true, d, mem),
                        LoadKind::I64Load32S => self.asm.movsxd_rm(d, mem),
                        _ => unreachable!(),
                    }
                    self.gp_commit(r, d);
                }
            }
            Op::Store {
                kind,
                addr,
                val,
                offset,
                checked,
            } => {
                let ar = self.gp_read(addr, SCRATCH_GP2);
                if !checked {
                    self.emit_bounds_check(ar, (offset as u64) + kind.bytes());
                }
                // Careful: if `offset > i32::MAX`, `mem_addr` uses `SCRATCH_GP1`, so read `val` cleanly!
                let mem = self.mem_addr(ar, offset);
                match kind {
                    StoreKind::F32 => {
                        if let Loc::Remat(RematConst::F32(bits)) = self.loc(val) {
                            self.asm.mov_mi32(false, mem, bits as i32);
                        } else {
                            let vr = self.fp_read(val, SCRATCH_FP1);
                            emit_fp_store(&mut self.asm, mem, vr, Ty::F32);
                        }
                    }
                    StoreKind::F64 => {
                        if let Loc::Remat(RematConst::F64(bits)) = self.loc(val)
                            && let Ok(imm32) = i32::try_from(bits as i64)
                        {
                            self.asm.mov_mi32(true, mem, imm32);
                        } else {
                            let vr = self.fp_read(val, SCRATCH_FP1);
                            emit_fp_store(&mut self.asm, mem, vr, Ty::F64);
                        }
                    }
                    StoreKind::V128 => {
                        let vr = self.fp_read(val, SCRATCH_FP1);
                        emit_fp_store(&mut self.asm, mem, vr, Ty::V128);
                    }
                    StoreKind::I8 | StoreKind::I16 | StoreKind::I32 | StoreKind::I64 => {
                        // Direct immediate store fast path when `val` is a rematerialized constant!
                        match (kind, self.loc(val)) {
                            (StoreKind::I8, Loc::Remat(RematConst::I32(c))) => {
                                self.asm.mov_mi8(mem, c as u8);
                                return Ok(());
                            }
                            (StoreKind::I8, Loc::Remat(RematConst::I64(c))) => {
                                self.asm.mov_mi8(mem, c as u8);
                                return Ok(());
                            }
                            (StoreKind::I32, Loc::Remat(RematConst::I32(c))) => {
                                self.asm.mov_mi32(false, mem, c);
                                return Ok(());
                            }
                            (StoreKind::I32, Loc::Remat(RematConst::I64(c))) => {
                                self.asm.mov_mi32(false, mem, c as i32);
                                return Ok(());
                            }
                            (StoreKind::I64, Loc::Remat(RematConst::I64(c)))
                                if i32::try_from(c).is_ok() =>
                            {
                                self.asm.mov_mi32(true, mem, c as i32);
                                return Ok(());
                            }
                            _ => {}
                        }
                        // If `offset > i32::MAX`, `mem` uses `SCRATCH_GP1` as index! Move `mem` base to `SCRATCH_GP2` first if `val` is not in a GP reg!
                        let (mem, vr) =
                            if offset > i32::MAX as u32 && !matches!(self.loc(val), Loc::Gp(_)) {
                                self.asm.mov_rr(true, SCRATCH_GP2, SCRATCH_GP1);
                                let vr = self.gp_read(val, SCRATCH_GP1);
                                (Mem::indexed(Gp::R14, SCRATCH_GP2, 1, 0), vr)
                            } else {
                                let vr = self.gp_read(val, SCRATCH_GP1);
                                (mem, vr)
                            };
                        match kind {
                            StoreKind::I8 => self.asm.mov_mr8(mem, vr),
                            StoreKind::I16 => self.asm.mov_mr16(mem, vr),
                            StoreKind::I32 => self.asm.mov_mr(false, mem, vr),
                            StoreKind::I64 => self.asm.mov_mr(true, mem, vr),
                            _ => unreachable!(),
                        }
                    }
                }
            }
            Op::MemSize => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                if let Some(cap) = self.layout.fixed_mem_bytes {
                    self.asm.mov_ri32(d, (cap >> 16) as u32);
                } else {
                    self.asm.mov_rr(true, d, Gp::R13);
                    self.asm.shr_imm(true, d, 16);
                }
                self.gp_commit(r, d);
            }
            Op::MemGrow(delta) => {
                let r = res.unwrap();
                let dr = self.gp_read(delta, Gp::Rdi);
                if dr != Gp::Rdi {
                    self.asm.mov_rr(false, Gp::Rdi, dr);
                }
                self.asm.call_reloc(RelocTarget::Helper(HELPER_MEM_GROW));
                self.gp_commit(r, Gp::Rax);
            }
            Op::MemCopy { dst, src, len } => {
                let moves = [
                    (MoveLoc::Gp(Gp::Rdi), MoveLoc::from(self.loc(dst)), Ty::I32),
                    (MoveLoc::Gp(Gp::Rsi), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Gp::Rdx), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                self.asm.call_reloc(RelocTarget::Helper(HELPER_MEM_COPY));
                let prop = self.trap_prop_lbl();
                self.asm.test_rr(false, SCRATCH_GP1, SCRATCH_GP1);
                self.asm.jcc(Cond::Ne, prop);
            }
            Op::MemFill { dst, val, len } => {
                let moves = [
                    (MoveLoc::Gp(Gp::Rdi), MoveLoc::from(self.loc(dst)), Ty::I32),
                    (MoveLoc::Gp(Gp::Rsi), MoveLoc::from(self.loc(val)), Ty::I32),
                    (MoveLoc::Gp(Gp::Rdx), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                self.asm.call_reloc(RelocTarget::Helper(HELPER_MEM_FILL));
                let prop = self.trap_prop_lbl();
                self.asm.test_rr(false, SCRATCH_GP1, SCRATCH_GP1);
                self.asm.jcc(Cond::Ne, prop);
            }
            Op::MemCopyConst {
                dst,
                src,
                len,
                dst_checked,
                src_checked,
            } => {
                // Mandatory bounds checks FIRST unless proven by BCE (even when len == 0!).
                if !dst_checked {
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    self.emit_bounds_check(dr, u64::from(len));
                }
                if !src_checked {
                    let sr = self.gp_read(src, SCRATCH_GP2);
                    self.emit_bounds_check(sr, u64::from(len));
                }
                if len == 0 {
                    // Never dereference R14 (NULL on 0-capacity memories).
                } else if len == 1 {
                    let sr = self.gp_read(src, SCRATCH_GP1);
                    self.asm
                        .movzx_rm8(false, SCRATCH_GP1, Mem::indexed(Gp::R14, sr, 1, 0));
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    self.asm
                        .mov_mr8(Mem::indexed(Gp::R14, dr, 1, 0), SCRATCH_GP1);
                } else if len == 2 {
                    let sr = self.gp_read(src, SCRATCH_GP1);
                    self.asm
                        .movzx_rm16(false, SCRATCH_GP1, Mem::indexed(Gp::R14, sr, 1, 0));
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    self.asm
                        .mov_mr16(Mem::indexed(Gp::R14, dr, 1, 0), SCRATCH_GP1);
                } else if len == 3 {
                    let sr = self.gp_read(src, SCRATCH_GP2);
                    self.asm
                        .movzx_rm16(false, SCRATCH_GP1, Mem::indexed(Gp::R14, sr, 1, 0));
                    self.asm
                        .movzx_rm8(false, SCRATCH_GP2, Mem::indexed(Gp::R14, sr, 1, 2));
                    self.asm.movd_xr(SCRATCH_FP2, SCRATCH_GP2);
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    self.asm
                        .mov_mr16(Mem::indexed(Gp::R14, dr, 1, 0), SCRATCH_GP1);
                    self.asm.movd_rx(SCRATCH_GP1, SCRATCH_FP2);
                    self.asm
                        .mov_mr8(Mem::indexed(Gp::R14, dr, 1, 2), SCRATCH_GP1);
                } else {
                    // Materialize `SCRATCH_GP2 = R14 + dst` and `SCRATCH_GP1 = R14 + src` (zero extra GP regs!).
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    if dr != SCRATCH_GP2 {
                        self.asm.mov_rr(false, SCRATCH_GP2, dr);
                    }
                    self.asm.add_rr(true, SCRATCH_GP2, Gp::R14);
                    let sr = self.gp_read(src, SCRATCH_GP1);
                    if sr != SCRATCH_GP1 {
                        self.asm.mov_rr(false, SCRATCH_GP1, sr);
                    }
                    self.asm.add_rr(true, SCRATCH_GP1, Gp::R14);
                    let l = len as i32;
                    if len == 4 {
                        self.asm.movss_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm.movss_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                    } else if len <= 7 {
                        self.asm.movss_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm
                            .movss_xm(SCRATCH_FP2, Mem::base_disp(SCRATCH_GP1, l - 4));
                        self.asm.movss_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                        self.asm
                            .movss_mx(Mem::base_disp(SCRATCH_GP2, l - 4), SCRATCH_FP2);
                    } else if len == 8 {
                        self.asm.movsd_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm.movsd_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                    } else if len <= 15 {
                        self.asm.movsd_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm
                            .movsd_xm(SCRATCH_FP2, Mem::base_disp(SCRATCH_GP1, l - 8));
                        self.asm.movsd_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                        self.asm
                            .movsd_mx(Mem::base_disp(SCRATCH_GP2, l - 8), SCRATCH_FP2);
                    } else if len == 16 {
                        self.asm.movdqu_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                    } else if len <= 32 {
                        self.asm.movdqu_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm
                            .movdqu_xm(SCRATCH_FP2, Mem::base_disp(SCRATCH_GP1, l - 16));
                        self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                        self.asm
                            .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 16), SCRATCH_FP2);
                    } else {
                        // Overlap-safe 33..=64 bytes: branch on `dst > src`.
                        let lbl_bwd = self.asm.new_label();
                        let lbl_done = self.asm.new_label();
                        self.asm.cmp_rr(true, SCRATCH_GP2, SCRATCH_GP1);
                        self.asm.jcc(Cond::A, lbl_bwd);
                        // Forward (`dst <= src`): write non-overlapping prefix chunks, then load last 2 overlapping chunks before writing both!
                        self.asm.movdqu_xm(SCRATCH_FP1, Mem::base(SCRATCH_GP1));
                        self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                        if len <= 48 {
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, 16));
                            self.asm
                                .movdqu_xm(SCRATCH_FP2, Mem::base_disp(SCRATCH_GP1, l - 16));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, 16), SCRATCH_FP1);
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 16), SCRATCH_FP2);
                        } else {
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, 16));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, 16), SCRATCH_FP1);
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, 32));
                            self.asm
                                .movdqu_xm(SCRATCH_FP2, Mem::base_disp(SCRATCH_GP1, l - 16));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, 32), SCRATCH_FP1);
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 16), SCRATCH_FP2);
                        }
                        self.asm.jmp(lbl_done);
                        self.asm.bind(lbl_bwd);
                        // Backward (`dst > src`): write non-overlapping suffix chunks, then load first 2 overlapping chunks before writing both!
                        self.asm
                            .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, l - 16));
                        self.asm
                            .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 16), SCRATCH_FP1);
                        if len <= 48 {
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, l - 32));
                            self.asm.movdqu_xm(SCRATCH_FP2, Mem::base(SCRATCH_GP1));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 32), SCRATCH_FP1);
                            self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP2);
                        } else {
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, l - 32));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 32), SCRATCH_FP1);
                            self.asm
                                .movdqu_xm(SCRATCH_FP1, Mem::base_disp(SCRATCH_GP1, l - 48));
                            self.asm.movdqu_xm(SCRATCH_FP2, Mem::base(SCRATCH_GP1));
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 48), SCRATCH_FP1);
                            self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP2);
                        }
                        self.asm.bind(lbl_done);
                    }
                }
            }
            Op::MemFillConst {
                dst,
                val,
                len,
                checked,
            } => {
                // Mandatory bounds check FIRST unless proven by BCE (even when len == 0!).
                if !checked {
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    self.emit_bounds_check(dr, u64::from(len));
                }
                if len > 0 {
                    let vr = self.gp_read(val, SCRATCH_GP1);
                    self.asm.movzx_rr8(SCRATCH_GP1, vr);
                    if len >= 2 {
                        self.asm.mov_ri64(SCRATCH_GP2, 0x0101_0101_0101_0101);
                        self.asm.imul_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                    }
                    let dr = self.gp_read(dst, SCRATCH_GP2);
                    if dr != SCRATCH_GP2 {
                        self.asm.mov_rr(false, SCRATCH_GP2, dr);
                    }
                    self.asm.add_rr(true, SCRATCH_GP2, Gp::R14);
                    let l = len as i32;
                    if len == 1 {
                        self.asm.mov_mr8(Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                    } else if len == 2 {
                        self.asm.mov_mr16(Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                    } else if len == 3 {
                        self.asm.mov_mr16(Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                        self.asm
                            .mov_mr8(Mem::base_disp(SCRATCH_GP2, 2), SCRATCH_GP1);
                    } else if len == 4 {
                        self.asm.mov_mr(false, Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                    } else if len <= 7 {
                        self.asm.mov_mr(false, Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                        self.asm
                            .mov_mr(false, Mem::base_disp(SCRATCH_GP2, l - 4), SCRATCH_GP1);
                    } else if len == 8 {
                        self.asm.mov_mr(true, Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                    } else if len <= 15 {
                        self.asm.mov_mr(true, Mem::base(SCRATCH_GP2), SCRATCH_GP1);
                        self.asm
                            .mov_mr(true, Mem::base_disp(SCRATCH_GP2, l - 8), SCRATCH_GP1);
                    } else {
                        self.asm.movq_xr(SCRATCH_FP1, SCRATCH_GP1);
                        self.asm.punpcklqdq(SCRATCH_FP1, SCRATCH_FP1);
                        self.asm.movdqu_mx(Mem::base(SCRATCH_GP2), SCRATCH_FP1);
                        if len > 32 {
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, 16), SCRATCH_FP1);
                        }
                        if len > 48 {
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, 32), SCRATCH_FP1);
                        }
                        if len > 16 {
                            self.asm
                                .movdqu_mx(Mem::base_disp(SCRATCH_GP2, l - 16), SCRATCH_FP1);
                        }
                    }
                }
            }
            Op::V128Splat { lane_bytes, val } => {
                let r = res.unwrap();
                let vr = self.gp_read(val, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                match lane_bytes {
                    1 => {
                        self.asm.movd_xr(d, vr);
                        self.asm.punpcklbw(d, d);
                        self.asm.punpcklwd(d, d);
                        self.asm.punpckldq(d, d);
                        self.asm.punpcklqdq(d, d);
                    }
                    2 => {
                        self.asm.movd_xr(d, vr);
                        self.asm.punpcklwd(d, d);
                        self.asm.punpckldq(d, d);
                        self.asm.punpcklqdq(d, d);
                    }
                    4 => {
                        self.asm.movd_xr(d, vr);
                        self.asm.punpckldq(d, d);
                        self.asm.punpcklqdq(d, d);
                    }
                    8 => {
                        self.asm.movq_xr(d, vr);
                        self.asm.punpcklqdq(d, d);
                    }
                    _ => unreachable!(),
                }
                self.fp_commit(r, d);
            }
            Op::I64x2ExtractLane { v, lane } => {
                let r = res.unwrap();
                let vr = self.fp_read(v, SCRATCH_FP1);
                let d = self.gp_write_reg(r);
                if lane == 0 {
                    self.asm.movq_rx(d, vr);
                } else {
                    self.asm.pextrq(d, vr, lane);
                }
                self.gp_commit(r, d);
            }
            Op::I64x2ReplaceLane { v, x, lane } => {
                let r = res.unwrap();
                let vr = self.fp_read(v, SCRATCH_FP1);
                let xr = self.gp_read(x, SCRATCH_GP1);
                let d = self.fp_write_reg(r);
                self.asm.movaps_rr(d, vr);
                self.asm.pinsrq(d, xr, lane);
                self.fp_commit(r, d);
            }
            Op::Call { func, args, rets } => self.emit_call_abi(args, rets, None, Some(func))?,
            Op::CallIndirect {
                ci_id,
                idx,
                args,
                rets,
            } => self.emit_call_abi(args, rets, Some((ci_id, idx)), None)?,
            Op::Trap(code) => {
                let lbl = match code {
                    TRAP_UNREACHABLE => self.trap_unreach_lbl(),
                    TRAP_OOB => self.trap_oob_lbl(),
                    TRAP_TABLE_OOB => self.trap_table_oob_lbl(),
                    _ => {
                        self.asm.mov_ri32(SCRATCH_GP1, u32::from(code));
                        let ep = self.epilogue_trap_lbl();
                        self.asm.jmp(ep);
                        return Ok(());
                    }
                };
                self.asm.jmp(lbl);
            }
        }
        Ok(())
    }

    fn emit_term(
        &mut self,
        term: Term,
        next_bb: Option<u32>,
        budget: Option<&CompileBudget>,
        charged_rodata: &mut usize,
    ) -> Result<(), String> {
        match term {
            Term::Jump { target, args } => {
                self.emit_edge_moves(target.0, args);
                if next_bb != Some(target.0) {
                    self.asm.jmp(self.bb_labels[target.0 as usize]);
                }
            }
            Term::BranchIf {
                cond,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => {
                let cr = self.gp_read(cond, SCRATCH_GP1);
                if next_bb == Some(then_bb.0) {
                    let else_lbl = self.edge_target_label(else_bb.0, else_args);
                    self.asm.test_rr(false, cr, cr);
                    self.asm.jcc(Cond::E, else_lbl);
                    self.emit_edge_moves(then_bb.0, then_args);
                } else {
                    let then_lbl = self.edge_target_label(then_bb.0, then_args);
                    self.asm.test_rr(false, cr, cr);
                    self.asm.jcc(Cond::Ne, then_lbl);
                    self.emit_edge_moves(else_bb.0, else_args);
                    if next_bb != Some(else_bb.0) {
                        self.asm.jmp(self.bb_labels[else_bb.0 as usize]);
                    }
                }
            }
            Term::CmpBranch {
                kind,
                a,
                b,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => {
                // Fast fallthrough when `then_bb` is next (integer cmp or self-NaN FP cmp):
                // invert to `else_lbl`, emit any `then_args` inline, and fall straight into `then_bb`.
                let fp_self_cond = match kind {
                    CmpBranchKind::F32(CmpOp::Eq) | CmpBranchKind::F64(CmpOp::Eq) if a == b => {
                        Some(Cond::Np)
                    }
                    CmpBranchKind::F32(CmpOp::Ne) | CmpBranchKind::F64(CmpOp::Ne) if a == b => {
                        Some(Cond::P)
                    }
                    _ => None,
                };
                let invert_to_else = next_bb == Some(then_bb.0)
                    && (fp_self_cond.is_some()
                        || matches!(
                            kind,
                            CmpBranchKind::I32(_)
                                | CmpBranchKind::I32Imm(_, _)
                                | CmpBranchKind::I64(_)
                                | CmpBranchKind::I64Imm(_, _)
                        ));
                if invert_to_else {
                    let else_lbl = self.edge_target_label(else_bb.0, else_args);
                    match kind {
                        CmpBranchKind::I32(cop) => {
                            let ar = self.gp_read(a, SCRATCH_GP1);
                            if let Loc::Spill(boff) = self.loc(b) {
                                self.asm.cmp_rm(false, ar, Mem::base_disp(Gp::Rsp, boff));
                            } else {
                                let br = self.gp_read(b, SCRATCH_GP2);
                                self.asm.cmp_rr(false, ar, br);
                            }
                            self.asm.jcc(cmp_to_cond(cop).invert(), else_lbl);
                        }
                        CmpBranchKind::I32Imm(cop, imm) => {
                            let ar = self.gp_read(a, SCRATCH_GP1);
                            if imm == 0 {
                                self.asm.test_rr(false, ar, ar);
                            } else {
                                self.asm.cmp_ri(false, ar, imm);
                            }
                            self.asm.jcc(cmp_to_cond(cop).invert(), else_lbl);
                        }
                        CmpBranchKind::I64(cop) => {
                            let ar = self.gp_read(a, SCRATCH_GP1);
                            if let Loc::Spill(boff) = self.loc(b) {
                                self.asm.cmp_rm(true, ar, Mem::base_disp(Gp::Rsp, boff));
                            } else {
                                let br = self.gp_read(b, SCRATCH_GP2);
                                self.asm.cmp_rr(true, ar, br);
                            }
                            self.asm.jcc(cmp_to_cond(cop).invert(), else_lbl);
                        }
                        CmpBranchKind::I64Imm(cop, imm) => {
                            let ar = self.gp_read(a, SCRATCH_GP1);
                            if imm == 0 {
                                self.asm.test_rr(true, ar, ar);
                            } else {
                                self.asm.cmp_ri(true, ar, imm);
                            }
                            self.asm.jcc(cmp_to_cond(cop).invert(), else_lbl);
                        }
                        CmpBranchKind::F32(_) | CmpBranchKind::F64(_) => {
                            let is64 = matches!(kind, CmpBranchKind::F64(_));
                            let ar = self.fp_read(a, SCRATCH_FP1);
                            if is64 {
                                self.asm.ucomisd(ar, ar);
                            } else {
                                self.asm.ucomiss(ar, ar);
                            }
                            self.asm.jcc(fp_self_cond.unwrap().invert(), else_lbl);
                        }
                    }
                    self.emit_edge_moves(then_bb.0, then_args);
                    return Ok(());
                }
                let then_lbl = self.edge_target_label(then_bb.0, then_args);
                match kind {
                    CmpBranchKind::I32(cop) => {
                        let ar = self.gp_read(a, SCRATCH_GP1);
                        if let Loc::Spill(boff) = self.loc(b) {
                            self.asm.cmp_rm(false, ar, Mem::base_disp(Gp::Rsp, boff));
                        } else {
                            let br = self.gp_read(b, SCRATCH_GP2);
                            self.asm.cmp_rr(false, ar, br);
                        }
                        self.asm.jcc(cmp_to_cond(cop), then_lbl);
                    }
                    CmpBranchKind::I32Imm(cop, imm) => {
                        let ar = self.gp_read(a, SCRATCH_GP1);
                        if imm == 0 {
                            self.asm.test_rr(false, ar, ar);
                        } else {
                            self.asm.cmp_ri(false, ar, imm);
                        }
                        self.asm.jcc(cmp_to_cond(cop), then_lbl);
                    }
                    CmpBranchKind::I64(cop) => {
                        let ar = self.gp_read(a, SCRATCH_GP1);
                        if let Loc::Spill(boff) = self.loc(b) {
                            self.asm.cmp_rm(true, ar, Mem::base_disp(Gp::Rsp, boff));
                        } else {
                            let br = self.gp_read(b, SCRATCH_GP2);
                            self.asm.cmp_rr(true, ar, br);
                        }
                        self.asm.jcc(cmp_to_cond(cop), then_lbl);
                    }
                    CmpBranchKind::I64Imm(cop, imm) => {
                        let ar = self.gp_read(a, SCRATCH_GP1);
                        if imm == 0 {
                            self.asm.test_rr(true, ar, ar);
                        } else {
                            self.asm.cmp_ri(true, ar, imm);
                        }
                        self.asm.jcc(cmp_to_cond(cop), then_lbl);
                    }
                    CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::F64(_));
                        match cop {
                            CmpOp::Eq if a == b => {
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                if is64 {
                                    self.asm.ucomisd(ar, ar);
                                } else {
                                    self.asm.ucomiss(ar, ar);
                                }
                                self.asm.jcc(Cond::Np, then_lbl);
                            }
                            CmpOp::Ne if a == b => {
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                if is64 {
                                    self.asm.ucomisd(ar, ar);
                                } else {
                                    self.asm.ucomiss(ar, ar);
                                }
                                self.asm.jcc(Cond::P, then_lbl);
                            }
                            CmpOp::Eq => {
                                let fall = self.asm.new_label();
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, ar, b);
                                self.asm.jcc(Cond::P, fall);
                                self.asm.jcc(Cond::E, then_lbl);
                                self.asm.bind(fall);
                            }
                            CmpOp::Ne => {
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, ar, b);
                                self.asm.jcc(Cond::P, then_lbl);
                                self.asm.jcc(Cond::Ne, then_lbl);
                            }
                            CmpOp::GtS => {
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, ar, b);
                                self.asm.jcc(Cond::A, then_lbl);
                            }
                            CmpOp::GeS => {
                                let ar = self.fp_read(a, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, ar, b);
                                self.asm.jcc(Cond::Ae, then_lbl);
                            }
                            CmpOp::LtS => {
                                let br = self.fp_read(b, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, br, a);
                                self.asm.jcc(Cond::A, then_lbl);
                            }
                            CmpOp::LeS => {
                                let br = self.fp_read(b, SCRATCH_FP1);
                                self.emit_ucomi_rhs(is64, br, a);
                                self.asm.jcc(Cond::Ae, then_lbl);
                            }
                            _ => unreachable!(),
                        }
                    }
                }
                self.emit_edge_moves(else_bb.0, else_args);
                if next_bb != Some(else_bb.0) {
                    self.asm.jmp(self.bb_labels[else_bb.0 as usize]);
                }
            }
            Term::BrTable { idx, table_idx } => {
                let bt = &self.ir.br_tables[table_idx as usize];
                let n_targets = bt.targets.len();
                let default = bt.default;
                if n_targets == 0 {
                    self.emit_edge_moves(default.0.0, default.1);
                    if next_bb != Some(default.0.0) {
                        self.asm.jmp(self.bb_labels[default.0.0 as usize]);
                    }
                    return Ok(());
                }
                // Pre-reserve jump-table rodata against BOTH per-function and cross-worker caps
                // BEFORE cloning targets or allocating jump-table labels/bytes.
                let est_rodata = n_targets.saturating_mul(4).saturating_add(4);
                let projected_rodata = self.asm.local_rodata.len().saturating_add(est_rodata);
                if projected_rodata > MAX_FN_LOCAL_RODATA_BYTES {
                    return Err(format!(
                        "x64: function {} local_rodata ({projected_rodata} B) exceeds cap {MAX_FN_LOCAL_RODATA_BYTES} B",
                        self.ir.func_index
                    ));
                }
                if let Some(b) = budget {
                    let dr = projected_rodata.saturating_sub(*charged_rodata);
                    if dr > 0 {
                        b.charge_rodata(dr)?;
                        *charged_rodata = projected_rodata;
                    }
                }
                let targets = bt.targets.clone();
                let ir = self.gp_read(idx, SCRATCH_GP2);
                if ir != SCRATCH_GP2 {
                    self.asm.mov_rr(false, SCRATCH_GP2, ir);
                }
                let def_lbl = self.edge_target_label(default.0.0, default.1);
                let mut tbl_lbls = Vec::with_capacity(targets.len());
                for (bb, args) in targets {
                    tbl_lbls.push(self.edge_target_label(bb.0, args));
                }
                self.asm.cmp_ri(false, SCRATCH_GP2, tbl_lbls.len() as i32);
                self.asm.jcc(Cond::Ae, def_lbl);
                let jt_off = self.asm.add_jump_table(&tbl_lbls);
                self.asm
                    .lea_rip(SCRATCH_GP1, RelocTarget::LocalRodata(jt_off));
                self.asm
                    .movsxd_rm(SCRATCH_GP2, Mem::indexed(SCRATCH_GP1, SCRATCH_GP2, 4, 0));
                self.asm.add_rr(true, SCRATCH_GP1, SCRATCH_GP2);
                self.asm.jmp_reg(SCRATCH_GP1);
            }
            Term::Return { vals } => {
                let ret_vals = self.ir.pool_slice(vals);
                if !ret_vals.is_empty() {
                    let ret_tys: Vec<Ty> = ret_vals.iter().map(|&v| self.ty(v)).collect();
                    let ret_layout = layout_rets(&ret_tys)?;
                    // First store any overflow return values into the caller-owned return buffer
                    // BEFORE `emit_parallel_moves` modifies return registers!
                    if ret_layout.stack_bytes > 0 {
                        let ptr_slot = self
                            .alloc
                            .saved_ret_ptr_off
                            .expect("saved_ret_ptr_off missing");
                        self.asm
                            .mov_rm(true, SCRATCH_GP1, Mem::base_disp(Gp::Rsp, ptr_slot));
                        for ((&rv, &ty), &slot) in
                            ret_vals.iter().zip(&ret_tys).zip(&ret_layout.slots)
                        {
                            if let AbiSlot::Stack(buf_off) = slot {
                                let mem = Mem::base_disp(SCRATCH_GP1, buf_off);
                                match self.loc(rv) {
                                    Loc::Remat(RematConst::I32(c)) => {
                                        self.asm.mov_mi32(false, mem, c)
                                    }
                                    Loc::Remat(RematConst::I64(c)) if i32::try_from(c).is_ok() => {
                                        self.asm.mov_mi32(true, mem, c as i32);
                                    }
                                    Loc::Remat(RematConst::F32(bits)) => {
                                        self.asm.mov_mi32(false, mem, bits as i32);
                                    }
                                    Loc::Remat(RematConst::F64(bits))
                                        if i32::try_from(bits as i64).is_ok() =>
                                    {
                                        self.asm.mov_mi32(true, mem, bits as i64 as i32);
                                    }
                                    _ if ty.is_fp() => {
                                        let xr = self.fp_read(rv, SCRATCH_FP2);
                                        emit_fp_store(&mut self.asm, mem, xr, ty);
                                    }
                                    _ => {
                                        let gr = self.gp_read(rv, SCRATCH_GP2);
                                        self.asm.mov_mr(ty == Ty::I64, mem, gr);
                                    }
                                }
                            }
                        }
                    }
                    let mut moves = Vec::with_capacity(ret_vals.len());
                    for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(ret_layout.slots) {
                        match slot {
                            AbiSlot::Gp(r) => {
                                moves.push((MoveLoc::Gp(r), MoveLoc::from(self.loc(rv)), ty))
                            }
                            AbiSlot::Xmm(x) => {
                                moves.push((MoveLoc::Xmm(x), MoveLoc::from(self.loc(rv)), ty))
                            }
                            AbiSlot::Stack(_) => {}
                        }
                    }
                    emit_parallel_moves(&mut self.asm, &moves);
                }
                self.asm.xor_rr(false, SCRATCH_GP1, SCRATCH_GP1);
                if self.alloc.frame_size == 0
                    && self.alloc.used_callee_gp.is_empty()
                    && self.alloc.used_callee_xmm.is_empty()
                {
                    self.asm.ret();
                } else {
                    let ep = *self.epilogue_ok.get_or_insert_with(|| self.asm.new_label());
                    // `epilogue_ok` is emitted immediately after the last basic block, so `next_bb.is_none()`
                    // falls straight through without a jump!
                    if next_bb.is_some() {
                        self.asm.jmp(ep);
                    }
                }
            }
            Term::Unreachable => {
                let lbl = self.trap_unreach_lbl();
                self.asm.jmp(lbl);
            }
        }
        Ok(())
    }
}

/// Lowers `ir` + `alloc` into position-independent x86-64 machine code (`Asm`).
#[allow(dead_code)]
pub fn lower_ir_to_x64(
    ir: &IrFunc,
    alloc: &AllocResult,
    layout: &CtxLayout,
    ro: &RodataConsts,
    isa: TargetIsa,
) -> Result<Asm, String> {
    lower_ir_to_x64_with_budget(ir, alloc, layout, ro, isa, None, None)
}

/// Lowers `ir` + `alloc` into position-independent x86-64 machine code (`Asm`) while enforcing
/// per-function caps, aggregate cross-worker code/rodata budgets, and cooperative deadlines.
pub fn lower_ir_to_x64_with_budget(
    ir: &IrFunc,
    alloc: &AllocResult,
    layout: &CtxLayout,
    ro: &RodataConsts,
    isa: TargetIsa,
    fn_can_trap: Option<&[bool]>,
    budget: Option<&CompileBudget>,
) -> Result<Asm, String> {
    if ir.blocks.len() > MAX_BLOCKS_PER_FN {
        return Err(format!(
            "x64: function {} block count ({}) exceeds cap {MAX_BLOCKS_PER_FN}",
            ir.func_index,
            ir.blocks.len()
        ));
    }
    if let Some(b) = budget {
        b.check_deadline()?;
    }
    let init_cap = ir
        .blocks
        .len()
        .saturating_mul(64)
        .saturating_add(128)
        .min(MAX_FN_CODE_BYTES);
    let mut asm = Asm::with_capacity(init_cap);
    let bb_labels: Vec<Label> = (0..ir.blocks.len()).map(|_| asm.new_label()).collect();

    let mut l = Lower {
        ir,
        alloc,
        layout,
        ro,
        isa,
        fn_can_trap,
        asm,
        bb_labels,
        trap_oob: None,
        trap_table_oob: None,
        trap_ci: None,
        trap_div0: None,
        trap_int_ovf: None,
        trap_unreach: None,
        trap_prop: None,
        epilogue_ok: None,
        epilogue_trap: None,
        edge_tramps: Vec::new(),
        edge_tramp_cache: HashMap::new(),
    };

    // ---- Wrap-Proof Prospective Stack Check BEFORE Any Push or Frame Allocation ----
    // Computes `r11 = ctx->stack_floor + total_push_and_frame` (cannot wrap: user-space addr < 2^47,
    // total <= MAX_FRAME_BYTES + ELIDE_MAX_SUBTREE_STACK_BYTES + 48), then `cmp rsp, r11; jb early_stack_trap`.
    let early_stack_trap = if ir.needs_stack_check {
        let lbl = l.asm.new_label();
        let push_bytes =
            i32::try_from(alloc.used_callee_gp.len() * 8).map_err(|_| "callee save overflow")?;
        let total_push_and_frame = alloc
            .frame_size
            .checked_add(push_bytes)
            .and_then(|v| v.checked_add(ir.elided_callee_stack_bytes))
            .ok_or("x64: stack check frame size overflow")?;
        l.asm
            .mov_rm(true, SCRATCH_GP1, Mem::base_disp(Gp::R15, OFF_STACK_FLOOR));
        if total_push_and_frame > 0 {
            l.asm.add_ri(true, SCRATCH_GP1, total_push_and_frame);
        }
        l.asm.cmp_rr(true, Gp::Rsp, SCRATCH_GP1);
        l.asm.jcc(Cond::B, lbl);
        Some(lbl)
    } else {
        None
    };

    // ---- Prologue ----
    for &g in &alloc.used_callee_gp {
        l.asm.push(g);
    }
    if alloc.frame_size > 0 {
        l.asm.sub_ri(true, Gp::Rsp, alloc.frame_size);
    }
    for (i, &x) in alloc.used_callee_xmm.iter().enumerate() {
        l.asm.movdqu_mx(
            Mem::base_disp(Gp::Rsp, alloc.saved_xmm_base + (i as i32) * 16),
            x,
        );
    }
    if let Some(ptr_slot) = alloc.saved_ret_ptr_off {
        l.asm.mov_rm(
            true,
            SCRATCH_GP1,
            Mem::base_disp(Gp::R15, OFF_RET_SPILL_PTR),
        );
        l.asm
            .mov_mr(true, Mem::base_disp(Gp::Rsp, ptr_slot), SCRATCH_GP1);
    }

    // Move incoming Wasm parameters using the unified typed ABI layout shared with `regalloc`.
    if !ir.params.is_empty() {
        let incoming_stack_base = alloc
            .frame_size
            .checked_add((alloc.used_callee_gp.len() as i32 + 1) * 8)
            .ok_or("x64: incoming stack base overflow")?;
        let param_tys: Vec<Ty> = ir
            .params
            .iter()
            .map(|&pv| ir.val_tys[pv.0 as usize])
            .collect();
        let arg_layout = layout_args(&param_tys)?;
        let mut moves = Vec::with_capacity(ir.params.len());
        for ((&pv, &ty), slot) in ir.params.iter().zip(&param_tys).zip(arg_layout.slots) {
            let src = match slot {
                AbiSlot::Gp(r) => MoveLoc::Gp(r),
                AbiSlot::Xmm(x) => MoveLoc::Xmm(x),
                AbiSlot::Stack(off) => {
                    let s = incoming_stack_base
                        .checked_add(off)
                        .ok_or("x64: incoming stack arg overflow")?;
                    MoveLoc::Stack(s)
                }
            };
            moves.push((MoveLoc::from(alloc.locs[pv.0 as usize]), src, ty));
        }
        emit_parallel_moves(&mut l.asm, &moves);
    }

    let mut charged_code = 0usize;
    let mut charged_rodata = 0usize;
    let check_limits = |asm: &Asm,
                        charged_code: &mut usize,
                        charged_rodata: &mut usize|
     -> Result<(), String> {
        if asm.code.len() > MAX_FN_CODE_BYTES {
            return Err(format!(
                "x64: function {} code ({} B) exceeds cap {MAX_FN_CODE_BYTES} B",
                ir.func_index,
                asm.code.len()
            ));
        }
        if asm.local_rodata.len() > MAX_FN_LOCAL_RODATA_BYTES {
            return Err(format!(
                "x64: function {} local_rodata ({} B) exceeds cap {MAX_FN_LOCAL_RODATA_BYTES} B",
                ir.func_index,
                asm.local_rodata.len()
            ));
        }
        if let Some(b) = budget {
            let dc = asm.code.len().saturating_sub(*charged_code);
            if dc > 0 {
                b.charge_code(dc)?;
                *charged_code = asm.code.len();
            }
            let dr = asm.local_rodata.len().saturating_sub(*charged_rodata);
            if dr > 0 {
                b.charge_rodata(dr)?;
                *charged_rodata = asm.local_rodata.len();
            }
        }
        Ok(())
    };

    // ---- Basic Blocks ----
    for (oi, &bid) in alloc.block_order.iter().enumerate() {
        if let Some(b) = budget
            && (oi & 63) == 0
        {
            b.check_deadline()?;
        }
        l.asm.bind(l.bb_labels[bid as usize]);
        let bb = &ir.blocks[bid as usize];
        for (ii, inst) in bb.insts.iter().enumerate() {
            l.emit_inst(inst.res, inst.op)?;
            if (ii & 255) == 255 {
                if let Some(b) = budget {
                    b.check_deadline()?;
                }
                check_limits(&l.asm, &mut charged_code, &mut charged_rodata)?;
            } else if l.asm.code.len() > MAX_FN_CODE_BYTES {
                return Err(format!(
                    "x64: function {} code ({} B) exceeds cap {MAX_FN_CODE_BYTES} B",
                    ir.func_index,
                    l.asm.code.len()
                ));
            }
        }
        let next_bb = alloc.block_order.get(oi + 1).copied();
        l.emit_term(bb.term, next_bb, budget, &mut charged_rodata)?;
        check_limits(&l.asm, &mut charged_code, &mut charged_rodata)?;
    }

    // ---- Shared Normal Epilogue (placed immediately after basic blocks for zero-jump fallthrough) ----
    if let Some(ep) = l.epilogue_ok {
        l.asm.bind(ep);
        l.emit_epilogue_body();
    }

    // ---- Edge-copy trampolines ----
    let tramps = std::mem::take(&mut l.edge_tramps);
    for (ti, (lbl, target_bb, args)) in tramps.into_iter().enumerate() {
        if let Some(b) = budget
            && (ti & 63) == 0
        {
            b.check_deadline()?;
        }
        l.asm.bind(lbl);
        l.emit_edge_moves(target_bb, args);
        l.asm.jmp(l.bb_labels[target_bb as usize]);
        check_limits(&l.asm, &mut charged_code, &mut charged_rodata)?;
    }

    // ---- Shared Cold Trap Stubs ----
    let has_any_trap = l.trap_oob.is_some()
        || l.trap_table_oob.is_some()
        || l.trap_ci.is_some()
        || l.trap_div0.is_some()
        || l.trap_int_ovf.is_some()
        || l.trap_unreach.is_some()
        || l.trap_prop.is_some()
        || l.epilogue_trap.is_some();

    if has_any_trap {
        let ep_trap = l.epilogue_trap_lbl();
        let stubs: [(Option<Label>, u8); 6] = [
            (l.trap_oob, TRAP_OOB),
            (l.trap_table_oob, TRAP_TABLE_OOB),
            (l.trap_ci, TRAP_INDIRECT_CALL),
            (l.trap_div0, TRAP_DIV_ZERO),
            (l.trap_int_ovf, TRAP_INT_OVERFLOW),
            (l.trap_unreach, TRAP_UNREACHABLE),
        ];
        let active_stubs: Vec<(Label, u8)> = stubs
            .into_iter()
            .filter_map(|(o, c)| o.map(|lbl| (lbl, c)))
            .collect();
        for (idx, (lbl, code)) in active_stubs.iter().copied().enumerate() {
            l.asm.bind(lbl);
            l.asm.mov_ri32(SCRATCH_GP1, u32::from(code));
            if idx + 1 < active_stubs.len() {
                l.asm.jmp(ep_trap);
            }
        }
        if let Some(prop) = l.trap_prop {
            l.asm.bind(prop);
        }
        l.asm.bind(ep_trap);
        l.emit_epilogue_body();
    }

    // Early pre-prologue stack trap returns directly without popping callee-saved regs.
    if let Some(early) = early_stack_trap {
        l.asm.bind(early);
        l.asm.mov_ri32(SCRATCH_GP1, u32::from(TRAP_CALL_STACK));
        l.asm.ret();
    }

    check_limits(&l.asm, &mut charged_code, &mut charged_rodata)?;
    l.asm.finish_labels()?;
    Ok(l.asm)
}
