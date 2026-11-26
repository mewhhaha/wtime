//! Fixed-width SIMD instruction emission, including outlining stack effects.

use super::{CEmitter, Res};
use wtime_types::ValType;
use wtime_validate::{Immediate, Instruction};

pub(super) const fn is_memory(op: u16) -> bool {
    matches!(op, 0xfd00..=0xfd0b | 0xfd54..=0xfd5d)
}

pub(super) fn stack_effect(op: u16) -> Option<(usize, usize)> {
    Some(match op {
        0xfd0c => (0, 1),
        0xfd0b | 0xfd58..=0xfd5b => (2, 0),
        0xfd52 => (3, 1),
        0xfd0d..=0xfd0e
        | 0xfd17
        | 0xfd1a
        | 0xfd1c
        | 0xfd1e
        | 0xfd20
        | 0xfd22
        | 0xfd23..=0xfd4c
        | 0xfd4e..=0xfd51
        | 0xfd54..=0xfd57
        | 0xfd65..=0xfd66
        | 0xfd6b..=0xfd73
        | 0xfd76..=0xfd79
        | 0xfd7b
        | 0xfd82
        | 0xfd85..=0xfd86
        | 0xfd8b..=0xfd93
        | 0xfd95..=0xfd99
        | 0xfd9b..=0xfd9f
        | 0xfdab..=0xfdae
        | 0xfdb1
        | 0xfdb5..=0xfdba
        | 0xfdbc..=0xfdbf
        | 0xfdcb..=0xfdce
        | 0xfdd1
        | 0xfdd5..=0xfddf
        | 0xfde4..=0xfdeb
        | 0xfdf0..=0xfdf7 => (2, 1),
        0xfd00..=0xfd0a
        | 0xfd0f..=0xfd16
        | 0xfd18..=0xfd19
        | 0xfd1b
        | 0xfd1d
        | 0xfd1f
        | 0xfd21
        | 0xfd4d
        | 0xfd53
        | 0xfd5c..=0xfd64
        | 0xfd67..=0xfd6a
        | 0xfd74..=0xfd75
        | 0xfd7a
        | 0xfd7c..=0xfd81
        | 0xfd83..=0xfd84
        | 0xfd87..=0xfd8a
        | 0xfd94
        | 0xfda0..=0xfda1
        | 0xfda3..=0xfda4
        | 0xfda7..=0xfdaa
        | 0xfdc0..=0xfdc1
        | 0xfdc3..=0xfdc4
        | 0xfdc7..=0xfdca
        | 0xfde0..=0xfde1
        | 0xfde3
        | 0xfdec..=0xfded
        | 0xfdef
        | 0xfdf8..=0xfdff => (1, 1),
        _ => return None,
    })
}

pub(super) fn emit(e: &mut CEmitter, ml: &str, insn: &Instruction) -> Res<()> {
    use ValType::{F32, F64, I32, I64, V128};
    let op = insn.opcode;
    match op {
        0xfd00..=0xfd0b | 0xfd54..=0xfd5d => emit_memory(e, ml, insn),
        0xfd0c => {
            let Immediate::Bytes(ref bytes) = insn.immediate else {
                return Err("v128.const requires bytes".into());
            };
            let bytes: [u8; 16] = bytes.as_ref().try_into().map_err(|_| "v128.const")?;
            let lo = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let hi = u64::from_le_bytes(bytes[8..].try_into().unwrap());
            e.push_plain(V128, format!("(((wt_v128){hi}ULL << 64) | {lo}ULL)"));
            Ok(())
        }
        0xfd0d => {
            let Immediate::Bytes(ref lanes) = insn.immediate else {
                return Err("i8x16.shuffle requires lane indices".into());
            };
            let (b, _) = e.pop()?;
            let (a, _) = e.pop()?;
            let lanes = lanes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            e.push(
                V128,
                &format!(
                    "wt_from_u8(__builtin_shufflevector(wt_as_u8({a}), wt_as_u8({b}), {lanes}))"
                ),
            )
        }
        0xfd0f..=0xfd14 => {
            let (a, ty) = e.pop()?;
            let bits = [8, 16, 32, 64, 32, 64][(op - 0xfd0f) as usize];
            e.push(
                V128,
                &format!("wt_simd_splat({bits}, {})", scalar_bits(&a, ty)),
            )
        }
        0xfd15..=0xfd22 => {
            let Immediate::Index(lane) = insn.immediate else {
                return Err("SIMD lane instruction requires a lane index".into());
            };
            let (bits, ty) = match op {
                0xfd15..=0xfd17 => (8, I32),
                0xfd18..=0xfd1a => (16, I32),
                0xfd1b..=0xfd1c => (32, I32),
                0xfd1d..=0xfd1e => (64, I64),
                0xfd1f..=0xfd20 => (32, F32),
                _ => (64, F64),
            };
            if matches!(op, 0xfd17 | 0xfd1a | 0xfd1c | 0xfd1e | 0xfd20 | 0xfd22) {
                let (x, _) = e.pop()?;
                let (v, _) = e.pop()?;
                e.push(
                    V128,
                    &format!(
                        "wt_simd_replace({v}, {bits}, {lane}, {})",
                        scalar_bits(&x, ty)
                    ),
                )
            } else {
                let (v, _) = e.pop()?;
                let x = format!("(({v}) >> {})", bits * lane);
                let expr = match op {
                    0xfd15 => format!("(int32_t)(int8_t)({x})"),
                    0xfd16 => format!("(int32_t)(uint8_t)({x})"),
                    0xfd18 => format!("(int32_t)(int16_t)({x})"),
                    0xfd19 => format!("(int32_t)(uint16_t)({x})"),
                    0xfd1b => format!("(int32_t)(uint32_t)({x})"),
                    0xfd1d => format!("(int64_t)(uint64_t)({x})"),
                    0xfd1f => format!("wt_f32_bits((uint32_t)({x}))"),
                    _ => format!("wt_f64_bits((uint64_t)({x}))"),
                };
                e.push(ty, &expr)
            }
        }
        _ => {
            let (pops, _) =
                stack_effect(op).ok_or_else(|| format!("unsupported instruction {op:#x}"))?;
            let mut args = Vec::with_capacity(pops);
            for _ in 0..pops {
                args.push(e.pop()?.0);
            }
            args.reverse();
            let ty = match op {
                0xfd53 | 0xfd63..=0xfd64 | 0xfd83..=0xfd84 | 0xfda3..=0xfda4 | 0xfdc3..=0xfdc4 => {
                    I32
                }
                _ => V128,
            };
            e.push(
                ty,
                &format!("wt_simd_{:02x}({})", op & 255, args.join(", ")),
            )
        }
    }
}

fn scalar_bits(x: &str, ty: ValType) -> String {
    match ty {
        ValType::F32 => format!("wt_f32_to_bits({x})"),
        ValType::F64 => format!("wt_f64_to_bits({x})"),
        _ => format!("(uint64_t)({x})"),
    }
}

fn emit_memory(e: &mut CEmitter, ml: &str, insn: &Instruction) -> Res<()> {
    let op = insn.opcode;
    let (offset, lane) = match insn.immediate {
        Immediate::MemArg {
            memory: 0, offset, ..
        } => (offset, None),
        Immediate::MemLane {
            memory: 0,
            offset,
            lane,
            ..
        } => (offset, Some(lane)),
        _ => return Err("SIMD access on another memory or missing memory immediate".into()),
    };
    let vector = if matches!(op, 0xfd0b | 0xfd54..=0xfd5b) {
        Some(e.pop()?.0)
    } else {
        None
    };
    let (addr, _) = e.pop()?;
    let trap = e.trap_oob_goto();
    if offset > u32::MAX as u64 {
        e.line(trap);
        e.set_unreachable();
        return Ok(());
    }

    let bytes = match op {
        0xfd00 | 0xfd0b => 16,
        0xfd01..=0xfd06 | 0xfd5d => 8,
        0xfd5c => 4,
        0xfd07..=0xfd0a => 1 << (op - 0xfd07),
        _ => 1 << ((op - 0xfd54) % 4),
    };
    let (unaligned_ty, raw_ty, raw) = match bytes {
        1 => ("uint8_t", "uint8_t", "tm8"),
        2 => ("wt_u16u", "uint16_t", "tm16"),
        4 => ("wt_u32u", "uint32_t", "tm32"),
        8 => ("wt_u64u", "uint64_t", "tm64"),
        _ => ("wt_u128u", "wt_v128", "tm128"),
    };
    let args = format!("{bytes}ULL, {unaligned_ty}, mb, {ml}, (uint32_t)({addr}), {offset}u");
    if matches!(op, 0xfd0b | 0xfd58..=0xfd5b) {
        let v = vector.unwrap();
        let value = if let Some(lane) = lane {
            format!("({raw_ty})(({v}) >> {})", 8 * bytes * usize::from(lane))
        } else {
            v
        };
        e.line(&format!("WT_S({args}, {value});"));
        return Ok(());
    }
    e.declare(raw, raw_ty);
    e.line(&format!("WT_L({args}, {raw});"));
    let result = match op {
        0xfd01..=0xfd06 => {
            let extend = ["87", "89", "a7", "a9", "c7", "c9"][(op - 0xfd01) as usize];
            format!("wt_simd_{extend}((wt_v128){raw})")
        }
        0xfd07..=0xfd0a => format!("wt_simd_splat({}, {raw})", 8 * bytes),
        0xfd54..=0xfd57 => format!(
            "wt_simd_replace({}, {}, {}, {raw})",
            vector.unwrap(),
            8 * bytes,
            lane.unwrap()
        ),
        _ => format!("(wt_v128){raw}"),
    };
    e.push(ValType::V128, &result)
}
