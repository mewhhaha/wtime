use super::*;
pub(super) fn exec(op: u16, s: &mut Vec<Value>) -> Result<()> {
    // Operands are rewritten in place: a unary operation replaces the top
    // value and a binary one pops its right operand and replaces the left.
    // Errors and their order match pop/check/push.
    macro_rules! unary {
        ($v:ident,$e:expr) => {{
            let slot = s.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
            let Value::$v(a) = *slot else {
                return Err(trap("numeric type mismatch"));
            };
            *slot = $e(a);
        }};
    }
    macro_rules! binary {
        ($v:ident,$e:expr) => {{
            let Value::$v(b) = pop(s)? else {
                return Err(trap("numeric type mismatch"));
            };
            let slot = s.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
            let Value::$v(a) = *slot else {
                return Err(trap("numeric type mismatch"));
            };
            *slot = $e(a, b);
        }};
    }
    macro_rules! intops {
        ($v:ident,$t:ty,$u:ty,$base:expr) => {{
            match op - $base {
                0 => unary!($v, |a: $t| Value::$v(a.leading_zeros() as $t)),
                1 => unary!($v, |a: $t| Value::$v(a.trailing_zeros() as $t)),
                2 => unary!($v, |a: $t| Value::$v(a.count_ones() as $t)),
                3 => binary!($v, |a: $t, b: $t| Value::$v(a.wrapping_add(b))),
                4 => binary!($v, |a: $t, b: $t| Value::$v(a.wrapping_sub(b))),
                5 => binary!($v, |a: $t, b: $t| Value::$v(a.wrapping_mul(b))),
                6..=9 => {
                    let Value::$v(b) = pop(s)? else {
                        return Err(trap("integer expected"));
                    };
                    let Value::$v(a) = pop(s)? else {
                        return Err(trap("integer expected"));
                    };
                    if b == 0 {
                        return Err(trap("integer divide by zero"));
                    }
                    let v = match op - $base {
                        6 => a.checked_div(b).ok_or_else(|| trap("integer overflow"))?,
                        7 => ((a as $u) / (b as $u)) as $t,
                        8 => a.wrapping_rem(b),
                        _ => ((a as $u) % (b as $u)) as $t,
                    };
                    s.push(Value::$v(v));
                }
                10 => binary!($v, |a: $t, b: $t| Value::$v(a & b)),
                11 => binary!($v, |a: $t, b: $t| Value::$v(a | b)),
                12 => binary!($v, |a: $t, b: $t| Value::$v(a ^ b)),
                13 => binary!($v, |a: $t, b: $t| Value::$v(a.wrapping_shl(b as u32))),
                14 => binary!($v, |a: $t, b: $t| Value::$v(a.wrapping_shr(b as u32))),
                15 => binary!($v, |a: $t, b: $t| Value::$v(
                    (a as $u).wrapping_shr(b as u32) as $t
                )),
                16 => binary!($v, |a: $t, b: $t| Value::$v(a.rotate_left(b as u32))),
                17 => binary!($v, |a: $t, b: $t| Value::$v(a.rotate_right(b as u32))),
                _ => unreachable!(),
            }
        }};
    }
    macro_rules! cmp {
        ($v:ident,$t:ty,$u:ty,$base:expr) => {{
            if op == $base {
                unary!($v, |a: $t| Value::I32((a == 0) as i32))
            } else {
                binary!($v, |a: $t, b: $t| Value::I32(match op - $base {
                    1 => a == b,
                    2 => a != b,
                    3 => a < b,
                    4 => (a as $u) < (b as $u),
                    5 => a > b,
                    6 => (a as $u) > (b as $u),
                    7 => a <= b,
                    8 => (a as $u) <= (b as $u),
                    9 => a >= b,
                    _ => (a as $u) >= (b as $u),
                } as i32))
            }
        }};
    }
    macro_rules! float {
        ($v:ident,$f:ty,$bits:ty,$base:expr) => {{
            let un = op - $base;
            if un < 7 {
                unary!($v, |a: $bits| {
                    let a = <$f>::from_bits(a);
                    let bits = match un {
                        0 => a.abs(),
                        1 => -a,
                        2 => a.ceil(),
                        3 => a.floor(),
                        4 => a.trunc(),
                        5 => a.round_ties_even(),
                        _ => a.sqrt(),
                    }
                    .to_bits();
                    Value::$v(if un >= 2 && <$f>::from_bits(bits).is_nan() {
                        bits | ((if <$bits>::BITS == 32 {
                            0x00400000u64
                        } else {
                            0x0008000000000000u64
                        }) as $bits)
                    } else {
                        bits
                    })
                })
            } else {
                binary!($v, |a: $bits, b: $bits| {
                    let (a, b) = (<$f>::from_bits(a), <$f>::from_bits(b));
                    Value::$v(
                        match un {
                            7 => a + b,
                            8 => a - b,
                            9 => a * b,
                            10 => a / b,
                            11 => {
                                if a.is_nan() || b.is_nan() {
                                    <$f>::NAN
                                } else if a == b {
                                    <$f>::from_bits(a.to_bits() | b.to_bits())
                                } else {
                                    a.min(b)
                                }
                            }
                            12 => {
                                if a.is_nan() || b.is_nan() {
                                    <$f>::NAN
                                } else if a == b {
                                    <$f>::from_bits(a.to_bits() & b.to_bits())
                                } else {
                                    a.max(b)
                                }
                            }
                            _ => a.copysign(b),
                        }
                        .to_bits(),
                    )
                })
            }
        }};
    }
    match op {
        0x45..=0x4f => cmp!(I32, i32, u32, 0x45),
        0x50..=0x5a => cmp!(I64, i64, u64, 0x50),
        0x5b..=0x60 => binary!(F32, |a, b| Value::I32(fcmp(
            f32::from_bits(a) as f64,
            f32::from_bits(b) as f64,
            op - 0x5b
        ) as i32)),
        0x61..=0x66 => {
            binary!(F64, |a, b| Value::I32(
                fcmp(f64::from_bits(a), f64::from_bits(b), op - 0x61) as i32
            ))
        }
        0x67..=0x78 => intops!(I32, i32, u32, 0x67),
        0x79..=0x8a => intops!(I64, i64, u64, 0x79),
        0x8b..=0x98 => float!(F32, f32, u32, 0x8b),
        0x99..=0xa6 => float!(F64, f64, u64, 0x99),
        0xa7 => unary!(I64, |a| Value::I32(a as i32)),
        0xac => unary!(I32, |a: i32| Value::I64(a as i64)),
        0xad => unary!(I32, |a: i32| Value::I64(a as u32 as i64)),
        0xa8..=0xab | 0xae..=0xb1 | 0xfc00..=0xfc07 => {
            let sat = op >= 0xfc00;
            let n = if sat {
                op - 0xfc00
            } else if op <= 0xab {
                op - 0xa8
            } else {
                op - 0xae + 4
            };
            let f = match pop(s)? {
                Value::F32(x) => f32::from_bits(x) as f64,
                Value::F64(x) => f64::from_bits(x),
                _ => return Err(trap("float expected")),
            };
            let wide = n >= 4;
            let unsigned = n % 2 == 1;
            let (min, max) = if wide {
                if unsigned {
                    (0., 18446744073709551616.)
                } else {
                    (-9223372036854775808., 9223372036854775808.)
                }
            } else if unsigned {
                (0., 4294967296.)
            } else {
                (-2147483648., 2147483648.)
            };
            let f = f.trunc();
            if !sat && f.is_nan() {
                return Err(trap("invalid integer conversion"));
            }
            if !sat && (f < min || f >= max) {
                return Err(trap("integer overflow"));
            }
            s.push(if wide {
                Value::I64(if unsigned { f as u64 as i64 } else { f as i64 })
            } else {
                Value::I32(if unsigned { f as u32 as i32 } else { f as i32 })
            });
        }
        0xb2..=0xb5 | 0xb7..=0xba => {
            let unsigned = op % 2 == 1;
            if op <= 0xb5 {
                let f = match pop(s)? {
                    Value::I32(x) => {
                        if unsigned {
                            x as u32 as f32
                        } else {
                            x as f32
                        }
                    }
                    Value::I64(x) => {
                        if unsigned {
                            x as u64 as f32
                        } else {
                            x as f32
                        }
                    }
                    _ => return Err(trap("integer expected")),
                };
                s.push(Value::F32(f.to_bits()))
            } else {
                let f = match pop(s)? {
                    Value::I32(x) => {
                        if !unsigned {
                            x as u32 as f64
                        } else {
                            x as f64
                        }
                    }
                    Value::I64(x) => {
                        if !unsigned {
                            x as u64 as f64
                        } else {
                            x as f64
                        }
                    }
                    _ => return Err(trap("integer expected")),
                };
                s.push(Value::F64(f.to_bits()))
            }
        }
        0xb6 => unary!(F64, |a| Value::F32((f64::from_bits(a) as f32).to_bits())),
        0xbb => unary!(F32, |a| Value::F64((f32::from_bits(a) as f64).to_bits())),
        0xbc => unary!(F32, |a| Value::I32(a as i32)),
        0xbd => unary!(F64, |a| Value::I64(a as i64)),
        0xbe => unary!(I32, |a| Value::F32(a as u32)),
        0xbf => unary!(I64, |a| Value::F64(a as u64)),
        0xc0 => unary!(I32, |a| Value::I32(a as i8 as i32)),
        0xc1 => unary!(I32, |a| Value::I32(a as i16 as i32)),
        0xc2 => unary!(I64, |a| Value::I64(a as i8 as i64)),
        0xc3 => unary!(I64, |a| Value::I64(a as i16 as i64)),
        0xc4 => unary!(I64, |a| Value::I64(a as i32 as i64)),
        _ => return Err(trap(format!("unsupported numeric opcode {op:x}"))),
    }
    Ok(())
}
fn fcmp(a: f64, b: f64, n: u16) -> bool {
    match n {
        0 => a == b,
        1 => a != b,
        2 => a < b,
        3 => a > b,
        4 => a <= b,
        _ => a >= b,
    }
}
