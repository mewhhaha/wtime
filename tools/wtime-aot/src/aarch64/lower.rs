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
use crate::x64::gc_rt::{
    GC_OFF_ALLOC_LIMIT, GC_OFF_ALLOC_PTR, GC_OFF_FROM_BASE, GC_OFF_FROM_CAP, GC_OFF_TO_BASE,
    GC_OFF_TO_CAP,
};
use crate::x64::ir::{
    AtomicRmwOp, AtomicWidth, CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, GcFieldLoadKind,
    GcHeapTarget, IntBinOp, IrFunc, LoadKind, Op, SimdLaneKind, SimdSplatKind, StoreKind, Term,
    TruncOp, Ty, Val, ValRange, VecElemKind, VecReduceKind,
};
use crate::x64::module::{
    GC_DISPLAY_LEN, GC_EXT_I31_TAG_HI, GC_HDR_KIND_ARRAY, GC_HDR_KIND_INTERNAL_EXTERN,
    GC_HDR_KIND_STRUCT, GcCanonType, GcCompositeInfo, GcStorageKind, X64Module,
};
use crate::x64::{
    CtxLayout, HELPER_ATOMIC_NOTIFY, HELPER_ATOMIC_WAIT, HELPER_EXTRA_MEM_COPY,
    HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW, HELPER_GC_ARRAY_COPY, HELPER_GC_ARRAY_FILL,
    HELPER_GC_ARRAY_FILL_RAW, HELPER_GC_ARRAY_INIT_DATA, HELPER_GC_ARRAY_INIT_ELEM,
    HELPER_GC_COLLECT, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW, HELPER_MEM_INIT,
    HELPER_STORE_EXN_ALLOC, HELPER_TABLE_COPY, HELPER_TABLE_FILL, HELPER_TABLE_GROW,
    HELPER_TABLE_INIT, OFF_MEM_BASE, OFF_MEM_LEN, OFF_MEM_RESERVE_BYTES, OFF_RET_SPILL_PTR,
    OFF_SCRATCH_TMP, OFF_SCRATCH_V128, OFF_STACK_FLOOR, OFF_STORE_CANON_MAP, OFF_STORE_FUNC_REFS,
    OFF_STORE_MEM_DESCS, OFF_STORE_PTR, OFF_STORE_TABLE_DESCS, OFF_STORE_TAG_IDS,
    STORE_OFF_EXN_ACTIVE, STORE_OFF_RET_SPILL_PTR, STORE_OFF_TYPE_DESCS_PTR, TRAP_ALLOC_FAILED,
    TRAP_CALL_STACK, TRAP_DIV_ZERO, TRAP_INDIRECT_CALL, TRAP_INT_OVERFLOW, TRAP_INVALID_CONV,
    TRAP_OOB, TRAP_TABLE_OOB, TRAP_UNALIGNED_ATOMIC, TRAP_UNREACHABLE,
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
    RematAddr {
        base_reg: Reg,
        disp: i32,
        is64: bool,
    },
}

impl From<A64Loc> for MoveLoc {
    fn from(l: A64Loc) -> Self {
        match l {
            A64Loc::Gp(r) => MoveLoc::Gp(r),
            A64Loc::Fp(v) => MoveLoc::Fp(v),
            A64Loc::Spill(off) => MoveLoc::Stack(off),
            A64Loc::Remat(rc) => MoveLoc::Remat(rc),
            A64Loc::RematAddr {
                base_reg,
                disp,
                is64,
                ..
            } => MoveLoc::RematAddr {
                base_reg,
                disp,
                is64,
            },
        }
    }
}

#[inline]
fn move_src_read_loc(src: MoveLoc) -> MoveLoc {
    match src {
        MoveLoc::RematAddr { base_reg, .. } => MoveLoc::Gp(base_reg),
        other => other,
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
        (
            MoveLoc::Gp(d),
            MoveLoc::RematAddr {
                base_reg,
                disp,
                is64,
            },
        ) => {
            if disp >= 0 {
                asm.add_imm(is64, d, base_reg, disp as u16);
            } else {
                asm.sub_imm(is64, d, base_reg, disp.unsigned_abs() as u16);
            }
        }
        (
            MoveLoc::Stack(off),
            MoveLoc::RematAddr {
                base_reg,
                disp,
                is64,
            },
        ) => {
            if disp >= 0 {
                asm.add_imm(is64, SGP1, base_reg, disp as u16);
            } else {
                asm.sub_imm(is64, SGP1, base_reg, disp.unsigned_abs() as u16);
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
                if (0..n).any(|j| j != i && active[j] && move_src_read_loc(moves[j].1) == dst) {
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
    module: &'a X64Module,
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
    trap_alloc: Option<Label>,
    trap_unaligned: Option<Label>,
    trap_prop: Option<Label>,
    epilogue_ok: Option<Label>,
    epilogue_trap: Option<Label>,
    edge_tramps: Vec<(Label, u32, ValRange)>,
    edge_tramp_cache: HashMap<(u32, u32, u32), Label>,
    cur_bb: u32,
    cur_inst: u32,
    /// Lazily populated map of SSA `Val`s whose AArch64 defining write guarantees bits `32..63 == 0`,
    /// allowing redundant `uxtw` on `I32WrapI64`/`I64ExtendI32U` to be elided.
    val_upper32_zero: Option<Vec<bool>>,
    /// Intra-block live integer NZCV flags for redundant `cmp` elision across adjacent
    /// `Op::CmpSelect` / `Term::CmpBranch` instructions.
    live_flags: Option<A64LiveFlags>,
}

impl<'a> LowerA64<'a> {
    fn is_upper32_zero(&mut self, v: Val) -> bool {
        let table =
            self.val_upper32_zero.get_or_insert_with(|| {
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
                            | Op::VecReduceLoop {
                                kind:
                                    VecReduceKind::I32Sum
                                    | VecReduceKind::I32Xor
                                    | VecReduceKind::I32Dot,
                                ..
                            }
                            | Op::SimdOuterLoop { .. }
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
            A64Loc::RematAddr {
                base_reg,
                disp,
                is64,
                ..
            } => {
                if disp >= 0 {
                    self.asm.add_imm(is64, scratch, base_reg, disp as u16);
                } else {
                    self.asm
                        .sub_imm(is64, scratch, base_reg, disp.unsigned_abs() as u16);
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
            A64Loc::Fp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => {
                unreachable!("aarch64: expected GP val")
            }
        }
    }

    /// Emits `Op::SimdOuterLoop` on ARMv8-A 4-wide NEON using caller-saved `X0..X4` and
    /// `V0..V7, V16..V23` (zero callee-saved registers touched; 100% `A64Vm`-compatible).
    fn emit_simd_outer_loop(&mut self, desc_idx: u32, args: ValRange, res: Val) {
        use crate::x64::ir::{CmpOp, FloatBinOp, FloatUnOp, OuterSimdStep};
        const VREGS: [VReg; 12] = [
            VReg::V0,
            VReg::V1,
            VReg::V2,
            VReg::V3,
            VReg::V4,
            VReg::V5,
            VReg::V6,
            VReg::V7,
            VReg::V16,
            VReg::V17,
            VReg::V18,
            VReg::V19,
        ];
        const V_IT: VReg = VReg::V20;
        const V_ACT: VReg = VReg::V21;
        const VT1: VReg = VReg::V22;
        const VT2: VReg = VReg::V23;

        let desc = self.ir.outer_simd_loops[desc_idx as usize].clone();
        let arg_vals: Vec<Val> = self.ir.pool_slice(args).to_vec();

        let gp_moves = [
            (
                MoveLoc::Gp(Reg::X0),
                MoveLoc::from(self.loc(arg_vals[0])),
                Ty::I32,
            ),
            (
                MoveLoc::Gp(Reg::X1),
                MoveLoc::from(self.loc(arg_vals[1])),
                Ty::I32,
            ),
        ];
        emit_parallel_moves(&mut self.asm, &gp_moves);
        let mut fp_moves: Vec<(MoveLoc, MoveLoc, Ty)> =
            Vec::with_capacity(desc.n_f32_args as usize);
        for k in 0..desc.n_f32_args as usize {
            fp_moves.push((
                MoveLoc::Fp(VREGS[k]),
                MoveLoc::from(self.loc(arg_vals[2 + k])),
                Ty::F32,
            ));
        }
        emit_parallel_moves(&mut self.asm, &fp_moves);

        let lbl_done = self.asm.new_label();
        let lbl_outer = self.asm.new_label();
        let lbl_inner = self.asm.new_label();
        let lbl_inner_done = self.asm.new_label();

        self.asm.mov_imm32(Reg::X2, 0);
        self.asm.cmp_r(false, Reg::X0, Reg::X1);
        self.asm.b_cond(Cond::Hs, lbl_done);
        self.asm.sub_r(false, Reg::X1, Reg::X1, Reg::X0);

        for &r in VREGS.iter().take(desc.n_f32_args as usize) {
            self.asm.dup_elt_128(2, r, r, 0);
        }

        let mut ramp_bytes = [0u8; 16];
        for i in 0u32..4 {
            ramp_bytes[(i as usize) * 4..(i as usize) * 4 + 4].copy_from_slice(&i.to_le_bytes());
        }
        let ramp_off = self.asm.add_local_rodata(16, &ramp_bytes);

        let bcast_u32 = |asm: &mut A64Asm, dst: VReg, bits: u32| {
            if bits == 0 {
                asm.movi_zero_16b(dst);
            } else {
                asm.mov_imm32(SGP1, bits);
                asm.dup_gp_128(2, dst, SGP1);
            }
        };
        let load_ramp = |asm: &mut A64Asm, dst: VReg| {
            asm.adr_reloc(SGP1, A64RelocTarget::LocalRodata(ramp_off));
            asm.ldr_q_uimm(dst, SGP1, 0);
        };
        let emit_step = |asm: &mut A64Asm, st: OuterSimdStep| match st {
            OuterSimdStep::BcastConst { dst, const_idx } => {
                bcast_u32(
                    asm,
                    VREGS[dst as usize],
                    desc.const_f32s[const_idx as usize],
                );
            }
            OuterSimdStep::CvtPxToF32 { dst, signed } => {
                let d = VREGS[dst as usize];
                asm.dup_gp_128(2, VT1, Reg::X0);
                load_ramp(asm, VT2);
                asm.add_v128(2, VT1, VT1, VT2);
                if signed {
                    asm.scvtf_v128(false, d, VT1);
                } else {
                    asm.ucvtf_v128(false, d, VT1);
                }
            }
            OuterSimdStep::Move { dst, src } => {
                let (d, s) = (VREGS[dst as usize], VREGS[src as usize]);
                if d != s {
                    asm.mov_16b(d, s);
                }
            }
            OuterSimdStep::F32Bin { op, dst, lhs, rhs } => {
                let (d, l, r) = (
                    VREGS[dst as usize],
                    VREGS[lhs as usize],
                    VREGS[rhs as usize],
                );
                match op {
                    FloatBinOp::Add => asm.fadd_v128(false, d, l, r),
                    FloatBinOp::Sub => asm.fsub_v128(false, d, l, r),
                    FloatBinOp::Mul => asm.fmul_v128(false, d, l, r),
                    FloatBinOp::Div => asm.fdiv_v128(false, d, l, r),
                    _ => unreachable!(),
                }
            }
            OuterSimdStep::F32Un { op, dst, src } => {
                let (d, s) = (VREGS[dst as usize], VREGS[src as usize]);
                match op {
                    FloatUnOp::Neg => asm.fneg_v128(false, d, s),
                    FloatUnOp::Abs => asm.fabs_v128(false, d, s),
                    FloatUnOp::Sqrt => asm.fsqrt_v128(false, d, s),
                    _ => unreachable!(),
                }
            }
            OuterSimdStep::BreakIf {
                cop,
                break_on_true,
                lhs,
                rhs,
                delta_after,
            } => {
                let (l, r) = (VREGS[lhs as usize], VREGS[rhs as usize]);
                let use_and = match (break_on_true, cop) {
                    (true, CmpOp::GtS) => {
                        asm.fcmgt_v128(false, VT1, l, r);
                        false
                    }
                    (true, CmpOp::GeS) => {
                        asm.fcmge_v128(false, VT1, l, r);
                        false
                    }
                    (true, CmpOp::LtS) => {
                        asm.fcmgt_v128(false, VT1, r, l);
                        false
                    }
                    (true, CmpOp::LeS) => {
                        asm.fcmge_v128(false, VT1, r, l);
                        false
                    }
                    (true, CmpOp::Eq) => {
                        asm.fcmeq_v128(false, VT1, l, r);
                        false
                    }
                    (true, CmpOp::Ne) => {
                        asm.fcmeq_v128(false, VT1, l, r);
                        true
                    }
                    (false, CmpOp::GtS) => {
                        asm.fcmgt_v128(false, VT1, l, r);
                        true
                    }
                    (false, CmpOp::GeS) => {
                        asm.fcmge_v128(false, VT1, l, r);
                        true
                    }
                    (false, CmpOp::LtS) => {
                        asm.fcmgt_v128(false, VT1, r, l);
                        true
                    }
                    (false, CmpOp::LeS) => {
                        asm.fcmge_v128(false, VT1, r, l);
                        true
                    }
                    (false, CmpOp::Eq) => {
                        asm.fcmeq_v128(false, VT1, l, r);
                        true
                    }
                    (false, CmpOp::Ne) => {
                        asm.fcmeq_v128(false, VT1, l, r);
                        false
                    }
                    _ => unreachable!(),
                };
                if use_and {
                    asm.and_16b(V_ACT, V_ACT, VT1);
                } else {
                    asm.bic_16b(V_ACT, V_ACT, VT1);
                }
                asm.umov(3, SGP1, V_ACT, 0);
                asm.umov(3, SGP2, V_ACT, 1);
                asm.orr_r(true, SGP1, SGP1, SGP2);
                asm.cbz(true, SGP1, lbl_inner_done);
                if delta_after == 1 {
                    asm.sub_v128(2, V_IT, V_IT, V_ACT);
                } else if delta_after == -1 {
                    asm.add_v128(2, V_IT, V_IT, V_ACT);
                } else if delta_after != 0 {
                    bcast_u32(asm, VT1, delta_after as u32);
                    asm.and_16b(VT1, VT1, V_ACT);
                    asm.add_v128(2, V_IT, V_IT, VT1);
                }
            }
        };

        for &st in &desc.init_steps {
            emit_step(&mut self.asm, st);
        }

        self.asm.bind(lbl_outer);
        self.asm.mov_imm32(Reg::X3, 4);
        self.asm.cmp_imm(false, Reg::X1, 4);
        self.asm.csel(false, Reg::X3, Reg::X1, Reg::X3, Cond::Lo);

        self.asm.dup_gp_128(2, VT1, Reg::X3);
        load_ramp(&mut self.asm, VT2);
        self.asm.cmgt_v128(2, V_ACT, VT1, VT2);
        self.asm.movi_zero_16b(V_IT);
        if desc.base_it != 0 {
            bcast_u32(&mut self.asm, VT1, desc.base_it as u32);
            self.asm.and_16b(VT1, VT1, V_ACT);
            self.asm.add_v128(2, V_IT, V_IT, VT1);
        }

        for &st in &desc.pre_steps {
            emit_step(&mut self.asm, st);
        }

        self.asm.mov_imm32(Reg::X4, desc.inner_trips);
        self.asm.bind(lbl_inner);
        for &st in &desc.inner_steps {
            emit_step(&mut self.asm, st);
        }
        let mut pending: Vec<(VReg, VReg)> = desc
            .backedge_copies
            .iter()
            .map(|&(d, s)| (VREGS[d as usize], VREGS[s as usize]))
            .filter(|&(d, s)| d != s)
            .collect();
        while !pending.is_empty() {
            if let Some(pos) = (0..pending.len()).find(|&i| {
                let d = pending[i].0;
                pending
                    .iter()
                    .enumerate()
                    .all(|(j, &(_, s))| j == i || s != d)
            }) {
                let (d, s) = pending.swap_remove(pos);
                self.asm.mov_16b(d, s);
            } else {
                let d0 = pending[0].0;
                self.asm.mov_16b(VT1, d0);
                for (_, s) in &mut pending {
                    if *s == d0 {
                        *s = VT1;
                    }
                }
            }
        }
        self.asm.sub_imm(false, Reg::X4, Reg::X4, 1);
        self.asm.cbnz(false, Reg::X4, lbl_inner);

        self.asm.bind(lbl_inner_done);
        for lane in 0..4u8 {
            self.asm.umov(2, SGP1, V_IT, lane);
            self.asm.add_r(false, Reg::X2, Reg::X2, SGP1);
        }
        self.asm.add_r(false, Reg::X0, Reg::X0, Reg::X3);
        self.asm.sub_r(false, Reg::X1, Reg::X1, Reg::X3);
        self.asm.cbnz(false, Reg::X1, lbl_outer);

        self.asm.bind(lbl_done);
        let d = self.gp_write_reg(res);
        if d != Reg::X2 {
            self.asm.mov_r(false, d, Reg::X2);
        }
        self.gp_commit(res, d);
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
            A64Loc::Fp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => unreachable!(),
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
            A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => {
                unreachable!("aarch64: expected FP val")
            }
        }
    }

    fn fp_write_reg(&self, v: Val) -> VReg {
        match self.loc(v) {
            A64Loc::Fp(r) => r,
            A64Loc::Spill(_) => SFP1,
            A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => unreachable!(),
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
            A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => unreachable!(),
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
    fn trap_alloc_lbl(&mut self) -> Label {
        *self.trap_alloc.get_or_insert_with(|| self.asm.new_label())
    }
    fn trap_unaligned_lbl(&mut self) -> Label {
        *self
            .trap_unaligned
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

    #[inline]
    fn frame_caller_ret_off(&self) -> u32 {
        debug_assert!(self.alloc.has_calls);
        (self.alloc.saved_fp_lr_off + 8) as u32
    }

    /// Records a machine-code GC safepoint at the current `asm.pos()` (immediately after a `bl`/`blr`).
    fn record_gc_safepoint(&mut self, inst_idx: u32) -> Result<(), String> {
        if !self.ir.uses_gc {
            return Ok(());
        }
        let plan = self
            .alloc
            .safepoint_for(self.cur_bb, inst_idx)
            .ok_or_else(|| {
                format!(
                    "aarch64: missing SafepointPlan for fn {} bb{} inst {inst_idx}",
                    self.ir.func_index, self.cur_bb
                )
            })?;
        self.asm.record_safepoint(
            self.frame_caller_ret_off(),
            plan.gc_slots.clone(),
            plan.ext_slots.clone(),
        );
        Ok(())
    }

    /// Publishes caller's overflow return buffer pointer (`reg`, must not be `SGP3`).
    fn emit_publish_ret_spill(&mut self, reg: Reg) {
        if self.layout.is_store {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(reg),
                None,
                SGP3,
                STORE_OFF_RET_SPILL_PTR,
                SGP3,
            );
        } else {
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(reg),
                None,
                CTX_REG,
                OFF_RET_SPILL_PTR,
                SGP3,
            );
        }
    }

    /// Saves caller's `CTX_REG` (`X28`) and calls imported function `f` via `StoreFuncRef*`,
    /// swapping `X28/X27/X26` while preserving all Wasm argument registers (`X0..X7, V0..V7`).
    fn emit_store_call_import(&mut self, f: u32) {
        let slot = self
            .alloc
            .store_saved_ctx_off
            .expect("store_saved_ctx_off missing");
        emit_store_off(
            &mut self.asm,
            Ty::I64,
            Some(CTX_REG),
            None,
            Reg::SP,
            slot,
            SGP3,
        );
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            OFF_STORE_FUNC_REFS,
            SGP3,
        );
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            SGP1,
            (f as i32) * 8,
            SGP3,
        );
        self.asm.ldr_uimm(true, CTX_REG, SGP1, 8);
        self.asm
            .ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
        self.asm
            .ldr_uimm(true, SGP2, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
        self.asm.ldr_uimm(true, MEM_LEN_REG, SGP2, 8);
        self.asm.ldr_uimm(true, SGP1, SGP1, 0);
        self.asm.blr(SGP1);
    }

    /// Restores caller's `X28`, `X27`, and `X26` immediately after a cross-module or indirect call.
    /// Uses ONLY `X28, X27, X26, SGP2, SGP3` — preserves `W16` (`SGP1` trap code) and all return regs!
    fn emit_store_restore_ctx(&mut self) {
        let slot = self
            .alloc
            .store_saved_ctx_off
            .expect("store_saved_ctx_off missing");
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(CTX_REG),
            None,
            Reg::SP,
            slot,
            SGP3,
        );
        self.asm
            .ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
        self.asm
            .ldr_uimm(true, SGP2, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
        self.asm.ldr_uimm(true, MEM_LEN_REG, SGP2, 8);
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
        if self.alloc.has_calls {
            let fp_lr_off = self.alloc.saved_fp_lr_off;
            if (fp_lr_off & 7) == 0 && (-512..=504).contains(&fp_lr_off) {
                self.asm.ldp_x(Reg::FP, Reg::LR, Reg::SP, fp_lr_off);
            } else {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::FP),
                    None,
                    Reg::SP,
                    fp_lr_off,
                    SGP3,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::LR),
                    None,
                    Reg::SP,
                    fp_lr_off + 8,
                    SGP3,
                );
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

    /// Emits a bounds check for a linear-memory 0 access `[addr + offset .. addr + offset + access_bytes)`.
    ///
    /// Under `spectre_mitigations`, uses branchless ARMv8 `csel(..., xzr, ls) + csdb` index
    /// clamping when `mem_min_bytes >= access_bytes`, keeping all speculative accesses strictly
    /// inside `[MEM_BASE_REG .. MEM_BASE_REG + mem_min_bytes)` without a full `dsb nsh; isb` flush.
    fn emit_bounds_check_for_access(
        &mut self,
        addr: Reg,
        offset: u64,
        access_bytes: u64,
    ) -> (Reg, u64) {
        if !self.layout.spectre_mitigations || (self.layout.is_store && self.module.memory_shared) {
            self.emit_bounds_check(addr, offset.saturating_add(access_bytes));
            return (addr, offset);
        }
        let oob = self.trap_oob_lbl();
        let Some(end_off) = offset.checked_add(access_bytes) else {
            self.asm.b(oob);
            return (addr, offset);
        };
        if !self.layout.memory64 {
            self.asm.uxtw(SGP2, addr);
        } else {
            self.asm.mov_r(true, SGP2, addr);
        }
        // Case 1: Fixed-size memory (`min == max`) with small offset (`< 16 MiB`).
        if let Some(cap) = self.layout.fixed_mem_bytes {
            if end_off > cap {
                self.asm.b(oob);
                return (addr, offset);
            }
            if access_bytes >= 1 && offset < (1u64 << 24) {
                let max_addr = cap - end_off;
                if max_addr < 4096 {
                    self.asm.cmp_imm(true, SGP2, max_addr as u16);
                } else {
                    self.asm.mov_imm64(SGP3, max_addr);
                    self.asm.cmp_r(true, SGP2, SGP3);
                }
                self.asm.b_cond(Cond::Hi, oob);
                self.asm.csel(true, SGP2, SGP2, Reg::XZR, Cond::Ls);
                self.asm.csdb();
                return (SGP2, offset);
            }
            self.emit_bounds_check(addr, end_off);
            return (addr, offset);
        }
        // Case 2: Dynamic memory where `end_off <= mem_min_bytes` and `offset < 16 MiB`.
        if access_bytes >= 1 && end_off <= self.layout.mem_min_bytes && offset < (1u64 << 24) {
            if end_off < 4096 {
                self.asm.sub_imm(true, SGP3, MEM_LEN_REG, end_off as u16);
            } else {
                self.asm.mov_imm64(SGP3, end_off);
                self.asm.sub_r(true, SGP3, MEM_LEN_REG, SGP3);
            }
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Hi, oob);
            self.asm.csel(true, SGP2, SGP2, Reg::XZR, Cond::Ls);
            self.asm.csdb();
            return (SGP2, offset);
        }
        // Case 3: Dynamic memory with `mem_min_bytes >= access_bytes` (large offset):
        // check & clamp the full effective byte offset `eff = zext(addr) + offset` to `0`.
        if (1..4096).contains(&access_bytes) && self.layout.mem_min_bytes >= access_bytes {
            if offset != 0 {
                if self.layout.memory64 || offset > u32::MAX as u64 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.adds_r(true, SGP2, SGP2, SGP3);
                    self.asm.b_cond(Cond::Hs, oob);
                    self.asm.csel(true, SGP2, SGP2, Reg::XZR, Cond::Lo);
                    self.asm.csdb();
                } else if offset < 4096 {
                    self.asm.add_imm(true, SGP2, SGP2, offset as u16);
                } else {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP2, SGP2, SGP3);
                }
            }
            self.asm
                .sub_imm(true, SGP3, MEM_LEN_REG, access_bytes as u16);
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Hi, oob);
            self.asm.csel(true, SGP2, SGP2, Reg::XZR, Cond::Ls);
            self.asm.csdb();
            return (SGP2, 0);
        }
        // Case 4: `min_pages == 0` — fall back to `dsb nsh; isb`.
        self.emit_bounds_check(addr, end_off);
        (addr, offset)
    }

    /// Checks bounds for an extra-memory (`mem >= 1`) access and computes the effective host pointer
    /// into `SGP2`, using branchless `csel + csdb` clamping under `spectre_mitigations` when
    /// `mem_min_bytes >= access_bytes`.
    fn emit_extra_mem_ptr(&mut self, mem: u32, addr: Val, offset: u64, access_bytes: u64) {
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
        if self.layout.is_store {
            if offset != 0 {
                if offset < 4096 && !is64 {
                    self.asm.add_imm(true, SGP1, SGP1, offset as u16);
                } else {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.adds_r(true, SGP1, SGP1, SGP3);
                    if is64 || offset > u32::MAX as u64 {
                        self.asm.b_cond(Cond::Hs, oob);
                    }
                }
            }
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_STORE_MEM_DESCS,
                SGP2,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                SGP3,
                (mem as i32) * 8,
                SGP2,
            );
            self.asm.mov_imm64(SGP2, access_bytes);
            self.asm.adds_r(true, SGP2, SGP1, SGP2);
            if is64 || offset.saturating_add(access_bytes) > u32::MAX as u64 {
                self.asm.b_cond(Cond::Hs, oob);
            }
            // Compare end (SGP2) against desc->len ([SGP3 + 8]).
            self.asm.ldr_uimm(true, SGP3, SGP3, 8);
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Hi, oob);
            self.emit_speculation_barrier();
            // Reload desc->base ([ [CTX_REG + OFF_STORE_MEM_DESCS][mem] + 0 ]) and add SGP1.
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_STORE_MEM_DESCS,
                SGP2,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                SGP3,
                (mem as i32) * 8,
                SGP2,
            );
            self.asm.ldr_uimm(true, SGP2, SGP3, 0);
            self.asm.add_r(true, SGP2, SGP2, SGP1);
            return;
        }
        let desc_off = self.layout.mem_desc_off(mem);
        if self.layout.spectre_mitigations
            && (1..4096).contains(&access_bytes)
            && self.layout.mem_min_bytes_idx(mem) >= access_bytes
        {
            if offset != 0 {
                if is64 || offset > u32::MAX as u64 {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.adds_r(true, SGP1, SGP1, SGP3);
                    self.asm.b_cond(Cond::Hs, oob);
                    self.asm.csel(true, SGP1, SGP1, Reg::XZR, Cond::Lo);
                    self.asm.csdb();
                } else if offset < 4096 {
                    self.asm.add_imm(true, SGP1, SGP1, offset as u16);
                } else {
                    self.asm.mov_imm64(SGP3, offset);
                    self.asm.add_r(true, SGP1, SGP1, SGP3);
                }
            }
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                CTX_REG,
                desc_off + 8,
                SGP3,
            );
            self.asm.sub_imm(true, SGP2, SGP2, access_bytes as u16);
            self.asm.cmp_r(true, SGP1, SGP2);
            self.asm.b_cond(Cond::Hi, oob);
            self.asm.csel(true, SGP1, SGP1, Reg::XZR, Cond::Ls);
            self.asm.csdb();
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
            return;
        }
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
            .mov_imm64(SGP3, offset.saturating_add(access_bytes));
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
    }

    fn emit_bounds_check(&mut self, addr: Reg, end_off: u64) {
        self.emit_bounds_check_maybe_barrier(addr, end_off, true);
    }

    fn emit_bounds_check_maybe_barrier(&mut self, addr: Reg, end_off: u64, with_barrier: bool) {
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
            if with_barrier {
                self.emit_speculation_barrier();
            }
            return;
        }
        if end_off == 0 {
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            if self.layout.is_store && self.module.memory_shared {
                let lbl_ok = self.asm.new_label();
                self.asm.b_cond(Cond::Ls, lbl_ok);
                self.asm
                    .ldr_uimm(true, MEM_LEN_REG, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
                self.asm.ldr_uimm(true, MEM_LEN_REG, MEM_LEN_REG, 8);
                self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
                self.asm.b_cond(Cond::Hi, oob);
                self.asm.bind(lbl_ok);
            } else {
                self.asm.b_cond(Cond::Hi, oob);
            }
            if with_barrier {
                self.emit_speculation_barrier();
            }
            return;
        }
        if self.layout.memory64 {
            // Fast 1-branch wrap-proof check when `mem_min_bytes >= end_off` (so `MEM_LEN_REG - end_off`
            // cannot underflow, because linear memory never shrinks): `addr <=u (MEM_LEN_REG - end_off)`!
            if !self.module.memory_shared && end_off <= self.layout.mem_min_bytes {
                if end_off < 4096 {
                    self.asm.sub_imm(true, SGP3, MEM_LEN_REG, end_off as u16);
                } else {
                    self.asm.mov_imm64(SGP3, end_off);
                    self.asm.sub_r(true, SGP3, MEM_LEN_REG, SGP3);
                }
                self.asm.cmp_r(true, SGP2, SGP3);
                self.asm.b_cond(Cond::Hi, oob);
                if with_barrier {
                    self.emit_speculation_barrier();
                }
                return;
            }
            self.asm.mov_imm64(SGP3, end_off);
            self.asm.adds_r(true, SGP2, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, oob); // carry set => 64-bit unsigned overflow!
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            if self.layout.is_store && self.module.memory_shared {
                let lbl_ok = self.asm.new_label();
                self.asm.b_cond(Cond::Ls, lbl_ok);
                self.asm
                    .ldr_uimm(true, MEM_LEN_REG, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
                self.asm.ldr_uimm(true, MEM_LEN_REG, MEM_LEN_REG, 8);
                self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
                self.asm.b_cond(Cond::Hi, oob);
                self.asm.bind(lbl_ok);
            } else {
                self.asm.b_cond(Cond::Hi, oob);
            }
            if with_barrier {
                self.emit_speculation_barrier();
            }
            return;
        }
        if end_off < 4096 {
            self.asm.add_imm(true, SGP2, SGP2, end_off as u16);
        } else {
            self.asm.mov_imm64(SGP3, end_off);
            self.asm.add_r(true, SGP2, SGP2, SGP3);
        }
        self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
        if self.layout.is_store && self.module.memory_shared {
            let lbl_ok = self.asm.new_label();
            self.asm.b_cond(Cond::Ls, lbl_ok);
            self.asm
                .ldr_uimm(true, MEM_LEN_REG, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
            self.asm.ldr_uimm(true, MEM_LEN_REG, MEM_LEN_REG, 8);
            self.asm.cmp_r(true, SGP2, MEM_LEN_REG);
            self.asm.b_cond(Cond::Hi, oob);
            self.asm.bind(lbl_ok);
        } else {
            self.asm.b_cond(Cond::Hi, oob);
        }
        if with_barrier {
            self.emit_speculation_barrier();
        }
    }

    /// Computes and validates the effective host pointer for an atomic access of `width` bytes
    /// on memory `mem`, checking natural alignment (`TRAP_UNALIGNED_ATOMIC`) and live bounds
    /// (`TRAP_OOB`). Leaves the host pointer in `SGP1 (X16)`.
    fn emit_atomic_ea(&mut self, mem: u32, addr: Val, offset: u64, width: u32) {
        let is64 = self.module.mem_is_64(mem);
        let trap_oob = self.trap_oob_lbl();
        let ar = self.gp_read(addr, SGP1);
        if is64 {
            if ar != SGP1 {
                self.asm.mov_r(true, SGP1, ar);
            }
        } else {
            self.asm.uxtw(SGP1, ar);
        }
        if offset != 0 {
            if is64 || offset > u32::MAX as u64 {
                self.asm.mov_imm64(SGP2, offset);
                self.asm.adds_r(true, SGP1, SGP1, SGP2);
                self.asm.b_cond(Cond::Hs, trap_oob);
            } else if offset < 4096 {
                self.asm.add_imm(true, SGP1, SGP1, offset as u16);
            } else {
                self.asm.mov_imm64(SGP2, offset);
                self.asm.add_r(true, SGP1, SGP1, SGP2);
            }
        }
        if width > 1 {
            let trap_unaligned = self.trap_unaligned_lbl();
            let k = width.trailing_zeros() as u8;
            self.asm.tst_imm_low_bits(false, SGP1, k);
            self.asm.b_cond(Cond::Ne, trap_unaligned);
        }
        self.asm.mov_imm32(SGP2, width);
        self.asm.adds_r(true, SGP2, SGP1, SGP2);
        if is64 {
            self.asm.b_cond(Cond::Hs, trap_oob);
        }
        if self.layout.is_store {
            if mem == 0 {
                self.asm
                    .ldr_uimm(true, SGP3, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
                self.asm.ldr_uimm(true, SGP3, SGP3, 8);
                if self.module.memory_shared {
                    self.asm.mov_r(true, MEM_LEN_REG, SGP3);
                }
                self.asm.cmp_r(true, SGP2, SGP3);
                self.asm.b_cond(Cond::Hi, trap_oob);
                self.emit_speculation_barrier();
                self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1);
            } else {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP3),
                    None,
                    CTX_REG,
                    OFF_STORE_MEM_DESCS,
                    SGP3,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP3),
                    None,
                    SGP3,
                    (mem as i32) * 8,
                    SGP3,
                );
                self.asm.ldr_uimm(true, SGP3, SGP3, 8);
                self.asm.cmp_r(true, SGP2, SGP3);
                self.asm.b_cond(Cond::Hi, trap_oob);
                self.emit_speculation_barrier();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP3),
                    None,
                    CTX_REG,
                    OFF_STORE_MEM_DESCS,
                    SGP3,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP3),
                    None,
                    SGP3,
                    (mem as i32) * 8,
                    SGP3,
                );
                self.asm.ldr_uimm(true, SGP2, SGP3, 0);
                self.asm.add_r(true, SGP1, SGP2, SGP1);
            }
            return;
        }
        if mem == 0 {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_MEM_LEN,
                SGP3,
            );
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Hi, trap_oob);
            self.emit_speculation_barrier();
            self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1);
        } else {
            let desc_off = self.layout.mem_desc_off(mem);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                desc_off + 8,
                SGP3,
            );
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Hi, trap_oob);
            self.emit_speculation_barrier();
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                desc_off,
                SGP3,
            );
            self.asm.add_r(true, SGP1, SGP3, SGP1);
        }
    }

    /// Writes the 32-bit (or zero-extended 64-bit for structs) GC object header word to `[obj_reg + 0]`.
    /// Clobbers ONLY `SGP1 (X16)` and `SGP3 (X24)`.
    fn emit_write_gc_header(&mut self, obj_reg: Reg, gt: &GcCanonType, is_struct_64: bool) {
        if self.layout.is_store {
            let kind_bits = match gt.composite {
                GcCompositeInfo::Struct { .. } => GC_HDR_KIND_STRUCT,
                GcCompositeInfo::Array { .. } => GC_HDR_KIND_ARRAY,
                GcCompositeInfo::Func => unreachable!(),
            };
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_CANON_MAP,
                SGP3,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I32,
                Some(SGP1),
                None,
                SGP1,
                (gt.canon_id as i32) * 4,
                SGP3,
            );
            self.asm.lsl_imm(false, SGP1, SGP1, 3);
            if kind_bits == GC_HDR_KIND_ARRAY {
                self.asm.orr_imm_2_w(SGP1, SGP1);
            } else if kind_bits != 0 {
                self.asm.mov_imm32(SGP3, kind_bits);
                self.asm.orr_r(false, SGP1, SGP1, SGP3);
            }
            self.asm.str_uimm(is_struct_64, SGP1, obj_reg, 0);
        } else if is_struct_64 {
            self.asm.mov_imm64(SGP1, u64::from(gt.header_word()));
            self.asm.str_uimm(true, SGP1, obj_reg, 0);
        } else {
            self.asm.mov_imm32(SGP1, gt.header_word());
            self.asm.str_uimm(false, SGP1, obj_reg, 0);
        }
    }

    /// Inline bump-pointer allocation of a fixed `size_bytes` (multiple of 8, `>= 8`) GC object
    /// with a cold preserve-all `HELPER_GC_COLLECT` safepoint.
    /// Returns the new object pointer in `SGP2 (X17)` while preserving all allocatable GP/FP regs.
    fn emit_gc_alloc_fixed(&mut self, size_bytes: u32) -> Result<(), String> {
        let gc_off = self
            .layout
            .gc
            .as_ref()
            .ok_or("aarch64: GC alloc without GcCtxLayout")?
            .gc_state_off;
        let lbl_have = self.asm.new_label();
        let trap_alloc = self.trap_alloc_lbl();
        let sz_i32 =
            i32::try_from(size_bytes).map_err(|_| "aarch64: fixed GC alloc size exceeds i32")?;
        if self.layout.is_store {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            self.asm.ldr_uimm(true, SGP2, SGP1, GC_OFF_ALLOC_PTR as u32);
            self.asm.add_i32_checked(true, SGP2, SGP2, sz_i32, SGP3);
            self.asm
                .ldr_uimm(true, SGP3, SGP1, GC_OFF_ALLOC_LIMIT as u32);
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Ls, lbl_have);
            self.asm.mov_imm64(SGP2, u64::from(size_bytes));
            self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
            self.record_gc_safepoint(self.cur_inst)?;
            self.asm.cbnz(false, SGP1, trap_alloc);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            self.asm.ldr_uimm(true, SGP2, SGP1, GC_OFF_ALLOC_PTR as u32);
            self.asm.add_i32_checked(true, SGP2, SGP2, sz_i32, SGP3);
            self.asm.bind(lbl_have);
            self.asm.str_uimm(true, SGP2, SGP1, GC_OFF_ALLOC_PTR as u32);
            self.asm.add_i32_checked(true, SGP2, SGP2, -sz_i32, SGP3);
            return Ok(());
        }
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        self.asm.add_i32_checked(true, SGP1, SGP2, sz_i32, SGP3);
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP3),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_LIMIT,
            SGP3,
        );
        self.asm.cmp_r(true, SGP1, SGP3);
        self.asm.b_cond(Cond::Ls, lbl_have);
        self.asm.mov_imm64(SGP2, u64::from(size_bytes));
        self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
        self.record_gc_safepoint(self.cur_inst)?;
        self.asm.cbnz(false, SGP1, trap_alloc);
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        self.asm.add_i32_checked(true, SGP1, SGP2, sz_i32, SGP3);
        self.asm.bind(lbl_have);
        emit_store_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        Ok(())
    }

    /// Inline bump-pointer allocation of a dynamic-length GC array (`align8(8 + len * elem_sz)`)
    /// with a cold preserve-all `HELPER_GC_COLLECT` safepoint, preserving ALL allocatable regs.
    /// Stores the freshly allocated `new_obj` pointer into `[CTX_REG + OFF_SCRATCH_TMP]`.
    fn emit_gc_alloc_array_dyn(&mut self, len_val: Val, elem_sz: u32) -> Result<(), String> {
        let gc_off = self
            .layout
            .gc
            .as_ref()
            .ok_or("aarch64: GC array alloc without GcCtxLayout")?
            .gc_state_off;
        let lbl_have = self.asm.new_label();
        let trap_alloc = self.trap_alloc_lbl();
        let lr = self.gp_read(len_val, SGP1);
        self.asm.uxtw(SGP2, lr);
        if elem_sz > 1 {
            debug_assert!(elem_sz.is_power_of_two());
            self.asm
                .lsl_imm(true, SGP2, SGP2, elem_sz.trailing_zeros() as u8);
        }
        self.asm.add_imm(true, SGP2, SGP2, 15);
        self.asm.and_imm_align8(SGP2, SGP2);
        emit_store_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_SCRATCH_TMP,
            SGP3,
        );

        if self.layout.is_store {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            self.asm.ldr_uimm(true, SGP3, SGP1, GC_OFF_ALLOC_PTR as u32);
            self.asm.adds_r(true, SGP2, SGP3, SGP2);
            self.asm.b_cond(Cond::Hs, trap_alloc);
            self.asm
                .ldr_uimm(true, SGP3, SGP1, GC_OFF_ALLOC_LIMIT as u32);
            self.asm.cmp_r(true, SGP2, SGP3);
            self.asm.b_cond(Cond::Ls, lbl_have);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
            self.record_gc_safepoint(self.cur_inst)?;
            self.asm.cbnz(false, SGP1, trap_alloc);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            self.asm.ldr_uimm(true, SGP2, SGP1, GC_OFF_ALLOC_PTR as u32);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            self.asm.add_r(true, SGP2, SGP2, SGP3);
            self.asm.bind(lbl_have);
            self.asm.str_uimm(true, SGP2, SGP1, GC_OFF_ALLOC_PTR as u32);
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP3),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            self.asm.sub_r(true, SGP2, SGP2, SGP3);
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP2),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            return Ok(());
        }

        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        self.asm.adds_r(true, SGP1, SGP1, SGP2);
        self.asm.b_cond(Cond::Hs, trap_alloc);
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP3),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_LIMIT,
            SGP3,
        );
        self.asm.cmp_r(true, SGP1, SGP3);
        self.asm.b_cond(Cond::Ls, lbl_have);
        self.asm.bl_reloc(A64RelocTarget::Helper(HELPER_GC_COLLECT));
        self.record_gc_safepoint(self.cur_inst)?;
        self.asm.cbnz(false, SGP1, trap_alloc);
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_SCRATCH_TMP,
            SGP3,
        );
        self.asm.add_r(true, SGP1, SGP1, SGP2);
        self.asm.bind(lbl_have);
        emit_store_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            gc_off + GC_OFF_ALLOC_PTR,
            SGP3,
        );
        emit_load_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP2),
            None,
            CTX_REG,
            OFF_SCRATCH_TMP,
            SGP3,
        );
        self.asm.sub_r(true, SGP1, SGP1, SGP2);
        emit_store_off(
            &mut self.asm,
            Ty::I64,
            Some(SGP1),
            None,
            CTX_REG,
            OFF_SCRATCH_TMP,
            SGP3,
        );
        Ok(())
    }

    /// Checks whether `actual_cid` (in `SGP2`, zero-extended u32) is a subtype of
    /// `(target_canon_id, target_depth)` using O(1) Cohen display / overflow table lookup.
    /// Clobbers `SGP1`, `SGP2`, `SGP3`; jumps to `lbl_false` on mismatch.
    fn emit_gc_canon_subtype_check(
        &mut self,
        type_desc_ro_off: u32,
        target_canon_id: u32,
        target_depth: u32,
        lbl_false: Label,
    ) {
        self.asm.lsl_imm(true, SGP2, SGP2, 6); // actual_cid * 64
        if self.layout.is_store {
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                SGP1,
                STORE_OFF_TYPE_DESCS_PTR,
                SGP3,
            );
            self.asm.add_r(true, SGP2, SGP1, SGP2); // &StoreTypeDesc[actual_cid]
            if (target_depth as usize) < GC_DISPLAY_LEN {
                let disp_off = 16 + target_depth * 4;
                self.asm.ldr_uimm(false, SGP2, SGP2, disp_off);
            } else {
                self.asm.ldr_uimm(false, SGP1, SGP2, 8); // actual_depth
                self.asm.mov_imm32(SGP3, target_depth);
                self.asm.cmp_r(false, SGP1, SGP3);
                self.asm.b_cond(Cond::Lo, lbl_false);
                self.asm.ldr_uimm(true, SGP2, SGP2, 56); // supertypes_all_ptr
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(SGP2),
                    None,
                    SGP2,
                    (target_depth as i32) * 4,
                    SGP3,
                );
            }
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_CANON_MAP,
                SGP3,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I32,
                Some(SGP1),
                None,
                SGP1,
                (target_canon_id as i32) * 4,
                SGP3,
            );
            self.asm.cmp_r(false, SGP2, SGP1);
            self.asm.b_cond(Cond::Ne, lbl_false);
            return;
        }
        self.asm
            .adr_reloc(SGP1, A64RelocTarget::Rodata(type_desc_ro_off));
        self.asm.add_r(true, SGP2, SGP1, SGP2); // &TypeDesc[actual_cid]
        if (target_depth as usize) < GC_DISPLAY_LEN {
            let disp_off = 24 + target_depth * 4;
            self.asm.ldr_uimm(false, SGP2, SGP2, disp_off);
            self.asm.mov_imm32(SGP1, target_canon_id);
            self.asm.cmp_r(false, SGP2, SGP1);
            self.asm.b_cond(Cond::Ne, lbl_false);
        } else {
            self.asm.ldr_uimm(false, SGP1, SGP2, 20); // actual_depth
            self.asm.mov_imm32(SGP3, target_depth);
            self.asm.cmp_r(false, SGP1, SGP3);
            self.asm.b_cond(Cond::Lo, lbl_false);
            self.asm.ldrsw_uimm(SGP2, SGP2, 16); // super_overflow_rel
            self.asm
                .adr_reloc(SGP1, A64RelocTarget::Rodata(type_desc_ro_off));
            self.asm.add_r(true, SGP2, SGP1, SGP2);
            emit_load_off(
                &mut self.asm,
                Ty::I32,
                Some(SGP1),
                None,
                SGP2,
                (target_depth as i32) * 4,
                SGP3,
            );
            self.asm.mov_imm32(SGP2, target_canon_id);
            self.asm.cmp_r(false, SGP1, SGP2);
            self.asm.b_cond(Cond::Ne, lbl_false);
        }
    }

    /// Tests whether non-null `val_reg` matches `target`, jumping to `lbl_false` on mismatch.
    /// Uses ONLY `SGP1`, `SGP2`, `SGP3`.
    fn emit_gc_nonnull_matches_target(
        &mut self,
        val_reg: Reg,
        target: GcHeapTarget,
        lbl_false: Label,
    ) -> Result<(), String> {
        let gc = self
            .layout
            .gc
            .as_ref()
            .ok_or("aarch64: GC type test without GcCtxLayout")?;
        match target {
            GcHeapTarget::Any | GcHeapTarget::Extern | GcHeapTarget::Func | GcHeapTarget::Exn => {}
            GcHeapTarget::None
            | GcHeapTarget::NoFunc
            | GcHeapTarget::NoExtern
            | GcHeapTarget::NoExn => {
                self.asm.b(lbl_false);
            }
            GcHeapTarget::I31 => {
                self.asm.tst_imm_low_bits(false, val_reg, 1);
                self.asm.b_cond(Cond::Eq, lbl_false);
            }
            GcHeapTarget::Eq => {
                let lbl_match = self.asm.new_label();
                self.asm.tst_imm_low_bits(false, val_reg, 1);
                self.asm.b_cond(Cond::Ne, lbl_match);
                self.asm.ldr_uimm(false, SGP1, val_reg, 0);
                self.asm.and_imm_6_w(SGP1, SGP1);
                self.asm
                    .cmp_imm(false, SGP1, GC_HDR_KIND_INTERNAL_EXTERN as u16);
                self.asm.b_cond(Cond::Eq, lbl_false);
                self.asm.bind(lbl_match);
            }
            GcHeapTarget::Struct => {
                self.asm.tst_imm_low_bits(false, val_reg, 1);
                self.asm.b_cond(Cond::Ne, lbl_false);
                self.asm.ldr_uimm(false, SGP1, val_reg, 0);
                self.asm.and_imm_6_w(SGP1, SGP1);
                self.asm.cmp_imm(false, SGP1, GC_HDR_KIND_STRUCT as u16);
                self.asm.b_cond(Cond::Ne, lbl_false);
            }
            GcHeapTarget::Array => {
                self.asm.tst_imm_low_bits(false, val_reg, 1);
                self.asm.b_cond(Cond::Ne, lbl_false);
                self.asm.ldr_uimm(false, SGP1, val_reg, 0);
                self.asm.and_imm_6_w(SGP1, SGP1);
                self.asm.cmp_imm(false, SGP1, GC_HDR_KIND_ARRAY as u16);
                self.asm.b_cond(Cond::Ne, lbl_false);
            }
            GcHeapTarget::Concrete { canon_id, is_func } => {
                let gt = &self.module.gc_canon_types[canon_id as usize];
                if is_func {
                    if self.layout.is_store {
                        self.asm.ldr_uimm(false, SGP2, val_reg, 16);
                        if gt.is_final {
                            emit_load_off(
                                &mut self.asm,
                                Ty::I64,
                                Some(SGP1),
                                None,
                                CTX_REG,
                                OFF_STORE_CANON_MAP,
                                SGP3,
                            );
                            emit_load_off(
                                &mut self.asm,
                                Ty::I32,
                                Some(SGP1),
                                None,
                                SGP1,
                                (gt.canon_id as i32) * 4,
                                SGP3,
                            );
                            self.asm.cmp_r(false, SGP2, SGP1);
                            self.asm.b_cond(Cond::Ne, lbl_false);
                        } else {
                            self.emit_gc_canon_subtype_check(
                                gc.type_desc_ro_off,
                                gt.canon_id,
                                gt.depth,
                                lbl_false,
                            );
                        }
                    } else {
                        self.asm.sub_imm(true, SGP2, val_reg, 1);
                        self.asm
                            .adr_reloc(SGP1, A64RelocTarget::Rodata(gc.func_canon_ro_off));
                        self.asm.ldr_reg_scaled(false, SGP2, SGP1, SGP2, true);
                        if gt.is_final {
                            self.asm.mov_imm32(SGP1, gt.canon_id);
                            self.asm.cmp_r(false, SGP2, SGP1);
                            self.asm.b_cond(Cond::Ne, lbl_false);
                        } else {
                            self.emit_gc_canon_subtype_check(
                                gc.type_desc_ro_off,
                                gt.canon_id,
                                gt.depth,
                                lbl_false,
                            );
                        }
                    }
                } else {
                    let expected_kind = match gt.composite {
                        GcCompositeInfo::Struct { .. } => GC_HDR_KIND_STRUCT,
                        GcCompositeInfo::Array { .. } => GC_HDR_KIND_ARRAY,
                        GcCompositeInfo::Func => unreachable!(),
                    };
                    self.asm.tst_imm_low_bits(false, val_reg, 1);
                    self.asm.b_cond(Cond::Ne, lbl_false);
                    self.asm.ldr_uimm(false, SGP2, val_reg, 0);
                    if gt.is_final {
                        if self.layout.is_store {
                            emit_load_off(
                                &mut self.asm,
                                Ty::I64,
                                Some(SGP1),
                                None,
                                CTX_REG,
                                OFF_STORE_CANON_MAP,
                                SGP3,
                            );
                            emit_load_off(
                                &mut self.asm,
                                Ty::I32,
                                Some(SGP1),
                                None,
                                SGP1,
                                (gt.canon_id as i32) * 4,
                                SGP3,
                            );
                            self.asm.lsl_imm(false, SGP1, SGP1, 3);
                            if expected_kind == GC_HDR_KIND_ARRAY {
                                self.asm.orr_imm_2_w(SGP1, SGP1);
                            } else if expected_kind != 0 {
                                self.asm.mov_imm32(SGP3, expected_kind);
                                self.asm.orr_r(false, SGP1, SGP1, SGP3);
                            }
                            self.asm.cmp_r(false, SGP2, SGP1);
                            self.asm.b_cond(Cond::Ne, lbl_false);
                        } else {
                            self.asm.mov_imm32(SGP1, gt.header_word());
                            self.asm.cmp_r(false, SGP2, SGP1);
                            self.asm.b_cond(Cond::Ne, lbl_false);
                        }
                    } else {
                        self.asm.and_imm_6_w(SGP1, SGP2);
                        self.asm.cmp_imm(false, SGP1, expected_kind as u16);
                        self.asm.b_cond(Cond::Ne, lbl_false);
                        self.asm.lsr_imm(false, SGP2, SGP2, 3);
                        self.emit_gc_canon_subtype_check(
                            gc.type_desc_ro_off,
                            gt.canon_id,
                            gt.depth,
                            lbl_false,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Emits the generational GC card-marking write barrier for `StoreCdylib` on AArch64.
    /// Uses only `SGP1` and `SGP3` (never clobbers `obj_reg` or `SGP2`).
    fn emit_store_gc_write_barrier(&mut self, obj_reg: Reg, check_null: bool) {
        if !self.layout.is_store {
            return;
        }
        debug_assert_ne!(obj_reg, SGP1);
        debug_assert_ne!(obj_reg, SGP3);
        let lbl_done = self.asm.new_label();
        if check_null {
            self.asm.cbz(true, obj_reg, lbl_done);
        }
        self.asm.ldr_uimm(true, SGP1, CTX_REG, OFF_STORE_PTR as u32);
        self.asm.ldr_uimm(true, SGP3, SGP1, GC_OFF_TO_CAP as u32);
        self.asm.cmp_r(true, obj_reg, SGP3);
        self.asm.b_cond(Cond::Hs, lbl_done);
        self.asm.ldr_uimm(true, SGP3, SGP1, GC_OFF_FROM_BASE as u32);
        self.asm.sub_r(true, SGP3, obj_reg, SGP3);
        self.asm.lsr_imm(true, SGP3, SGP3, 9);
        self.asm.ldr_uimm(true, SGP1, SGP1, GC_OFF_TO_BASE as u32);
        self.asm.add_r(true, SGP1, SGP1, SGP3);
        self.asm.movz(false, SGP3, 1, 0);
        self.asm.strb_uimm(SGP3, SGP1, 0);
        self.asm.bind(lbl_done);
    }

    /// Stores a GC struct/array field of `storage` kind from SSA `val` into `[base + off]`.
    /// Must NOT clobber `base` (caller passes `SGP2` or an allocatable GP reg; uses `SGP1`/`SGP3`/`SFP1`).
    fn emit_gc_storage_store(&mut self, base: Reg, off: i32, storage: GcStorageKind, val: Val) {
        match storage {
            GcStorageKind::I8 => {
                let vr = self.gp_read(val, SGP1);
                if (0..=4095).contains(&off) {
                    self.asm.strb_uimm(vr, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.strb_uimm(vr, SGP3, 0);
                }
            }
            GcStorageKind::I16 => {
                let vr = self.gp_read(val, SGP1);
                if (0..=8190).contains(&off) && (off & 1) == 0 {
                    self.asm.strh_uimm(vr, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.strh_uimm(vr, SGP3, 0);
                }
            }
            GcStorageKind::I32 => {
                let vr = self.gp_read(val, SGP1);
                emit_store_off(&mut self.asm, Ty::I32, Some(vr), None, base, off, SGP3);
            }
            GcStorageKind::I64
            | GcStorageKind::GcRef
            | GcStorageKind::ExternRef
            | GcStorageKind::OtherRef => {
                let vr = self.gp_read(val, SGP1);
                emit_store_off(&mut self.asm, Ty::I64, Some(vr), None, base, off, SGP3);
            }
            GcStorageKind::F32 => {
                let fx = self.fp_read(val, SFP1);
                emit_store_off(&mut self.asm, Ty::F32, None, Some(fx), base, off, SGP3);
            }
            GcStorageKind::F64 => {
                let fx = self.fp_read(val, SFP1);
                emit_store_off(&mut self.asm, Ty::F64, None, Some(fx), base, off, SGP3);
            }
            GcStorageKind::V128 => {
                let fx = self.fp_read(val, SFP1);
                emit_store_off(&mut self.asm, Ty::V128, None, Some(fx), base, off, SGP3);
            }
        }
    }

    /// Loads a GC struct/array field of `kind` from `[base + off]` into SSA result `r`.
    fn emit_gc_field_load(&mut self, r: Val, base: Reg, off: i32, kind: GcFieldLoadKind) {
        match kind {
            GcFieldLoadKind::I8S => {
                let d = self.gp_write_reg(r);
                if (0..=4095).contains(&off) {
                    self.asm.ldrsb_uimm(false, d, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.ldrsb_uimm(false, d, SGP3, 0);
                }
                self.gp_commit(r, d);
            }
            GcFieldLoadKind::I8U => {
                let d = self.gp_write_reg(r);
                if (0..=4095).contains(&off) {
                    self.asm.ldrb_uimm(d, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.ldrb_uimm(d, SGP3, 0);
                }
                self.gp_commit(r, d);
            }
            GcFieldLoadKind::I16S => {
                let d = self.gp_write_reg(r);
                if (0..=8190).contains(&off) && (off & 1) == 0 {
                    self.asm.ldrsh_uimm(false, d, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.ldrsh_uimm(false, d, SGP3, 0);
                }
                self.gp_commit(r, d);
            }
            GcFieldLoadKind::I16U => {
                let d = self.gp_write_reg(r);
                if (0..=8190).contains(&off) && (off & 1) == 0 {
                    self.asm.ldrh_uimm(d, base, off as u32);
                } else {
                    self.asm.add_i32_checked(true, SGP3, base, off, SGP3);
                    self.asm.ldrh_uimm(d, SGP3, 0);
                }
                self.gp_commit(r, d);
            }
            GcFieldLoadKind::Val(ty) => {
                if ty.is_fp() {
                    let d = self.fp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, None, Some(d), base, off, SGP3);
                    self.fp_commit(r, d);
                } else {
                    let d = self.gp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, Some(d), None, base, off, SGP3);
                    self.gp_commit(r, d);
                }
            }
        }
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
        if self.layout.is_store {
            debug_assert!(!is_tail_jump, "StoreCdylib indirect calls never tail-jump");
            let slot = self
                .alloc
                .store_saved_ctx_off
                .expect("store_saved_ctx_off missing");
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
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(CTX_REG),
                    None,
                    Reg::SP,
                    slot,
                    SGP3,
                );
                self.asm.ldr_uimm(true, SGP1, SGP2, 0);
                self.asm.ldr_uimm(true, CTX_REG, SGP2, 8);
                self.asm
                    .ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
                self.asm
                    .ldr_uimm(true, SGP2, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
                self.asm.ldr_uimm(true, MEM_LEN_REG, SGP2, 8);
                self.asm.blr(SGP1);
                return true;
            }
            // `call_indirect` through `OFF_STORE_TABLE_DESCS[table_idx]` using ONLY SGP1/SGP2/SGP3!
            let tbl = ci.dyn_table_desc_off;
            emit_load_off(
                &mut self.asm,
                if ci.table_is64 { Ty::I64 } else { Ty::I32 },
                Some(SGP2),
                None,
                CTX_REG,
                OFF_SCRATCH_TMP,
                SGP3,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_TABLE_DESCS,
                SGP3,
            );
            emit_load_off(
                &mut self.asm,
                Ty::I64,
                Some(SGP1),
                None,
                SGP1,
                tbl * 8,
                SGP3,
            );
            self.asm.ldr_uimm(false, SGP3, SGP1, 8); // cur_len zero-extended to u64
            self.asm.cmp_r(ci.table_is64, SGP2, SGP3);
            self.asm.b_cond(Cond::Hs, toob);
            self.emit_speculation_barrier();
            self.asm.ldr_uimm(true, SGP1, SGP1, 0); // table elements ptr
            self.asm.ldr_reg_scaled(true, SGP2, SGP1, SGP2, true); // *const StoreFuncRef
            self.asm.cbz(true, SGP2, bad);
            self.emit_speculation_barrier();
            // Verify `StoreFuncRef.canon_type_id <: exp_gt` keeping `SGP2` intact!
            let exp_ty = self.module.ci_sigs[ci_id as usize].type_idx;
            let exp_gt = self.module.gc_type(exp_ty);
            self.asm.ldr_uimm(false, SGP3, SGP2, 16); // actual_cid
            if exp_gt.is_final {
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_STORE_CANON_MAP,
                    SGP1,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(SGP1),
                    None,
                    SGP1,
                    (exp_gt.canon_id as i32) * 4,
                    SGP1,
                );
                self.asm.cmp_r(false, SGP3, SGP1);
                self.asm.b_cond(Cond::Ne, bad);
            } else {
                self.asm.lsl_imm(true, SGP3, SGP3, 6); // actual_cid * 64
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_STORE_PTR,
                    SGP1,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    SGP1,
                    STORE_OFF_TYPE_DESCS_PTR,
                    SGP1,
                );
                self.asm.add_r(true, SGP3, SGP1, SGP3); // &StoreTypeDesc[actual_cid]
                if (exp_gt.depth as usize) < GC_DISPLAY_LEN {
                    self.asm.ldr_uimm(false, SGP3, SGP3, 16 + exp_gt.depth * 4);
                } else {
                    self.asm.ldr_uimm(false, SGP1, SGP3, 8); // actual_depth
                    if exp_gt.depth < 4096 {
                        self.asm.cmp_imm(false, SGP1, exp_gt.depth as u16);
                    } else {
                        // Save SGP2 temporarily? Use OFF_SCRATCH_TMP (already consumed!).
                        emit_store_off(
                            &mut self.asm,
                            Ty::I64,
                            Some(SGP2),
                            None,
                            CTX_REG,
                            OFF_SCRATCH_TMP,
                            SGP2,
                        );
                        self.asm.mov_imm32(SGP2, exp_gt.depth);
                        self.asm.cmp_r(false, SGP1, SGP2);
                        emit_load_off(
                            &mut self.asm,
                            Ty::I64,
                            Some(SGP2),
                            None,
                            CTX_REG,
                            OFF_SCRATCH_TMP,
                            SGP1,
                        );
                    }
                    self.asm.b_cond(Cond::Lo, bad);
                    self.asm.ldr_uimm(true, SGP3, SGP3, 56); // supertypes_all_ptr
                    emit_load_off(
                        &mut self.asm,
                        Ty::I32,
                        Some(SGP3),
                        None,
                        SGP3,
                        (exp_gt.depth as i32) * 4,
                        SGP1,
                    );
                }
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_STORE_CANON_MAP,
                    SGP1,
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(SGP1),
                    None,
                    SGP1,
                    (exp_gt.canon_id as i32) * 4,
                    SGP1,
                );
                self.asm.cmp_r(false, SGP3, SGP1);
                self.asm.b_cond(Cond::Ne, bad);
            }
            self.emit_speculation_barrier();
            emit_store_off(
                &mut self.asm,
                Ty::I64,
                Some(CTX_REG),
                None,
                Reg::SP,
                slot,
                SGP3,
            );
            self.asm.ldr_uimm(true, SGP1, SGP2, 0);
            self.asm.ldr_uimm(true, CTX_REG, SGP2, 8);
            self.asm
                .ldr_uimm(true, MEM_BASE_REG, CTX_REG, OFF_MEM_BASE as u32);
            self.asm
                .ldr_uimm(true, SGP2, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
            self.asm.ldr_uimm(true, MEM_LEN_REG, SGP2, 8);
            self.asm.blr(SGP1);
            return true;
        }
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
            self.emit_publish_ret_spill(SGP1);
        }

        let mut need_store_restore = false;
        if let Some(f) = direct_func {
            if self.layout.is_store && (f as usize) < self.module.imports.len() {
                self.emit_store_call_import(f);
                need_store_restore = true;
            } else {
                self.asm.bl_reloc(A64RelocTarget::Func(f));
            }
        } else if let Some((ci_id, _)) = ci_idx {
            if !self.emit_ci_dispatch(ci_id, false) {
                return Ok(());
            }
            need_store_restore = self.layout.is_store;
        }
        self.record_gc_safepoint(self.cur_inst)?;
        if need_store_restore {
            self.emit_store_restore_ctx();
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
            self.emit_publish_ret_spill(SGP1);
        }

        let mut need_store_restore = false;
        if let Some(f) = direct_func {
            if self.layout.is_store && (f as usize) < self.module.imports.len() {
                self.emit_store_call_import(f);
                need_store_restore = true;
            } else {
                self.asm.bl_reloc(A64RelocTarget::Func(f));
            }
        } else if let Some((ci_id, _)) = ci_idx {
            if !self.emit_ci_dispatch(ci_id, false) {
                return Ok(());
            }
            need_store_restore = self.layout.is_store;
        }
        self.record_gc_safepoint(self.cur_inst)?;
        if need_store_restore {
            self.emit_store_restore_ctx();
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
        let store_forced_fallback = self.layout.is_store
            && (ci_idx.is_some()
                || direct_func.is_some_and(|f| (f as usize) < self.module.imports.len()));
        let can_tail_jump = arg_layout.stack_bytes <= self_param_stack && !store_forced_fallback;

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
                self.emit_publish_ret_spill(SGP2);
            }
            self.emit_epilogue_teardown();
            if let Some(f) = direct_func {
                self.asm.b_reloc(A64RelocTarget::Func(f));
            } else {
                self.asm.br(SGP1);
            }
            return Ok(());
        }

        // Growing-arity or cross-module StoreCdylib fallback: emit normal `bl`/`blr`
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
            self.emit_publish_ret_spill(SGP2);
        }
        let mut need_store_restore = false;
        if let Some(f) = direct_func {
            if self.layout.is_store && (f as usize) < self.module.imports.len() {
                self.emit_store_call_import(f);
                need_store_restore = true;
            } else {
                self.asm.bl_reloc(A64RelocTarget::Func(f));
            }
        } else if let Some((ci_id, _)) = ci_idx {
            if !self.emit_ci_dispatch(ci_id, false) {
                return Ok(());
            }
            need_store_restore = self.layout.is_store;
        }
        self.record_gc_safepoint(u32::MAX)?;
        if need_store_restore {
            self.emit_store_restore_ctx();
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

    fn emit_inst(
        &mut self,
        res: Option<Val>,
        op: Op,
        bounds_check_guard_covered: bool,
    ) -> Result<(), String> {
        let prev_flags = self.live_flags.take();
        match op {
            Op::Copy(src) => {
                let r = res.unwrap();
                let dl = MoveLoc::from(self.loc(r));
                let sl = MoveLoc::from(self.loc(src));
                let ty = self.ty(r);
                if dl != sl {
                    emit_single_move(&mut self.asm, dl, sl, ty);
                } else {
                    self.live_flags = prev_flags;
                }
            }
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
            Op::I32BinImm(bop, a, imm) => {
                let r = res.unwrap();
                if matches!(self.loc(r), A64Loc::RematAddr { .. }) {
                    self.live_flags = prev_flags;
                    return Ok(());
                }
                self.emit_int_bin_imm(false, bop, r, a, imm);
            }
            Op::I64BinImm(bop, a, imm) => {
                let r = res.unwrap();
                if matches!(self.loc(r), A64Loc::RematAddr { .. }) {
                    self.live_flags = prev_flags;
                    return Ok(());
                }
                self.emit_int_bin_imm(true, bop, r, a, imm);
            }
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
                    let (base, eff_off) = if self.layout.is_store
                        && g < self.layout.imported_global_count
                    {
                        emit_load_off(&mut self.asm, Ty::I64, Some(SGP2), None, CTX_REG, off, SGP3);
                        (SGP2, 0)
                    } else {
                        (CTX_REG, off)
                    };
                    if ty.is_fp() {
                        let d = self.fp_write_reg(r);
                        emit_load_off(&mut self.asm, ty, None, Some(d), base, eff_off, SGP3);
                        self.fp_commit(r, d);
                    } else {
                        let d = self.gp_write_reg(r);
                        emit_load_off(&mut self.asm, ty, Some(d), None, base, eff_off, SGP3);
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
                    let (base, eff_off) = if self.layout.is_store
                        && g < self.layout.imported_global_count
                    {
                        emit_load_off(&mut self.asm, Ty::I64, Some(SGP2), None, CTX_REG, off, SGP3);
                        (SGP2, 0)
                    } else {
                        (CTX_REG, off)
                    };
                    if ty.is_fp() {
                        let vr = self.fp_read(v, SFP1);
                        emit_store_off(&mut self.asm, ty, None, Some(vr), base, eff_off, SGP3);
                    } else {
                        let vr = self.gp_read(v, SGP1);
                        emit_store_off(&mut self.asm, ty, Some(vr), None, base, eff_off, SGP3);
                    }
                }
            }
            Op::TableGet { table, idx } => {
                let r = res.unwrap();
                let t64 = self.layout.table_is_64(table);
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        OFF_STORE_TABLE_DESCS,
                        SGP2,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        SGP3,
                        (table as i32) * 8,
                        SGP2,
                    );
                    self.asm.ldr_uimm(false, SGP2, SGP3, 8);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    self.asm.ldr_uimm(true, SGP3, SGP3, 0);
                    let d = self.gp_write_reg(r);
                    self.asm.ldr_reg_scaled(true, d, SGP3, ir, true);
                    self.gp_commit(r, d);
                } else if let Some(&(off, len)) = self.layout.extern_tables.get(&table) {
                    if len == 0 {
                        self.asm.b(toob);
                        return Ok(());
                    }
                    self.asm.mov_imm32(SGP2, len);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    let eff_ir = if self.layout.spectre_mitigations {
                        self.asm.csel(t64, SGP1, ir, Reg::XZR, Cond::Lo);
                        self.asm.csdb();
                        SGP1
                    } else {
                        ir
                    };
                    self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP2);
                    let d = self.gp_write_reg(r);
                    self.asm.ldr_reg_scaled(true, d, SGP3, eff_ir, true);
                    self.gp_commit(r, d);
                } else {
                    let (desc_off, min_len, _) = self.layout.dyn_tables[&table];
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
                    let eff_ir = if self.layout.spectre_mitigations && min_len >= 1 {
                        self.asm.csel(t64, SGP1, ir, Reg::XZR, Cond::Lo);
                        self.asm.csdb();
                        SGP1
                    } else {
                        self.emit_speculation_barrier();
                        ir
                    };
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
                    self.asm.ldr_reg_scaled(true, d, SGP3, eff_ir, true);
                    self.gp_commit(r, d);
                }
            }
            Op::TableSet { table, idx, val } => {
                let t64 = self.layout.table_is_64(table);
                let ir = self.gp_read(idx, SGP1);
                let toob = self.trap_table_oob_lbl();
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        OFF_STORE_TABLE_DESCS,
                        SGP2,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        SGP3,
                        (table as i32) * 8,
                        SGP2,
                    );
                    self.asm.ldr_uimm(false, SGP2, SGP3, 8);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    self.emit_speculation_barrier();
                    self.asm.uxtw(SGP2, ir);
                    self.asm.lsl_imm(true, SGP2, SGP2, 3);
                    self.asm.ldr_uimm(true, SGP3, SGP3, 0);
                    self.asm.add_r(true, SGP2, SGP3, SGP2);
                    let vr = self.gp_read(val, SGP1);
                    self.asm.str_uimm(true, vr, SGP2, 0);
                } else if let Some(&(off, len)) = self.layout.extern_tables.get(&table) {
                    if len == 0 {
                        self.asm.b(toob);
                        return Ok(());
                    }
                    self.asm.mov_imm32(SGP2, len);
                    self.asm.cmp_r(t64, ir, SGP2);
                    self.asm.b_cond(Cond::Hs, toob);
                    if self.layout.spectre_mitigations {
                        self.asm.csel(t64, SGP2, ir, Reg::XZR, Cond::Lo);
                        self.asm.csdb();
                        let vr = self.gp_read(val, SGP1);
                        self.asm.add_i32_checked(true, SGP3, CTX_REG, off, SGP3);
                        self.asm.str_reg_scaled(true, vr, SGP3, SGP2, true);
                    } else {
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
                    }
                } else {
                    let (desc_off, min_len, _) = self.layout.dyn_tables[&table];
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
                    let eff_ir = if self.layout.spectre_mitigations && min_len >= 1 {
                        self.asm.csel(t64, SGP1, ir, Reg::XZR, Cond::Lo);
                        self.asm.csdb();
                        SGP1
                    } else {
                        self.emit_speculation_barrier();
                        ir
                    };
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        desc_off,
                        SGP2,
                    );
                    self.asm.uxtw(SGP2, eff_ir);
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
                    && bounds_check_guard_covered
                {
                    return Ok(());
                }
                let ar = self.gp_read(addr, SGP1);
                self.emit_bounds_check_maybe_barrier(ar, end_off, for_bulk);
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
                let (ar, eff_off) = if self.layout.spectre_mitigations || (!checked && !guard_ok) {
                    self.emit_bounds_check_for_access(ar, offset, kind.bytes())
                } else {
                    (ar, offset)
                };
                let uxtw = !self.layout.memory64 && eff_off == offset;
                if eff_off == 0 {
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
                    let (ptr, uimm) = self.compute_mem_base_uimm(ar, eff_off, kind.bytes());
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
                let (ar, eff_off) = if self.layout.spectre_mitigations || (!checked && !guard_ok) {
                    self.emit_bounds_check_for_access(ar, offset, kind.bytes())
                } else {
                    (ar, offset)
                };
                let uxtw = !self.layout.memory64 && eff_off == offset;
                if eff_off == 0 {
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
                    let (ptr, uimm) = self.compute_mem_base_uimm(ar, eff_off, kind.bytes());
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
                let (ar, eff_off) = if self.layout.spectre_mitigations || (!checked && !guard_ok) {
                    self.emit_bounds_check_for_access(ar, offset, u64::from(width))
                } else {
                    (ar, offset)
                };
                let (ptr, uimm) = self.compute_mem_base_uimm(ar, eff_off, u64::from(width));
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
                let (ar, eff_off) = if self.layout.spectre_mitigations || (!checked && !guard_ok) {
                    self.emit_bounds_check_for_access(ar, offset, u64::from(width))
                } else {
                    (ar, offset)
                };
                let (ptr, uimm) = self.compute_mem_base_uimm(ar, eff_off, u64::from(width));
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
                if self.layout.is_store {
                    self.asm
                        .ldr_uimm(true, MEM_LEN_REG, CTX_REG, OFF_MEM_RESERVE_BYTES as u32);
                    self.asm.ldr_uimm(true, MEM_LEN_REG, MEM_LEN_REG, 8);
                }
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
                self.emit_extra_mem_ptr(mem, addr, offset, kind.bytes());
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
                self.emit_extra_mem_ptr(mem, addr, offset, kind.bytes());
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
                self.emit_extra_mem_ptr(mem, addr, offset, u64::from(width));
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
                self.emit_extra_mem_ptr(mem, addr, offset, u64::from(width));
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
                let d = self.gp_write_reg(r);
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_MEM_DESCS,
                        SGP3,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        SGP1,
                        (mem as i32) * 8,
                        SGP3,
                    );
                    self.asm.ldr_uimm(true, d, SGP1, 8);
                } else {
                    let desc_off = self.layout.mem_desc_off(mem);
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(d),
                        None,
                        CTX_REG,
                        desc_off + 8,
                        SGP3,
                    );
                }
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
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_TABLE_DESCS,
                        SGP3,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        SGP1,
                        (table as i32) * 8,
                        SGP3,
                    );
                    self.asm.ldr_uimm(false, d, SGP1, 8);
                } else if let Some(&(desc_off, _, _)) = self.layout.dyn_tables.get(&table) {
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
                let spectre_clamped = self.layout.spectre_mitigations
                    && len > 0
                    && self.layout.mem_min_bytes >= u64::from(len);
                if spectre_clamped {
                    let dr0 = self.gp_read(dst, SGP1);
                    let (dr_c, _) = self.emit_bounds_check_for_access(dr0, 0, u64::from(len));
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(dr_c),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    let sr0 = self.gp_read(src, SGP1);
                    let (sr_c, _) = self.emit_bounds_check_for_access(sr0, 0, u64::from(len));
                    self.asm.add_r(true, SGP2, MEM_BASE_REG, sr_c);
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    self.asm.add_r(true, SGP1, MEM_BASE_REG, SGP1);
                } else {
                    if self.layout.spectre_mitigations || !dst_checked {
                        let dr = self.gp_read(dst, SGP1);
                        self.emit_bounds_check(dr, u64::from(len));
                    }
                    if self.layout.spectre_mitigations || !src_checked {
                        let sr = self.gp_read(src, SGP1);
                        self.emit_bounds_check(sr, u64::from(len));
                    }
                    if len > 0 {
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
                    }
                }
                if len > 0 {
                    // Overlap-safe wide NEON 16B / 8B / 4B / 2B / 1B inline copy (`len <= 64`).
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
                let spectre_clamped = self.layout.spectre_mitigations
                    && len > 0
                    && self.layout.mem_min_bytes >= u64::from(len);
                if spectre_clamped {
                    let dr0 = self.gp_read(dst, SGP1);
                    let (dr_c, _) = self.emit_bounds_check_for_access(dr0, 0, u64::from(len));
                    self.asm.add_r(true, SGP3, MEM_BASE_REG, dr_c);
                } else {
                    if self.layout.spectre_mitigations || !checked {
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
                    }
                }
                if len > 0 {
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
            Op::VecElemLoop {
                kind,
                dst,
                src_a,
                src_b,
                count,
                rhs_is_scalar,
            } => {
                // Regalloc reserves clobbers X0..X3 and V0..V2.
                // X0 = dst_ptr, X1 = src_a_ptr, X2 = src_b_ptr (if array), X3 = count.
                let is8 = kind.elem_bytes() == 8;
                let size_code: u8 = if is8 { 3 } else { 2 };
                let lanes: u16 = if is8 { 2 } else { 4 };
                if rhs_is_scalar {
                    if kind.elem_ty().is_fp() {
                        let rr = self.fp_read(src_b, VReg::V0);
                        self.asm.dup_elt_128(size_code, VReg::V0, rr, 0);
                    } else {
                        let gr = self.gp_read(src_b, SGP1);
                        self.asm.dup_gp_128(size_code, VReg::V0, gr);
                    }
                    let moves = [
                        (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(dst)), Ty::I32),
                        (
                            MoveLoc::Gp(Reg::X1),
                            MoveLoc::from(self.loc(src_a)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X3),
                            MoveLoc::from(self.loc(count)),
                            Ty::I32,
                        ),
                    ];
                    emit_parallel_moves(&mut self.asm, &moves);
                } else {
                    let moves = [
                        (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(dst)), Ty::I32),
                        (
                            MoveLoc::Gp(Reg::X1),
                            MoveLoc::from(self.loc(src_a)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X2),
                            MoveLoc::from(self.loc(src_b)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X3),
                            MoveLoc::from(self.loc(count)),
                            Ty::I32,
                        ),
                    ];
                    emit_parallel_moves(&mut self.asm, &moves);
                }
                let lbl_done = self.asm.new_label();
                self.asm.uxtw(Reg::X3, Reg::X3);
                self.asm.cbz(true, Reg::X3, lbl_done);
                self.asm.uxtw(Reg::X0, Reg::X0);
                self.asm.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
                self.asm.uxtw(Reg::X1, Reg::X1);
                self.asm.add_r(true, Reg::X1, MEM_BASE_REG, Reg::X1);
                if !rhs_is_scalar {
                    self.asm.uxtw(Reg::X2, Reg::X2);
                    self.asm.add_r(true, Reg::X2, MEM_BASE_REG, Reg::X2);
                }

                let lbl_vec = self.asm.new_label();
                let lbl_tail = self.asm.new_label();
                self.asm.cmp_imm(true, Reg::X3, lanes);
                self.asm.b_cond(Cond::Lo, lbl_tail);

                self.asm.bind(lbl_vec);
                self.asm.ldr_q_uimm(VReg::V1, Reg::X1, 0);
                let rhs_v = if rhs_is_scalar {
                    VReg::V0
                } else {
                    self.asm.ldr_q_uimm(VReg::V2, Reg::X2, 0);
                    self.asm.add_imm(true, Reg::X2, Reg::X2, 16);
                    VReg::V2
                };
                match kind {
                    VecElemKind::I32Add => self.asm.add_v128(2, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::I32Sub => self.asm.sub_v128(2, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::I32Mul => self.asm.mul_v128(2, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::I32And | VecElemKind::I64And => {
                        self.asm.and_16b(VReg::V1, VReg::V1, rhs_v)
                    }
                    VecElemKind::I32Or | VecElemKind::I64Or => {
                        self.asm.orr_16b(VReg::V1, VReg::V1, rhs_v)
                    }
                    VecElemKind::I32Xor | VecElemKind::I64Xor => {
                        self.asm.eor_16b(VReg::V1, VReg::V1, rhs_v)
                    }
                    VecElemKind::I64Add => self.asm.add_v128(3, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::I64Sub => self.asm.sub_v128(3, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F32Add => self.asm.fadd_v128(false, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F32Sub => self.asm.fsub_v128(false, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F32Mul => self.asm.fmul_v128(false, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F32Div => self.asm.fdiv_v128(false, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F64Add => self.asm.fadd_v128(true, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F64Sub => self.asm.fsub_v128(true, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F64Mul => self.asm.fmul_v128(true, VReg::V1, VReg::V1, rhs_v),
                    VecElemKind::F64Div => self.asm.fdiv_v128(true, VReg::V1, VReg::V1, rhs_v),
                }
                self.asm.str_q_uimm(VReg::V1, Reg::X0, 0);
                self.asm.add_imm(true, Reg::X0, Reg::X0, 16);
                self.asm.add_imm(true, Reg::X1, Reg::X1, 16);
                self.asm.sub_imm(true, Reg::X3, Reg::X3, lanes);
                self.asm.cmp_imm(true, Reg::X3, lanes);
                self.asm.b_cond(Cond::Hs, lbl_vec);

                self.asm.bind(lbl_tail);
                self.asm.cbz(true, Reg::X3, lbl_done);
                if rhs_is_scalar && !kind.elem_ty().is_fp() {
                    self.asm.umov(size_code, SGP2, VReg::V0, 0);
                }
                let lbl_sc = self.asm.new_label();
                self.asm.bind(lbl_sc);
                match kind {
                    VecElemKind::I32Add
                    | VecElemKind::I32Sub
                    | VecElemKind::I32Mul
                    | VecElemKind::I32And
                    | VecElemKind::I32Or
                    | VecElemKind::I32Xor
                    | VecElemKind::I64Add
                    | VecElemKind::I64Sub
                    | VecElemKind::I64And
                    | VecElemKind::I64Or
                    | VecElemKind::I64Xor => {
                        self.asm.ldr_uimm(is8, SGP1, Reg::X1, 0);
                        let rhs_r = if rhs_is_scalar {
                            SGP2
                        } else {
                            self.asm.ldr_uimm(is8, SGP2, Reg::X2, 0);
                            self.asm
                                .add_imm(true, Reg::X2, Reg::X2, if is8 { 8 } else { 4 });
                            SGP2
                        };
                        match kind {
                            VecElemKind::I32Add | VecElemKind::I64Add => {
                                self.asm.add_r(is8, SGP1, SGP1, rhs_r)
                            }
                            VecElemKind::I32Sub | VecElemKind::I64Sub => {
                                self.asm.sub_r(is8, SGP1, SGP1, rhs_r)
                            }
                            VecElemKind::I32Mul => self.asm.mul(false, SGP1, SGP1, rhs_r),
                            VecElemKind::I32And | VecElemKind::I64And => {
                                self.asm.and_r(is8, SGP1, SGP1, rhs_r)
                            }
                            VecElemKind::I32Or | VecElemKind::I64Or => {
                                self.asm.orr_r(is8, SGP1, SGP1, rhs_r)
                            }
                            VecElemKind::I32Xor | VecElemKind::I64Xor => {
                                self.asm.eor_r(is8, SGP1, SGP1, rhs_r)
                            }
                            _ => unreachable!(),
                        }
                        self.asm.str_uimm(is8, SGP1, Reg::X0, 0);
                    }
                    VecElemKind::F32Add
                    | VecElemKind::F32Sub
                    | VecElemKind::F32Mul
                    | VecElemKind::F32Div => {
                        self.asm.ldr_s_uimm(VReg::V1, Reg::X1, 0);
                        let rhs_v = if rhs_is_scalar {
                            VReg::V0
                        } else {
                            self.asm.ldr_s_uimm(VReg::V2, Reg::X2, 0);
                            self.asm.add_imm(true, Reg::X2, Reg::X2, 4);
                            VReg::V2
                        };
                        match kind {
                            VecElemKind::F32Add => self.asm.fadd(false, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F32Sub => self.asm.fsub(false, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F32Mul => self.asm.fmul(false, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F32Div => self.asm.fdiv(false, VReg::V1, VReg::V1, rhs_v),
                            _ => unreachable!(),
                        }
                        self.asm.str_s_uimm(VReg::V1, Reg::X0, 0);
                    }
                    VecElemKind::F64Add
                    | VecElemKind::F64Sub
                    | VecElemKind::F64Mul
                    | VecElemKind::F64Div => {
                        self.asm.ldr_d_uimm(VReg::V1, Reg::X1, 0);
                        let rhs_v = if rhs_is_scalar {
                            VReg::V0
                        } else {
                            self.asm.ldr_d_uimm(VReg::V2, Reg::X2, 0);
                            self.asm.add_imm(true, Reg::X2, Reg::X2, 8);
                            VReg::V2
                        };
                        match kind {
                            VecElemKind::F64Add => self.asm.fadd(true, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F64Sub => self.asm.fsub(true, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F64Mul => self.asm.fmul(true, VReg::V1, VReg::V1, rhs_v),
                            VecElemKind::F64Div => self.asm.fdiv(true, VReg::V1, VReg::V1, rhs_v),
                            _ => unreachable!(),
                        }
                        self.asm.str_d_uimm(VReg::V1, Reg::X0, 0);
                    }
                }
                self.asm
                    .add_imm(true, Reg::X0, Reg::X0, if is8 { 8 } else { 4 });
                self.asm
                    .add_imm(true, Reg::X1, Reg::X1, if is8 { 8 } else { 4 });
                self.asm.sub_imm(true, Reg::X3, Reg::X3, 1);
                self.asm.cbnz(true, Reg::X3, lbl_sc);
                self.asm.bind(lbl_done);
            }
            Op::VecReduceLoop {
                kind,
                src_a,
                src_b,
                count,
            } => {
                // Regalloc reserves clobbers X0..X3 and V0..V2.
                // X0 = src_a_ptr, X1 = src_b_ptr (if dot), X2 = count.
                let r = res.unwrap();
                let is8 = kind.elem_bytes() == 8;
                let is_dot = matches!(kind, VecReduceKind::I32Dot);
                let is_xor = matches!(kind, VecReduceKind::I32Xor | VecReduceKind::I64Xor);
                let lanes: u16 = if is8 { 2 } else { 4 };
                if is_dot {
                    let moves = [
                        (
                            MoveLoc::Gp(Reg::X0),
                            MoveLoc::from(self.loc(src_a)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X1),
                            MoveLoc::from(self.loc(src_b)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X2),
                            MoveLoc::from(self.loc(count)),
                            Ty::I32,
                        ),
                    ];
                    emit_parallel_moves(&mut self.asm, &moves);
                } else {
                    let moves = [
                        (
                            MoveLoc::Gp(Reg::X0),
                            MoveLoc::from(self.loc(src_a)),
                            Ty::I32,
                        ),
                        (
                            MoveLoc::Gp(Reg::X2),
                            MoveLoc::from(self.loc(count)),
                            Ty::I32,
                        ),
                    ];
                    emit_parallel_moves(&mut self.asm, &moves);
                }
                let lbl_zero = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                self.asm.uxtw(Reg::X2, Reg::X2);
                self.asm.cbz(true, Reg::X2, lbl_zero);
                self.asm.uxtw(Reg::X0, Reg::X0);
                self.asm.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
                if is_dot {
                    self.asm.uxtw(Reg::X1, Reg::X1);
                    self.asm.add_r(true, Reg::X1, MEM_BASE_REG, Reg::X1);
                }

                self.asm.movi_zero_16b(VReg::V0);
                let lbl_vec = self.asm.new_label();
                let lbl_red = self.asm.new_label();
                self.asm.cmp_imm(true, Reg::X2, lanes);
                self.asm.b_cond(Cond::Lo, lbl_red);

                self.asm.bind(lbl_vec);
                self.asm.ldr_q_uimm(VReg::V1, Reg::X0, 0);
                if is_dot {
                    self.asm.ldr_q_uimm(VReg::V2, Reg::X1, 0);
                    self.asm.mul_v128(2, VReg::V1, VReg::V1, VReg::V2);
                    self.asm.add_v128(2, VReg::V0, VReg::V0, VReg::V1);
                    self.asm.add_imm(true, Reg::X1, Reg::X1, 16);
                } else if is_xor {
                    self.asm.eor_16b(VReg::V0, VReg::V0, VReg::V1);
                } else if is8 {
                    self.asm.add_v128(3, VReg::V0, VReg::V0, VReg::V1);
                } else {
                    self.asm.add_v128(2, VReg::V0, VReg::V0, VReg::V1);
                }
                self.asm.add_imm(true, Reg::X0, Reg::X0, 16);
                self.asm.sub_imm(true, Reg::X2, Reg::X2, lanes);
                self.asm.cmp_imm(true, Reg::X2, lanes);
                self.asm.b_cond(Cond::Hs, lbl_vec);

                self.asm.bind(lbl_red);
                // Extract lanes into SGP1 using umov + add/eor (fully supported in A64Vm!).
                if is8 {
                    self.asm.umov(3, SGP1, VReg::V0, 0);
                    self.asm.umov(3, SGP2, VReg::V0, 1);
                    if is_xor {
                        self.asm.eor_r(true, SGP1, SGP1, SGP2);
                    } else {
                        self.asm.add_r(true, SGP1, SGP1, SGP2);
                    }
                } else {
                    self.asm.umov(2, SGP1, VReg::V0, 0);
                    for lane in 1u8..4 {
                        self.asm.umov(2, SGP2, VReg::V0, lane);
                        if is_xor {
                            self.asm.eor_r(false, SGP1, SGP1, SGP2);
                        } else {
                            self.asm.add_r(false, SGP1, SGP1, SGP2);
                        }
                    }
                }

                self.asm.cbz(true, Reg::X2, lbl_done);
                let lbl_sc = self.asm.new_label();
                self.asm.bind(lbl_sc);
                self.asm.ldr_uimm(is8, SGP2, Reg::X0, 0);
                if is_dot {
                    self.asm.ldr_uimm(false, SGP3, Reg::X1, 0);
                    self.asm.mul(false, SGP2, SGP2, SGP3);
                    self.asm.add_r(false, SGP1, SGP1, SGP2);
                    self.asm.add_imm(true, Reg::X1, Reg::X1, 4);
                } else if is_xor {
                    self.asm.eor_r(is8, SGP1, SGP1, SGP2);
                } else {
                    self.asm.add_r(is8, SGP1, SGP1, SGP2);
                }
                self.asm
                    .add_imm(true, Reg::X0, Reg::X0, if is8 { 8 } else { 4 });
                self.asm.sub_imm(true, Reg::X2, Reg::X2, 1);
                self.asm.cbnz(true, Reg::X2, lbl_sc);
                self.asm.b(lbl_done);

                self.asm.bind(lbl_zero);
                self.asm.mov_imm64(SGP1, 0);
                self.asm.bind(lbl_done);
                self.gp_commit(r, SGP1);
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
            Op::SimdOuterLoop { desc_idx, args } => {
                self.emit_simd_outer_loop(desc_idx, args, res.unwrap());
            }
            Op::SlpWideChain { desc_idx, args } => {
                use crate::x64::ir::{WideStep, WideStepOp};
                let desc = self.ir.wide_chains[desc_idx as usize].clone();
                let arg_vals: Vec<Val> = self.ir.pool_slice(args).to_vec();
                let dst_val = arg_vals[0];
                let load_vals = &arg_vals[1..1 + desc.n_loads as usize];
                let bcast_vals = &arg_vals[1 + desc.n_loads as usize..];
                let slot_reg = |s: u8| if s == 0 { SFP1 } else { SFP2 };
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
                        if off < 4096 {
                            this.asm.add_imm(true, out_reg, out_reg, off as u16);
                        } else {
                            this.asm.mov_imm64(SGP3, off);
                            this.asm.add_r(true, out_reg, out_reg, SGP3);
                        }
                    }
                };
                let emit_bin =
                    |asm: &mut A64Asm, op: WideStepOp, d: VReg, l: VReg, r: VReg| match op {
                        WideStepOp::F32Add => asm.fadd_v128(false, d, l, r),
                        WideStepOp::F32Sub => asm.fsub_v128(false, d, l, r),
                        WideStepOp::F32Mul => asm.fmul_v128(false, d, l, r),
                        WideStepOp::F32Div => asm.fdiv_v128(false, d, l, r),
                        WideStepOp::F64Add => asm.fadd_v128(true, d, l, r),
                        WideStepOp::F64Sub => asm.fsub_v128(true, d, l, r),
                        WideStepOp::F64Mul => asm.fmul_v128(true, d, l, r),
                        WideStepOp::F64Div => asm.fdiv_v128(true, d, l, r),
                        WideStepOp::I32Add => asm.add_v128(2, d, l, r),
                        WideStepOp::I32Sub => asm.sub_v128(2, d, l, r),
                        WideStepOp::I32Mul => asm.mul_v128(2, d, l, r),
                        WideStepOp::I64Add => asm.add_v128(3, d, l, r),
                        WideStepOp::I64Sub => asm.sub_v128(3, d, l, r),
                        WideStepOp::VAnd => asm.and_16b(d, l, r),
                        WideStepOp::VOr => asm.orr_16b(d, l, r),
                        WideStepOp::VXor => asm.eor_16b(d, l, r),
                        WideStepOp::F32CanonNan | WideStepOp::F64CanonNan => {
                            let is64 = matches!(op, WideStepOp::F64CanonNan);
                            asm.fcmeq_v128(is64, VReg::V0, l, l);
                            asm.bsl_16b(VReg::V0, l, r);
                            if d != VReg::V0 {
                                asm.mov_16b(d, VReg::V0);
                            }
                        }
                        WideStepOp::F32Madd
                        | WideStepOp::F32Nmadd
                        | WideStepOp::F64Madd
                        | WideStepOp::F64Nmadd => unreachable!(),
                    };
                let run_chunk = |this: &mut Self, c: u64| {
                    let delta = c * 16;
                    for &step in &desc.steps {
                        match step {
                            WideStep::Load {
                                dst_slot,
                                load_idx,
                                offset,
                            } => {
                                emit_ptr(
                                    this,
                                    load_vals[load_idx as usize],
                                    u64::from(offset) + delta,
                                    SGP1,
                                );
                                this.asm.ldr_q_uimm(slot_reg(dst_slot), SGP1, 0);
                            }
                            WideStep::Broadcast {
                                dst_slot,
                                bcast_idx,
                            } => {
                                let d = slot_reg(dst_slot);
                                let br = this.fp_read(bcast_vals[bcast_idx as usize], d);
                                if br != d {
                                    this.asm.mov_16b(d, br);
                                }
                            }
                            WideStep::Bin {
                                op,
                                dst_slot,
                                lhs_slot,
                                rhs_slot,
                            } => {
                                emit_bin(
                                    &mut this.asm,
                                    op,
                                    slot_reg(dst_slot),
                                    slot_reg(lhs_slot),
                                    slot_reg(rhs_slot),
                                );
                            }
                            WideStep::BinMem {
                                op,
                                dst_slot,
                                lhs_slot,
                                load_idx,
                                offset,
                            } => {
                                emit_ptr(
                                    this,
                                    load_vals[load_idx as usize],
                                    u64::from(offset) + delta,
                                    SGP1,
                                );
                                this.asm.ldr_q_uimm(VReg::V0, SGP1, 0);
                                emit_bin(
                                    &mut this.asm,
                                    op,
                                    slot_reg(dst_slot),
                                    slot_reg(lhs_slot),
                                    VReg::V0,
                                );
                            }
                            WideStep::TerMem {
                                op,
                                acc_slot,
                                a_slot,
                                load_idx,
                                offset,
                            } => {
                                emit_ptr(
                                    this,
                                    load_vals[load_idx as usize],
                                    u64::from(offset) + delta,
                                    SGP1,
                                );
                                this.asm.ldr_q_uimm(VReg::V0, SGP1, 0);
                                let acc = slot_reg(acc_slot);
                                let ar = slot_reg(a_slot);
                                match op {
                                    WideStepOp::F32Madd => {
                                        this.asm.fmla_v128(false, acc, ar, VReg::V0)
                                    }
                                    WideStepOp::F32Nmadd => {
                                        this.asm.fmls_v128(false, acc, ar, VReg::V0)
                                    }
                                    WideStepOp::F64Madd => {
                                        this.asm.fmla_v128(true, acc, ar, VReg::V0)
                                    }
                                    WideStepOp::F64Nmadd => {
                                        this.asm.fmls_v128(true, acc, ar, VReg::V0)
                                    }
                                    _ => unreachable!(),
                                }
                            }
                        }
                    }
                };
                if desc.buffer_all_loads {
                    // `chunks == 2`: compute both chunks before storing either (overlap-exact).
                    run_chunk(self, 0);
                    self.asm.str_q_uimm(
                        slot_reg(desc.result_slot),
                        CTX_REG,
                        OFF_SCRATCH_V128 as u32,
                    );
                    run_chunk(self, 1);
                    self.asm
                        .ldr_q_uimm(VReg::V0, CTX_REG, OFF_SCRATCH_V128 as u32);
                    emit_ptr(self, dst_val, u64::from(desc.dst_off), SGP1);
                    self.asm.str_q_uimm(VReg::V0, SGP1, 0);
                    self.asm.str_q_uimm(slot_reg(desc.result_slot), SGP1, 16);
                } else {
                    for j in 0..u64::from(desc.chunks) {
                        let c = if desc.descending {
                            u64::from(desc.chunks) - 1 - j
                        } else {
                            j
                        };
                        run_chunk(self, c);
                        emit_ptr(self, dst_val, u64::from(desc.dst_off) + c * 16, SGP1);
                        self.asm.str_q_uimm(slot_reg(desc.result_slot), SGP1, 0);
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
            Op::V128Ternary { op, a, b, c } if matches!(op, 0xfa05..=0xfa08) => {
                // Wasm 3.0 relaxed FMA: `±(a * b) + c` via single-instruction ARMv8 NEON `fmla`/`fmls`!
                let r = res.unwrap();
                let is64 = op >= 0xfa07;
                let is_neg = (op & 1) == 0;
                let d = self.fp_write_reg(r);
                if d != SFP1 && d != SFP2 {
                    // Save `a`/`b` out of `d` BEFORE placing `c` into `d` (handles `d == a` or `d == b` cleanly).
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
                        A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => {
                            unreachable!()
                        }
                    }
                    if is_neg {
                        self.asm.fmls_v128(is64, d, a_use, b_use);
                    } else {
                        self.asm.fmla_v128(is64, d, a_use, b_use);
                    }
                } else {
                    // Spilled destination (`d == SFP1`): save `a` in `[CTX + OFF_SCRATCH_V128]` if `ar == SFP1`
                    // so loading `c` into `SFP1` never clobbers `a` or `b`.
                    let ar = self.fp_read(a, SFP1);
                    let a_in_tmp = ar == SFP1;
                    if a_in_tmp {
                        self.asm
                            .str_q_uimm(ar, CTX_REG, crate::x64::OFF_SCRATCH_V128 as u32);
                    }
                    let br = self.fp_read(b, SFP2);
                    let b_in_tmp = br == SFP2;
                    let cr = self.fp_read(c, SFP1);
                    if cr != d {
                        self.asm.mov_16b(d, cr);
                    }
                    if a_in_tmp {
                        if b_in_tmp {
                            // Both `a` and `b` were spilled (`b` is already in `SFP2`, `a` is in `tmp`, `d == SFP1` holds `c`).
                            // Move `c` (`d`) to a temp? Wait: we only have `SFP1`, `SFP2`, `tmp`:
                            // Compute `SFP2 = a * b` isn't single-round FMA! Instead stash `c` on stack or save `b` into `X16:X17`!
                            self.asm.umov(3, SGP1, SFP2, 0);
                            self.asm.umov(3, SGP2, SFP2, 1);
                            self.asm
                                .ldr_q_uimm(SFP2, CTX_REG, crate::x64::OFF_SCRATCH_V128 as u32); // SFP2 = a
                            // Stash `a`? Wait: we still need `b` in a VReg for `fmla(d, a, b)`.
                            // Move `SFP2` (`a`) -> `tmp` isn't helpful; we have `d` (`SFP1` = c), `SFP2` = a, `X16:X17` = b.
                            // Use `str_q` of `c` (`SFP1`)? We only have 2 scratch VRegs (`SFP1`/`SFP2`) so we can't hold 3 spilled operands in 2 VRegs!
                            // Save a callee-free scratch VReg (`V0`) into `[CTX + OFF_SCRATCH_V128]` after reading `a` into `V0`!
                            // Even simpler: save `V0` to `X16:X17`, load `a` from `tmp` into `V0`, reload `b`? Wait `SGP1/SGP2` already hold `b`!
                            // Cleanest 6-insn sequence: save `V0` into `[CTX+OFF_SCRATCH_V128]` after loading `a` into `SFP2` (`b` is in `SGP1:SGP2`):
                            // 1. `SFP2` holds `a` (loaded above!)
                            // 2. `str_q(V0, CTX+tmp)` -> saves `V0`!
                            // 3. `ins_gp(3, V0, 0, SGP1); ins_gp(3, V0, 1, SGP2)` -> `V0 = b`!
                            // 4. `fmla/fmls(d (=SFP1=c), SFP2 (=a), V0 (=b))`!
                            // 5. `ldr_q(V0, CTX+tmp)` -> restores `V0`!
                            self.asm.str_q_uimm(
                                VReg(0),
                                CTX_REG,
                                crate::x64::OFF_SCRATCH_V128 as u32,
                            );
                            self.asm.ins_gp(3, VReg(0), 0, SGP1);
                            self.asm.ins_gp(3, VReg(0), 1, SGP2);
                            if is_neg {
                                self.asm.fmls_v128(is64, d, SFP2, VReg(0));
                            } else {
                                self.asm.fmla_v128(is64, d, SFP2, VReg(0));
                            }
                            self.asm.ldr_q_uimm(
                                VReg(0),
                                CTX_REG,
                                crate::x64::OFF_SCRATCH_V128 as u32,
                            );
                        } else {
                            // `br` is an allocated register (`!= SFP1`, `!= SFP2`), `a` is in `tmp`, `d == SFP1` holds `c`:
                            self.asm
                                .ldr_q_uimm(SFP2, CTX_REG, crate::x64::OFF_SCRATCH_V128 as u32);
                            if is_neg {
                                self.asm.fmls_v128(is64, d, SFP2, br);
                            } else {
                                self.asm.fmla_v128(is64, d, SFP2, br);
                            }
                        }
                    } else if is_neg {
                        self.asm.fmls_v128(is64, d, ar, br);
                    } else {
                        self.asm.fmla_v128(is64, d, ar, br);
                    }
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
                        A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => {
                            unreachable!()
                        }
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
                let (ref fields, rec_size) = self.layout.tag_layouts[tag as usize];
                let arg_vals: Vec<Val> = self.ir.pool_slice(args).to_vec();
                let field_specs: Vec<(Ty, i32)> = fields.clone();
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_TAG_IDS,
                        SGP3,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I32,
                        Some(SGP1),
                        None,
                        SGP1,
                        (tag as i32) * 4,
                        SGP3,
                    );
                    self.asm.mov_imm32(SGP2, rec_size);
                    self.asm
                        .bl_reloc(A64RelocTarget::Helper(HELPER_STORE_EXN_ALLOC));
                    let prop = self.trap_prop_lbl();
                    self.asm.cbnz(false, SGP1, prop);
                    // `SGP2 (X17)` holds `exn_ptr` (with `global_tag_id` already written at `+0`).
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP2),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
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
                } else {
                    let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
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
                    self.asm.mov_imm32(SGP1, u32::from(TRAP_ALLOC_FAILED));
                    let ep = self.epilogue_trap_lbl();
                    self.asm.b(ep);
                    self.asm.bind(lbl_ok);
                }
            }
            Op::ExnSetActive(h) => {
                let hr = self.gp_read(h, SGP1);
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP2),
                        None,
                        CTX_REG,
                        OFF_STORE_PTR,
                        SGP3,
                    );
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(hr),
                        None,
                        SGP2,
                        STORE_OFF_EXN_ACTIVE,
                        SGP3,
                    );
                } else {
                    let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
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
            }
            Op::ExnGetActive => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_PTR,
                        SGP3,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(d),
                        None,
                        SGP1,
                        STORE_OFF_EXN_ACTIVE,
                        SGP3,
                    );
                } else {
                    let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(d),
                        None,
                        CTX_REG,
                        exn_off,
                        SGP3,
                    );
                }
                self.gp_commit(r, d);
            }
            Op::ExnClearActive => {
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP2),
                        None,
                        CTX_REG,
                        OFF_STORE_PTR,
                        SGP3,
                    );
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(Reg::XZR),
                        None,
                        SGP2,
                        STORE_OFF_EXN_ACTIVE,
                        SGP3,
                    );
                } else {
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
            }
            Op::ExnGetTag(h) => {
                let r = res.unwrap();
                let hr = self.gp_read(h, SGP1);
                let d = self.gp_write_reg(r);
                if self.layout.is_store {
                    self.asm.ldr_uimm(false, d, hr, 0);
                } else {
                    let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
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
                    self.asm.ldr_uimm(false, d, SGP2, 0);
                }
                self.gp_commit(r, d);
            }
            Op::ExnGetPayload {
                handle,
                tag,
                field,
                ty,
            } => {
                let r = res.unwrap();
                let foff = self.layout.tag_layouts[tag as usize].0[field as usize].1;
                let hr = self.gp_read(handle, SGP1);
                let base = if self.layout.is_store {
                    hr
                } else {
                    let exn_off = self.layout.exn_state_off.expect("exn_state_off missing");
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
                    SGP2
                };
                if ty.is_fp() {
                    let d = self.fp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, None, Some(d), base, foff, SGP3);
                    self.fp_commit(r, d);
                } else {
                    let d = self.gp_write_reg(r);
                    emit_load_off(&mut self.asm, ty, Some(d), None, base, foff, SGP3);
                    self.gp_commit(r, d);
                }
            }
            Op::PgoFuncEnter(def_idx) => {
                if let Some(ref pgo) = self.layout.pgo_gen {
                    let off = (crate::pgo::WTPGO1_HEADER_BYTES as i32)
                        .checked_add((def_idx as i32) * 8)
                        .ok_or("aarch64: PgoFuncEnter offset overflow")?;
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        pgo.ctx_ptr_off,
                        SGP3,
                    );
                    emit_load_off(&mut self.asm, Ty::I64, Some(SGP2), None, SGP1, off, SGP3);
                    self.asm.add_imm(true, SGP2, SGP2, 1);
                    emit_store_off(&mut self.asm, Ty::I64, Some(SGP2), None, SGP1, off, SGP3);
                }
            }
            Op::PgoRecordCi { site, ci_id, idx } => {
                if let Some(ref pgo) = self.layout.pgo_gen {
                    let ci = self.layout.ci_tables[ci_id as usize];
                    let is64 = ci.is_call_ref || ci.table_is64;
                    let ir = self.gp_read(idx, SGP2);
                    if ir != SGP2 {
                        self.asm.mov_r(is64, SGP2, ir);
                    }
                    let lbl_done = self.asm.new_label();
                    if ci.is_call_ref {
                        self.asm.mov_r(true, Reg::X0, SGP2);
                        self.asm.cbz(true, Reg::X0, lbl_done);
                    } else if ci.dyn_table_desc_off >= 0 {
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
                        self.asm.b_cond(Cond::Hs, lbl_done);
                        emit_load_off(
                            &mut self.asm,
                            Ty::I64,
                            Some(SGP1),
                            None,
                            CTX_REG,
                            ci.dyn_table_desc_off,
                            SGP3,
                        );
                        self.asm.ldr_reg_scaled(true, Reg::X0, SGP1, SGP2, true);
                        self.asm.cbz(true, Reg::X0, lbl_done);
                    } else if ci.span == 0 || ci.pgo_func_id_ro_off == 0 {
                        self.asm.b(lbl_done);
                    } else {
                        self.asm.mov_imm32(SGP3, ci.table_len);
                        self.asm.cmp_r(ci.table_is64, SGP2, SGP3);
                        self.asm.b_cond(Cond::Hs, lbl_done);
                        if ci.min_slot > 0 {
                            self.asm.mov_imm32(SGP3, ci.min_slot);
                            self.asm.sub_r(false, SGP2, SGP2, SGP3);
                        }
                        self.asm.mov_imm32(SGP3, ci.span);
                        self.asm.cmp_r(false, SGP2, SGP3);
                        self.asm.b_cond(Cond::Hs, lbl_done);
                        self.asm
                            .adr_reloc(SGP1, A64RelocTarget::Rodata(ci.pgo_func_id_ro_off));
                        self.asm.ldr_reg_scaled(false, Reg::X0, SGP1, SGP2, true);
                        self.asm.cbz(false, Reg::X0, lbl_done);
                    }
                    // W0 = func_idx (0-based).
                    self.asm.sub_imm(false, Reg::X0, Reg::X0, 1);
                    let slot_off = (crate::pgo::WTPGO1_HEADER_BYTES as u64)
                        .checked_add(u64::from(pgo.n_defs) * 8)
                        .and_then(|v| {
                            v.checked_add(
                                u64::from(site) * (crate::pgo::WTPGO1_CI_SITE_BYTES as u64),
                            )
                        })
                        .ok_or("aarch64: PgoRecordCi slot offset overflow")?;
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        pgo.ctx_ptr_off,
                        SGP3,
                    );
                    self.asm.mov_imm64(SGP3, slot_off);
                    self.asm.add_r(true, SGP1, SGP1, SGP3);
                    // total_hits += 1
                    self.asm.ldr_uimm(true, SGP2, SGP1, 0);
                    self.asm.add_imm(true, SGP2, SGP2, 1);
                    self.asm.str_uimm(true, SGP2, SGP1, 0);
                    // top_hits -> SGP2, top_target -> SGP3
                    self.asm.ldr_uimm(true, SGP2, SGP1, 8);
                    self.asm.ldr_uimm(false, SGP3, SGP1, 16);
                    let lbl_miss = self.asm.new_label();
                    let lbl_zero = self.asm.new_label();
                    self.asm.cmp_r(false, Reg::X0, SGP3);
                    self.asm.b_cond(Cond::Ne, lbl_miss);
                    self.asm.add_imm(true, SGP2, SGP2, 1);
                    self.asm.str_uimm(true, SGP2, SGP1, 8);
                    self.asm.b(lbl_done);
                    self.asm.bind(lbl_miss);
                    self.asm.cbz(true, SGP2, lbl_zero);
                    self.asm.sub_imm(true, SGP2, SGP2, 1);
                    self.asm.str_uimm(true, SGP2, SGP1, 8);
                    self.asm.b(lbl_done);
                    self.asm.bind(lbl_zero);
                    self.asm.str_uimm(false, Reg::X0, SGP1, 16);
                    self.asm.mov_imm64(SGP2, 1);
                    self.asm.str_uimm(true, SGP2, SGP1, 8);
                    self.asm.bind(lbl_done);
                }
            }
            Op::CiCheckTarget {
                ci_id,
                idx,
                expected_func,
            } => {
                let r = res.unwrap();
                let ci = self.layout.ci_tables[ci_id as usize];
                let is64 = ci.is_call_ref || ci.table_is64;
                let ir = self.gp_read(idx, SGP2);
                if ir != SGP2 {
                    self.asm.mov_r(is64, SGP2, ir);
                }
                let d = self.gp_write_reg(r);
                let lbl_false = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                if ci.is_call_ref {
                    self.asm.mov_imm64(SGP1, u64::from(expected_func) + 1);
                    self.asm.cmp_r(true, SGP2, SGP1);
                    self.asm.cset(false, d, Cond::Eq);
                    self.asm.b(lbl_done);
                } else if ci.dyn_table_desc_off >= 0 {
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
                    self.asm.b_cond(Cond::Hs, lbl_false);
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
                    self.asm.mov_imm64(SGP1, u64::from(expected_func) + 1);
                    self.asm.cmp_r(true, SGP2, SGP1);
                    self.asm.cset(false, d, Cond::Eq);
                    self.asm.b(lbl_done);
                } else if ci.span > 0 {
                    self.asm.mov_imm32(SGP3, ci.table_len);
                    self.asm.cmp_r(ci.table_is64, SGP2, SGP3);
                    self.asm.b_cond(Cond::Hs, lbl_false);
                    self.emit_speculation_barrier();
                    if ci.min_slot > 0 {
                        self.asm.mov_imm32(SGP3, ci.min_slot);
                        self.asm.sub_r(false, SGP2, SGP2, SGP3);
                    }
                    self.asm.mov_imm32(SGP3, ci.span);
                    self.asm.cmp_r(false, SGP2, SGP3);
                    self.asm.b_cond(Cond::Hs, lbl_false);
                    self.emit_speculation_barrier();
                    self.asm
                        .adr_reloc(SGP1, A64RelocTarget::Rodata(ci.rodata_off));
                    self.asm.ldrsw_reg_scaled(SGP3, SGP1, SGP2, true);
                    self.asm.add_r(true, SGP2, SGP1, SGP3);
                    self.asm
                        .adr_reloc(SGP1, A64RelocTarget::Func(expected_func));
                    self.asm.cmp_r(true, SGP2, SGP1);
                    self.asm.cset(false, d, Cond::Eq);
                    self.asm.b(lbl_done);
                }
                self.asm.bind(lbl_false);
                self.asm.movz(false, d, 0, 0);
                self.asm.bind(lbl_done);
                self.gp_commit(r, d);
            }
            Op::TagId(tag) => {
                let r = res.unwrap();
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_STORE_TAG_IDS,
                    SGP3,
                );
                let d = self.gp_write_reg(r);
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(d),
                    None,
                    SGP1,
                    (tag as i32) * 4,
                    SGP3,
                );
                self.gp_commit(r, d);
            }
            Op::FuncRef(func) => {
                let r = res.unwrap();
                let d = self.gp_write_reg(r);
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_FUNC_REFS,
                        SGP3,
                    );
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(d),
                        None,
                        SGP1,
                        (func as i32) * 8,
                        SGP3,
                    );
                } else {
                    self.asm.mov_imm64(d, u64::from(func) + 1);
                }
                self.gp_commit(r, d);
            }
            Op::GcNull | Op::ExternNull => {
                let r = res.unwrap();
                if !matches!(self.loc(r), A64Loc::Remat(_)) {
                    let d = self.gp_write_reg(r);
                    self.asm.movz(true, d, 0, 0);
                    self.gp_commit(r, d);
                }
            }
            Op::GcConstI31(enc) => {
                let r = res.unwrap();
                if !matches!(self.loc(r), A64Loc::Remat(_)) {
                    let d = self.gp_write_reg(r);
                    self.asm.mov_imm64(d, u64::from(enc));
                    self.gp_commit(r, d);
                }
            }
            Op::ExternConstI31(enc) => {
                let r = res.unwrap();
                if !matches!(self.loc(r), A64Loc::Remat(_)) {
                    let d = self.gp_write_reg(r);
                    self.asm.mov_imm64(d, GC_EXT_I31_TAG_HI | u64::from(enc));
                    self.gp_commit(r, d);
                }
            }
            Op::RefEq(a, b) => {
                let r = res.unwrap();
                let ar = self.gp_read(a, SGP1);
                let br = self.gp_read(b, SGP2);
                self.asm.cmp_r(true, ar, br);
                let d = self.gp_write_reg(r);
                self.asm.cset(false, d, Cond::Eq);
                self.gp_commit(r, d);
            }
            Op::RefI31(x) => {
                let r = res.unwrap();
                let xr = self.gp_read(x, SGP1);
                let d = self.gp_write_reg(r);
                self.asm.lsl_imm(false, d, xr, 1);
                self.asm.orr_imm_low_bits(false, d, d, 1);
                self.gp_commit(r, d);
            }
            Op::I31GetS(v) => {
                let r = res.unwrap();
                let trap = self.trap_ci_lbl();
                let vr = self.gp_read(v, SGP1);
                self.asm.cbz(true, vr, trap);
                let d = self.gp_write_reg(r);
                self.asm.asr_imm(false, d, vr, 1);
                self.gp_commit(r, d);
            }
            Op::I31GetU(v) => {
                let r = res.unwrap();
                let trap = self.trap_ci_lbl();
                let vr = self.gp_read(v, SGP1);
                self.asm.cbz(true, vr, trap);
                let d = self.gp_write_reg(r);
                self.asm.lsr_imm(false, d, vr, 1);
                self.gp_commit(r, d);
            }
            Op::ExternConvertAny(v) => {
                let r = res.unwrap();
                let vr = self.gp_read(v, SGP2);
                let d = self.gp_write_reg(r);
                if d != vr {
                    self.asm.mov_r(true, d, vr);
                }
                let lbl_done = self.asm.new_label();
                let lbl_i31 = self.asm.new_label();
                self.asm.cbz(true, d, lbl_done);
                self.asm.tst_imm_low_bits(false, d, 1);
                self.asm.b_cond(Cond::Ne, lbl_i31);
                let tmp = if d == SGP1 { SGP2 } else { SGP1 };
                self.asm.ldr_uimm(false, tmp, d, 0);
                self.asm.and_imm_6_w(tmp, tmp);
                self.asm
                    .cmp_imm(false, tmp, GC_HDR_KIND_INTERNAL_EXTERN as u16);
                self.asm.b_cond(Cond::Ne, lbl_done);
                self.asm.ldr_uimm(true, d, d, 8);
                self.asm.b(lbl_done);
                self.asm.bind(lbl_i31);
                self.asm.mov_imm64(tmp, GC_EXT_I31_TAG_HI);
                self.asm.orr_r(true, d, d, tmp);
                self.asm.bind(lbl_done);
                self.gp_commit(r, d);
            }
            Op::AnyConvertExtern(v) => {
                let r = res.unwrap();
                let gc_off = self
                    .layout
                    .gc
                    .as_ref()
                    .ok_or("aarch64: AnyConvertExtern without GcCtxLayout")?
                    .gc_state_off;
                let vr = self.gp_read(v, SGP2);
                if vr != SGP2 {
                    self.asm.mov_r(true, SGP2, vr);
                }
                let lbl_done = self.asm.new_label();
                let lbl_untag_i31 = self.asm.new_label();
                let lbl_box = self.asm.new_label();
                self.asm.cbz(true, SGP2, lbl_done);
                self.asm.lsr_imm(true, SGP1, SGP2, 32);
                self.asm.mov_imm32(SGP3, (GC_EXT_I31_TAG_HI >> 32) as u32);
                self.asm.cmp_r(false, SGP1, SGP3);
                self.asm.b_cond(Cond::Eq, lbl_untag_i31);
                self.asm.tst_imm_low_bits(true, SGP2, 3);
                self.asm.b_cond(Cond::Ne, lbl_box);
                if self.layout.is_store {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_STORE_PTR,
                        SGP3,
                    );
                    self.asm.ldr_uimm(true, SGP3, SGP1, GC_OFF_FROM_BASE as u32);
                    self.asm.subs_r(true, SGP3, SGP2, SGP3);
                    self.asm.b_cond(Cond::Lo, lbl_box);
                    self.asm.ldr_uimm(true, SGP1, SGP1, GC_OFF_FROM_CAP as u32);
                    self.asm.cmp_r(true, SGP3, SGP1);
                    self.asm.b_cond(Cond::Lo, lbl_done);
                } else {
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP3),
                        None,
                        CTX_REG,
                        gc_off + GC_OFF_FROM_BASE,
                        SGP3,
                    );
                    self.asm.subs_r(true, SGP3, SGP2, SGP3);
                    self.asm.b_cond(Cond::Lo, lbl_box);
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        gc_off + GC_OFF_FROM_CAP,
                        SGP1,
                    );
                    self.asm.cmp_r(true, SGP3, SGP1);
                    self.asm.b_cond(Cond::Lo, lbl_done);
                }
                self.asm.bind(lbl_box);
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP2),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.emit_gc_alloc_fixed(16)?;
                self.asm
                    .mov_imm64(SGP1, u64::from(GC_HDR_KIND_INTERNAL_EXTERN));
                self.asm.str_uimm(true, SGP1, SGP2, 0);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.asm.str_uimm(true, SGP1, SGP2, 8);
                self.asm.b(lbl_done);

                self.asm.bind(lbl_untag_i31);
                self.asm.uxtw(SGP2, SGP2);

                self.asm.bind(lbl_done);
                let d = self.gp_write_reg(r);
                if d != SGP2 {
                    self.asm.mov_r(true, d, SGP2);
                }
                self.gp_commit(r, d);
            }
            Op::StructNew { ty, fields } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let (field_defs, total_bytes) = match &gt.composite {
                    GcCompositeInfo::Struct {
                        fields,
                        total_bytes,
                        ..
                    } => (fields.clone(), *total_bytes),
                    _ => return Err("aarch64: StructNew on non-struct type".into()),
                };
                self.emit_gc_alloc_fixed(total_bytes)?;
                self.emit_write_gc_header(SGP2, gt, true);
                let fvals = self.ir.pool_slice(fields).to_vec();
                for (&fv, fd) in fvals.iter().zip(field_defs.iter()) {
                    self.emit_gc_storage_store(SGP2, fd.offset as i32, fd.storage, fv);
                }
                let d = self.gp_write_reg(r);
                if d != SGP2 {
                    self.asm.mov_r(true, d, SGP2);
                }
                self.gp_commit(r, d);
            }
            Op::StructNewDefault(ty) => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let total_bytes = match gt.composite {
                    GcCompositeInfo::Struct { total_bytes, .. } => total_bytes,
                    _ => return Err("aarch64: StructNewDefault on non-struct type".into()),
                };
                self.emit_gc_alloc_fixed(total_bytes)?;
                self.emit_write_gc_header(SGP2, gt, true);
                let zero_qwords = (total_bytes - 8) / 8;
                for q in 0..zero_qwords {
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(Reg::XZR),
                        None,
                        SGP2,
                        8 + (q as i32) * 8,
                        SGP3,
                    );
                }
                let d = self.gp_write_reg(r);
                if d != SGP2 {
                    self.asm.mov_r(true, d, SGP2);
                }
                self.gp_commit(r, d);
            }
            Op::StructGet {
                obj,
                ty,
                field,
                kind,
            } => {
                let r = res.unwrap();
                let trap = self.trap_ci_lbl();
                let gt = self.module.gc_type(ty);
                let foff = match &gt.composite {
                    GcCompositeInfo::Struct { fields, .. } => fields[field as usize].offset as i32,
                    _ => return Err("aarch64: StructGet on non-struct type".into()),
                };
                let or = self.gp_read(obj, SGP2);
                self.asm.cbz(true, or, trap);
                self.emit_gc_field_load(r, or, foff, kind);
            }
            Op::StructSet {
                obj,
                ty,
                field,
                val,
            } => {
                let trap = self.trap_ci_lbl();
                let gt = self.module.gc_type(ty);
                let fd = match &gt.composite {
                    GcCompositeInfo::Struct { fields, .. } => fields[field as usize],
                    _ => return Err("aarch64: StructSet on non-struct type".into()),
                };
                let or = self.gp_read(obj, SGP2);
                self.asm.cbz(true, or, trap);
                self.emit_gc_storage_store(or, fd.offset as i32, fd.storage, val);
                if matches!(fd.storage, GcStorageKind::GcRef | GcStorageKind::ExternRef) {
                    self.emit_store_gc_write_barrier(or, false);
                }
            }
            Op::ArrayNew { ty, init, len } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNew on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                self.emit_gc_alloc_array_dyn(len, elem_sz)?;
                let len_ml = MoveLoc::from(self.loc(len));
                match elem {
                    GcStorageKind::F32 => {
                        emit_parallel_moves(
                            &mut self.asm,
                            &[(MoveLoc::Gp(Reg::X1), len_ml, Ty::I32)],
                        );
                        let fx = self.fp_read(init, SFP1);
                        self.asm.fmov_gp_from_fp(false, Reg::X2, fx);
                    }
                    GcStorageKind::F64 => {
                        emit_parallel_moves(
                            &mut self.asm,
                            &[(MoveLoc::Gp(Reg::X1), len_ml, Ty::I32)],
                        );
                        let fx = self.fp_read(init, SFP1);
                        self.asm.fmov_gp_from_fp(true, Reg::X2, fx);
                    }
                    GcStorageKind::V128 => {
                        emit_parallel_moves(
                            &mut self.asm,
                            &[(MoveLoc::Gp(Reg::X1), len_ml, Ty::I32)],
                        );
                        let fx = self.fp_read(init, SFP1);
                        self.asm.mov_16b(SFP1, fx);
                    }
                    _ => {
                        let moves = [
                            (MoveLoc::Gp(Reg::X1), len_ml, Ty::I32),
                            (
                                MoveLoc::Gp(Reg::X2),
                                MoveLoc::from(self.loc(init)),
                                self.ty(init),
                            ),
                        ];
                        emit_parallel_moves(&mut self.asm, &moves);
                    }
                }
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.emit_write_gc_header(Reg::X0, gt, false);
                self.asm.str_uimm(false, Reg::X1, Reg::X0, 4);
                self.asm.mov_imm32(Reg::X3, elem_sz);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
                let d = self.gp_write_reg(r);
                if d != Reg::X0 {
                    self.asm.mov_r(true, d, Reg::X0);
                }
                self.gp_commit(r, d);
            }
            Op::ArrayNewDefault { ty, len } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNewDefault on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                self.emit_gc_alloc_array_dyn(len, elem_sz)?;
                let len_ml = MoveLoc::from(self.loc(len));
                emit_parallel_moves(&mut self.asm, &[(MoveLoc::Gp(Reg::X1), len_ml, Ty::I32)]);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.emit_write_gc_header(Reg::X0, gt, false);
                self.asm.str_uimm(false, Reg::X1, Reg::X0, 4);
                if elem_sz == 16 {
                    self.asm.movi_zero_16b(SFP1);
                } else {
                    self.asm.movz(true, Reg::X2, 0, 0);
                }
                self.asm.mov_imm32(Reg::X3, elem_sz);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_FILL_RAW));
                let d = self.gp_write_reg(r);
                if d != Reg::X0 {
                    self.asm.mov_r(true, d, Reg::X0);
                }
                self.gp_commit(r, d);
            }
            Op::ArrayNewFixed { ty, elems } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNewFixed on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                let evals = self.ir.pool_slice(elems).to_vec();
                let byte_size = u32::try_from((8usize + evals.len() * (elem_sz as usize) + 7) & !7)
                    .map_err(|_| "aarch64: ArrayNewFixed byte size exceeds u32")?;
                self.emit_gc_alloc_fixed(byte_size)?;
                self.emit_write_gc_header(SGP2, gt, false);
                self.asm.mov_imm32(SGP1, evals.len() as u32);
                self.asm.str_uimm(false, SGP1, SGP2, 4);
                for (i, &ev) in evals.iter().enumerate() {
                    self.emit_gc_storage_store(SGP2, 8 + (i as i32) * (elem_sz as i32), elem, ev);
                }
                let d = self.gp_write_reg(r);
                if d != SGP2 {
                    self.asm.mov_r(true, d, SGP2);
                }
                self.gp_commit(r, d);
            }
            Op::ArrayNewData { ty, data, src, len } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayNewData on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                let drop_off = self
                    .layout
                    .data_dropped_off
                    .ok_or("aarch64: ArrayNewData missing data_dropped_off")?;
                let trap_oob = self.trap_oob_lbl();
                let lr = self.gp_read(len, SGP1);
                self.asm.uxtw(SGP2, lr);
                if elem_sz > 1 {
                    self.asm
                        .lsl_imm(true, SGP2, SGP2, elem_sz.trailing_zeros() as u8);
                }
                let sr = self.gp_read(src, SGP1);
                self.asm.uxtw(SGP1, sr);
                self.asm.adds_r(true, SGP2, SGP2, SGP1);
                self.asm.b_cond(Cond::Hs, trap_oob);
                let lbl_not_dropped = self.asm.new_label();
                let lbl_cmp_seg = self.asm.new_label();
                self.asm
                    .add_i32_checked(true, SGP3, CTX_REG, drop_off + data as i32, SGP3);
                self.asm.ldrb_uimm(SGP1, SGP3, 0);
                self.asm.cbz(false, SGP1, lbl_not_dropped);
                self.asm.movz(true, SGP1, 0, 0);
                self.asm.b(lbl_cmp_seg);
                self.asm.bind(lbl_not_dropped);
                self.asm.adr_reloc(
                    SGP1,
                    A64RelocTarget::Rodata(self.layout.passive_data_table_off),
                );
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(SGP1),
                    None,
                    SGP1,
                    (data as i32) * 8 + 4,
                    SGP3,
                );
                self.asm.bind(lbl_cmp_seg);
                self.asm.cmp_r(true, SGP2, SGP1);
                self.asm.b_cond(Cond::Hi, trap_oob);

                self.emit_gc_alloc_array_dyn(len, elem_sz)?;
                let moves = [
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.emit_write_gc_header(Reg::X0, gt, false);
                self.asm.str_uimm(false, Reg::X3, Reg::X0, 4);
                self.asm.movz(false, Reg::X1, 0, 0); // dst_elem_off = 0
                self.asm
                    .mov_imm64(Reg::X4, (u64::from(data) << 32) | u64::from(elem_sz));
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_INIT_DATA));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
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
            }
            Op::ArrayNewElem { ty, elem, src, len } => {
                let r = res.unwrap();
                let gt = self.module.gc_type(ty);
                let elem_sz = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem.byte_size(),
                    _ => return Err("aarch64: ArrayNewElem on non-array type".into()),
                };
                let gc_elem_desc_ro = self
                    .layout
                    .gc
                    .as_ref()
                    .ok_or("aarch64: ArrayNewElem missing GcCtxLayout")?
                    .gc_elem_desc_ro_off;
                let drop_off = self
                    .layout
                    .elem_dropped_off
                    .ok_or("aarch64: ArrayNewElem missing elem_dropped_off")?;
                let trap_oob = self.trap_oob_lbl();
                let lr = self.gp_read(len, SGP1);
                self.asm.uxtw(SGP2, lr);
                let sr = self.gp_read(src, SGP1);
                self.asm.uxtw(SGP1, sr);
                self.asm.add_r(true, SGP2, SGP2, SGP1);
                let lbl_not_dropped = self.asm.new_label();
                let lbl_cmp_seg = self.asm.new_label();
                self.asm
                    .add_i32_checked(true, SGP3, CTX_REG, drop_off + elem as i32, SGP3);
                self.asm.ldrb_uimm(SGP1, SGP3, 0);
                self.asm.cbz(false, SGP1, lbl_not_dropped);
                self.asm.movz(true, SGP1, 0, 0);
                self.asm.b(lbl_cmp_seg);
                self.asm.bind(lbl_not_dropped);
                self.asm
                    .adr_reloc(SGP1, A64RelocTarget::Rodata(gc_elem_desc_ro));
                emit_load_off(
                    &mut self.asm,
                    Ty::I32,
                    Some(SGP1),
                    None,
                    SGP1,
                    (elem as i32) * 16 + 4,
                    SGP3,
                );
                self.asm.bind(lbl_cmp_seg);
                self.asm.cmp_r(true, SGP2, SGP1);
                self.asm.b_cond(Cond::Hi, trap_oob);

                self.emit_gc_alloc_array_dyn(len, elem_sz)?;
                let moves = [
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(Reg::X0),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                self.emit_write_gc_header(Reg::X0, gt, false);
                self.asm.str_uimm(false, Reg::X3, Reg::X0, 4);
                self.asm.movz(false, Reg::X1, 0, 0);
                self.asm.mov_imm32(Reg::X4, elem);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_INIT_ELEM));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
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
            }
            Op::ArrayGet { arr, idx, ty, kind } => {
                let r = res.unwrap();
                let trap_ci = self.trap_ci_lbl();
                let trap_oob = self.trap_oob_lbl();
                let gt = self.module.gc_type(ty);
                let elem_sz = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem.byte_size(),
                    _ => return Err("aarch64: ArrayGet on non-array type".into()),
                };
                let ar = self.gp_read(arr, SGP2);
                self.asm.cbz(true, ar, trap_ci);
                let ir = self.gp_read(idx, SGP1);
                self.asm.ldr_uimm(false, SGP3, ar, 4);
                self.asm.cmp_r(false, ir, SGP3);
                self.asm.b_cond(Cond::Hs, trap_oob);
                self.emit_speculation_barrier();
                self.asm.uxtw(SGP1, ir);
                if elem_sz > 1 {
                    self.asm
                        .lsl_imm(true, SGP1, SGP1, elem_sz.trailing_zeros() as u8);
                }
                self.asm.add_imm(true, SGP1, SGP1, 8);
                self.asm.add_r(true, SGP2, ar, SGP1);
                self.emit_gc_field_load(r, SGP2, 0, kind);
            }
            Op::ArraySet { arr, idx, val, ty } => {
                let trap_ci = self.trap_ci_lbl();
                let trap_oob = self.trap_oob_lbl();
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArraySet on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                let ar = self.gp_read(arr, SGP2);
                self.asm.cbz(true, ar, trap_ci);
                let ir = self.gp_read(idx, SGP1);
                self.asm.ldr_uimm(false, SGP3, ar, 4);
                self.asm.cmp_r(false, ir, SGP3);
                self.asm.b_cond(Cond::Hs, trap_oob);
                self.emit_speculation_barrier();
                self.asm.uxtw(SGP1, ir);
                if elem_sz > 1 {
                    self.asm
                        .lsl_imm(true, SGP1, SGP1, elem_sz.trailing_zeros() as u8);
                }
                self.asm.add_imm(true, SGP1, SGP1, 8);
                self.asm.add_r(true, SGP2, ar, SGP1);
                self.emit_gc_storage_store(SGP2, 0, elem, val);
                if self.layout.is_store
                    && matches!(elem, GcStorageKind::GcRef | GcStorageKind::ExternRef)
                {
                    let ar_wb = self.gp_read(arr, SGP2);
                    self.emit_store_gc_write_barrier(ar_wb, false);
                }
            }
            Op::ArrayLen(arr) => {
                let r = res.unwrap();
                let trap = self.trap_ci_lbl();
                let ar = self.gp_read(arr, SGP2);
                self.asm.cbz(true, ar, trap);
                let d = self.gp_write_reg(r);
                self.asm.ldr_uimm(false, d, ar, 4);
                self.gp_commit(r, d);
            }
            Op::ArrayFill {
                ty,
                arr,
                dst,
                val,
                len,
            } => {
                let gt = self.module.gc_type(ty);
                let elem = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayFill on non-array type".into()),
                };
                let elem_sz = elem.byte_size();
                match elem {
                    GcStorageKind::F32 | GcStorageKind::F64 | GcStorageKind::V128 => {
                        let moves = [
                            (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(arr)), Ty::I64),
                            (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(dst)), Ty::I32),
                            (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                        ];
                        emit_parallel_moves(&mut self.asm, &moves);
                        let fx = self.fp_read(val, SFP1);
                        match elem {
                            GcStorageKind::F32 => self.asm.fmov_gp_from_fp(false, Reg::X2, fx),
                            GcStorageKind::F64 => self.asm.fmov_gp_from_fp(true, Reg::X2, fx),
                            GcStorageKind::V128 => self.asm.mov_16b(SFP1, fx),
                            _ => unreachable!(),
                        }
                    }
                    _ => {
                        let moves = [
                            (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(arr)), Ty::I64),
                            (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(dst)), Ty::I32),
                            (
                                MoveLoc::Gp(Reg::X2),
                                MoveLoc::from(self.loc(val)),
                                self.ty(val),
                            ),
                            (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                        ];
                        emit_parallel_moves(&mut self.asm, &moves);
                    }
                }
                if matches!(elem, GcStorageKind::GcRef | GcStorageKind::ExternRef) {
                    self.emit_store_gc_write_barrier(Reg::X0, true);
                }
                self.asm.mov_imm32(Reg::X4, elem_sz);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_FILL));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::ArrayCopy {
                dst_ty,
                dst,
                dst_off,
                src,
                src_off,
                len,
                ..
            } => {
                let gt = self.module.gc_type(dst_ty);
                let (elem, elem_sz) = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => (elem, elem.byte_size()),
                    _ => return Err("aarch64: ArrayCopy on non-array type".into()),
                };
                let moves = [
                    (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(dst)), Ty::I64),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(dst_off)),
                        Ty::I32,
                    ),
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(src)), Ty::I64),
                    (
                        MoveLoc::Gp(Reg::X3),
                        MoveLoc::from(self.loc(src_off)),
                        Ty::I32,
                    ),
                    (MoveLoc::Gp(Reg::X4), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if matches!(elem, GcStorageKind::GcRef | GcStorageKind::ExternRef) {
                    self.emit_store_gc_write_barrier(Reg::X0, true);
                }
                self.asm.mov_imm32(Reg::X5, elem_sz);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_COPY));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::ArrayInitData {
                ty,
                data,
                arr,
                dst,
                src,
                len,
            } => {
                let gt = self.module.gc_type(ty);
                let elem_sz = match gt.composite {
                    GcCompositeInfo::Array { elem, .. } => elem.byte_size(),
                    _ => return Err("aarch64: ArrayInitData on non-array type".into()),
                };
                let moves = [
                    (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(arr)), Ty::I64),
                    (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(dst)), Ty::I32),
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                self.asm
                    .mov_imm64(Reg::X4, (u64::from(data) << 32) | u64::from(elem_sz));
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_INIT_DATA));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::ArrayInitElem {
                ty,
                elem,
                arr,
                dst,
                src,
                len,
            } => {
                let elem_storage = match self.module.gc_type(ty).composite {
                    GcCompositeInfo::Array { elem, .. } => elem,
                    _ => return Err("aarch64: ArrayInitElem on non-array type".into()),
                };
                let moves = [
                    (MoveLoc::Gp(Reg::X0), MoveLoc::from(self.loc(arr)), Ty::I64),
                    (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(dst)), Ty::I32),
                    (MoveLoc::Gp(Reg::X2), MoveLoc::from(self.loc(src)), Ty::I32),
                    (MoveLoc::Gp(Reg::X3), MoveLoc::from(self.loc(len)), Ty::I32),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                if matches!(
                    elem_storage,
                    GcStorageKind::GcRef | GcStorageKind::ExternRef
                ) {
                    self.emit_store_gc_write_barrier(Reg::X0, true);
                }
                self.asm.mov_imm32(Reg::X4, elem);
                self.asm
                    .bl_reloc(A64RelocTarget::Helper(HELPER_GC_ARRAY_INIT_ELEM));
                let prop = self.trap_prop_lbl();
                self.asm.cbnz(false, SGP1, prop);
            }
            Op::RefTest {
                val,
                heap,
                nullable,
            } => {
                let r = res.unwrap();
                let lbl_true = self.asm.new_label();
                let lbl_false = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                let vr = self.gp_read(val, SGP2);
                self.asm
                    .cbz(true, vr, if nullable { lbl_true } else { lbl_false });
                self.emit_gc_nonnull_matches_target(vr, heap, lbl_false)?;
                self.asm.bind(lbl_true);
                let d = self.gp_write_reg(r);
                self.asm.movz(false, d, 1, 0);
                self.asm.b(lbl_done);
                self.asm.bind(lbl_false);
                self.asm.movz(false, d, 0, 0);
                self.asm.bind(lbl_done);
                self.gp_commit(r, d);
            }
            Op::RefCast {
                val,
                heap,
                nullable,
            } => {
                let r = res.unwrap();
                let trap = self.trap_ci_lbl();
                let lbl_ok = self.asm.new_label();
                match (self.loc(val), self.loc(r)) {
                    (A64Loc::Gp(g), _) => {
                        self.asm.cbz(true, g, if nullable { lbl_ok } else { trap });
                        self.emit_gc_nonnull_matches_target(g, heap, trap)?;
                        self.asm.bind(lbl_ok);
                        let d = self.gp_write_reg(r);
                        if d != g {
                            self.asm.mov_r(true, d, g);
                        }
                        self.gp_commit(r, d);
                    }
                    (_, A64Loc::Gp(dg)) => {
                        let vr = self.gp_read(val, dg);
                        if vr != dg {
                            self.asm.mov_r(true, dg, vr);
                        }
                        self.asm.cbz(true, dg, if nullable { lbl_ok } else { trap });
                        self.emit_gc_nonnull_matches_target(dg, heap, trap)?;
                        self.asm.bind(lbl_ok);
                        self.gp_commit(r, dg);
                    }
                    _ => {
                        let vr = self.gp_read(val, SGP2);
                        self.asm.cbz(true, vr, if nullable { lbl_ok } else { trap });
                        self.emit_gc_nonnull_matches_target(vr, heap, trap)?;
                        self.asm.bind(lbl_ok);
                        if self.loc(val) != self.loc(r) {
                            let vr2 = self.gp_read(val, SGP1);
                            self.gp_commit(r, vr2);
                        }
                    }
                }
            }
            // ---- Wasm Threads / Atomics (`0xfe00..=0xfe4e`) ----
            Op::AtomicFence => {
                self.asm.dmb_ish();
            }
            Op::AtomicLoad {
                mem,
                width,
                addr,
                offset,
                ..
            } => {
                let r = res.unwrap();
                self.emit_atomic_ea(mem, addr, offset, width.bytes());
                let d = self.gp_write_reg(r);
                match width {
                    AtomicWidth::B1 => self.asm.ldarb(d, SGP1),
                    AtomicWidth::B2 => self.asm.ldarh(d, SGP1),
                    AtomicWidth::B4 => self.asm.ldar_w(d, SGP1),
                    AtomicWidth::B8 => self.asm.ldar(d, SGP1),
                }
                self.gp_commit(r, d);
            }
            Op::AtomicStore {
                mem,
                width,
                addr,
                val,
                offset,
            } => {
                self.emit_atomic_ea(mem, addr, offset, width.bytes());
                let vr = self.gp_read(val, SGP2);
                match width {
                    AtomicWidth::B1 => self.asm.stlrb(vr, SGP1),
                    AtomicWidth::B2 => self.asm.stlrh(vr, SGP1),
                    AtomicWidth::B4 => self.asm.stlr_w(vr, SGP1),
                    AtomicWidth::B8 => self.asm.stlr(vr, SGP1),
                }
            }
            Op::AtomicRmw {
                mem,
                op,
                width,
                res_ty,
                addr,
                val,
                offset,
            } => {
                let r = res.unwrap();
                let is64 = width == AtomicWidth::B8;
                self.emit_atomic_ea(mem, addr, offset, width.bytes());
                let vr = self.gp_read(val, SGP2);
                if vr != SGP2 {
                    self.asm.mov_r(is64, SGP2, vr);
                }
                let lp = self.asm.new_label();
                self.asm.bind(lp);
                match width {
                    AtomicWidth::B1 => self.asm.ldaxrb(Reg::X0, SGP1),
                    AtomicWidth::B2 => self.asm.ldaxrh(Reg::X0, SGP1),
                    AtomicWidth::B4 => self.asm.ldaxr_w(Reg::X0, SGP1),
                    AtomicWidth::B8 => self.asm.ldaxr(Reg::X0, SGP1),
                }
                match op {
                    AtomicRmwOp::Add => self.asm.add_r(is64, Reg::X1, Reg::X0, SGP2),
                    AtomicRmwOp::Sub => self.asm.sub_r(is64, Reg::X1, Reg::X0, SGP2),
                    AtomicRmwOp::And => self.asm.and_r(is64, Reg::X1, Reg::X0, SGP2),
                    AtomicRmwOp::Or => self.asm.orr_r(is64, Reg::X1, Reg::X0, SGP2),
                    AtomicRmwOp::Xor => self.asm.eor_r(is64, Reg::X1, Reg::X0, SGP2),
                    AtomicRmwOp::Xchg => self.asm.mov_r(is64, Reg::X1, SGP2),
                }
                match width {
                    AtomicWidth::B1 => self.asm.stlxrb(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B2 => self.asm.stlxrh(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B4 => self.asm.stlxr_w(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B8 => self.asm.stlxr(SGP3, Reg::X1, SGP1),
                }
                self.asm.cbnz(false, SGP3, lp);
                let d = self.gp_write_reg(r);
                if d != Reg::X0 {
                    self.asm.mov_r(res_ty == Ty::I64, d, Reg::X0);
                }
                self.gp_commit(r, d);
            }
            Op::AtomicCmpxchg {
                mem,
                width,
                res_ty,
                addr,
                expected,
                replacement,
                offset,
            } => {
                let r = res.unwrap();
                let is64 = width == AtomicWidth::B8;
                self.emit_atomic_ea(mem, addr, offset, width.bytes());
                emit_store_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                let moves = [
                    (
                        MoveLoc::Gp(Reg::X0),
                        MoveLoc::from(self.loc(expected)),
                        res_ty,
                    ),
                    (
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(replacement)),
                        res_ty,
                    ),
                ];
                emit_parallel_moves(&mut self.asm, &moves);
                self.asm.mov_r(is64, SGP2, Reg::X0);
                match width {
                    AtomicWidth::B1 => self.asm.and_imm_low_bits(false, SGP2, SGP2, 8),
                    AtomicWidth::B2 => self.asm.and_imm_low_bits(false, SGP2, SGP2, 16),
                    AtomicWidth::B4 => self.asm.uxtw(SGP2, SGP2),
                    AtomicWidth::B8 => {}
                }
                emit_load_off(
                    &mut self.asm,
                    Ty::I64,
                    Some(SGP1),
                    None,
                    CTX_REG,
                    OFF_SCRATCH_TMP,
                    SGP3,
                );
                let lp = self.asm.new_label();
                let lbl_fail = self.asm.new_label();
                let lbl_done = self.asm.new_label();
                self.asm.bind(lp);
                match width {
                    AtomicWidth::B1 => self.asm.ldaxrb(Reg::X0, SGP1),
                    AtomicWidth::B2 => self.asm.ldaxrh(Reg::X0, SGP1),
                    AtomicWidth::B4 => self.asm.ldaxr_w(Reg::X0, SGP1),
                    AtomicWidth::B8 => self.asm.ldaxr(Reg::X0, SGP1),
                }
                self.asm.cmp_r(is64, Reg::X0, SGP2);
                self.asm.b_cond(Cond::Ne, lbl_fail);
                match width {
                    AtomicWidth::B1 => self.asm.stlxrb(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B2 => self.asm.stlxrh(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B4 => self.asm.stlxr_w(SGP3, Reg::X1, SGP1),
                    AtomicWidth::B8 => self.asm.stlxr(SGP3, Reg::X1, SGP1),
                }
                self.asm.cbnz(false, SGP3, lp);
                self.asm.b(lbl_done);
                self.asm.bind(lbl_fail);
                self.asm.clrex();
                self.asm.bind(lbl_done);
                let d = self.gp_write_reg(r);
                if d != Reg::X0 {
                    self.asm.mov_r(res_ty == Ty::I64, d, Reg::X0);
                }
                self.gp_commit(r, d);
            }
            Op::AtomicWait {
                mem,
                is_64,
                addr,
                expected,
                timeout,
                offset,
            } => {
                let r = res.unwrap();
                if !self.module.mem_is_shared(mem) {
                    let lbl = self.trap_unreach_lbl();
                    self.asm.b(lbl);
                } else {
                    self.emit_atomic_ea(mem, addr, offset, if is_64 { 8 } else { 4 });
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    let ety = if is_64 { Ty::I64 } else { Ty::I32 };
                    let moves = [
                        (MoveLoc::Gp(Reg::X1), MoveLoc::from(self.loc(expected)), ety),
                        (
                            MoveLoc::Gp(Reg::X2),
                            MoveLoc::from(self.loc(timeout)),
                            Ty::I64,
                        ),
                    ];
                    emit_parallel_moves(&mut self.asm, &moves);
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(Reg::X0),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    self.asm.mov_imm32(Reg::X3, u32::from(is_64));
                    self.asm
                        .bl_reloc(A64RelocTarget::Helper(HELPER_ATOMIC_WAIT));
                    let d = self.gp_write_reg(r);
                    if d != Reg::X0 {
                        self.asm.mov_r(false, d, Reg::X0);
                    }
                    self.gp_commit(r, d);
                }
            }
            Op::AtomicNotify {
                mem,
                addr,
                count,
                offset,
            } => {
                let r = res.unwrap();
                self.emit_atomic_ea(mem, addr, offset, 4);
                if !self.module.mem_is_shared(mem) {
                    let d = self.gp_write_reg(r);
                    self.asm.movz(false, d, 0, 0);
                    self.gp_commit(r, d);
                } else {
                    emit_store_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(SGP1),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    let moves = [(
                        MoveLoc::Gp(Reg::X1),
                        MoveLoc::from(self.loc(count)),
                        Ty::I32,
                    )];
                    emit_parallel_moves(&mut self.asm, &moves);
                    emit_load_off(
                        &mut self.asm,
                        Ty::I64,
                        Some(Reg::X0),
                        None,
                        CTX_REG,
                        OFF_SCRATCH_TMP,
                        SGP3,
                    );
                    self.asm
                        .bl_reloc(A64RelocTarget::Helper(HELPER_ATOMIC_NOTIFY));
                    let d = self.gp_write_reg(r);
                    if d != Reg::X0 {
                        self.asm.mov_r(false, d, Reg::X0);
                    }
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
    module: &X64Module,
) -> Result<A64Asm, String> {
    let mut asm = A64Asm::new();
    let bb_labels: Vec<Label> = (0..ir.blocks.len()).map(|_| asm.new_label()).collect();
    let mut l = LowerA64 {
        ir,
        alloc,
        layout,
        module,
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
        trap_alloc: None,
        trap_unaligned: None,
        trap_prop: None,
        epilogue_ok: None,
        epilogue_trap: None,
        edge_tramps: Vec::new(),
        edge_tramp_cache: HashMap::new(),
        cur_bb: 0,
        cur_inst: 0,
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
    if l.alloc.has_calls {
        let fp_lr_off = l.alloc.saved_fp_lr_off;
        if (fp_lr_off & 7) == 0 && (-512..=504).contains(&fp_lr_off) {
            l.asm.stp_x(Reg::FP, Reg::LR, Reg::SP, fp_lr_off);
        } else {
            emit_store_off(
                &mut l.asm,
                Ty::I64,
                Some(Reg::FP),
                None,
                Reg::SP,
                fp_lr_off,
                SGP3,
            );
            emit_store_off(
                &mut l.asm,
                Ty::I64,
                Some(Reg::LR),
                None,
                Reg::SP,
                fp_lr_off + 8,
                SGP3,
            );
        }
        l.asm
            .add_i32_checked(true, Reg::FP, Reg::SP, fp_lr_off, SGP3);
    }
    {
        let mut gp_off = l.alloc.saved_gp_base;
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
        if l.layout.is_store {
            emit_load_off(
                &mut l.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_STORE_PTR,
                SGP3,
            );
            emit_load_off(
                &mut l.asm,
                Ty::I64,
                Some(SGP1),
                None,
                SGP1,
                STORE_OFF_RET_SPILL_PTR,
                SGP3,
            );
        } else {
            emit_load_off(
                &mut l.asm,
                Ty::I64,
                Some(SGP1),
                None,
                CTX_REG,
                OFF_RET_SPILL_PTR,
                SGP3,
            );
        }
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

    // 2. Emit basic blocks in RPO order (with 16-byte loop-header alignment and `madd`/`msub` fusion).
    let n_ord = l.alloc.block_order.len();
    for oi in 0..n_ord {
        let bid = l.alloc.block_order[oi];
        let next_bb = l.alloc.block_order.get(oi + 1).copied();
        if oi > 0 && l.ir.blocks[bid as usize].is_loop_header {
            while (l.asm.pos() & 15) != 0 {
                l.asm.nop();
            }
        }
        l.asm.bind(l.bb_labels[bid as usize]);
        l.cur_bb = bid;
        l.live_flags = None;
        let bb = &l.ir.blocks[bid as usize];
        let mut ii = 0usize;
        while ii < bb.insts.len() {
            l.cur_inst = ii as u32;
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
                        A64Loc::Gp(_) | A64Loc::Remat(_) | A64Loc::RematAddr { .. } => true,
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
            let bc_guard_covered = match inst.op {
                Op::BoundsCheck {
                    addr,
                    end_off,
                    for_bulk: false,
                } if layout.use_guard_pages && !layout.memory64 => {
                    crate::x64::lower::bounds_check_covered_by_subsequent_mem_op(
                        &bb.insts,
                        ii + 1,
                        addr,
                        end_off,
                    )
                }
                _ => false,
            };
            l.emit_inst(inst.res, inst.op, bc_guard_covered)?;
            ii += 1;
        }
        l.cur_inst = u32::MAX;
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
        (l.trap_alloc, TRAP_ALLOC_FAILED),
        (l.trap_unaligned, TRAP_UNALIGNED_ATOMIC),
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
