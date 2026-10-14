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
    /// Metadata for a pending original opcode; never an executed guest return.
    ExitToDecoded,
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
    Table {
        selector: u32,
        targets: Range<u32>,
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
#[derive(Clone)]
pub(super) struct Budget {
    work: usize,
    bytes: usize,
}
impl Budget {
    pub(super) fn remaining(&self) -> (usize, usize) {
        (self.work, self.bytes)
    }
    pub(super) fn reserve(&mut self, work: usize, bytes: usize) -> Checked<()> {
        if work > self.work || bytes > self.bytes {
            return Err(Declined::Budget);
        }
        self.work -= work;
        self.bytes -= bytes;
        Ok(())
    }
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
        Self::inspect_range(module, body, budget, None)
    }
    pub(super) fn inspect_prefix(
        module: &Module,
        body: usize,
        budget: &Budget,
        end: usize,
    ) -> Checked<Self> {
        Self::inspect_range(module, body, budget, Some(end))
    }
    fn inspect_range(
        module: &Module,
        body: usize,
        budget: &Budget,
        prefix: Option<usize>,
    ) -> Checked<Self> {
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
        if prefix.is_some_and(|end| end == 0 || end >= code.instructions.len()) {
            return Err(Declined::Metadata);
        }
        let source_count = prefix.map_or(code.instructions.len(), |end| end + 1);
        let mut work = plus(
            plus(
                source_count.checked_mul(4).ok_or(Declined::Budget)?,
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
        for instruction in code
            .instructions
            .iter()
            .take(prefix.unwrap_or(code.instructions.len()))
        {
            if !matches!(instruction.opcode, 1..=5 | 0xb..=0xf | 0x1a | 0x20..=0x22 | 0x28..=0x3e | 0x41..=0xc4 | 0xfc00..=0xfc07)
            {
                return Err(Declined::Unsupported);
            }
            match &instruction.immediate {
                Immediate::MemArg { memory, .. } => {
                    work = plus(work, module.data.inner.decoded().imports.len())?;
                    if work > budget.work {
                        return Err(Declined::Budget);
                    }
                    if !super::memory::declared(module, *memory)
                        .is_some_and(super::memory::eligible)
                    {
                        return Err(Declined::Unsupported);
                    }
                }
                Immediate::BrTable(targets, _) => {
                    work = plus(work, plus(targets.len(), 1)?)?;
                    if work > budget.work {
                        return Err(Declined::Budget);
                    }
                }
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
    pub(super) fn slots(&self) -> u32 {
        self.admission.slot_count()
    }
    pub(super) fn operation(&self, pc: usize) -> Option<&Operation> {
        self.draft.operations.get(pc)
    }
    pub(super) fn constant(&self, index: u32) -> Option<Constant> {
        self.draft.constants.get(index as usize).copied()
    }
    pub(super) fn memory_argument(&self, index: u32) -> Option<&MemoryArgument> {
        self.draft.memory.get(index as usize)
    }
    pub(super) fn memory_arguments(&self) -> &[MemoryArgument] {
        &self.draft.memory
    }
    pub(super) fn result_slots(&self, range: Range<u32>) -> Option<&[u32]> {
        self.draft
            .results
            .get(range.start as usize..range.end as usize)
    }
    pub(super) fn costs(&self) -> (usize, usize) {
        (self.work, self.retained_bytes)
    }
    #[cfg(test)]
    pub(super) fn p131_revalidate(
        self,
        module: &Module,
        end: usize,
        corruption: usize,
        budget: &mut Budget,
    ) -> Checked<Self> {
        let Self {
            admission,
            mut draft,
            ..
        } = self;
        match corruption {
            0 => {
                draft.operations[end] = Operation::Jump {
                    target: end as u32 + 1,
                }
            }
            1 => draft.events[end].source_pc = end - 1,
            2 => {
                let slot = draft.events[end].stack.start as usize;
                draft.stack_slots[slot].ty = NumericType::F64;
            }
            3 => {
                draft.events.remove(0);
            }
            4 => {
                let Operation::Const { to, .. } = &mut draft.operations[0] else {
                    return Err(Declined::Metadata);
                };
                *to = u32::MAX;
            }
            5 => draft.events[end].fuel_charge = 0,
            6 => return Self::from_draft(module, admission, draft, budget),
            _ => {}
        }
        Self::from_prefix_draft(module, admission, draft, budget, end)
    }
    pub(super) fn targets(&self, range: Range<u32>) -> Option<&[u32]> {
        self.draft
            .targets
            .get(range.start as usize..range.end as usize)
    }
    pub(super) fn opcode(&self, pc: usize) -> Option<u16> {
        self.draft.events.get(pc).map(|event| event.opcode)
    }
    pub(super) fn materialization(
        &self,
        module: &Module,
        prepared_pc: usize,
    ) -> Checked<MaterializationRecipe<'_>> {
        if !self.admission.matches(module) {
            return Err(Declined::Metadata);
        }
        self.boundary(prepared_pc)
    }
    /// Private entry certificate: activate() checks the immutable source owner.
    /// No-call whole functions cannot replace module/context during execution.
    pub(super) fn boundary(&self, prepared_pc: usize) -> Checked<MaterializationRecipe<'_>> {
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
        Self::check_draft(module, admission, draft, budget, None)
    }
    #[allow(dead_code)] // P064 consumes this separately admitted artifact.
    pub(super) fn from_prefix_draft(
        module: &Module,
        admission: Admission,
        draft: Draft,
        budget: &mut Budget,
        end: usize,
    ) -> Checked<Self> {
        Self::check_draft(module, admission, draft, budget, Some(end))
    }
    fn check_draft(
        module: &Module,
        admission: Admission,
        draft: Draft,
        budget: &mut Budget,
        prefix: Option<usize>,
    ) -> Checked<Self> {
        if !admission.matches(module) {
            return Err(Declined::Metadata);
        }
        let code = &module.data.inner.decoded().code[admission.body];
        if prefix.is_some_and(|end| end == 0 || end >= code.instructions.len()) {
            return Err(Declined::Metadata);
        }
        let n = prefix.map_or(code.instructions.len(), |end| end + 1);
        if draft.events.len() != n || draft.operations.len() != n {
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
        let work = plus(
            work,
            draft
                .memory
                .len()
                .checked_mul(module.data.inner.decoded().imports.len())
                .ok_or(Declined::Budget)?,
        )?;
        // Include temporary control labels in the preparation peak.
        let work = plus(
            work,
            if prefix.is_some() {
                draft.stack_slots.len()
            } else {
                0
            },
        )?;
        let shape_bytes = if prefix.is_some() {
            vector_bytes::<NumericType>(admission.stack_peak)?
        } else {
            0
        };
        let peak = plus(plus(bytes, vector_bytes::<Label>(n)?)?, shape_bytes)?;
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
        let mut prefix_shape = Vec::new();
        if prefix.is_some() {
            prefix_shape
                .try_reserve_exact(admission.stack_peak)
                .map_err(|_| Declined::Allocation)?;
            if plus(
                plus(bytes, vector_bytes::<Label>(labels.capacity())?)?,
                vector_bytes::<NumericType>(prefix_shape.capacity())?,
            )? > budget.bytes
            {
                return Err(Declined::Budget);
            }
        }
        let mut edge_end = 0u32;
        let mut memory_end = 0usize;
        for (pc, ((event, operation), source)) in draft
            .events
            .iter()
            .zip(&draft.operations)
            .zip(&code.instructions)
            .enumerate()
        {
            let barrier = prefix == Some(pc);
            if barrier != matches!(operation, Operation::ExitToDecoded)
                || prefix.is_some() && !barrier && !prefix_instruction(module, source)
            {
                return Err(Declined::Metadata);
            }
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
            if prefix.is_some()
                && (stack.len() != prefix_shape.len()
                    || stack
                        .iter()
                        .zip(&prefix_shape)
                        .any(|(slot, ty)| slot.ty != *ty))
            {
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
            let expected: [Option<u32>; 2] = if barrier {
                [None, None]
            } else {
                match source.opcode {
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
                    0xe => {
                        let Immediate::BrTable(depths, default) = &source.immediate else {
                            return Err(Declined::Metadata);
                        };
                        if targets.len() != plus(depths.len(), 1)? {
                            return Err(Declined::Metadata);
                        }
                        for (target, depth) in targets
                            .iter()
                            .zip(depths.iter().chain(std::iter::once(default)))
                        {
                            if *target != branch(&labels, *depth, terminal)? {
                                return Err(Declined::Metadata);
                            }
                        }
                        [None, None]
                    }
                    0xf => [Some(terminal), None],
                    _ => [Some(next), None],
                }
            };
            if ((source.opcode != 0xe || barrier)
                && targets.iter().copied().ne(expected.into_iter().flatten()))
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
                Operation::ExitToDecoded => barrier && targets.is_empty(),
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
                Operation::Table {
                    selector,
                    targets: range,
                } => {
                    source.opcode == 0xe
                        && *range == event.successors
                        && stack.last().is_some_and(|top| top.slot == *selector)
                        && input(*selector, NumericType::I32)
                        && !targets.is_empty()
                }
                Operation::Memory {
                    argument,
                    address,
                    value,
                } => {
                    let Some((ty, _, store)) = super::memory::signature(source.opcode) else {
                        return Err(Declined::Metadata);
                    };
                    let Immediate::MemArg {
                        memory,
                        offset,
                        align,
                    } = source.immediate
                    else {
                        return Err(Declined::Metadata);
                    };
                    let arg = draft
                        .memory
                        .get(*argument as usize)
                        .ok_or(Declined::Metadata)?;
                    let identity = *argument as usize == memory_end
                        && *arg
                            == MemoryArgument {
                                memory,
                                offset,
                                align,
                            }
                        && super::memory::declared(module, memory)
                            .is_some_and(super::memory::eligible);
                    memory_end = plus(memory_end, 1)?;
                    identity
                        && if store {
                            stack.len().checked_sub(2).is_some_and(|height| {
                                stack[height].slot == *address
                                    && stack[height + 1].slot == *value
                                    && input(*address, NumericType::I32)
                                    && input(*value, ty)
                            })
                        } else {
                            stack
                                .last()
                                .is_some_and(|top| top.slot == *address && *address == *value)
                                && input(*address, NumericType::I32)
                                && output(*value, ty)
                        }
                }
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
            if prefix.is_some() && !barrier {
                match source.opcode {
                    1..=3 | 0xb | 0x22 => {}
                    0x1a | 0x21 => {
                        prefix_shape.pop().ok_or(Declined::Metadata)?;
                    }
                    0x20 => {
                        let Immediate::Index(index) = source.immediate else {
                            return Err(Declined::Metadata);
                        };
                        prefix_shape.push(
                            *admission
                                .locals
                                .get(index as usize)
                                .ok_or(Declined::Metadata)?,
                        );
                    }
                    0x41..=0x44 => {
                        let ty = match source.immediate {
                            Immediate::I32(_) => NumericType::I32,
                            Immediate::I64(_) => NumericType::I64,
                            Immediate::F32(_) => NumericType::F32,
                            Immediate::F64(_) => NumericType::F64,
                            _ => return Err(Declined::Metadata),
                        };
                        prefix_shape.push(ty);
                    }
                    0x28..=0x3e => {
                        let (ty, _, store) =
                            super::memory::signature(source.opcode).ok_or(Declined::Metadata)?;
                        prefix_shape.pop().ok_or(Declined::Metadata)?;
                        if store {
                            prefix_shape.pop().ok_or(Declined::Metadata)?;
                        } else {
                            prefix_shape.push(ty);
                        }
                    }
                    _ => {
                        let (_, result, binary) =
                            numeric_signature(source.opcode).ok_or(Declined::Metadata)?;
                        prefix_shape.pop().ok_or(Declined::Metadata)?;
                        if binary {
                            prefix_shape.pop().ok_or(Declined::Metadata)?;
                        }
                        prefix_shape.push(result);
                    }
                }
            }
            stack_end = event.stack.end;
            edge_end = event.successors.end;
        }
        if stack_end as usize != draft.stack_slots.len()
            || edge_end as usize != draft.targets.len()
            || memory_end != draft.memory.len()
            || prefix.is_none() && !labels.is_empty()
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

/// The first unsupported operation becomes a pending decoded boundary.
/// No nonlocal transfer executes inside this initial prefix subset.
pub(super) fn prefix_instruction(module: &Module, instruction: &crate::Instruction) -> bool {
    if !matches!(instruction.opcode, 1..=3 | 0xb | 0x1a | 0x20..=0x22 | 0x28..=0x3e | 0x41..=0xc4 | 0xfc00..=0xfc07)
    {
        return false;
    }
    match instruction.immediate {
        Immediate::MemArg { memory, .. } => {
            super::memory::declared(module, memory).is_some_and(super::memory::eligible)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Engine};
    include!("../../../tests/performance/p057_private.rs");
    include!("../../../tests/performance/p063_private.rs");
}
