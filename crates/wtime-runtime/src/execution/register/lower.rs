//! Bounded typed stack-to-slot translation. Edge values are simultaneous move
//! requests, not an execution schedule; P060 owns their scheduling.
use super::ir::*;
use crate::execution::effects::classify;
use crate::{Immediate, Module, ValType};
use std::mem::size_of;
type Checked<T> = Result<T, Declined>;
fn add(a: usize, b: usize) -> Checked<usize> {
    a.checked_add(b).ok_or(Declined::Budget)
}
fn mul(a: usize, b: usize) -> Checked<usize> {
    a.checked_mul(b).ok_or(Declined::Budget)
}
fn narrow(n: usize) -> Checked<u32> {
    u32::try_from(n).map_err(|_| Declined::Budget)
}
fn reserved<T>(n: usize) -> Checked<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Declined::Allocation)?;
    Ok(v)
}
fn push_type(stack: &mut Vec<NumericType>, ty: NumericType, peak: usize) -> Checked<()> {
    if stack.len() >= peak {
        return Err(Declined::Metadata);
    }
    stack.push(ty);
    Ok(())
}
fn replace_shape(
    stack: &mut Vec<NumericType>,
    prefix: &[NumericType],
    values: &[NumericType],
    peak: usize,
) -> Checked<()> {
    if add(prefix.len(), values.len())? > peak {
        return Err(Declined::Metadata);
    }
    stack.clear();
    stack.extend_from_slice(prefix);
    stack.extend_from_slice(values);
    Ok(())
}
fn one_type(ty: NumericType) -> Checked<Vec<NumericType>> {
    let mut values = reserved(1)?;
    values.push(ty);
    Ok(values)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EdgeMove {
    pub from: u32,
    pub to: u32,
    pub ty: NumericType,
}
pub(super) struct EdgeRequest {
    pub source_pc: usize,
    pub target: usize,
    pub moves: Vec<EdgeMove>,
}
pub(super) struct LoweredFunction {
    pub program: RegisterFunction,
    pub edges: Vec<EdgeRequest>,
    pub block_entries: Vec<usize>,
    pub work: usize,
    pub retained_bytes: usize,
}
struct Label {
    start: usize,
    end: usize,
    loop_: bool,
    prefix: Vec<NumericType>,
    params: Vec<NumericType>,
    results: Vec<NumericType>,
    parent_dead: bool,
}
fn signature(
    module: &Module,
    immediate: &Immediate,
    peak: usize,
) -> Checked<(Vec<NumericType>, Vec<NumericType>)> {
    let converted = |types: &[ValType]| -> Checked<Vec<NumericType>> {
        if types.len() > peak {
            return Err(Declined::Metadata);
        }
        let mut out = reserved(types.len())?;
        for ty in types {
            out.push(NumericType::admitted(*ty)?);
        }
        Ok(out)
    };
    match immediate {
        Immediate::Block(-64) => Ok((vec![], vec![])),
        Immediate::BlockType(ty) => Ok((vec![], one_type(NumericType::admitted(*ty)?)?)),
        Immediate::Block(index) if *index < 0 => {
            let ty = match index {
                -1 => NumericType::I32,
                -2 => NumericType::I64,
                -3 => NumericType::F32,
                -4 => NumericType::F64,
                _ => return Err(Declined::Unsupported),
            };
            Ok((vec![], one_type(ty)?))
        }
        Immediate::Block(index) => {
            let ty = module
                .data
                .inner
                .decoded()
                .types
                .get(usize::try_from(*index).map_err(|_| Declined::Metadata)?)
                .ok_or(Declined::Metadata)?;
            Ok((converted(&ty.params)?, converted(&ty.results)?))
        }
        _ => Err(Declined::Unsupported),
    }
}
fn tail(stack: &[NumericType], expected: &[NumericType]) -> Checked<usize> {
    let start = stack
        .len()
        .checked_sub(expected.len())
        .ok_or(Declined::Metadata)?;
    if &stack[start..] != expected {
        return Err(Declined::Metadata);
    }
    Ok(start)
}
fn pop(stack: &mut Vec<NumericType>, expected: NumericType) -> Checked<usize> {
    if stack.pop() != Some(expected) {
        return Err(Declined::Metadata);
    }
    Ok(stack.len())
}

impl LoweredFunction {
    pub(super) fn lower(module: &Module, body: usize, budget: &mut Budget) -> Checked<Self> {
        let mut transaction = budget.clone();
        let admission = Admission::inspect(module, body, &transaction)?;
        let locals = admission.local_types().len();
        let peak = admission.slot_count() as usize - locals;
        let layout = module.function_layout(body).ok_or(Declined::MissingPlan)?;
        let code = &module
            .data
            .inner
            .decoded()
            .code
            .get(body)
            .ok_or(Declined::Metadata)?
            .instructions;
        let n = code.len();
        let pool = mul(n, peak)?;
        // Conservative allocation peak, including label signatures/prefixes,
        // both branch edges, result requests and the immutable format tables.
        let per_instruction = size_of::<Operation>()
            + size_of::<SourceEvent>()
            + size_of::<Label>()
            + 2 * size_of::<EdgeRequest>()
            + size_of::<Constant>()
            + 2 * size_of::<u32>()
            + 2 * size_of::<usize>()
            + size_of::<(u32, u32, bool)>();
        let per_height = size_of::<StackSlot>()
            + size_of::<u32>()
            + 2 * size_of::<EdgeMove>()
            + 3 * size_of::<NumericType>();
        let planned_tables = add(
            add(mul(n, per_instruction)?, mul(pool, per_height)?)?,
            size_of::<Self>(),
        )?;
        let planned = add(
            planned_tables,
            mul(
                add(mul(locals, 2)?, add(mul(layout.results.len(), 2)?, peak)?)?,
                size_of::<NumericType>(),
            )?,
        )?;
        let analysis_work = add(mul(n, 8)?, mul(pool, 4)?)?;
        if planned > transaction.remaining().1 || analysis_work > transaction.remaining().0 {
            return Err(Declined::Budget);
        }
        let mut local_types = reserved(locals)?;
        local_types.extend_from_slice(admission.local_types());
        let mut returns = reserved(layout.results.len())?;
        for ty in layout.results {
            returns.push(NumericType::admitted(*ty)?);
        }
        let mut draft = Draft {
            operations: reserved(n)?,
            events: reserved(n)?,
            stack_slots: reserved(pool)?,
            targets: reserved(mul(n, 2)?)?,
            constants: reserved(n)?,
            memory: vec![],
            results: reserved(pool)?,
        };
        let mut stack = reserved(peak)?;
        let mut labels: Vec<Label> = reserved(n)?;
        let mut edges: Vec<EdgeRequest> = reserved(mul(n, 2)?)?;
        let mut block_entries = reserved(mul(n, 2)?)?;
        let mut dead = false;
        for (pc, instruction) in code.iter().enumerate() {
            // Unreachable structural ends have a declared polymorphic result
            // shape. No executable edge observes uninitialized virtual values.
            if instruction.opcode == 0xb && dead {
                if let Some(label) = labels.last() {
                    replace_shape(&mut stack, &label.prefix, &label.results, peak)?;
                } else {
                    replace_shape(&mut stack, &[], &returns, peak)?;
                }
            }
            if stack.len() > peak {
                return Err(Declined::Metadata);
            }
            let stack_start = narrow(draft.stack_slots.len())?;
            for (height, ty) in stack.iter().enumerate() {
                draft.stack_slots.push(StackSlot {
                    slot: narrow(add(locals, height)?)?,
                    ty: *ty,
                });
            }
            let stack_end = narrow(draft.stack_slots.len())?;
            let edge_start = narrow(draft.targets.len())?;
            let mut targets = [pc + 1, 0];
            let mut target_count = 1;
            let operation = match (&instruction.immediate, instruction.opcode) {
                (Immediate::Index(index), 0x20) => {
                    let ty = *local_types.get(*index as usize).ok_or(Declined::Metadata)?;
                    let to = narrow(add(locals, stack.len())?)?;
                    push_type(&mut stack, ty, peak)?;
                    Operation::Move {
                        from: *index,
                        to,
                        ty,
                    }
                }
                (Immediate::Index(index), 0x21 | 0x22) => {
                    let ty = *local_types.get(*index as usize).ok_or(Declined::Metadata)?;
                    let height = pop(&mut stack, ty)?;
                    let from = narrow(add(locals, height)?)?;
                    if instruction.opcode == 0x22 {
                        push_type(&mut stack, ty, peak)?;
                    }
                    Operation::Move {
                        from,
                        to: *index,
                        ty,
                    }
                }
                (_, 0x41..=0x44) => {
                    let (constant, ty) = match instruction.immediate {
                        Immediate::I32(v) => (Constant::I32(v as u32), NumericType::I32),
                        Immediate::I64(v) => (Constant::I64(v as u64), NumericType::I64),
                        Immediate::F32(v) => (Constant::F32(v), NumericType::F32),
                        Immediate::F64(v) => (Constant::F64(v), NumericType::F64),
                        _ => return Err(Declined::Metadata),
                    };
                    let to = narrow(add(locals, stack.len())?)?;
                    push_type(&mut stack, ty, peak)?;
                    let constant_index = narrow(draft.constants.len())?;
                    draft.constants.push(constant);
                    Operation::Const {
                        constant: constant_index,
                        to,
                    }
                }
                (_, 0x45..=0xc4 | 0xfc00..=0xfc07) => {
                    let (input, result, binary) =
                        numeric_signature(instruction.opcode).ok_or(Declined::Unsupported)?;
                    let right = narrow(add(locals, pop(&mut stack, input)?)?)?;
                    let operation = if binary {
                        let left = narrow(add(locals, pop(&mut stack, input)?)?)?;
                        Operation::Binary {
                            opcode: instruction.opcode,
                            left,
                            right,
                            to: left,
                            input,
                            result,
                        }
                    } else {
                        Operation::Unary {
                            opcode: instruction.opcode,
                            from: right,
                            to: right,
                            input,
                            result,
                        }
                    };
                    push_type(&mut stack, result, peak)?;
                    operation
                }
                (_, 1) => Operation::Jump {
                    target: narrow(pc + 1)?,
                },
                (_, 0x1a) => {
                    stack.pop().ok_or(Declined::Metadata)?;
                    Operation::Jump {
                        target: narrow(pc + 1)?,
                    }
                }
                (_, 2..=4) => {
                    let condition = if instruction.opcode == 4 {
                        Some(narrow(add(locals, pop(&mut stack, NumericType::I32)?)?)?)
                    } else {
                        None
                    };
                    let (params, results) = signature(module, &instruction.immediate, peak)?;
                    let base = tail(&stack, &params)?;
                    let (end, otherwise) = module
                        .data
                        .control
                        .get(body)
                        .and_then(|v| v.get(pc))
                        .copied()
                        .flatten()
                        .ok_or(Declined::Metadata)?;
                    if end <= pc
                        || end >= n
                        || code[end].opcode != 0xb
                        || otherwise.is_some_and(|other| {
                            instruction.opcode != 4
                                || other <= pc
                                || other >= end
                                || code[other].opcode != 5
                        })
                    {
                        return Err(Declined::Metadata);
                    }
                    let mut prefix = reserved(base)?;
                    prefix.extend(&stack[..base]);
                    labels.push(Label {
                        start: pc + 1,
                        end,
                        loop_: instruction.opcode == 3,
                        prefix,
                        params,
                        results,
                        parent_dead: dead,
                    });
                    block_entries.push(pc + 1);
                    if let Some(condition) = condition {
                        let no = otherwise.map_or(end, |v| v + 1);
                        targets[1] = no;
                        target_count = 2;
                        block_entries.push(no);
                        Operation::Branch {
                            condition,
                            yes: narrow(pc + 1)?,
                            no: narrow(no)?,
                        }
                    } else {
                        Operation::Jump {
                            target: narrow(pc + 1)?,
                        }
                    }
                }
                (_, 5) => {
                    let label = labels.last().ok_or(Declined::Metadata)?;
                    if !dead
                        && (stack.len() != label.prefix.len() + label.results.len()
                            || tail(&stack, &label.results)? != label.prefix.len())
                    {
                        return Err(Declined::Metadata);
                    }
                    targets[0] = label.end;
                    replace_shape(&mut stack, &label.prefix, &label.params, peak)?;
                    dead = label.parent_dead;
                    Operation::Jump {
                        target: narrow(label.end)?,
                    }
                }
                (Immediate::Index(depth), 0xc | 0xd) => {
                    let condition = if instruction.opcode == 0xd {
                        Some(narrow(add(locals, pop(&mut stack, NumericType::I32)?)?)?)
                    } else {
                        None
                    };
                    let depth = *depth as usize;
                    let (target, base, args) = if depth == labels.len() {
                        (n, 0, returns.as_slice())
                    } else {
                        let index = labels
                            .len()
                            .checked_sub(add(depth, 1)?)
                            .ok_or(Declined::Metadata)?;
                        let label = &labels[index];
                        (
                            if label.loop_ {
                                label.start
                            } else {
                                label.end + 1
                            },
                            label.prefix.len(),
                            if label.loop_ {
                                label.params.as_slice()
                            } else {
                                label.results.as_slice()
                            },
                        )
                    };
                    let from = tail(&stack, args)?;
                    let mut moves = reserved(args.len())?;
                    for (index, ty) in args.iter().enumerate() {
                        moves.push(EdgeMove {
                            from: narrow(add(locals, from + index)?)?,
                            to: narrow(add(locals, base + index)?)?,
                            ty: *ty,
                        });
                    }
                    edges.push(EdgeRequest {
                        source_pc: pc,
                        target,
                        moves,
                    });
                    targets[0] = target;
                    if let Some(condition) = condition {
                        targets[1] = pc + 1;
                        target_count = 2;
                        Operation::Branch {
                            condition,
                            yes: narrow(target)?,
                            no: narrow(pc + 1)?,
                        }
                    } else {
                        dead = true;
                        Operation::Jump {
                            target: narrow(target)?,
                        }
                    }
                }
                (_, 0xb) if !labels.is_empty() => {
                    let label = labels.pop().ok_or(Declined::Metadata)?;
                    if pc != label.end
                        || stack.len() != label.prefix.len() + label.results.len()
                        || tail(&stack, &label.results)? != label.prefix.len()
                    {
                        return Err(Declined::Metadata);
                    }
                    dead = label.parent_dead;
                    Operation::Jump {
                        target: narrow(pc + 1)?,
                    }
                }
                (_, 0xb | 0xf) => {
                    let start = tail(&stack, &returns)?;
                    let first = narrow(draft.results.len())?;
                    for height in start..stack.len() {
                        draft.results.push(narrow(add(locals, height)?)?);
                    }
                    targets[0] = n;
                    if instruction.opcode == 0xf {
                        dead = true;
                    }
                    Operation::Exit {
                        results: first..narrow(draft.results.len())?,
                    }
                }
                _ => return Err(Declined::Unsupported),
            };
            if stack.len() > peak || draft.stack_slots.len() > pool || draft.results.len() > pool {
                return Err(Declined::Metadata);
            }
            for target in &targets[..target_count] {
                draft.targets.push(narrow(*target)?);
            }
            draft.events.push(SourceEvent {
                source_pc: pc,
                opcode: instruction.opcode,
                effect: classify(instruction.opcode),
                fuel_charge: 1,
                cancellation_checkpoint: true,
                stack: stack_start..stack_end,
                successors: edge_start..narrow(draft.targets.len())?,
            });
            draft.operations.push(operation);
        }
        if !labels.is_empty() {
            return Err(Declined::Metadata);
        }
        let mut extra_bytes = add(
            size_of::<Self>() - size_of::<RegisterFunction>(),
            add(
                mul(edges.capacity(), size_of::<EdgeRequest>())?,
                mul(block_entries.capacity(), size_of::<usize>())?,
            )?,
        )?;
        for edge in &edges {
            extra_bytes = add(
                extra_bytes,
                mul(edge.moves.capacity(), size_of::<EdgeMove>())?,
            )?;
        }
        let program = RegisterFunction::from_draft(module, admission, draft, &mut transaction)?;
        transaction.reserve(analysis_work, extra_bytes)?;
        let (program_work, program_bytes) = program.costs();
        let work = add(program_work, analysis_work)?;
        let retained_bytes = add(program_bytes, extra_bytes)?;
        *budget = transaction;
        Ok(Self {
            program,
            edges,
            block_entries,
            work,
            retained_bytes,
        })
    }
    pub(super) fn edge(&self, pc: usize, target: usize) -> Option<&EdgeRequest> {
        self.edges
            .iter()
            .find(|edge| edge.source_pc == pc && edge.target == target)
    }
}

#[cfg(all(test, feature = "execution-reference"))]
mod tests {
    use super::*;
    use crate::*;
    include!("../../../tests/performance/p058_private.rs");
}
