//! Direct safe compact fields. Called after the common charged source boundary.
//! Step::Return uses the existing frame-result contract.
use super::*;

pub(crate) enum Step {
    Continue,
    Return,
    Fusion(super::fuse::Pattern),
    Call { index: u32, tail: bool },
    Memory(super::memory::MemoryOp),
    Exit { pc: usize, opcode: u16 },
}

pub(super) fn record(op: CompactOp, store: &Store) {
    #[cfg(not(feature = "opcode-coverage"))]
    let _ = op;
    #[cfg(feature = "performance-counters")]
    {
        store.perf.record(perf::Event::Dispatch, 1);
        // CompactStep is a new diagnostic event to add during integration.
        store.perf.record(perf::Event::CompactStep, 1);
    }
    #[cfg(not(feature = "performance-counters"))]
    let _ = store;
    #[cfg(feature = "opcode-coverage")]
    crate::coverage::record(op.opcode);
}

pub(super) fn execute(
    op: CompactOp,
    locals: &mut [Value],
    stack: &mut Vec<Value>,
    _store: &Store,
) -> Result<Step> {
    match op.opcode {
        1 => {}
        0x0b | 0x0f => return Ok(Step::Return),
        0x10 | 0x12 => {
            return Ok(Step::Call {
                index: op.aux,
                tail: op.opcode == 0x12,
            });
        }
        0x1a => {
            pop(stack)?;
        }
        0x1b | 0x1c => {
            let condition = address(pop(stack)?)?;
            let second = pop(stack)?;
            let first = pop(stack)?;
            stack.push(if condition != 0 { first } else { second });
        }
        0x20 => stack.push(
            *locals
                .get(op.aux as usize)
                .ok_or_else(|| trap("local index"))?,
        ),
        0x21 => {
            let value = pop(stack)?;
            *locals
                .get_mut(op.aux as usize)
                .ok_or_else(|| trap("local index"))? = value;
        }
        0x22 => {
            let value = *stack.last().ok_or_else(|| trap("empty stack"))?;
            *locals
                .get_mut(op.aux as usize)
                .ok_or_else(|| trap("local index"))? = value;
        }
        0x41 => stack.push(Value::I32(op.payload as u32 as i32)),
        0x42 => stack.push(Value::I64(op.payload as i64)),
        0x45..=0x5a
        | 0x67..=0x6c
        | 0x71..=0x7e
        | 0x83..=0x8a
        | 0xa7
        | 0xac
        | 0xad
        | 0xc0..=0xc4 => numeric::exec(op.opcode, stack)?,
        _ => return Err(trap("unverified compact scalar opcode")),
    }
    Ok(Step::Continue)
}
