//! Strict scalar float semantics on raw numeric slots. No fast-math policy.
use crate::{Result, trap};

pub(super) fn supported(op: u16) -> bool {
    matches!(op, 0x5b..=0x66 | 0x8b..=0xa6 | 0xa8..=0xab | 0xae..=0xbf | 0xfc00..=0xfc07)
}

pub(super) fn unary(op: u16, bits: u64) -> Result<u64> {
    macro_rules! arithmetic {
        ($f:ty, $u:ty, $base:expr, $quiet:expr) => {{
            let a = <$f>::from_bits(bits as $u);
            let n = op - $base;
            let value = match n {
                0 => a.abs(),
                1 => -a,
                2 => a.ceil(),
                3 => a.floor(),
                4 => a.trunc(),
                5 => a.round_ties_even(),
                6 => a.sqrt(),
                _ => return Err(trap("unverified register float unary opcode")),
            };
            let result = value.to_bits();
            // Arithmetic unary operations quiet signaling NaNs. Abs/neg retain
            // the source payload and only change their specified sign bit.
            (if n >= 2 && value.is_nan() {
                result | $quiet
            } else {
                result
            }) as u64
        }};
    }
    Ok(match op {
        0x8b..=0x91 => arithmetic!(f32, u32, 0x8b, 0x0040_0000),
        0x99..=0x9f => arithmetic!(f64, u64, 0x99, 0x0008_0000_0000_0000),
        0xa8..=0xab | 0xae..=0xb1 | 0xfc00..=0xfc07 => {
            let saturating = op >= 0xfc00;
            let n = if saturating {
                op - 0xfc00
            } else if op <= 0xab {
                op - 0xa8
            } else {
                op - 0xae + 4
            };
            let a = if n % 4 < 2 {
                f32::from_bits(bits as u32) as f64
            } else {
                f64::from_bits(bits)
            };
            convert(a, n >= 4, n % 2 == 1, saturating)?
        }
        0xb2 => (bits as u32 as i32 as f32).to_bits() as u64,
        0xb3 => (bits as u32 as f32).to_bits() as u64,
        0xb4 => (bits as i64 as f32).to_bits() as u64,
        0xb5 => (bits as f32).to_bits() as u64,
        0xb6 => (f64::from_bits(bits) as f32).to_bits() as u64,
        0xb7 => (bits as u32 as i32 as f64).to_bits(),
        0xb8 => (bits as u32 as f64).to_bits(),
        0xb9 => (bits as i64 as f64).to_bits(),
        0xba => (bits as f64).to_bits(),
        0xbb => (f32::from_bits(bits as u32) as f64).to_bits(),
        0xbc | 0xbe => bits as u32 as u64,
        0xbd | 0xbf => bits,
        _ => return Err(trap("unverified register float unary opcode")),
    })
}

fn convert(a: f64, wide: bool, unsigned: bool, saturating: bool) -> Result<u64> {
    let (min, max) = match (wide, unsigned) {
        (false, false) => (-2147483648., 2147483648.),
        (false, true) => (0., 4294967296.),
        (true, false) => (-9223372036854775808., 9223372036854775808.),
        (true, true) => (0., 18446744073709551616.),
    };
    let a = a.trunc();
    if !saturating && a.is_nan() {
        return Err(trap("invalid integer conversion"));
    }
    if !saturating && (a < min || a >= max) {
        return Err(trap("integer overflow"));
    }
    // Rust's saturating casts implement the Wasm NaN->0 and endpoint clamps.
    Ok(match (wide, unsigned) {
        (false, false) => a as i32 as u32 as u64,
        (false, true) => a as u32 as u64,
        (true, false) => a as i64 as u64,
        (true, true) => a as u64,
    })
}

fn compare(a: f64, b: f64, n: u16) -> u64 {
    u64::from(match n {
        0 => a == b,
        1 => a != b,
        2 => a < b,
        3 => a > b,
        4 => a <= b,
        _ => a >= b,
    })
}

pub(super) fn binary(op: u16, left: u64, right: u64) -> Result<u64> {
    macro_rules! arithmetic {
        ($f:ty, $u:ty, $base:expr) => {{
            let (a, b) = (<$f>::from_bits(left as $u), <$f>::from_bits(right as $u));
            match op - $base {
                0 => a + b,
                1 => a - b,
                2 => a * b,
                3 => a / b,
                4 => {
                    if a.is_nan() || b.is_nan() {
                        <$f>::NAN
                    } else if a == b {
                        <$f>::from_bits(a.to_bits() | b.to_bits())
                    } else {
                        a.min(b)
                    }
                }
                5 => {
                    if a.is_nan() || b.is_nan() {
                        <$f>::NAN
                    } else if a == b {
                        <$f>::from_bits(a.to_bits() & b.to_bits())
                    } else {
                        a.max(b)
                    }
                }
                6 => a.copysign(b),
                _ => return Err(trap("unverified register float binary opcode")),
            }
            .to_bits() as u64
        }};
    }
    Ok(match op {
        0x5b..=0x60 => compare(
            f32::from_bits(left as u32) as f64,
            f32::from_bits(right as u32) as f64,
            op - 0x5b,
        ),
        0x61..=0x66 => compare(f64::from_bits(left), f64::from_bits(right), op - 0x61),
        0x92..=0x98 => arithmetic!(f32, u32, 0x92),
        0xa0..=0xa6 => arithmetic!(f64, u64, 0xa0),
        _ => return Err(trap("unverified register float binary opcode")),
    })
}
