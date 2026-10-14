//! Immutable compact storage constructed only from an admitted module.
//! This foundation does not select or execute the compact tier.
use super::*;
use crate::execution::effects::{Effect, classify};
#[path = "memory.rs"]
pub(super) mod memory;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompactOp {
    opcode: u16,
    form: u16,
    aux: u32,
    payload: u64,
}

const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;
const _: () = assert!(std::mem::size_of::<CompactOp>() <= 24);

const NONE: u16 = 0;
const INDEX: u16 = 1;
const PAIR: u16 = 2;
const BLOCK: u16 = 3;
const I32_BITS: u16 = 4;
const I64_BITS: u16 = 5;
const F32_BITS: u16 = 6;
const F64_BITS: u16 = 7;
const SIDE: u16 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
enum BlockImmediate {
    Signed(i64),
    Value(ValType),
}

#[derive(Debug, PartialEq)]
enum SideImmediate {
    BlockType(ValType),
    Branches { targets: Vec<u32>, default: u32 },
    Catches { block: BlockImmediate, catches: Vec<wtime_validate::Catch> },
    Resume { ty: u32, tag: Option<u32>, handlers: Vec<(u32, Option<u32>)> },
    Vector([u8; 16]),
    Memory { align: u32, offset: u64, memory: u32 },
    Lane { align: u32, offset: u64, memory: u32, lane: u8 },
    Types(Vec<ValType>),
    Heap(HeapType),
    Cast { flags: u8, label: u32, from: HeapType, to: HeapType },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceOrigin {
    pc: usize,
    opcode: u16,
    effect: Effect,
    fuel: u8,
    checkpoint: bool,
}

#[derive(Debug)]
pub(super) struct CompactFunction {
    engine: u64,
    source: Arc<[u8]>,
    body: usize,
    type_index: usize,
    ops: Vec<CompactOp>,
    sides: Vec<SideImmediate>,
    origins: Vec<SourceOrigin>,
    retained_bytes: usize,
    work: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Declined {
    Plan,
    Unsupported,
    Budget,
    Allocation,
    Verification,
}

fn vector_bytes<T>(capacity: usize) -> Option<usize> {
    capacity.checked_mul(std::mem::size_of::<T>())
}

fn copy_vector<T: Clone>(source: &[T]) -> std::result::Result<Vec<T>, Declined> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(source.len()).map_err(|_| Declined::Allocation)?;
    copy.extend_from_slice(source);
    Ok(copy)
}

impl SideImmediate {
    fn dynamic_bytes(&self) -> Option<usize> {
        match self {
            Self::Branches { targets, .. } => vector_bytes::<u32>(targets.capacity()),
            Self::Catches { catches, .. } => vector_bytes::<wtime_validate::Catch>(catches.capacity()),
            Self::Resume { handlers, .. } => vector_bytes::<(u32, Option<u32>)>(handlers.capacity()),
            Self::Types(types) => vector_bytes::<ValType>(types.capacity()),
            _ => Some(0),
        }
    }

    fn from_immediate(immediate: &Immediate) -> std::result::Result<Self, Declined> {
        Ok(match immediate {
            Immediate::BlockType(value) => Self::BlockType(*value),
            Immediate::BrTable(targets, default) => Self::Branches { targets: copy_vector(targets)?, default: *default },
            Immediate::TryTable { block, catches } => {
                let block = match block.as_ref() {
                    Immediate::Block(value) => BlockImmediate::Signed(*value),
                    Immediate::BlockType(value) => BlockImmediate::Value(*value),
                    _ => return Err(Declined::Unsupported),
                };
                Self::Catches { block, catches: copy_vector(catches)? }
            }
            Immediate::Resume { ty, tag, handlers } => Self::Resume { ty: *ty, tag: *tag, handlers: copy_vector(handlers)? },
            Immediate::Bytes(bytes) => Self::Vector(bytes.as_slice().try_into().map_err(|_| Declined::Unsupported)?),
            Immediate::MemArg { align, offset, memory } => Self::Memory { align: *align, offset: *offset, memory: *memory },
            Immediate::MemLane { align, offset, memory, lane } => Self::Lane { align: *align, offset: *offset, memory: *memory, lane: *lane },
            Immediate::Types(types) => Self::Types(copy_vector(types)?),
            Immediate::Heap(heap) => Self::Heap(*heap),
            Immediate::Cast { flags, label, from, to } => Self::Cast { flags: *flags, label: *label, from: *from, to: *to },
            _ => return Err(Declined::Unsupported),
        })
    }

    fn matches(&self, immediate: &Immediate) -> bool {
        match (self, immediate) {
            (Self::BlockType(a), Immediate::BlockType(b)) => a == b,
            (Self::Branches { targets: a, default: x }, Immediate::BrTable(b, y)) => a == b && x == y,
            (Self::Catches { block: a, catches: x }, Immediate::TryTable { block: b, catches: y }) => {
                x == y && match (a, b.as_ref()) {
                    (BlockImmediate::Signed(a), Immediate::Block(b)) => a == b,
                    (BlockImmediate::Value(a), Immediate::BlockType(b)) => a == b,
                    _ => false,
                }
            }
            (Self::Resume { ty: a, tag: x, handlers: p }, Immediate::Resume { ty: b, tag: y, handlers: q }) => a == b && x == y && p == q,
            (Self::Vector(a), Immediate::Bytes(b)) => a.as_slice() == b.as_slice(),
            (Self::Memory { align: a, offset: x, memory: p }, Immediate::MemArg { align: b, offset: y, memory: q }) => a == b && x == y && p == q,
            (Self::Lane { align: a, offset: x, memory: p, lane: i }, Immediate::MemLane { align: b, offset: y, memory: q, lane: j }) => a == b && x == y && p == q && i == j,
            (Self::Types(a), Immediate::Types(b)) => a == b,
            (Self::Heap(a), Immediate::Heap(b)) => a == b,
            (Self::Cast { flags: a, label: x, from: p, to: i }, Immediate::Cast { flags: b, label: y, from: q, to: j }) => a == b && x == y && p == q && i == j,
            _ => false,
        }
    }
}

fn inline(opcode: u16, immediate: &Immediate) -> Option<CompactOp> {
    let mut op = CompactOp { opcode, form: NONE, aux: 0, payload: 0 };
    match immediate {
        Immediate::None => {},
        Immediate::Index(value) => { op.form = INDEX; op.aux = *value; },
        Immediate::Pair(a, b) => { op.form = PAIR; op.aux = *a; op.payload = u64::from(*b); },
        Immediate::Block(value) => { op.form = BLOCK; op.payload = *value as u64; },
        Immediate::I32(value) => { op.form = I32_BITS; op.payload = u64::from(*value as u32); },
        Immediate::I64(value) => { op.form = I64_BITS; op.payload = *value as u64; },
        Immediate::F32(value) => { op.form = F32_BITS; op.payload = u64::from(*value); },
        Immediate::F64(value) => { op.form = F64_BITS; op.payload = *value; },
        _ => return None,
    }
    Some(op)
}

fn shape(immediate: &Immediate) -> Option<(usize, usize)> {
    let (count, bytes) = match immediate {
        Immediate::BrTable(targets, _) => (targets.len(), vector_bytes::<u32>(targets.len())?),
        Immediate::TryTable { block, catches } => {
            if !matches!(block.as_ref(), Immediate::Block(_) | Immediate::BlockType(_)) { return None; }
            (catches.len(), vector_bytes::<wtime_validate::Catch>(catches.len())?)
        },
        Immediate::Resume { handlers, .. } => (handlers.len(), vector_bytes::<(u32, Option<u32>)>(handlers.len())?),
        Immediate::Types(types) => (types.len(), vector_bytes::<ValType>(types.len())?),
        Immediate::Bytes(bytes) if bytes.len() != 16 => return None,
        _ => (0, 0),
    };
    Some((count, bytes))
}

impl CompactFunction {
    pub(super) fn from_module(module: &Module, body: usize, max_work: usize, max_bytes: usize)
        -> std::result::Result<Self, Declined>
    {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let plan = module.data.plans.as_ref().and_then(|plans| plans.functions.get(body)).ok_or(Declined::Plan)?;
        let code = module.data.inner.decoded().code.get(body).ok_or(Declined::Plan)?;
        if plan.body != body { return Err(Declined::Plan); }
        let mut side_count = 0usize;
        let mut dynamic_bytes = 0usize;
        // Source sizing/emission/verification plus capacity-accounting passes
        // are all charged; nested vectors add their bounded element visits.
        let mut work = code.instructions.len().checked_mul(6).ok_or(Declined::Budget)?;
        if work > max_work { return Err(Declined::Budget); }
        for instruction in &code.instructions {
            if inline(instruction.opcode, &instruction.immediate).is_none() {
                side_count = side_count.checked_add(1).ok_or(Declined::Budget)?;
                let (elements, bytes) = shape(&instruction.immediate).ok_or(Declined::Unsupported)?;
                work = work.checked_add(elements.checked_mul(3).ok_or(Declined::Budget)?).ok_or(Declined::Budget)?;
                if work > max_work { return Err(Declined::Budget); }
                dynamic_bytes = dynamic_bytes.checked_add(bytes).ok_or(Declined::Budget)?;
            }
        }
        if side_count > u32::MAX as usize || work > max_work { return Err(Declined::Budget); }
        let bytes = std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<CompactOp>(code.instructions.len()).ok_or(Declined::Budget)?)
            .and_then(|bytes| bytes.checked_add(vector_bytes::<SourceOrigin>(code.instructions.len())?))
            .and_then(|bytes| bytes.checked_add(vector_bytes::<SideImmediate>(side_count)?))
            .and_then(|bytes| bytes.checked_add(dynamic_bytes)).ok_or(Declined::Budget)?;
        if bytes > max_bytes { return Err(Declined::Budget); }
        let mut completed = Self { engine: module.data.engine, source: module.data.inner.decoded().bytes.clone(), body, type_index: plan.type_index, ops: vec![], sides: vec![], origins: vec![], retained_bytes: 0, work };
        completed.ops.try_reserve_exact(code.instructions.len()).map_err(|_| Declined::Allocation)?;
        completed.origins.try_reserve_exact(code.instructions.len()).map_err(|_| Declined::Allocation)?;
        completed.sides.try_reserve_exact(side_count).map_err(|_| Declined::Allocation)?;
        let mut retained = completed.bytes().ok_or(Declined::Budget)?;
        if retained > max_bytes { return Err(Declined::Budget); }
        for (pc, instruction) in code.instructions.iter().enumerate() {
            let op = match inline(instruction.opcode, &instruction.immediate) {
                Some(op) => op,
                None => {
                    let index = u32::try_from(completed.sides.len()).map_err(|_| Declined::Budget)?;
                    let side = SideImmediate::from_immediate(&instruction.immediate)?;
                    retained = retained.checked_add(side.dynamic_bytes().ok_or(Declined::Budget)?).ok_or(Declined::Budget)?;
                    if retained > max_bytes {
                        return Err(Declined::Budget);
                    }
                    completed.sides.push(side);
                    CompactOp { opcode: instruction.opcode, form: SIDE, aux: index, payload: 0 }
                }
            };
            completed.ops.push(op);
            completed.origins.push(SourceOrigin { pc, opcode: instruction.opcode, effect: classify(instruction.opcode), fuel: 1, checkpoint: true });
        }
        completed.retained_bytes = completed.bytes().ok_or(Declined::Budget)?;
        if completed.retained_bytes > max_bytes { return Err(Declined::Budget); }
        if !completed.verify(module) { return Err(Declined::Verification); }
        Ok(completed)
    }

    fn bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<CompactOp>(self.ops.capacity())?)?
            .checked_add(vector_bytes::<SourceOrigin>(self.origins.capacity())?)?
            .checked_add(vector_bytes::<SideImmediate>(self.sides.capacity())?)?;
        for side in &self.sides { bytes = bytes.checked_add(side.dynamic_bytes()?)?; }
        Some(bytes)
    }

    pub(super) fn verify(&self, module: &Module) -> bool {
        if self.work > MAX_WORK || self.retained_bytes > MAX_BYTES { return false; }
        if self.engine != module.data.engine || !Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes) { return false; }
        let Some(plan) = module.data.plans.as_ref().and_then(|plans| plans.functions.get(self.body)) else { return false; };
        let Some(code) = module.data.inner.decoded().code.get(self.body) else { return false; };
        if plan.body != self.body || plan.type_index != self.type_index || self.ops.len() != code.instructions.len()
            || self.origins.len() != self.ops.len() || self.bytes() != Some(self.retained_bytes) { return false; }
        let Some(mut expected_work) = code.instructions.len().checked_mul(6) else { return false; };
        if expected_work > MAX_WORK { return false; }
        let mut side_index = 0usize;
        for (pc, ((op, origin), instruction)) in self.ops.iter().zip(&self.origins).zip(&code.instructions).enumerate() {
            if *origin != (SourceOrigin { pc, opcode: instruction.opcode, effect: classify(instruction.opcode), fuel: 1, checkpoint: true }) { return false; }
            if let Some(expected) = inline(instruction.opcode, &instruction.immediate) {
                if *op != expected { return false; }
            } else {
                let Some((elements, _)) = shape(&instruction.immediate) else { return false; };
                let Some(next_work) = elements.checked_mul(3).and_then(|elements| expected_work.checked_add(elements)) else { return false; };
                expected_work = next_work;
                if expected_work > MAX_WORK { return false; }
                if op.opcode != instruction.opcode || op.form != SIDE || op.payload != 0 || op.aux as usize != side_index
                    || !self.sides.get(side_index).is_some_and(|side| side.matches(&instruction.immediate)) { return false; }
                side_index += 1;
            }
        }
        side_index == self.sides.len() && expected_work == self.work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p036_private.rs");
}

#[path = "fuse.rs"]
pub(super) mod fuse;
#[path = "inline.rs"]
pub(super) mod inline;

#[path = "source_map.rs"]
mod source_map;
#[path = "lower_scalar.rs"]
pub(super) mod lower_scalar;
#[path = "lower_numeric.rs"]
pub(super) mod lower_numeric;
#[path = "lower_control.rs"]
pub(super) mod lower_control;
#[path = "execute.rs"]
pub(super) mod execute;
#[path = "prefix.rs"]
pub(super) mod prefix;
