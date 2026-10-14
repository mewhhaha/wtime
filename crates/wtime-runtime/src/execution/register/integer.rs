//! Integer slot semantics. i32 results are always zero-extended bit patterns.
use crate::{Result, trap};
pub(super) fn supported(op: u16) -> bool {
    matches!(op, 0x45..=0x5a | 0x67..=0x8a | 0xa7 | 0xac | 0xad | 0xc0..=0xc4)
}
pub(super) fn unary(op: u16, a: u64) -> Result<u64> {
    Ok(match op {
        0x45 => u64::from(a as u32 == 0),
        0x50 => u64::from(a == 0),
        0x67 => (a as u32).leading_zeros() as u64,
        0x68 => (a as u32).trailing_zeros() as u64,
        0x69 => (a as u32).count_ones() as u64,
        0x79 => a.leading_zeros() as u64,
        0x7a => a.trailing_zeros() as u64,
        0x7b => a.count_ones() as u64,
        0xa7 | 0xad => a as u32 as u64,
        0xac => a as u32 as i32 as i64 as u64,
        0xc0 => a as u8 as i8 as i32 as u32 as u64,
        0xc1 => a as u16 as i16 as i32 as u32 as u64,
        0xc2 => a as u8 as i8 as i64 as u64,
        0xc3 => a as u16 as i16 as i64 as u64,
        0xc4 => a as u32 as i32 as i64 as u64,
        _ => return Err(trap("unverified register unary opcode")),
    })
}
pub(super) fn binary(op: u16, a: u64, b: u64) -> Result<u64> {
    macro_rules! compare {
        ($base:expr, $u:ty, $s:ty) => {{
            let (a, b) = (a as $u, b as $u);
            u64::from(match op - $base {
                0 => a == b,
                1 => a != b,
                2 => (a as $s) < (b as $s),
                3 => a < b,
                4 => (a as $s) > (b as $s),
                5 => a > b,
                6 => (a as $s) <= (b as $s),
                7 => a <= b,
                8 => (a as $s) >= (b as $s),
                9 => a >= b,
                _ => return Err(trap("unverified register comparison opcode")),
            })
        }};
    }
    macro_rules! arithmetic {
        ($base:expr, $u:ty, $s:ty) => {{
            let (a, b) = (a as $u, b as $u);
            let offset = op - $base;
            if (3..=6).contains(&offset) && b == 0 {
                return Err(trap("integer divide by zero"));
            }
            let value: $u = match offset {
                0 => a.wrapping_add(b),
                1 => a.wrapping_sub(b),
                2 => a.wrapping_mul(b),
                3 => (a as $s)
                    .checked_div(b as $s)
                    .ok_or_else(|| trap("integer overflow"))? as $u,
                4 => a / b,
                5 => (a as $s).wrapping_rem(b as $s) as $u,
                6 => a % b,
                7 => a & b,
                8 => a | b,
                9 => a ^ b,
                10 => a.wrapping_shl(b as u32),
                11 => (a as $s).wrapping_shr(b as u32) as $u,
                12 => a.wrapping_shr(b as u32),
                13 => a.rotate_left(b as u32),
                14 => a.rotate_right(b as u32),
                _ => return Err(trap("unverified register arithmetic opcode")),
            };
            value as u64
        }};
    }
    Ok(match op {
        0x46..=0x4f => compare!(0x46, u32, i32),
        0x51..=0x5a => compare!(0x51, u64, i64),
        0x6a..=0x78 => arithmetic!(0x6a, u32, i32),
        0x7c..=0x8a => arithmetic!(0x7c, u64, i64),
        _ => return Err(trap("unverified register binary opcode")),
    })
}
