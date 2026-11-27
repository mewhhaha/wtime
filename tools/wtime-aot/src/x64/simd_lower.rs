//! Complete 128-bit Wasm SIMD (`0xfd00..=0xfdff`) x86-64 SSE2/SSSE3/SSE4.1/SSE4.2 lowering.
//!
//! Uses `SCRATCH_FP1` (`Xmm15`), `SCRATCH_FP2` (`Xmm14`), `SCRATCH_GP1` (`R11`),
//! `SCRATCH_GP2` (`R10`), and the 16-byte aligned scratch slot `[R15 + OFF_SCRATCH_V128]`
//! inside `Ctx` so complex multi-step ops never touch `Rsp` and never clobber any
//! allocator-assigned GP or XMM register.

use super::asm::{Asm, Cond, Gp, Mem, RelocTarget, Xmm};
use super::ir::{SimdLaneKind, SimdSplatKind};
use super::lower::TargetIsa;
use super::{OFF_SCRATCH_V128, RodataConsts};

/// Internal `V128Bin` pseudo-opcode for SSE2 `psadbw` (sum of absolute differences of 8-bit lanes
/// into two 64-bit lanes; used by the byte-sum reduction vectorizer with a zero operand).
pub const V128_OP_PSADBW: u16 = 0xfe00;

const SGP1: Gp = Gp::R11;
const SGP2: Gp = Gp::R10;
const SFP1: Xmm = Xmm::Xmm15;
const SFP2: Xmm = Xmm::Xmm14;

#[inline]
fn ctx_v128_tmp() -> Mem {
    Mem::base_disp(Gp::R15, OFF_SCRATCH_V128)
}

fn load_local_v128(asm: &mut Asm, dst: Xmm, bytes: &[u8; 16]) {
    let off = asm.add_local_rodata(16, bytes);
    asm.movdqu_rip(dst, RelocTarget::LocalRodata(off));
}

fn splat16_u8(b: u8) -> [u8; 16] {
    [b; 16]
}

fn splat8_u16(w: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bs = w.to_le_bytes();
    for i in 0..8 {
        out[i * 2..i * 2 + 2].copy_from_slice(&bs);
    }
    out
}

fn splat4_u32(v: u32) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bs = v.to_le_bytes();
    for i in 0..4 {
        out[i * 4..i * 4 + 4].copy_from_slice(&bs);
    }
    out
}

fn splat2_u64(v: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bs = v.to_le_bytes();
    out[0..8].copy_from_slice(&bs);
    out[8..16].copy_from_slice(&bs);
    out
}

/// Emits `V128Splat`: GP or FP scalar into `dst`.
pub fn emit_v128_splat(
    asm: &mut Asm,
    kind: SimdSplatKind,
    dst: Xmm,
    gp_src: Option<Gp>,
    fp_src: Option<Xmm>,
    isa: TargetIsa,
) {
    if isa.has_avx2 {
        match kind {
            SimdSplatKind::I8x16 => {
                asm.vmovd_xr(dst, gp_src.unwrap());
                asm.vpbroadcastb_xx(dst, dst);
            }
            SimdSplatKind::I16x8 => {
                asm.vmovd_xr(dst, gp_src.unwrap());
                asm.vpbroadcastw_xx(dst, dst);
            }
            SimdSplatKind::I32x4 => {
                asm.vmovd_xr(dst, gp_src.unwrap());
                asm.vpbroadcastd_xx(dst, dst);
            }
            SimdSplatKind::I64x2 => {
                asm.vmovq_xr(dst, gp_src.unwrap());
                asm.vpbroadcastq_xx(dst, dst);
            }
            SimdSplatKind::F32x4 => {
                asm.vbroadcastss_xx(dst, fp_src.unwrap());
            }
            SimdSplatKind::F64x2 => {
                asm.vmovddup_xx(dst, fp_src.unwrap());
            }
        }
        return;
    }
    match kind {
        SimdSplatKind::I8x16 => {
            asm.movd_xr(dst, gp_src.unwrap());
            asm.punpcklbw(dst, dst);
            asm.punpcklwd(dst, dst);
            asm.punpckldq(dst, dst);
            asm.punpcklqdq(dst, dst);
        }
        SimdSplatKind::I16x8 => {
            asm.movd_xr(dst, gp_src.unwrap());
            asm.punpcklwd(dst, dst);
            asm.punpckldq(dst, dst);
            asm.punpcklqdq(dst, dst);
        }
        SimdSplatKind::I32x4 => {
            asm.movd_xr(dst, gp_src.unwrap());
            asm.pshufd(dst, dst, 0);
        }
        SimdSplatKind::I64x2 => {
            asm.movq_xr(dst, gp_src.unwrap());
            asm.punpcklqdq(dst, dst);
        }
        SimdSplatKind::F32x4 => {
            asm.pshufd(dst, fp_src.unwrap(), 0);
        }
        SimdSplatKind::F64x2 => {
            asm.pshufd(dst, fp_src.unwrap(), 0x44);
        }
    }
}

/// Emits `V128ExtractLane` into GP (`i8/i16/i32/i64`) or FP (`f32/f64`).
pub fn emit_v128_extract_lane(
    asm: &mut Asm,
    kind: SimdLaneKind,
    v: Xmm,
    lane: u8,
    gp_dst: Option<Gp>,
    fp_dst: Option<Xmm>,
    has_avx2: bool,
) {
    match kind {
        SimdLaneKind::I8x16S => {
            let d = gp_dst.unwrap();
            if has_avx2 {
                asm.vpextrb(d, v, lane);
            } else {
                asm.pextrb(d, v, lane);
            }
            asm.movsx_rr8(false, d, d);
        }
        SimdLaneKind::I8x16U => {
            let d = gp_dst.unwrap();
            if has_avx2 {
                asm.vpextrb(d, v, lane);
            } else {
                asm.pextrb(d, v, lane);
            }
        }
        SimdLaneKind::I16x8S => {
            let d = gp_dst.unwrap();
            if has_avx2 {
                asm.vpextrw(d, v, lane);
            } else {
                asm.pextrw(d, v, lane);
            }
            asm.movsx_rr16(false, d, d);
        }
        SimdLaneKind::I16x8U => {
            let d = gp_dst.unwrap();
            if has_avx2 {
                asm.vpextrw(d, v, lane);
            } else {
                asm.pextrw(d, v, lane);
            }
        }
        SimdLaneKind::I32x4 => {
            let d = gp_dst.unwrap();
            if lane == 0 {
                asm.movd_rx(d, v);
            } else if has_avx2 {
                asm.vpextrd(d, v, lane);
            } else {
                asm.pextrd(d, v, lane);
            }
        }
        SimdLaneKind::I64x2 => {
            let d = gp_dst.unwrap();
            if lane == 0 {
                asm.movq_rx(d, v);
            } else if has_avx2 {
                asm.vpextrq(d, v, lane);
            } else {
                asm.pextrq(d, v, lane);
            }
        }
        SimdLaneKind::F32x4 => {
            let d = fp_dst.unwrap();
            if lane == 0 {
                asm.movaps_rr(d, v);
            } else if has_avx2 {
                asm.vpshufd(d, v, lane);
            } else {
                asm.pshufd(d, v, lane);
            }
        }
        SimdLaneKind::F64x2 => {
            let d = fp_dst.unwrap();
            if lane == 0 {
                asm.movaps_rr(d, v);
            } else if has_avx2 {
                asm.vpshufd(d, v, 0xee);
            } else {
                asm.pshufd(d, v, 0xee);
            }
        }
    }
}

/// Emits `V128ReplaceLane`: `dst` gets `v` with `lane` replaced by `x`.
#[allow(clippy::too_many_arguments)]
pub fn emit_v128_replace_lane(
    asm: &mut Asm,
    kind: SimdLaneKind,
    dst: Xmm,
    v: Xmm,
    lane: u8,
    gp_x: Option<Gp>,
    fp_x: Option<Xmm>,
    isa: TargetIsa,
) {
    if isa.has_avx2 {
        match kind {
            SimdLaneKind::I8x16S | SimdLaneKind::I8x16U => {
                asm.vpinsrb(dst, v, gp_x.unwrap(), lane);
            }
            SimdLaneKind::I16x8S | SimdLaneKind::I16x8U => {
                asm.vpinsrw(dst, v, gp_x.unwrap(), lane);
            }
            SimdLaneKind::I32x4 => {
                asm.vpinsrd(dst, v, gp_x.unwrap(), lane);
            }
            SimdLaneKind::I64x2 => {
                asm.vpinsrq(dst, v, gp_x.unwrap(), lane);
            }
            SimdLaneKind::F32x4 => {
                asm.vmovd_rx(SGP1, fp_x.unwrap());
                asm.vpinsrd(dst, v, SGP1, lane);
            }
            SimdLaneKind::F64x2 => {
                asm.vmovq_rx(SGP1, fp_x.unwrap());
                asm.vpinsrq(dst, v, SGP1, lane);
            }
        }
        return;
    }
    match kind {
        SimdLaneKind::I8x16S | SimdLaneKind::I8x16U => {
            asm.movaps_rr(dst, v);
            asm.pinsrb(dst, gp_x.unwrap(), lane);
        }
        SimdLaneKind::I16x8S | SimdLaneKind::I16x8U => {
            asm.movaps_rr(dst, v);
            asm.pinsrw(dst, gp_x.unwrap(), lane);
        }
        SimdLaneKind::I32x4 => {
            asm.movaps_rr(dst, v);
            asm.pinsrd(dst, gp_x.unwrap(), lane);
        }
        SimdLaneKind::I64x2 => {
            asm.movaps_rr(dst, v);
            asm.pinsrq(dst, gp_x.unwrap(), lane);
        }
        SimdLaneKind::F32x4 => {
            // Read `fp_x` into SGP1 BEFORE writing `dst` in case `dst == fp_x`!
            asm.movd_rx(SGP1, fp_x.unwrap());
            asm.movaps_rr(dst, v);
            asm.pinsrd(dst, SGP1, lane);
        }
        SimdLaneKind::F64x2 => {
            asm.movq_rx(SGP1, fp_x.unwrap());
            asm.movaps_rr(dst, v);
            asm.pinsrq(dst, SGP1, lane);
        }
    }
}

/// Emits `i8x16.shuffle(a, b, lanes)` into `dst` (safe for any `dst`/`a`/`b` aliasing).
/// Recognizes single-instruction 32-bit/64-bit/16-bit/8-bit unpacks, `shufps`, `pshufd`, and
/// `palignr` slides before falling back to `vpshufb`/`pshufb`.
pub fn emit_v128_shuffle(
    asm: &mut Asm,
    dst: Xmm,
    a: Xmm,
    b: Xmm,
    lanes: &[u8; 16],
    isa: TargetIsa,
) {
    // 0. Identity moves
    if (0..16u8).all(|i| lanes[i as usize] == i) {
        asm.movaps_rr(dst, a);
        return;
    }
    if (0..16u8).all(|i| lanes[i as usize] == i + 16) {
        asm.movaps_rr(dst, b);
        return;
    }

    // 1. 32-bit lane-aligned patterns (`vpshufd`, `vunpcklps`, `vunpckhps`, `vshufps` — covers all 4x4 transposes!)
    let is_u32_aligned = (0..4).all(|k| {
        let base = lanes[4 * k];
        (base & 3) == 0
            && lanes[4 * k + 1] == base + 1
            && lanes[4 * k + 2] == base + 2
            && lanes[4 * k + 3] == base + 3
    });
    if is_u32_aligned {
        let d32 = [lanes[0] / 4, lanes[4] / 4, lanes[8] / 4, lanes[12] / 4];
        if d32.iter().all(|&x| x < 4) {
            let imm = d32[0] | (d32[1] << 2) | (d32[2] << 4) | (d32[3] << 6);
            if isa.has_avx2 {
                asm.vpshufd(dst, a, imm);
            } else {
                asm.pshufd(dst, a, imm);
            }
            return;
        }
        if d32.iter().all(|&x| x >= 4) {
            let imm =
                (d32[0] - 4) | ((d32[1] - 4) << 2) | ((d32[2] - 4) << 4) | ((d32[3] - 4) << 6);
            if isa.has_avx2 {
                asm.vpshufd(dst, b, imm);
            } else {
                asm.pshufd(dst, b, imm);
            }
            return;
        }
        // 2-source `unpcklps` / `unpckhps` / `shufps`
        if d32 == [0, 4, 1, 5] {
            if isa.has_avx2 {
                asm.vunpcklps_xxx(dst, a, b);
            } else {
                let r = prep_bin(asm, dst, a, b);
                asm.unpcklps(dst, r);
            }
            return;
        }
        if d32 == [4, 0, 5, 1] {
            if isa.has_avx2 {
                asm.vunpcklps_xxx(dst, b, a);
            } else {
                let r = prep_bin_swap(asm, dst, a, b);
                asm.unpcklps(dst, r);
            }
            return;
        }
        if d32 == [2, 6, 3, 7] {
            if isa.has_avx2 {
                asm.vunpckhps_xxx(dst, a, b);
            } else {
                let r = prep_bin(asm, dst, a, b);
                asm.unpckhps(dst, r);
            }
            return;
        }
        if d32 == [6, 2, 7, 3] {
            if isa.has_avx2 {
                asm.vunpckhps_xxx(dst, b, a);
            } else {
                let r = prep_bin_swap(asm, dst, a, b);
                asm.unpckhps(dst, r);
            }
            return;
        }
        if d32[0] < 4 && d32[1] < 4 && d32[2] >= 4 && d32[3] >= 4 {
            let imm = d32[0] | (d32[1] << 2) | ((d32[2] - 4) << 4) | ((d32[3] - 4) << 6);
            if isa.has_avx2 {
                asm.vshufps_xxxi(dst, a, b, imm);
            } else {
                let r = prep_bin(asm, dst, a, b);
                asm.shufps(dst, r, imm);
            }
            return;
        }
        if d32[0] >= 4 && d32[1] >= 4 && d32[2] < 4 && d32[3] < 4 {
            let imm = (d32[0] - 4) | ((d32[1] - 4) << 2) | (d32[2] << 4) | (d32[3] << 6);
            if isa.has_avx2 {
                asm.vshufps_xxxi(dst, b, a, imm);
            } else {
                let r = prep_bin_swap(asm, dst, a, b);
                asm.shufps(dst, r, imm);
            }
            return;
        }
    }

    // 2. Contiguous 16-byte slide (`vpalignr` / `palignr`)
    let start = lanes[0];
    if start > 0 && start < 16 && (0..16u8).all(|i| lanes[i as usize] == start + i) {
        if isa.has_avx2 {
            asm.vpalignr_xxxi(dst, b, a, start);
        } else {
            let r = prep_bin_swap(asm, dst, a, b);
            asm.palignr(dst, r, start);
        }
        return;
    }
    if start > 16 && (0..16u8).all(|i| lanes[i as usize] == (start + i) % 32) {
        let imm = start - 16;
        if isa.has_avx2 {
            asm.vpalignr_xxxi(dst, a, b, imm);
        } else {
            let r = prep_bin(asm, dst, a, b);
            asm.palignr(dst, r, imm);
        }
        return;
    }
    if start > 0 && start < 16 && (0..16u8).all(|i| lanes[i as usize] == (start + i) % 16) {
        if isa.has_avx2 {
            asm.vpalignr_xxxi(dst, a, a, start);
        } else {
            asm.movaps_rr(dst, a);
            asm.palignr(dst, dst, start);
        }
        return;
    }
    if start > 16 && (0..16u8).all(|i| lanes[i as usize] == 16 + ((start - 16 + i) % 16)) {
        let imm = start - 16;
        if isa.has_avx2 {
            asm.vpalignr_xxxi(dst, b, b, imm);
        } else {
            asm.movaps_rr(dst, b);
            asm.palignr(dst, dst, imm);
        }
        return;
    }

    // 3. Exact byte/word interleaves (`punpcklbw`/`hbw`/`lwd`/`hwd`)
    let is_unpck_b = |lo_off: u8, swap: bool| {
        (0..8u8).all(|k| {
            let x = lo_off + k;
            let (e0, e1) = if swap { (x + 16, x) } else { (x, x + 16) };
            lanes[(2 * k) as usize] == e0 && lanes[(2 * k + 1) as usize] == e1
        })
    };
    if is_unpck_b(0, false) {
        if isa.has_avx2 {
            asm.vpunpcklbw_xxx(dst, a, b);
        } else {
            let r = prep_bin(asm, dst, a, b);
            asm.punpcklbw(dst, r);
        }
        return;
    }
    if is_unpck_b(0, true) {
        if isa.has_avx2 {
            asm.vpunpcklbw_xxx(dst, b, a);
        } else {
            let r = prep_bin_swap(asm, dst, a, b);
            asm.punpcklbw(dst, r);
        }
        return;
    }
    if is_unpck_b(8, false) {
        if isa.has_avx2 {
            asm.vpunpckhbw_xxx(dst, a, b);
        } else {
            let r = prep_bin(asm, dst, a, b);
            asm.punpckhbw(dst, r);
        }
        return;
    }
    if is_unpck_b(8, true) {
        if isa.has_avx2 {
            asm.vpunpckhbw_xxx(dst, b, a);
        } else {
            let r = prep_bin_swap(asm, dst, a, b);
            asm.punpckhbw(dst, r);
        }
        return;
    }
    let is_unpck_w = |lo_off: u8, swap: bool| {
        (0..4u8).all(|k| {
            let x = lo_off + 2 * k;
            let (e0, e1) = if swap { (x + 16, x) } else { (x, x + 16) };
            lanes[(4 * k) as usize] == e0
                && lanes[(4 * k + 1) as usize] == e0 + 1
                && lanes[(4 * k + 2) as usize] == e1
                && lanes[(4 * k + 3) as usize] == e1 + 1
        })
    };
    if is_unpck_w(0, false) {
        if isa.has_avx2 {
            asm.vpunpcklwd_xxx(dst, a, b);
        } else {
            let r = prep_bin(asm, dst, a, b);
            asm.punpcklwd(dst, r);
        }
        return;
    }
    if is_unpck_w(0, true) {
        if isa.has_avx2 {
            asm.vpunpcklwd_xxx(dst, b, a);
        } else {
            let r = prep_bin_swap(asm, dst, a, b);
            asm.punpcklwd(dst, r);
        }
        return;
    }
    if is_unpck_w(8, false) {
        if isa.has_avx2 {
            asm.vpunpckhwd_xxx(dst, a, b);
        } else {
            let r = prep_bin(asm, dst, a, b);
            asm.punpckhwd(dst, r);
        }
        return;
    }
    if is_unpck_w(8, true) {
        if isa.has_avx2 {
            asm.vpunpckhwd_xxx(dst, b, a);
        } else {
            let r = prep_bin_swap(asm, dst, a, b);
            asm.punpckhwd(dst, r);
        }
        return;
    }

    // 4. General byte shuffle (`vpshufb` / `pshufb`)
    let mut mask_a = [0x80u8; 16];
    let mut mask_b = [0x80u8; 16];
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
    if isa.has_avx2 {
        if !uses_b {
            let off = asm.add_local_rodata(16, &mask_a);
            asm.vpshufb_xx_rip(dst, a, RelocTarget::LocalRodata(off));
            return;
        }
        if !uses_a {
            let off = asm.add_local_rodata(16, &mask_b);
            asm.vpshufb_xx_rip(dst, b, RelocTarget::LocalRodata(off));
            return;
        }
        // Compute `b` into `SFP2` FIRST so even `dst == b` (or `dst == a == b`) is 100% alias-safe with zero copies!
        let off_b = asm.add_local_rodata(16, &mask_b);
        let off_a = asm.add_local_rodata(16, &mask_a);
        asm.vpshufb_xx_rip(SFP2, b, RelocTarget::LocalRodata(off_b));
        asm.vpshufb_xx_rip(dst, a, RelocTarget::LocalRodata(off_a));
        asm.vpor_xxx(dst, dst, SFP2);
        return;
    }
    if !uses_b {
        asm.movaps_rr(dst, a);
        let off = asm.add_local_rodata(16, &mask_a);
        asm.pshufb_rip(dst, RelocTarget::LocalRodata(off));
        return;
    }
    if !uses_a {
        asm.movaps_rr(dst, b);
        let off = asm.add_local_rodata(16, &mask_b);
        asm.pshufb_rip(dst, RelocTarget::LocalRodata(off));
        return;
    }
    // Both inputs used: keep `b` safe in SFP2 if `dst == b` (remember `dst != SFP2`).
    let b_reg = if dst == b {
        asm.movaps_rr(SFP2, b);
        SFP2
    } else {
        b
    };
    asm.movaps_rr(dst, a);
    let off_a = asm.add_local_rodata(16, &mask_a);
    asm.pshufb_rip(dst, RelocTarget::LocalRodata(off_a));
    asm.movaps_rr(SFP2, b_reg);
    let off_b = asm.add_local_rodata(16, &mask_b);
    asm.pshufb_rip(SFP2, RelocTarget::LocalRodata(off_b));
    asm.por(dst, SFP2);
}

/// Emits `v128.bitselect(a, b, c)` = `(a & c) | (b & !c)` = `((a ^ b) & c) ^ b`.
/// `a_reg`, `b_reg`, `c_reg` are loaded with `a_reg` possibly SFP1, `b_reg` possibly SFP2,
/// and `c` passed via `c_xmm` or saved in `ctx_v128_tmp()`.
pub fn emit_v128_bitselect(
    asm: &mut Asm,
    dst: Xmm,
    a: Xmm,
    b: Xmm,
    c_in_tmp: bool,
    c: Xmm,
    isa: TargetIsa,
) {
    if isa.has_avx2 {
        if !c_in_tmp {
            // `c` is an allocated register (`Xmm0..13`), `a != SFP2`, `dst != SFP2`.
            // Step 1 writes `SFP2 = !c & b` (safe even if `b == SFP2`), keeping `a` and `c` intact.
            asm.vpandn_xxx(SFP2, c, b);
            asm.vpand_xxx(dst, a, c);
            asm.vpor_xxx(dst, dst, SFP2);
        } else {
            // `c` is in `ctx_v128_tmp()`: compute `((a ^ b) & [tmp]) ^ b`.
            // Keep `b` in `SFP2` if `dst == b` (safe even if `b == SFP2`).
            let b_saved = if dst == b {
                asm.vmovaps_rr(SFP2, b);
                SFP2
            } else {
                b
            };
            asm.vpxor_xxx(dst, a, b_saved);
            asm.vpand_xxm(dst, dst, ctx_v128_tmp());
            asm.vpxor_xxx(dst, dst, b_saved);
        }
        return;
    }
    if !c_in_tmp {
        asm.movdqu_mx(ctx_v128_tmp(), c);
    }
    let b_saved = if dst == b {
        asm.movaps_rr(SFP2, b);
        SFP2
    } else {
        b
    };
    asm.movaps_rr(dst, a);
    asm.pxor(dst, b_saved);
    asm.andps_xm(dst, ctx_v128_tmp());
    asm.pxor(dst, b_saved);
}

/// Emits all `V128 -> I32` ops (`any_true`, `all_true`, `bitmask`).
pub fn emit_v128_to_i32(
    asm: &mut Asm,
    op: u16,
    dst: Gp,
    a: Xmm,
    isa: TargetIsa,
) -> Result<(), String> {
    if isa.has_avx2 {
        match op {
            0xfd53 => {
                asm.vptest(a, a);
                asm.setcc(Cond::Ne, dst);
                asm.movzx_rr8(dst, dst);
            }
            0xfd63 | 0xfd83 | 0xfda3 | 0xfdc3 => {
                asm.vpxor_xxx(SFP2, SFP2, SFP2);
                match op {
                    0xfd63 => asm.vpcmpeqb_xxx(SFP2, SFP2, a),
                    0xfd83 => asm.vpcmpeqw_xxx(SFP2, SFP2, a),
                    0xfda3 => asm.vpcmpeqd_xxx(SFP2, SFP2, a),
                    _ => asm.vpcmpeqq_xxx(SFP2, SFP2, a),
                }
                asm.vptest(SFP2, SFP2);
                asm.setcc(Cond::E, dst);
                asm.movzx_rr8(dst, dst);
            }
            0xfd64 => asm.vpmovmskb(dst, a),
            0xfd84 => {
                asm.vpxor_xxx(SFP2, SFP2, SFP2);
                asm.vpacksswb_xxx(SFP2, a, SFP2);
                asm.vpmovmskb(dst, SFP2);
            }
            0xfda4 => asm.vmovmskps(dst, a),
            0xfdc4 => asm.vmovmskpd(dst, a),
            _ => return Err(format!("x64: unknown V128ToI32 opcode {op:#x}")),
        }
        return Ok(());
    }
    match op {
        0xfd53 => {
            // v128.any_true
            asm.ptest(a, a);
            asm.setcc(Cond::Ne, dst);
            asm.movzx_rr8(dst, dst);
        }
        0xfd63 => {
            // i8x16.all_true
            asm.pxor(SFP2, SFP2);
            asm.pcmpeqb(SFP2, a);
            asm.ptest(SFP2, SFP2);
            asm.setcc(Cond::E, dst);
            asm.movzx_rr8(dst, dst);
        }
        0xfd83 => {
            // i16x8.all_true
            asm.pxor(SFP2, SFP2);
            asm.pcmpeqw(SFP2, a);
            asm.ptest(SFP2, SFP2);
            asm.setcc(Cond::E, dst);
            asm.movzx_rr8(dst, dst);
        }
        0xfda3 => {
            // i32x4.all_true
            asm.pxor(SFP2, SFP2);
            asm.pcmpeqd(SFP2, a);
            asm.ptest(SFP2, SFP2);
            asm.setcc(Cond::E, dst);
            asm.movzx_rr8(dst, dst);
        }
        0xfdc3 => {
            // i64x2.all_true
            asm.pxor(SFP2, SFP2);
            asm.pcmpeqq(SFP2, a);
            asm.ptest(SFP2, SFP2);
            asm.setcc(Cond::E, dst);
            asm.movzx_rr8(dst, dst);
        }
        0xfd64 => {
            // i8x16.bitmask
            asm.pmovmskb(dst, a);
        }
        0xfd84 => {
            // i16x8.bitmask: packsswb(a, 0) preserves sign bit of each i16 lane in low 8 bytes.
            asm.movaps_rr(SFP2, a);
            asm.pxor(SFP1, SFP1);
            asm.packsswb(SFP2, SFP1);
            asm.pmovmskb(dst, SFP2);
        }
        0xfda4 => {
            // i32x4.bitmask
            asm.movmskps(dst, a);
        }
        0xfdc4 => {
            // i64x2.bitmask
            asm.movmskpd(dst, a);
        }
        _ => return Err(format!("x64: unknown V128ToI32 opcode {op:#x}")),
    }
    Ok(())
}

/// Emits all `V128 -> V128` unary ops.
/// Note: `dst` may alias `a` or `SFP1`, but NEVER equals `SFP2`.
pub fn emit_v128_un(
    asm: &mut Asm,
    op: u16,
    dst: Xmm,
    a: Xmm,
    ro: &RodataConsts,
) -> Result<(), String> {
    match op {
        0xfd4d => {
            // v128.not
            asm.movaps_rr(dst, a);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd5e => asm.cvtpd2ps(dst, a), // f32x4.demote_f64x2_zero
        0xfd5f => asm.cvtps2pd(dst, a), // f64x2.promote_low_f32x4
        0xfd60 => asm.pabsb(dst, a),    // i8x16.abs
        0xfd61 => {
            // i8x16.neg
            asm.movaps_rr(SFP2, a);
            asm.pxor(dst, dst);
            asm.psubb(dst, SFP2);
        }
        0xfd62 => {
            // i8x16.popcnt via 4-bit nibble PSHUFB lookup table
            const LUT: [u8; 16] = [0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];
            asm.movdqu_mx(ctx_v128_tmp(), a);
            load_local_v128(asm, SFP2, &splat16_u8(0x0f));
            // Low nibbles: dst = LUT[a & 0x0f]
            asm.movdqu_xm(dst, ctx_v128_tmp());
            asm.pand(dst, SFP2);
            let lut_off = asm.add_local_rodata(16, &LUT);
            // Put LUT in SFP2, shuffle by dst? pshufb dst, idx -> dst holds LUT, idx holds nibbles!
            // Save low nibbles in ctx_v128_tmp? Even better: compute hi nibbles first!
            // Re-read a, compute hi = (a >> 4) & 0x0f in SFP2:
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.psrlw_imm(SFP2, 4);
            // Save low nibbles in ctx_v128_tmp:
            asm.movdqu_mx(ctx_v128_tmp(), dst);
            load_local_v128(asm, dst, &splat16_u8(0x0f));
            asm.pand(SFP2, dst); // SFP2 = hi nibbles
            asm.movdqu_rip(dst, RelocTarget::LocalRodata(lut_off));
            asm.pshufb(dst, SFP2); // dst = popcnt(hi)
            asm.movdqu_xm(SFP2, ctx_v128_tmp()); // SFP2 = lo nibbles
            asm.movdqu_mx(ctx_v128_tmp(), dst); // save popcnt(hi)
            asm.movdqu_rip(dst, RelocTarget::LocalRodata(lut_off));
            asm.pshufb(dst, SFP2); // dst = popcnt(lo)
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.paddb(dst, SFP2);
        }
        0xfd67 => asm.roundps(dst, a, 0x0a), // f32x4.ceil
        0xfd68 => asm.roundps(dst, a, 0x09), // f32x4.floor
        0xfd69 => asm.roundps(dst, a, 0x0b), // f32x4.trunc
        0xfd6a => asm.roundps(dst, a, 0x08), // f32x4.nearest
        0xfd74 => asm.roundpd(dst, a, 0x0a), // f64x2.ceil
        0xfd75 => asm.roundpd(dst, a, 0x09), // f64x2.floor
        0xfd7a => asm.roundpd(dst, a, 0x0b), // f64x2.trunc
        0xfd94 => asm.roundpd(dst, a, 0x08), // f64x2.nearest
        0xfd7c => {
            // i16x8.extadd_pairwise_i8x16_s: pmaddubsw([1u8;16], a)
            asm.movaps_rr(SFP2, a);
            load_local_v128(asm, dst, &splat16_u8(1));
            asm.pmaddubsw(dst, SFP2);
        }
        0xfd7d => {
            // i16x8.extadd_pairwise_i8x16_u: pmaddubsw(a, [1i8;16])
            asm.movaps_rr(dst, a);
            load_local_v128(asm, SFP2, &splat16_u8(1));
            asm.pmaddubsw(dst, SFP2);
        }
        0xfd7e => {
            // i32x4.extadd_pairwise_i16x8_s: pmaddwd(a, [1i16;8])
            asm.movaps_rr(dst, a);
            load_local_v128(asm, SFP2, &splat8_u16(1));
            asm.pmaddwd(dst, SFP2);
        }
        0xfd7f => {
            // i32x4.extadd_pairwise_i16x8_u
            asm.movaps_rr(dst, a);
            load_local_v128(asm, SFP2, &splat8_u16(0x8000));
            asm.pxor(dst, SFP2);
            load_local_v128(asm, SFP2, &splat8_u16(1));
            asm.pmaddwd(dst, SFP2);
            load_local_v128(asm, SFP2, &splat4_u32(0x10000));
            asm.paddd(dst, SFP2);
        }
        0xfd80 => asm.pabsw(dst, a), // i16x8.abs
        0xfd81 => {
            asm.movaps_rr(SFP2, a);
            asm.pxor(dst, dst);
            asm.psubw(dst, SFP2);
        }
        0xfd87 => asm.pmovsxbw(dst, a), // i16x8.extend_low_i8x16_s
        0xfd88 => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovsxbw(dst, dst);
        }
        0xfd89 => asm.pmovzxbw(dst, a), // i16x8.extend_low_i8x16_u
        0xfd8a => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovzxbw(dst, dst);
        }
        0xfda0 => asm.pabsd(dst, a), // i32x4.abs
        0xfda1 => {
            asm.movaps_rr(SFP2, a);
            asm.pxor(dst, dst);
            asm.psubd(dst, SFP2);
        }
        0xfda7 => asm.pmovsxwd(dst, a), // i32x4.extend_low_i16x8_s
        0xfda8 => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovsxwd(dst, dst);
        }
        0xfda9 => asm.pmovzxwd(dst, a), // i32x4.extend_low_i16x8_u
        super::ir::V128_UN_I32X4_EXTEND_LOW_I8X16_U => asm.pmovzxbd(dst, a), // zero-extend low 4 u8 -> i32x4
        0xfdaa => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovzxwd(dst, dst);
        }
        0xfdc0 => {
            // i64x2.abs: sign = psrad(pshufd(a, 0xf5), 31); (a ^ sign) - sign
            asm.movaps_rr(dst, a);
            asm.pshufd(SFP2, dst, 0xf5);
            asm.psrad_imm(SFP2, 31);
            asm.pxor(dst, SFP2);
            asm.psubq(dst, SFP2);
        }
        0xfdc1 => {
            asm.movaps_rr(SFP2, a);
            asm.pxor(dst, dst);
            asm.psubq(dst, SFP2);
        }
        0xfdc7 => asm.pmovsxdq(dst, a), // i64x2.extend_low_i32x4_s
        0xfdc8 => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovsxdq(dst, dst);
        }
        0xfdc9 => asm.pmovzxdq(dst, a), // i64x2.extend_low_i32x4_u
        0xfdca => {
            asm.movaps_rr(dst, a);
            asm.psrldq_imm(dst, 8);
            asm.pmovzxdq(dst, dst);
        }
        0xfde0 => {
            asm.movaps_rr(dst, a);
            asm.xmm_rip_op(None, 0x54, dst, RelocTarget::Rodata(ro.f32_abs));
        }
        0xfde1 => {
            asm.movaps_rr(dst, a);
            asm.xmm_rip_op(None, 0x57, dst, RelocTarget::Rodata(ro.f32_neg));
        }
        0xfde3 => asm.sqrtps(dst, a),
        0xfdec => {
            asm.movaps_rr(dst, a);
            asm.xmm_rip_op(Some(0x66), 0x54, dst, RelocTarget::Rodata(ro.f64_abs));
        }
        0xfded => {
            asm.movaps_rr(dst, a);
            asm.xmm_rip_op(Some(0x66), 0x57, dst, RelocTarget::Rodata(ro.f64_neg));
        }
        0xfdef => asm.sqrtpd(dst, a),
        0xfdfa => asm.cvtdq2ps(dst, a), // f32x4.convert_i32x4_s
        0xfdfb => {
            // f32x4.convert_i32x4_u: lo = a & 0xffff, hi = a >> 16, float(lo) + float(hi) * 65536.0
            asm.movaps_rr(dst, a);
            asm.movaps_rr(SFP2, dst);
            asm.psrld_imm(SFP2, 16); // hi
            asm.movdqu_mx(ctx_v128_tmp(), SFP2);
            load_local_v128(asm, SFP2, &splat4_u32(0x0000_ffff));
            asm.pand(dst, SFP2); // lo
            asm.cvtdq2ps(dst, dst);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.cvtdq2ps(SFP2, SFP2);
            asm.movdqu_mx(ctx_v128_tmp(), dst);
            load_local_v128(asm, dst, &splat4_u32(65536.0f32.to_bits()));
            asm.mulps(SFP2, dst);
            asm.movdqu_xm(dst, ctx_v128_tmp());
            asm.addps(dst, SFP2);
        }
        0xfdfe => asm.cvtdq2pd(dst, a), // f64x2.convert_low_i32x4_s
        0xfdff => {
            // f64x2.convert_low_i32x4_u
            let two52 = splat2_u64(0x4330_0000_0000_0000);
            asm.pmovzxdq(dst, a);
            load_local_v128(asm, SFP2, &two52);
            asm.por(dst, SFP2);
            asm.subpd(dst, SFP2);
        }
        _ => return Err(format!("x64: unknown V128Un opcode {op:#x}")),
    }
    Ok(())
}

/// Complete unary entry handling the trunc_sat ops with zero `dst == SFP1` aliasing!
pub fn emit_v128_un_full(
    asm: &mut Asm,
    op: u16,
    dst: Xmm,
    a: Xmm,
    ro: &RodataConsts,
    isa: TargetIsa,
) -> Result<(), String> {
    if isa.has_avx512 && op == 0xfdfb {
        // AVX-512VL 1-instruction bit-exact `f32x4.convert_i32x4_u`!
        asm.vcvtudq2ps_xx(dst, a);
        return Ok(());
    }
    if isa.has_avx512 && op == 0xfd4d {
        // AVX-512VL 1-instruction native EVEX `v128.not` (`!a`, works directly on `Xmm0..Xmm31`).
        asm.vpternlogd_xxx(dst, a, a, 0x55);
        return Ok(());
    }
    if isa.has_avx2 {
        match op {
            0xfd4d => {
                asm.vpcmpeqd_xxx(SFP2, SFP2, SFP2);
                asm.vpxor_xxx(dst, a, SFP2);
                return Ok(());
            }
            0xfd5e => {
                asm.vcvtpd2ps_xx(dst, a);
                return Ok(());
            }
            0xfd5f => {
                asm.vcvtps2pd_xx(dst, a);
                return Ok(());
            }
            0xfd60 => {
                asm.vpabsb_xx(dst, a);
                return Ok(());
            }
            0xfd61 | 0xfd81 | 0xfda1 | 0xfdc1 => {
                asm.vpxor_xxx(SFP2, SFP2, SFP2);
                match op {
                    0xfd61 => asm.vpsubb_xxx(dst, SFP2, a),
                    0xfd81 => asm.vpsubw_xxx(dst, SFP2, a),
                    0xfda1 => asm.vpsubd_xxx(dst, SFP2, a),
                    _ => asm.vpsubq_xxx(dst, SFP2, a),
                }
                return Ok(());
            }
            0xfd67 => {
                asm.vroundps_xxi(dst, a, 0x0a);
                return Ok(());
            }
            0xfd68 => {
                asm.vroundps_xxi(dst, a, 0x09);
                return Ok(());
            }
            0xfd69 => {
                asm.vroundps_xxi(dst, a, 0x0b);
                return Ok(());
            }
            0xfd6a => {
                asm.vroundps_xxi(dst, a, 0x08);
                return Ok(());
            }
            0xfd74 => {
                asm.vroundpd_xxi(dst, a, 0x0a);
                return Ok(());
            }
            0xfd75 => {
                asm.vroundpd_xxi(dst, a, 0x09);
                return Ok(());
            }
            0xfd7a => {
                asm.vroundpd_xxi(dst, a, 0x0b);
                return Ok(());
            }
            0xfd94 => {
                asm.vroundpd_xxi(dst, a, 0x08);
                return Ok(());
            }
            0xfd7c => {
                load_local_v128(asm, SFP2, &splat16_u8(1));
                asm.vpmaddubsw_xxx(dst, SFP2, a);
                return Ok(());
            }
            0xfd7d => {
                load_local_v128(asm, SFP2, &splat16_u8(1));
                asm.vpmaddubsw_xxx(dst, a, SFP2);
                return Ok(());
            }
            0xfd7e => {
                load_local_v128(asm, SFP2, &splat8_u16(1));
                asm.vpmaddwd_xxx(dst, a, SFP2);
                return Ok(());
            }
            0xfd7f => {
                load_local_v128(asm, SFP2, &splat8_u16(0x8000));
                asm.vpxor_xxx(dst, a, SFP2);
                load_local_v128(asm, SFP2, &splat8_u16(1));
                asm.vpmaddwd_xxx(dst, dst, SFP2);
                load_local_v128(asm, SFP2, &splat4_u32(0x10000));
                asm.vpaddd_xxx(dst, dst, SFP2);
                return Ok(());
            }
            0xfd80 => {
                asm.vpabsw_xx(dst, a);
                return Ok(());
            }
            0xfd87 => {
                asm.vpmovsxbw_xx(dst, a);
                return Ok(());
            }
            0xfd88 => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovsxbw_xx(dst, SFP2);
                return Ok(());
            }
            0xfd89 => {
                asm.vpmovzxbw_xx(dst, a);
                return Ok(());
            }
            0xfd8a => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovzxbw_xx(dst, SFP2);
                return Ok(());
            }
            0xfda0 => {
                asm.vpabsd_xx(dst, a);
                return Ok(());
            }
            0xfda7 => {
                asm.vpmovsxwd_xx(dst, a);
                return Ok(());
            }
            0xfda8 => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovsxwd_xx(dst, SFP2);
                return Ok(());
            }
            0xfda9 => {
                asm.vpmovzxwd_xx(dst, a);
                return Ok(());
            }
            super::ir::V128_UN_I32X4_EXTEND_LOW_I8X16_U => {
                asm.vpmovzxbd_xx(dst, a);
                return Ok(());
            }
            0xfdaa => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovzxwd_xx(dst, SFP2);
                return Ok(());
            }
            0xfdc0 => {
                asm.vpshufd(SFP2, a, 0xf5);
                asm.vpsrad_xxi(SFP2, SFP2, 31);
                asm.vpxor_xxx(dst, a, SFP2);
                asm.vpsubq_xxx(dst, dst, SFP2);
                return Ok(());
            }
            0xfdc7 => {
                asm.vpmovsxdq_xx(dst, a);
                return Ok(());
            }
            0xfdc8 => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovsxdq_xx(dst, SFP2);
                return Ok(());
            }
            0xfdc9 => {
                asm.vpmovzxdq_xx(dst, a);
                return Ok(());
            }
            0xfdca => {
                asm.vpsrldq_xxi(SFP2, a, 8);
                asm.vpmovzxdq_xx(dst, SFP2);
                return Ok(());
            }
            0xfde0 => {
                asm.vandps_xx_rip(dst, a, RelocTarget::Rodata(ro.f32_abs));
                return Ok(());
            }
            0xfde1 => {
                asm.vxorps_xx_rip(dst, a, RelocTarget::Rodata(ro.f32_neg));
                return Ok(());
            }
            0xfde3 => {
                asm.vsqrtps_xx(dst, a);
                return Ok(());
            }
            0xfdec => {
                asm.vandpd_xx_rip(dst, a, RelocTarget::Rodata(ro.f64_abs));
                return Ok(());
            }
            0xfded => {
                asm.vxorpd_xx_rip(dst, a, RelocTarget::Rodata(ro.f64_neg));
                return Ok(());
            }
            0xfdef => {
                asm.vsqrtpd_xx(dst, a);
                return Ok(());
            }
            0xfdfa => {
                asm.vcvtdq2ps_xx(dst, a);
                return Ok(());
            }
            0xfdfb => {
                // AVX2 zero-spill `f32x4.convert_i32x4_u`: lo = a & 0xffff, hi = a >> 16, float(lo) + float(hi) * 65536.0
                let mask_off = asm.add_local_rodata(16, &splat4_u32(0x0000_ffff));
                let scale_off = asm.add_local_rodata(16, &splat4_u32(65536.0f32.to_bits()));
                asm.vpsrld_xxi(SFP2, a, 16);
                asm.vpand_xx_rip(dst, a, RelocTarget::LocalRodata(mask_off));
                asm.vcvtdq2ps_xx(dst, dst);
                asm.vcvtdq2ps_xx(SFP2, SFP2);
                asm.vmulps_xx_rip(SFP2, SFP2, RelocTarget::LocalRodata(scale_off));
                asm.vaddps_xxx(dst, dst, SFP2);
                return Ok(());
            }
            0xfdfe => {
                asm.vcvtdq2pd_xx(dst, a);
                return Ok(());
            }
            0xfdff => {
                let two52 = splat2_u64(0x4330_0000_0000_0000);
                asm.vpmovzxdq_xx(dst, a);
                load_local_v128(asm, SFP2, &two52);
                asm.vpor_xxx(dst, dst, SFP2);
                asm.vsubpd_xxx(dst, dst, SFP2);
                return Ok(());
            }
            _ => {}
        }
    }
    match op {
        0xfdf8 => {
            // i32x4.trunc_sat_f32x4_s (only uses `dst`, `SFP2`, and `ctx_v128_tmp`)
            asm.movaps_rr(dst, a);
            load_local_v128(asm, SFP2, &splat4_u32(2147483648.0f32.to_bits()));
            asm.cmpps(SFP2, dst, 2); // SFP2 = (2^31 <= a), 0 for NaN
            asm.movdqu_mx(ctx_v128_tmp(), SFP2);
            asm.movaps_rr(SFP2, dst);
            asm.cmpps(SFP2, SFP2, 7); // SFP2 = ord(a, a)
            asm.cvttps2dq(dst, dst);
            asm.pand(dst, SFP2);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.pxor(dst, SFP2);
            Ok(())
        }
        0xfdf9 => {
            // i32x4.trunc_sat_f32x4_u (uses only `dst`, `SFP2`, and `ctx_v128_tmp`)
            // 1. x = maxps(a, 0.0)  (NaN and <=0 become 0.0)
            asm.movaps_rr(dst, a);
            asm.pxor(SFP2, SFP2);
            asm.maxps(dst, SFP2);
            // 2. ovf = (2^32 <= x) -> save in tmp
            load_local_v128(asm, SFP2, &splat4_u32(4294967296.0f32.to_bits()));
            asm.cmpps(SFP2, dst, 2); // 2^32 <= x
            asm.movdqu_mx(ctx_v128_tmp(), SFP2);
            // 3. hi = (2^31 <= x) in SFP2
            load_local_v128(asm, SFP2, &splat4_u32(2147483648.0f32.to_bits()));
            asm.cmpps(SFP2, dst, 2); // 2^31 <= x
            // Need both `x` and `hi` to compute `x - (hi & 2^31)` AND later `pxor(cvt, hi & 0x8000_0000)`:
            // Notice: `(hi & 0x8000_0000)` is just `pslld(hi, 31)`! And `hi & 2^31_f32` is `hi & 0x4f00_0000`!
            // Save `x`? Better: if we pack `ovf` first: wait — if `x >= 2^32`, `por(..., ovf)` overrides everything anyway!
            // Even cleaner: compute `sub = (2^31 <= x) & 2^31_f32` directly:
            //   - `SFP2` has `hi` (`0` or `0xffff_ffff`)
            //   - `dst` has `x`
            //   - `tmp` has `ovf`
            // If we clamp `x` to `4294967040.0`? No, if `x >= 2^32`, `ovf` is all-1s so `por res, ovf` = `0xffff_ffff` regardless of what `cvttps2dq` returns!
            // Wait: we DO need `hi` both before and after `cvttps2dq`:
            // Notice `hi` is 0 or -1, and `2^31_f32` is `0x4f00_0000`:
            //   - `sub = hi & 0x4f00_0000`: `0` or `0x4f00_0000`
            //   - Notice `0x4f00_0000 << 7`? Bit 24 is set -> `(sub << 7)` = `0x8000_0000`!!
            //   WAIT A SECOND!! CHECK THAT!!
            //   `2147483648.0f32.to_bits() = 0x4f00_0000`!
            //   Bit 24 of `0x4f00_0000` is `1` (`0x0100_0000`), and bits 0..23 are ALL `0`!
            //   `0x4f00_0000u32 << 7 = 0x8000_0000u32`!!
            //   And `0u32 << 7 = 0u32`!!
            //   SO `sub` STILL ENCODES `hi & 0x8000_0000` VIA ONE SINGLE `pslld SFP2, 7`!!
            //   Check!
            //     - `SFP2` = `hi` (`0` or `0xffff_ffff`)
            //     - `pand SFP2, [0x4f00_0000]`? No, need a register for `[0x4f00_0000]`:
            //       Instead: `psrad` / `psrld` / `pslld`? Wait: `0xffff_ffff` -> we have `xmm_rip_op` for `andps` (`0x54`)!!
            //       `andps_rip(SFP2, LocalRodata([0x4f00_0000; 4]))`!!
            //     - `subps dst, SFP2` (`dst = x - (hi ? 2^31 : 0)`)
            //     - `cvttps2dq dst, dst`
            //     - `pslld_imm SFP2, 7` (`0x4f00_0000 << 7 = 0x8000_0000`, `0 << 7 = 0`!)
            //     - `pxor dst, SFP2` (adds `2^31` back!)
            //     - `movdqu SFP2, [tmp]`; `por dst, SFP2` (saturates `>= 2^32` to `u32::MAX`!)
            //   MIND BLOWN!! ZERO extra registers, ZERO branches, 100% BIT-EXACT!
            let f2p31_off = asm.add_local_rodata(16, &splat4_u32(2147483648.0f32.to_bits()));
            asm.xmm_rip_op(None, 0x54, SFP2, RelocTarget::LocalRodata(f2p31_off));
            asm.subps(dst, SFP2);
            asm.cvttps2dq(dst, dst);
            asm.pslld_imm(SFP2, 7);
            asm.pxor(dst, SFP2);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.por(dst, SFP2);
            Ok(())
        }
        0xfdfc => {
            // i32x4.trunc_sat_f64x2_s_zero
            asm.movaps_rr(dst, a);
            asm.movaps_rr(SFP2, dst);
            asm.cmppd(SFP2, SFP2, 7); // ord(a, a)
            asm.pshufd(SFP2, SFP2, 0x08);
            asm.movdqu_mx(ctx_v128_tmp(), SFP2);
            load_local_v128(asm, SFP2, &splat2_u64((-2147483648.0f64).to_bits()));
            asm.maxpd(dst, SFP2);
            load_local_v128(asm, SFP2, &splat2_u64(2147483647.0f64.to_bits()));
            asm.minpd(dst, SFP2);
            asm.cvttpd2dq(dst, dst);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.pand(dst, SFP2);
            Ok(())
        }
        0xfdfd => {
            // i32x4.trunc_sat_f64x2_u_zero
            asm.movaps_rr(dst, a);
            asm.pxor(SFP2, SFP2);
            asm.maxpd(dst, SFP2);
            load_local_v128(asm, SFP2, &splat2_u64(4294967295.0f64.to_bits()));
            asm.minpd(dst, SFP2);
            asm.roundpd(dst, dst, 0x0b); // trunc toward zero
            load_local_v128(asm, SFP2, &splat2_u64(0x4330_0000_0000_0000)); // 2^52
            asm.addpd(dst, SFP2);
            asm.pshufd(dst, dst, 0x08);
            asm.movq_rx(SGP1, dst);
            asm.movq_xr(dst, SGP1); // zeroes upper 64 bits!
            Ok(())
        }
        _ => emit_v128_un(asm, op, dst, a, ro),
    }
}

/// Normalize `dst = a`, returning `rhs = b` without clobbering `b` if `dst == b`.
/// Precondition: `dst != SFP2` and `a != SFP2`.
#[inline]
fn prep_bin(asm: &mut Asm, dst: Xmm, a: Xmm, b: Xmm) -> Xmm {
    if dst == b && dst != a {
        asm.movaps_rr(SFP2, b);
        asm.movaps_rr(dst, a);
        SFP2
    } else {
        asm.movaps_rr(dst, a);
        b
    }
}

/// Swap normalization: `dst = b`, returning `rhs = a` without clobbering `a` or `b`
/// even when `dst == a` AND `b == SFP2` (both `a`/`dst` and `b` spilled, or `dst == a` reg with spilled `b`).
/// Precondition: `dst != SFP2` and `a != SFP2`.
#[inline]
fn prep_bin_swap(asm: &mut Asm, dst: Xmm, a: Xmm, b: Xmm) -> Xmm {
    if dst == a && dst != b {
        if b == SFP2 {
            // `a` lives in `dst` and `b` lives in `SFP2`: swap via `ctx_v128_tmp()` so
            // saving `a` does not overwrite `b` in `SFP2`.
            asm.movdqu_mx(ctx_v128_tmp(), a);
            asm.movaps_rr(dst, b);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
        } else {
            asm.movaps_rr(SFP2, a);
            asm.movaps_rr(dst, b);
        }
        SFP2
    } else {
        asm.movaps_rr(dst, b);
        a
    }
}

/// Emits all `(V128, V128) -> V128` binary ops.
/// `dst` may alias `a`, `b`, or `SFP1`, but NEVER equals `SFP2`.
pub fn emit_v128_bin(asm: &mut Asm, op: u16, dst: Xmm, a: Xmm, b: Xmm) -> Result<(), String> {
    let prep = |asm: &mut Asm| -> Xmm { prep_bin(asm, dst, a, b) };
    let prep_swap = |asm: &mut Asm| -> Xmm { prep_bin_swap(asm, dst, a, b) };

    match op {
        0xfd0e => {
            // i8x16.swizzle: paddusb(b, 0x70) maps 0..15 -> 0x70..0x7f, >=16 -> >=0x80 (zero in pshufb)
            let rhs = prep(asm);
            asm.movaps_rr(SFP2, rhs);
            let off = asm.add_local_rodata(16, &splat16_u8(0x70));
            // paddusb SFP2, [0x70]: load via ctx_v128_tmp or save dst
            asm.movdqu_mx(ctx_v128_tmp(), dst);
            asm.movdqu_rip(dst, RelocTarget::LocalRodata(off));
            asm.paddusb(SFP2, dst);
            asm.movdqu_xm(dst, ctx_v128_tmp());
            asm.pshufb(dst, SFP2);
        }
        // ---- Integer comparisons ----
        0xfd23 => {
            let r = prep(asm);
            asm.pcmpeqb(dst, r);
        }
        0xfd24 => {
            let r = prep(asm);
            asm.pcmpeqb(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd25 => {
            let r = prep_swap(asm);
            asm.pcmpgtb(dst, r);
        }
        _ => return emit_v128_bin_rest(asm, op, dst, a, b),
    }
    Ok(())
}

fn emit_v128_bin_rest(asm: &mut Asm, op: u16, dst: Xmm, a: Xmm, b: Xmm) -> Result<(), String> {
    let prep = |asm: &mut Asm| -> Xmm { prep_bin(asm, dst, a, b) };
    let prep_swap = |asm: &mut Asm| -> Xmm { prep_bin_swap(asm, dst, a, b) };

    // Helper for unsigned comparisons using `pminu`/`pmaxu` + `pcmpeq` (never aliases `dst` with `a`!).
    // `a <=_u b`  <=>  `min_u(a, b) == a`
    // `a >=_u b`  <=>  `max_u(a, b) == a`
    // `a <_u b`   <=>  `~(max_u(a, b) == a)`
    // `a >_u b`   <=>  `~(min_u(a, b) == a)`
    // Put `b` into `dst` and `a` into `SFP2` (safe even when `dst == a` and `b == SFP2`),
    // compute `dst = min/max(b, a)` into `dst`, then compare `dst == SFP2`.
    let emit_ucmp = |asm: &mut Asm, use_max: bool, invert: bool, width: u8| {
        let a_reg = prep_swap(asm);
        if a_reg != SFP2 {
            asm.movaps_rr(SFP2, a_reg);
        }
        match (use_max, width) {
            (false, 1) => asm.pminub(dst, SFP2),
            (true, 1) => asm.pmaxub(dst, SFP2),
            (false, 2) => asm.pminuw(dst, SFP2),
            (true, 2) => asm.pmaxuw(dst, SFP2),
            (false, 4) => asm.pminud(dst, SFP2),
            (true, 4) => asm.pmaxud(dst, SFP2),
            _ => unreachable!(),
        }
        match width {
            1 => asm.pcmpeqb(dst, SFP2),
            2 => asm.pcmpeqw(dst, SFP2),
            4 => asm.pcmpeqd(dst, SFP2),
            _ => unreachable!(),
        }
        if invert {
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
    };

    match op {
        // i8x16 cmp
        0xfd27 => {
            let r = prep(asm);
            asm.pcmpgtb(dst, r);
        }
        0xfd28 => emit_ucmp(asm, false, true, 1), // i8x16.gt_u
        0xfd29 => {
            let r = prep(asm);
            asm.pcmpgtb(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd2a => emit_ucmp(asm, false, false, 1), // i8x16.le_u
        0xfd2b => {
            let r = prep_swap(asm);
            asm.pcmpgtb(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd2c => emit_ucmp(asm, true, false, 1), // i8x16.ge_u

        // i16x8 cmp
        0xfd2d => {
            let r = prep(asm);
            asm.pcmpeqw(dst, r);
        }
        0xfd2e => {
            let r = prep(asm);
            asm.pcmpeqw(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd2f => {
            let r = prep_swap(asm);
            asm.pcmpgtw(dst, r);
        }
        0xfd30 => emit_ucmp(asm, true, true, 2), // i16x8.lt_u
        0xfd31 => {
            let r = prep(asm);
            asm.pcmpgtw(dst, r);
        }
        0xfd32 => emit_ucmp(asm, false, true, 2), // i16x8.gt_u
        0xfd33 => {
            let r = prep(asm);
            asm.pcmpgtw(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd34 => emit_ucmp(asm, false, false, 2), // i16x8.le_u
        0xfd35 => {
            let r = prep_swap(asm);
            asm.pcmpgtw(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd36 => emit_ucmp(asm, true, false, 2), // i16x8.ge_u

        // i32x4 cmp
        0xfd37 => {
            let r = prep(asm);
            asm.pcmpeqd(dst, r);
        }
        0xfd38 => {
            let r = prep(asm);
            asm.pcmpeqd(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd39 => {
            let r = prep_swap(asm);
            asm.pcmpgtd(dst, r);
        }
        0xfd3a => emit_ucmp(asm, true, true, 4), // i32x4.lt_u
        0xfd3b => {
            let r = prep(asm);
            asm.pcmpgtd(dst, r);
        }
        0xfd3c => emit_ucmp(asm, false, true, 4), // i32x4.gt_u
        0xfd3d => {
            let r = prep(asm);
            asm.pcmpgtd(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd3e => emit_ucmp(asm, false, false, 4), // i32x4.le_u
        0xfd3f => {
            let r = prep_swap(asm);
            asm.pcmpgtd(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfd40 => emit_ucmp(asm, true, false, 4), // i32x4.ge_u

        // f32x4 cmp
        0xfd41 => {
            let r = prep(asm);
            asm.cmpps(dst, r, 0);
        } // eq
        0xfd42 => {
            let r = prep(asm);
            asm.cmpps(dst, r, 4);
        } // ne
        0xfd43 => {
            let r = prep(asm);
            asm.cmpps(dst, r, 1);
        } // lt
        0xfd44 => {
            let r = prep_swap(asm);
            asm.cmpps(dst, r, 1);
        } // gt
        0xfd45 => {
            let r = prep(asm);
            asm.cmpps(dst, r, 2);
        } // le
        0xfd46 => {
            let r = prep_swap(asm);
            asm.cmpps(dst, r, 2);
        } // ge

        // f64x2 cmp
        0xfd47 => {
            let r = prep(asm);
            asm.cmppd(dst, r, 0);
        } // eq
        0xfd48 => {
            let r = prep(asm);
            asm.cmppd(dst, r, 4);
        } // ne
        0xfd49 => {
            let r = prep(asm);
            asm.cmppd(dst, r, 1);
        } // lt
        0xfd4a => {
            let r = prep_swap(asm);
            asm.cmppd(dst, r, 1);
        } // gt
        0xfd4b => {
            let r = prep(asm);
            asm.cmppd(dst, r, 2);
        } // le
        0xfd4c => {
            let r = prep_swap(asm);
            asm.cmppd(dst, r, 2);
        } // ge

        // bitwise
        0xfd4e => {
            let r = prep(asm);
            asm.pand(dst, r);
        }
        0xfd4f => {
            let r = prep_swap(asm);
            asm.pandn(dst, r);
        } // !b & a
        0xfd50 => {
            let r = prep(asm);
            asm.por(dst, r);
        }
        0xfd51 => {
            let r = prep(asm);
            asm.pxor(dst, r);
        }

        // narrow
        0xfd65 => {
            let r = prep(asm);
            asm.packsswb(dst, r);
        }
        0xfd66 => {
            let r = prep(asm);
            asm.packuswb(dst, r);
        }
        0xfd85 => {
            let r = prep(asm);
            asm.packssdw(dst, r);
        }
        0xfd86 => {
            let r = prep(asm);
            asm.packusdw(dst, r);
        }

        // i8x16 arithmetic
        0xfd6e => {
            let r = prep(asm);
            asm.paddb(dst, r);
        }
        0xfd6f => {
            let r = prep(asm);
            asm.paddsb(dst, r);
        }
        0xfd70 => {
            let r = prep(asm);
            asm.paddusb(dst, r);
        }
        0xfd71 => {
            let r = prep(asm);
            asm.psubb(dst, r);
        }
        0xfd72 => {
            let r = prep(asm);
            asm.psubsb(dst, r);
        }
        0xfd73 => {
            let r = prep(asm);
            asm.psubusb(dst, r);
        }
        0xfd76 => {
            let r = prep(asm);
            asm.pminsb(dst, r);
        }
        0xfd77 => {
            let r = prep(asm);
            asm.pminub(dst, r);
        }
        0xfd78 => {
            let r = prep(asm);
            asm.pmaxsb(dst, r);
        }
        0xfd79 => {
            let r = prep(asm);
            asm.pmaxub(dst, r);
        }
        0xfd7b => {
            let r = prep(asm);
            asm.pavgb(dst, r);
        }

        // i16x8 arithmetic & q15mulr & extmul
        0xfd82 => {
            // i16x8.q15mulr_sat_s: pmulhrsw(a, b); flip 0x8000 -> 0x7fff
            let r = prep(asm);
            asm.pmulhrsw(dst, r);
            load_local_v128(asm, SFP2, &splat8_u16(0x8000));
            asm.pcmpeqw(SFP2, dst);
            asm.pxor(dst, SFP2);
        }
        0xfd8e => {
            let r = prep(asm);
            asm.paddw(dst, r);
        }
        0xfd8f => {
            let r = prep(asm);
            asm.paddsw(dst, r);
        }
        0xfd90 => {
            let r = prep(asm);
            asm.paddusw(dst, r);
        }
        0xfd91 => {
            let r = prep(asm);
            asm.psubw(dst, r);
        }
        0xfd92 => {
            let r = prep(asm);
            asm.psubsw(dst, r);
        }
        0xfd93 => {
            let r = prep(asm);
            asm.psubusw(dst, r);
        }
        0xfd95 => {
            let r = prep(asm);
            asm.pmullw(dst, r);
        }
        0xfd96 => {
            let r = prep(asm);
            asm.pminsw(dst, r);
        }
        0xfd97 => {
            let r = prep(asm);
            asm.pminuw(dst, r);
        }
        0xfd98 => {
            let r = prep(asm);
            asm.pmaxsw(dst, r);
        }
        0xfd99 => {
            let r = prep(asm);
            asm.pmaxuw(dst, r);
        }
        0xfd9b => {
            let r = prep(asm);
            asm.pavgw(dst, r);
        }
        0xfd9c => {
            // i16x8.extmul_low_i8x16_s (read `r` into `SFP2` before widening `dst` in case `r == dst`)
            let r = prep(asm);
            asm.pmovsxbw(SFP2, r);
            asm.pmovsxbw(dst, dst);
            asm.pmullw(dst, SFP2);
        }
        0xfd9d => {
            // i16x8.extmul_high_i8x16_s
            let r = prep(asm);
            asm.movaps_rr(SFP2, r);
            asm.psrldq_imm(SFP2, 8);
            asm.pmovsxbw(SFP2, SFP2);
            asm.psrldq_imm(dst, 8);
            asm.pmovsxbw(dst, dst);
            asm.pmullw(dst, SFP2);
        }
        0xfd9e => {
            // i16x8.extmul_low_i8x16_u
            let r = prep(asm);
            asm.pmovzxbw(SFP2, r);
            asm.pmovzxbw(dst, dst);
            asm.pmullw(dst, SFP2);
        }
        0xfd9f => {
            // i16x8.extmul_high_i8x16_u
            let r = prep(asm);
            asm.movaps_rr(SFP2, r);
            asm.psrldq_imm(SFP2, 8);
            asm.pmovzxbw(SFP2, SFP2);
            asm.psrldq_imm(dst, 8);
            asm.pmovzxbw(dst, dst);
            asm.pmullw(dst, SFP2);
        }

        // i32x4 arithmetic & dot & extmul
        0xfdae => {
            let r = prep(asm);
            asm.paddd(dst, r);
        }
        0xfdb1 => {
            let r = prep(asm);
            asm.psubd(dst, r);
        }
        0xfdb5 => {
            let r = prep(asm);
            asm.pmulld(dst, r);
        }
        0xfdb6 => {
            let r = prep(asm);
            asm.pminsd(dst, r);
        }
        0xfdb7 => {
            let r = prep(asm);
            asm.pminud(dst, r);
        }
        0xfdb8 => {
            let r = prep(asm);
            asm.pmaxsd(dst, r);
        }
        0xfdb9 => {
            let r = prep(asm);
            asm.pmaxud(dst, r);
        }
        0xfdba => {
            let r = prep(asm);
            asm.pmaddwd(dst, r);
        } // i32x4.dot_i16x8_s
        0xfdbc => {
            let r = prep(asm);
            asm.pmovsxwd(SFP2, r);
            asm.pmovsxwd(dst, dst);
            asm.pmulld(dst, SFP2);
        }
        0xfdbd => {
            let r = prep(asm);
            asm.movaps_rr(SFP2, r);
            asm.psrldq_imm(SFP2, 8);
            asm.pmovsxwd(SFP2, SFP2);
            asm.psrldq_imm(dst, 8);
            asm.pmovsxwd(dst, dst);
            asm.pmulld(dst, SFP2);
        }
        0xfdbe => {
            let r = prep(asm);
            asm.pmovzxwd(SFP2, r);
            asm.pmovzxwd(dst, dst);
            asm.pmulld(dst, SFP2);
        }
        0xfdbf => {
            let r = prep(asm);
            asm.movaps_rr(SFP2, r);
            asm.psrldq_imm(SFP2, 8);
            asm.pmovzxwd(SFP2, SFP2);
            asm.psrldq_imm(dst, 8);
            asm.pmovzxwd(dst, dst);
            asm.pmulld(dst, SFP2);
        }

        // i64x2 arithmetic, cmp, extmul
        0xfdce => {
            let r = prep(asm);
            asm.paddq(dst, r);
        }
        0xfdd1 => {
            let r = prep(asm);
            asm.psubq(dst, r);
        }
        0xfdd6 => {
            let r = prep(asm);
            asm.pcmpeqq(dst, r);
        }
        0xfdd7 => {
            let r = prep(asm);
            asm.pcmpeqq(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfdd8 => {
            let r = prep_swap(asm);
            asm.pcmpgtq(dst, r);
        }
        0xfdd9 => {
            let r = prep(asm);
            asm.pcmpgtq(dst, r);
        }
        0xfdda => {
            let r = prep(asm);
            asm.pcmpgtq(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfddb => {
            let r = prep_swap(asm);
            asm.pcmpgtq(dst, r);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
        }
        0xfddc => {
            // i64x2.extmul_low_i32x4_s
            let r = prep(asm);
            asm.pshufd(SFP2, r, 0x50);
            asm.pshufd(dst, dst, 0x50);
            asm.pmuldq(dst, SFP2);
        }
        0xfddd => {
            // i64x2.extmul_high_i32x4_s
            let r = prep(asm);
            asm.pshufd(SFP2, r, 0xfa);
            asm.pshufd(dst, dst, 0xfa);
            asm.pmuldq(dst, SFP2);
        }
        0xfdde => {
            // i64x2.extmul_low_i32x4_u
            let r = prep(asm);
            asm.pshufd(SFP2, r, 0x50);
            asm.pshufd(dst, dst, 0x50);
            asm.pmuludq(dst, SFP2);
        }
        0xfddf => {
            // i64x2.extmul_high_i32x4_u
            let r = prep(asm);
            asm.pshufd(SFP2, r, 0xfa);
            asm.pshufd(dst, dst, 0xfa);
            asm.pmuludq(dst, SFP2);
        }

        // f32x4 arithmetic
        0xfde4 => {
            let r = prep(asm);
            asm.addps(dst, r);
        }
        0xfde5 => {
            let r = prep(asm);
            asm.subps(dst, r);
        }
        0xfde6 => {
            let r = prep(asm);
            asm.mulps(dst, r);
        }
        0xfde7 => {
            let r = prep(asm);
            asm.divps(dst, r);
        }
        0xfdea => {
            let r = prep_swap(asm);
            asm.minps(dst, r);
        } // f32x4.pmin(a,b) = b < a ? b : a
        0xfdeb => {
            let r = prep_swap(asm);
            asm.maxps(dst, r);
        } // f32x4.pmax(a,b) = a < b ? b : a

        // f64x2 arithmetic
        0xfdf0 => {
            let r = prep(asm);
            asm.addpd(dst, r);
        }
        0xfdf1 => {
            let r = prep(asm);
            asm.subpd(dst, r);
        }
        0xfdf2 => {
            let r = prep(asm);
            asm.mulpd(dst, r);
        }
        0xfdf3 => {
            let r = prep(asm);
            asm.divpd(dst, r);
        }
        0xfdf6 => {
            let r = prep_swap(asm);
            asm.minpd(dst, r);
        } // f64x2.pmin
        0xfdf7 => {
            let r = prep_swap(asm);
            asm.maxpd(dst, r);
        } // f64x2.pmax
        V128_OP_PSADBW => {
            let r = prep(asm);
            asm.psadbw(dst, r);
        }

        0xfde8 | 0xfde9 | 0xfdf4 | 0xfdf5 => unreachable!(),
        _ => return Err(format!("x64: unknown V128Bin opcode {op:#x}")),
    }
    Ok(())
}

/// Attempts to emit a 3-address VEX 128-bit binary op with a memory RHS operand (`dst = a op [mem]`).
/// Returns `true` if `op` has a direct unaligned-safe VEX memory-operand form.
pub fn try_emit_v128_bin_mem_vex(asm: &mut Asm, op: u16, dst: Xmm, a: Xmm, mem: Mem) -> bool {
    match op {
        0xfd4e => asm.vpand_xxm(dst, a, mem),
        0xfd50 => asm.vpor_xxm(dst, a, mem),
        0xfd51 => asm.vpxor_xxm(dst, a, mem),
        0xfd6e => asm.vpaddb_xxm(dst, a, mem),
        0xfd71 => asm.vpsubb_xxm(dst, a, mem),
        0xfd8e => asm.vpaddw_xxm(dst, a, mem),
        0xfd91 => asm.vpsubw_xxm(dst, a, mem),
        0xfd95 => asm.vpmullw_xxm(dst, a, mem),
        0xfdae => asm.vpaddd_xxm(dst, a, mem),
        0xfdb1 => asm.vpsubd_xxm(dst, a, mem),
        0xfdb5 => asm.vpmulld_xxm(dst, a, mem),
        0xfdce => asm.vpaddq_xxm(dst, a, mem),
        0xfdd1 => asm.vpsubq_xxm(dst, a, mem),
        0xfde4 => asm.vaddps_xxm(dst, a, mem),
        0xfde5 => asm.vsubps_xxm(dst, a, mem),
        0xfde6 => asm.vmulps_xxm(dst, a, mem),
        0xfde7 => asm.vdivps_xxm(dst, a, mem),
        0xfdf0 => asm.vaddpd_xxm(dst, a, mem),
        0xfdf1 => asm.vsubpd_xxm(dst, a, mem),
        0xfdf2 => asm.vmulpd_xxm(dst, a, mem),
        0xfdf3 => asm.vdivpd_xxm(dst, a, mem),
        _ => return false,
    }
    true
}

/// True if `op` is a bit-exact commutative integer/bitwise V128 binary op (`a op b == b op a`).
/// (FP ops are excluded so NaN operand-order bit patterns stay 100% identical to the oracle.)
#[inline]
pub fn is_bitexact_commutative_v128_bin(op: u16) -> bool {
    matches!(
        op,
        0xfd4e | 0xfd50 | 0xfd51 | 0xfd6e | 0xfd8e | 0xfd95 | 0xfdae | 0xfdb5 | 0xfdce
    )
}

/// 3-address VEX 128-bit lowering for all binary SIMD ops (zero `movaps` copies, alias-safe for
/// any `dst`/`a`/`b` with `dst != SFP2` and `a != SFP2`).
fn emit_v128_bin_vex(asm: &mut Asm, op: u16, dst: Xmm, a: Xmm, b: Xmm) -> Result<bool, String> {
    // Unsigned compare: `SFP2 = min/max(a, b)` (safe even when `b == SFP2` or `dst == a`),
    // then `dst = (SFP2 == a)`, then optional invert.
    let emit_ucmp = |asm: &mut Asm, use_max: bool, invert: bool, width: u8| {
        match (use_max, width) {
            (false, 1) => asm.vpminub_xxx(SFP2, a, b),
            (true, 1) => asm.vpmaxub_xxx(SFP2, a, b),
            (false, 2) => asm.vpminuw_xxx(SFP2, a, b),
            (true, 2) => asm.vpmaxuw_xxx(SFP2, a, b),
            (false, 4) => asm.vpminud_xxx(SFP2, a, b),
            (true, 4) => asm.vpmaxud_xxx(SFP2, a, b),
            _ => unreachable!(),
        }
        match width {
            1 => asm.vpcmpeqb_xxx(dst, SFP2, a),
            2 => asm.vpcmpeqw_xxx(dst, SFP2, a),
            4 => asm.vpcmpeqd_xxx(dst, SFP2, a),
            _ => unreachable!(),
        }
        if invert {
            asm.vpcmpeqd_xxx(SFP2, SFP2, SFP2);
            asm.vpxor_xxx(dst, dst, SFP2);
        }
    };
    let invert_dst = |asm: &mut Asm| {
        asm.vpcmpeqd_xxx(SFP2, SFP2, SFP2);
        asm.vpxor_xxx(dst, dst, SFP2);
    };

    match op {
        0xfd0e => {
            // i8x16.swizzle: 2 VEX instructions, zero memory scratch!
            let off = asm.add_local_rodata(16, &splat16_u8(0x70));
            asm.vpaddusb_xx_rip(SFP2, b, RelocTarget::LocalRodata(off));
            asm.vpshufb_xxx(dst, a, SFP2);
        }
        // i8x16 cmp
        0xfd23 => asm.vpcmpeqb_xxx(dst, a, b),
        0xfd24 => {
            asm.vpcmpeqb_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd25 => asm.vpcmpgtb_xxx(dst, b, a),
        0xfd26 => emit_ucmp(asm, true, true, 1),
        0xfd27 => asm.vpcmpgtb_xxx(dst, a, b),
        0xfd28 => emit_ucmp(asm, false, true, 1),
        0xfd29 => {
            asm.vpcmpgtb_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd2a => emit_ucmp(asm, false, false, 1),
        0xfd2b => {
            asm.vpcmpgtb_xxx(dst, b, a);
            invert_dst(asm);
        }
        0xfd2c => emit_ucmp(asm, true, false, 1),
        // i16x8 cmp
        0xfd2d => asm.vpcmpeqw_xxx(dst, a, b),
        0xfd2e => {
            asm.vpcmpeqw_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd2f => asm.vpcmpgtw_xxx(dst, b, a),
        0xfd30 => emit_ucmp(asm, true, true, 2),
        0xfd31 => asm.vpcmpgtw_xxx(dst, a, b),
        0xfd32 => emit_ucmp(asm, false, true, 2),
        0xfd33 => {
            asm.vpcmpgtw_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd34 => emit_ucmp(asm, false, false, 2),
        0xfd35 => {
            asm.vpcmpgtw_xxx(dst, b, a);
            invert_dst(asm);
        }
        0xfd36 => emit_ucmp(asm, true, false, 2),
        // i32x4 cmp
        0xfd37 => asm.vpcmpeqd_xxx(dst, a, b),
        0xfd38 => {
            asm.vpcmpeqd_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd39 => asm.vpcmpgtd_xxx(dst, b, a),
        0xfd3a => emit_ucmp(asm, true, true, 4),
        0xfd3b => asm.vpcmpgtd_xxx(dst, a, b),
        0xfd3c => emit_ucmp(asm, false, true, 4),
        0xfd3d => {
            asm.vpcmpgtd_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfd3e => emit_ucmp(asm, false, false, 4),
        0xfd3f => {
            asm.vpcmpgtd_xxx(dst, b, a);
            invert_dst(asm);
        }
        0xfd40 => emit_ucmp(asm, true, false, 4),
        // f32x4 cmp
        0xfd41 => asm.vcmpps_xxxi(dst, a, b, 0),
        0xfd42 => asm.vcmpps_xxxi(dst, a, b, 4),
        0xfd43 => asm.vcmpps_xxxi(dst, a, b, 1),
        0xfd44 => asm.vcmpps_xxxi(dst, b, a, 1),
        0xfd45 => asm.vcmpps_xxxi(dst, a, b, 2),
        0xfd46 => asm.vcmpps_xxxi(dst, b, a, 2),
        // f64x2 cmp
        0xfd47 => asm.vcmppd_xxxi(dst, a, b, 0),
        0xfd48 => asm.vcmppd_xxxi(dst, a, b, 4),
        0xfd49 => asm.vcmppd_xxxi(dst, a, b, 1),
        0xfd4a => asm.vcmppd_xxxi(dst, b, a, 1),
        0xfd4b => asm.vcmppd_xxxi(dst, a, b, 2),
        0xfd4c => asm.vcmppd_xxxi(dst, b, a, 2),
        // bitwise
        0xfd4e => asm.vpand_xxx(dst, a, b),
        0xfd4f => asm.vpandn_xxx(dst, b, a), // !b & a
        0xfd50 => asm.vpor_xxx(dst, a, b),
        0xfd51 => asm.vpxor_xxx(dst, a, b),
        // narrow
        0xfd65 => asm.vpacksswb_xxx(dst, a, b),
        0xfd66 => asm.vpackuswb_xxx(dst, a, b),
        0xfd85 => asm.vpackssdw_xxx(dst, a, b),
        0xfd86 => asm.vpackusdw_xxx(dst, a, b),
        // i8x16 arith
        0xfd6e => asm.vpaddb_xxx(dst, a, b),
        0xfd6f => asm.vpaddsb_xxx(dst, a, b),
        0xfd70 => asm.vpaddusb_xxx(dst, a, b),
        0xfd71 => asm.vpsubb_xxx(dst, a, b),
        0xfd72 => asm.vpsubsb_xxx(dst, a, b),
        0xfd73 => asm.vpsubusb_xxx(dst, a, b),
        0xfd76 => asm.vpminsb_xxx(dst, a, b),
        0xfd77 => asm.vpminub_xxx(dst, a, b),
        0xfd78 => asm.vpmaxsb_xxx(dst, a, b),
        0xfd79 => asm.vpmaxub_xxx(dst, a, b),
        0xfd7b => asm.vpavgb_xxx(dst, a, b),
        // i16x8 arith & extmul
        0xfd82 => {
            asm.vpmulhrsw_xxx(dst, a, b);
            load_local_v128(asm, SFP2, &splat8_u16(0x8000));
            asm.vpcmpeqw_xxx(SFP2, SFP2, dst);
            asm.vpxor_xxx(dst, dst, SFP2);
        }
        0xfd8e => asm.vpaddw_xxx(dst, a, b),
        0xfd8f => asm.vpaddsw_xxx(dst, a, b),
        0xfd90 => asm.vpaddusw_xxx(dst, a, b),
        0xfd91 => asm.vpsubw_xxx(dst, a, b),
        0xfd92 => asm.vpsubsw_xxx(dst, a, b),
        0xfd93 => asm.vpsubusw_xxx(dst, a, b),
        0xfd95 => asm.vpmullw_xxx(dst, a, b),
        0xfd96 => asm.vpminsw_xxx(dst, a, b),
        0xfd97 => asm.vpminuw_xxx(dst, a, b),
        0xfd98 => asm.vpmaxsw_xxx(dst, a, b),
        0xfd99 => asm.vpmaxuw_xxx(dst, a, b),
        0xfd9b => asm.vpavgw_xxx(dst, a, b),
        0xfd9c => {
            asm.vpmovsxbw_xx(SFP2, b);
            asm.vpmovsxbw_xx(dst, a);
            asm.vpmullw_xxx(dst, dst, SFP2);
        }
        0xfd9d => {
            asm.vpsrldq_xxi(SFP2, b, 8);
            asm.vpmovsxbw_xx(SFP2, SFP2);
            asm.vpsrldq_xxi(dst, a, 8);
            asm.vpmovsxbw_xx(dst, dst);
            asm.vpmullw_xxx(dst, dst, SFP2);
        }
        0xfd9e => {
            asm.vpmovzxbw_xx(SFP2, b);
            asm.vpmovzxbw_xx(dst, a);
            asm.vpmullw_xxx(dst, dst, SFP2);
        }
        0xfd9f => {
            asm.vpsrldq_xxi(SFP2, b, 8);
            asm.vpmovzxbw_xx(SFP2, SFP2);
            asm.vpsrldq_xxi(dst, a, 8);
            asm.vpmovzxbw_xx(dst, dst);
            asm.vpmullw_xxx(dst, dst, SFP2);
        }
        // i32x4 arith & dot & extmul
        0xfdae => asm.vpaddd_xxx(dst, a, b),
        0xfdb1 => asm.vpsubd_xxx(dst, a, b),
        0xfdb5 => asm.vpmulld_xxx(dst, a, b),
        0xfdb6 => asm.vpminsd_xxx(dst, a, b),
        0xfdb7 => asm.vpminud_xxx(dst, a, b),
        0xfdb8 => asm.vpmaxsd_xxx(dst, a, b),
        0xfdb9 => asm.vpmaxud_xxx(dst, a, b),
        0xfdba => asm.vpmaddwd_xxx(dst, a, b),
        0xfdbc => {
            asm.vpmovsxwd_xx(SFP2, b);
            asm.vpmovsxwd_xx(dst, a);
            asm.vpmulld_xxx(dst, dst, SFP2);
        }
        0xfdbd => {
            asm.vpsrldq_xxi(SFP2, b, 8);
            asm.vpmovsxwd_xx(SFP2, SFP2);
            asm.vpsrldq_xxi(dst, a, 8);
            asm.vpmovsxwd_xx(dst, dst);
            asm.vpmulld_xxx(dst, dst, SFP2);
        }
        0xfdbe => {
            asm.vpmovzxwd_xx(SFP2, b);
            asm.vpmovzxwd_xx(dst, a);
            asm.vpmulld_xxx(dst, dst, SFP2);
        }
        0xfdbf => {
            asm.vpsrldq_xxi(SFP2, b, 8);
            asm.vpmovzxwd_xx(SFP2, SFP2);
            asm.vpsrldq_xxi(dst, a, 8);
            asm.vpmovzxwd_xx(dst, dst);
            asm.vpmulld_xxx(dst, dst, SFP2);
        }
        // i64x2 arith, cmp, extmul
        0xfdce => asm.vpaddq_xxx(dst, a, b),
        0xfdd1 => asm.vpsubq_xxx(dst, a, b),
        0xfdd6 => asm.vpcmpeqq_xxx(dst, a, b),
        0xfdd7 => {
            asm.vpcmpeqq_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfdd8 => asm.vpcmpgtq_xxx(dst, b, a),
        0xfdd9 => asm.vpcmpgtq_xxx(dst, a, b),
        0xfdda => {
            asm.vpcmpgtq_xxx(dst, a, b);
            invert_dst(asm);
        }
        0xfddb => {
            asm.vpcmpgtq_xxx(dst, b, a);
            invert_dst(asm);
        }
        0xfddc => {
            asm.vpshufd(SFP2, b, 0x50);
            asm.vpshufd(dst, a, 0x50);
            asm.vpmuldq_xxx(dst, dst, SFP2);
        }
        0xfddd => {
            asm.vpshufd(SFP2, b, 0xfa);
            asm.vpshufd(dst, a, 0xfa);
            asm.vpmuldq_xxx(dst, dst, SFP2);
        }
        0xfdde => {
            asm.vpshufd(SFP2, b, 0x50);
            asm.vpshufd(dst, a, 0x50);
            asm.vpmuludq_xxx(dst, dst, SFP2);
        }
        0xfddf => {
            asm.vpshufd(SFP2, b, 0xfa);
            asm.vpshufd(dst, a, 0xfa);
            asm.vpmuludq_xxx(dst, dst, SFP2);
        }
        // f32x4 arith
        0xfde4 => asm.vaddps_xxx(dst, a, b),
        0xfde5 => asm.vsubps_xxx(dst, a, b),
        0xfde6 => asm.vmulps_xxx(dst, a, b),
        0xfde7 => asm.vdivps_xxx(dst, a, b),
        0xfdea => asm.vminps_xxx(dst, b, a), // f32x4.pmin(a, b) == vminps(b, a)
        0xfdeb => asm.vmaxps_xxx(dst, b, a), // f32x4.pmax(a, b) == vmaxps(b, a)
        // f64x2 arith
        0xfdf0 => asm.vaddpd_xxx(dst, a, b),
        0xfdf1 => asm.vsubpd_xxx(dst, a, b),
        0xfdf2 => asm.vmulpd_xxx(dst, a, b),
        0xfdf3 => asm.vdivpd_xxx(dst, a, b),
        0xfdf6 => asm.vminpd_xxx(dst, b, a),
        0xfdf7 => asm.vmaxpd_xxx(dst, b, a),
        V128_OP_PSADBW => asm.vpsadbw_xxx(dst, a, b),
        _ => return Ok(false),
    }
    Ok(true)
}

/// Complete binary entry point handling `i8x16.lt_u`, `i64x2.mul`, and `f32x4`/`f64x2` `min`/`max` cleanly.
pub fn emit_v128_bin_full(
    asm: &mut Asm,
    op: u16,
    dst: Xmm,
    a: Xmm,
    b: Xmm,
    isa: TargetIsa,
) -> Result<(), String> {
    if isa.has_avx2 && emit_v128_bin_vex(asm, op, dst, a, b)? {
        return Ok(());
    }
    match op {
        0xfd26 => {
            // i8x16.lt_u: ~(max_u(a, b) == a)
            let a_reg = prep_bin_swap(asm, dst, a, b);
            if a_reg != SFP2 {
                asm.movaps_rr(SFP2, a_reg);
            }
            asm.pmaxub(dst, SFP2);
            asm.pcmpeqb(dst, SFP2);
            asm.pcmpeqd(SFP2, SFP2);
            asm.pxor(dst, SFP2);
            Ok(())
        }
        0xfdd5 => {
            // i64x2.mul: uses only GP scratch (`SGP1`/`SGP2`) + `dst` + `SFP2`!
            // 2 scalar 64-bit multiplies (4 cycles latency, ZERO aliasing issues!).
            // Must save `b` into `SFP2` whenever `dst == b` (including `dst == a == b`),
            // because lane 0's `movq_xr(dst, SGP1)` zeroes lane 1 of `dst` before lane 1 reads `b_reg`.
            let b_reg = if dst == b {
                asm.movaps_rr(SFP2, b);
                SFP2
            } else {
                b
            };
            let a_reg = if dst == a {
                asm.movdqu_mx(ctx_v128_tmp(), a);
                None
            } else {
                Some(a)
            };
            // Lane 0
            if let Some(ar) = a_reg {
                asm.movq_rx(SGP1, ar);
            } else {
                asm.mov_rm(true, SGP1, ctx_v128_tmp());
            }
            asm.movq_rx(SGP2, b_reg);
            asm.imul_rr(true, SGP1, SGP2);
            asm.movq_xr(dst, SGP1);
            // Lane 1
            if let Some(ar) = a_reg {
                asm.pextrq(SGP1, ar, 1);
            } else {
                asm.mov_rm(true, SGP1, Mem::base_disp(Gp::R15, OFF_SCRATCH_V128 + 8));
            }
            asm.pextrq(SGP2, b_reg, 1);
            asm.imul_rr(true, SGP1, SGP2);
            asm.pinsrq(dst, SGP1, 1);
            Ok(())
        }
        0xfde8 | 0xfde9 | 0xfdf4 | 0xfdf5 => {
            // Wasm IEEE-754 min/max (NaN -> canonical qNaN, -0.0 < +0.0)
            let is_f64 = matches!(op, 0xfdf4 | 0xfdf5);
            let is_max = matches!(op, 0xfde9 | 0xfdf5);
            // First put `a` in `dst` and `b` in `SFP2` (or keep `b` in `ctx_v128_tmp` too).
            let b_reg = if dst == b && dst != a {
                asm.movaps_rr(SFP2, b);
                asm.movaps_rr(dst, a);
                SFP2
            } else {
                asm.movaps_rr(dst, a);
                b
            };
            asm.movdqu_mx(ctx_v128_tmp(), b_reg);
            // Step 1: SFP2 = min/max(b, a)
            asm.movaps_rr(SFP2, b_reg);
            if is_f64 {
                if is_max {
                    asm.maxpd(SFP2, dst);
                } else {
                    asm.minpd(SFP2, dst);
                }
            } else if is_max {
                asm.maxps(SFP2, dst);
            } else {
                asm.minps(SFP2, dst);
            }
            // Wait: we still need BOTH `a` and `b` for `unord = cmpunord(a, b)` AND `min/max(a, b)`!
            // Check: `dst` still holds `a`, `tmp` holds `b`, `SFP2` holds `op(b, a)`.
            // Compute `op(a, b)` in `dst`? If we overwrite `dst` with `op(a, b)`, we lose `a` for `cmpunord(a, b)`!
            // WAIT: Check `CMPUNORD(op(a, b), op(b, a))`!!
            // - Suppose `a` is NaN and `b` is finite:
            //   `op(a, b) = b` (finite), `op(b, a) = a` (NaN) -> `CMPUNORD(op(a,b), op(b,a))` IS ALL 1s!!
            // - Suppose `b` is NaN and `a` is finite:
            //   `op(a, b) = b` (NaN), `op(b, a) = a` (finite) -> `CMPUNORD(op(a,b), op(b,a))` IS ALL 1s!!
            // - Suppose BOTH are NaN:
            //   both are NaN -> `CMPUNORD` IS ALL 1s!!
            // - Suppose NEITHER is NaN:
            //   both are finite -> `CMPUNORD` IS ALL 0s!!
            // WHOA!! `CMPUNORD(op(a, b), op(b, a))` IS MATHEMATICALLY IDENTICAL TO `CMPUNORD(a, b)`!!
            // Now trace with ONLY `dst`, `SFP2`, and `ctx_v128_tmp`:
            //   1. `dst = a`, `SFP2 = op(b, a)`, `tmp = b`
            //   2. Swap `SFP2` into `tmp` while computing `dst = op(a, b)`:
            //      Wait: load `b` from `tmp` would overwrite `SFP2`!
            //      Instead, save `a` in `tmp`? No, compute `op(a, b)` using `b_reg` FIRST if `b_reg != SFP2`:
            //      Actually even simpler:
            //      Step A: `dst = op(a, b)` (using `b` in `SFP2` if needed -> save `b` in `tmp` FIRST!)
            //      Wait: we want `t1 = op(a, b)` and `t2 = op(b, a)` in `dst` and `SFP2`:
            //      - `tmp` = `b` (done above)
            //      - `SFP2` = `op(b, a)` (done above!)
            //      - Now we need `dst = op(a, tmp)`! We can add `minps_xm`/`maxps_xm`/`minpd_xm`/`maxpd_xm` OR just use `sse_xm`:
            //        Wait — `minps`/`maxps`/`minpd`/`maxpd` with memory operand! Or even simpler:
            //        We already have `minss_xm` etc. in `asm.rs`; let's check if we have `minps_xm`? Not yet — let's do `SFP1`? No, `dst` might be `SFP1`!
            //        WAIT: Look at `dst`: we only need `b` in memory if `dst == b`!
            //        Even cleaner: save `a` in `[R15 + OFF_SCRATCH_V128]`, compute `op(a, b)` in `dst` BEFORE `op(b, a)`? No, still needs `b`!
            //        Wait: `minps_xm` / `maxps_xm` / `minpd_xm` / `maxpd_xm` is 4 one-line methods on `Asm`, OR we can encode `sse_xm` directly? `sse_xm` is private to `asm.rs` — let's expose `minps_xm`, `maxps_xm`, `minpd_xm`, `maxpd_xm` (or `sse_xm`) in `asm.rs`! Wait: let's check what we already have in `asm.rs`!
            //        Wait: we ALREADY have `movq_rx` / `pextrq`? No, adding 4 one-liner `pub fn minps_xm/maxps_xm/minpd_xm/maxpd_xm` to `asm.rs` is cleaner AND faster (1 instruction instead of 3)!
            //        Let's trace the complete 9-instruction sequence with `minps_xm`/`maxps_xm`/`minpd_xm`/`maxpd_xm`:
            //          1. `b` -> `[tmp]`
            //          2. `SFP2 = b`; `op(SFP2, dst(=a))`  => `SFP2 = op(b, a)`
            //          3. `op_xm(dst(=a), [tmp])`          => `dst = op(a, b)`
            //          4. `movdqu [tmp], dst`              => `tmp = op(a, b)`
            //          5. `cmpunord(dst, SFP2)`            => `dst = unord`
            //          6. If max: `andps_xm(SFP2, [tmp])` else `orps_xm(SFP2, [tmp])` => `SFP2 = res_if_ord`
            //          7. `movdqu [tmp], SFP2`             => `tmp = res_if_ord`
            //          8. `load qnan into SFP2; pand SFP2, dst` => `SFP2 = unord & qnan`
            //          9. `pandn dst, [tmp]` -> wait: `pandn_xm` or `andnps_xm`! `andnps_xm(dst, [tmp])` => `dst = ~unord & res_if_ord`
            //         10. `por dst, SFP2`                  => `dst = final`!
            //        EVERY SINGLE STEP IS 100% ALIAS-FREE EVEN WHEN `dst == SFP1` OR `dst == a` OR `dst == b`!
            if is_f64 {
                if is_max {
                    asm.maxpd_xm(dst, ctx_v128_tmp());
                } else {
                    asm.minpd_xm(dst, ctx_v128_tmp());
                }
                asm.movdqu_mx(ctx_v128_tmp(), dst);
                asm.cmppd(dst, SFP2, 3); // dst = unord(op(a,b), op(b,a))
                if is_max {
                    asm.andpd_xm(SFP2, ctx_v128_tmp());
                } else {
                    asm.orpd_xm(SFP2, ctx_v128_tmp());
                }
                asm.movdqu_mx(ctx_v128_tmp(), SFP2);
                load_local_v128(asm, SFP2, &splat2_u64(0x7ff8_0000_0000_0000));
                asm.pand(SFP2, dst);
                asm.andnpd_xm(dst, ctx_v128_tmp());
                asm.por(dst, SFP2);
            } else {
                if is_max {
                    asm.maxps_xm(dst, ctx_v128_tmp());
                } else {
                    asm.minps_xm(dst, ctx_v128_tmp());
                }
                asm.movdqu_mx(ctx_v128_tmp(), dst);
                asm.cmpps(dst, SFP2, 3); // dst = unord(op(a,b), op(b,a))
                if is_max {
                    asm.andps_xm(SFP2, ctx_v128_tmp());
                } else {
                    asm.orps_xm(SFP2, ctx_v128_tmp());
                }
                asm.movdqu_mx(ctx_v128_tmp(), SFP2);
                load_local_v128(asm, SFP2, &splat4_u32(0x7fc0_0000));
                asm.pand(SFP2, dst);
                asm.andnps_xm(dst, ctx_v128_tmp());
                asm.por(dst, SFP2);
            }
            Ok(())
        }
        _ => emit_v128_bin(asm, op, dst, a, b),
    }
}

/// Emits constant-immediate SIMD lane shifts (`i16x8`/`i32x4`/`i64x2`) in 1 instruction
/// (`vpsll*/vpsrl*/vpsra* xmm, xmm, imm8` or SSE2 `psll*/psrl*/psra*`), masking `shift` per Wasm.
pub fn emit_v128_shift_imm(asm: &mut Asm, op: u16, dst: Xmm, v: Xmm, shift: u32, isa: TargetIsa) {
    let mask = match op {
        0xfd8b..=0xfd8d => 15,
        0xfdab..=0xfdad => 31,
        0xfdcb | 0xfdcd => 63,
        _ => unreachable!("unsupported immediate V128Shift op {op:#x}"),
    };
    let cnt = (shift & mask) as u8;
    if cnt == 0 {
        if dst != v {
            asm.movaps_rr(dst, v);
        }
        return;
    }
    if isa.has_avx2 {
        match op {
            0xfd8b => asm.vpsllw_xxi(dst, v, cnt),
            0xfd8c => asm.vpsraw_xxi(dst, v, cnt),
            0xfd8d => asm.vpsrlw_xxi(dst, v, cnt),
            0xfdab => asm.vpslld_xxi(dst, v, cnt),
            0xfdac => asm.vpsrad_xxi(dst, v, cnt),
            0xfdad => asm.vpsrld_xxi(dst, v, cnt),
            0xfdcb => asm.vpsllq_xxi(dst, v, cnt),
            0xfdcd => asm.vpsrlq_xxi(dst, v, cnt),
            _ => unreachable!(),
        }
        return;
    }
    if dst != v {
        asm.movaps_rr(dst, v);
    }
    match op {
        0xfd8b => asm.psllw_imm(dst, cnt),
        0xfd8c => asm.psraw_imm(dst, cnt),
        0xfd8d => asm.psrlw_imm(dst, cnt),
        0xfdab => asm.pslld_imm(dst, cnt),
        0xfdac => asm.psrad_imm(dst, cnt),
        0xfdad => asm.psrld_imm(dst, cnt),
        0xfdcb => asm.psllq_imm(dst, cnt),
        0xfdcd => asm.psrlq_imm(dst, cnt),
        _ => unreachable!(),
    }
}

/// Emits all `(V128, I32) -> V128` SIMD shift ops with ZERO GP clobbers.
pub fn emit_v128_shift(
    asm: &mut Asm,
    op: u16,
    dst: Xmm,
    v: Xmm,
    shift: Gp,
    isa: TargetIsa,
) -> Result<(), String> {
    if isa.has_avx2 {
        match op {
            0xfd6b | 0xfd6d => {
                let mut tbl = [0u8; 128];
                for k in 0..8usize {
                    let m = if op == 0xfd6b {
                        0xffu8.wrapping_shl(k as u32)
                    } else {
                        0xffu8 >> (k as u32)
                    };
                    tbl[k * 16..(k + 1) * 16].fill(m);
                }
                let tbl_off = asm.add_local_rodata(16, &tbl);
                asm.mov_rr(false, SGP2, shift);
                asm.and_ri(false, SGP2, 7);
                asm.vmovd_xr(SFP2, SGP2);
                if op == 0xfd6b {
                    asm.vpsllw_xxx(dst, v, SFP2);
                } else {
                    asm.vpsrlw_xxx(dst, v, SFP2);
                }
                asm.shl_imm(true, SGP2, 4);
                asm.lea_rip(SGP1, RelocTarget::LocalRodata(tbl_off));
                asm.vpand_xxm(dst, dst, Mem::indexed(SGP1, SGP2, 1, 0));
                return Ok(());
            }
            0xfd8b..=0xfd8d => {
                asm.mov_rr(false, SGP2, shift);
                asm.and_ri(false, SGP2, 15);
                asm.vmovd_xr(SFP2, SGP2);
                match op {
                    0xfd8b => asm.vpsllw_xxx(dst, v, SFP2),
                    0xfd8c => asm.vpsraw_xxx(dst, v, SFP2),
                    _ => asm.vpsrlw_xxx(dst, v, SFP2),
                }
                return Ok(());
            }
            0xfdab..=0xfdad => {
                asm.mov_rr(false, SGP2, shift);
                asm.and_ri(false, SGP2, 31);
                asm.vmovd_xr(SFP2, SGP2);
                match op {
                    0xfdab => asm.vpslld_xxx(dst, v, SFP2),
                    0xfdac => asm.vpsrad_xxx(dst, v, SFP2),
                    _ => asm.vpsrld_xxx(dst, v, SFP2),
                }
                return Ok(());
            }
            0xfdcb | 0xfdcd => {
                asm.mov_rr(false, SGP2, shift);
                asm.and_ri(false, SGP2, 63);
                asm.vmovd_xr(SFP2, SGP2);
                if op == 0xfdcb {
                    asm.vpsllq_xxx(dst, v, SFP2);
                } else {
                    asm.vpsrlq_xxx(dst, v, SFP2);
                }
                return Ok(());
            }
            0xfdcc => {
                asm.vpshufd(SFP2, v, 0xf5);
                asm.vpsrad_xxi(SFP2, SFP2, 31);
                asm.vmovdqu_mx(ctx_v128_tmp(), SFP2);
                asm.vpxor_xxx(dst, v, SFP2);
                asm.mov_rr(false, SGP2, shift);
                asm.and_ri(false, SGP2, 63);
                asm.vmovd_xr(SFP2, SGP2);
                asm.vpsrlq_xxx(dst, dst, SFP2);
                asm.vpxor_xxm(dst, dst, ctx_v128_tmp());
                return Ok(());
            }
            _ => {}
        }
    }
    asm.movaps_rr(dst, v);
    match op {
        0xfd6b | 0xfd6d => {
            // i8x16.shl / i8x16.shr_u: psllw/psrlw by (shift & 7), then mask with 8x16 table
            let mut tbl = [0u8; 128];
            for k in 0..8usize {
                let m = if op == 0xfd6b {
                    0xffu8.wrapping_shl(k as u32)
                } else {
                    0xffu8 >> (k as u32)
                };
                tbl[k * 16..(k + 1) * 16].fill(m);
            }
            let tbl_off = asm.add_local_rodata(16, &tbl);
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 7);
            asm.movd_xr(SFP2, SGP2);
            if op == 0xfd6b {
                asm.psllw(dst, SFP2);
            } else {
                asm.psrlw(dst, SFP2);
            }
            asm.shl_imm(true, SGP2, 4);
            asm.lea_rip(SGP1, RelocTarget::LocalRodata(tbl_off));
            asm.movdqu_xm(SFP2, Mem::indexed(SGP1, SGP2, 1, 0));
            asm.pand(dst, SFP2);
        }
        0xfd6c => {
            // i8x16.shr_s:
            //   odd bytes  = psraw(v, cnt) & 0xff00
            //   even bytes = psraw(psllw(v, 8), cnt + 8) & 0x00ff
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 7);
            // Save original `v` in ctx_v128_tmp
            asm.movdqu_mx(ctx_v128_tmp(), dst);
            // Odd bytes into dst:
            asm.movd_xr(SFP2, SGP2);
            asm.psraw(dst, SFP2);
            load_local_v128(asm, SFP2, &splat8_u16(0xff00));
            asm.pand(dst, SFP2);
            // Even bytes: swap `odd` into ctx_v128_tmp, load `v` into SFP2
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.movdqu_mx(ctx_v128_tmp(), dst); // tmp = odd
            asm.movaps_rr(dst, SFP2); // dst = v
            asm.psllw_imm(dst, 8);
            asm.add_ri(false, SGP2, 8);
            asm.movd_xr(SFP2, SGP2);
            asm.psraw(dst, SFP2);
            load_local_v128(asm, SFP2, &splat8_u16(0x00ff));
            asm.pand(dst, SFP2);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.por(dst, SFP2);
        }
        0xfd8b..=0xfd8d => {
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 15);
            asm.movd_xr(SFP2, SGP2);
            match op {
                0xfd8b => asm.psllw(dst, SFP2),
                0xfd8c => asm.psraw(dst, SFP2),
                _ => asm.psrlw(dst, SFP2),
            }
        }
        0xfdab..=0xfdad => {
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 31);
            asm.movd_xr(SFP2, SGP2);
            match op {
                0xfdab => asm.pslld(dst, SFP2),
                0xfdac => asm.psrad(dst, SFP2),
                _ => asm.psrld(dst, SFP2),
            }
        }
        0xfdcb | 0xfdcd => {
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 63);
            asm.movd_xr(SFP2, SGP2);
            if op == 0xfdcb {
                asm.psllq(dst, SFP2);
            } else {
                asm.psrlq(dst, SFP2);
            }
        }
        0xfdcc => {
            // i64x2.shr_s: pure SSE2, zero GP clobbers!
            // sign = psrad(pshufd(v, 0xf5), 31); ((v ^ sign) >>_u (shift & 63)) ^ sign
            asm.pshufd(SFP2, dst, 0xf5);
            asm.psrad_imm(SFP2, 31);
            asm.movdqu_mx(ctx_v128_tmp(), SFP2);
            asm.pxor(dst, SFP2);
            asm.mov_rr(false, SGP2, shift);
            asm.and_ri(false, SGP2, 63);
            asm.movd_xr(SFP2, SGP2);
            asm.psrlq(dst, SFP2);
            asm.movdqu_xm(SFP2, ctx_v128_tmp());
            asm.pxor(dst, SFP2);
        }
        _ => return Err(format!("x64: unknown V128Shift opcode {op:#x}")),
    }
    Ok(())
}
