//! Wide arithmetic's pair order is low limb, then high limb.
use super::*;
fn operand(stack: &mut Vec<Value>) -> Result<i64> {
    match pop(stack)? {
        Value::I64(x) => Ok(x),
        _ => Err(trap("wide arithmetic requires i64")),
    }
}
pub(crate) fn execute(opcode: u16, stack: &mut Vec<Value>) -> Result<()> {
    let bits = match opcode {
        0xfc13 | 0xfc14 => {
            let b_hi = operand(stack)? as u64 as u128;
            let b_lo = operand(stack)? as u64 as u128;
            let a_hi = operand(stack)? as u64 as u128;
            let a_lo = operand(stack)? as u64 as u128;
            let (a, b) = ((a_hi << 64) | a_lo, (b_hi << 64) | b_lo);
            if opcode == 0xfc13 {
                a.wrapping_add(b)
            } else {
                a.wrapping_sub(b)
            }
        }
        0xfc15 => {
            let b = operand(stack)? as i128;
            let a = operand(stack)? as i128;
            (a * b) as u128
        }
        0xfc16 => {
            let b = operand(stack)? as u64 as u128;
            let a = operand(stack)? as u64 as u128;
            a * b
        }
        _ => return Err(trap("unknown wide arithmetic instruction")),
    };
    stack.push(Value::I64(bits as u64 as i64));
    stack.push(Value::I64((bits >> 64) as u64 as i64));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn eval(op: u16, inputs: &[i64]) -> Vec<Value> {
        let mut s = inputs.iter().copied().map(Value::I64).collect();
        execute(op, &mut s).unwrap();
        s
    }
    #[test]
    fn t091_case_01() {
        assert_eq!(eval(0xfc13, &[-1, 0, 1, 0]), [Value::I64(0), Value::I64(1)]);
        assert_eq!(eval(0xfc14, &[0, 1, 1, 0]), [Value::I64(-1), Value::I64(0)]);
        assert_eq!(
            eval(0xfc13, &[-1, -1, 1, 0]),
            [Value::I64(0), Value::I64(0)]
        );
    }
    #[test]
    fn t091_case_02() {
        assert_eq!(
            eval(0xfc15, &[i64::MIN, -1]),
            [Value::I64(i64::MIN), Value::I64(0)]
        );
        assert_eq!(
            eval(0xfc15, &[i64::MIN, 2]),
            [Value::I64(0), Value::I64(-1)]
        );
    }
    #[test]
    fn t091_case_03() {
        assert_eq!(eval(0xfc16, &[-1, -1]), [Value::I64(1), Value::I64(-2)]);
    }
}
