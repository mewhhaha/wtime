//! Lowered function bodies for the Decoded tier's straight-line runs.
//!
//! A body is lowered once it is warm (see straight.rs `warm`; until then its
//! runs execute every instruction from its source, as `Slow`), to one
//! 16-byte `Op` per source instruction: the same program counters, with
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
    // because no first instruction is a control instruction (see `group`).
    LocalSetGet { set: u32, get: u32 },
    LocalGetGet { first: u32, second: u32 },
    LocalGetI32Const { local: u32, value: i32 },
    I32ConstAdd(i32),
    GlobalGetLocalSet { global: u32, local: u32 },
    I32AddLocalSet(u32),
    LocalGetLocalSet { from: u32, to: u32 },
    LocalGetGlobalSet { local: u32, global: u32 },
    /// `i32.add; load` on memory 0 with a 32-bit static offset.
    I32AddLoad { offset: u32, opcode: u16 },
    // Fused expressions over locals: the op at the first instruction runs
    // them all without materializing intermediate operands, charging each
    // later instruction before it runs with the operand-stack height it
    // would have. As with pairs, every other instruction keeps its own op.
    /// `local.get local; T.const value; binop` for an inline integer binop
    /// of type T (an i64 constant that fits in i32 is stored narrowed).
    LocalConstBin { local: u32, value: i32, opcode: u16 },
    /// `local.get local; T.const value; binop; local.set to`.
    LocalConstBinSet { local: u32, value: i32, to: u32, opcode: u16 },
    /// `local.get local; load` on memory 0 with a 32-bit static offset.
    LocalLoad { local: u32, offset: u32, opcode: u16 },
    /// `local.get local; load; local.set to`.
    LocalLoadSet { local: u32, offset: u32, to: u32, opcode: u16 },
    /// `local.get addr; local.get value; store` on memory 0.
    LocalLocalStore { addr: u32, value: u32, offset: u32, opcode: u16 },
    /// `local.get a; local.get b; binop` for an inline integer binop.
    LocalLocalBin { a: u32, b: u32, opcode: u16 },
    /// `local.get local; unop` for i32.eqz or an inline integer conversion.
    LocalUnary { local: u32, opcode: u16 },
    /// `local.get local; i32.wrap_i64; i32.const value; binop`.
    LocalWrapConstBin { local: u32, value: i32, opcode: u16 },
    // A computation ending in a conditional: the control instruction may
    // close a group because it is preceded by an instruction of the group,
    // so it is neither a branch target nor a resumption point either.
    /// An inline integer operator or i32.eqz, then `if` with these fields.
    IntIf { opcode: u16, end: u32, other: u32, params: u16, results: u16 },
    /// An inline integer operator or i32.eqz, then `br_if depth`.
    IntBrIf { opcode: u16, depth: u32 },
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
        (Op::Int(0x6a), Op::Memory0 { opcode, offset }) if is_load(opcode) => {
            Op::I32AddLoad { offset: u32::try_from(offset).ok()?, opcode }
        }
        (Op::LocalGet(local), Op::I32Eqz) => Op::LocalUnary { local, opcode: 0x45 },
        (Op::LocalGet(local), Op::Int(opcode)) if is_unary(opcode) => Op::LocalUnary { local, opcode },
        (Op::Int(opcode), Op::If { end, other, params, results }) => Op::IntIf { opcode, end, other, params, results },
        (Op::I32Eqz, Op::If { end, other, params, results }) => Op::IntIf { opcode: 0x45, end, other, params, results },
        (Op::Int(opcode), Op::BrIf(depth)) => Op::IntBrIf { opcode, depth },
        (Op::I32Eqz, Op::BrIf(depth)) => Op::IntBrIf { opcode: 0x45, depth },
        _ => return None,
    })
}

/// The fused expression starting at `ops[pc]` with the most instructions, and
/// how many it covers.
fn fuse_expression(ops: &[Op], pc: usize) -> Option<(usize, Op)> {
    let at = |n: usize| ops.get(pc + n).copied();
    let constant = |op: Option<Op>, opcode: u16| match op {
        Some(Op::I32Const(v)) if i32_binop(opcode) => Some(v),
        Some(Op::I64Const(v)) if i64_binop(opcode) => i32::try_from(v).ok(),
        _ => None,
    };
    let offset32 = |offset: u64| u32::try_from(offset).ok();
    let Some(Op::LocalGet(local)) = at(0) else { return None };
    match (at(1), at(2), at(3)) {
        (value, Some(Op::Int(opcode)), set) if constant(value, opcode).is_some() => {
            let value = constant(value, opcode)?;
            Some(match set {
                Some(Op::LocalSet(to)) => (4, Op::LocalConstBinSet { local, value, to, opcode }),
                _ => (3, Op::LocalConstBin { local, value, opcode }),
            })
        }
        (Some(Op::Memory0 { opcode, offset }), set, _) if is_load(opcode) => {
            let offset = offset32(offset)?;
            Some(match set {
                Some(Op::LocalSet(to)) => (3, Op::LocalLoadSet { local, offset, to, opcode }),
                _ => (2, Op::LocalLoad { local, offset, opcode }),
            })
        }
        (Some(Op::LocalGet(value)), Some(Op::Memory0 { opcode, offset }), _) if !is_load(opcode) => {
            Some((3, Op::LocalLocalStore { addr: local, value, offset: offset32(offset)?, opcode }))
        }
        (Some(Op::LocalGet(b)), Some(Op::Int(opcode)), _) if !is_unary(opcode) => {
            Some((3, Op::LocalLocalBin { a: local, b, opcode }))
        }
        (Some(Op::Int(0xa7)), Some(Op::I32Const(value)), Some(Op::Int(opcode))) if i32_binop(opcode) => {
            Some((4, Op::LocalWrapConstBin { local, value, opcode }))
        }
        _ => None,
    }
}

/// Of the inline integer operators (`Op::Int`), the unary ones.
pub(crate) fn is_unary(opcode: u16) -> bool {
    matches!(opcode, 0xa7 | 0xac | 0xad)
}

/// A scalar load opcode (0x28..=0x35); stores are 0x36..=0x3e.
pub(crate) fn is_load(opcode: u16) -> bool {
    opcode < 0x36
}

/// Of the inline integer operators (`Op::Int`), the binops on i32 operands.
pub(crate) fn i32_binop(opcode: u16) -> bool {
    matches!(opcode, 0x46..=0x4f | 0x6a..=0x78)
}

/// Of the inline integer operators (`Op::Int`), the binops on i64 operands.
pub(crate) fn i64_binop(opcode: u16) -> bool {
    matches!(opcode, 0x51..=0x5a | 0x7c..=0x8a)
}

/// Integer operators the run computes inline; the same set as its fast arms.
fn inline_int(opcode: u16) -> bool {
    matches!(
        opcode,
        0x46..=0x4f | 0x51..=0x5a | 0x6a..=0x6c | 0x71..=0x76 | 0x7c..=0x7e | 0x83 | 0x84 | 0x86 | 0x88 | 0xa7 | 0xac | 0xad
    )
}

fn arity16(params: usize, results: usize) -> Option<(u16, u16)> {
    Some((u16::try_from(params).ok()?, u16::try_from(results).ok()?))
}

/// Lowers body `body` of `module`. Positions and arities that do not fit the
/// compact fields leave the instruction `Slow`.
/// The ops of a cold body: Slow for each of its first `SLOW.len()`
/// instructions, so that each runs from its source. Instructions past these
/// leave the run to the general loop, as at the end of a body. Read-only
/// data, paged in as far as cold bodies reach.
pub(crate) static SLOW: [Op; 1 << 16] = [Op::Slow; 1 << 16];

#[inline(never)]
pub(crate) fn lower(module: &Module, body: usize) -> Box<[Op]> {
    let code = &module.data.inner.decoded().code[body];
    let mut ops: Box<[Op]> = code
        .instructions
        .iter()
        .enumerate()
        .map(|(pc, instruction)| lower_one(module, body, pc, instruction).unwrap_or(Op::Slow))
        .collect();
    group(&mut ops);
    ops
}

/// Replaces the first op of each chosen group with its fused op.
fn group(ops: &mut [Op]) {
    // Group instructions into the fewest dispatches: best[pc] is the
    // number of ops dispatched from pc to the end, len[pc] the length of the
    // group starting there (0 for none); the fused op is formed again when
    // applied, which keeps the scratch at five bytes per instruction. No
    // group starts with a control instruction or holds one before its last
    // instruction, so only its first instruction can be a branch target or a
    // resumption point.
    let n = ops.len();
    let mut best = vec![0u32; n + 1];
    let mut len = vec![0u8; n];
    for pc in (0..n).rev() {
        best[pc] = 1 + best[pc + 1];
        for (k, _) in candidates(ops, pc).into_iter().flatten() {
            let cost = 1 + best[pc + k];
            if cost < best[pc] || (cost == best[pc] && len[pc] != 0 && usize::from(len[pc]) < k) {
                best[pc] = cost;
                len[pc] = k as u8;
            }
        }
    }
    let mut pc = 0;
    while pc < n {
        let k = usize::from(len[pc]);
        if k == 0 {
            pc += 1;
            continue;
        }
        if let Some((_, op)) = candidates(ops, pc).into_iter().flatten().find(|c| c.0 == k) {
            ops[pc] = op;
        }
        pc += k;
    }
}

/// The groups that can start at `pc`, as (instructions covered, fused op).
fn candidates(ops: &[Op], pc: usize) -> [Option<(usize, Op)>; 2] {
    let pair = ops.get(pc + 1).and_then(|&second| fuse(ops[pc], second)).map(|op| (2, op));
    [fuse_expression(ops, pc), pair]
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
    fn groups_cover_the_fewest_dispatches() {
        let mut ops = [
            Op::LocalSet(0),
            Op::LocalGet(1),
            Op::I32Const(8),
            Op::Int(0x6c),
            Op::LocalSet(2),
            Op::LocalGet(2),
            Op::Memory0 { opcode: 0x29, offset: 16 },
            Op::LocalGet(3),
            Op::I64Const(-3),
            Op::Int(0x7c),
            Op::LocalGet(0),
            Op::LocalGet(1),
            Op::Memory0 { opcode: 0x36, offset: 1 << 32 },
            Op::End,
        ];
        let original = ops;
        group(&mut ops);
        let mut expected = original;
        expected[1] = Op::LocalConstBinSet { local: 1, value: 8, to: 2, opcode: 0x6c };
        expected[5] = Op::LocalLoad { local: 2, offset: 16, opcode: 0x29 };
        expected[7] = Op::LocalConstBin { local: 3, value: -3, opcode: 0x7c };
        // An offset beyond 32 bits leaves the store unfused.
        expected[10] = Op::LocalGetGet { first: 0, second: 1 };
        assert_eq!(ops, expected);
    }

    #[test]
    fn every_pair_forms() {
        let store = Op::Memory0 { opcode: 0x36, offset: 0 };
        // Each pair, then an op that groups with neither of its instructions.
        let cases = [
            ([Op::LocalSet(0), Op::LocalGet(1), store], Op::LocalSetGet { set: 0, get: 1 }),
            ([Op::LocalGet(0), Op::LocalGet(1), Op::End], Op::LocalGetGet { first: 0, second: 1 }),
            ([Op::LocalGet(0), Op::I32Const(7), Op::End], Op::LocalGetI32Const { local: 0, value: 7 }),
            ([Op::I32Const(7), Op::Int(0x6a), Op::End], Op::I32ConstAdd(7)),
            ([Op::GlobalGet(0), Op::LocalSet(1), Op::End], Op::GlobalGetLocalSet { global: 0, local: 1 }),
            ([Op::Int(0x6a), Op::LocalSet(1), Op::End], Op::I32AddLocalSet(1)),
            ([Op::LocalGet(0), Op::LocalSet(1), Op::End], Op::LocalGetLocalSet { from: 0, to: 1 }),
            ([Op::LocalGet(0), Op::GlobalSet(1), Op::End], Op::LocalGetGlobalSet { local: 0, global: 1 }),
            (
                [Op::Int(0x6a), Op::Memory0 { opcode: 0x28, offset: 4 }, Op::End],
                Op::I32AddLoad { offset: 4, opcode: 0x28 },
            ),
        ];
        for (mut ops, pair) in cases {
            let original = ops;
            group(&mut ops);
            assert_eq!(ops, [pair, original[1], original[2]]);
        }
    }

    #[test]
    fn conditionals_close_groups() {
        let mut ops = [
            Op::I32Const(5),
            Op::LocalGet(0),
            Op::Int(0x46),
            Op::BrIf(1),
            Op::LocalGet(0),
            Op::I32Eqz,
            Op::If { end: 9, other: NO_ELSE, params: 0, results: 0 },
            Op::LocalGet(1),
            Op::Int(0xa7),
            Op::End,
            Op::LocalGet(1),
            Op::Int(0xa7),
            Op::I32Const(3),
            Op::Int(0x6c),
            Op::End,
        ];
        let original = ops;
        group(&mut ops);
        let mut expected = original;
        expected[2] = Op::IntBrIf { opcode: 0x46, depth: 1 };
        expected[5] = Op::IntIf { opcode: 0x45, end: 9, other: NO_ELSE, params: 0, results: 0 };
        expected[7] = Op::LocalUnary { local: 1, opcode: 0xa7 };
        expected[10] = Op::LocalWrapConstBin { local: 1, value: 3, opcode: 0x6c };
        assert_eq!(ops, expected);
    }

    /// Every group of up to four ops drawn from a mix of control and other
    /// ops: none starts with a control op, and only its last op may be one
    /// (an if or br_if after a computation).
    #[test]
    fn groups_hold_control_only_last() {
        let alphabet = [
            Op::LocalGet(0),
            Op::LocalSet(1),
            Op::I32Const(1),
            Op::I64Const(1),
            Op::Int(0x6a),
            Op::Int(0x7c),
            Op::Int(0xa7),
            Op::I32Eqz,
            Op::Memory0 { opcode: 0x28, offset: 0 },
            Op::Memory0 { opcode: 0x36, offset: 0 },
            Op::GlobalGet(0),
            Op::GlobalSet(0),
            Op::Block { end: 9, params: 0, results: 0 },
            Op::Loop { end: 9, params: 0, results: 0 },
            Op::If { end: 9, other: NO_ELSE, params: 0, results: 0 },
            Op::Else,
            Op::End,
            Op::Br(0),
            Op::BrIf(0),
            Op::Leave,
            Op::Slow,
        ];
        let control = |op: Op| {
            matches!(
                op,
                Op::Block { .. } | Op::Loop { .. } | Op::If { .. } | Op::Else | Op::End | Op::Br(_) | Op::BrIf(_) | Op::Leave | Op::Slow
            )
        };
        let n = alphabet.len();
        let mut groups = 0;
        for code in 0..n.pow(4) {
            let ops: Vec<Op> = (0..4).map(|i| alphabet[code / n.pow(i) % n]).collect();
            for (len, _) in candidates(&ops, 0).into_iter().flatten() {
                groups += 1;
                assert!(len >= 2 && len <= ops.len(), "{ops:?}");
                assert!(!control(ops[0]), "{ops:?}");
                assert!(ops[1..len - 1].iter().all(|&op| !control(op)), "{ops:?}");
                assert!(!control(ops[len - 1]) || matches!(ops[len - 1], Op::If { .. } | Op::BrIf(_)), "{ops:?}");
            }
        }
        assert!(groups > 1000);
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
