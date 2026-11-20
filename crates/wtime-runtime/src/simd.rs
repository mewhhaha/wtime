//! Portable lane-wise SIMD. Memory instructions are handled by Store.
use super::*;
fn vector(s: &mut Vec<Value>) -> Result<[u8; 16]> {
    match pop(s)? {
        Value::V128(x) => Ok(x.0),
        _ => Err(trap("expected v128")),
    }
}
fn lane(a: &[u8; 16], i: usize, w: usize) -> u64 {
    let mut b = [0; 8];
    b[..w].copy_from_slice(&a[i * w..i * w + w]);
    u64::from_le_bytes(b)
}
fn put(a: &mut [u8; 16], i: usize, w: usize, v: u64) {
    a[i * w..i * w + w].copy_from_slice(&v.to_le_bytes()[..w]);
}
fn signed(v: u64, w: usize) -> i64 {
    ((v << (64 - w * 8)) as i64) >> (64 - w * 8)
}
fn map(a: [u8; 16], w: usize, mut fun: impl FnMut(u64) -> u64) -> [u8; 16] {
    let mut out = [0; 16];
    for i in 0..16 / w {
        put(&mut out, i, w, fun(lane(&a, i, w)));
    }
    out
}
fn zip(a: [u8; 16], b: [u8; 16], w: usize, mut fun: impl FnMut(u64, u64) -> u64) -> [u8; 16] {
    let mut out = [0; 16];
    for i in 0..16 / w {
        put(&mut out, i, w, fun(lane(&a, i, w), lane(&b, i, w)));
    }
    out
}
pub(super) fn execute(opcode: u16, immediate: &Immediate, s: &mut Vec<Value>) -> Result<()> {
    if opcode & 0xff00 == 0xfa00 {
        return relaxed(opcode & 255, s);
    }
    let op = opcode & 255;
    let out = match op {
        12 => {
            if let Immediate::Bytes(b) = immediate {
                (&b[..])
                    .try_into()
                    .map_err(|_| trap("invalid v128 constant"))?
            } else {
                return Err(trap("missing v128 constant"));
            }
        }
        13 => {
            let b = vector(s)?;
            let a = vector(s)?;
            let Immediate::Bytes(ix) = immediate else {
                return Err(trap("missing shuffle lanes"));
            };
            if ix.len() != 16 || ix.iter().any(|x| *x >= 32) {
                return Err(trap("invalid shuffle lane"));
            }
            let mut out = [0; 16];
            for i in 0..16 {
                out[i] = if ix[i] < 16 {
                    a[ix[i] as usize]
                } else {
                    b[ix[i] as usize - 16]
                };
            }
            out
        }
        14 => {
            let b = vector(s)?;
            let a = vector(s)?;
            let mut out = [0; 16];
            for i in 0..16 {
                out[i] = a.get(b[i] as usize).copied().unwrap_or(0)
            }
            out
        }
        77 => map(vector(s)?, 1, |a| !a),
        78..=81 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| match op {
                78 => a & b,
                79 => a & !b,
                80 => a | b,
                _ => a ^ b,
            })
        }
        82 => {
            let c = vector(s)?;
            let b = vector(s)?;
            let a = vector(s)?;
            let mut out = [0; 16];
            for i in 0..16 {
                out[i] = (a[i] & c[i]) | (b[i] & !c[i])
            }
            out
        }
        83 => {
            let a = vector(s)?;
            s.push(Value::I32(i32::from(a.iter().any(|x| *x != 0))));
            return Ok(());
        }
        15 => {
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 1, |_| x)
        } // i8x16.splat
        16 => {
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 2, |_| x)
        } // i16x8.splat
        17 => {
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 4, |_| x)
        } // i32x4.splat
        18 => {
            let x = match pop(s)? {
                Value::I64(x) => x as u64,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 8, |_| x)
        } // i64x2.splat
        19 => {
            let x = match pop(s)? {
                Value::F32(x) => x as u64,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 4, |_| x)
        } // f32x4.splat
        20 => {
            let x = match pop(s)? {
                Value::F64(x) => x,
                _ => return Err(trap("invalid splat scalar")),
            };
            map([0; 16], 8, |_| x)
        } // f64x2.splat
        21 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 16 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 1);
            s.push(Value::I32(signed(x, 1) as i32));
            return Ok(());
        } // i8x16.extract_lane_s
        22 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 16 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 1);
            s.push(Value::I32(x as i32));
            return Ok(());
        } // i8x16.extract_lane_u
        23 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 16 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 1, x);
            a
        } // i8x16.replace_lane
        24 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 8 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 2);
            s.push(Value::I32(signed(x, 2) as i32));
            return Ok(());
        } // i16x8.extract_lane_s
        25 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 8 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 2);
            s.push(Value::I32(x as i32));
            return Ok(());
        } // i16x8.extract_lane_u
        26 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 8 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 2, x);
            a
        } // i16x8.replace_lane
        27 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 4 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 4);
            s.push(Value::I32(x as i32));
            return Ok(());
        } // i32x4.extract_lane
        28 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 4 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::I32(x) => x as u64,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 4, x);
            a
        } // i32x4.replace_lane
        29 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 2 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 8);
            s.push(Value::I64(x as i64));
            return Ok(());
        } // i64x2.extract_lane
        30 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 2 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::I64(x) => x as u64,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 8, x);
            a
        } // i64x2.replace_lane
        31 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 4 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 4);
            s.push(Value::F32(x as u32));
            return Ok(());
        } // f32x4.extract_lane
        32 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 4 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::F32(x) => x as u64,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 4, x);
            a
        } // f32x4.replace_lane
        33 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 2 {
                return Err(trap("invalid lane"));
            };
            let a = vector(s)?;
            let x = lane(&a, i, 8);
            s.push(Value::F64(x));
            return Ok(());
        } // f64x2.extract_lane
        34 => {
            let Immediate::Index(i) = immediate else {
                return Err(trap("missing lane"));
            };
            let i = *i as usize;
            if i >= 2 {
                return Err(trap("invalid lane"));
            };
            let x = match pop(s)? {
                Value::F64(x) => x,
                _ => return Err(trap("invalid lane scalar")),
            };
            let mut a = vector(s)?;
            put(&mut a, i, 8, x);
            a
        } // f64x2.replace_lane
        35 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a == b { u64::MAX } else { 0 })
        } // i8x16.eq
        36 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a != b { u64::MAX } else { 0 })
        } // i8x16.ne
        37 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                if signed(a, 1) < signed(b, 1) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i8x16.lt_s
        38 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a < b { u64::MAX } else { 0 })
        } // i8x16.lt_u
        39 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                if signed(a, 1) > signed(b, 1) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i8x16.gt_s
        40 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a > b { u64::MAX } else { 0 })
        } // i8x16.gt_u
        41 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                if signed(a, 1) <= signed(b, 1) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i8x16.le_s
        42 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a <= b { u64::MAX } else { 0 })
        } // i8x16.le_u
        43 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                if signed(a, 1) >= signed(b, 1) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i8x16.ge_s
        44 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| if a >= b { u64::MAX } else { 0 })
        } // i8x16.ge_u
        45 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a == b { u64::MAX } else { 0 })
        } // i16x8.eq
        46 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a != b { u64::MAX } else { 0 })
        } // i16x8.ne
        47 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                if signed(a, 2) < signed(b, 2) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i16x8.lt_s
        48 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a < b { u64::MAX } else { 0 })
        } // i16x8.lt_u
        49 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                if signed(a, 2) > signed(b, 2) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i16x8.gt_s
        50 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a > b { u64::MAX } else { 0 })
        } // i16x8.gt_u
        51 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                if signed(a, 2) <= signed(b, 2) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i16x8.le_s
        52 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a <= b { u64::MAX } else { 0 })
        } // i16x8.le_u
        53 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                if signed(a, 2) >= signed(b, 2) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i16x8.ge_s
        54 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| if a >= b { u64::MAX } else { 0 })
        } // i16x8.ge_u
        55 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a == b { u64::MAX } else { 0 })
        } // i32x4.eq
        56 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a != b { u64::MAX } else { 0 })
        } // i32x4.ne
        57 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if signed(a, 4) < signed(b, 4) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i32x4.lt_s
        58 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a < b { u64::MAX } else { 0 })
        } // i32x4.lt_u
        59 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if signed(a, 4) > signed(b, 4) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i32x4.gt_s
        60 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a > b { u64::MAX } else { 0 })
        } // i32x4.gt_u
        61 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if signed(a, 4) <= signed(b, 4) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i32x4.le_s
        62 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a <= b { u64::MAX } else { 0 })
        } // i32x4.le_u
        63 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if signed(a, 4) >= signed(b, 4) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i32x4.ge_s
        64 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| if a >= b { u64::MAX } else { 0 })
        } // i32x4.ge_u
        65 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) == f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.eq
        66 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) != f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.ne
        67 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) < f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.lt
        68 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) > f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.gt
        69 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) <= f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.le
        70 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                if f32::from_bits(a as u32) >= f32::from_bits(b as u32) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f32x4.ge
        71 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) == f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.eq
        72 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) != f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.ne
        73 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) < f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.lt
        74 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) > f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.gt
        75 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) <= f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.le
        76 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if f64::from_bits(a) >= f64::from_bits(b) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // f64x2.ge
        96 => map(vector(s)?, 1, |a| signed(a, 1).wrapping_abs() as u64), // i8x16.abs
        97 => map(vector(s)?, 1, |a| a.wrapping_neg()),                   // i8x16.neg
        99 => {
            let a = vector(s)?;
            s.push(Value::I32((0..16).all(|i| lane(&a, i, 1) != 0) as i32));
            return Ok(());
        } // i8x16.all_true
        100 => {
            let a = vector(s)?;
            s.push(Value::I32(
                (0..16).fold(0, |v, i| v | (((lane(&a, i, 1) >> 7) as i32) << i)),
            ));
            return Ok(());
        } // i8x16.bitmask
        107 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 8,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 1, |a| a.wrapping_shl(n))
        } // i8x16.shl
        108 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 8,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 1, |a| (signed(a, 1) >> n) as u64)
        } // i8x16.shr_s
        109 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 8,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 1, |a| a >> n)
        } // i8x16.shr_u
        110 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.wrapping_add(b))
        } // i8x16.add
        111 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                (signed(a, 1) as i128 + signed(b, 1) as i128).clamp(-128, 127) as u64
            })
        } // i8x16.add_sat_s
        112 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.saturating_add(b).min(255))
        } // i8x16.add_sat_u
        113 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.wrapping_sub(b))
        } // i8x16.sub
        114 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| {
                (signed(a, 1) as i128 - signed(b, 1) as i128).clamp(-128, 127) as u64
            })
        } // i8x16.sub_sat_s
        115 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.saturating_sub(b).min(255))
        } // i8x16.sub_sat_u
        118 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| signed(a, 1).min(signed(b, 1)) as u64)
        } // i8x16.min_s
        119 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.min(b))
        } // i8x16.min_u
        120 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| signed(a, 1).max(signed(b, 1)) as u64)
        } // i8x16.max_s
        121 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| a.max(b))
        } // i8x16.max_u
        123 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 1, |a, b| (a + b).div_ceil(2))
        } // i8x16.avgr_u
        128 => map(vector(s)?, 2, |a| signed(a, 2).wrapping_abs() as u64), // i16x8.abs
        129 => map(vector(s)?, 2, |a| a.wrapping_neg()),                  // i16x8.neg
        131 => {
            let a = vector(s)?;
            s.push(Value::I32((0..8).all(|i| lane(&a, i, 2) != 0) as i32));
            return Ok(());
        } // i16x8.all_true
        132 => {
            let a = vector(s)?;
            s.push(Value::I32(
                (0..8).fold(0, |v, i| v | (((lane(&a, i, 2) >> 15) as i32) << i)),
            ));
            return Ok(());
        } // i16x8.bitmask
        139 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 16,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 2, |a| a.wrapping_shl(n))
        } // i16x8.shl
        140 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 16,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 2, |a| (signed(a, 2) >> n) as u64)
        } // i16x8.shr_s
        141 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 16,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 2, |a| a >> n)
        } // i16x8.shr_u
        142 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.wrapping_add(b))
        } // i16x8.add
        143 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                (signed(a, 2) as i128 + signed(b, 2) as i128).clamp(-32768, 32767) as u64
            })
        } // i16x8.add_sat_s
        144 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.saturating_add(b).min(65535))
        } // i16x8.add_sat_u
        145 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.wrapping_sub(b))
        } // i16x8.sub
        146 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| {
                (signed(a, 2) as i128 - signed(b, 2) as i128).clamp(-32768, 32767) as u64
            })
        } // i16x8.sub_sat_s
        147 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.saturating_sub(b).min(65535))
        } // i16x8.sub_sat_u
        149 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.wrapping_mul(b))
        } // i16x8.mul
        150 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| signed(a, 2).min(signed(b, 2)) as u64)
        } // i16x8.min_s
        151 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.min(b))
        } // i16x8.min_u
        152 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| signed(a, 2).max(signed(b, 2)) as u64)
        } // i16x8.max_s
        153 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| a.max(b))
        } // i16x8.max_u
        155 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 2, |a, b| (a + b).div_ceil(2))
        } // i16x8.avgr_u
        160 => map(vector(s)?, 4, |a| signed(a, 4).wrapping_abs() as u64), // i32x4.abs
        161 => map(vector(s)?, 4, |a| a.wrapping_neg()),                  // i32x4.neg
        163 => {
            let a = vector(s)?;
            s.push(Value::I32((0..4).all(|i| lane(&a, i, 4) != 0) as i32));
            return Ok(());
        } // i32x4.all_true
        164 => {
            let a = vector(s)?;
            s.push(Value::I32(
                (0..4).fold(0, |v, i| v | (((lane(&a, i, 4) >> 31) as i32) << i)),
            ));
            return Ok(());
        } // i32x4.bitmask
        171 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 32,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 4, |a| a.wrapping_shl(n))
        } // i32x4.shl
        172 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 32,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 4, |a| (signed(a, 4) >> n) as u64)
        } // i32x4.shr_s
        173 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 32,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 4, |a| a >> n)
        } // i32x4.shr_u
        174 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| a.wrapping_add(b))
        } // i32x4.add
        177 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| a.wrapping_sub(b))
        } // i32x4.sub
        181 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| a.wrapping_mul(b))
        } // i32x4.mul
        182 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| signed(a, 4).min(signed(b, 4)) as u64)
        } // i32x4.min_s
        183 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| a.min(b))
        } // i32x4.min_u
        184 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| signed(a, 4).max(signed(b, 4)) as u64)
        } // i32x4.max_s
        185 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| a.max(b))
        } // i32x4.max_u
        192 => map(vector(s)?, 8, |a| signed(a, 8).wrapping_abs() as u64), // i64x2.abs
        193 => map(vector(s)?, 8, |a| a.wrapping_neg()),                  // i64x2.neg
        196 => {
            let a = vector(s)?;
            s.push(Value::I32(
                (0..2).fold(0, |v, i| v | (((lane(&a, i, 8) >> 63) as i32) << i)),
            ));
            return Ok(());
        } // i64x2.bitmask
        203 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 64,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 8, |a| a.wrapping_shl(n))
        } // i64x2.shl
        204 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 64,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 8, |a| (signed(a, 8) >> n) as u64)
        } // i64x2.shr_s
        205 => {
            let n = match pop(s)? {
                Value::I32(n) => (n as u32) % 64,
                _ => return Err(trap("invalid shift")),
            };
            map(vector(s)?, 8, |a| a >> n)
        } // i64x2.shr_u
        206 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| a.wrapping_add(b))
        } // i64x2.add
        209 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| a.wrapping_sub(b))
        } // i64x2.sub
        213 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| a.wrapping_mul(b))
        } // i64x2.mul
        103 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.ceil()).to_bits() as u64
            }
        }), // f32x4.ceil
        104 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.floor()).to_bits() as u64
            }
        }), // f32x4.floor
        105 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.trunc()).to_bits() as u64
            }
        }), // f32x4.trunc
        106 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.round_ties_even()).to_bits() as u64
            }
        }), // f32x4.nearest
        116 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.ceil()).to_bits()
            }
        }), // f64x2.ceil
        117 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.floor()).to_bits()
            }
        }), // f64x2.floor
        122 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.trunc()).to_bits()
            }
        }), // f64x2.trunc
        148 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            if x.is_nan() {
                a | if std::mem::size_of_val(&x) == 4 {
                    0x00400000
                } else {
                    0x0008000000000000
                }
            } else {
                (x.round_ties_even()).to_bits()
            }
        }), // f64x2.nearest
        224 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            (x.abs()).to_bits() as u64
        }), // f32x4.abs
        225 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            (-x).to_bits() as u64
        }), // f32x4.neg
        227 => map(vector(s)?, 4, |a| {
            let x = f32::from_bits(a as u32);
            (x.sqrt()).to_bits() as u64
        }), // f32x4.sqrt
        228 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (x + y).to_bits() as u64
            })
        } // f32x4.add
        229 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (x - y).to_bits() as u64
            })
        } // f32x4.sub
        230 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (x * y).to_bits() as u64
            })
        } // f32x4.mul
        231 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (x / y).to_bits() as u64
            })
        } // f32x4.div
        232 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (if x.is_nan() || y.is_nan() {
                    f32::NAN
                } else if x == y {
                    f32::from_bits(x.to_bits() | y.to_bits())
                } else {
                    x.min(y)
                })
                .to_bits() as u64
            })
        } // f32x4.min
        233 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (if x.is_nan() || y.is_nan() {
                    f32::NAN
                } else if x == y {
                    f32::from_bits(x.to_bits() & y.to_bits())
                } else {
                    x.max(y)
                })
                .to_bits() as u64
            })
        } // f32x4.max
        234 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (if y < x { y } else { x }).to_bits() as u64
            })
        } // f32x4.pmin
        235 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 4, |a, b| {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (if x < y { y } else { x }).to_bits() as u64
            })
        } // f32x4.pmax
        236 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            (x.abs()).to_bits()
        }), // f64x2.abs
        237 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            (-x).to_bits()
        }), // f64x2.neg
        239 => map(vector(s)?, 8, |a| {
            let x = f64::from_bits(a);
            (x.sqrt()).to_bits()
        }), // f64x2.sqrt
        240 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (x + y).to_bits()
            })
        } // f64x2.add
        241 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (x - y).to_bits()
            })
        } // f64x2.sub
        242 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (x * y).to_bits()
            })
        } // f64x2.mul
        243 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (x / y).to_bits()
            })
        } // f64x2.div
        244 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (if x.is_nan() || y.is_nan() {
                    f64::NAN
                } else if x == y {
                    f64::from_bits(x.to_bits() | y.to_bits())
                } else {
                    x.min(y)
                })
                .to_bits()
            })
        } // f64x2.min
        245 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (if x.is_nan() || y.is_nan() {
                    f64::NAN
                } else if x == y {
                    f64::from_bits(x.to_bits() & y.to_bits())
                } else {
                    x.max(y)
                })
                .to_bits()
            })
        } // f64x2.max
        246 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (if y < x { y } else { x }).to_bits()
            })
        } // f64x2.pmin
        247 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                let x = f64::from_bits(a);
                let y = f64::from_bits(b);
                (if x < y { y } else { x }).to_bits()
            })
        } // f64x2.pmax
        214 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| if a == b { u64::MAX } else { 0 })
        } // i64x2.eq
        215 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| if a != b { u64::MAX } else { 0 })
        } // i64x2.ne
        216 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if signed(a, 8) < signed(b, 8) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i64x2.lt_s
        217 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if signed(a, 8) > signed(b, 8) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i64x2.gt_s
        218 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if signed(a, 8) <= signed(b, 8) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i64x2.le_s
        219 => {
            let b = vector(s)?;
            let a = vector(s)?;
            zip(a, b, 8, |a, b| {
                if signed(a, 8) >= signed(b, 8) {
                    u64::MAX
                } else {
                    0
                }
            })
        } // i64x2.ge_s
        195 => {
            let a = vector(s)?;
            s.push(Value::I32((0..2).all(|i| lane(&a, i, 8) != 0) as i32));
            return Ok(());
        } // i64x2.all_true
        98 => map(vector(s)?, 1, |a| a.count_ones() as u64),              // i8x16.popcnt
        _ => return special(op, s),
    };
    s.push(Value::V128(V128(out)));
    Ok(())
}
fn special(op: u16, s: &mut Vec<Value>) -> Result<()> {
    let mut out = [0; 16];
    match op {
        101 | 102 | 133 | 134 => {
            let b = vector(s)?;
            let a = vector(s)?;
            let w = if op < 128 { 1 } else { 2 };
            let unsigned = op.is_multiple_of(2);
            for i in 0..16 / w {
                let src = if i < 8 / w { &a } else { &b };
                let x = signed(lane(src, i % (8 / w), w * 2), w * 2);
                let v = if unsigned {
                    x.clamp(0, (1i64 << (w * 8)) - 1)
                } else {
                    x.clamp(-(1i64 << (w * 8 - 1)), (1i64 << (w * 8 - 1)) - 1)
                };
                put(&mut out, i, w, v as u64)
            }
        }
        135..=138 | 167..=170 | 199..=202 => {
            let a = vector(s)?;
            let base = if op < 160 {
                135
            } else if op < 192 {
                167
            } else {
                199
            };
            let w = if base == 135 {
                2
            } else if base == 167 {
                4
            } else {
                8
            };
            let mode = op - base;
            let start = if mode % 2 == 1 { 16 / w } else { 0 };
            for i in 0..16 / w {
                let x = lane(&a, i + start, w / 2);
                put(
                    &mut out,
                    i,
                    w,
                    if mode < 2 { signed(x, w / 2) as u64 } else { x },
                )
            }
        }
        156..=159 | 188..=191 | 220..=223 => {
            let b = vector(s)?;
            let a = vector(s)?;
            let base = if op < 180 {
                156
            } else if op < 210 {
                188
            } else {
                220
            };
            let w = if base == 156 {
                2
            } else if base == 188 {
                4
            } else {
                8
            };
            let mode = op - base;
            let start = if mode % 2 == 1 { 16 / w } else { 0 };
            for i in 0..16 / w {
                let x = lane(&a, i + start, w / 2);
                let y = lane(&b, i + start, w / 2);
                put(
                    &mut out,
                    i,
                    w,
                    if mode < 2 {
                        signed(x, w / 2).wrapping_mul(signed(y, w / 2)) as u64
                    } else {
                        x.wrapping_mul(y)
                    },
                )
            }
        }
        124..=127 => {
            let a = vector(s)?;
            let w = if op < 126 { 2 } else { 4 };
            for i in 0..16 / w {
                let x = lane(&a, i * 2, w / 2);
                let y = lane(&a, i * 2 + 1, w / 2);
                let v = if op.is_multiple_of(2) {
                    signed(x, w / 2) + signed(y, w / 2)
                } else {
                    (x + y) as i64
                };
                put(&mut out, i, w, v as u64)
            }
        }
        130 => {
            let b = vector(s)?;
            let a = vector(s)?;
            out = zip(a, b, 2, |a, b| {
                ((signed(a, 2) * signed(b, 2) + 0x4000) >> 15).clamp(-32768, 32767) as u64
            })
        }
        186 => {
            let b = vector(s)?;
            let a = vector(s)?;
            for i in 0..4 {
                let mut sum = 0i32;
                for j in 0..2 {
                    sum = sum.wrapping_add(
                        (signed(lane(&a, 2 * i + j, 2), 2) as i32).wrapping_mul(signed(
                            lane(&b, 2 * i + j, 2),
                            2,
                        )
                            as i32),
                    )
                }
                put(&mut out, i, 4, sum as u64)
            }
        }
        248 | 249 => {
            let a = vector(s)?;
            out = map(a, 4, |a| {
                let x = f32::from_bits(a as u32);
                if op == 248 {
                    (x as i32) as u32 as u64
                } else {
                    (x as u32) as u64
                }
            })
        }
        250 | 251 => {
            let a = vector(s)?;
            out = map(a, 4, |a| {
                let x = if op == 250 {
                    (a as i32) as f32
                } else {
                    (a as u32) as f32
                };
                x.to_bits() as u64
            })
        }
        252 | 253 => {
            let a = vector(s)?;
            for i in 0..2 {
                let x = f64::from_bits(lane(&a, i, 8));
                put(
                    &mut out,
                    i,
                    4,
                    if op == 252 {
                        (x as i32) as u32 as u64
                    } else {
                        (x as u32) as u64
                    },
                )
            }
        }
        254 | 255 => {
            let a = vector(s)?;
            for i in 0..2 {
                let x = lane(&a, i, 4);
                put(
                    &mut out,
                    i,
                    8,
                    if op == 254 {
                        (x as i32 as f64).to_bits()
                    } else {
                        (x as u32 as f64).to_bits()
                    },
                )
            }
        }
        94 => {
            let a = vector(s)?;
            for i in 0..2 {
                put(
                    &mut out,
                    i,
                    4,
                    (f64::from_bits(lane(&a, i, 8)) as f32).to_bits() as u64,
                )
            }
        }
        95 => {
            let a = vector(s)?;
            for i in 0..2 {
                put(
                    &mut out,
                    i,
                    8,
                    (f32::from_bits(lane(&a, i, 4) as u32) as f64).to_bits(),
                )
            }
        }
        _ => return Err(trap("unknown SIMD opcode")),
    }
    s.push(Value::V128(V128(out)));
    Ok(())
}
fn relaxed(op: u16, s: &mut Vec<Value>) -> Result<()> {
    let standard = match op {
        0 => Some(14),
        1 => Some(248),
        2 => Some(249),
        3 => Some(252),
        4 => Some(253),
        9..=12 => Some(82),
        13 => Some(232),
        14 => Some(233),
        15 => Some(244),
        16 => Some(245),
        17 => Some(130),
        _ => None,
    };
    if let Some(n) = standard {
        return execute(0xfd00 | n, &Immediate::None, s);
    }
    let out = match op {
        5..=8 => {
            let c = vector(s)?;
            let b = vector(s)?;
            let a = vector(s)?;
            let w = if op < 7 { 4 } else { 8 };
            let mut out = [0; 16];
            for i in 0..16 / w {
                let x = lane(&a, i, w);
                let y = lane(&b, i, w);
                let z = lane(&c, i, w);
                let r = if w == 4 {
                    let x = f32::from_bits(x as u32);
                    let y = f32::from_bits(y as u32);
                    let z = f32::from_bits(z as u32);
                    ((if op.is_multiple_of(2) { -x } else { x }) * y + z).to_bits() as u64
                } else {
                    let x = f64::from_bits(x);
                    let y = f64::from_bits(y);
                    let z = f64::from_bits(z);
                    ((if op.is_multiple_of(2) { -x } else { x }) * y + z).to_bits()
                };
                put(&mut out, i, w, r)
            }
            out
        }
        18 | 19 => {
            let c = if op == 19 { Some(vector(s)?) } else { None };
            let b = vector(s)?;
            let a = vector(s)?;
            let mut out = [0; 16];
            let w = if op == 18 { 2 } else { 4 };
            for i in 0..16 / w {
                let mut sum = if let Some(c) = c {
                    lane(&c, i, 4) as i32
                } else {
                    0
                };
                for j in 0..w {
                    let x = a[w * i + j] as i8 as i32;
                    let y = b[w * i + j] as i8 as i32;
                    sum = sum.wrapping_add(x * y)
                }
                put(&mut out, i, w, sum as u64)
            }
            out
        }
        _ => return Err(trap("unknown relaxed SIMD opcode")),
    };
    s.push(Value::V128(V128(out)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(bytes: [u8; 16]) -> Value {
        Value::V128(V128(bytes))
    }
    fn run(op: u16, args: Vec<Value>) -> Value {
        let mut s = args;
        execute(op, &Immediate::None, &mut s).unwrap();
        assert_eq!(s.len(), 1);
        s[0]
    }
    #[test]
    fn t093_simd_wrapping_saturation_and_signed_comparison() {
        assert_eq!(run(0xfd6e, vec![v([255; 16]), v([1; 16])]), v([0; 16]));
        assert_eq!(run(0xfd70, vec![v([255; 16]), v([1; 16])]), v([255; 16]));
        assert_eq!(run(0xfd25, vec![v([255; 16]), v([1; 16])]), v([255; 16]));
    }
    #[test]
    fn t094_simd_extension_uses_upper_half() {
        let mut a = [0; 16];
        a[8..].fill(255);
        assert_eq!(run(0xfd88, vec![v(a)]), v([255; 16]));
    }
    #[test]
    fn t095_simd_float_min_signed_zero_and_nan() {
        let z = v([0; 16]);
        let n = v(map([0; 16], 4, |_| 0x80000000));
        assert_eq!(run(0xfde8, vec![z, n]), n);
        assert_eq!(run(0xfde9, vec![z, n]), z);
        let nan = v(map([0; 16], 4, |_| f32::NAN.to_bits() as u64));
        let Value::V128(result) = run(0xfde8, vec![z, nan]) else {
            panic!()
        };
        assert!(f32::from_bits(lane(&result.0, 0, 4) as u32).is_nan());
    }
    #[test]
    fn t097_relaxed_multiply_add_and_bitselect() {
        let one = v(map([0; 16], 4, |_| 1f32.to_bits() as u64));
        let three = v(map([0; 16], 4, |_| 3f32.to_bits() as u64));
        let two = v(map([0; 16], 4, |_| 2f32.to_bits() as u64));
        assert_eq!(run(0xfa05, vec![one, two, one]), three);
        assert_eq!(
            run(0xfa09, vec![v([17; 16]), v([34; 16]), v([255; 16])]),
            v([17; 16])
        );
    }
    #[test]
    fn t093_shuffle_validates_lanes() {
        let mut s = vec![v([0; 16]), v([1; 16])];
        assert!(execute(0xfd0d, &Immediate::Bytes(vec![32; 16].into_boxed_slice()), &mut s).is_err());
    }
}

// Borrowed compact vector adapter, sharing canonical portable lane semantics.
// This helper is absent from default builds and creates no immediate Vec.
#[cfg(feature = "compact-executor")]
pub(super) fn execute_prepared_vector(opcode: u16, bytes: &[u8; 16], s: &mut Vec<Value>) -> Result<()> {
    let out = match opcode {
        0xfd0c => *bytes,
        0xfd0d => {
            let b = vector(s)?;
            let a = vector(s)?;
            if bytes.iter().any(|index| *index >= 32) {
                return Err(trap("invalid shuffle lane"));
            }
            let mut out = [0; 16];
            for (destination, index) in out.iter_mut().zip(bytes) {
                *destination = if *index < 16 { a[*index as usize] } else { b[*index as usize - 16] };
            }
            out
        }
        _ => return Err(trap("unverified prepared vector opcode")),
    };
    s.push(Value::V128(V128(out)));
    Ok(())
}
