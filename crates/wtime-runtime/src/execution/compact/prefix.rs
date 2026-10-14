//! Bounded straight-line records ending before one original decoded operation.
//! Canonical Values and frames need no conversion and remain visible to roots.
use super::*;
use super::super::calls::CallTypes;

#[derive(Debug)]
pub(super) struct Prefix {
    engine: u64,
    source: Arc<[u8]>,
    body: usize,
    type_index: usize,
    ops: Vec<CompactOp>,
    exit: SourceOrigin,
    pub(super) bytes: usize,
    pub(super) work: usize,
    pub(super) has_calls: bool,
}

fn eligible(op: &Instruction, calls: &CallTypes) -> bool {
    match op.opcode {
        // Direct numeric calls return through the same canonical frame stack.
        0x10 => matches!(op.immediate, Immediate::Index(index) if calls.supports(index)),
        1 | 0x1a | 0x1b | 0x20..=0x22 | 0x41 | 0x42 |
        0x45..=0x5a | 0x67..=0x6c | 0x71..=0x7e | 0x83..=0x8a |
        0xa7 | 0xac | 0xad | 0xc0..=0xc4 => super::lower_scalar::supported(op, 0, 0),
        _ => false,
    }
}

fn cost(count: usize, declarations: usize) -> Option<usize> {
    count.checked_mul(8)?.checked_add(declarations.checked_mul(2)?)?.checked_add(8)
}
fn storage(capacity: usize) -> Option<usize> {
    vector_bytes::<CompactOp>(capacity)?.checked_add(std::mem::size_of::<Prefix>())
}

impl Prefix {
    pub(super) fn belongs_to(&self, body: usize) -> bool { self.body == body }
    #[cfg(test)]
    pub(super) fn p045_corrupt(&mut self, case: usize) {
        match case {
            0 => self.exit.pc -= 1,
            1 => self.exit.checkpoint = false,
            2 => self.ops[0].form = u16::MAX,
            3 => self.bytes = usize::MAX,
            4 => self.has_calls = !self.has_calls,
            5 => self.body = usize::MAX,
            _ => unreachable!(),
        }
    }
    pub(super) fn prepare(module: &Module, body: usize, calls: &CallTypes,
        max_work: usize, max_bytes: usize) -> std::result::Result<Self, Declined>
    {
        let plan = module.data.plans.as_ref().and_then(|p| p.functions.get(body)).ok_or(Declined::Plan)?;
        let code = module.data.inner.decoded().code.get(body).ok_or(Declined::Plan)?;
        let ty = module.data.inner.decoded().types.get(plan.type_index).ok_or(Declined::Plan)?;
        if plan.body != body { return Err(Declined::Plan); }
        let declarations = ty.params.len().checked_add(ty.results.len()).and_then(|n| n.checked_add(code.locals.len())).ok_or(Declined::Budget)?;
        let mut count = 0usize;
        for op in &code.instructions {
            if cost(count, declarations).ok_or(Declined::Budget)? > max_work.min(MAX_WORK) { return Err(Declined::Budget); }
            if !eligible(op, calls) { break; }
            count = count.checked_add(1).ok_or(Declined::Budget)?;
        }
        if count == 0 { return Err(Declined::Unsupported); }
        let original = code.instructions.get(count).ok_or(Declined::Unsupported)?;
        let work = cost(count, declarations).ok_or(Declined::Budget)?;
        if work > max_work.min(MAX_WORK) || storage(count).ok_or(Declined::Budget)? > max_bytes.min(MAX_BYTES) { return Err(Declined::Budget); }
        let mut ops = Vec::new();
        ops.try_reserve_exact(count).map_err(|_| Declined::Allocation)?;
        let bytes = storage(ops.capacity()).ok_or(Declined::Budget)?;
        if bytes > max_bytes.min(MAX_BYTES) { return Err(Declined::Budget); }
        for op in &code.instructions[..count] { ops.push(inline(op.opcode, &op.immediate).ok_or(Declined::Unsupported)?); }
        let has_calls = ops.iter().any(|op| op.opcode == 0x10);
        let completed = Self { engine: module.data.engine, source: module.data.inner.decoded().bytes.clone(),
            body, type_index: plan.type_index, ops, exit: SourceOrigin { pc: count, opcode: original.opcode,
                effect: classify(original.opcode), fuel: 1, checkpoint: true }, bytes, work, has_calls };
        if !completed.verify(module, calls, max_work, max_bytes) { return Err(Declined::Verification); }
        Ok(completed)
    }

    pub(super) fn verify(&self, module: &Module, calls: &CallTypes, max_work: usize, max_bytes: usize) -> bool {
        if self.engine != module.data.engine || !Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
            || self.work > max_work.min(MAX_WORK) || self.bytes > max_bytes.min(MAX_BYTES)
            || storage(self.ops.capacity()) != Some(self.bytes) || self.ops.is_empty() { return false; }
        let Some(plan) = module.data.plans.as_ref().and_then(|p| p.functions.get(self.body)) else { return false; };
        let Some(code) = module.data.inner.decoded().code.get(self.body) else { return false; };
        let Some(ty) = module.data.inner.decoded().types.get(self.type_index) else { return false; };
        let Some(declarations) = ty.params.len().checked_add(ty.results.len()).and_then(|n| n.checked_add(code.locals.len())) else { return false; };
        if plan.body != self.body || plan.type_index != self.type_index || cost(self.ops.len(), declarations) != Some(self.work)
            || self.exit.pc != self.ops.len() || self.has_calls != self.ops.iter().any(|op| op.opcode == 0x10) { return false; }
        let Some(exit) = code.instructions.get(self.exit.pc) else { return false; };
        if eligible(exit, calls) || self.exit != (SourceOrigin { pc: self.exit.pc, opcode: exit.opcode,
            effect: classify(exit.opcode), fuel: 1, checkpoint: true }) { return false; }
        self.ops.iter().zip(&code.instructions).all(|(record, original)|
            eligible(original, calls) && inline(original.opcode, &original.immediate) == Some(*record))
    }

    #[cfg(feature = "execution-reference")]
    pub(super) fn is_step(&self, pc: usize) -> bool { pc < self.ops.len() }

    pub(super) fn step(&self, pc: usize, frame: &mut Frame, stack: &mut Vec<Value>, store: &Store) -> Result<super::execute::Step> {
        if pc == self.exit.pc { return Ok(super::execute::Step::Exit { pc, opcode: self.exit.opcode }); }
        let op = self.ops.get(pc).copied().ok_or_else(|| trap("compact prefix source cursor"))?;
        super::execute::record(op, store);
        super::execute::execute(op, &mut frame.locals, stack, store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p114_private.rs");
}
