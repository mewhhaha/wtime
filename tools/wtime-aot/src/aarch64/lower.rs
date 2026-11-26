//! SSA IR (`x64::ir::IrFunc`) -> AArch64 machine-code lowering (`#![forbid(unsafe_code)]`).
//!
//! Register conventions:
//! - Pinned: `X28 = Ctx*`, `X27 = mem_base`, `X26 = mem_len`, `X25 = pinned_global` (optional).
//! - Scratch (never allocated):
//!   - `X16`: primary scratch & trap status return (`W16`: `0` = ok, `1..=22` = trap)
//!   - `X17`: secondary scratch & parallel-move cycle break
//!   - `X24`: tertiary scratch (mem-to-mem move temp & wide address/offset materialization)
//!   - `V30`, `V31`: FP/V128 scratch registers

use std::collections::HashMap;

use super::asm::{A64Asm, A64RelocTarget, Cond, Label, Reg, VReg};
use super::regalloc::{
    A64AbiSlot, A64AllocResult, A64Loc, A64RematConst, layout_args, layout_rets,
};
use super::simd_lower::{
    emit_a64_v128_bin, emit_a64_v128_shift, emit_a64_v128_shuffle, emit_a64_v128_to_i32,
    emit_a64_v128_un,
};
use crate::x64::ir::{
    CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, IntBinOp, IrFunc, LoadKind, Op, SimdLaneKind,
    SimdSplatKind, StoreKind, Term, TruncOp, Ty, Val, ValRange,
};
use crate::x64::{
    CtxLayout, HELPER_EXTRA_MEM_COPY, HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW,
    HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, HELPER_MEM_INIT, HELPER_TABLE_COPY,
    HELPER_TABLE_FILL, HELPER_TABLE_GROW, HELPER_TABLE_INIT, OFF_RET_SPILL_PTR, OFF_SCRATCH_TMP,
    OFF_SCRATCH_V128, OFF_STACK_FLOOR, TRAP_CALL_STACK, TRAP_DIV_ZERO, TRAP_INDIRECT_CALL,
    TRAP_INT_OVERFLOW, TRAP_INVALID_CONV, TRAP_OOB, TRAP_TABLE_OOB, TRAP_UNREACHABLE,
};

pub const CTX_REG: Reg = Reg::X28;
pub const MEM_BASE_REG: Reg = Reg::X27;
pub const MEM_LEN_REG: Reg = Reg::X26;
pub const PINNED_GLOB_REG: Reg = Reg::X25;

pub const SGP1: Reg = Reg::X16;
pub const SGP2: Reg = Reg::X17;
pub const SGP3: Reg = Reg::X24;
pub const SFP1: VReg = VReg::V30;
pub const SFP2: VReg = VReg::V31;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MoveLoc {
    Gp(Reg),
    Fp(VReg),
    Stack(i32),
    Remat(A64RematConst),
}

impl From<A64Loc> for MoveLoc {
    fn from(l: A64Loc) -> Self {
        match l {
            A64Loc::Gp(r) => MoveLoc::Gp(r),
            A64Loc::Fp(v) => MoveLoc::Fp(v),
            A64Loc::Spill(off) => MoveLoc::Stack(off),
            A64Loc::Remat(rc) => MoveLoc::Remat(rc),
        }
    }
}

/// Emits a load from `[base + off]` supporting arbitrary `off >= 0` using `addr_tmp` if needed.
pub fn emit_load_off(
    asm: &mut A64Asm,
    ty: Ty,
    dst_gp: Option<Reg>,
    dst_fp: Option<VReg>,
    base: Reg,
    off: i32,
    addr_tmp: Reg,
) {
    assert!(off >= 0);
    let uoff = off as u32;
    let (align_mask, max_uimm) = match ty {
        Ty::I32 | Ty::F32 => (3u32, 4095 * 4),
        Ty::I64 | Ty::F64 => (7u32, 4095 * 8),
        Ty::V128 => (15u32, 4095 * 16),
    };
    let (eff_base, eff_off) = if (uoff & align_mask) == 0 && uoff <= max_uimm {
        (base, uoff)
    } else {
        asm.add_i32_checked(true, addr_tmp, base, off, addr_tmp);
        (addr_tmp, 0)
    };
    match ty {
        Ty::I32 => asm.ldr_uimm(false, dst_gp.unwrap(), eff_base, eff_off),
        Ty::I64 => asm.ldr_uimm(true, dst_gp.unwrap(), eff_base, eff_off),
        Ty::F32 => asm.ldr_s_uimm(dst_fp.unwrap(), eff_base, eff_off),
        Ty::F64 => asm.ldr_d_uimm(dst_fp.unwrap(), eff_base, eff_off),
        Ty::V128 => asm.ldr_q_uimm(dst_fp.unwrap(), eff_base, eff_off),
    }
}

/// Emits a store to `[base + off]` supporting arbitrary `off >= 0` using `addr_tmp` if needed.
pub fn emit_store_off(
    asm: &mut A64Asm,
    ty: Ty,
    src_gp: Option<Reg>,
    src_fp: Option<VReg>,
    base: Reg,
    off: i32,
    addr_tmp: Reg,
) {
    assert!(off >= 0);
    let uoff = off as u32;
    let (align_mask, max_uimm) = match ty {
        Ty::I32 | Ty::F32 => (3u32, 4095 * 4),
        Ty::I64 | Ty::F64 => (7u32, 4095 * 8),
        Ty::V128 => (15u32, 4095 * 16),
    };
    let (eff_base, eff_off) = if (uoff & align_mask) == 0 && uoff <= max_uimm {
        (base, uoff)
    } else {
        asm.add_i32_checked(true, addr_tmp, base, off, addr_tmp);
        (addr_tmp, 0)
    };
    match ty {
        Ty::I32 => asm.str_uimm(false, src_gp.unwrap(), eff_base, eff_off),
        Ty::I64 => asm.str_uimm(true, src_gp.unwrap(), eff_base, eff_off),
        Ty::F32 => asm.str_s_uimm(src_fp.unwrap(), eff_base, eff_off),
        Ty::F64 => asm.str_d_uimm(src_fp.unwrap(), eff_base, eff_off),
        Ty::V128 => asm.str_q_uimm(src_fp.unwrap(), eff_base, eff_off),
    }
}

fn emit_single_move(asm: &mut A64Asm, dst: MoveLoc, src: MoveLoc, ty: Ty) {
    if dst == src {
        return;
    }
    let is64 = ty == Ty::I64;
    match (dst, src) {
        (MoveLoc::Gp(d), MoveLoc::Gp(s)) => asm.mov_r(is64, d, s),
        (MoveLoc::Gp(d), MoveLoc::Stack(off)) => {
            emit_load_off(asm, ty, Some(d), None, Reg::SP, off, SGP3);
        }
        (MoveLoc::Stack(off), MoveLoc::Gp(s)) => {
            emit_store_off(asm, ty, Some(s), None, Reg::SP, off, SGP3);
        }
        (MoveLoc::Fp(d), MoveLoc::Fp(s)) => {
            if ty == Ty::V128 {
                asm.mov_16b(d, s);
            } else {
                asm.fmov_fp(ty == Ty::F64, d, s);
            }
        }
        (MoveLoc::Fp(d), MoveLoc::Stack(off)) => {
            emit_load_off(asm, ty, None, Some(d), Reg::SP, off, SGP3);
        }
        (MoveLoc::Stack(off), MoveLoc::Fp(s)) => {
            emit_store_off(asm, ty, None, Some(s), Reg::SP, off, SGP3);
        }
        (MoveLoc::Gp(d), MoveLoc::Remat(rc)) => match rc {
            A64RematConst::I32(c) => asm.mov_imm32(d, c as u32),
            A64RematConst::I64(c) => asm.mov_imm64(d, c as u64),
        },
        (MoveLoc::Stack(off), MoveLoc::Remat(rc)) => {
            match rc {
                A64RematConst::I32(c) => asm.mov_imm32(SGP1, c as u32),
                A64RematConst::I64(c) => asm.mov_imm64(SGP1, c as u64),
            }
            emit_store_off(asm, ty, Some(SGP1), None, Reg::SP, off, SGP3);
        }
        (MoveLoc::Stack(doff), MoveLoc::Stack(soff)) => {
            if ty.is_fp() {
                emit_load_off(asm, ty, None, Some(SFP2), Reg::SP, soff, SGP3);
                emit_store_off(asm, ty, None, Some(SFP2), Reg::SP, doff, SGP3);
            } else {
                emit_load_off(asm, ty, Some(SGP1), None, Reg::SP, soff, SGP3);
                emit_store_off(asm, ty, Some(SGP1), None, Reg::SP, doff, SGP3);
            }
        }
        _ => unreachable!("aarch64 mismatched move: {dst:?} <- {src:?} ({ty:?})"),
    }
}

fn emit_parallel_moves(asm: &mut A64Asm, raw_moves: &[(MoveLoc, MoveLoc, Ty)]) {
    let mut moves: Vec<(MoveLoc, MoveLoc, Ty)> = raw_moves
        .iter()
        .copied()
        .filter(|(d, s, _)| d != s)
        .collect();
    let n = moves.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        emit_single_move(asm, moves[0].0, moves[0].1, moves[0].2);
        return;
    }
    let mut active = vec![true; n];
    let mut remaining = n;
    while remaining > 0 {
        let mut progressed = true;
        while progressed {
            progressed = false;
            for i in 0..n {
                if !active[i] {
                    continue;
                }
                let dst = moves[i].0;
                if (0..n).any(|j| active[j] && moves[j].1 == dst) {
                    continue;
                }
                active[i] = false;
                remaining -= 1;
                emit_single_move(asm, moves[i].0, moves[i].1, moves[i].2);
                progressed = true;
            }
        }
        if remaining == 0 {
            break;
        }
        let k = (0..n).find(|&i| active[i]).unwrap();
        let (_, src0, ty0) = moves[k];
        let tmp = if ty0.is_fp() {
            MoveLoc::Fp(SFP1)
        } else {
            MoveLoc::Gp(SGP2)
        };
        emit_single_move(asm, tmp, src0, ty0);
        moves[k].1 = tmp;
    }
}

fn cmp_to_cond(op: CmpOp) -> Cond {
    match op {
        CmpOp::Eq => Cond::Eq,
        CmpOp::Ne => Cond::Ne,
        CmpOp::LtS => Cond::Lt,
        CmpOp::LtU => Cond::Lo,
        CmpOp::GtS => Cond::Gt,
        CmpOp::GtU => Cond::Hi,
        CmpOp::LeS => Cond::Le,
        CmpOp::LeU => Cond::Ls,
        CmpOp::GeS => Cond::Ge,
        CmpOp::GeU => Cond::Hs,
    }
}

/// Intra-block live integer NZCV flag state preserved across flag-clean AArch64 instructions
/// (`mov`, `csel`, `fcsel`, `ldr`, `str`, `add`, `sub`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum A64LiveFlags {
    IntRR { is64: bool, a: Val, b: Val },
    IntRI { is64: bool, a: Val, imm: i32 },
}

struct LowerA64<'a> {
    ir: &'a IrFunc,
    alloc: &'a A64AllocResult,
    layout: &'a CtxLayout,
    asm: A64Asm,
    bb_labels: Vec<Label>,
    trap_oob: Option<Label>,
    trap_table_oob: Option<Label>,
    trap_ci: Option<Label>,
    trap_div0: Option<Label>,
    trap_int_ovf: Option<Label>,
    trap_inv_conv: Option<Label>,
    trap_unreach: Option<Label>,
    trap_stack: Option<Label>,
    trap_prop: Option<Label>,
    epilogue_ok: Option<Label>,
    epilogue_trap: Option<Label>,
    edge_tramps: Vec<(Label, u32, ValRange)>,
    edge_tramp_cache: HashMap<(u32, u32, u32), Label>,
    /// Lazily populated map of SSA `Val`s whose AArch64 defining write guarantees bits `32..63 == 0`,
    /// allowing redundant `uxtw` on `I32WrapI64`/`I64ExtendI32U` to be elided.
    val_upper32_zero: Option<Vec<bool>>,
    /// Intra-block live integer NZCV flags for redundant `cmp` elision across adjacent
    /// `Op::CmpSelect` / `Term::CmpBranch` instructions.
    live_flags: Option<A64LiveFlags>,
}

impl<'a> LowerA64<'a> {
    fn is_upper32_zero(&mut self, v: Val) -> bool {
        let table = self.val_upper32_zero.get_or_insert_with(|| {
            let mut flags = vec![false; self.ir.val_tys.len()];
            for bb in &self.ir.blocks {
                for inst in &bb.insts {
                    let Some(res) = inst.res else {
                        continue;
                    };
                    let z = match inst.op {
                        Op::ConstI32(_)
                        | Op::I32Eqz(_)
                        | Op::I64Eqz(_)
                        | Op::I32Clz(_)
                        | Op::I32Ctz(_)
                        | Op::I32Popcnt(_)
                        | Op::I64Clz(_)
                        | Op::I64Ctz(_)
                        | Op::I64Popcnt(_)
                        | Op::I32WrapI64(_)
                        | Op::I64ExtendI32U(_)
                        | Op::I32Extend8S(_)
                        | Op::I32Extend16S(_)
                        | Op::I32Bin(..)
                        | Op::I32BinImm(..)
                        | Op::AddScaled { is64: false, .. }
                        | Op::I32Cmp(..)
                        | Op::I32CmpImm(..)
                        | Op::I64Cmp(..)
                        | Op::I64CmpImm(..)
                        | Op::F32Cmp(..)
                        | Op::F64Cmp(..)
                        | Op::I32ReinterpretF32(_)
                        | Op::MemByteSum { .. }
                        | Op::V128ToI32 { .. } => true,
                        Op::ConstI64(c) => (c as u64) <= u32::MAX as u64,
                        Op::I64BinImm(IntBinOp::ShrU, _, imm) => (imm & 63) >= 32,
                        Op::I64BinImm(IntBinOp::And, _, imm) => imm >= 0,
                        Op::Load { kind, .. } | Op::LoadScaled { kind, .. } => matches!(
                            kind,
                            LoadKind::I32
                                | LoadKind::I32Load8S
                                | LoadKind::I32Load8U
                                | LoadKind::I32Load16S
                                | LoadKind::I32Load16U
                                | LoadKind::I64Load8U
                                | LoadKind::I64Load16U
                                | LoadKind::I64Load32U
                        ),
                        Op::V128ExtractLane { kind, .. } => matches!(
                            kind,
                            SimdLaneKind::I8x16S
                                | SimdLaneKind::I8x16U
                                | SimdLaneKind::I16x8S
                                | SimdLaneKind::I16x8U
                                | SimdLaneKind::I32x4
                        ),
                        _ => false,
                    };
                    if z {
                        flags[res.0 as usize] = true;
                    }
                }
            }
            flags
        });
        table.get(v.0 as usize).copied().unwrap_or(false)
    }
    #[inline]
    fn loc(&self, v: Val) -> A64Loc {
        self.alloc.locs[v.0 as usize]
    }
    #[inline]
    fn ty(&self, v: Val) -> Ty {
        self.ir.val_tys[v.0 as usize]
    }

    fn gp_read(&mut self, v: Val, scratch: Reg) -> Reg {
        match self.loc(v) {
            A64Loc::Gp(r) => r,
            A64Loc::Spill(off) => {
                let ty = self.ty(v);
                emit_load_off(&mut self.asm, ty, Some(scratch), None, Reg::SP, off, SGP3);
                scratch
            }
            A64Loc::Remat(rc) => {
                match rc {
                    A64RematConst::I32(c) => self.asm.mov_imm32(scratch, c as u32),
                    A64RematConst::I64(c) => self.asm.mov_imm64(scratch, c as u64),
                }
                scratch
            }
            A64Loc::Fp(_) => unreachable!("aarch64: expected GP val"),
        }
    }

    fn gp_write_reg(&self, v: Val) -> Reg {
        match self.loc(v) {
            A64Loc::Gp(r) => r,
            A64Loc::Spill(_) => SGP1,
            A64Loc::Fp(_) | A64Loc::Remat(_) => unreachable!("aarch64: expected GP val"),
        }
    }

    fn gp_commit(&mut self, v: Val, r: Reg) {
        let ty = self.ty(v);
        match self.loc(v) {
            A64Loc::Gp(d) => {
                if d != r {
                    self.asm.mov_r(ty == Ty::I64, d, r);
                }
            }
            A64Loc::Spill(off) => {
                emit_store_off(&mut self.asm, ty, Some(r), None, Reg::SP, off, SGP3);
            }
            A64Loc::Fp(_) | A64Loc::Remat(_) => unreachable!(),
        }
    }

    fn fp_read(&mut self, v: Val, scratch: VReg) -> VReg {
        match self.loc(v) {
            A64Loc::Fp(r) => r,
            A64Loc::Spill(off) => {
                let ty = self.ty(v);
                emit_load_off(&mut self.asm, ty, None, Some(scratch), Reg::SP, off, SGP3);
                scratch
            }
            A64Loc::Gp(_) | A64Loc::Remat(_) => unreachable!("aarch64: expected FP val"),
        }
    }

    fn fp_write_reg(&self, v: Val) -> VReg {
        match self.loc(v) {
            A64Loc::Fp(r) => r,
            A64Loc::Spill(_) => SFP1,
            A64Loc::Gp(_) | A64Loc::Remat(_) => unreachable!(),
        }
    }

    fn fp_commit(&mut self, v: Val, r: VReg) {
        let ty = self.ty(v);
        match self.loc(v) {
            A64Loc::Fp(d) => {
                if d != r {
                    if ty == Ty::V128 {
                        self.asm.mov_16b(d, r);
                    } else {
                        self.asm.fmov_fp(ty == Ty::F64, d, r);
                    }
                }
            }
            A64Loc::Spill(off) => {
                emit_store_off(&mut self.asm, ty, None, Some(r), Reg::SP, off, SGP3);
            }
            A64Loc::Gp(_) | A64Loc::Remat(_) => unreachable!(),
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
    fn trap_inv_conv_lbl(&mut self) -> Label {
        *self
            .trap_inv_conv
            .get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_unreach_lbl(&mut self) -> Label {
        *self
            .trap_unreach
            .get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_stack_lbl(&mut self) -> Label {
        *self.trap_stack.get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_prop_lbl(&mut self) -> Label {
        *self.trap_prop.get_or_insert_with(|| self.asm.new_label())
    }
    fn epilogue_trap_lbl(&mut self) -> Label {
        *self
            .epilogue_trap
            .get_or_insert_with(|| self.asm.new_label())
    }

    fn emit_epilogue_teardown(&mut self) {
        let mut fi = 0usize;
        let fps = &self.alloc.used_callee_fp;
        while fi < fps.len() {
            let off = self.alloc.saved_fp_base + (fi as i32) * 16;
            if fi + 1 < fps.len() && (off & 15) == 0 && (-1024..=1008).contains(&off) {
                self.asm.ldp_q(fps[fi], fps[fi + 1], Reg::SP, off);
                fi += 2;
            } else {
                emit_load_off(
                    &mut self.asm,
                    Ty::V128,
                    None,
                    Some(fps[fi]),
                    Reg::SP,
                    off,
                    SGP3,
                );
                fi += 1;
            }
        }
        let mut gp_off = self.alloc.saved_gp_base;
        if self.alloc.has_calls {
            if (gp_off & 7) == 0 && (-512..=504).contains(&gp_off) {
                self.asm.ldp_x(Reg::FP, Reg::LR, Reg::SP, gp_off);
            } else {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::FP),
                    None,
                    Reg::SP,
                    gp_off,
                    SGP3,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::LR),
                    None,
                    Reg::SP,
                    gp_off + 8,
                    SGP3,
                );
            }
            gp_off += 16;
        }
        let gps = &self.alloc.used_callee_gp;
        let mut gi = 0usize;
        while gi < gps.len() {
            if gi + 1 < gps.len() && (gp_off & 7) == 0 && (-512..=504).contains(&gp_off) {
                self.asm.ldp_x(gps[gi], gps[gi + 1], Reg::SP, gp_off);
                gp_off += 16;
                gi += 2;
            } else {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(gps[gi]),
                    None,
                    Reg::SP,
                    gp_off,
                    SGP3,
                );
                gp_off += 8;
                gi += 1;
            }
        }
        if self.alloc.frame_size > 0 {
            self.asm
                .add_i32_checked(true, Reg::SP, Reg::SP, self.alloc.frame_size, SGP3);
        }
    }

    fn emit_epilogue_body(&mut self) {
        self.emit_epilogue_teardown();
        self.asm.ret();
    }

    fn emit_speculation_barrier(&mut self) {
        if self.layout.spectre_mitigations {
            self.asm.dsb_nsh();
            self.asm.isb();
        }
    }

    fn emit_bounds_check(&mut self, addr: Reg, end_off: u64) {
        let oob = self.trap_oob_lbl();
        if !self.layout.memory64 {
            // Zero-extend 32-bit guest address into 64-bit SGP2.
            self.asm.uxtw(SGP2, addr);
        } else {
            self.asm.mov_r(true, SGP2, addr);
        }
        if let Some(cap) = self.layout.fixed_mem_bytes {
            if end_off > cap {
                self.asm.b(oob);
                return;
            }
            let max_addr = cap - end_off;
            if max_addr < 4096 {
                self.asm.cmp_imm(true, SGP2, max_addr as u16);
            } else {
                self.asm.mov_imm64(SGP3, max_addr);
                self.asm.cmp_r(true, SGP2, SGP3);
            }
            self.asm.b_cond(Cond::Hi, oob);
            self.emit_speculation_barrier();
            return;
        }
        if end_off == 0 {
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            self.asm.b_cond(Cond::Hi, oob);
            self.emit_speculation_barrier();
            return;
        }
        if self.layout.memory64 {
            // Fast 1-branch wrap-proof check when `mem_min_bytes >= end_off` (so `MEM_LEN_REG - end_off`
            // cannot underflow, because linear memory never shrinks): `addr <=u (MEM_LEN_REG - end_off)`!
            if end_off <= self.layout.mem_min_bytes {
                if end_off < 4096 {
                    self.asm.sub_imm(true, SGP3, MEM_LEN_REG, end_off as u16);
                } else {
                    self.asm.mov_imm64(SGP3, end_off);
                    self.asm.sub_r(true, SGP3, MEM_LEN_REG, SGP3);
                }
                self.asm.cmp_r(true, SGP2, SGP3);
                self.asm.b_cond(Cond::Hi, oob);
                self.emit_speculation_barrier();
                return;
            }
            self.asm.mov_imm64(SGP3, end_off);
            self.asm.adds_r(true, SGP2, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, oob); // carry set => 64-bit unsigned overflow!
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            self.asm.b_cond(Cond::Hi, oob);
            self.emit_speculation_barrier();
            return;
        }
        if end_off < 4096 {
            self.asm.add_imm(true, SGP2, SGP2, end_off as u16);
        } else {
            self.asm.mov_imm64(SGP3, end_off);
            self.asm.add_r(true, SGP2, SGP2, SGP3);
        }
        self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
        self.asm.b_cond(Cond::Hi, oob);
        self.emit_speculation_barrier();
    }

    /// Computes effective host address `MEM_BASE_REG + zext(addr) + offset` into `SGP3`
    /// using extended-register `add xd, x27, wm, uxtw` (saves 1 instruction on every `memory32` access).
    fn compute_mem_ptr(&mut self, addr: Reg, offset: u64) -> Reg {
        if !self.layout.memory64 {
            self.asm.add_ext_uxtw(SGP3, MEM_BASE_REG, addr);
        } else {
            self.asm.add_r(true, SGP3, MEM_BASE_REG, addr);
        }
        if offset > 0 {
            if offset < 4096 {
                self.asm.add_imm(true, SGP3, SGP3, offset as u16);
            } else if offset < (1 << 24) {
                self.asm
                    .add_imm_lsl12(true, SGP3, SGP3, (offset >> 12) as u16);
                let lo = (offset & 0xfff) as u16;
                if lo != 0 {
                    self.asm.add_imm(true, SGP3, SGP3, lo);
                }
            } else {
                self.asm.mov_imm64(SGP2, offset);
                self.asm.add_r(true, SGP3, SGP3, SGP2);
            }
        }
        SGP3
    }

    /// Computes `(base_reg, uimm_off)` for an access of `access_bytes` (`1|2|4|8|16`) when `offset > 0`,
    /// folding naturally aligned small `offset` directly into the AArch64 `[base, #uimm]` load/store.
    fn compute_mem_base_uimm(&mut self, addr: Reg, offset: u64, access_bytes: u64) -> (Reg, u32) {
        let align_mask = access_bytes - 1;
        let max_uimm = 4095 * access_bytes;
        if offset > 0 && (offset & align_mask) == 0 && offset <= max_uimm {
            if !self.layout.memory64 {
                self.asm.add_ext_uxtw(SGP3, MEM_BASE_REG, addr);
            } else {
                self.asm.add_r(true, SGP3, MEM_BASE_REG, addr);
            }
            (SGP3, offset as u32)
        } else {
            (self.compute_mem_ptr(addr, offset), 0)
        }
    }

    /// Computes `(base_reg, uimm_off)` for a scaled Wasm32 access `MEM_BASE_REG + zext(addr) * scale + offset`.
    fn compute_scaled_mem_base_uimm(
        &mut self,
        addr_val: Val,
        ar: Reg,
        scale: u8,
        offset: u64,
        access_bytes: u64,
    ) -> (Reg, u32) {
        let shift = scale.trailing_zeros() as u8;
        let ar_z = if self.is_upper32_zero(addr_val) {
            ar
        } else {
            self.asm.mov_r(false, SGP2, ar);
            SGP2
        };
        self.asm.add_r_lsl(true, SGP3, MEM_BASE_REG, ar_z, shift);
        if offset == 0 {
            return (SGP3, 0);
        }
        let align_mask = access_bytes - 1;
        let max_uimm = 4095 * access_bytes;
        if (offset & align_mask) == 0 && offset <= max_uimm {
            return (SGP3, offset as u32);
        }
        if offset < 4096 {
            self.asm.add_imm(true, SGP3, SGP3, offset as u16);
        } else if offset < (1 << 24) {
            self.asm
                .add_imm_lsl12(true, SGP3, SGP3, (offset >> 12) as u16);
            let lo = (offset & 0xfff) as u16;
            if lo != 0 {
                self.asm.add_imm(true, SGP3, SGP3, lo);
            }
        } else {
            self.asm.mov_imm64(SGP2, offset);
            self.asm.add_r(true, SGP3, SGP3, SGP2);
        }
        (SGP3, 0)
    }

    fn emit_int_cmp_flags_reusing(
        &mut self,
        prev_flags: Option<A64LiveFlags>,
        kind: CmpBranchKind,
        a: Val,
        b: Val,
    ) -> (Cond, Option<A64LiveFlags>) {
        match kind {
            CmpBranchKind::I32(cop) | CmpBranchKind::I64(cop) => {
                let is64 = matches!(kind, CmpBranchKind::I64(_));
                if prev_flags == Some(A64LiveFlags::IntRR { is64, a, b }) {
                    return (cmp_to_cond(cop), prev_flags);
                }
                if prev_flags == Some(A64LiveFlags::IntRR { is64, a: b, b: a }) {
                    return (cmp_to_cond(cop.swap()), prev_flags);
                }
                let ar = self.gp_read(a, SGP1);
                let br = self.gp_read(b, SGP2);
                self.asm.cmp_r(is64, ar, br);
                (cmp_to_cond(cop), Some(A64LiveFlags::IntRR { is64, a, b }))
            }
            CmpBranchKind::I32Imm(cop, imm) | CmpBranchKind::I64Imm(cop, imm) => {
                let is64 = matches!(kind, CmpBranchKind::I64Imm(..));
                if prev_flags == Some(A64LiveFlags::IntRI { is64, a, imm }) {
                    return (cmp_to_cond(cop), prev_flags);
                }
                let ar = self.gp_read(a, SGP1);
                if (0..4096).contains(&imm) {
                    self.asm.cmp_imm(is64, ar, imm as u16);
                } else {
                    if is64 {
                        self.asm.mov_imm64(SGP2, imm as i64 as u64);
                    } else {
                        self.asm.mov_imm32(SGP2, imm as u32);
                    }
                    self.asm.cmp_r(is64, ar, SGP2);
                }
                (cmp_to_cond(cop), Some(A64LiveFlags::IntRI { is64, a, imm }))
            }
            _ => unreachable!("expected integer CmpBranchKind"),
        }
    }

    fn emit_edge_moves(&mut self, target_bb: u32, args: ValRange) {
        let params = &self.ir.blocks[target_bb as usize].params;
        let arg_vals = self.ir.pool_slice(args);
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

    fn emit_fp_cmp_set(&mut self, is64: bool, op: CmpOp, dst: Reg, a: VReg, b: VReg) {
        self.asm.fcmp(is64, a, b);
        match op {
            CmpOp::Eq => self.asm.cset(false, dst, Cond::Eq),
            CmpOp::Ne => self.asm.cset(false, dst, Cond::Ne),
            CmpOp::LtS => self.asm.cset(false, dst, Cond::Mi), // N=1 (strictly <, false on NaN!)
            CmpOp::LeS => self.asm.cset(false, dst, Cond::Ls), // C=0 || Z=1 (<=, false on NaN!)
            CmpOp::GtS => self.asm.cset(false, dst, Cond::Gt), // Z=0 && N==V (>, false on NaN!)
            CmpOp::GeS => self.asm.cset(false, dst, Cond::Ge), // N==V (>=, false on NaN!)
            _ => unreachable!("invalid FP CmpOp {op:?}"),
        }
    }

    fn fp_cmp_branch_cond(op: CmpOp) -> Cond {
        match op {
            CmpOp::Eq => Cond::Eq,
            CmpOp::Ne => Cond::Ne,
            CmpOp::LtS => Cond::Mi,
            CmpOp::LeS => Cond::Ls,
            CmpOp::GtS => Cond::Gt,
            CmpOp::GeS => Cond::Ge,
            _ => unreachable!("invalid FP CmpOp {op:?}"),
        }
    }

    fn emit_trunc(&mut self, op: TruncOp, dst: Reg, src: VReg) {
        let inv = self.trap_inv_conv_lbl();
        let ovf = self.trap_int_ovf_lbl();
        match op {
            // ARMv8-A `fcvtzs` / `fcvtzu` natively implements Wasm saturating trunc semantics in 1 instruction!
            TruncOp::I32F32SatS => self.asm.fcvtzs(false, false, dst, src),
            TruncOp::I32F32SatU => self.asm.fcvtzu(false, false, dst, src),
            TruncOp::I32F64SatS => self.asm.fcvtzs(false, true, dst, src),
            TruncOp::I32F64SatU => self.asm.fcvtzu(false, true, dst, src),
            TruncOp::I64F32SatS => self.asm.fcvtzs(true, false, dst, src),
            TruncOp::I64F32SatU => self.asm.fcvtzu(true, false, dst, src),
            TruncOp::I64F64SatS => self.asm.fcvtzs(true, true, dst, src),
            TruncOp::I64F64SatU => self.asm.fcvtzu(true, true, dst, src),
            // Trapping truncs: check NaN (TRAP_INVALID_CONV) and exact IEEE-754 bounds (TRAP_INT_OVERFLOW).
            TruncOp::I32F32S
            | TruncOp::I32F32U
            | TruncOp::I32F64S
            | TruncOp::I32F64U
            | TruncOp::I64F32S
            | TruncOp::I64F32U
            | TruncOp::I64F64S
            | TruncOp::I64F64U => {
                let is_f64 = matches!(
                    op,
                    TruncOp::I32F64S | TruncOp::I32F64U | TruncOp::I64F64S | TruncOp::I64F64U
                );
                let is_i64 = matches!(
                    op,
                    TruncOp::I64F32S | TruncOp::I64F32U | TruncOp::I64F64S | TruncOp::I64F64U
                );
                let is_signed = matches!(
                    op,
                    TruncOp::I32F32S | TruncOp::I32F64S | TruncOp::I64F32S | TruncOp::I64F64S
                );
                // 1. NaN check: `fcmp src, src; b.vs trap_inv_conv`.
                self.asm.fcmp(is_f64, src, src);
                self.asm.b_cond(Cond::Vs, inv);
                // 2. Bounds check (exclusive upper bound `hi`, inclusive/exclusive lower bound `lo`).
                let (lo_bits, lo_strict, hi_bits): (u64, bool, u64) = match op {
                    TruncOp::I32F32S => (
                        (-2147483648.0f32).to_bits() as u64,
                        false,
                        2147483648.0f32.to_bits() as u64,
                    ),
                    TruncOp::I32F32U => (
                        (-1.0f32).to_bits() as u64,
                        true,
                        4294967296.0f32.to_bits() as u64,
                    ),
                    TruncOp::I32F64S => (
                        (-2147483649.0f64).to_bits(),
                        true,
                        2147483648.0f64.to_bits(),
                    ),
                    TruncOp::I32F64U => ((-1.0f64).to_bits(), true, 4294967296.0f64.to_bits()),
                    TruncOp::I64F32S => (
                        (-9223372036854775808.0f32).to_bits() as u64,
                        false,
                        9223372036854775808.0f32.to_bits() as u64,
                    ),
                    TruncOp::I64F32U => (
                        (-1.0f32).to_bits() as u64,
                        true,
                        18446744073709551616.0f32.to_bits() as u64,
                    ),
                    TruncOp::I64F64S => (
                        (-9223372036854775808.0f64).to_bits(),
                        false,
                        9223372036854775808.0f64.to_bits(),
                    ),
                    TruncOp::I64F64U => (
                        (-1.0f64).to_bits(),
                        true,
                        18446744073709551616.0f64.to_bits(),
                    ),
                    _ => unreachable!(),
                };
                if is_f64 {
                    self.asm.mov_imm64(SGP3, lo_bits);
                    self.asm.fmov_fp_from_gp(true, SFP2, SGP3);
                } else {
                    self.asm.mov_imm32(SGP3, lo_bits as u32);
                    self.asm.fmov_fp_from_gp(false, SFP2, SGP3);
                }
                self.asm.fcmp(is_f64, src, SFP2);
                self.asm
                    .b_cond(if lo_strict { Cond::Ls } else { Cond::Mi }, ovf);

                if is_f64 {
                    self.asm.mov_imm64(SGP3, hi_bits);
                    self.asm.fmov_fp_from_gp(true, SFP2, SGP3);
                } else {
                    self.asm.mov_imm32(SGP3, hi_bits as u32);
                    self.asm.fmov_fp_from_gp(false, SFP2, SGP3);
                }
                self.asm.fcmp(is_f64, src, SFP2);
                self.asm.b_cond(Cond::Ge, ovf);

                if is_signed {
                    self.asm.fcvtzs(is_i64, is_f64, dst, src);
                } else {
                    self.asm.fcvtzu(is_i64, is_f64, dst, src);
                }
            }
        }
    }

    /// Resolves and dispatches `call_indirect` (static `.rodata`, dynamic `Ctx` table, 32-bit or
    /// `table64`) or typed `call_ref` for `ci_id`.
    /// On `is_tail_jump`, leaves resolved target PC in `SGP1` (`X16`); otherwise emits `blr SGP1`.
    /// Returns `false` if a statically empty table branched unconditionally to `TRAP_INDIRECT_CALL`.
    fn emit_ci_dispatch(&mut self, ci_id: u32, is_tail_jump: bool) -> bool {
        let ci = self.layout.ci_tables[ci_id as usize];
        let toob = self.trap_table_oob_lbl();
        let bad = self.trap_ci_lbl();
        if ci.is_call_ref {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            self.asm.cbz(true, SGP2, bad);
            self.emit_speculation_barrier();
            self.asm.sub_imm(true, SGP2, SGP2, 1);
            self.asm
                .adr_reloc(SGP1, A64RelocTarget::Rodata(self.layout.func_addr_ro_off));
            self.asm.ldrsw_reg_scaled(SGP3, SGP1, SGP2, true);
            self.asm.add_r(true, SGP1, SGP1, SGP3);
            if !is_tail_jump {
                self.asm.blr(SGP1);
            }
            return true;
        }
        emit_load_off(
            &mut self.asm,
            if ci.table_is64 { Ty::I64 } else { Ty::I32 },
            Some(SGP2),
            None,
            CTX_REG,
            OFF_SCRATCH_TMP,
            SGP3,
        );
        if ci.dyn_table_desc_off >= 0 {
            emit_load_off(
                &mut self.asm,
                Ty::I32,
                Some(SGP3),
                None,
                CTX_REG,
                ci.dyn_table_desc_off + 8,
                SGP1,
            );
            self.asm.cmp_r(ci.table_is64, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, toob);
            self.emit_speculation_barrier();
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                ci.dyn_table_desc_off,
                SGP3,
            );
            self.asm.ldr_reg_scaled(true, SGP2, SGP1, SGP2, true);
            self.asm.cbz(true, SGP2, bad);
            self.emit_speculation_barrier();
            self.asm.sub_imm(true, SGP2, SGP2, 1);
            self.asm
                .adr_reloc(SGP1, A64RelocTarget::Rodata(self.layout.func_sig_ro_off));
            self.asm.ldr_reg_scaled(false, SGP3, SGP1, SGP2, true);
            self.asm.mov_imm32(SGP1, ci.canon_sig);
            self.asm.cmp_r(false, SGP3, SGP1);
            self.asm.b_cond(Cond::Ne, bad);
            self.emit_speculation_barrier();
            self.asm
                .adr_reloc(SGP1, A64RelocTarget::Rodata(self.layout.func_addr_ro_off));
            self.asm.ldrsw_reg_scaled(SGP3, SGP1, SGP2, true);
            self.asm.add_r(true, SGP1, SGP1, SGP3);
            if !is_tail_jump {
                self.asm.blr(SGP1);
            }
            return true;
        }
        self.asm.mov_imm32(SGP3, ci.table_len);
        self.asm.cmp_r(ci.table_is64, SGP2, SGP3);
        self.asm.b_cond(Cond::Hs, toob);
        self.emit_speculation_barrier();
        if ci.span == 0 {
            self.asm.b(bad);
            return false;
        }
        if ci.min_slot > 0 {
            self.asm.mov_imm32(SGP3, ci.min_slot);
            self.asm.sub_r(false, SGP2, SGP2, SGP3);
        }
        self.asm.mov_imm32(SGP3, ci.span);
        self.asm.cmp_r(false, SGP2, SGP3);
        self.asm.b_cond(Cond::Hs, bad);
        self.emit_speculation_barrier();
        self.asm
            .adr_reloc(SGP1, A64RelocTarget::Rodata(ci.rodata_off));
        self.asm.ldrsw_reg_scaled(SGP3, SGP1, SGP2, true);
        self.asm.cbz(true, SGP3, bad);
        self.emit_speculation_barrier();
        self.asm.add_r(true, SGP1, SGP1, SGP3);
        if !is_tail_jump {
            self.asm.blr(SGP1);
        }
        true
    }

    fn emit_call_abi(
        &mut self,
        args: ValRange,
        rets: ValRange,
        ci_idx: Option<(u32, Val)>,
        direct_func: Option<u32>,
    ) -> Result<(), String> {
        if let Some((ci_id, idx_val)) = ci_idx {
            let ci = self.layout.ci_tables[ci_id as usize];
            let ity = if ci.is_call_ref || ci.table_is64 {
                Ty::I64
            } else {
                Ty::I32
            };
            let ir = self.gp_read(idx_val, SGP2);
            emit_store_off(
                &mut self.asm,
                ity,
                Some(ir),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
        }
        let arg_vals = self.ir.pool_slice(args);
        if !arg_vals.is_empty() {
            let arg_tys: Vec<Ty> = arg_vals.iter().map(|&v| self.ty(v)).collect();
            let arg_layout = layout_args(&arg_tys)?;
            let mut moves = Vec::with_capacity(arg_vals.len());
            for ((&av, &ty), slot) in arg_vals.iter().zip(&arg_tys).zip(arg_layout.slots) {
                let dst = match slot {
                    A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                    A64AbiSlot::Stack(off) => MoveLoc::Stack(off),
                };
                moves.push((dst, MoveLoc::from(self.loc(av)), ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }

        let ret_vals = self.ir.pool_slice(rets);
        let ret_tys: Vec<Ty> = ret_vals.iter().map(|&v| self.ty(v)).collect();
        let ret_layout = layout_rets(&ret_tys)?;
        if ret_layout.stack_bytes > 0 {
            self.asm
                .add_i32_checked(true, SGP1, Reg::SP, self.alloc.ret_buf_off, SGP3);
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_RET_SPILL_PTR,
                SGP3,
            );
        }

        if let Some(f) = direct_func {
            self.asm.bl_reloc(A64RelocTarget::Func(f));
        } else if let Some((ci_id, _)) = ci_idx
            && !self.emit_ci_dispatch(ci_id, false)
        {
            return Ok(());
        }

        // Check trap in W16.
        let prop = self.trap_prop_lbl();
        self.asm.cbnz(false, SGP1, prop);

        if !ret_vals.is_empty() {
            let mut moves = Vec::with_capacity(ret_vals.len());
            for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(ret_layout.slots) {
                let src = match slot {
                    A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                    A64AbiSlot::Stack(off) => MoveLoc::Stack(self.alloc.ret_buf_off + off),
                };
                moves.push((MoveLoc::from(self.loc(rv)), src, ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_call_catch_abi(
        &mut self,
        args: ValRange,
        ok_bb: crate::x64::ir::BlockId,
        catch_bb: crate::x64::ir::BlockId,
        catch_args: ValRange,
        ci_idx: Option<(u32, Val)>,
        direct_func: Option<u32>,
        next_bb: Option<u32>,
    ) -> Result<(), String> {
        if let Some((ci_id, idx_val)) = ci_idx {
            let ci = self.layout.ci_tables[ci_id as usize];
            let ity = if ci.is_call_ref || ci.table_is64 {
                Ty::I64
            } else {
                Ty::I32
            };
            let ir = self.gp_read(idx_val, SGP2);
            emit_store_off(
                &mut self.asm,
                ity,
                Some(ir),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
        }
        let arg_vals = self.ir.pool_slice(args);
        if !arg_vals.is_empty() {
            let arg_tys: Vec<Ty> = arg_vals.iter().map(|&v| self.ty(v)).collect();
            let arg_layout = layout_args(&arg_tys)?;
            let mut moves = Vec::with_capacity(arg_vals.len());
            for ((&av, &ty), slot) in arg_vals.iter().zip(&arg_tys).zip(arg_layout.slots) {
                let dst = match slot {
                    A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                    A64AbiSlot::Stack(off) => MoveLoc::Stack(off),
                };
                moves.push((dst, MoveLoc::from(self.loc(av)), ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }

        let ret_vals = self.ir.blocks[ok_bb.0 as usize].params.clone();
        let ret_tys: Vec<Ty> = ret_vals.iter().map(|&v| self.ty(v)).collect();
        let ret_layout = layout_rets(&ret_tys)?;
        if ret_layout.stack_bytes > 0 {
            self.asm
                .add_i32_checked(true, SGP1, Reg::SP, self.alloc.ret_buf_off, SGP3);
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_RET_SPILL_PTR,
                SGP3,
            );
        }

        if let Some(f) = direct_func {
            self.asm.bl_reloc(A64RelocTarget::Func(f));
        } else if let Some((ci_id, _)) = ci_idx
            && !self.emit_ci_dispatch(ci_id, false)
        {
            return Ok(());
        }

        let lbl_ok = self.asm.new_label();
        let prop = self.trap_prop_lbl();
        let catch_lbl = self.edge_target_label(catch_bb.0, catch_args);
        self.asm.cbz(false, SGP1, lbl_ok);
        self.asm
            .cmp_imm(false, SGP1, u16::from(crate::x64::INTERNAL_EXN_SIGNAL));
        self.asm.b_cond(Cond::Eq, catch_lbl);
        self.asm.b(prop);
        self.asm.bind(lbl_ok);

        if !ret_vals.is_empty() {
            let mut moves = Vec::with_capacity(ret_vals.len());
            for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(ret_layout.slots) {
                let src = match slot {
                    A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                    A64AbiSlot::Stack(off) => MoveLoc::Stack(self.alloc.ret_buf_off + off),
                };
                moves.push((MoveLoc::from(self.loc(rv)), src, ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }
        if next_bb != Some(ok_bb.0) {
            self.asm.b(self.bb_labels[ok_bb.0 as usize]);
        }
        Ok(())
    }

    fn emit_tail_call(
        &mut self,
        args: ValRange,
        ci_idx: Option<(u32, Val)>,
        direct_func: Option<u32>,
    ) -> Result<(), String> {
        let param_tys: Vec<Ty> = self.ir.params.iter().map(|&pv| self.ty(pv)).collect();
        let self_param_stack = layout_args(&param_tys)?.stack_bytes;
        let arg_vals = self.ir.pool_slice(args);
        let arg_tys: Vec<Ty> = arg_vals.iter().map(|&v| self.ty(v)).collect();
        let arg_layout = layout_args(&arg_tys)?;
        let can_tail_jump = arg_layout.stack_bytes <= self_param_stack;

        if let Some((ci_id, idx_val)) = ci_idx {
            let ci = self.layout.ci_tables[ci_id as usize];
            let ity = if ci.is_call_ref || ci.table_is64 {
                Ty::I64
            } else {
                Ty::I32
            };
            let ir = self.gp_read(idx_val, SGP2);
            emit_store_off(
                &mut self.asm,
                ity,
                Some(ir),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
        }

        if can_tail_jump {
            // True O(1)-stack tail call: overwrite incoming stack-arg slots at `[sp + frame_size + off]`,
            // resolve indirect target into SGP1 (X16, preserved across epilogue teardown), tear down
            // our frame (restoring LR if saved), and branch directly with `b` / `br X16`!
            if !arg_vals.is_empty() {
                let mut moves = Vec::with_capacity(arg_vals.len());
                for ((&av, &ty), slot) in arg_vals.iter().zip(&arg_tys).zip(arg_layout.slots) {
                    let dst = match slot {
                        A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                        A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                        A64AbiSlot::Stack(off) => MoveLoc::Stack(self.alloc.frame_size + off),
                    };
                    moves.push((dst, MoveLoc::from(self.loc(av)), ty));
                }
                emit_parallel_moves(&mut self.asm, &moves);
            }
            if let Some((ci_id, _)) = ci_idx
                && !self.emit_ci_dispatch(ci_id, true)
            {
                return Ok(());
            }
            if let Some(saved_off) = self.alloc.saved_ret_ptr_off {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    Reg::SP,
                    saved_off,
                    SGP3,
                );
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_RET_SPILL_PTR,
                    SGP3,
                );
            }
            self.emit_epilogue_teardown();
            if let Some(f) = direct_func {
                self.asm.b_reloc(A64RelocTarget::Func(f));
            } else {
                self.asm.br(SGP1);
            }
            return Ok(());
        }

        // Growing-arity fallback (`callee_arg_stack > caller_param_stack`): emit normal `bl`/`blr`
        // forwarding caller's overflow return buffer pointer, then jump to epilogue.
        if !arg_vals.is_empty() {
            let mut moves = Vec::with_capacity(arg_vals.len());
            for ((&av, &ty), slot) in arg_vals.iter().zip(&arg_tys).zip(arg_layout.slots) {
                let dst = match slot {
                    A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
                    A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
                    A64AbiSlot::Stack(off) => MoveLoc::Stack(off),
                };
                moves.push((dst, MoveLoc::from(self.loc(av)), ty));
            }
            emit_parallel_moves(&mut self.asm, &moves);
        }
        if let Some(saved_off) = self.alloc.saved_ret_ptr_off {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                Reg::SP,
                saved_off,
                SGP3,
            );
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                CTX_REG,
                OFF_RET_SPILL_PTR,
                SGP3,
            );
        }
        if let Some(f) = direct_func {
            self.asm.bl_reloc(A64RelocTarget::Func(f));
        } else if let Some((ci_id, _)) = ci_idx
            && !self.emit_ci_dispatch(ci_id, false)
        {
            return Ok(());
        }
        // Return values & W16 trap status are already in exact ABI registers.
        let ep = *self.epilogue_ok.get_or_insert_with(|| self.asm.new_label());
        self.asm.b(ep);
        Ok(())
    }

    fn emit_int_bin(&mut self, is64: bool, op: IntBinOp, dst_val: Val, a: Val, b: Val) {
        let ar = self.gp_read(a, SGP1);
        let br = self.gp_read(b, SGP2);
        let d = self.gp_write_reg(dst_val);
        match op {
            IntBinOp::Add => self.asm.add_r(is64, d, ar, br),
            IntBinOp::Sub => self.asm.sub_r(is64, d, ar, br),
            IntBinOp::Mul => self.asm.mul(is64, d, ar, br),
            IntBinOp::MulHiS => {
                debug_assert!(is64);
                self.asm.smulh(d, ar, br);
            }
            IntBinOp::MulHiU => {
                debug_assert!(is64);
                self.asm.umulh(d, ar, br);
            }
            IntBinOp::And => self.asm.and_r(is64, d, ar, br),
            IntBinOp::Or => self.asm.orr_r(is64, d, ar, br),
            IntBinOp::Xor => self.asm.eor_r(is64, d, ar, br),
            IntBinOp::Shl => self.asm.lslv(is64, d, ar, br),
            IntBinOp::ShrS => self.asm.asrv(is64, d, ar, br),
            IntBinOp::ShrU => self.asm.lsrv(is64, d, ar, br),
            IntBinOp::Rotr => self.asm.rorv(is64, d, ar, br),
            IntBinOp::Rotl => {
                // rotl(a, b) = ror(a, -b) (AArch64 rorv masks shift count mod 32/64 automatically!)
                self.asm.neg_r(is64, SGP3, br);
                self.asm.rorv(is64, d, ar, SGP3);
            }
            IntBinOp::DivU => {
                let div0 = self.trap_div0_lbl();
                self.asm.cbz(is64, br, div0);
                self.asm.udiv(is64, d, ar, br);
            }
            IntBinOp::DivUNz => {
                self.asm.udiv(is64, d, ar, br);
            }
            IntBinOp::RemU => {
                let div0 = self.trap_div0_lbl();
                self.asm.cbz(is64, br, div0);
                self.asm.udiv(is64, SGP3, ar, br);
                self.asm.msub(is64, d, SGP3, br, ar);
            }
            IntBinOp::RemUNz => {
                self.asm.udiv(is64, SGP3, ar, br);
                self.asm.msub(is64, d, SGP3, br, ar);
            }
            IntBinOp::DivS => {
                let div0 = self.trap_div0_lbl();
                let ovf = self.trap_int_ovf_lbl();
                let ok = self.asm.new_label();
                self.asm.cbz(is64, br, div0);
                if is64 {
                    self.asm.mov_imm64(SGP3, u64::MAX);
                } else {
                    self.asm.mov_imm32(SGP3, u32::MAX);
                }
                self.asm.cmp_r(is64, br, SGP3);
                self.asm.b_cond(Cond::Ne, ok);
                if is64 {
                    self.asm.mov_imm64(SGP3, i64::MIN as u64);
                } else {
                    self.asm.mov_imm32(SGP3, i32::MIN as u32);
                }
                self.asm.cmp_r(is64, ar, SGP3);
                self.asm.b_cond(Cond::Eq, ovf);
                self.asm.bind(ok);
                self.asm.sdiv(is64, d, ar, br);
            }
            IntBinOp::RemS => {
                // Note: ARMv8 `sdiv(MIN, -1)` returns `MIN`, and `msub(MIN, -1, MIN) == 0`!
                // Zero overflow trap needed on `RemS`, ONLY div-by-zero check!
                let div0 = self.trap_div0_lbl();
                self.asm.cbz(is64, br, div0);
                self.asm.sdiv(is64, SGP3, ar, br);
                self.asm.msub(is64, d, SGP3, br, ar);
            }
        }
        self.gp_commit(dst_val, d);
    }

    fn emit_int_bin_imm(&mut self, is64: bool, op: IntBinOp, dst_val: Val, a: Val, imm: i32) {
        let ar = self.gp_read(a, SGP1);
        let d = self.gp_write_reg(dst_val);
        match op {
            IntBinOp::Add => {
                self.asm.add_i32_checked(is64, d, ar, imm, SGP3);
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::Sub => {
                if let Some(neg) = imm.checked_neg() {
                    self.asm.add_i32_checked(is64, d, ar, neg, SGP3);
                    self.gp_commit(dst_val, d);
                    return;
                }
            }
            IntBinOp::Shl => {
                self.asm.lsl_imm(is64, d, ar, imm as u8);
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::ShrS => {
                self.asm.asr_imm(is64, d, ar, imm as u8);
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::ShrU => {
                self.asm.lsr_imm(is64, d, ar, imm as u8);
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::Rotr => {
                self.asm.ror_imm(is64, d, ar, imm as u8);
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::Rotl => {
                let w = if is64 { 64u8 } else { 32u8 };
                let sh = (imm as u8) & (w - 1);
                self.asm.ror_imm(is64, d, ar, (w - sh) & (w - 1));
                self.gp_commit(dst_val, d);
                return;
            }
            IntBinOp::Mul => {
                // Strength-reduce small/power-of-two constant multiplies into 1–2 single-cycle
                // shifted `add`/`sub`/`lsl` ops instead of `mov_imm + mul`.
                let imm64 = imm as i64;
                let neg = imm64 < 0;
                let u = imm64.unsigned_abs();
                if u == 0 {
                    self.asm.movz(is64, d, 0, 0);
                    self.gp_commit(dst_val, d);
                    return;
                }
                let tz = u.trailing_zeros() as u8;
                let odd = u >> tz;
                let w = if is64 { 64u8 } else { 32u8 };
                if tz < w {
                    if odd == 1 {
                        if tz == 0 {
                            if neg {
                                self.asm.neg_r(is64, d, ar);
                            } else {
                                self.asm.mov_r(is64, d, ar);
                            }
                        } else {
                            self.asm.lsl_imm(is64, d, ar, tz);
                            if neg {
                                self.asm.neg_r(is64, d, d);
                            }
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    }
                    if (!neg || tz == 0)
                        && let Some(k) = (1u8..=6u8).find(|&k| odd == (1u64 << k) + 1)
                    {
                        self.asm.add_r_lsl(is64, d, ar, ar, k);
                        if tz > 0 {
                            self.asm.lsl_imm(is64, d, d, tz);
                        }
                        if neg {
                            self.asm.neg_r(is64, d, d);
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    }
                    if tz == 0
                        && let Some(k) = (3u8..=6u8).find(|&k| odd == (1u64 << k) - 1)
                    {
                        if neg {
                            self.asm.sub_r_lsl(is64, d, ar, ar, k);
                        } else {
                            self.asm.lsl_imm(is64, SGP2, ar, k);
                            self.asm.sub_r(is64, d, SGP2, ar);
                        }
                        self.gp_commit(dst_val, d);
                        return;
                    }
                    if !neg && tz == 0 {
                        let pair = match odd {
                            25 => Some((2u8, 2u8)),
                            27 => Some((1u8, 3u8)),
                            45 => Some((2u8, 3u8)),
                            81 => Some((3u8, 3u8)),
                            _ => None,
                        };
                        if let Some((k1, k2)) = pair {
                            self.asm.add_r_lsl(is64, SGP2, ar, ar, k1);
                            self.asm.add_r_lsl(is64, d, SGP2, SGP2, k2);
                            self.gp_commit(dst_val, d);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
        // Fallback: materialize immediate in SGP2 and reuse general register op.
        if is64 {
            self.asm.mov_imm64(SGP2, imm as i64 as u64);
        } else {
            self.asm.mov_imm32(SGP2, imm as u32);
        }
        match op {
            IntBinOp::Sub => self.asm.sub_r(is64, d, ar, SGP2),
            IntBinOp::Mul => self.asm.mul(is64, d, ar, SGP2),
            IntBinOp::And => self.asm.and_r(is64, d, ar, SGP2),
            IntBinOp::Or => self.asm.orr_r(is64, d, ar, SGP2),
            IntBinOp::Xor => self.asm.eor_r(is64, d, ar, SGP2),
            IntBinOp::DivU | IntBinOp::DivUNz => {
                if imm == 0 {
                    let div0 = self.trap_div0_lbl();
                    self.asm.b(div0);
                } else {
                    self.asm.udiv(is64, d, ar, SGP2);
                }
            }
            IntBinOp::RemU | IntBinOp::RemUNz => {
                if imm == 0 {
                    let div0 = self.trap_div0_lbl();
                    self.asm.b(div0);
                } else {
                    self.asm.udiv(is64, SGP3, ar, SGP2);
                    self.asm.msub(is64, d, SGP3, SGP2, ar);
                }
            }
            IntBinOp::DivS => {
                if imm == 0 {
                    let div0 = self.trap_div0_lbl();
                    self.asm.b(div0);
                } else {
                    if imm == -1 {
                        let ovf = self.trap_int_ovf_lbl();
                        if is64 {
                            self.asm.mov_imm64(SGP3, i64::MIN as u64);
                        } else {
                            self.asm.mov_imm32(SGP3, i32::MIN as u32);
                        }
                        self.asm.cmp_r(is64, ar, SGP3);
                        self.asm.b_cond(Cond::Eq, ovf);
                    }
                    self.asm.sdiv(is64, d, ar, SGP2);
                }
            }
            IntBinOp::RemS => {
                if imm == 0 {
                    let div0 = self.trap_div0_lbl();
                    self.asm.b(div0);
                } else {
                    self.asm.sdiv(is64, SGP3, ar, SGP2);
                    self.asm.msub(is64, d, SGP3, SGP2, ar);
                }
            }
            _ => unreachable!(),
        }
        self.gp_commit(dst_val, d);
    }

    fn emit_inst(&mut self, res: Option<Val>, op: Op) -> Result<(), String> {
        let prev_flags = self.live_flags.take();
        match op {
            Op::ConstI32(c) => {
                let r = res.unwrap();
                if matches!(self.loc(r), A64Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.gp_write_reg(r);
                self.asm.mov_imm32(d, c as u32);
                self.gp_commit(r, d);
            }
            Op::ConstI64(c) => {
                let r = res.unwrap();
                if matches!(self.loc(r), A64Loc::Remat(_)) {
                    return Ok(());
                }
                let d = self.gp_write_reg(r);
                self.asm.mov_imm64(d, c as u64);
                self.gp_commit(r, d);
            }
            Op::ConstF32(bits) => {
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                self.asm.mov_imm32(SGP1, bits);
                self.asm.fmov_fp_from_gp(false, d, SGP1);
                self.fp_commit(r, d);
            }
            Op::ConstF64(bits) => {
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                self.asm.mov_imm64(SGP1, bits);
                self.asm.fmov_fp_from_gp(true, d, SGP1);
                self.fp_commit(r, d);
            }
            Op::ConstV128(idx) => {
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                let bytes = self.ir.v128_consts[idx as usize];
                if bytes == [0u8; 16] {
                    self.asm.movi_zero_16b(d);
                } else {
                    let loff = self.asm.intern_v128(bytes);
                    self.asm.adr_reloc(SGP1, A64RelocTarget::LocalRodata(loff));
                    self.asm.ldr_q_uimm(d, SGP1, 0);
                }
                self.fp_commit(r, d);
            }
            Op::I32Eqz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.cmp_imm(false, ar, 0);
                self.asm.cset(false, d, Cond::Eq);
                self.gp_commit(r, d);
            }
            Op::I64Eqz(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.cmp_imm(true, ar, 0);
                self.asm.cset(false, d, Cond::Eq);
                self.gp_commit(r, d);
            }
            Op::I32Clz(a) | Op::I64Clz(a) => {
                let is64 = matches!(op, Op::I64Clz(_));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.clz(is64, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Ctz(a) | Op::I64Ctz(a) => {
                let is64 = matches!(op, Op::I64Ctz(_));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.rbit(is64, d, ar);
                self.asm.clz(is64, d, d);
                self.gp_commit(r, d);
            }
            Op::I32Popcnt(a) | Op::I64Popcnt(a) => {
                let is64 = matches!(op, Op::I64Popcnt(_));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                if !is64 {
                    self.asm.uxtw(SGP2, ar);
                    self.asm.fmov_fp_from_gp(true, SFP1, SGP2);
                } else {
                    self.asm.fmov_fp_from_gp(true, SFP1, ar);
                }
                self.asm.cnt_v(false, SFP1, SFP1);
                self.asm.addv_8b(SFP1, SFP1);
                self.asm.umov(0, d, SFP1, 0);
                self.gp_commit(r, d);
            }
            Op::I32WrapI64(a) | Op::I64ExtendI32U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                if self.is_upper32_zero(a) {
                    if d != ar {
                        self.asm.mov_r(true, d, ar);
                    }
                } else {
                    self.asm.uxtw(d, ar);
                }
                self.gp_commit(r, d);
            }
            Op::I64ExtendI32S(a) | Op::I64Extend32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.sxtw(d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Extend8S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.sxtb(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Extend16S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.sxth(false, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64Extend8S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.sxtb(true, d, ar);
                self.gp_commit(r, d);
            }
            Op::I64Extend16S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.sxth(true, d, ar);
                self.gp_commit(r, d);
            }
            Op::I32Bin(bop, a, b) => self.emit_int_bin(false, bop, res.unwrap(), a, b),
            Op::I64Bin(bop, a, b) => self.emit_int_bin(true, bop, res.unwrap(), a, b),
            Op::I32BinImm(bop, a, imm) => self.emit_int_bin_imm(false, bop, res.unwrap(), a, imm),
            Op::I64BinImm(bop, a, imm) => self.emit_int_bin_imm(true, bop, res.unwrap(), a, imm),
            Op::AddScaled {
                is64,
                base,
                idx,
                shift,
                disp,
            } => {
                let r = res.unwrap();
                let br = self.gp_read(base, SGP1);
                let ir = self.gp_read(idx, SGP2);
                let d = self.gp_write_reg(r);
                self.asm.add_r_lsl(is64, d, br, ir, shift);
                if disp != 0 {
                    if disp > 0 && disp < 4096 {
                        self.asm.add_imm(is64, d, d, disp as u16);
                    } else if disp < 0 && disp > -4096 {
                        self.asm.sub_imm(is64, d, d, (-disp) as u16);
                    } else {
                        self.asm.mov_imm64(SGP3, i64::from(disp) as u64);
                        self.asm.add_r(is64, d, d, SGP3);
                    }
                }
                self.gp_commit(r, d);
            }
            Op::I32Cmp(cop, a, b) | Op::I64Cmp(cop, a, b) => {
                let is64 = matches!(op, Op::I64Cmp(..));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let br = self.gp_read(b, SGP2);
                let d = self.gp_write_reg(r);
                self.asm.cmp_r(is64, ar, br);
                self.asm.cset(false, d, cmp_to_cond(cop));
                self.gp_commit(r, d);
            }
            Op::I32CmpImm(cop, a, imm) | Op::I64CmpImm(cop, a, imm) => {
                let is64 = matches!(op, Op::I64CmpImm(..));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.gp_write_reg(r);
                if (0..4096).contains(&imm) {
                    self.asm.cmp_imm(is64, ar, imm as u16);
                } else {
                    if is64 {
                        self.asm.mov_imm64(SGP2, imm as i64 as u64);
                    } else {
                        self.asm.mov_imm32(SGP2, imm as u32);
                    }
                    self.asm.cmp_r(is64, ar, SGP2);
                }
                self.asm.cset(false, d, cmp_to_cond(cop));
                self.gp_commit(r, d);
            }
            Op::F32Cmp(cop, a, b) | Op::F64Cmp(cop, a, b) => {
                let is64 = matches!(op, Op::F64Cmp(..));
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let d = self.gp_write_reg(r);
                self.emit_fp_cmp_set(is64, cop, d, ar, br);
                self.gp_commit(r, d);
            }
            Op::F32Un(uop, a) | Op::F64Un(uop, a) => {
                let is64 = matches!(op, Op::F64Un(..));
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.fp_write_reg(r);
                match uop {
                    FloatUnOp::Abs => self.asm.fabs(is64, d, ar),
                    FloatUnOp::Neg => self.asm.fneg(is64, d, ar),
                    FloatUnOp::Sqrt => self.asm.fsqrt(is64, d, ar),
                    FloatUnOp::Ceil => self.asm.frintp(is64, d, ar),
                    FloatUnOp::Floor => self.asm.frintm(is64, d, ar),
                    FloatUnOp::Trunc => self.asm.frintz(is64, d, ar),
                    FloatUnOp::Nearest => self.asm.frintn(is64, d, ar),
                }
                self.fp_commit(r, d);
            }
            Op::F32Bin(bop, a, b) | Op::F64Bin(bop, a, b) => {
                let is64 = matches!(op, Op::F64Bin(..));
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let d = self.fp_write_reg(r);
                match bop {
                    FloatBinOp::Add => self.asm.fadd(is64, d, ar, br),
                    FloatBinOp::Sub => self.asm.fsub(is64, d, ar, br),
                    FloatBinOp::Mul => self.asm.fmul(is64, d, ar, br),
                    FloatBinOp::Div => self.asm.fdiv(is64, d, ar, br),
                    FloatBinOp::Min => self.asm.fmin(is64, d, ar, br),
                    FloatBinOp::Max => self.asm.fmax(is64, d, ar, br),
                    FloatBinOp::Copysign => {
                        self.asm.fmov_gp_from_fp(is64, SGP1, ar);
                        self.asm.fmov_gp_from_fp(is64, SGP2, br);
                        if is64 {
                            self.asm.mov_imm64(SGP3, 0x8000_0000_0000_0000);
                        } else {
                            self.asm.mov_imm32(SGP3, 0x8000_0000);
                        }
                        self.asm.bic_r(is64, SGP1, SGP1, SGP3);
                        self.asm.and_r(is64, SGP2, SGP2, SGP3);
                        self.asm.orr_r(is64, SGP1, SGP1, SGP2);
                        self.asm.fmov_fp_from_gp(is64, d, SGP1);
                    }
                }
                self.fp_commit(r, d);
            }
            Op::F32DemoteF64(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.fp_write_reg(r);
                self.asm.fcvt_s_d(d, ar);
                self.fp_commit(r, d);
            }
            Op::F64PromoteF32(a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.fp_write_reg(r);
                self.asm.fcvt_d_s(d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.scvtf(false, false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI32U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.ucvtf(false, false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI64S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.scvtf(true, false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F32ConvertI64U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.ucvtf(true, false, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI32S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.scvtf(false, true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI32U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.ucvtf(false, true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI64S(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.scvtf(true, true, d, ar);
                self.fp_commit(r, d);
            }
            Op::F64ConvertI64U(a) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.ucvtf(true, true, d, ar);
                self.fp_commit(r, d);
            }
            Op::I32ReinterpretF32(a) | Op::I64ReinterpretF64(a) => {
                let is64 = matches!(op, Op::I64ReinterpretF64(_));
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.gp_write_reg(r);
                self.asm.fmov_gp_from_fp(is64, d, ar);
                self.gp_commit(r, d);
            }
            Op::F32ReinterpretI32(a) | Op::F64ReinterpretI64(a) => {
                let is64 = matches!(op, Op::F64ReinterpretI64(_));
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let d = self.fp_write_reg(r);
                self.asm.fmov_fp_from_gp(is64, d, ar);
                self.fp_commit(r, d);
            }
            Op::Trunc(top, a) => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.gp_write_reg(r);
                self.emit_trunc(top, d, ar);
                self.gp_commit(r, d);
            }
            Op::Select { cond, a, b } => {
                let r = res.unwrap();
                let cr = self.gp_read(cond, SGP3);
                self.asm.cmp_imm(false, cr, 0);
                let ty = self.ty(r);
                if ty == Ty::V128 {
                    let lbl_f = self.asm.new_label();
                    let lbl_d = self.asm.new_label();
                    let ar = self.fp_read(a, SFP1);
                    let br = self.fp_read(b, SFP2);
                    let d = self.fp_write_reg(r);
                    self.asm.b_cond(Cond::Eq, lbl_f);
                    self.asm.mov_16b(d, ar);
                    self.asm.b(lbl_d);
                    self.asm.bind(lbl_f);
                    self.asm.mov_16b(d, br);
                    self.asm.bind(lbl_d);
                    self.fp_commit(r, d);
                } else if ty.is_fp() {
                    let ar = self.fp_read(a, SFP1);
                    let br = self.fp_read(b, SFP2);
                    let d = self.fp_write_reg(r);
                    self.asm.fcsel(ty == Ty::F64, d, ar, br, Cond::Ne);
                    self.fp_commit(r, d);
                } else {
                    let ar = self.gp_read(a, SGP1);
                    let br = self.gp_read(b, SGP2);
                    let d = self.gp_write_reg(r);
                    self.asm.csel(ty == Ty::I64, d, ar, br, Cond::Ne);
                    self.gp_commit(r, d);
                }
            }
            Op::CmpSelect { kind, a, b, tv, fv } => {
                let r = res.unwrap();
                let ty = self.ty(r);
                let (cond, next_flags) = match kind {
                    CmpBranchKind::I32(_)
                    | CmpBranchKind::I64(_)
                    | CmpBranchKind::I32Imm(..)
                    | CmpBranchKind::I64Imm(..) => {
                        self.emit_int_cmp_flags_reusing(prev_flags, kind, a, b)
                    }
                    CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::F64(_));
                        let ar = self.fp_read(a, SFP1);
                        let br = self.fp_read(b, SFP2);
                        self.asm.fcmp(is64, ar, br);
                        (Self::fp_cmp_branch_cond(cop), None)
                    }
                };
                // Neither `gp_read`/`fp_read`/`emit_load_off`/`add_i32_checked`/`csel`/`fcsel`/`mov`
                // nor `gp_commit`/`fp_commit` modifies NZCV flags on AArch64, so we emit direct
                // `csel`/`fcsel`/`b.cond` with zero `cset + cmp` overhead and keep `live_flags` live!
                if ty == Ty::V128 {
                    let lbl_f = self.asm.new_label();
                    let lbl_d = self.asm.new_label();
                    let tr = self.fp_read(tv, SFP1);
                    let fr = self.fp_read(fv, SFP2);
                    let d = self.fp_write_reg(r);
                    self.asm.b_cond(cond.invert(), lbl_f);
                    self.asm.mov_16b(d, tr);
                    self.asm.b(lbl_d);
                    self.asm.bind(lbl_f);
                    self.asm.mov_16b(d, fr);
                    self.asm.bind(lbl_d);
                    self.fp_commit(r, d);
                } else if ty.is_fp() {
                    let tr = self.fp_read(tv, SFP1);
                    let fr = self.fp_read(fv, SFP2);
                    let d = self.fp_write_reg(r);
                    self.asm.fcsel(ty == Ty::F64, d, tr, fr, cond);
                    self.fp_commit(r, d);
                } else {
                    let tr = self.gp_read(tv, SGP1);
                    let fr = self.gp_read(fv, SGP2);
                    let d = self.gp_write_reg(r);
                    self.asm.csel(ty == Ty::I64, d, tr, fr, cond);
                    self.gp_commit(r, d);
                }
                self.live_flags = next_flags;
            }
            Op::GlobalGet(g) => {
                let r = res.unwrap();
                let ty = self.ty(r);
                if self.layout.pinned_global == Some(g) && !ty.is_fp() {
                    self.gp_commit(r, PINNED_GLOB_REG);
                } else {
                    let off = self.layout.global_offs[g as usize];
                    if ty.is_fp() {
                        let d = self.fp_write_reg(r);
                        emit_load_off(&mut self.asm, ty, None, Some(d), CTX_REG, off, SGP3);
                        self.fp_commit(r, d);
                    } else {
                        let d = self.gp_write_reg(r);
                        emit_load_off(&mut self.asm, ty, Some(d), None, CTX_REG, off, SGP3);
                        self.gp_commit(r, d);
                    }
                }
            }
            Op::GlobalSet(g, v) => {
                let ty = self.ty(v);
                if self.layout.pinned_global == Some(g) && !ty.is_fp() {
                    let vr = self.gp_read(v, PINNED_GLOB_REG);
                    if vr != PINNED_GLOB_REG {
                        self.asm.mov_r(ty == Ty::I64, PINNED_GLOB_REG, vr);
                    }
                } else {
                    let off = self.layout.global_offs[g as usize];
                    if ty.is_fp() {
                        let vr = self.fp_read(v, SFP1);
                        emit_store_off(&mut self.asm, ty, None, Some(vr), CTX_REG, off, SGP3);
                    } else {
                        let vr = self.gp_read(v, SGP1);
                        emit_store_off(&mut self.asm, ty, Some(vr), None, CTX_REG, off, SGP3);
                    }
                }
            }
            Op::TableGet { table, idx } => {
                let r = res.unwrap();
                let t64 = self.layout.table_is_64(table);
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                if let Some(&(off, len)) = self.layout.extern_tables.get(&table) {
                    self.asm.mov_imm32(SGP2, len);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP2);
                    let d = self.gp_write_reg(r);
                    self.asm.ldr_reg_scaled(true, d, SGP3, ir, true);
                    self.gp_commit(r, d);
                } else {
                    let (desc_off, _, _) = self.layout.dyn_tables[&table];
                    emit_load_off(
                        &mut self.asm,
                        Ty::I32,
                        Some(SGP2),
                        None,
                        CTX_REG,
                        desc_off + 8,
                        SGP3,
                    );
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        desc_off,
                        SGP2,
                    );
                    let d = self.gp_write_reg(r);
                    self.asm.ldr_reg_scaled(true, d, SGP3, ir, true);
                    self.gp_commit(r, d);
                }
            }
            Op::TableSet { table, idx, val } => {
                let t64 = self.layout.table_is_64(table);
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                if let Some(&(off, len)) = self.layout.extern_tables.get(&table) {
                    self.asm.mov_imm32(SGP2, len);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    let vr = self.gp_read(val, SGP1);
                    let idx_r = if self.loc(idx) != A64Loc::Gp(ir) {
                        self.asm.mov_r(true, SGP2, vr);
                        let ii = self.gp_read(idx, SGP1);
                        (ii, SGP2)
                    } else {
                        (ir, vr)
                    };
                    self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP3);
                    self.asm.str_reg_scaled(true, idx_r.1, SGP3, idx_r.0, true);
                } else {
                    let (desc_off, _, _) = self.layout.dyn_tables[&table];
                    emit_load_off(
                        &mut self.asm,
                        Ty::I32,
                        Some(SGP2),
                        None,
                        CTX_REG,
                        desc_off + 8,
                        SGP3,
                    );
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        desc_off,
                        SGP2,
                    );
                    self.asm.uxtw(SGP2, ir);
                    self.asm.lsl_imm(true, SGP2, SGP2, 3);
                    self.asm.add_r(true, SGP2, SGP3, SGP2);
                    let vr = self.gp_read(val, SGP1);
                    self.asm.str_uimm(true, vr, SGP2, 0);
                }
            }
            Op::BoundsCheck {
                addr,
                end_off,
                for_bulk,
            } => {
                if !for_bulk
                    && self.layout.use_guard_pages
                    && !self.layout.memory64
                    && end_off < (1u64 << 32)
                {
                    return Ok(());
                }
                let ar = self.gp_read(addr, SGP1);
                self.emit_bounds_check(ar, end_off);
            }
            Op::Load {
                kind,
                addr,
                offset,
                checked,
            } => {
                let r = res.unwrap();
                let ar = self.gp_read(addr, SGP1);
                let guard_ok =
                    self.layout.use_guard_pages && !self.layout.memory64 && offset < (1u64 << 32);
                if !checked && !guard_ok {
                    self.emit_bounds_check(ar, offset.saturating_add(kind.bytes()));
                }
                let uxtw = !self.layout.memory64;
                if offset == 0 {
                    // Single-instruction register-indexed addressing `[MEM_BASE_REG, addr, UXTW/LSL #0]`!
                    match kind {
                        LoadKind::I32 => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_reg(false, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64 => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_reg(true, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::F32 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_s_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.fp_commit(r, d);
                        }
                        LoadKind::F64 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.fp_commit(r, d);
                        }
                        LoadKind::I32Load8S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsb_reg(false, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load8U | LoadKind::I64Load8U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrb_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load16S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsh_reg(false, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load16U | LoadKind::I64Load16U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrh_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load8S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsb_reg(true, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load16S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsh_reg(true, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load32S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsw_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load32U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_reg(false, d, MEM_BASE_REG, ar, uxtw);
                            self.gp_commit(r, d);
                        }
                        LoadKind::V128 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_q_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8x8S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(false, false, 0, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8x8U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(true, false, 0, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16x4S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(false, false, 1, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16x4U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(true, false, 1, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32x2S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(false, false, 2, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32x2U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.asm.xtl_128(true, false, 2, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldrb_reg(SGP1, MEM_BASE_REG, ar, uxtw);
                            self.asm.dup_gp_128(0, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldrh_reg(SGP1, MEM_BASE_REG, ar, uxtw);
                            self.asm.dup_gp_128(1, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_reg(false, SGP1, MEM_BASE_REG, ar, uxtw);
                            self.asm.dup_gp_128(2, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load64Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_reg(true, SGP1, MEM_BASE_REG, ar, uxtw);
                            self.asm.dup_gp_128(3, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32Zero => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_s_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load64Zero => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_reg(d, MEM_BASE_REG, ar, uxtw);
                            self.fp_commit(r, d);
                        }
                    }
                } else {
                    let (ptr, uimm) = self.compute_mem_base_uimm(ar, offset, kind.bytes());
                    match kind {
                        LoadKind::I32 => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_uimm(false, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64 => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_uimm(true, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::F32 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_s_uimm(d, ptr, uimm);
                            self.fp_commit(r, d);
                        }
                        LoadKind::F64 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.fp_commit(r, d);
                        }
                        LoadKind::I32Load8S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsb_uimm(false, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load8U | LoadKind::I64Load8U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrb_uimm(d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load16S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsh_uimm(false, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I32Load16U | LoadKind::I64Load16U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrh_uimm(d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load8S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsb_uimm(true, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load16S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsh_uimm(true, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load32S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsw_uimm(d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::I64Load32U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_uimm(false, d, ptr, uimm);
                            self.gp_commit(r, d);
                        }
                        LoadKind::V128 => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_q_uimm(d, ptr, uimm);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8x8S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(false, false, 0, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8x8U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(true, false, 0, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16x4S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(false, false, 1, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16x4U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(true, false, 1, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32x2S => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(false, false, 2, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32x2U => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.asm.xtl_128(true, false, 2, d, d);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load8Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldrb_uimm(SGP1, ptr, uimm);
                            self.asm.dup_gp_128(0, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load16Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldrh_uimm(SGP1, ptr, uimm);
                            self.asm.dup_gp_128(1, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_uimm(false, SGP1, ptr, uimm);
                            self.asm.dup_gp_128(2, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load64Splat => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_uimm(true, SGP1, ptr, uimm);
                            self.asm.dup_gp_128(3, d, SGP1);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load32Zero => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_s_uimm(d, ptr, uimm);
                            self.fp_commit(r, d);
                        }
                        LoadKind::V128Load64Zero => {
                            let d = self.fp_write_reg(r);
                            self.asm.ldr_d_uimm(d, ptr, uimm);
                            self.fp_commit(r, d);
                        }
                    }
                }
            }
            Op::LoadScaled {
                kind,
                addr,
                scale,
                offset,
            } => {
                let r = res.unwrap();
                let ar = self.gp_read(addr, SGP1);
                if offset == 0 && u64::from(scale) == kind.bytes() {
                    match kind {
                        LoadKind::I32 | LoadKind::I64Load32U => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_reg_scaled(false, d, MEM_BASE_REG, ar, true);
                            self.gp_commit(r, d);
                            return Ok(());
                        }
                        LoadKind::I64 => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldr_reg_scaled(true, d, MEM_BASE_REG, ar, true);
                            self.gp_commit(r, d);
                            return Ok(());
                        }
                        LoadKind::I64Load32S => {
                            let d = self.gp_write_reg(r);
                            self.asm.ldrsw_reg_scaled(d, MEM_BASE_REG, ar, true);
                            self.gp_commit(r, d);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                let (ptr, uimm) =
                    self.compute_scaled_mem_base_uimm(addr, ar, scale, offset, kind.bytes());
                match kind {
                    LoadKind::I32 | LoadKind::I64Load32U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64 => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::F32 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::F64 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::I32Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load8U | LoadKind::I64Load8U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrb_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16U | LoadKind::I64Load16U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrh_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load32S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsw_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::V128 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_q_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8x8S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 0, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8x8U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 0, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16x4S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 1, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16x4U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 1, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32x2S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 2, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32x2U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 2, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrb_uimm(SGP1, ptr, uimm);
                        self.asm.dup_gp_128(0, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrh_uimm(SGP1, ptr, uimm);
                        self.asm.dup_gp_128(1, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(false, SGP1, ptr, uimm);
                        self.asm.dup_gp_128(2, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(true, SGP1, ptr, uimm);
                        self.asm.dup_gp_128(3, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                }
            }
            Op::Store {
                kind,
                addr,
                val,
                offset,
                checked,
            } => {
                let ar = self.gp_read(addr, SGP1);
                let guard_ok =
                    self.layout.use_guard_pages && !self.layout.memory64 && offset < (1u64 << 32);
                if !checked && !guard_ok {
                    self.emit_bounds_check(ar, offset.saturating_add(kind.bytes()));
                }
                let uxtw = !self.layout.memory64;
                if offset == 0 {
                    // Preserve `ar` in `SGP2` if `addr` spilled into `SGP1` and `val` is also spilled GP.
                    let ar_safe = if ar == SGP1
                        && !matches!(kind, StoreKind::F32 | StoreKind::F64 | StoreKind::V128)
                    {
                        self.asm.mov_r(true, SGP2, ar);
                        SGP2
                    } else {
                        ar
                    };
                    match kind {
                        StoreKind::I8 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.strb_reg(vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::I16 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.strh_reg(vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::I32 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.str_reg(false, vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::I64 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.str_reg(true, vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::F32 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_s_reg(vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::F64 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_d_reg(vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                        StoreKind::V128 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_q_reg(vr, MEM_BASE_REG, ar_safe, uxtw);
                        }
                    }
                } else {
                    let (ptr, uimm) = self.compute_mem_base_uimm(ar, offset, kind.bytes());
                    // Preserve `ptr` in `SGP2` before `gp_read`/`fp_read`, since large spill
                    // offsets in `emit_load_off` clobber `SGP3`.
                    let ptr_safe = if ptr == SGP3 {
                        self.asm.mov_r(true, SGP2, ptr);
                        SGP2
                    } else {
                        ptr
                    };
                    match kind {
                        StoreKind::I8 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.strb_uimm(vr, ptr_safe, uimm);
                        }
                        StoreKind::I16 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.strh_uimm(vr, ptr_safe, uimm);
                        }
                        StoreKind::I32 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.str_uimm(false, vr, ptr_safe, uimm);
                        }
                        StoreKind::I64 => {
                            let vr = self.gp_read(val, SGP1);
                            self.asm.str_uimm(true, vr, ptr_safe, uimm);
                        }
                        StoreKind::F32 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_s_uimm(vr, ptr_safe, uimm);
                        }
                        StoreKind::F64 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_d_uimm(vr, ptr_safe, uimm);
                        }
                        StoreKind::V128 => {
                            let vr = self.fp_read(val, SFP1);
                            self.asm.str_q_uimm(vr, ptr_safe, uimm);
                        }
                    }
                }
            }
            Op::StoreScaled {
                kind,
                addr,
                val,
                scale,
                offset,
            } => {
                let ar = self.gp_read(addr, SGP1);
                if offset == 0 && u64::from(scale) == kind.bytes() {
                    match kind {
                        StoreKind::I32 | StoreKind::I64 => {
                            let ar_safe = if ar == SGP1 {
                                self.asm.mov_r(true, SGP2, ar);
                                SGP2
                            } else {
                                ar
                            };
                            let vr = self.gp_read(val, SGP1);
                            self.asm.str_reg_scaled(
                                kind == StoreKind::I64,
                                vr,
                                MEM_BASE_REG,
                                ar_safe,
                                true,
                            );
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                let (ptr, uimm) =
                    self.compute_scaled_mem_base_uimm(addr, ar, scale, offset, kind.bytes());
                // Preserve `ptr` in `SGP2` before `gp_read`/`fp_read`, since large spill
                // offsets in `emit_load_off` clobber `SGP3`.
                let ptr_safe = if ptr == SGP3 {
                    self.asm.mov_r(true, SGP2, ptr);
                    SGP2
                } else {
                    ptr
                };
                match kind {
                    StoreKind::I8 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strb_uimm(vr, ptr_safe, uimm);
                    }
                    StoreKind::I16 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strh_uimm(vr, ptr_safe, uimm);
                    }
                    StoreKind::I32 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(false, vr, ptr_safe, uimm);
                    }
                    StoreKind::I64 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(true, vr, ptr_safe, uimm);
                    }
                    StoreKind::F32 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_s_uimm(vr, ptr_safe, uimm);
                    }
                    StoreKind::F64 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_d_uimm(vr, ptr_safe, uimm);
                    }
                    StoreKind::V128 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_q_uimm(vr, ptr_safe, uimm);
                    }
                }
            }
            Op::V128LoadLane {
                width,
                addr,
                vec,
                offset,
                lane,
                checked,
            } => {
                let r = res.unwrap();
                let ar = self.gp_read(addr, SGP1);
                let guard_ok =
                    self.layout.use_guard_pages && !self.layout.memory64 && offset < (1u64 << 32);
                if !checked && !guard_ok {
                    self.emit_bounds_check(ar, offset.saturating_add(u64::from(width)));
                }
                let (ptr, uimm) = self.compute_mem_base_uimm(ar, offset, u64::from(width));
                let sz = match width {
                    1 => {
                        self.asm.ldrb_uimm(SGP1, ptr, uimm);
                        0u8
                    }
                    2 => {
                        self.asm.ldrh_uimm(SGP1, ptr, uimm);
                        1u8
                    }
                    4 => {
                        self.asm.ldr_uimm(false, SGP1, ptr, uimm);
                        2u8
                    }
                    8 => {
                        self.asm.ldr_uimm(true, SGP1, ptr, uimm);
                        3u8
                    }
                    _ => return Err(format!("aarch64: invalid V128LoadLane width {width}")),
                };
                let vr = self.fp_read(vec, SFP1);
                let d = self.fp_write_reg(r);
                self.asm.mov_16b(d, vr);
                self.asm.ins_gp(sz, d, lane, SGP1);
                self.fp_commit(r, d);
            }
            Op::V128StoreLane {
                width,
                addr,
                vec,
                offset,
                lane,
                checked,
            } => {
                let ar = self.gp_read(addr, SGP1);
                let guard_ok =
                    self.layout.use_guard_pages && !self.layout.memory64 && offset < (1u64 << 32);
                if !checked && !guard_ok {
                    self.emit_bounds_check(ar, offset.saturating_add(u64::from(width)));
                }
                let (ptr, uimm) = self.compute_mem_base_uimm(ar, offset, u64::from(width));
                // Preserve `ptr` in `SGP2` before `fp_read`, since large spill offsets clobber `SGP3`.
                let ptr_safe = if ptr == SGP3 {
                    self.asm.mov_r(true, SGP2, ptr);
                    SGP2
                } else {
                    ptr
                };
                let vr = self.fp_read(vec, SFP1);
                match width {
                    1 => {
                        self.asm.umov(0, SGP1, vr, lane);
                        self.asm.strb_uimm(SGP1, ptr_safe, uimm);
                    }
                    2 => {
                        self.asm.umov(1, SGP1, vr, lane);
                        self.asm.strh_uimm(SGP1, ptr_safe, uimm);
                    }
                    4 => {
                        self.asm.umov(2, SGP1, vr, lane);
                        self.asm.str_uimm(false, SGP1, ptr_safe, uimm);
                    }
                    8 => {
                        self.asm.umov(3, SGP1, vr, lane);
                        self.asm.str_uimm(true, SGP1, ptr_safe, uimm);
                    }
                    _ => return Err(format!("aarch64: invalid V128StoreLane width {width}")),
                }
            }
            Op::MemSize => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                self.asm.lsr_imm(true, d, MEM_LEN_REG, 16);
                self.gp_commit(r, d);
            }
            Op::MemGrow(delta) => {
                let r = res.unwrap();
                let dr = self.gp_read(delta, Reg::X0);
                if self.layout.memory64 {
                    self.asm.mov_r(true, Reg::X0, dr);
                } else {
                    self.asm.uxtw(Reg::X0, dr);
                }
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_MEM_GROW));
                self.gp_commit(r, Reg::X0);
            }
            Op::MemCopy { dst, src, len } => {
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        self.ty(dst),
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(src)),
                        self.ty(src),
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        self.ty(len),
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !self.layout.memory64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                    self.asm.uxtw(Reg::X1, Reg::X1);
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_MEM_COPY));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::MemFill { dst, val, len } => {
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        self.ty(dst),
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(val)),
                        self.ty(val),
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        self.ty(len),
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !self.layout.memory64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_MEM_FILL));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::LoadMem {
                mem,
                kind,
                addr,
                offset,
            } => {
                let r = res.unwrap();
                let desc_off = self.layout.mem_desc_off(mem);
                let is64 = self.layout.mem_is64(mem);
                let ar = self.gp_read(addr, SGP1);
                if is64 {
                    if ar != SGP1 {
                        self.asm.mov_r(true, SGP1, ar);
                    }
                } else {
                    self.asm.uxtw(SGP1, ar);
                }
                let oob = self.trap_oob_lbl();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                self.asm
                    .mov_imm64(SGP3, offset.saturating_add(kind.bytes()));
                self.asm.adds_r(true, SGP3, SGP1, SGP3);
                self.asm.b_cond(Cond::Hs, oob);
                self.asm.cmp_r(true, SGP3, SGP2);
                self.asm.b_cond(Cond::Hi, oob);
                self.emit_speculation_barrier();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, SGP1);
                if offset != 0 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP2, SGP2, SGP3);
                }
                let ptr = SGP2;
                let uimm = 0u32;
                match kind {
                    LoadKind::I32 => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64 => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::F32 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::F64 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::I32Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load8U | LoadKind::I64Load8U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrb_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16U | LoadKind::I64Load16U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrh_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(true, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load32S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsw_uimm(d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load32U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(false, d, ptr, uimm);
                        self.gp_commit(r, d);
                    }
                    LoadKind::V128 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_q_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8x8S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 0, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8x8U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 0, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16x4S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 1, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16x4U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 1, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32x2S => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(false, false, 2, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32x2U => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.asm.xtl_128(true, false, 2, d, d);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrb_uimm(SGP1, ptr, uimm);
                        self.asm.dup_gp_128(0, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrh_uimm(SGP1, ptr, uimm);
                        self.asm.dup_gp_128(1, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(false, SGP1, ptr, uimm);
                        self.asm.dup_gp_128(2, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(true, SGP1, ptr, uimm);
                        self.asm.dup_gp_128(3, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, uimm);
                        self.fp_commit(r, d);
                    }
                }
            }
            Op::StoreMem {
                mem,
                kind,
                addr,
                val,
                offset,
            } => {
                let desc_off = self.layout.mem_desc_off(mem);
                let is64 = self.layout.mem_is64(mem);
                let ar = self.gp_read(addr, SGP1);
                if is64 {
                    if ar != SGP1 {
                        self.asm.mov_r(true, SGP1, ar);
                    }
                } else {
                    self.asm.uxtw(SGP1, ar);
                }
                let oob = self.trap_oob_lbl();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                self.asm
                    .mov_imm64(SGP3, offset.saturating_add(kind.bytes()));
                self.asm.adds_r(true, SGP3, SGP1, SGP3);
                self.asm.b_cond(Cond::Hs, oob);
                self.asm.cmp_r(true, SGP3, SGP2);
                self.asm.b_cond(Cond::Hi, oob);
                self.emit_speculation_barrier();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, SGP1);
                if offset != 0 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP2, SGP2, SGP3);
                }
                match kind {
                    StoreKind::I8 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strb_uimm(vr, SGP2, 0);
                    }
                    StoreKind::I16 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strh_uimm(vr, SGP2, 0);
                    }
                    StoreKind::I32 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(false, vr, SGP2, 0);
                    }
                    StoreKind::I64 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(true, vr, SGP2, 0);
                    }
                    StoreKind::F32 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_s_uimm(vr, SGP2, 0);
                    }
                    StoreKind::F64 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_d_uimm(vr, SGP2, 0);
                    }
                    StoreKind::V128 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_q_uimm(vr, SGP2, 0);
                    }
                }
            }
            Op::V128LoadLaneMem {
                mem,
                width,
                addr,
                vec,
                offset,
                lane,
            } => {
                let r = res.unwrap();
                let desc_off = self.layout.mem_desc_off(mem);
                let is64 = self.layout.mem_is64(mem);
                let ar = self.gp_read(addr, SGP1);
                if is64 {
                    if ar != SGP1 {
                        self.asm.mov_r(true, SGP1, ar);
                    }
                } else {
                    self.asm.uxtw(SGP1, ar);
                }
                let oob = self.trap_oob_lbl();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                self.asm
                    .mov_imm64(SGP3, offset.saturating_add(u64::from(width)));
                self.asm.adds_r(true, SGP3, SGP1, SGP3);
                self.asm.b_cond(Cond::Hs, oob);
                self.asm.cmp_r(true, SGP3, SGP2);
                self.asm.b_cond(Cond::Hi, oob);
                self.emit_speculation_barrier();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, SGP1);
                if offset != 0 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP2, SGP2, SGP3);
                }
                let sz = match width {
                    1 => {
                        self.asm.ldrb_uimm(SGP1, SGP2, 0);
                        0u8
                    }
                    2 => {
                        self.asm.ldrh_uimm(SGP1, SGP2, 0);
                        1u8
                    }
                    4 => {
                        self.asm.ldr_uimm(false, SGP1, SGP2, 0);
                        2u8
                    }
                    8 => {
                        self.asm.ldr_uimm(true, SGP1, SGP2, 0);
                        3u8
                    }
                    _ => return Err(format!("aarch64: invalid V128LoadLaneMem width {width}")),
                };
                let vr = self.fp_read(vec, SFP1);
                let d = self.fp_write_reg(r);
                self.asm.mov_16b(d, vr);
                self.asm.ins_gp(sz, d, lane, SGP1);
                self.fp_commit(r, d);
            }
            Op::V128StoreLaneMem {
                mem,
                width,
                addr,
                vec,
                offset,
                lane,
            } => {
                let desc_off = self.layout.mem_desc_off(mem);
                let is64 = self.layout.mem_is64(mem);
                let ar = self.gp_read(addr, SGP1);
                if is64 {
                    if ar != SGP1 {
                        self.asm.mov_r(true, SGP1, ar);
                    }
                } else {
                    self.asm.uxtw(SGP1, ar);
                }
                let oob = self.trap_oob_lbl();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                self.asm
                    .mov_imm64(SGP3, offset.saturating_add(u64::from(width)));
                self.asm.adds_r(true, SGP3, SGP1, SGP3);
                self.asm.b_cond(Cond::Hs, oob);
                self.asm.cmp_r(true, SGP3, SGP2);
                self.asm.b_cond(Cond::Hi, oob);
                self.emit_speculation_barrier();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    desc_off,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, SGP1);
                if offset != 0 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP2, SGP2, SGP3);
                }
                let vr = self.fp_read(vec, SFP1);
                match width {
                    1 => {
                        self.asm.umov(0, SGP1, vr, lane);
                        self.asm.strb_uimm(SGP1, SGP2, 0);
                    }
                    2 => {
                        self.asm.umov(1, SGP1, vr, lane);
                        self.asm.strh_uimm(SGP1, SGP2, 0);
                    }
                    4 => {
                        self.asm.umov(2, SGP1, vr, lane);
                        self.asm.str_uimm(false, SGP1, SGP2, 0);
                    }
                    8 => {
                        self.asm.umov(3, SGP1, vr, lane);
                        self.asm.str_uimm(true, SGP1, SGP2, 0);
                    }
                    _ => return Err(format!("aarch64: invalid V128StoreLaneMem width {width}")),
                }
            }
            Op::MemSizeIdx(mem) => {
                let r = res.unwrap();
                let desc_off = self.layout.mem_desc_off(mem);
                let d = self.gp_write_reg(r);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(d),
                    None,
                    CTX_REG,
                    desc_off + 8,
                    SGP3,
                );
                self.asm.lsr_imm(true, d, d, 16);
                self.gp_commit(r, d);
            }
            Op::MemGrowIdx { mem, delta } => {
                let r = res.unwrap();
                let desc_off = self.layout.mem_desc_off(mem);
                let is64 = self.layout.mem_is64(mem);
                let dr = self.gp_read(delta, Reg::X0);
                if is64 {
                    self.asm.mov_r(true, Reg::X0, dr);
                } else {
                    self.asm.uxtw(Reg::X0, dr);
                }
                self.asm.mov_imm32(Reg::X1, desc_off as u32);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_EXTRA_MEM_GROW));
                if !is64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                self.gp_commit(r, Reg::X0);
            }
            Op::MemCopyIdx {
                dst_mem,
                src_mem,
                dst,
                src,
                len,
            } => {
                let d64 = self.layout.mem_is64(dst_mem);
                let s64 = self.layout.mem_is64(src_mem);
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        self.ty(dst),
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(src)),
                        self.ty(src),
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        self.ty(len),
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !d64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                if !s64 {
                    self.asm.uxtw(Reg::X1, Reg::X1);
                }
                if !(d64 && s64) {
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm
                    .mov_imm32(Reg::X3, self.layout.mem_desc_off(dst_mem) as u32);
                self.asm
                    .mov_imm32(Reg::X4, self.layout.mem_desc_off(src_mem) as u32);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_EXTRA_MEM_COPY));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::MemFillIdx { mem, dst, val, len } => {
                let is64 = self.layout.mem_is64(mem);
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        self.ty(dst),
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(val)),
                        self.ty(val),
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        self.ty(len),
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !is64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm
                    .mov_imm32(Reg::X3, self.layout.mem_desc_off(mem) as u32);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_EXTRA_MEM_FILL));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::MemInit {
                data,
                mem,
                dst,
                src,
                len,
            } => {
                let d64 = self.layout.mem_is64(mem);
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        self.ty(dst),
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(src)),
                        self.ty(src),
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        self.ty(len),
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !d64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                self.asm.uxtw(Reg::X1, Reg::X1);
                self.asm.uxtw(Reg::X2, Reg::X2);
                self.asm.mov_imm32(Reg::X3, data);
                self.asm
                    .mov_imm32(Reg::X4, self.layout.mem_desc_off(mem) as u32);
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_MEM_INIT));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::DataDrop(data) => {
                let off = self.layout.data_dropped_off.unwrap() + (data as i32);
                self.asm.add_i32_checked(true, SGP2, CTX_REG, off, SGP3);
                self.asm.mov_imm32(SGP1, 1);
                self.asm.strb_uimm(SGP1, SGP2, 0);
            }
            Op::TableInit {
                elem,
                table,
                dst,
                src,
                len,
            } => {
                let d64 = self.layout.table_is_64(table);
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        if d64 { Ty::I64 } else { Ty::I32 },
                    ),
                    (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !d64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                self.asm.uxtw(Reg::X1, Reg::X1);
                self.asm.uxtw(Reg::X2, Reg::X2);
                self.asm.mov_imm32(Reg::X3, elem);
                self.asm
                    .mov_imm32(Reg::X4, self.layout.dyn_tables[&table].0 as u32);
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_TABLE_INIT));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::ElemDrop(elem) => {
                let off = self.layout.elem_dropped_off.unwrap() + (elem as i32);
                self.asm.add_i32_checked(true, SGP2, CTX_REG, off, SGP3);
                self.asm.mov_imm32(SGP1, 1);
                self.asm.strb_uimm(SGP1, SGP2, 0);
            }
            Op::TableCopy {
                dst_table,
                src_table,
                dst,
                src,
                len,
            } => {
                let d64 = self.layout.table_is_64(dst_table);
                let s64 = self.layout.table_is_64(src_table);
                let l64 = d64 && s64;
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(dst)),
                        if d64 { Ty::I64 } else { Ty::I32 },
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(src)),
                        if s64 { Ty::I64 } else { Ty::I32 },
                    ),
                    (
                        MoveLoc::Gp(Reg::X2),
                        MoveLoc::from(self.loc(len)),
                        if l64 { Ty::I64 } else { Ty::I32 },
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !d64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                if !s64 {
                    self.asm.uxtw(Reg::X1, Reg::X1);
                }
                if !l64 {
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm
                    .mov_imm32(Reg::X3, self.layout.dyn_tables[&dst_table].0 as u32);
                self.asm
                    .mov_imm32(Reg::X4, self.layout.dyn_tables[&src_table].0 as u32);
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_TABLE_COPY));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::TableGrow { table, init, delta } => {
                let r = res.unwrap();
                let t64 = self.layout.table_is_64(table);
                let moves = [
                    (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(init)), Ty::I64),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(delta)),
                        if t64 { Ty::I64 } else { Ty::I32 },
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !t64 {
                    self.asm.uxtw(Reg::X1, Reg::X1);
                }
                self.asm
                    .mov_imm32(Reg::X2, self.layout.dyn_tables[&table].0 as u32);
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_TABLE_GROW));
                if !t64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                }
                self.gp_commit(r, Reg::X0);
            }
            Op::TableSize(table) => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                if let Some(&(desc_off, _, _)) = self.layout.dyn_tables.get(&table) {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I32,
                        Some(d),
                        None,
                        CTX_REG,
                        desc_off + 8,
                        SGP3,
                    );
                } else {
                    self.asm
                        .mov_imm32(d, self.layout.table_lens[table as usize]);
                }
                self.gp_commit(r, d);
            }
            Op::TableFill {
                table,
                dst,
                val,
                len,
            } => {
                let t64 = self.layout.table_is_64(table);
                let ity = if t64 { Ty::I64 } else { Ty::I32 };
                let moves = [
                    (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(dst)), ity),
                    (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(val)), Ty::I64),
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(len)), ity),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if !t64 {
                    self.asm.uxtw(Reg::X0, Reg::X0);
                    self.asm.uxtw(Reg::X2, Reg::X2);
                }
                self.asm
                    .mov_imm32(Reg::X3, self.layout.dyn_tables[&table].0 as u32);
                self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_TABLE_FILL));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::MemCopyConst {
                dst,
                src,
                len,
                dst_checked,
                src_checked,
            } => {
                if !dst_checked {
                    let dr = self.gp_read(dst, SGP1);
                    self.emit_bounds_check(dr, u64::from(len));
                }
                if !src_checked {
                    let sr = self.gp_read(src, SGP1);
                    self.emit_bounds_check(sr, u64::from(len));
                }
                if len > 0 {
                    // Overlap-safe wide NEON 16B / 8B / 4B / 2B / 1B inline copy (`len <= 64`).
                    let dr = self.gp_read(dst, SGP1);
                    if !self.layout.memory64 {
                        self.asm.add_ext_uxtw(SGP1, MEM_BASE_REG, dr);
                    } else {
                        self.asm.add_r(true, SGP1, MEM_BASE_REG, dr);
                    }
                    let sr = self.gp_read(src, SGP2);
                    if !self.layout.memory64 {
                        self.asm.add_ext_uxtw(SGP2, MEM_BASE_REG, sr);
                    } else {
                        self.asm.add_r(true, SGP2, MEM_BASE_REG, sr);
                    }
                    let lbl_bwd = self.asm.new_label();
                    let lbl_done = self.asm.new_label();
                    self.asm.cmp_r(true, SGP1, SGP2);
                    self.asm.b_cond(Cond::Hi, lbl_bwd);
                    // Forward: naturally aligned 16B -> 8B -> 4B -> 2B -> 1B chunks.
                    let mut off = 0u32;
                    while off + 16 <= len {
                        self.asm.ldr_q_uimm(SFP1, SGP2, off);
                        self.asm.str_q_uimm(SFP1, SGP1, off);
                        off += 16;
                    }
                    if off + 8 <= len {
                        self.asm.ldr_uimm(true, SGP3, SGP2, off);
                        self.asm.str_uimm(true, SGP3, SGP1, off);
                        off += 8;
                    }
                    if off + 4 <= len {
                        self.asm.ldr_uimm(false, SGP3, SGP2, off);
                        self.asm.str_uimm(false, SGP3, SGP1, off);
                        off += 4;
                    }
                    if off + 2 <= len {
                        self.asm.ldrh_uimm(SGP3, SGP2, off);
                        self.asm.strh_uimm(SGP3, SGP1, off);
                        off += 2;
                    }
                    if off < len {
                        self.asm.ldrb_uimm(SGP3, SGP2, off);
                        self.asm.strb_uimm(SGP3, SGP1, off);
                    }
                    self.asm.b(lbl_done);
                    // Backward: peel low bits of `len` from high to low so every offset stays aligned!
                    self.asm.bind(lbl_bwd);
                    let mut cur = len;
                    if (cur & 1) != 0 {
                        cur -= 1;
                        self.asm.ldrb_uimm(SGP3, SGP2, cur);
                        self.asm.strb_uimm(SGP3, SGP1, cur);
                    }
                    if (cur & 2) != 0 {
                        cur -= 2;
                        self.asm.ldrh_uimm(SGP3, SGP2, cur);
                        self.asm.strh_uimm(SGP3, SGP1, cur);
                    }
                    if (cur & 4) != 0 {
                        cur -= 4;
                        self.asm.ldr_uimm(false, SGP3, SGP2, cur);
                        self.asm.str_uimm(false, SGP3, SGP1, cur);
                    }
                    if (cur & 8) != 0 {
                        cur -= 8;
                        self.asm.ldr_uimm(true, SGP3, SGP2, cur);
                        self.asm.str_uimm(true, SGP3, SGP1, cur);
                    }
                    while cur >= 16 {
                        cur -= 16;
                        self.asm.ldr_q_uimm(SFP1, SGP2, cur);
                        self.asm.str_q_uimm(SFP1, SGP1, cur);
                    }
                    self.asm.bind(lbl_done);
                }
            }
            Op::MemFillConst {
                dst,
                val,
                len,
                checked,
            } => {
                if !checked {
                    let dr = self.gp_read(dst, SGP1);
                    self.emit_bounds_check(dr, u64::from(len));
                }
                if len > 0 {
                    let dr = self.gp_read(dst, SGP1);
                    if !self.layout.memory64 {
                        self.asm.add_ext_uxtw(SGP3, MEM_BASE_REG, dr);
                    } else {
                        self.asm.add_r(true, SGP3, MEM_BASE_REG, dr);
                    }
                    let is_zero = matches!(self.loc(val), A64Loc::Remat(A64RematConst::I32(0)));
                    let vr = if is_zero {
                        Reg::XZR
                    } else {
                        self.gp_read(val, SGP1)
                    };
                    if len >= 4 {
                        if is_zero {
                            self.asm.movi_zero_16b(SFP1);
                        } else {
                            self.asm.dup_gp_128(0, SFP1, vr);
                        }
                        let mut off = 0u32;
                        while off + 16 <= len {
                            self.asm.str_q_uimm(SFP1, SGP3, off);
                            off += 16;
                        }
                        if off + 8 <= len {
                            self.asm.str_d_uimm(SFP1, SGP3, off);
                            off += 8;
                        }
                        if off + 4 <= len {
                            self.asm.str_s_uimm(SFP1, SGP3, off);
                            off += 4;
                        }
                        while off < len {
                            self.asm.strb_uimm(vr, SGP3, off);
                            off += 1;
                        }
                    } else {
                        for k in 0..len {
                            self.asm.strb_uimm(vr, SGP3, k);
                        }
                    }
                }
            }
            Op::MemByteSum {
                base,
                count,
                offset,
            } => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                let br = self.gp_read(base, SGP1);
                self.asm.uxtw(SGP1, br);
                let cr = self.gp_read(count, SGP2);
                self.asm.uxtw(SGP2, cr);
                let lbl_zero = self.asm.new_label();
                let lbl_loop = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                self.asm.cbz(false, SGP2, lbl_zero);
                if offset != 0 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP1, SGP1, SGP3);
                }
                self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1); // cur
                self.asm.add_r(true, SGP2, SGP1, SGP2); // end
                self.asm.movi_zero_16b(SFP1);
                self.asm.bind(lbl_loop);
                self.asm.ldr_q_uimm(SFP2, SGP1, 0);
                self.asm.uaddlp_128(0, SFP2, SFP2);
                self.asm.uaddlp_128(1, SFP2, SFP2);
                self.asm.uaddlp_128(2, SFP2, SFP2);
                self.asm.add_v128(3, SFP1, SFP1, SFP2);
                self.asm.add_imm(true, SGP1, SGP1, 16);
                self.asm.cmp_r(true, SGP1, SGP2);
                self.asm.b_cond(Cond::Lo, lbl_loop);
                self.asm.umov(2, d, SFP1, 0);
                self.asm.umov(2, SGP3, SFP1, 2);
                self.asm.add_r(false, d, d, SGP3);
                self.asm.b(lbl_done);
                self.asm.bind(lbl_zero);
                self.asm.mov_imm32(d, 0);
                self.asm.bind(lbl_done);
                self.gp_commit(r, d);
            }
            Op::MemByteBlur3Div {
                base,
                count,
                src_off,
                dst_off,
                divisor,
            } => {
                let br = self.gp_read(base, SGP1);
                self.asm.uxtw(SGP1, br);
                let cr = self.gp_read(count, SGP2);
                self.asm.uxtw(SGP2, cr);
                let lbl_done = self.asm.new_label();
                self.asm.cbz(false, SGP2, lbl_done);
                // Stash `count` in SFP1.d0 while building `src_ptr` (SGP1) and `dst_ptr` (SGP3).
                self.asm.fmov_fp_from_gp(true, SFP1, SGP2);
                self.asm.add_r(true, SGP3, MEM_BASE_REG, SGP1);
                if dst_off != 0 {
                    self.asm.mov_imm64(SGP2, dst_off);
                    self.asm.add_r(true, SGP3, SGP3, SGP2);
                }
                self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1);
                if src_off != 0 {
                    self.asm.mov_imm64(SGP2, src_off);
                    self.asm.add_r(true, SGP1, SGP1, SGP2);
                }
                // `SFP1.d0 = end_src = src_ptr + count`, `SFP2.d0 = dst_ptr`.
                self.asm.fmov_gp_from_fp(true, SGP2, SFP1);
                self.asm.add_r(true, SGP2, SGP1, SGP2);
                self.asm.fmov_fp_from_gp(true, SFP1, SGP2);
                self.asm.fmov_fp_from_gp(true, SFP2, SGP3);
                let is_pow2 = divisor.is_power_of_two();
                let (magic16, shift16) = if is_pow2 {
                    (0u32, divisor.trailing_zeros() as u8)
                } else {
                    let (m, sh, _) = crate::x64::lower::magic_u(16, u64::from(divisor));
                    (m as u32, sh as u8)
                };
                let lbl_loop = self.asm.new_label();
                self.asm.bind(lbl_loop);
                self.asm.ldrb_uimm(SGP2, SGP1, 0);
                self.asm.ldrb_uimm(SGP3, SGP1, 1);
                self.asm.add_r(false, SGP2, SGP2, SGP3);
                self.asm.ldrb_uimm(SGP3, SGP1, 2);
                self.asm.add_r(false, SGP2, SGP2, SGP3);
                if !is_pow2 {
                    self.asm.mov_imm32(SGP3, magic16);
                    self.asm.mul(false, SGP2, SGP2, SGP3);
                    self.asm.lsr_imm(false, SGP2, SGP2, 16 + shift16);
                } else if shift16 > 0 {
                    self.asm.lsr_imm(false, SGP2, SGP2, shift16);
                }
                self.asm.fmov_gp_from_fp(true, SGP3, SFP2);
                self.asm.strb_uimm(SGP2, SGP3, 0);
                self.asm.add_imm(true, SGP3, SGP3, 1);
                self.asm.fmov_fp_from_gp(true, SFP2, SGP3);
                self.asm.add_imm(true, SGP1, SGP1, 1);
                self.asm.fmov_gp_from_fp(true, SGP3, SFP1);
                self.asm.cmp_r(true, SGP1, SGP3);
                self.asm.b_cond(Cond::Lo, lbl_loop);
                self.asm.bind(lbl_done);
            }
            Op::F64AxpyLoop {
                dst,
                src,
                count,
                scale,
                is_sub,
            } => {
                // `SGP1 = dst_ptr`, `SGP2 = src_ptr`, `SGP3 = count` (elements).
                // Save `V0` into `[CTX_REG + OFF_SCRATCH_V128]` to broadcast `[scale, scale]`
                // with zero regalloc clobbers!
                let sr = self.fp_read(scale, SFP2);
                if sr != SFP2 {
                    self.asm.fmov_fp(true, SFP2, sr);
                }
                let dr = self.gp_read(dst, SGP1);
                self.asm.uxtw(SGP1, dr);
                let sr_ptr = self.gp_read(src, SGP2);
                self.asm.uxtw(SGP2, sr_ptr);
                let cr = self.gp_read(count, SGP3);
                self.asm.uxtw(SGP3, cr);

                let lbl_done = self.asm.new_label();
                let lbl_restore = self.asm.new_label();
                self.asm.cbz(true, SGP3, lbl_done);

                self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1);
                self.asm.add_r(true, SGP2, MEM_BASE_REG, SGP2);

                self.asm
                    .str_q_uimm(VReg::V0, CTX_REG, OFF_SCRATCH_V128 as u32);
                self.asm.dup_elt_128(3, VReg::V0, SFP2, 0);

                let lbl_vec2 = self.asm.new_label();
                let lbl_tail = self.asm.new_label();
                self.asm.cmp_imm(true, SGP3, 2);
                self.asm.b_cond(Cond::Lo, lbl_tail);

                self.asm.bind(lbl_vec2);
                self.asm.ldr_q_uimm(SFP1, SGP2, 0);
                self.asm.fmul_v128(true, SFP1, SFP1, VReg::V0);
                self.asm.ldr_q_uimm(SFP2, SGP1, 0);
                if !is_sub {
                    self.asm.fadd_v128(true, SFP2, SFP2, SFP1);
                } else {
                    self.asm.fsub_v128(true, SFP2, SFP2, SFP1);
                }
                self.asm.str_q_uimm(SFP2, SGP1, 0);
                self.asm.add_imm(true, SGP1, SGP1, 16);
                self.asm.add_imm(true, SGP2, SGP2, 16);
                self.asm.sub_imm(true, SGP3, SGP3, 2);
                self.asm.cmp_imm(true, SGP3, 2);
                self.asm.b_cond(Cond::Hs, lbl_vec2);

                self.asm.bind(lbl_tail);
                self.asm.cbz(true, SGP3, lbl_restore);
                self.asm.ldr_d_uimm(SFP1, SGP2, 0);
                self.asm.fmul(true, SFP1, SFP1, VReg::V0);
                self.asm.ldr_d_uimm(SFP2, SGP1, 0);
                if !is_sub {
                    self.asm.fadd(true, SFP2, SFP2, SFP1);
                } else {
                    self.asm.fsub(true, SFP2, SFP2, SFP1);
                }
                self.asm.str_d_uimm(SFP2, SGP1, 0);

                self.asm.bind(lbl_restore);
                self.asm
                    .ldr_q_uimm(VReg::V0, CTX_REG, OFF_SCRATCH_V128 as u32);
                self.asm.bind(lbl_done);
            }
            Op::F64x4BinMem {
                op,
                dst,
                dst_off,
                lhs,
                lhs_off,
                rhs,
                rhs_off,
                rhs_is_splat,
            } => {
                let emit_ptr = |this: &mut Self, addr_v: Val, off: u64, out_reg: Reg| {
                    let ar = this.gp_read(addr_v, out_reg);
                    if this.ir.memory64 {
                        if ar != out_reg {
                            this.asm.mov_r(true, out_reg, ar);
                        }
                    } else {
                        this.asm.uxtw(out_reg, ar);
                    }
                    this.asm.add_r(true, out_reg, MEM_BASE_REG, out_reg);
                    if off != 0 {
                        this.asm.mov_imm64(SGP3, off);
                        this.asm.add_r(true, out_reg, out_reg, SGP3);
                    }
                };
                let emit_fop = |asm: &mut A64Asm, vd: VReg, vn: VReg, vm: VReg| match op {
                    FloatBinOp::Add => asm.fadd_v128(true, vd, vn, vm),
                    FloatBinOp::Sub => asm.fsub_v128(true, vd, vn, vm),
                    FloatBinOp::Mul => asm.fmul_v128(true, vd, vn, vm),
                    FloatBinOp::Div => asm.fdiv_v128(true, vd, vn, vm),
                    _ => unreachable!(),
                };
                // Compute both 16-byte halves BEFORE writing `dst` so partial overlap is alias-safe.
                if rhs_is_splat {
                    let rr = self.fp_read(rhs, SFP2);
                    self.asm.dup_elt_128(3, SFP2, rr, 0);
                    emit_ptr(self, lhs, lhs_off, SGP1);
                    self.asm.ldr_q_uimm(SFP1, SGP1, 0);
                    emit_fop(&mut self.asm, SFP1, SFP1, SFP2);
                    self.asm.str_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                    self.asm.ldr_q_uimm(SFP1, SGP1, 16);
                    emit_fop(&mut self.asm, SFP1, SFP1, SFP2);
                } else {
                    emit_ptr(self, lhs, lhs_off, SGP1);
                    emit_ptr(self, rhs, rhs_off, SGP2);
                    self.asm.ldr_q_uimm(SFP1, SGP1, 0);
                    self.asm.ldr_q_uimm(SFP2, SGP2, 0);
                    emit_fop(&mut self.asm, SFP1, SFP1, SFP2);
                    self.asm.str_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                    self.asm.ldr_q_uimm(SFP1, SGP1, 16);
                    self.asm.ldr_q_uimm(SFP2, SGP2, 16);
                    emit_fop(&mut self.asm, SFP1, SFP1, SFP2);
                }
                self.asm.ldr_q_uimm(SFP2, CTX_REG, OFF_SCRATCH_V128 as u32);
                emit_ptr(self, dst, dst_off, SGP1);
                self.asm.str_q_uimm(SFP2, SGP1, 0);
                self.asm.str_q_uimm(SFP1, SGP1, 16);
            }
            Op::V128Splat { kind, val } => {
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                match kind {
                    SimdSplatKind::I8x16 => {
                        let v = self.gp_read(val, SGP1);
                        self.asm.dup_gp_128(0, d, v);
                    }
                    SimdSplatKind::I16x8 => {
                        let v = self.gp_read(val, SGP1);
                        self.asm.dup_gp_128(1, d, v);
                    }
                    SimdSplatKind::I32x4 => {
                        let v = self.gp_read(val, SGP1);
                        self.asm.dup_gp_128(2, d, v);
                    }
                    SimdSplatKind::I64x2 => {
                        let v = self.gp_read(val, SGP1);
                        self.asm.dup_gp_128(3, d, v);
                    }
                    SimdSplatKind::F32x4 => {
                        let v = self.fp_read(val, SFP1);
                        self.asm.dup_elt_128(2, d, v, 0);
                    }
                    SimdSplatKind::F64x2 => {
                        let v = self.fp_read(val, SFP1);
                        self.asm.dup_elt_128(3, d, v, 0);
                    }
                }
                self.fp_commit(r, d);
            }
            Op::V128ExtractLane { kind, v, lane } => {
                let r = res.unwrap();
                let vr = self.fp_read(v, SFP1);
                match kind {
                    SimdLaneKind::I8x16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.smov(false, 0, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::I8x16U => {
                        let d = self.gp_write_reg(r);
                        self.asm.umov(0, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::I16x8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.smov(false, 1, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::I16x8U => {
                        let d = self.gp_write_reg(r);
                        self.asm.umov(1, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::I32x4 => {
                        let d = self.gp_write_reg(r);
                        self.asm.umov(2, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::I64x2 => {
                        let d = self.gp_write_reg(r);
                        self.asm.umov(3, d, vr, lane);
                        self.gp_commit(r, d);
                    }
                    SimdLaneKind::F32x4 => {
                        let d = self.fp_write_reg(r);
                        if lane == 0 {
                            self.asm.fmov_fp(false, d, vr);
                        } else {
                            self.asm.ins_elt(2, d, 0, vr, lane);
                        }
                        self.fp_commit(r, d);
                    }
                    SimdLaneKind::F64x2 => {
                        let d = self.fp_write_reg(r);
                        if lane == 0 {
                            self.asm.fmov_fp(true, d, vr);
                        } else {
                            self.asm.ins_elt(3, d, 0, vr, lane);
                        }
                        self.fp_commit(r, d);
                    }
                }
            }
            Op::V128ReplaceLane { kind, v, x, lane } => {
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                let vr = self.fp_read(v, SFP1);
                match kind {
                    SimdLaneKind::I8x16S | SimdLaneKind::I8x16U => {
                        let xr = self.gp_read(x, SGP1);
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_gp(0, d, lane, xr);
                    }
                    SimdLaneKind::I16x8S | SimdLaneKind::I16x8U => {
                        let xr = self.gp_read(x, SGP1);
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_gp(1, d, lane, xr);
                    }
                    SimdLaneKind::I32x4 => {
                        let xr = self.gp_read(x, SGP1);
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_gp(2, d, lane, xr);
                    }
                    SimdLaneKind::I64x2 => {
                        let xr = self.gp_read(x, SGP1);
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_gp(3, d, lane, xr);
                    }
                    SimdLaneKind::F32x4 => {
                        let xr = self.fp_read(x, SFP2);
                        let src_x = if d == xr {
                            self.asm.fmov_fp(false, SFP2, xr);
                            SFP2
                        } else {
                            xr
                        };
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_elt(2, d, lane, src_x, 0);
                    }
                    SimdLaneKind::F64x2 => {
                        let xr = self.fp_read(x, SFP2);
                        let src_x = if d == xr {
                            self.asm.fmov_fp(true, SFP2, xr);
                            SFP2
                        } else {
                            xr
                        };
                        self.asm.mov_16b(d, vr);
                        self.asm.ins_elt(3, d, lane, src_x, 0);
                    }
                }
                self.fp_commit(r, d);
            }
            Op::V128Shuffle { a, b, lanes_idx } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let d = self.fp_write_reg(r);
                let lanes = self.ir.v128_consts[lanes_idx as usize];
                emit_a64_v128_shuffle(&mut self.asm, d, ar, br, &lanes);
                self.fp_commit(r, d);
            }
            Op::V128ToI32 { op: top, a } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.gp_write_reg(r);
                emit_a64_v128_to_i32(&mut self.asm, top, d, ar)?;
                self.gp_commit(r, d);
            }
            Op::V128Un { op: uop, a } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.fp_write_reg(r);
                emit_a64_v128_un(&mut self.asm, uop, d, ar)?;
                self.fp_commit(r, d);
            }
            Op::V128Bin { op: bop, a, b } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let d = self.fp_write_reg(r);
                emit_a64_v128_bin(&mut self.asm, bop, d, ar, br)?;
                self.fp_commit(r, d);
            }
            Op::V128Shift { op: sop, v, shift } => {
                let r = res.unwrap();
                let vr = self.fp_read(v, SFP1);
                let sr = self.gp_read(shift, SGP1);
                let d = self.fp_write_reg(r);
                emit_a64_v128_shift(&mut self.asm, sop, d, vr, sr)?;
                self.fp_commit(r, d);
            }
            Op::V128Ternary {
                op: 0xfd52,
                a,
                b,
                c,
            } => {
                // v128.bitselect(a, b, c) = (a & c) | (b & ~c) = bsl(c, a, b)
                let r = res.unwrap();
                let d = self.fp_write_reg(r);
                if d != SFP1 && d != SFP2 {
                    // `d` is a real FP register: save `a`/`b` out of `d` BEFORE placing `c` into `d`.
                    let ar = self.fp_read(a, SFP1);
                    let a_use = if ar == d {
                        self.asm.mov_16b(SFP1, ar);
                        SFP1
                    } else {
                        ar
                    };
                    let br = self.fp_read(b, SFP2);
                    let b_use = if br == d {
                        self.asm.mov_16b(SFP2, br);
                        SFP2
                    } else {
                        br
                    };
                    match self.loc(c) {
                        A64Loc::Fp(v) => {
                            if v != d {
                                self.asm.mov_16b(d, v);
                            }
                        }
                        A64Loc::Spill(off) => {
                            emit_load_off(
                                &mut self.asm,
                                Ty::V128,
                                None,
                                Some(d),
                                Reg::SP,
                                off,
                                SGP3,
                            );
                        }
                        A64Loc::Gp(_) | A64Loc::Remat(_) => unreachable!(),
                    }
                    self.asm.bsl_16b(d, a_use, b_use);
                } else {
                    // Spilled destination (`d == SFP1`): evaluate `(a & c) | (b & ~c)` using only
                    // `SFP1`, `SFP2`, and `[CTX_REG + OFF_SCRATCH_V128]` so arbitrary spill/alias
                    // combinations never overwrite an operand or live FP register.
                    let ar = self.fp_read(a, SFP1);
                    let cr = self.fp_read(c, SFP2);
                    self.asm.and_16b(SFP1, ar, cr);
                    self.asm
                        .str_q_uimm(SFP1, CTX_REG, crate::x64::OFF_SCRATCH_V128 as u32);
                    let br = self.fp_read(b, SFP1);
                    self.asm.bic_16b(SFP1, br, cr);
                    self.asm
                        .ldr_q_uimm(SFP2, CTX_REG, crate::x64::OFF_SCRATCH_V128 as u32);
                    self.asm.orr_16b(d, SFP1, SFP2);
                }
                self.fp_commit(r, d);
            }
            Op::Call { func, args, rets } => self.emit_call_abi(args, rets, None, Some(func))?,
            Op::CallIndirect {
                ci_id,
                idx,
                args,
                rets,
            } => self.emit_call_abi(args, rets, Some((ci_id, idx)), None)?,
            Op::ExnAlloc { tag, args } => {
                let r = res.unwrap();
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                let (ref fields, rec_size) = self.layout.tag_layouts[tag as usize];
                let lbl_oom = self.asm.new_label();
                let lbl_ok = self.asm.new_label();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    exn_off + 16,
                    SGP3,
                );
                self.asm.mov_imm32(SGP3, rec_size);
                self.asm.add_r(true, SGP3, SGP1, SGP3);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    exn_off + 24,
                    SGP2,
                );
                self.asm.cmp_r(true, SGP3, SGP2);
                self.asm.b_cond(Cond::Hi, lbl_oom);
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP3),
                    None,
                    CTX_REG,
                    exn_off + 16,
                    SGP2,
                );
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP2,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    exn_off + 8,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, SGP1); // SGP2 = record_ptr
                self.asm.mov_imm32(SGP1, tag);
                self.asm.str_uimm(false, SGP1, SGP2, 0);
                self.asm.str_uimm(false, Reg::XZR, SGP2, 4);
                let arg_vals: Vec<Val> = self.ir.pool_slice(args).to_vec();
                let field_specs: Vec<(Ty, i32)> = fields.clone();
                for (av, (ty, foff)) in arg_vals.into_iter().zip(field_specs) {
                    if ty.is_fp() {
                        let vr = self.fp_read(av, SFP1);
                        emit_store_off(&mut self.asm, ty, None, Some(vr), SGP2, foff, SGP3);
                    } else {
                        let gr = self.gp_read(av, SGP1);
                        emit_store_off(&mut self.asm, ty, Some(gr), None, SGP2, foff, SGP3);
                    }
                }
                let d = self.gp_write_reg(r);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(d),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.gp_commit(r, d);
                self.asm.b(lbl_ok);
                self.asm.bind(lbl_oom);
                self.asm
                    .mov_imm32(SGP1, u32::from(crate::x64::TRAP_ALLOC_FAILED));
                let ep = self.epilogue_trap_lbl();
                self.asm.b(ep);
                self.asm.bind(lbl_ok);
            }
            Op::ExnSetActive(h) => {
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                let hr = self.gp_read(h, SGP1);
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(hr),
                    None,
                    CTX_REG,
                    exn_off,
                    SGP3,
                );
            }
            Op::ExnGetActive => {
                let r = res.unwrap();
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                let d = self.gp_write_reg(r);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(d),
                    None,
                    CTX_REG,
                    exn_off,
                    SGP3,
                );
                self.gp_commit(r, d);
            }
            Op::ExnClearActive => {
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::XZR),
                    None,
                    CTX_REG,
                    exn_off,
                    SGP3,
                );
            }
            Op::ExnGetTag(h) => {
                let r = res.unwrap();
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                let hr = self.gp_read(h, SGP1);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    exn_off + 8,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, hr);
                let d = self.gp_write_reg(r);
                self.asm.ldr_uimm(false, d, SGP2, 0);
                self.gp_commit(r, d);
            }
            Op::ExnGetPayload {
                handle,
                tag,
                field,
                ty,
            } => {
                let r = res.unwrap();
                let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                let foff = self.layout.tag_layouts[tag as usize].0[field as usize].1;
                let hr = self.gp_read(handle, SGP1);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    exn_off + 8,
                    SGP3,
                );
                self.asm.add_r(true, SGP2, SGP2, hr);
                if ty.is_fp() {
                    let d = self.fp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, None, Some(d), SGP2, foff, SGP3);
                    self.fp_commit(r, d);
                } else {
                    let d = self.gp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, Some(d), None, SGP2, foff, SGP3);
                    self.gp_commit(r, d);
                }
            }
            Op::Trap(code) => {
                let lbl = match code {
                    TRAP_UNREACHABLE => self.trap_unreach_lbl(),
                    TRAP_OOB => self.trap_oob_lbl(),
                    TRAP_TABLE_OOB => self.trap_table_oob_lbl(),
                    _ => {
                        self.asm.mov_imm32(SGP1, u32::from(code));
                        let ep = self.epilogue_trap_lbl();
                        self.asm.b(ep);
                        return Ok(());
                    }
                };
                self.asm.b(lbl);
            }
            other => {
                return Err(format!("aarch64: unsupported IR op {other:?}"));
            }
        }
        Ok(())
    }

    fn emit_term(&mut self, term: Term, next_bb: Option<u32>) -> Result<(), String> {
        let prev_flags = self.live_flags.take();
        match term {
            Term::Jump { target, args } => {
                self.emit_edge_moves(target.0, args);
                if next_bb != Some(target.0) {
                    self.asm.b(self.bb_labels[target.0 as usize]);
                }
            }
            Term::BranchIf {
                cond,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => {
                let cr = self.gp_read(cond, SGP1);
                if next_bb == Some(then_bb.0) && then_bb != else_bb {
                    let else_lbl = self.edge_target_label(else_bb.0, else_args);
                    self.asm.cbz(false, cr, else_lbl);
                    self.emit_edge_moves(then_bb.0, then_args);
                } else {
                    let then_lbl = self.edge_target_label(then_bb.0, then_args);
                    self.asm.cbnz(false, cr, then_lbl);
                    self.emit_edge_moves(else_bb.0, else_args);
                    if next_bb != Some(else_bb.0) {
                        self.asm.b(self.bb_labels[else_bb.0 as usize]);
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
                let cond = match kind {
                    CmpBranchKind::I32(_)
                    | CmpBranchKind::I64(_)
                    | CmpBranchKind::I32Imm(..)
                    | CmpBranchKind::I64Imm(..) => {
                        self.emit_int_cmp_flags_reusing(prev_flags, kind, a, b).0
                    }
                    CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::F64(_));
                        let ar = self.fp_read(a, SFP1);
                        let br = self.fp_read(b, SFP2);
                        self.asm.fcmp(is64, ar, br);
                        Self::fp_cmp_branch_cond(cop)
                    }
                };
                if next_bb == Some(then_bb.0) && then_bb != else_bb {
                    let else_lbl = self.edge_target_label(else_bb.0, else_args);
                    self.asm.b_cond(cond.invert(), else_lbl);
                    self.emit_edge_moves(then_bb.0, then_args);
                } else {
                    let then_lbl = self.edge_target_label(then_bb.0, then_args);
                    self.asm.b_cond(cond, then_lbl);
                    self.emit_edge_moves(else_bb.0, else_args);
                    if next_bb != Some(else_bb.0) {
                        self.asm.b(self.bb_labels[else_bb.0 as usize]);
                    }
                }
            }
            Term::BrTable { idx, table_idx } => {
                let td = &self.ir.br_tables[table_idx as usize];
                let ir = self.gp_read(idx, SGP1);
                let def_lbl = self.edge_target_label(td.default.0.0, td.default.1);
                if td.targets.is_empty() {
                    self.asm.b(def_lbl);
                    return Ok(());
                }
                let targets: Vec<Label> = td
                    .targets
                    .iter()
                    .map(|&(t, a)| self.edge_target_label(t.0, a))
                    .collect();
                let n = targets.len() as u32;
                self.asm.mov_imm32(SGP2, n);
                self.asm.cmp_r(false, ir, SGP2);
                self.asm.b_cond(Cond::Hs, def_lbl);
                self.emit_speculation_barrier();
                let jt_off = self.asm.add_jump_table(targets);
                self.asm
                    .adr_reloc(SGP2, A64RelocTarget::LocalRodata(jt_off));
                self.asm.ldrsw_reg_scaled(SGP3, SGP2, ir, true);
                self.asm.add_r(true, SGP2, SGP2, SGP3);
                self.asm.br(SGP2);
            }
            Term::Return { vals } => {
                let ret_vals = self.ir.pool_slice(vals);
                let ret_tys: Vec<Ty> = ret_vals.iter().map(|&v| self.ty(v)).collect();
                let ret_layout = layout_rets(&ret_tys)?;
                if ret_layout.stack_bytes > 0 {
                    let saved_off = self.alloc.saved_ret_ptr_off.unwrap();
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP2),
                        None,
                        Reg::SP,
                        saved_off,
                        SGP3,
                    );
                    for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(&ret_layout.slots) {
                        if let A64AbiSlot::Stack(off) = *slot {
                            if ty.is_fp() {
                                let vr = self.fp_read(rv, SFP1);
                                emit_store_off(&mut self.asm, ty, None, Some(vr), SGP2, off, SGP3);
                            } else {
                                let gr = self.gp_read(rv, SGP1);
                                emit_store_off(&mut self.asm, ty, Some(gr), None, SGP2, off, SGP3);
                            }
                        }
                    }
                }
                let mut reg_moves = Vec::new();
                for ((&rv, &ty), slot) in ret_vals.iter().zip(&ret_tys).zip(&ret_layout.slots) {
                    match *slot {
                        A64AbiSlot::Gp(r) => {
                            reg_moves.push((MoveLoc::Gp(r), MoveLoc::from(self.loc(rv)), ty));
                        }
                        A64AbiSlot::Fp(v) => {
                            reg_moves.push((MoveLoc::Fp(v), MoveLoc::from(self.loc(rv)), ty));
                        }
                        A64AbiSlot::Stack(_) => {}
                    }
                }
                emit_parallel_moves(&mut self.asm, &reg_moves);
                self.asm.movz(false, SGP1, 0, 0); // trap = 0
                let ep = *self.epilogue_ok.get_or_insert_with(|| self.asm.new_label());
                self.asm.b(ep);
            }
            Term::ReturnCall { func, args } => {
                self.emit_tail_call(args, None, Some(func))?;
            }
            Term::ReturnCallIndirect { ci_id, idx, args } => {
                self.emit_tail_call(args, Some((ci_id, idx)), None)?;
            }
            Term::CallCatch {
                func,
                args,
                ok_bb,
                catch_bb,
                catch_args,
            } => {
                self.emit_call_catch_abi(
                    args,
                    ok_bb,
                    catch_bb,
                    catch_args,
                    None,
                    Some(func),
                    next_bb,
                )?;
            }
            Term::CallIndirectCatch {
                ci_id,
                idx,
                args,
                ok_bb,
                catch_bb,
                catch_args,
            } => {
                self.emit_call_catch_abi(
                    args,
                    ok_bb,
                    catch_bb,
                    catch_args,
                    Some((ci_id, idx)),
                    None,
                    next_bb,
                )?;
            }
            Term::ThrowActive => {
                self.asm
                    .mov_imm32(SGP1, u32::from(crate::x64::INTERNAL_EXN_SIGNAL));
                let ep = self.epilogue_trap_lbl();
                self.asm.b(ep);
            }
            Term::Unreachable => {
                let lbl = self.trap_unreach_lbl();
                self.asm.b(lbl);
            }
        }
        Ok(())
    }
}

/// Lowers a single verified `IrFunc` + `A64AllocResult` into AArch64 machine code.
pub fn lower_ir_to_aarch64(
    ir: &IrFunc,
    alloc: &A64AllocResult,
    layout: &CtxLayout,
) -> Result<A64Asm, String> {
    let mut asm = A64Asm::new();
    let bb_labels: Vec<Label> = (0..ir.blocks.len()).map(|_| asm.new_label()).collect();
    let mut l = LowerA64 {
        ir,
        alloc,
        layout,
        asm,
        bb_labels,
        trap_oob: None,
        trap_table_oob: None,
        trap_ci: None,
        trap_div0: None,
        trap_int_ovf: None,
        trap_inv_conv: None,
        trap_unreach: None,
        trap_stack: None,
        trap_prop: None,
        epilogue_ok: None,
        epilogue_trap: None,
        edge_tramps: Vec::new(),
        edge_tramp_cache: HashMap::new(),
        val_upper32_zero: None,
        live_flags: None,
    };

    // 1. Prologue: wrap-proof prospective stack overflow floor check (elided for zero-frame leaf
    //    functions that make no calls and never adjust SP) + allocate frame + save FP/LR + callee-saved regs.
    if l.alloc.has_calls || l.alloc.frame_size > 0 {
        let stack_lbl = l.trap_stack_lbl();
        emit_load_off(
            &mut l.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            OFF_STACK_FLOOR,
            SGP3,
        );
        if l.alloc.frame_size > 0 {
            l.asm
                .add_i32_checked(true, SGP1, SGP1, l.alloc.frame_size, SGP3);
        }
        l.asm.mov_r(true, SGP2, Reg::SP);
        l.asm.cmp_r(true, SGP2, SGP1);
        l.asm.b_cond(Cond::Lo, stack_lbl);
    }

    if l.alloc.frame_size > 0 {
        l.asm
            .add_i32_checked(true, Reg::SP, Reg::SP, -l.alloc.frame_size, SGP3);
    }
    let mut gp_off = l.alloc.saved_gp_base;
    if l.alloc.has_calls {
        if (gp_off & 7) == 0 && (-512..=504).contains(&gp_off) {
            l.asm.stp_x(Reg::FP, Reg::LR, Reg::SP, gp_off);
        } else {
            emit_store_off(
                &mut l.asm,
                Ty::I64,
                Some(Reg::FP),
                None,
                Reg::SP,
                gp_off,
                SGP3,
            );
            emit_store_off(
                &mut l.asm,
                Ty::I64,
                Some(Reg::LR),
                None,
                Reg::SP,
                gp_off + 8,
                SGP3,
            );
        }
        l.asm.add_i32_checked(true, Reg::FP, Reg::SP, gp_off, SGP3);
        gp_off += 16;
    }
    {
        let gps = &l.alloc.used_callee_gp;
        let mut gi = 0usize;
        while gi < gps.len() {
            if gi + 1 < gps.len() && (gp_off & 7) == 0 && (-512..=504).contains(&gp_off) {
                l.asm.stp_x(gps[gi], gps[gi + 1], Reg::SP, gp_off);
                gp_off += 16;
                gi += 2;
            } else {
                emit_store_off(
                    &mut l.asm,
                    Ty::I64,
                    Some(gps[gi]),
                    None,
                    Reg::SP,
                    gp_off,
                    SGP3,
                );
                gp_off += 8;
                gi += 1;
            }
        }
        let fps = &l.alloc.used_callee_fp;
        let mut fi = 0usize;
        while fi < fps.len() {
            let off = l.alloc.saved_fp_base + (fi as i32) * 16;
            if fi + 1 < fps.len() && (off & 15) == 0 && (-1024..=1008).contains(&off) {
                l.asm.stp_q(fps[fi], fps[fi + 1], Reg::SP, off);
                fi += 2;
            } else {
                emit_store_off(
                    &mut l.asm,
                    Ty::V128,
                    None,
                    Some(fps[fi]),
                    Reg::SP,
                    off,
                    SGP3,
                );
                fi += 1;
            }
        }
    }
    if let Some(saved_off) = l.alloc.saved_ret_ptr_off {
        emit_load_off(
            &mut l.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            OFF_RET_SPILL_PTR,
            SGP3,
        );
        emit_store_off(
            &mut l.asm,
            Ty::I64,
            Some(SGP1),
            None,
            Reg::SP,
            saved_off,
            SGP3,
        );
    }

    // Marshal incoming arguments from ABI registers & caller overflow stack area into assigned locs.
    let param_tys: Vec<Ty> = ir
        .params
        .iter()
        .map(|&pv| ir.val_tys[pv.0 as usize])
        .collect();
    let param_layout = layout_args(&param_tys)?;
    let mut param_moves = Vec::with_capacity(ir.params.len());
    for ((&pv, &ty), slot) in ir.params.iter().zip(&param_tys).zip(param_layout.slots) {
        let src = match slot {
            A64AbiSlot::Gp(r) => MoveLoc::Gp(r),
            A64AbiSlot::Fp(v) => MoveLoc::Fp(v),
            A64AbiSlot::Stack(off) => MoveLoc::Stack(l.alloc.frame_size + off),
        };
        param_moves.push((MoveLoc::from(l.loc(pv)), src, ty));
    }
    emit_parallel_moves(&mut l.asm, &param_moves);

    // Count value uses once so adjacent single-use `mul` + `add`/`sub` fuse into `madd`/`msub`!
    let mut use_cnt = vec![0u32; ir.val_tys.len()];
    for &bid in &l.alloc.block_order {
        let bb = &ir.blocks[bid as usize];
        for inst in &bb.insts {
            inst.op.for_each_use(&ir.val_pool, |v| {
                use_cnt[v.0 as usize] = use_cnt[v.0 as usize].saturating_add(1);
            });
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            use_cnt[v.0 as usize] = use_cnt[v.0 as usize].saturating_add(1);
        });
    }

    // 2. Emit basic blocks in RPO order (with integer `madd`/`msub` fusion).
    let n_ord = l.alloc.block_order.len();
    for oi in 0..n_ord {
        let bid = l.alloc.block_order[oi];
        let next_bb = l.alloc.block_order.get(oi + 1).copied();
        l.asm.bind(l.bb_labels[bid as usize]);
        l.live_flags = None;
        let bb = &l.ir.blocks[bid as usize];
        let mut ii = 0usize;
        while ii < bb.insts.len() {
            let inst = &bb.insts[ii];
            // Try `madd`/`msub` fusion: `tmp = x * y` (single use) followed by `z + tmp`, `tmp + z`, or `z - tmp`.
            if ii + 1 < bb.insts.len()
                && let Some(tmp) = inst.res
                && use_cnt[tmp.0 as usize] == 1
                && let Some((is64, x, y)) = match inst.op {
                    Op::I32Bin(IntBinOp::Mul, x, y) => Some((false, x, y)),
                    Op::I64Bin(IntBinOp::Mul, x, y) => Some((true, x, y)),
                    _ => None,
                }
            {
                let next = &bb.insts[ii + 1];
                let fused = match (is64, next.op) {
                    (false, Op::I32Bin(IntBinOp::Add, a, b))
                    | (true, Op::I64Bin(IntBinOp::Add, a, b)) => {
                        if a == tmp {
                            Some((false, b))
                        } else if b == tmp {
                            Some((false, a))
                        } else {
                            None
                        }
                    }
                    (false, Op::I32Bin(IntBinOp::Sub, a, b))
                    | (true, Op::I64Bin(IntBinOp::Sub, a, b))
                        if b == tmp =>
                    {
                        Some((true, a))
                    }
                    _ => None,
                };
                if let Some((is_sub, z)) = fused
                    && let Some(res_val) = next.res
                    && match l.loc(z) {
                        A64Loc::Gp(_) | A64Loc::Remat(_) => true,
                        A64Loc::Spill(off) => {
                            let max_uimm = if is64 { 4095 * 8 } else { 4095 * 4 };
                            let mask = if is64 { 7 } else { 3 };
                            off >= 0 && (off as u32 & mask) == 0 && (off as u32) <= max_uimm
                        }
                        A64Loc::Fp(_) => false,
                    }
                {
                    let xr = l.gp_read(x, SGP1);
                    let yr = l.gp_read(y, SGP2);
                    let zr = l.gp_read(z, SGP3);
                    let d = l.gp_write_reg(res_val);
                    if is_sub {
                        l.asm.msub(is64, d, xr, yr, zr);
                    } else {
                        l.asm.madd(is64, d, xr, yr, zr);
                    }
                    l.gp_commit(res_val, d);
                    ii += 2;
                    continue;
                }
            }
            l.emit_inst(inst.res, inst.op)?;
            ii += 1;
        }
        l.emit_term(bb.term, next_bb)?;
    }

    // 3. Emit deferred cold edge-copy trampolines.
    let tramps = std::mem::take(&mut l.edge_tramps);
    for (lbl, target_bb, args) in tramps {
        l.asm.bind(lbl);
        l.emit_edge_moves(target_bb, args);
        l.asm.b(l.bb_labels[target_bb as usize]);
    }

    // 4. Emit epilogues & cold trap stubs.
    if let Some(ep) = l.epilogue_ok {
        l.asm.bind(ep);
        l.emit_epilogue_body();
    }
    let trap_stubs = [
        (l.trap_oob, TRAP_OOB),
        (l.trap_table_oob, TRAP_TABLE_OOB),
        (l.trap_ci, TRAP_INDIRECT_CALL),
        (l.trap_div0, TRAP_DIV_ZERO),
        (l.trap_int_ovf, TRAP_INT_OVERFLOW),
        (l.trap_inv_conv, TRAP_INVALID_CONV),
        (l.trap_unreach, TRAP_UNREACHABLE),
    ];
    let need_ep_trap = l.epilogue_trap.is_some()
        || l.trap_prop.is_some()
        || trap_stubs.iter().any(|(o, _)| o.is_some());
    if need_ep_trap {
        let ep_trap = l.epilogue_trap_lbl();
        for (lbl_opt, code) in trap_stubs {
            if let Some(lbl) = lbl_opt {
                l.asm.bind(lbl);
                l.asm.mov_imm32(SGP1, u32::from(code));
                l.asm.b(ep_trap);
            }
        }
        if let Some(lbl) = l.trap_prop {
            l.asm.bind(lbl);
        }
        l.asm.bind(ep_trap);
        l.emit_epilogue_body();
    }
    if let Some(lbl) = l.trap_stack {
        l.asm.bind(lbl);
        l.asm.mov_imm32(SGP1, u32::from(TRAP_CALL_STACK));
        l.asm.ret();
    }

    l.asm.finish()?;
    Ok(l.asm)
}
