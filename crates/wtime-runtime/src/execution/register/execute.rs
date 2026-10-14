//! Whole-function numeric tier. The caller charges the original source boundary.
use super::{
    control::{MovePlan, MoveSource, MoveStep},
    float, integer,
    ir::{Budget, Constant, NumericType, Operation},
    lower::LoweredFunction,
};
use crate::execution::{Frame, Label, arity};
use crate::{Immediate, Module, Result, Store, ValType, Value, trap};
use std::mem::size_of;
const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ACTIVATION_BYTES: usize = 2 * 1024 * 1024;
struct EdgePlan {
    target: usize,
    moves: MovePlan,
}
struct PreparedFunction {
    lowered: LoweredFunction,
    edges: Vec<Vec<EdgePlan>>,
}
pub(crate) struct PreparedModule {
    functions: Vec<Option<PreparedFunction>>,
}
impl std::fmt::Debug for PreparedModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterPreparedModule")
            .field("prepared", &self.count())
            .finish()
    }
}
fn valid_operation(op: &Operation, function: &LoweredFunction) -> bool {
    match op {
        Operation::Move { .. } => true,
        Operation::Const { constant, .. } => function.program.constant(*constant).is_some(),
        Operation::Unary { opcode, .. } | Operation::Binary { opcode, .. } => {
            integer::supported(*opcode) || float::supported(*opcode)
        }
        Operation::Jump { .. }
        | Operation::Branch { .. }
        | Operation::Table { .. }
        | Operation::Exit { .. } => true,
        Operation::Memory { .. } => false,
    }
}
impl PreparedModule {
    pub(crate) fn count(&self) -> usize {
        self.functions
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }
    fn function(&self, body: usize) -> Option<&PreparedFunction> {
        self.functions.get(body)?.as_ref()
    }
    pub(crate) fn prepare(module: &Module) -> Option<Self> {
        let plans = module.data.plans.as_ref()?;
        let count = module.data.inner.decoded().code.len();
        let index_bytes = count
            .checked_mul(size_of::<Option<PreparedFunction>>())?
            .checked_add(size_of::<Self>())?;
        if count > MAX_WORK || index_bytes > MAX_BYTES {
            return None;
        }
        let mut functions = Vec::new();
        functions.try_reserve_exact(count).ok()?;
        let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
        let bytes = functions
            .capacity()
            .checked_mul(size_of::<Option<PreparedFunction>>())?
            .checked_add(size_of::<Self>())?;
        budget.reserve(count, bytes).ok()?;
        functions.resize_with(count, || None);
        // Failed preparations also consume attempted work, separately from
        // transactional retained metadata. This bounds scans of declined bodies.
        let mut attempted_work = MAX_WORK.checked_sub(count)?;
        for (body, entry) in functions.iter_mut().enumerate() {
            let code = &module.data.inner.decoded().code[body].instructions;
            let n = code.len();
            let peak = plans.functions.get(body)?.stack_peak;
            let layout = module.function_layout(body)?;
            let locals = layout.params.len().checked_add(layout.locals.len())?;
            let table_extra =
                code.iter().try_fold(0usize, |total, instruction| {
                    match &instruction.immediate {
                        Immediate::BrTable(targets, _) => {
                            total.checked_add(targets.len().checked_add(1)?)
                        }
                        _ => Some(total),
                    }
                })?;
            let per_edge = peak
                .checked_mul(peak)?
                .checked_mul(peak)?
                .checked_mul(2)?
                .checked_add(peak.checked_mul(peak)?.checked_mul(3)?)?
                .checked_add(peak.checked_mul(8)?)?
                .checked_add(1)?;
            let charge =
                n.checked_mul(20)?
                    .checked_add(n.checked_mul(peak)?.checked_mul(10)?)?
                    .checked_add(n.checked_mul(per_edge)?)?
                    .checked_add(table_extra.checked_mul(
                        per_edge.checked_add(peak.checked_mul(8)?)?.checked_add(8)?,
                    )?)?
                    .checked_add(locals.checked_mul(2)?)?
                    .checked_add(layout.results.len().checked_mul(2)?)?;
            let Some(remaining) = attempted_work.checked_sub(charge) else {
                break;
            };
            attempted_work = remaining;
            if !layout
                .params
                .iter()
                .chain(layout.locals)
                .chain(layout.results)
                .all(|ty| {
                    matches!(
                        ty,
                        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
                    )
                })
            {
                continue;
            }
            let activation_bytes = locals
                .checked_add(peak)?
                .checked_mul(size_of::<u64>())?
                .checked_add(n.checked_mul(size_of::<Label>())?)?;
            if activation_bytes > MAX_ACTIVATION_BYTES {
                continue;
            }
            let mut transaction = budget.clone();
            let Ok(lowered) = LoweredFunction::lower(module, body, &mut transaction) else {
                continue;
            };
            if (0..n).any(|pc| {
                !lowered
                    .program
                    .operation(pc)
                    .is_some_and(|op| valid_operation(op, &lowered))
            }) {
                continue;
            }
            let extra = n
                .checked_mul(size_of::<Vec<EdgePlan>>())?
                .checked_add(size_of::<PreparedFunction>() - size_of::<LoweredFunction>())?;
            if transaction.reserve(n, extra).is_err() {
                continue;
            }
            let mut edges = Vec::new();
            if edges.try_reserve_exact(n).is_err() {
                continue;
            }
            let actual = edges
                .capacity()
                .checked_mul(size_of::<Vec<EdgePlan>>())?
                .checked_add(size_of::<PreparedFunction>() - size_of::<LoweredFunction>())?;
            if actual > extra && transaction.reserve(0, actual - extra).is_err() {
                continue;
            }
            edges.resize_with(n, Vec::new);
            let mut valid = true;
            let mut offset = 0;
            while offset < lowered.edges.len() {
                let pc = lowered.edges[offset].source_pc;
                if pc >= n || !edges[pc].is_empty() {
                    valid = false;
                    break;
                }
                let first = offset;
                while offset < lowered.edges.len() && lowered.edges[offset].source_pc == pc {
                    offset += 1;
                }
                let group = &lowered.edges[first..offset];
                let Some(bytes) = group.len().checked_mul(size_of::<EdgePlan>()) else {
                    valid = false;
                    break;
                };
                if transaction.reserve(group.len(), bytes).is_err()
                    || edges[pc].try_reserve_exact(group.len()).is_err()
                {
                    valid = false;
                    break;
                }
                let Some(actual_bytes) = edges[pc].capacity().checked_mul(size_of::<EdgePlan>())
                else {
                    valid = false;
                    break;
                };
                if actual_bytes > bytes && transaction.reserve(0, actual_bytes - bytes).is_err() {
                    valid = false;
                    break;
                }
                for edge in group {
                    match MovePlan::prepare(&edge.moves, lowered.program.slots(), &mut transaction)
                    {
                        Ok(moves) => edges[pc].push(EdgePlan {
                            target: edge.target,
                            moves,
                        }),
                        Err(_) => {
                            valid = false;
                            break;
                        }
                    }
                }
                if !valid {
                    break;
                }
            }
            if valid {
                *entry = Some(PreparedFunction { lowered, edges });
                budget = transaction;
            }
        }
        Some(Self { functions })
    }
}
#[derive(Clone)]
pub(crate) struct Activation {
    slots: Vec<u64>,
}
fn bits(value: Value) -> Option<u64> {
    match value {
        Value::I32(n) => Some(n as u32 as u64),
        Value::I64(n) => Some(n as u64),
        Value::F32(n) => Some(n as u64),
        Value::F64(n) => Some(n),
        _ => None,
    }
}
fn value(ty: NumericType, bits: u64) -> Result<Value> {
    match ty {
        NumericType::I32 => Ok(Value::I32(bits as u32 as i32)),
        NumericType::I64 => Ok(Value::I64(bits as i64)),
        NumericType::F32 => Ok(Value::F32(bits as u32)),
        NumericType::F64 => Ok(Value::F64(bits)),
    }
}
fn read(slots: &[u64], index: u32) -> Result<u64> {
    slots
        .get(index as usize)
        .copied()
        .ok_or_else(|| trap("register slot index"))
}
fn write(slots: &mut [u64], index: u32, value: u64) -> Result<()> {
    *slots
        .get_mut(index as usize)
        .ok_or_else(|| trap("register slot index"))? = value;
    Ok(())
}
fn function<'a>(frame: &Frame, store: &'a Store) -> Result<(&'a Module, &'a PreparedFunction)> {
    let module = &store.instances[frame.function.instance].module;
    let function = module
        .data
        .register
        .as_ref()
        .and_then(|program| program.function(frame.function.body))
        .ok_or_else(|| trap("missing admitted register function"))?;
    Ok((module, function))
}
pub(crate) fn activate(
    module: &Module,
    body: usize,
    locals: &[Value],
    labels: &mut Vec<Label>,
) -> Option<Activation> {
    let function = module.data.register.as_ref()?.function(body)?;
    let recipe = function.lowered.program.materialization(module, 0).ok()?;
    if recipe.locals.len() != locals.len() {
        return None;
    }
    let count = function.lowered.program.slots() as usize;
    let mut slots = Vec::new();
    slots.try_reserve_exact(count).ok()?;
    let mut prepared_labels = Vec::new();
    prepared_labels
        .try_reserve_exact(function.edges.len())
        .ok()?;
    if slots
        .capacity()
        .checked_mul(size_of::<u64>())?
        .checked_add(prepared_labels.capacity().checked_mul(size_of::<Label>())?)?
        > MAX_ACTIVATION_BYTES
    {
        return None;
    }
    for (ty, local) in recipe.locals.iter().zip(locals) {
        if !matches!(
            (ty, local),
            (NumericType::I32, Value::I32(_))
                | (NumericType::I64, Value::I64(_))
                | (NumericType::F32, Value::F32(_))
                | (NumericType::F64, Value::F64(_))
        ) {
            return None;
        }
        slots.push(bits(*local)?);
    }
    slots.resize(count, 0);
    *labels = prepared_labels;
    Some(Activation { slots })
}
pub(crate) fn logical_height(frame: &Frame, store: &Store) -> Result<usize> {
    let (_, function) = function(frame, store)?;
    let recipe = function
        .lowered
        .program
        .boundary(frame.pc)
        .map_err(|_| trap("register source identity"))?;
    frame
        .base
        .checked_add(recipe.operands.len())
        .ok_or_else(|| trap("register logical height overflow"))
}
#[cfg(feature = "execution-reference")]
pub(crate) fn materialize(frame: &mut Frame, stack: &mut Vec<Value>, store: &Store) -> Result<()> {
    let (module, function) = function(frame, store)?;
    let recipe = function
        .lowered
        .program
        .materialization(module, frame.pc)
        .map_err(|_| trap("register materialization identity"))?;
    let slots = &frame
        .register
        .as_ref()
        .ok_or_else(|| trap("register activation"))?
        .slots;
    for (index, ty) in recipe.locals.iter().enumerate() {
        frame.locals[index] = value(*ty, read(slots, index as u32)?)?;
    }
    stack.truncate(frame.base);
    for operand in recipe.operands {
        stack.push(value(operand.ty, read(slots, operand.slot)?)?);
    }
    Ok(())
}
pub(crate) enum Step {
    Continue,
    Return,
}
fn apply_moves(plan: &MovePlan, bank: &mut [u64]) -> Result<()> {
    let mut scratch = None;
    for step in plan.steps() {
        match *step {
            MoveStep::Save { from, ty } => scratch = Some((ty, read(bank, from)?)),
            MoveStep::Copy { from, to, ty } => {
                let v = match from {
                    MoveSource::Slot(index) => read(bank, index)?,
                    MoveSource::Scratch => {
                        let (saved_ty, value) =
                            scratch.ok_or_else(|| trap("register scratch initialization"))?;
                        if saved_ty != ty {
                            return Err(trap("register scratch type"));
                        }
                        value
                    }
                };
                write(bank, to, v)?;
            }
        }
    }
    Ok(())
}
fn results(
    slots: &[u64],
    indices: Option<&[u32]>,
    local_count: usize,
    types: &[ValType],
    base: usize,
    stack: &mut Vec<Value>,
) -> Result<()> {
    if indices.is_some_and(|indices| indices.len() != types.len()) {
        return Err(trap("register result arity"));
    }
    stack.truncate(base);
    for (index, ty) in types.iter().enumerate() {
        let slot = match indices {
            Some(indices) => indices[index],
            None => u32::try_from(
                local_count
                    .checked_add(index)
                    .ok_or_else(|| trap("register result index"))?,
            )
            .map_err(|_| trap("register result index"))?,
        };
        stack.push(value(
            NumericType::admitted(*ty).map_err(|_| trap("register result type"))?,
            read(slots, slot)?,
        )?);
    }
    Ok(())
}
pub(crate) fn step(frame: &mut Frame, stack: &mut Vec<Value>, store: &Store) -> Result<Step> {
    let (module, function) = function(frame, store)?;
    let program = &function.lowered.program;
    let pc = frame.pc;
    let recipe = program
        .boundary(pc)
        .map_err(|_| trap("register source identity"))?;
    let opcode = program
        .opcode(pc)
        .ok_or_else(|| trap("register source pc"))?;
    let instruction = if matches!(opcode, 2..=4 | 0xc..=0xe) {
        Some(&module.data.inner.decoded().code[frame.function.body].instructions[pc])
    } else {
        None
    };
    #[cfg(feature = "performance-counters")]
    {
        store.perf.record(crate::perf::Event::Dispatch, 1);
        store.perf.record(crate::perf::Event::RegisterStep, 1);
    }
    #[cfg(feature = "opcode-coverage")]
    crate::coverage::record(opcode);
    frame.pc = pc
        .checked_add(1)
        .ok_or_else(|| trap("register pc overflow"))?;
    let bank = &mut frame
        .register
        .as_mut()
        .ok_or_else(|| trap("register activation"))?
        .slots;
    let mut taken = false;
    let mut table_depth = None;
    let next = match program
        .operation(pc)
        .ok_or_else(|| trap("register operation pc"))?
    {
        Operation::Move { from, to, .. } => {
            let v = read(bank, *from)?;
            write(bank, *to, v)?;
            pc + 1
        }
        Operation::Const { constant, to } => {
            let v = match program.constant(*constant) {
                Some(Constant::I32(n)) => n as u64,
                Some(Constant::I64(n)) => n,
                Some(Constant::F32(n)) => n as u64,
                Some(Constant::F64(n)) => n,
                _ => return Err(trap("unverified register constant")),
            };
            write(bank, *to, v)?;
            pc + 1
        }
        Operation::Unary {
            opcode, from, to, ..
        } => {
            let a = read(bank, *from)?;
            let v = if integer::supported(*opcode) {
                integer::unary(*opcode, a)?
            } else {
                float::unary(*opcode, a)?
            };
            write(bank, *to, v)?;
            pc + 1
        }
        Operation::Binary {
            opcode,
            left,
            right,
            to,
            ..
        } => {
            let (a, b) = (read(bank, *left)?, read(bank, *right)?);
            let v = if integer::supported(*opcode) {
                integer::binary(*opcode, a, b)?
            } else {
                float::binary(*opcode, a, b)?
            };
            write(bank, *to, v)?;
            pc + 1
        }
        Operation::Jump { target } => {
            taken = opcode == 0xc;
            *target as usize
        }
        Operation::Branch { condition, yes, no } => {
            taken = read(bank, *condition)? as u32 != 0;
            if taken { *yes as usize } else { *no as usize }
        }
        Operation::Table {
            selector,
            targets: range,
        } => {
            let which = read(bank, *selector)? as u32 as usize;
            let targets = program
                .targets(range.clone())
                .ok_or_else(|| trap("register table targets"))?;
            let target = targets
                .get(which)
                .or_else(|| targets.last())
                .ok_or_else(|| trap("register table default"))?;
            let Immediate::BrTable(depths, default) = &instruction
                .ok_or_else(|| trap("register table source"))?
                .immediate
            else {
                return Err(trap("register table source"));
            };
            table_depth = Some(depths.get(which).copied().unwrap_or(*default));
            taken = true;
            *target as usize
        }
        Operation::Exit { results: range } => {
            let indices = program
                .result_slots(range.clone())
                .ok_or_else(|| trap("register result metadata"))?;
            results(
                bank,
                Some(indices),
                recipe.locals.len(),
                &frame.function.ty.results,
                frame.base,
                stack,
            )?;
            return Ok(Step::Return);
        }
        Operation::Memory { .. } => return Err(trap("unverified register memory")),
    };
    if let Some(edge) = function
        .edges
        .get(pc)
        .and_then(|entries| entries.iter().find(|edge| edge.target == next))
    {
        apply_moves(&edge.moves, bank)?;
    }
    match opcode {
        2..=4 => {
            let instruction = instruction.ok_or_else(|| trap("register control source"))?;
            let (params, results) = arity(&instruction.immediate, module);
            let (end, _) = module.data.control[frame.function.body][pc]
                .ok_or_else(|| trap("register control metadata"))?;
            let height = frame
                .base
                .checked_add(recipe.operands.len())
                .and_then(|height| height.checked_sub(params + usize::from(opcode == 4)))
                .ok_or_else(|| trap("register label height"))?;
            frame.labels.push(Label {
                catches: vec![],
                start: pc + 1,
                end,
                height,
                params,
                results,
                loop_: opcode == 3,
            });
        }
        0xb => {
            frame
                .labels
                .pop()
                .ok_or_else(|| trap("register label end"))?;
        }
        0xc..=0xe if taken => {
            let depth = if let Some(depth) = table_depth {
                depth
            } else {
                let Immediate::Index(depth) = instruction
                    .ok_or_else(|| trap("register branch source"))?
                    .immediate
                else {
                    return Err(trap("register branch depth"));
                };
                depth
            };
            let depth = depth as usize;
            if depth < frame.labels.len() {
                let index = frame.labels.len() - depth - 1;
                let retain = index + usize::from(frame.labels[index].loop_);
                frame.labels.truncate(retain);
            } else if depth != frame.labels.len() {
                return Err(trap("register branch depth"));
            }
        }
        _ => {}
    }
    if next == function.edges.len() {
        results(
            bank,
            None,
            recipe.locals.len(),
            &frame.function.ty.results,
            frame.base,
            stack,
        )?;
        return Ok(Step::Return);
    }
    frame.pc = next;
    Ok(Step::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    include!("../../../tests/performance/p061_private.rs");
    include!("../../../tests/performance/p060_private.rs");
    include!("../../../tests/performance/p062_private.rs");
}
