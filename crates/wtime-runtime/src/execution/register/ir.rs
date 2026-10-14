//! Syntax, admission and ownership checks for trusted register lowering.
//! These checks do not prove dataflow equivalence to Wasm; the lowering pass
//! must supply that proof and exact source-prefix tests before execution.
use crate::execution::effects::{Effect, classify};
use crate::{Immediate, Module, ValType};
use std::{mem::size_of, ops::Range, sync::Arc};

const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;
type Checked<T> = Result<T, Declined>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Declined {
    Unsupported,
    MissingPlan,
    Budget,
    Allocation,
    Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NumericType {
    I32,
    I64,
    F32,
    F64,
}
impl NumericType {
    pub(super) fn admitted(value: ValType) -> Checked<Self> {
        match value {
            ValType::I32 => Ok(Self::I32),
            ValType::I64 => Ok(Self::I64),
            ValType::F32 => Ok(Self::F32),
            ValType::F64 => Ok(Self::F64),
            _ => Err(Declined::Unsupported),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Constant {
    I32(u32),
    I64(u64),
    F32(u32),
    F64(u64),
}
impl Constant {
    fn ty(self) -> NumericType {
        match self {
            Self::I32(_) => NumericType::I32,
            Self::I64(_) => NumericType::I64,
            Self::F32(_) => NumericType::F32,
            Self::F64(_) => NumericType::F64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    Move {
        from: u32,
        to: u32,
        ty: NumericType,
    },
    Const {
        constant: u32,
        to: u32,
    },
    Unary {
        opcode: u16,
        from: u32,
        to: u32,
        input: NumericType,
        result: NumericType,
    },
    Binary {
        opcode: u16,
        left: u32,
        right: u32,
        to: u32,
        input: NumericType,
        result: NumericType,
    },
    Jump {
        target: u32,
    },
    Branch {
        condition: u32,
        yes: u32,
        no: u32,
    },
    // Defined for the later memory task; whole-function admission excludes it.
    Memory {
        argument: u32,
        address: u32,
        value: u32,
    },
    Exit {
        results: Range<u32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MemoryArgument {
    pub memory: u32,
    pub offset: u64,
    pub align: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StackSlot {
    pub slot: u32,
    pub ty: NumericType,
}

#[derive(Clone, Debug)]
pub(super) struct SourceEvent {
    pub source_pc: usize,
    pub opcode: u16,
    pub effect: Effect,
    pub fuel_charge: u8,
    pub cancellation_checkpoint: bool,
    /// Exact logical operand shape before this original instruction, not the
    /// physical slot count. Types at one temporary height may change over time.
    pub stack: Range<u32>,
    pub successors: Range<u32>,
}

/// Internal drafts can only be published against normal Wasm admission.
/// P058 must bound allocation while constructing these tables, and then use
/// the aggregate verifier below before retaining anything in a Module.
pub(super) struct Draft {
    pub operations: Vec<Operation>,
    pub events: Vec<SourceEvent>,
    pub stack_slots: Vec<StackSlot>,
    pub targets: Vec<u32>,
    pub constants: Vec<Constant>,
    pub memory: Vec<MemoryArgument>,
    pub results: Vec<u32>,
}

/// Shared across a module's preparations. Commit only after all validation;
/// allocation or verification failure leaves both budgets unchanged.
pub(super) struct Budget {
    work: usize,
    bytes: usize,
}
impl Budget {
    pub(super) fn new(work: usize, bytes: usize) -> Self {
        Self {
            work: work.min(MAX_WORK),
            bytes: bytes.min(MAX_BYTES),
        }
    }
}

pub(super) struct Admission {
    engine: u64,
    source: Arc<[u8]>,
    body: usize,
    type_index: usize,
    locals: Vec<NumericType>,
    results: Vec<NumericType>,
    stack_peak: usize,
    slots: u32,
    work: usize,
}
fn plus(a: usize, b: usize) -> Checked<usize> {
    a.checked_add(b).ok_or(Declined::Budget)
}
fn vector_bytes<T>(capacity: usize) -> Checked<usize> {
    capacity.checked_mul(size_of::<T>()).ok_or(Declined::Budget)
}
fn copy_types<'a>(
    types: impl Iterator<Item = &'a ValType>,
    count: usize,
) -> Checked<Vec<NumericType>> {
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| Declined::Allocation)?;
    for ty in types {
        out.push(NumericType::admitted(*ty)?);
    }
    Ok(out)
}
impl Admission {
    pub(super) fn local_types(&self) -> &[NumericType] {
        &self.locals
    }
    pub(super) fn slot_count(&self) -> u32 {
        self.slots
    }
    pub(super) fn inspect(module: &Module, body: usize, budget: &Budget) -> Checked<Self> {
        let plan = module
            .data
            .plans
            .as_ref()
            .and_then(|p| p.functions.get(body))
            .ok_or(Declined::MissingPlan)?;
        let layout = module.function_layout(body).ok_or(Declined::MissingPlan)?;
        let code = module
            .data
            .inner
            .decoded()
            .code
            .get(body)
            .ok_or(Declined::Metadata)?;
        let local_count = plus(layout.params.len(), layout.locals.len())?;
        let mut work = plus(
            plus(
                code.instructions
                    .len()
                    .checked_mul(4)
                    .ok_or(Declined::Budget)?,
                local_count,
            )?,
            layout.results.len(),
        )?;
        let slots =
            u32::try_from(plus(local_count, plan.stack_peak)?).map_err(|_| Declined::Budget)?;
        let bytes = plus(
            size_of::<Self>(),
            vector_bytes::<NumericType>(plus(local_count, layout.results.len())?)?,
        )?;
        if work > budget.work || bytes > budget.bytes {
            return Err(Declined::Budget);
        }
        // Decline unsupported types and effects before allocating metadata.
        for ty in layout
            .params
            .iter()
            .chain(layout.locals)
            .chain(layout.results)
        {
            NumericType::admitted(*ty)?;
        }
        for instruction in &code.instructions {
            if !matches!(instruction.opcode, 1..=5 | 0xb..=0xd | 0xf | 0x1a | 0x20..=0x22 | 0x41..=0xc4 | 0xfc00..=0xfc07)
            {
                return Err(Declined::Unsupported);
            }
            match &instruction.immediate {
                Immediate::BlockType(ty) => {
                    NumericType::admitted(*ty)?;
                }
                Immediate::Block(index) if *index >= 0 => {
                    let ty = module
                        .data
                        .inner
                        .decoded()
                        .types
                        .get(usize::try_from(*index).map_err(|_| Declined::Metadata)?)
                        .ok_or(Declined::Metadata)?;
                    work = plus(work, plus(ty.params.len(), ty.results.len())?)?;
                    if work > budget.work {
                        return Err(Declined::Budget);
                    }
                    for value in ty.params.iter().chain(&ty.results) {
                        NumericType::admitted(*value)?;
                    }
                }
                Immediate::Block(index) if !matches!(*index, -64 | -1 | -2 | -3 | -4) => {
                    return Err(Declined::Unsupported);
                }
                _ => {}
            }
        }
        let locals = copy_types(layout.params.iter().chain(layout.locals), local_count)?;
        let results = copy_types(layout.results.iter(), layout.results.len())?;
        let actual = plus(
            size_of::<Self>(),
            vector_bytes::<NumericType>(plus(locals.capacity(), results.capacity())?)?,
        )?;
        if actual > budget.bytes {
            return Err(Declined::Budget);
        }
        Ok(Self {
            engine: module.data.engine,
            source: module.data.inner.decoded().bytes.clone(),
            body,
            type_index: plan.type_index,
            locals,
            results,
            stack_peak: plan.stack_peak,
            slots,
            work,
        })
    }
    fn matches(&self, module: &Module) -> bool {
        self.engine == module.data.engine
            && Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
            && module
                .data
                .plans
                .as_ref()
                .and_then(|p| p.functions.get(self.body))
                .is_some_and(|p| p.type_index == self.type_index)
    }
}

/// Canonical operand/result classes and arity; no unchecked type annotations.
pub(super) fn numeric_signature(opcode: u16) -> Option<(NumericType, NumericType, bool)> {
    use NumericType::*;
    Some(match opcode {
        0x45 => (I32, I32, false),
        0x50 => (I64, I32, false),
        0x46..=0x4f => (I32, I32, true),
        0x51..=0x5a => (I64, I32, true),
        0x5b..=0x60 => (F32, I32, true),
        0x61..=0x66 => (F64, I32, true),
        0x67..=0x69 | 0xc0..=0xc1 => (I32, I32, false),
        0x6a..=0x78 => (I32, I32, true),
        0x79..=0x7b | 0xc2..=0xc4 => (I64, I64, false),
        0x7c..=0x8a => (I64, I64, true),
        0x8b..=0x91 => (F32, F32, false),
        0x92..=0x98 => (F32, F32, true),
        0x99..=0x9f => (F64, F64, false),
        0xa0..=0xa6 => (F64, F64, true),
        0xa7 => (I64, I32, false),
        0xa8..=0xa9 | 0xfc00..=0xfc01 => (F32, I32, false),
        0xaa..=0xab | 0xfc02..=0xfc03 => (F64, I32, false),
        0xac..=0xad => (I32, I64, false),
        0xae..=0xaf | 0xfc04..=0xfc05 => (F32, I64, false),
        0xb0..=0xb1 | 0xfc06..=0xfc07 => (F64, I64, false),
        0xb2..=0xb3 => (I32, F32, false),
        0xb4..=0xb5 => (I64, F32, false),
        0xb6 => (F64, F32, false),
        0xb7..=0xb8 => (I32, F64, false),
        0xb9..=0xba => (I64, F64, false),
        0xbb => (F32, F64, false),
        0xbc => (F32, I32, false),
        0xbd => (F64, I64, false),
        0xbe => (I32, F32, false),
        0xbf => (I64, F64, false),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct Label {
    start: u32,
    end: u32,
    loop_: bool,
}
fn branch(labels: &[Label], depth: u32, terminal: u32) -> Checked<u32> {
    let depth = usize::try_from(depth).map_err(|_| Declined::Metadata)?;
    if depth == labels.len() {
        return Ok(terminal);
    }
    let label = labels
        .get(
            labels
                .len()
                .checked_sub(plus(depth, 1)?)
                .ok_or(Declined::Metadata)?,
        )
        .ok_or(Declined::Metadata)?;
    if label.loop_ {
        Ok(label.start)
    } else {
        label.end.checked_add(1).ok_or(Declined::Metadata)
    }
}

/// Retains bytes, never Arc<ModuleData>, so future Module ownership is acyclic.
pub(super) struct RegisterFunction {
    admission: Admission,
    draft: Draft,
    retained_bytes: usize,
    work: usize,
}
/// Values are supplied by a later executor. This immutable recipe maps a
/// pending source boundary back to fixed locals and ordered logical operands.
pub(super) struct MaterializationRecipe<'a> {
    pub body: usize,
    pub source_pc: usize,
    pub prepared_pc: usize,
    pub locals: &'a [NumericType],
    pub operands: &'a [StackSlot],
}
impl RegisterFunction {
    pub(super) fn materialization(
        &self,
        module: &Module,
        prepared_pc: usize,
    ) -> Checked<MaterializationRecipe<'_>> {
        if !self.admission.matches(module) {
            return Err(Declined::Metadata);
        }
        let event = self
            .draft
            .events
            .get(prepared_pc)
            .ok_or(Declined::Metadata)?;
        let operands = self
            .draft
            .stack_slots
            .get(event.stack.start as usize..event.stack.end as usize)
            .ok_or(Declined::Metadata)?;
        Ok(MaterializationRecipe {
            body: self.admission.body,
            source_pc: event.source_pc,
            prepared_pc,
            locals: &self.admission.locals,
            operands,
        })
    }
    pub(super) fn from_draft(
        module: &Module,
        admission: Admission,
        draft: Draft,
        budget: &mut Budget,
    ) -> Checked<Self> {
        if !admission.matches(module) {
            return Err(Declined::Metadata);
        }
        let code = &module.data.inner.decoded().code[admission.body];
        let n = code.instructions.len();
        if draft.events.len() != n || draft.operations.len() != n || !draft.memory.is_empty() {
            return Err(Declined::Metadata);
        }
        let mut bytes = plus(
            size_of::<Self>(),
            vector_bytes::<NumericType>(plus(
                admission.locals.capacity(),
                admission.results.capacity(),
            )?)?,
        )?;
        for capacity in [
            vector_bytes::<Operation>(draft.operations.capacity())?,
            vector_bytes::<SourceEvent>(draft.events.capacity())?,
            vector_bytes::<StackSlot>(draft.stack_slots.capacity())?,
            vector_bytes::<u32>(draft.targets.capacity())?,
            vector_bytes::<Constant>(draft.constants.capacity())?,
            vector_bytes::<MemoryArgument>(draft.memory.capacity())?,
            vector_bytes::<u32>(draft.results.capacity())?,
        ] {
            bytes = plus(bytes, capacity)?;
        }
        let work = plus(
            admission.work,
            plus(
                n.checked_mul(4).ok_or(Declined::Budget)?,
                plus(
                    plus(draft.stack_slots.len(), draft.targets.len())?,
                    plus(draft.constants.len(), draft.results.len())?,
                )?,
            )?,
        )?;
        // Include temporary control labels in the preparation peak.
        let peak = plus(bytes, vector_bytes::<Label>(n)?)?;
        if work > budget.work || peak > budget.bytes {
            return Err(Declined::Budget);
        }
        let terminal = u32::try_from(n).map_err(|_| Declined::Budget)?;
        let mut labels = Vec::new();
        labels
            .try_reserve_exact(n)
            .map_err(|_| Declined::Allocation)?;
        if plus(bytes, vector_bytes::<Label>(labels.capacity())?)? > budget.bytes {
            return Err(Declined::Budget);
        }
        let mut stack_end = 0u32;
        let mut edge_end = 0u32;
        for (pc, ((event, operation), source)) in draft
            .events
            .iter()
            .zip(&draft.operations)
            .zip(&code.instructions)
            .enumerate()
        {
            if event.source_pc != pc
                || event.opcode != source.opcode
                || event.effect != classify(source.opcode)
                || event.fuel_charge != 1
                || !event.cancellation_checkpoint
                || event.stack.start != stack_end
                || event.successors.start != edge_end
            {
                return Err(Declined::Metadata);
            }
            let stack = draft
                .stack_slots
                .get(event.stack.start as usize..event.stack.end as usize)
                .ok_or(Declined::Metadata)?;
            let targets = draft
                .targets
                .get(event.successors.start as usize..event.successors.end as usize)
                .ok_or(Declined::Metadata)?;
            if stack.len() > admission.stack_peak {
                return Err(Declined::Metadata);
            }
            for (height, slot) in stack.iter().enumerate() {
                if usize::try_from(slot.slot).map_err(|_| Declined::Metadata)?
                    != plus(admission.locals.len(), height)?
                {
                    return Err(Declined::Metadata);
                }
            }
            let next = u32::try_from(pc + 1).map_err(|_| Declined::Budget)?;
            let expected: [Option<u32>; 2] = match source.opcode {
                2..=4 => {
                    let (end, other) = module
                        .data
                        .control
                        .get(admission.body)
                        .and_then(|v| v.get(pc))
                        .copied()
                        .flatten()
                        .ok_or(Declined::Metadata)?;
                    let end = u32::try_from(end).map_err(|_| Declined::Budget)?;
                    labels.push(Label {
                        start: next,
                        end,
                        loop_: source.opcode == 3,
                    });
                    [
                        Some(next),
                        if source.opcode == 4 {
                            Some(
                                u32::try_from(other.map_or(end as usize, |x| x + 1))
                                    .map_err(|_| Declined::Budget)?,
                            )
                        } else {
                            None
                        },
                    ]
                }
                5 => [Some(labels.last().ok_or(Declined::Metadata)?.end), None],
                0xb => {
                    labels.pop();
                    [Some(next), None]
                }
                0xc | 0xd => {
                    let Immediate::Index(depth) = source.immediate else {
                        return Err(Declined::Metadata);
                    };
                    let target = branch(&labels, depth, terminal)?;
                    if source.opcode == 0xd {
                        [Some(target), Some(next)]
                    } else {
                        [Some(target), None]
                    }
                }
                0xf => [Some(terminal), None],
                _ => [Some(next), None],
            };
            if targets.iter().copied().ne(expected.into_iter().flatten())
                || targets.iter().any(|x| *x > terminal)
            {
                return Err(Declined::Metadata);
            }
            let input = |slot: u32, ty: NumericType| {
                if let Some(local) = admission.locals.get(slot as usize) {
                    *local == ty
                } else {
                    stack
                        .iter()
                        .any(|value| value.slot == slot && value.ty == ty)
                }
            };
            let output = |slot: u32, ty: NumericType| {
                slot < admission.slots
                    && admission
                        .locals
                        .get(slot as usize)
                        .is_none_or(|local| *local == ty)
            };
            let valid = match operation {
                Operation::Move { from, to, ty } => {
                    let Immediate::Index(local) = source.immediate else {
                        return Err(Declined::Metadata);
                    };
                    let shape = match source.opcode {
                        0x20 => {
                            *from == local
                                && *to as usize == plus(admission.locals.len(), stack.len())?
                        }
                        0x21 | 0x22 => {
                            *to == local && stack.last().is_some_and(|top| top.slot == *from)
                        }
                        _ => false,
                    };
                    shape && input(*from, *ty) && output(*to, *ty)
                }
                Operation::Const { constant, to } => {
                    draft.constants.get(*constant as usize).is_some_and(|c| {
                        let original = match source.immediate {
                            Immediate::I32(v) => Some(Constant::I32(v as u32)),
                            Immediate::I64(v) => Some(Constant::I64(v as u64)),
                            Immediate::F32(v) => Some(Constant::F32(v)),
                            Immediate::F64(v) => Some(Constant::F64(v)),
                            _ => None,
                        };
                        original == Some(*c)
                            && *to as usize == admission.locals.len() + stack.len()
                            && output(*to, c.ty())
                    })
                }
                Operation::Unary {
                    opcode,
                    from,
                    to,
                    input: i,
                    result,
                } => {
                    *opcode == source.opcode
                        && numeric_signature(*opcode) == Some((*i, *result, false))
                        && stack
                            .last()
                            .is_some_and(|top| top.slot == *from && *to == *from)
                        && input(*from, *i)
                        && output(*to, *result)
                }
                Operation::Binary {
                    opcode,
                    left,
                    right,
                    to,
                    input: i,
                    result,
                } => {
                    *opcode == source.opcode
                        && numeric_signature(*opcode) == Some((*i, *result, true))
                        && stack.len().checked_sub(2).is_some_and(|height| {
                            stack[height].slot == *left
                                && stack[height + 1].slot == *right
                                && *to == *left
                        })
                        && input(*left, *i)
                        && input(*right, *i)
                        && output(*to, *result)
                }
                Operation::Jump { target } => {
                    matches!(source.opcode, 1..=3 | 5 | 0xb | 0xc | 0x1a)
                        && pc + 1 < n
                        && targets == [*target]
                }
                Operation::Branch { condition, yes, no } => {
                    matches!(source.opcode, 4 | 0xd)
                        && stack.last().is_some_and(|top| top.slot == *condition)
                        && input(*condition, NumericType::I32)
                        && targets == [*yes, *no]
                }
                Operation::Memory { .. } => false,
                Operation::Exit { results } => {
                    (source.opcode == 0xf || pc + 1 == n && source.opcode == 0xb)
                        && results.start <= results.end
                        && draft
                            .results
                            .get(results.start as usize..results.end as usize)
                            .is_some_and(|values| {
                                values.len() == admission.results.len()
                                    && values
                                        .iter()
                                        .zip(&admission.results)
                                        .all(|(slot, ty)| input(*slot, *ty))
                            })
                }
            };
            if !valid {
                return Err(Declined::Metadata);
            }
            stack_end = event.stack.end;
            edge_end = event.successors.end;
        }
        if stack_end as usize != draft.stack_slots.len()
            || edge_end as usize != draft.targets.len()
            || !labels.is_empty()
        {
            return Err(Declined::Metadata);
        }
        budget.work -= work;
        budget.bytes -= bytes;
        Ok(Self {
            admission,
            draft,
            retained_bytes: bytes,
            work,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Engine};
    include!("../../../tests/performance/p057_private.rs");
}
