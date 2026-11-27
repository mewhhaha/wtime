//! Complete 236-opcode Wasm 128-bit SIMD (`0xfd00..=0xfdff`) lowering for ARMv8-A NEON.
//!
//! Uses only AArch64 scratch registers `SGP1` (`X16`), `SGP2` (`X17`), `SGP3` (`X24`),
//! `SFP1` (`V30`), `SFP2` (`V31`), and `[CTX_REG + OFF_SCRATCH_V128]` so no allocator-assigned
//! register is ever clobbered, and all operations are 100% safe when `d == a`, `d == b`, or
//! `d == SFP1`.

use super::asm::{A64Asm, A64RelocTarget, Cond, Reg, VReg};
use super::lower::{CTX_REG, SFP1, SFP2, SGP1, SGP2, SGP3};
use crate::x64::OFF_SCRATCH_V128;
use crate::x64::simd_lower::V128_OP_PSADBW;

fn load_v128_const(asm: &mut A64Asm, dst: VReg, bytes: [u8; 16]) {
    let off = asm.intern_v128(bytes);
    asm.adr_reloc(SGP3, A64RelocTarget::LocalRodata(off));
    asm.ldr_q_uimm(dst, SGP3, 0);
}

/// Checks whether `lanes` matches one of the 128-bit AdvSIMD permutes (`zip1`/`zip2`/`uzp1`/`uzp2`/`trn1`/`trn2`)
/// at `size` (`0`=16b, `1`=8h, `2`=4s, `3`=2d) on `(lhs, rhs)` (with `lhs` = bytes 0..16, `rhs` = bytes 16..32).
fn match_a64_perm(lanes: &[u8; 16], swapped: bool) -> Option<(u8, u8)> {
    // Returns `(kind, size)` where `kind`: 0=uzp1, 1=uzp2, 2=trn1, 3=trn2, 4=zip1, 5=zip2.
    for size in 0u8..=3 {
        let elem = 1usize << size;
        let n = 16 / elem;
        let half = n / 2;
        let map_byte = |b: u8| -> u8 {
            if swapped {
                if b < 16 { b + 16 } else { b - 16 }
            } else {
                b
            }
        };
        let mut ok = [true; 6];
        for i in 0..n {
            // Expected source element index (0..n = lhs, n..2n = rhs) for each of the 6 permutes:
            let exp_elems = [
                // uzp1: even elements from lhs, then even elements from rhs
                if i < half { 2 * i } else { n + 2 * (i - half) },
                // uzp2: odd elements from lhs, then odd elements from rhs
                if i < half {
                    2 * i + 1
                } else {
                    n + 2 * (i - half) + 1
                },
                // trn1: [lhs[0], rhs[0], lhs[2], rhs[2], ...]
                if (i & 1) == 0 { i } else { n + (i - 1) },
                // trn2: [lhs[1], rhs[1], lhs[3], rhs[3], ...]
                if (i & 1) == 0 { i + 1 } else { n + i },
                // zip1: [lhs[0], rhs[0], lhs[1], rhs[1], ...]
                if (i & 1) == 0 { i / 2 } else { n + (i / 2) },
                // zip2: [lhs[half], rhs[half], ...]
                if (i & 1) == 0 {
                    half + i / 2
                } else {
                    n + half + i / 2
                },
            ];
            for k in 0..6 {
                if !ok[k] {
                    continue;
                }
                let base = (exp_elems[k] * elem) as u8;
                for j in 0..elem {
                    if lanes[i * elem + j] != map_byte(base + j as u8) {
                        ok[k] = false;
                        break;
                    }
                }
            }
        }
        // Prefer zip1/zip2/uzp1/uzp2/trn1/trn2
        for k in [4usize, 5, 0, 1, 2, 3] {
            if ok[k] {
                return Some((k as u8, size));
            }
        }
    }
    None
}

/// Emits `i8x16.shuffle(a, b, lanes)` using 1-cycle NEON permute/EXT/DUP fast paths when possible,
/// falling back to 1-table NEON `TBL` (out-of-range index `0xff` yields 0) for arbitrary shuffles.
pub fn emit_a64_v128_shuffle(asm: &mut A64Asm, d: VReg, a: VReg, b: VReg, lanes: &[u8; 16]) {
    // Fast path 0: identity copies
    if (0..16u8).all(|i| lanes[i as usize] == i) {
        asm.mov_16b(d, a);
        return;
    }
    if (0..16u8).all(|i| lanes[i as usize] == i + 16) {
        asm.mov_16b(d, b);
        return;
    }
    // Fast path 1: contiguous 16-byte slide (`EXT Vd.16B, Vn.16B, Vm.16B, #imm4`)
    let start = lanes[0];
    if start < 16 && (0..16u8).all(|i| lanes[i as usize] == start + i) {
        asm.ext_16b(d, a, b, start);
        return;
    }
    if start >= 16 && (0..16u8).all(|i| lanes[i as usize] == (start + i) % 32) {
        asm.ext_16b(d, b, a, start - 16);
        return;
    }
    if start < 16 && (0..16u8).all(|i| lanes[i as usize] == (start + i) % 16) {
        asm.ext_16b(d, a, a, start);
        return;
    }
    if start >= 16 && (0..16u8).all(|i| lanes[i as usize] == 16 + ((start - 16 + i) % 16)) {
        asm.ext_16b(d, b, b, start - 16);
        return;
    }
    // Fast path 2: single-element broadcast (`DUP Vd.<T>, Vn.<T>[lane]`)
    for size in (0u8..=3).rev() {
        let elem = 1usize << size;
        let first = lanes[0];
        let src_is_b = first >= 16;
        let base = if src_is_b { first - 16 } else { first };
        if (base as usize).is_multiple_of(elem)
            && (0..16).all(|i| lanes[i] == first + (i % elem) as u8)
        {
            let lane = base / (elem as u8);
            asm.dup_elt_128(size, d, if src_is_b { b } else { a }, lane);
            return;
        }
    }
    // Fast path 3: `zip1`/`zip2`/`uzp1`/`uzp2`/`trn1`/`trn2` (covers ALL 4x4 AoS<->SoA transposes!)
    for (swapped, vn, vm) in [(false, a, b), (true, b, a)] {
        if let Some((kind, size)) = match_a64_perm(lanes, swapped) {
            match kind {
                0 => asm.uzp1_128(size, d, vn, vm),
                1 => asm.uzp2_128(size, d, vn, vm),
                2 => asm.trn1_128(size, d, vn, vm),
                3 => asm.trn2_128(size, d, vn, vm),
                4 => asm.zip1_128(size, d, vn, vm),
                _ => asm.zip2_128(size, d, vn, vm),
            }
            return;
        }
    }

    let mut mask_a = [0xffu8; 16];
    let mut mask_b = [0xffu8; 16];
    let mut uses_a = false;
    let mut uses_b = false;
    for i in 0..16 {
        let l = lanes[i];
        if l < 16 {
            mask_a[i] = l;
            uses_a = true;
        } else {
            mask_b[i] = l - 16;
            uses_b = true;
        }
    }
    if !uses_b {
        let mask_reg = if a == SFP2 { SFP1 } else { SFP2 };
        load_v128_const(asm, mask_reg, mask_a);
        asm.tbl_16b(d, a, mask_reg);
        return;
    }
    if !uses_a {
        let mask_reg = if b == SFP2 { SFP1 } else { SFP2 };
        load_v128_const(asm, mask_reg, mask_b);
        asm.tbl_16b(d, b, mask_reg);
        return;
    }
    // Both inputs used: preserve `b` in GP scratch (`X16:X17`) and `tbl_a` in `[CTX+OFF_SCRATCH_V128]`
    // so this is 100% alias-safe even when `d == a`, `d == b`, or `d == SFP1`.
    asm.umov(3, SGP1, b, 0);
    asm.umov(3, SGP2, b, 1);
    load_v128_const(asm, SFP2, mask_a);
    asm.tbl_16b(d, a, SFP2);
    asm.str_q_uimm(d, CTX_REG, OFF_SCRATCH_V128 as u32);
    asm.ins_gp(3, SFP1, 0, SGP1);
    asm.ins_gp(3, SFP1, 1, SGP2);
    load_v128_const(asm, SFP2, mask_b);
    asm.tbl_16b(SFP2, SFP1, SFP2);
    asm.ldr_q_uimm(d, CTX_REG, OFF_SCRATCH_V128 as u32);
    asm.orr_16b(d, d, SFP2);
}

/// Emits all `V128 -> I32` ops (`any_true`, `all_true`, `bitmask`).
pub fn emit_a64_v128_to_i32(asm: &mut A64Asm, op: u16, d: Reg, a: VReg) -> Result<(), String> {
    match op {
        0xfd53 => {
            // v128.any_true
            asm.umov(3, SGP1, a, 0);
            asm.umov(3, SGP2, a, 1);
            asm.orr_r(true, SGP1, SGP1, SGP2);
            asm.cmp_imm(true, SGP1, 0);
            asm.cset(false, d, Cond::Ne);
        }
        0xfd63 => {
            // i8x16.all_true
            asm.uminv_128(0, SFP2, a);
            asm.umov(0, SGP1, SFP2, 0);
            asm.cmp_imm(false, SGP1, 0);
            asm.cset(false, d, Cond::Ne);
        }
        0xfd83 => {
            // i16x8.all_true
            asm.uminv_128(1, SFP2, a);
            asm.umov(1, SGP1, SFP2, 0);
            asm.cmp_imm(false, SGP1, 0);
            asm.cset(false, d, Cond::Ne);
        }
        0xfda3 => {
            // i32x4.all_true
            asm.uminv_128(2, SFP2, a);
            asm.umov(2, SGP1, SFP2, 0);
            asm.cmp_imm(false, SGP1, 0);
            asm.cset(false, d, Cond::Ne);
        }
        0xfdc3 => {
            // i64x2.all_true
            asm.umov(3, SGP1, a, 0);
            asm.umov(3, SGP2, a, 1);
            asm.cmp_imm(true, SGP1, 0);
            asm.cset(false, SGP1, Cond::Ne);
            asm.cmp_imm(true, SGP2, 0);
            asm.cset(false, SGP2, Cond::Ne);
            asm.and_r(false, d, SGP1, SGP2);
        }
        0xfd64 => {
            // i8x16.bitmask: sign bits -> weighted [1,2,4,8,16,32,64,128]x2 -> 3x addp.16b -> umov.h[0]
            asm.sshr_imm_v128(0, SFP1, a, 7);
            load_v128_const(
                asm,
                SFP2,
                [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128],
            );
            asm.and_16b(SFP1, SFP1, SFP2);
            asm.addp_v128(0, SFP1, SFP1, SFP1);
            asm.addp_v128(0, SFP1, SFP1, SFP1);
            asm.addp_v128(0, SFP1, SFP1, SFP1);
            asm.umov(1, d, SFP1, 0);
        }
        0xfd84 => {
            // i16x8.bitmask: sign bits -> weighted [1,2,4,8,16,32,64,128] u16 -> 3x addp.8h -> umov.h[0]
            asm.sshr_imm_v128(1, SFP1, a, 15);
            let mut w = [0u8; 16];
            for i in 0..8 {
                w[i * 2..i * 2 + 2].copy_from_slice(&(1u16 << i).to_le_bytes());
            }
            load_v128_const(asm, SFP2, w);
            asm.and_16b(SFP1, SFP1, SFP2);
            asm.addp_v128(1, SFP1, SFP1, SFP1);
            asm.addp_v128(1, SFP1, SFP1, SFP1);
            asm.addp_v128(1, SFP1, SFP1, SFP1);
            asm.umov(1, d, SFP1, 0);
        }
        0xfda4 => {
            // i32x4.bitmask
            asm.sshr_imm_v128(2, SFP1, a, 31);
            let mut w = [0u8; 16];
            for i in 0..4 {
                w[i * 4..i * 4 + 4].copy_from_slice(&(1u32 << i).to_le_bytes());
            }
            load_v128_const(asm, SFP2, w);
            asm.and_16b(SFP1, SFP1, SFP2);
            asm.addp_v128(2, SFP1, SFP1, SFP1);
            asm.addp_v128(2, SFP1, SFP1, SFP1);
            asm.umov(2, d, SFP1, 0);
        }
        0xfdc4 => {
            // i64x2.bitmask
            asm.umov(3, SGP1, a, 0);
            asm.umov(3, SGP2, a, 1);
            asm.lsr_imm(true, SGP1, SGP1, 63);
            asm.lsr_imm(true, SGP2, SGP2, 63);
            asm.lsl_imm(false, SGP2, SGP2, 1);
            asm.orr_r(false, d, SGP1, SGP2);
        }
        _ => return Err(format!("aarch64: unsupported V128ToI32 opcode {op:#x}")),
    }
    Ok(())
}

/// Emits all `V128 -> V128` unary ops on ARMv8-A NEON.
pub fn emit_a64_v128_un(asm: &mut A64Asm, op: u16, d: VReg, a: VReg) -> Result<(), String> {
    match op {
        0xfd4d => asm.not_16b(d, a),
        0xfd5e => asm.fcvtn_2s_2d(d, a), // f32x4.demote_f64x2_zero (clears upper 64b!)
        0xfd5f => asm.fcvtl_2d_2s(d, a), // f64x2.promote_low_f32x4
        0xfd60 => asm.abs_v128(0, d, a),
        0xfd61 => asm.neg_v128(0, d, a),
        0xfd62 => asm.cnt_v(true, d, a),
        0xfd67 => asm.frintp_v128(false, d, a),
        0xfd68 => asm.frintm_v128(false, d, a),
        0xfd69 => asm.frintz_v128(false, d, a),
        0xfd6a => asm.frintn_v128(false, d, a),
        0xfd74 => asm.frintp_v128(true, d, a),
        0xfd75 => asm.frintm_v128(true, d, a),
        0xfd7a => asm.frintz_v128(true, d, a),
        0xfd94 => asm.frintn_v128(true, d, a),
        0xfd7c => asm.saddlp_128(0, d, a),
        0xfd7d => asm.uaddlp_128(0, d, a),
        0xfd7e => asm.saddlp_128(1, d, a),
        0xfd7f => asm.uaddlp_128(1, d, a),
        0xfd80 => asm.abs_v128(1, d, a),
        0xfd81 => asm.neg_v128(1, d, a),
        0xfd87 => asm.xtl_128(false, false, 0, d, a),
        0xfd88 => asm.xtl_128(false, true, 0, d, a),
        0xfd89 => asm.xtl_128(true, false, 0, d, a),
        0xfd8a => asm.xtl_128(true, true, 0, d, a),
        0xfda0 => asm.abs_v128(2, d, a),
        0xfda1 => asm.neg_v128(2, d, a),
        0xfda7 => asm.xtl_128(false, false, 1, d, a),
        0xfda8 => asm.xtl_128(false, true, 1, d, a),
        0xfda9 => asm.xtl_128(true, false, 1, d, a),
        crate::x64::ir::V128_UN_I32X4_EXTEND_LOW_I8X16_U => {
            asm.xtl_128(true, false, 0, d, a);
            asm.xtl_128(true, false, 1, d, d);
        }
        0xfdaa => asm.xtl_128(true, true, 1, d, a),
        0xfdc0 => asm.abs_v128(3, d, a),
        0xfdc1 => asm.neg_v128(3, d, a),
        0xfdc7 => asm.xtl_128(false, false, 2, d, a),
        0xfdc8 => asm.xtl_128(false, true, 2, d, a),
        0xfdc9 => asm.xtl_128(true, false, 2, d, a),
        0xfdca => asm.xtl_128(true, true, 2, d, a),
        0xfde0 => asm.fabs_v128(false, d, a),
        0xfde1 => asm.fneg_v128(false, d, a),
        0xfde3 => asm.fsqrt_v128(false, d, a),
        0xfdec => asm.fabs_v128(true, d, a),
        0xfded => asm.fneg_v128(true, d, a),
        0xfdef => asm.fsqrt_v128(true, d, a),
        0xfdf8 => asm.fcvtzs_v128(false, d, a), // i32x4.trunc_sat_f32x4_s (exact 1 insn!)
        0xfdf9 => asm.fcvtzu_v128(false, d, a), // i32x4.trunc_sat_f32x4_u (exact 1 insn!)
        0xfdfa => asm.scvtf_v128(false, d, a),
        0xfdfb => asm.ucvtf_v128(false, d, a),
        0xfdfc | 0xfdfd => {
            // i32x4.trunc_sat_f64x2_{s,u}_zero: exact scalar 32-bit saturating fcvtzs/fcvtzu per lane!
            let unsigned = op == 0xfdfd;
            if unsigned {
                asm.fcvtzu(false, true, SGP1, a);
            } else {
                asm.fcvtzs(false, true, SGP1, a);
            }
            asm.ins_elt(3, SFP2, 0, a, 1);
            if unsigned {
                asm.fcvtzu(false, true, SGP2, SFP2);
            } else {
                asm.fcvtzs(false, true, SGP2, SFP2);
            }
            asm.movi_zero_16b(d);
            asm.ins_gp(2, d, 0, SGP1);
            asm.ins_gp(2, d, 1, SGP2);
        }
        0xfdfe => {
            // f64x2.convert_low_i32x4_s
            asm.xtl_128(false, false, 2, d, a);
            asm.scvtf_v128(true, d, d);
        }
        0xfdff => {
            // f64x2.convert_low_i32x4_u
            asm.xtl_128(true, false, 2, d, a);
            asm.ucvtf_v128(true, d, d);
        }
        _ => return Err(format!("aarch64: unsupported V128Un opcode {op:#x}")),
    }
    Ok(())
}

/// Emits all `(V128, V128) -> V128` binary ops on ARMv8-A NEON (alias-safe for any `d`/`a`/`b`).
pub fn emit_a64_v128_bin(
    asm: &mut A64Asm,
    op: u16,
    d: VReg,
    a: VReg,
    b: VReg,
) -> Result<(), String> {
    match op {
        0xfd0e => asm.tbl_16b(d, a, b), // i8x16.swizzle (exact 1 insn!)

        // i8x16 comparisons
        0xfd23 => asm.cmeq_v128(0, d, a, b),
        0xfd24 => {
            asm.cmeq_v128(0, d, a, b);
            asm.not_16b(d, d);
        }
        0xfd25 => asm.cmgt_v128(0, d, b, a),
        0xfd26 => asm.cmhi_v128(0, d, b, a),
        0xfd27 => asm.cmgt_v128(0, d, a, b),
        0xfd28 => asm.cmhi_v128(0, d, a, b),
        0xfd29 => asm.cmge_v128(0, d, b, a),
        0xfd2a => asm.cmhs_v128(0, d, b, a),
        0xfd2b => asm.cmge_v128(0, d, a, b),
        0xfd2c => asm.cmhs_v128(0, d, a, b),

        // i16x8 comparisons
        0xfd2d => asm.cmeq_v128(1, d, a, b),
        0xfd2e => {
            asm.cmeq_v128(1, d, a, b);
            asm.not_16b(d, d);
        }
        0xfd2f => asm.cmgt_v128(1, d, b, a),
        0xfd30 => asm.cmhi_v128(1, d, b, a),
        0xfd31 => asm.cmgt_v128(1, d, a, b),
        0xfd32 => asm.cmhi_v128(1, d, a, b),
        0xfd33 => asm.cmge_v128(1, d, b, a),
        0xfd34 => asm.cmhs_v128(1, d, b, a),
        0xfd35 => asm.cmge_v128(1, d, a, b),
        0xfd36 => asm.cmhs_v128(1, d, a, b),

        // i32x4 comparisons
        0xfd37 => asm.cmeq_v128(2, d, a, b),
        0xfd38 => {
            asm.cmeq_v128(2, d, a, b);
            asm.not_16b(d, d);
        }
        0xfd39 => asm.cmgt_v128(2, d, b, a),
        0xfd3a => asm.cmhi_v128(2, d, b, a),
        0xfd3b => asm.cmgt_v128(2, d, a, b),
        0xfd3c => asm.cmhi_v128(2, d, a, b),
        0xfd3d => asm.cmge_v128(2, d, b, a),
        0xfd3e => asm.cmhs_v128(2, d, b, a),
        0xfd3f => asm.cmge_v128(2, d, a, b),
        0xfd40 => asm.cmhs_v128(2, d, a, b),

        // f32x4 comparisons
        0xfd41 => asm.fcmeq_v128(false, d, a, b),
        0xfd42 => {
            asm.fcmeq_v128(false, d, a, b);
            asm.not_16b(d, d);
        }
        0xfd43 => asm.fcmgt_v128(false, d, b, a),
        0xfd44 => asm.fcmgt_v128(false, d, a, b),
        0xfd45 => asm.fcmge_v128(false, d, b, a),
        0xfd46 => asm.fcmge_v128(false, d, a, b),

        // f64x2 comparisons
        0xfd47 => asm.fcmeq_v128(true, d, a, b),
        0xfd48 => {
            asm.fcmeq_v128(true, d, a, b);
            asm.not_16b(d, d);
        }
        0xfd49 => asm.fcmgt_v128(true, d, b, a),
        0xfd4a => asm.fcmgt_v128(true, d, a, b),
        0xfd4b => asm.fcmge_v128(true, d, b, a),
        0xfd4c => asm.fcmge_v128(true, d, a, b),

        // Bitwise
        0xfd4e => asm.and_16b(d, a, b),
        0xfd4f => asm.bic_16b(d, a, b),
        0xfd50 => asm.orr_16b(d, a, b),
        0xfd51 => asm.eor_16b(d, a, b),

        // Narrowing (guard `b` if `d == b` since `sqxtn` writes low half of `d` before `sqxtn2` reads `b`!)
        0xfd65 | 0xfd66 | 0xfd85 | 0xfd86 => {
            let size = if matches!(op, 0xfd65 | 0xfd66) { 0 } else { 1 };
            let unsigned = matches!(op, 0xfd66 | 0xfd86);
            let b_use = if d == b {
                asm.mov_16b(SFP2, b);
                SFP2
            } else {
                b
            };
            if unsigned {
                asm.sqxtun_128(false, size, d, a);
                asm.sqxtun_128(true, size, d, b_use);
            } else {
                asm.sqxtn_128(false, size, d, a);
                asm.sqxtn_128(true, size, d, b_use);
            }
        }

        // i8x16 arithmetic
        0xfd6e => asm.add_v128(0, d, a, b),
        0xfd6f => asm.sqadd_v128(0, d, a, b),
        0xfd70 => asm.uqadd_v128(0, d, a, b),
        0xfd71 => asm.sub_v128(0, d, a, b),
        0xfd72 => asm.sqsub_v128(0, d, a, b),
        0xfd73 => asm.uqsub_v128(0, d, a, b),
        0xfd76 => asm.smin_v128(0, d, a, b),
        0xfd77 => asm.umin_v128(0, d, a, b),
        0xfd78 => asm.smax_v128(0, d, a, b),
        0xfd79 => asm.umax_v128(0, d, a, b),
        0xfd7b => asm.urhadd_v128(0, d, a, b),

        // i16x8 arithmetic & q15mulr & extmul
        0xfd82 => asm.sqrdmulh_8h(d, a, b),
        0xfd8e => asm.add_v128(1, d, a, b),
        0xfd8f => asm.sqadd_v128(1, d, a, b),
        0xfd90 => asm.uqadd_v128(1, d, a, b),
        0xfd91 => asm.sub_v128(1, d, a, b),
        0xfd92 => asm.sqsub_v128(1, d, a, b),
        0xfd93 => asm.uqsub_v128(1, d, a, b),
        0xfd95 => asm.mul_v128(1, d, a, b),
        0xfd96 => asm.smin_v128(1, d, a, b),
        0xfd97 => asm.umin_v128(1, d, a, b),
        0xfd98 => asm.smax_v128(1, d, a, b),
        0xfd99 => asm.umax_v128(1, d, a, b),
        0xfd9b => asm.urhadd_v128(1, d, a, b),
        0xfd9c => asm.mull_128(false, false, 0, d, a, b),
        0xfd9d => asm.mull_128(false, true, 0, d, a, b),
        0xfd9e => asm.mull_128(true, false, 0, d, a, b),
        0xfd9f => asm.mull_128(true, true, 0, d, a, b),

        // i32x4 arithmetic, dot, extmul
        0xfdae => asm.add_v128(2, d, a, b),
        0xfdb1 => asm.sub_v128(2, d, a, b),
        0xfdb5 => asm.mul_v128(2, d, a, b),
        0xfdb6 => asm.smin_v128(2, d, a, b),
        0xfdb7 => asm.umin_v128(2, d, a, b),
        0xfdb8 => asm.smax_v128(2, d, a, b),
        0xfdb9 => asm.umax_v128(2, d, a, b),
        0xfdba => {
            // i32x4.dot_i16x8_s: smull.4s(lo) + smull2.4s(hi) -> addp.4s (100% alias-safe!).
            asm.mov_16b(SFP2, b);
            asm.mov_16b(SFP1, a);
            if d != SFP1 {
                asm.mull_128(false, false, 1, d, SFP1, SFP2);
                asm.mull_128(false, true, 1, SFP2, SFP1, SFP2);
                asm.addp_v128(2, d, d, SFP2);
            } else {
                asm.str_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.mull_128(false, false, 1, SFP1, SFP1, SFP2);
                asm.umov(3, SGP1, SFP1, 0);
                asm.umov(3, SGP2, SFP1, 1);
                asm.ldr_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.mull_128(false, true, 1, SFP2, SFP1, SFP2);
                asm.ins_gp(3, SFP1, 0, SGP1);
                asm.ins_gp(3, SFP1, 1, SGP2);
                asm.addp_v128(2, d, SFP1, SFP2);
            }
        }
        0xfdbc => asm.mull_128(false, false, 1, d, a, b),
        0xfdbd => asm.mull_128(false, true, 1, d, a, b),
        0xfdbe => asm.mull_128(true, false, 1, d, a, b),
        0xfdbf => asm.mull_128(true, true, 1, d, a, b),

        // i64x2 arithmetic, cmp, extmul
        0xfdce => asm.add_v128(3, d, a, b),
        0xfdd1 => asm.sub_v128(3, d, a, b),
        0xfdd5 => {
            // i64x2.mul: 2 scalar 64-bit multiplies (100% alias-safe even if `d == a` or `d == b`!).
            asm.umov(3, SGP1, a, 0);
            asm.umov(3, SGP2, b, 0);
            asm.mul(true, SGP3, SGP1, SGP2);
            asm.umov(3, SGP1, a, 1);
            asm.umov(3, SGP2, b, 1);
            asm.mul(true, SGP1, SGP1, SGP2);
            asm.ins_gp(3, d, 0, SGP3);
            asm.ins_gp(3, d, 1, SGP1);
        }
        0xfdd6 => asm.cmeq_v128(3, d, a, b),
        0xfdd7 => {
            asm.cmeq_v128(3, d, a, b);
            asm.not_16b(d, d);
        }
        0xfdd8 => asm.cmgt_v128(3, d, b, a),
        0xfdd9 => asm.cmgt_v128(3, d, a, b),
        0xfdda => asm.cmge_v128(3, d, b, a),
        0xfddb => asm.cmge_v128(3, d, a, b),
        0xfddc => asm.mull_128(false, false, 2, d, a, b),
        0xfddd => asm.mull_128(false, true, 2, d, a, b),
        0xfdde => asm.mull_128(true, false, 2, d, a, b),
        0xfddf => asm.mull_128(true, true, 2, d, a, b),

        // f32x4 arithmetic
        0xfde4 => asm.fadd_v128(false, d, a, b),
        0xfde5 => asm.fsub_v128(false, d, a, b),
        0xfde6 => asm.fmul_v128(false, d, a, b),
        0xfde7 => asm.fdiv_v128(false, d, a, b),
        0xfde8 => asm.fmin_v128(false, d, a, b),
        0xfde9 => asm.fmax_v128(false, d, a, b),
        0xfdea | 0xfdeb | 0xfdf6 | 0xfdf7 => {
            // pmin(a, b) = b < a ? b : a  (mask = a > b)
            // pmax(a, b) = a < b ? b : a  (mask = b > a)
            let is64 = matches!(op, 0xfdf6 | 0xfdf7);
            let is_pmax = matches!(op, 0xfdeb | 0xfdf7);
            if a == b {
                // `x < x` is always false (even for NaN and ±0.0), so pmin(x, x) == pmax(x, x) == x.
                if d != a {
                    asm.mov_16b(d, a);
                }
            } else if d == SFP1 || d == SFP2 {
                // Spilled destination (`d` aliases scratch): compute `(b & mask) | (a & ~mask)`
                // preserving `a` in `[CTX_REG + OFF_SCRATCH_V128]` and `b` in `SGP1:SGP2` so
                // every alias combination (including `d == a == SFP1`, `b == SFP2`) is exact.
                asm.str_q_uimm(a, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.umov(3, SGP1, b, 0);
                asm.umov(3, SGP2, b, 1);
                if is_pmax {
                    asm.fcmgt_v128(is64, SFP2, b, a);
                } else {
                    asm.fcmgt_v128(is64, SFP2, a, b);
                }
                asm.ldr_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.bic_16b(SFP1, SFP1, SFP2);
                asm.str_q_uimm(SFP1, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.ins_gp(3, SFP1, 0, SGP1);
                asm.ins_gp(3, SFP1, 1, SGP2);
                asm.and_16b(SFP1, SFP1, SFP2);
                asm.ldr_q_uimm(SFP2, CTX_REG, OFF_SCRATCH_V128 as u32);
                asm.orr_16b(d, SFP1, SFP2);
            } else {
                let b_safe = if d == b {
                    let tmp = if a == SFP2 { SFP1 } else { SFP2 };
                    asm.mov_16b(tmp, b);
                    tmp
                } else {
                    b
                };
                let a_safe = if d == a {
                    let tmp = if b_safe == SFP1 { SFP2 } else { SFP1 };
                    asm.mov_16b(tmp, a);
                    tmp
                } else {
                    a
                };
                if is_pmax {
                    asm.fcmgt_v128(is64, d, b_safe, a_safe);
                } else {
                    asm.fcmgt_v128(is64, d, a_safe, b_safe);
                }
                asm.bsl_16b(d, b_safe, a_safe);
            }
        }

        // f64x2 arithmetic
        0xfdf0 => asm.fadd_v128(true, d, a, b),
        0xfdf1 => asm.fsub_v128(true, d, a, b),
        0xfdf2 => asm.fmul_v128(true, d, a, b),
        0xfdf3 => asm.fdiv_v128(true, d, a, b),
        0xfdf4 => asm.fmin_v128(true, d, a, b),
        0xfdf5 => asm.fmax_v128(true, d, a, b),

        V128_OP_PSADBW => {
            asm.uaddlp_128(0, d, a);
            asm.uaddlp_128(1, d, d);
            asm.uaddlp_128(2, d, d);
        }

        _ => return Err(format!("aarch64: unsupported V128Bin opcode {op:#x}")),
    }
    Ok(())
}

/// Emits all 12 `(V128, I32) -> V128` SIMD shifts via NEON `SSHL`/`USHL`.
pub fn emit_a64_v128_shift(
    asm: &mut A64Asm,
    op: u16,
    d: VReg,
    v: VReg,
    shift: Reg,
) -> Result<(), String> {
    let (size, mask, is_right, is_signed) = match op {
        0xfd6b => (0u8, 7u32, false, false),
        0xfd6c => (0, 7, true, true),
        0xfd6d => (0, 7, true, false),
        0xfd8b => (1, 15, false, false),
        0xfd8c => (1, 15, true, true),
        0xfd8d => (1, 15, true, false),
        0xfdab => (2, 31, false, false),
        0xfdac => (2, 31, true, true),
        0xfdad => (2, 31, true, false),
        0xfdcb => (3, 63, false, false),
        0xfdcc => (3, 63, true, true),
        0xfdcd => (3, 63, true, false),
        _ => return Err(format!("aarch64: unsupported V128Shift opcode {op:#x}")),
    };
    asm.mov_imm32(SGP2, mask);
    asm.and_r(false, SGP2, shift, SGP2);
    if is_right {
        // NEON SSHL/USHL interprets negative signed shift count in each lane as right shift!
        asm.neg_r(size == 3, SGP2, SGP2);
    }
    asm.dup_gp_128(size, SFP2, SGP2);
    if is_signed {
        asm.sshl_v128(size, d, v, SFP2);
    } else {
        asm.ushl_v128(size, d, v, SFP2);
    }
    Ok(())
}
