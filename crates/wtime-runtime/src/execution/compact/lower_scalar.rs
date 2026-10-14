//! Whole-function integer admission with checked source control plans.
//! Admission and source-map verification precede publication.
use super::*;
use super::source_map::SourceMap;
use super::lower_control::ControlPlans;
use crate::execution::Frame;
#[path = "verify.rs"]
mod verify;

#[derive(Debug)]
struct WholeFunction {
    function: CompactFunction,
    map: SourceMap,
    control: ControlPlans,
    has_calls: bool,
    fusions: Option<super::fuse::Fusions>,
    memory: Option<super::memory::MemoryAccesses>,
}

#[derive(Debug)]
enum PreparedFunction {
    Whole(WholeFunction),
    Prefix(super::prefix::Prefix),
}
impl PreparedFunction {
    fn has_calls(&self) -> bool { match self { Self::Whole(p) => p.has_calls, Self::Prefix(p) => p.has_calls } }
    fn has_memory(&self) -> bool { matches!(self, Self::Whole(p) if p.memory.is_some()) }
}

#[derive(Debug)]
pub(crate) struct PreparedModule {
    functions: Vec<Option<PreparedFunction>>,
    retained_bytes: usize,
    attempted_work: usize,
    declined: usize,
    inline: Option<super::inline::Leaves>,
}

fn integer(value: &ValType) -> bool { matches!(value, ValType::I32 | ValType::I64) }

pub(super) fn supported(instruction: &wtime_validate::Instruction, _pc: usize, _terminal: usize) -> bool {
    match instruction.opcode {
        0 | 1 | 5 | 0x0b | 0x0f | 0x1a | 0x1b => matches!(instruction.immediate, Immediate::None),
        2..=4 => matches!(instruction.immediate, Immediate::Block(_) | Immediate::BlockType(_)),
        0x0c..=0x0d => matches!(instruction.immediate, Immediate::Index(_)),
        0x0e => matches!(instruction.immediate, Immediate::BrTable(_, _)),
        0x1c => matches!(&instruction.immediate, Immediate::Types(types) if types.len() == 1 && integer(&types[0])),
        0x20..=0x22 => matches!(instruction.immediate, Immediate::Index(_)),
        0x41 => matches!(instruction.immediate, Immediate::I32(_)),
        0x42 => matches!(instruction.immediate, Immediate::I64(_)),
        0x45..=0x5a | 0x67..=0x6c | 0x71..=0x7e | 0x83..=0x8a | 0xa7 | 0xac | 0xad | 0xc0..=0xc4 => matches!(instruction.immediate, Immediate::None),
        _ => false,
    }
}

// Charge nested branch payloads and repeated indexed block arity before any
// traversal. Failed candidates consume their attempted work as well.
fn preparation_charge(code: &wtime_validate::FunctionBody, declarations: usize, module: &Module, remaining: usize) -> Option<usize> {
    let mut charge = code.instructions.len().checked_mul(80)?.checked_add(declarations.checked_mul(2)?)?;
    if charge > remaining { return None; }
    for instruction in &code.instructions {
        let extra = match &instruction.immediate {
            Immediate::BrTable(targets, _) => targets.len().checked_mul(14)?,
            Immediate::Block(index) if *index >= 0 => {
                let ty = module.data.inner.decoded().types.get(usize::try_from(*index).ok()?)?;
                ty.params.len().checked_add(ty.results.len())?.checked_mul(6)?
            }
            _ => 0,
        };
        charge = charge.checked_add(extra)?;
        if charge > remaining { return None; }
    }
    Some(charge)
}

fn whole_eligible(code: &wtime_validate::FunctionBody, ty: &FuncType,
    module: &Module, calls: &super::super::calls::CallTypes, memories: &super::memory::MemoryTypes) -> bool {
    let allowed = super::lower_numeric::value_type;
    ty.params.iter().chain(&ty.results).chain(&code.locals).all(allowed)
        && !code.instructions.is_empty()
        && code.instructions.iter().enumerate().all(|(pc, op)| {
            if matches!(op.opcode, 0x10 | 0x12) {
                return matches!(op.immediate, Immediate::Index(index) if calls.supports(index));
            }
            if matches!(op.opcode, 2..=4) {
                match op.immediate {
                    Immediate::Block(index) if index >= 0 => {
                        if !module.data.inner.decoded().types.get(index as usize)
                            .is_some_and(|ty| ty.params.iter().chain(&ty.results).all(allowed)) { return false; }
                    }
                    Immediate::BlockType(ty) if !allowed(&ty) => return false,
                    _ => {}
                }
            }
            memories.supports(op) || super::lower_numeric::supported(op, pc, code.instructions.len())
        })
}

impl PreparedModule {
    pub(crate) fn prepare(module: &Module) -> Option<Self> {
        #[cfg(test)]
        if verify::fault(verify::Fault::LowerAllocation) || verify::fault(verify::Fault::LowerWork) { return None; }
        let plans = module.data.plans.as_ref()?;
        let decoded = module.data.inner.decoded();
        let count = decoded.code.len();
        // Charge the complete module index before considering any function.
        if count > MAX_WORK { return None; }
        let planned = vector_bytes::<Option<PreparedFunction>>(count)?.checked_add(std::mem::size_of::<Self>())?;
        if planned > MAX_BYTES { return None; }
        let mut functions = Vec::new();
        functions.try_reserve_exact(count).ok()?;
        let mut retained_bytes = vector_bytes::<Option<PreparedFunction>>(functions.capacity())?.checked_add(std::mem::size_of::<Self>())?;
        if retained_bytes > MAX_BYTES { return None; }
        let call_types = super::super::calls::CallTypes::prepare(module, MAX_WORK.checked_sub(count)?, MAX_BYTES.checked_sub(retained_bytes)?)?;
        let initial_work = count.checked_add(call_types.work)?;
        let memory_types = super::memory::MemoryTypes::prepare(module,
            MAX_WORK.checked_sub(initial_work)?,
            MAX_BYTES.checked_sub(retained_bytes)?.checked_sub(call_types.bytes)?)?;
        let initial_work = initial_work.checked_add(memory_types.work)?;
        let leaf_charge = super::inline::Leaves::charge(module)?;
        let inline_work = if leaf_charge <= MAX_WORK.checked_sub(initial_work)? { leaf_charge } else { 0 };
        let inline = if inline_work > 0 {
            super::inline::Leaves::prepare(module, inline_work,
                MAX_BYTES.checked_sub(retained_bytes)?.checked_sub(call_types.bytes)?.checked_sub(memory_types.bytes)?)
        } else { None };
        retained_bytes = retained_bytes.checked_add(inline.as_ref().map_or(0, |leaves| leaves.bytes))?;
        let mut attempted_work = initial_work.checked_add(inline_work)?;
        let mut declined = 0;
        for (body, code) in decoded.code.iter().enumerate() {
            let candidate = (|| -> std::result::Result<WholeFunction, Declined> {
                let plan = plans.functions.get(body).ok_or(Declined::Plan)?;
                let ty = decoded.types.get(plan.type_index).ok_or(Declined::Plan)?;
                let declarations = ty.params.len().checked_add(ty.results.len()).and_then(|n| n.checked_add(code.locals.len())).ok_or(Declined::Budget)?;
                // Covers factory, map construction/reconstruction, comparison,
                // and eligibility passes. Absolute cap applies to attempted
                // work including functions which later decline.
                let remaining_work = MAX_WORK.checked_sub(attempted_work).ok_or(Declined::Budget)?;
                let charge = match preparation_charge(code, declarations, module, remaining_work) {
                    Some(charge) => charge,
                    None => { attempted_work = MAX_WORK; return Err(Declined::Budget); }
                };
                attempted_work += charge;
                if !whole_eligible(code, ty, module, &call_types, &memory_types) { return Err(Declined::Unsupported); }
                // Compound branch payloads and indexed type arities were charged
                // before traversal; unsupported shapes still decline whole bodies.
                let remaining_bytes = MAX_BYTES.checked_sub(retained_bytes).and_then(|bytes| bytes.checked_sub(call_types.bytes)).and_then(|bytes| bytes.checked_sub(memory_types.bytes)).ok_or(Declined::Budget)?;
                let function = CompactFunction::from_module(module, body, remaining_work, remaining_bytes)?;
                let map = SourceMap::bounded(&function, module, remaining_work, remaining_bytes)?;
                let bytes = function.retained_bytes.checked_add(map.retained_bytes).ok_or(Declined::Budget)?;
                let control_budget = remaining_bytes.checked_sub(bytes).ok_or(Declined::Budget)?;
                let control = ControlPlans::prepare(&function, module, super::lower_numeric::value_type, remaining_work, control_budget)?;
                if bytes.checked_add(control.retained_bytes).ok_or(Declined::Budget)? > remaining_bytes { return Err(Declined::Budget); }
                let has_calls = code.instructions.iter().any(|instruction| matches!(instruction.opcode, 0x10 | 0x12));
                let used = bytes.checked_add(control.retained_bytes).ok_or(Declined::Budget)?;
                let memory_budget = remaining_bytes.checked_sub(used).ok_or(Declined::Budget)?;
                let memory = super::memory::MemoryAccesses::prepare(&function, module, &memory_types, remaining_work, memory_budget)?;
                let fusion_budget = memory_budget.checked_sub(memory.as_ref().map_or(0, |m| m.bytes)).ok_or(Declined::Budget)?;
                let fusions = super::fuse::Fusions::prepare(&function, module, &map, remaining_work, fusion_budget)?;
                Ok(WholeFunction { function, map, control, has_calls, memory, fusions })
            })();
            match candidate {
                Ok(candidate) => {
                    retained_bytes = retained_bytes.checked_add(candidate.function.retained_bytes)?.checked_add(candidate.map.retained_bytes)?.checked_add(candidate.control.retained_bytes)?.checked_add(candidate.memory.as_ref().map_or(0, |memory| memory.bytes))?.checked_add(candidate.fusions.as_ref().map_or(0, |f| f.bytes))?;
                    functions.push(Some(PreparedFunction::Whole(candidate)));
                }
                Err(Declined::Unsupported) => {
                    // A failed whole candidate has already charged its passes.
                    // Charge this second bounded factory even if it declines.
                    let candidate = (|| {
                        let plan = plans.functions.get(body)?;
                        let ty = decoded.types.get(plan.type_index)?;
                        let declarations = ty.params.len().checked_add(ty.results.len())?.checked_add(code.locals.len())?;
                        let charge = code.instructions.len().checked_mul(8)?.checked_add(declarations.checked_mul(2)?)?.checked_add(8)?;
                        if charge > MAX_WORK.checked_sub(attempted_work)? { attempted_work = MAX_WORK; return None; }
                        attempted_work += charge;
                        let bytes = MAX_BYTES.checked_sub(retained_bytes)?.checked_sub(call_types.bytes)?.checked_sub(memory_types.bytes)?;
                        super::prefix::Prefix::prepare(module, body, &call_types, charge, bytes).ok()
                    })();
                    if let Some(prefix) = candidate {
                        retained_bytes = retained_bytes.checked_add(prefix.bytes)?;
                        functions.push(Some(PreparedFunction::Prefix(prefix)));
                    } else { declined += 1; functions.push(None); }
                }
                Err(_) => { declined += 1; functions.push(None); }
            }
        }
        if functions.iter().all(Option::is_none) { return None; }
        let prepared = Self { functions, retained_bytes, attempted_work, declined, inline };
        prepared.verify(module, &call_types, &memory_types).then_some(prepared)
    }
    pub(crate) fn contains(&self, body: usize) -> bool {
        self.functions.get(body).is_some_and(Option::is_some)
    }
    pub(crate) fn has_calls(&self, body: usize) -> bool {
        self.functions.get(body).and_then(Option::as_ref).is_some_and(PreparedFunction::has_calls)
    }
    pub(crate) fn has_memory(&self, body: usize) -> bool {
        self.functions.get(body).and_then(Option::as_ref).is_some_and(PreparedFunction::has_memory)
    }
    pub(crate) fn needs_memory_bindings(&self) -> bool {
        self.functions.iter().flatten().any(PreparedFunction::has_memory)
    }
    pub(crate) fn needs_bindings(&self) -> bool {
        self.functions.iter().flatten().any(PreparedFunction::has_calls)
    }
    #[cfg(feature = "execution-reference")]
    pub(crate) fn is_prefix_step(&self, body: usize, pc: usize) -> bool {
        matches!(self.functions.get(body).and_then(Option::as_ref), Some(PreparedFunction::Prefix(p)) if p.is_step(pc))
    }
    pub(crate) fn step(&self, body: usize, pc: usize, frame: &mut Frame, stack: &mut Vec<Value>, store: &Store) -> Result<super::execute::Step> {
        let prepared = self.functions.get(body).and_then(Option::as_ref)
            .ok_or_else(|| trap("missing compact function"))?;
        let prepared = match prepared {
            PreparedFunction::Prefix(prefix) => return prefix.step(pc, frame, stack, store),
            PreparedFunction::Whole(whole) => whole,
        };
        let op = prepared.function.ops.get(pc).copied()
            .ok_or_else(|| trap("compact instruction pointer out of bounds"))?;
        super::execute::record(op, store);
        if let Some(pattern) = prepared.fusions.as_ref().and_then(|fusions| fusions.get(pc)) {
            return Ok(super::execute::Step::Fusion(pattern));
        }
        if let Some(access) = prepared.memory.as_ref().and_then(|memory| memory.get(pc)) {
            return Ok(super::execute::Step::Memory(access));
        }
        if let Some(step) = prepared.control.execute(&prepared.function, &prepared.map, op, pc, frame, stack, store)? { return Ok(step); }
        if super::lower_numeric::execute(&prepared.function, op, stack)? { return Ok(super::execute::Step::Continue); }
        super::execute::execute(op, &mut frame.locals, stack, store)
    }
    pub(crate) fn inline_leaf(&self, body: usize) -> Option<super::inline::Leaf> {
        if !self.contains(body) { return None; }
        self.inline.as_ref()?.get(body)
    }
    #[cfg(feature = "performance-counters")]
    pub(crate) fn counts(&self) -> (usize, usize, usize, usize) {
        (self.functions.iter().filter(|function| function.is_some()).count(), self.declined, self.attempted_work, self.retained_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p045_private.rs");
}
