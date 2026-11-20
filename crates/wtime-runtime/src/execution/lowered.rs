//! Lowered function bodies for the Decoded tier's straight-line runs.
//!
//! A body is lowered once, the first time a straight-line run reaches it, to
//! one 16-byte `Op` per source instruction: the same program counters, with
//! operands resolved ahead of time (local and global indices, constants,
//! memory-0 offsets, block arities and control boundaries). The straight-line
//! loop dispatches on these instead of decoding immediates, looking up block
//! types and the control map on every execution. Anything not lowered is
//! `Slow` (run from the source instruction exactly as before) or `Leave` (not
//! a straight-line instruction). Lowering never changes what an instruction
//! does, which errors it raises or how it is charged.
use super::*;

/// Sentinel for an absent else position.
pub(crate) const NO_ELSE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Not a straight-line instruction: the run hands it to the general loop.
    Leave,
    /// A straight-line instruction run from its source `Instruction`.
    Slow,
    Nop,
    /// `block`, `loop`, `if`: the end and else positions and the arity.
    Block { end: u32, params: u16, results: u16 },
    Loop { end: u32, params: u16, results: u16 },
    If { end: u32, other: u32, params: u16, results: u16 },
    Else,
    End,
    Br(u32),
    BrIf(u32),
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    /// A scalar load or store on memory 0, by opcode and static offset.
    Memory0 { opcode: u16, offset: u64 },
    I32Const(i32),
    I64Const(i64),
    F32Const(u32),
    F64Const(u64),
    I32Eqz,
    /// An integer operator computed inline by the run (opcode).
    Int(u16),
    /// Any other numeric operator, through numeric::exec (opcode).
    Numeric(u16),
    // Fused pairs: the op at a pair's first instruction runs both
    // instructions, charging the second before it runs exactly as the loop
    // would. The second instruction keeps its own op, so the array stays one
    // op per instruction; no pair's second instruction is a branch target,
    // because no first instruction is a control instruction.
    LocalSetGet { set: u32, get: u32 },
    LocalGetGet { first: u32, second: u32 },
    LocalGetI32Const { local: u32, value: i32 },
    I32ConstAdd(i32),
    GlobalGetLocalSet { global: u32, local: u32 },
    I32AddLocalSet(u32),
    LocalGetLocalSet { from: u32, to: u32 },
    LocalGetGlobalSet { local: u32, global: u32 },
}

/// The fused op for two consecutive lowered ops, if any.
fn fuse(first: Op, second: Op) -> Option<Op> {
    Some(match (first, second) {
        (Op::LocalSet(set), Op::LocalGet(get)) => Op::LocalSetGet { set, get },
        (Op::GlobalGet(global), Op::LocalSet(local)) => Op::GlobalGetLocalSet { global, local },
        (Op::I32Const(value), Op::Int(0x6a)) => Op::I32ConstAdd(value),
        (Op::Int(0x6a), Op::LocalSet(local)) => Op::I32AddLocalSet(local),
        (Op::LocalGet(local), Op::I32Const(value)) => Op::LocalGetI32Const { local, value },
        (Op::LocalGet(first), Op::LocalGet(second)) => Op::LocalGetGet { first, second },
        (Op::LocalGet(from), Op::LocalSet(to)) => Op::LocalGetLocalSet { from, to },
        (Op::LocalGet(local), Op::GlobalSet(global)) => Op::LocalGetGlobalSet { local, global },
        _ => return None,
    })
}

/// Integer operators the run computes inline; the same set as its fast arms.
fn inline_int(opcode: u16) -> bool {
    matches!(
        opcode,
        0x46..=0x4f | 0x51..=0x54 | 0x6a..=0x6c | 0x71..=0x76 | 0x7c..=0x7e | 0x83 | 0x84 | 0x86 | 0x88
    )
}

fn arity16(params: usize, results: usize) -> Option<(u16, u16)> {
    Some((u16::try_from(params).ok()?, u16::try_from(results).ok()?))
}

/// Lowers body `body` of `module`. Positions and arities that do not fit the
/// compact fields leave the instruction `Slow`.
pub(crate) fn lower(module: &Module, body: usize) -> Box<[Op]> {
    let code = &module.data.inner.decoded().code[body];
    let mut ops: Box<[Op]> = code
        .instructions
        .iter()
        .enumerate()
        .map(|(pc, instruction)| lower_one(module, body, pc, instruction).unwrap_or(Op::Slow))
        .collect();
    // Pairs, greedily from the left; an instruction ends at most one pair.
    let mut pc = 0;
    while pc + 1 < ops.len() {
        if let Some(fused) = fuse(ops[pc], ops[pc + 1]) {
            ops[pc] = fused;
            pc += 2;
        } else {
            pc += 1;
        }
    }
    ops
}

fn lower_one(module: &Module, body: usize, pc: usize, i: &Instruction) -> Option<Op> {
    Some(match (i.opcode, &i.immediate) {
        (0x01, _) => Op::Nop,
        (0x02..=0x04, immediate) => {
            let (params, results) = arity(immediate, module);
            let (params, results) = arity16(params, results)?;
            let (end, other) = module.data.control.get(body, pc)?;
            let end = u32::try_from(end).ok()?;
            match i.opcode {
                0x02 => Op::Block { end, params, results },
                0x03 => Op::Loop { end, params, results },
                _ => Op::If { end, other: other.map_or(Some(NO_ELSE), |x| u32::try_from(x).ok())?, params, results },
            }
        }
        (0x05, _) => Op::Else,
        (0x0b, _) => Op::End,
        (0x0c, Immediate::Index(x)) => Op::Br(*x),
        (0x0d, Immediate::Index(x)) => Op::BrIf(*x),
        (0x0e, _) => Op::Slow,
        (0x1a, _) => Op::Drop,
        (0x1b | 0x1c, _) => Op::Select,
        (0x20, Immediate::Index(x)) => Op::LocalGet(*x),
        (0x21, Immediate::Index(x)) => Op::LocalSet(*x),
        (0x22, Immediate::Index(x)) => Op::LocalTee(*x),
        (0x23, Immediate::Index(x)) => Op::GlobalGet(*x),
        (0x24, Immediate::Index(x)) => Op::GlobalSet(*x),
        (0x28..=0x3e, Immediate::MemArg { memory: 0, offset, .. }) => Op::Memory0 { opcode: i.opcode, offset: *offset },
        (0x28..=0x3e, _) => Op::Slow,
        (0x41, Immediate::I32(x)) => Op::I32Const(*x),
        (0x42, Immediate::I64(x)) => Op::I64Const(*x),
        (0x43, Immediate::F32(x)) => Op::F32Const(*x),
        (0x44, Immediate::F64(x)) => Op::F64Const(*x),
        (0x45, _) => Op::I32Eqz,
        (opcode @ 0x46..=0xc4, _) if inline_int(opcode) => Op::Int(opcode),
        (opcode @ 0x46..=0xc4, _) => Op::Numeric(opcode),
        // Other straight-line opcodes with an unexpected immediate run from
        // the instruction, which reports the same error as before.
        (0x0c | 0x0d | 0x20..=0x24 | 0x41..=0x44, _) => Op::Slow,
        _ => Op::Leave,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<Op>(), 16);
    }

    #[test]
    fn fused_pairs_never_start_with_control() {
        let controls = [
            Op::Block { end: 1, params: 0, results: 0 },
            Op::Loop { end: 1, params: 0, results: 0 },
            Op::If { end: 1, other: NO_ELSE, params: 0, results: 0 },
            Op::Else,
            Op::End,
            Op::Br(0),
            Op::BrIf(0),
            Op::Leave,
            Op::Slow,
        ];
        let any = [Op::LocalGet(0), Op::LocalSet(0), Op::I32Const(0), Op::Int(0x6a), Op::GlobalSet(0)];
        for first in controls {
            for second in any {
                assert_eq!(fuse(first, second), None);
            }
        }
    }
}
