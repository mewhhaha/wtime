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
use super::regalloc::{A64AbiSlot, A64AllocResult, A64Loc, layout_args, layout_rets};
use crate::x64::ir::{
    CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, IntBinOp, IrFunc, LoadKind, Op, SimdLaneKind,
    SimdSplatKind, StoreKind, Term, TruncOp, Ty, Val, ValRange,
};
use crate::x64::simd_lower::V128_OP_PSADBW;
use crate::x64::{
    CtxLayout, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, OFF_RET_SPILL_PTR,
    OFF_SCRATCH_TMP, OFF_STACK_FLOOR, TRAP_CALL_STACK, TRAP_DIV_ZERO, TRAP_INDIRECT_CALL,
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
}

impl From<A64Loc> for MoveLoc {
    fn from(l: A64Loc) -> Self {
        match l {
            A64Loc::Gp(r) => MoveLoc::Gp(r),
            A64Loc::Fp(v) => MoveLoc::Fp(v),
            A64Loc::Spill(off) => MoveLoc::Stack(off),
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
}

impl<'a> LowerA64<'a> {
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
            A64Loc::Fp(_) => unreachable!("aarch64: expected GP val"),
        }
    }

    fn gp_write_reg(&self, v: Val) -> Reg {
        match self.loc(v) {
            A64Loc::Gp(r) => r,
            A64Loc::Spill(_) => SGP1,
            A64Loc::Fp(_) => unreachable!("aarch64: expected GP val"),
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
            A64Loc::Fp(_) => unreachable!(),
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
            A64Loc::Gp(_) => unreachable!("aarch64: expected FP val"),
        }
    }

    fn fp_write_reg(&self, v: Val) -> VReg {
        match self.loc(v) {
            A64Loc::Fp(r) => r,
            A64Loc::Spill(_) => SFP1,
            A64Loc::Gp(_) => unreachable!(),
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
            A64Loc::Gp(_) => unreachable!(),
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

    fn emit_epilogue_body(&mut self) {
        for (i, &v) in self.alloc.used_callee_fp.iter().enumerate() {
            let off = self.alloc.saved_fp_base + (i as i32) * 16;
            emit_load_off(&mut self.asm, Ty::V128, None, Some(v), Reg::SP, off, SGP3);
        }
        let mut gp_off = self.alloc.saved_gp_base;
        if self.alloc.has_calls {
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
            gp_off += 16;
        }
        for &r in &self.alloc.used_callee_gp {
            emit_load_off(&mut self.asm, Ty::I64, Some(r), None, Reg::SP, gp_off, SGP3);
            gp_off += 8;
        }
        if self.alloc.frame_size > 0 {
            self.asm
                .add_i32_checked(true, Reg::SP, Reg::SP, self.alloc.frame_size, SGP3);
        }
        self.asm.ret();
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
            return;
        }
        if end_off == 0 {
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            self.asm.b_cond(Cond::Hi, oob);
            return;
        }
        if self.layout.memory64 {
            self.asm.mov_imm64(SGP3, end_off);
            self.asm.adds_r(true, SGP2, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, oob); // carry set => 64-bit unsigned overflow!
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            self.asm.b_cond(Cond::Hi, oob);
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
    }

    /// Computes effective host address `MEM_BASE_REG + zext(addr) + offset` into `SGP3`.
    fn compute_mem_ptr(&mut self, addr: Reg, offset: u64) -> Reg {
        if !self.layout.memory64 {
            self.asm.uxtw(SGP3, addr);
        } else {
            self.asm.mov_r(true, SGP3, addr);
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
        self.asm.add_r(true, SGP3, MEM_BASE_REG, SGP3);
        SGP3
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

    fn emit_call_abi(
        &mut self,
        args: ValRange,
        rets: ValRange,
        ci_idx: Option<(u32, Val)>,
        direct_func: Option<u32>,
    ) -> Result<(), String> {
        if let Some((_, idx_val)) = ci_idx {
            let ir = self.gp_read(idx_val, SGP2);
            emit_store_off(
                &mut self.asm,
                Ty::I32,
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
        } else if let Some((ci_id, _)) = ci_idx {
            let ci = &self.layout.ci_tables[ci_id as usize];
            let toob = self.trap_table_oob_lbl();
            let bad = self.trap_ci_lbl();
            emit_load_off(
                &mut self.asm,
                Ty::I32,
                Some(SGP2),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            self.asm.mov_imm32(SGP3, ci.table_len);
            self.asm.cmp_r(false, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, toob);
            if ci.span == 0 {
                self.asm.b(bad);
                return Ok(());
            }
            if ci.min_slot > 0 {
                self.asm.mov_imm32(SGP3, ci.min_slot);
                self.asm.sub_r(false, SGP2, SGP2, SGP3);
            }
            self.asm.mov_imm32(SGP3, ci.span);
            self.asm.cmp_r(false, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, bad);
            self.asm
                .adr_reloc(SGP1, A64RelocTarget::Rodata(ci.rodata_off));
            // Each entry in ci_table is signed 32-bit offset relative to `ci_table` start.
            self.asm.lsl_imm(true, SGP2, SGP2, 2);
            self.asm.ldrsw_reg(SGP3, SGP1, SGP2, false);
            self.asm.cbz(true, SGP3, bad);
            self.asm.add_r(true, SGP1, SGP1, SGP3);
            self.asm.blr(SGP1);
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

    fn emit_int_bin(&mut self, is64: bool, op: IntBinOp, dst_val: Val, a: Val, b: Val) {
        let ar = self.gp_read(a, SGP1);
        let br = self.gp_read(b, SGP2);
        let d = self.gp_write_reg(dst_val);
        match op {
            IntBinOp::Add => self.asm.add_r(is64, d, ar, br),
            IntBinOp::Sub => self.asm.sub_r(is64, d, ar, br),
            IntBinOp::Mul => self.asm.mul(is64, d, ar, br),
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
            IntBinOp::RemU => {
                let div0 = self.trap_div0_lbl();
                self.asm.cbz(is64, br, div0);
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
            IntBinOp::DivU => {
                if imm == 0 {
                    let div0 = self.trap_div0_lbl();
                    self.asm.b(div0);
                } else {
                    self.asm.udiv(is64, d, ar, SGP2);
                }
            }
            IntBinOp::RemU => {
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
        match op {
            Op::ConstI32(c) => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                self.asm.mov_imm32(d, c as u32);
                self.gp_commit(r, d);
            }
            Op::ConstI64(c) => {
                let r = res.unwrap();
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
                self.asm.uxtw(d, ar);
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
                // Stage tv/fv BEFORE setting flags if they live in spill slots (or compare first when reg).
                let cond = match kind {
                    CmpBranchKind::I32(cop) | CmpBranchKind::I64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::I64(_));
                        let ar = self.gp_read(a, SGP1);
                        let br = self.gp_read(b, SGP2);
                        self.asm.cmp_r(is64, ar, br);
                        cmp_to_cond(cop)
                    }
                    CmpBranchKind::I32Imm(cop, imm) | CmpBranchKind::I64Imm(cop, imm) => {
                        let is64 = matches!(kind, CmpBranchKind::I64Imm(..));
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
                        cmp_to_cond(cop)
                    }
                    CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::F64(_));
                        let ar = self.fp_read(a, SFP1);
                        let br = self.fp_read(b, SFP2);
                        self.asm.fcmp(is64, ar, br);
                        Self::fp_cmp_branch_cond(cop)
                    }
                };
                // Save condition into SGP3 before reading potentially spilled tv/fv (so `add_i32_checked` never clobbers flags!).
                self.asm.cset(false, SGP3, cond);
                self.asm.cmp_imm(false, SGP3, 0);
                if ty == Ty::V128 {
                    let lbl_f = self.asm.new_label();
                    let lbl_d = self.asm.new_label();
                    let tr = self.fp_read(tv, SFP1);
                    let fr = self.fp_read(fv, SFP2);
                    let d = self.fp_write_reg(r);
                    self.asm.b_cond(Cond::Eq, lbl_f);
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
                    self.asm.fcsel(ty == Ty::F64, d, tr, fr, Cond::Ne);
                    self.fp_commit(r, d);
                } else {
                    let tr = self.gp_read(tv, SGP1);
                    let fr = self.gp_read(fv, SGP2);
                    let d = self.gp_write_reg(r);
                    self.asm.csel(ty == Ty::I64, d, tr, fr, Cond::Ne);
                    self.gp_commit(r, d);
                }
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
                let (off, len) = self.layout.extern_tables[&table];
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                self.asm.mov_imm32(SGP2, len);
                self.asm.cmp_r(false, ir, SGP2);
                self.asm.b_cond(Cond::Hs, toob);
                self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP2);
                self.asm.uxtw(SGP2, ir);
                self.asm.lsl_imm(true, SGP2, SGP2, 3);
                let d = self.gp_write_reg(r);
                self.asm.ldr_reg(true, d, SGP3, SGP2, false);
                self.gp_commit(r, d);
            }
            Op::TableSet { table, idx, val } => {
                let (off, len) = self.layout.extern_tables[&table];
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                self.asm.mov_imm32(SGP2, len);
                self.asm.cmp_r(false, ir, SGP2);
                self.asm.b_cond(Cond::Hs, toob);
                let vr = self.gp_read(val, SGP1);
                // Re-read idx safely if both spilled into SGP1!
                let idx_r = if self.loc(idx) != A64Loc::Gp(ir) {
                    self.asm.mov_r(true, SGP2, vr);
                    let ii = self.gp_read(idx, SGP1);
                    (ii, SGP2)
                } else {
                    (ir, vr)
                };
                self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP3);
                self.asm.uxtw(SGP1, idx_r.0);
                self.asm.lsl_imm(true, SGP1, SGP1, 3);
                self.asm.str_reg(true, idx_r.1, SGP3, SGP1, false);
            }
            Op::BoundsCheck { addr, end_off } => {
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
                if !checked {
                    self.emit_bounds_check(ar, offset.saturating_add(kind.bytes()));
                }
                let ptr = self.compute_mem_ptr(ar, offset);
                match kind {
                    LoadKind::I32 => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(false, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64 => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(true, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::F32 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, 0);
                        self.fp_commit(r, d);
                    }
                    LoadKind::F64 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, 0);
                        self.fp_commit(r, d);
                    }
                    LoadKind::I32Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(false, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load8U | LoadKind::I64Load8U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrb_uimm(d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(false, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I32Load16U | LoadKind::I64Load16U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrh_uimm(d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load8S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsb_uimm(true, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load16S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsh_uimm(true, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load32S => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldrsw_uimm(d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::I64Load32U => {
                        let d = self.gp_write_reg(r);
                        self.asm.ldr_uimm(false, d, ptr, 0);
                        self.gp_commit(r, d);
                    }
                    LoadKind::V128 => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_q_uimm(d, ptr, 0);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load8Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrb_uimm(SGP1, ptr, 0);
                        self.asm.dup_gp_128(0, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load16Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldrh_uimm(SGP1, ptr, 0);
                        self.asm.dup_gp_128(1, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(false, SGP1, ptr, 0);
                        self.asm.dup_gp_128(2, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Splat => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_uimm(true, SGP1, ptr, 0);
                        self.asm.dup_gp_128(3, d, SGP1);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load32Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_s_uimm(d, ptr, 0);
                        self.fp_commit(r, d);
                    }
                    LoadKind::V128Load64Zero => {
                        let d = self.fp_write_reg(r);
                        self.asm.ldr_d_uimm(d, ptr, 0);
                        self.fp_commit(r, d);
                    }
                    _ => {
                        return Err(format!("aarch64: unsupported SIMD load kind {kind:?}"));
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
                if !checked {
                    self.emit_bounds_check(ar, offset.saturating_add(kind.bytes()));
                }
                let ptr = self.compute_mem_ptr(ar, offset);
                match kind {
                    StoreKind::I8 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strb_uimm(vr, ptr, 0);
                    }
                    StoreKind::I16 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.strh_uimm(vr, ptr, 0);
                    }
                    StoreKind::I32 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(false, vr, ptr, 0);
                    }
                    StoreKind::I64 => {
                        let vr = self.gp_read(val, SGP1);
                        self.asm.str_uimm(true, vr, ptr, 0);
                    }
                    StoreKind::F32 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_s_uimm(vr, ptr, 0);
                    }
                    StoreKind::F64 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_d_uimm(vr, ptr, 0);
                    }
                    StoreKind::V128 => {
                        let vr = self.fp_read(val, SFP1);
                        self.asm.str_q_uimm(vr, ptr, 0);
                    }
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
                    // Overlap-safe small copy (up to 64 B) using a short loop with direction check.
                    let dr = self.gp_read(dst, SGP1);
                    if !self.layout.memory64 {
                        self.asm.uxtw(SGP1, dr);
                    } else {
                        self.asm.mov_r(true, SGP1, dr);
                    }
                    let sr = self.gp_read(src, SGP2);
                    if !self.layout.memory64 {
                        self.asm.uxtw(SGP2, sr);
                    } else {
                        self.asm.mov_r(true, SGP2, sr);
                    }
                    self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1); // dst_ptr
                    self.asm.add_r(true, SGP2, MEM_BASE_REG, SGP2); // src_ptr
                    let lbl_bwd = self.asm.new_label();
                    let lbl_done = self.asm.new_label();
                    self.asm.cmp_r(true, SGP1, SGP2);
                    self.asm.b_cond(Cond::Hi, lbl_bwd);
                    // Forward byte copy (zero register clobbers, small len <= 64).
                    for k in 0..len {
                        self.asm.ldrb_uimm(SGP3, SGP2, k);
                        self.asm.strb_uimm(SGP3, SGP1, k);
                    }
                    self.asm.b(lbl_done);
                    self.asm.bind(lbl_bwd);
                    for k in (0..len).rev() {
                        self.asm.ldrb_uimm(SGP3, SGP2, k);
                        self.asm.strb_uimm(SGP3, SGP1, k);
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
                        self.asm.uxtw(SGP3, dr);
                    } else {
                        self.asm.mov_r(true, SGP3, dr);
                    }
                    self.asm.add_r(true, SGP3, MEM_BASE_REG, SGP3);
                    let vr = self.gp_read(val, SGP1);
                    for k in 0..len {
                        self.asm.strb_uimm(vr, SGP3, k);
                    }
                }
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
            Op::V128Un { op: uop, a } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let d = self.fp_write_reg(r);
                match uop {
                    0xfd4d => self.asm.not_16b(d, ar),
                    0xfd62 => self.asm.cnt_v(true, d, ar),
                    0xfde0 => self.asm.fabs_v128(false, d, ar),
                    0xfde1 => self.asm.fneg_v128(false, d, ar),
                    0xfde3 => self.asm.fsqrt_v128(false, d, ar),
                    0xfdec => self.asm.fabs_v128(true, d, ar),
                    0xfded => self.asm.fneg_v128(true, d, ar),
                    0xfdef => self.asm.fsqrt_v128(true, d, ar),
                    _ => return Err(format!("aarch64: unsupported V128Un opcode {uop:#x}")),
                }
                self.fp_commit(r, d);
            }
            Op::V128Bin { op: bop, a, b } => {
                let r = res.unwrap();
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let d = self.fp_write_reg(r);
                match bop {
                    0xfd4e => self.asm.and_16b(d, ar, br),
                    0xfd4f => self.asm.bic_16b(d, ar, br),
                    0xfd50 => self.asm.orr_16b(d, ar, br),
                    0xfd51 => self.asm.eor_16b(d, ar, br),
                    0xfd6e => self.asm.add_v128(0, d, ar, br),
                    0xfd71 => self.asm.sub_v128(0, d, ar, br),
                    0xfd8e => self.asm.add_v128(1, d, ar, br),
                    0xfd91 => self.asm.sub_v128(1, d, ar, br),
                    0xfd95 => self.asm.mul_v128(1, d, ar, br),
                    0xfdae => self.asm.add_v128(2, d, ar, br),
                    0xfdb1 => self.asm.sub_v128(2, d, ar, br),
                    0xfdb5 => self.asm.mul_v128(2, d, ar, br),
                    0xfdce => self.asm.add_v128(3, d, ar, br),
                    0xfdd1 => self.asm.sub_v128(3, d, ar, br),
                    0xfde4 => self.asm.fadd_v128(false, d, ar, br),
                    0xfde5 => self.asm.fsub_v128(false, d, ar, br),
                    0xfde6 => self.asm.fmul_v128(false, d, ar, br),
                    0xfde7 => self.asm.fdiv_v128(false, d, ar, br),
                    0xfde8 => self.asm.fmin_v128(false, d, ar, br),
                    0xfde9 => self.asm.fmax_v128(false, d, ar, br),
                    0xfdea => {
                        // f32x4.pmin(a, b) = b < a ? b : a  (mask = a > b = fcmgt(a, b))
                        self.asm.fcmgt_v128(false, SFP1, ar, br);
                        self.asm.bsl_16b(SFP1, br, ar);
                        self.asm.mov_16b(d, SFP1);
                    }
                    0xfdeb => {
                        // f32x4.pmax(a, b) = a < b ? b : a  (mask = b > a = fcmgt(b, a))
                        self.asm.fcmgt_v128(false, SFP1, br, ar);
                        self.asm.bsl_16b(SFP1, br, ar);
                        self.asm.mov_16b(d, SFP1);
                    }
                    0xfdf0 => self.asm.fadd_v128(true, d, ar, br),
                    0xfdf1 => self.asm.fsub_v128(true, d, ar, br),
                    0xfdf2 => self.asm.fmul_v128(true, d, ar, br),
                    0xfdf3 => self.asm.fdiv_v128(true, d, ar, br),
                    0xfdf4 => self.asm.fmin_v128(true, d, ar, br),
                    0xfdf5 => self.asm.fmax_v128(true, d, ar, br),
                    0xfdf6 => {
                        // f64x2.pmin(a, b) = b < a ? b : a
                        let b_safe = if ar == br {
                            ar
                        } else if d == br {
                            self.asm.mov_16b(SFP2, br);
                            SFP2
                        } else {
                            br
                        };
                        let a_safe = if d == ar {
                            self.asm.mov_16b(SFP1, ar);
                            SFP1
                        } else {
                            ar
                        };
                        self.asm.fcmgt_v128(true, d, a_safe, b_safe);
                        self.asm.bsl_16b(d, b_safe, a_safe);
                    }
                    0xfdf7 => {
                        // f64x2.pmax(a, b) = a < b ? b : a
                        let b_safe = if ar == br {
                            ar
                        } else if d == br {
                            self.asm.mov_16b(SFP2, br);
                            SFP2
                        } else {
                            br
                        };
                        let a_safe = if d == ar {
                            self.asm.mov_16b(SFP1, ar);
                            SFP1
                        } else {
                            ar
                        };
                        self.asm.fcmgt_v128(true, d, b_safe, a_safe);
                        self.asm.bsl_16b(d, b_safe, a_safe);
                    }
                    V128_OP_PSADBW => {
                        // Exact 16-byte pairwise-sum into two u64 lanes (matches x86 `psadbw(a, 0)`):
                        // uaddlp.8h(a) -> uaddlp.4s -> uaddlp.2d!
                        self.asm.uaddlp_128(0, d, ar);
                        self.asm.uaddlp_128(1, d, d);
                        self.asm.uaddlp_128(2, d, d);
                    }
                    _ => return Err(format!("aarch64: unsupported V128Bin opcode {bop:#x}")),
                }
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
                let ar = self.fp_read(a, SFP1);
                let br = self.fp_read(b, SFP2);
                let cr = match self.loc(c) {
                    A64Loc::Fp(v) => v,
                    A64Loc::Spill(off) => {
                        emit_load_off(&mut self.asm, Ty::V128, None, Some(d), Reg::SP, off, SGP3);
                        d
                    }
                    A64Loc::Gp(_) => unreachable!(),
                };
                // Save `a`/`b` if aliasing `d` before moving `c` into `d`.
                let a_use = if d == ar {
                    self.asm.mov_16b(SFP1, ar);
                    SFP1
                } else {
                    ar
                };
                let b_use = if d == br {
                    self.asm.mov_16b(SFP2, br);
                    SFP2
                } else {
                    br
                };
                self.asm.mov_16b(d, cr);
                self.asm.bsl_16b(d, a_use, b_use);
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
                let then_lbl = self.edge_target_label(then_bb.0, then_args);
                self.asm.cbnz(false, cr, then_lbl);
                self.emit_edge_moves(else_bb.0, else_args);
                if next_bb != Some(else_bb.0) {
                    self.asm.b(self.bb_labels[else_bb.0 as usize]);
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
                let then_lbl = self.edge_target_label(then_bb.0, then_args);
                let cond = match kind {
                    CmpBranchKind::I32(cop) | CmpBranchKind::I64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::I64(_));
                        let ar = self.gp_read(a, SGP1);
                        let br = self.gp_read(b, SGP2);
                        self.asm.cmp_r(is64, ar, br);
                        cmp_to_cond(cop)
                    }
                    CmpBranchKind::I32Imm(cop, imm) | CmpBranchKind::I64Imm(cop, imm) => {
                        let is64 = matches!(kind, CmpBranchKind::I64Imm(..));
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
                        cmp_to_cond(cop)
                    }
                    CmpBranchKind::F32(cop) | CmpBranchKind::F64(cop) => {
                        let is64 = matches!(kind, CmpBranchKind::F64(_));
                        let ar = self.fp_read(a, SFP1);
                        let br = self.fp_read(b, SFP2);
                        self.asm.fcmp(is64, ar, br);
                        Self::fp_cmp_branch_cond(cop)
                    }
                };
                self.asm.b_cond(cond, then_lbl);
                self.emit_edge_moves(else_bb.0, else_args);
                if next_bb != Some(else_bb.0) {
                    self.asm.b(self.bb_labels[else_bb.0 as usize]);
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
                let jt_off = self.asm.add_jump_table(targets);
                self.asm
                    .adr_reloc(SGP2, A64RelocTarget::LocalRodata(jt_off));
                self.asm.uxtw(SGP1, ir);
                self.asm.lsl_imm(true, SGP1, SGP1, 2);
                self.asm.ldrsw_reg(SGP3, SGP2, SGP1, false);
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
    };

    // 1. Prologue: stack overflow floor check + allocate frame + save FP/LR + callee-saved regs.
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
    l.asm.mov_r(true, SGP2, Reg::SP);
    l.asm.cmp_r(true, SGP2, SGP1);
    l.asm.b_cond(Cond::Lo, stack_lbl);

    if l.alloc.frame_size > 0 {
        l.asm
            .add_i32_checked(true, Reg::SP, Reg::SP, -l.alloc.frame_size, SGP3);
    }
    let mut gp_off = l.alloc.saved_gp_base;
    if l.alloc.has_calls {
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
        l.asm.add_i32_checked(true, Reg::FP, Reg::SP, gp_off, SGP3);
        gp_off += 16;
    }
    for &r in &l.alloc.used_callee_gp {
        emit_store_off(&mut l.asm, Ty::I64, Some(r), None, Reg::SP, gp_off, SGP3);
        gp_off += 8;
    }
    for (i, &v) in l.alloc.used_callee_fp.iter().enumerate() {
        let off = l.alloc.saved_fp_base + (i as i32) * 16;
        emit_store_off(&mut l.asm, Ty::V128, None, Some(v), Reg::SP, off, SGP3);
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

    // 2. Emit basic blocks in RPO order.
    let n_ord = l.alloc.block_order.len();
    for oi in 0..n_ord {
        let bid = l.alloc.block_order[oi];
        let next_bb = l.alloc.block_order.get(oi + 1).copied();
        l.asm.bind(l.bb_labels[bid as usize]);
        let bb = &l.ir.blocks[bid as usize];
        for inst in &bb.insts {
            l.emit_inst(inst.res, inst.op)?;
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
