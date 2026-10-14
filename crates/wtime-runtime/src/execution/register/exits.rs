//! One-way handoff of a numeric prefix to the original decoded instruction.
use super::{execute, ir::Budget, lower::LoweredFunction};
use crate::{Module, Result, Store, Value, execution::Frame};
// These transitions may create, publish, trace or capture reference state.
// They always execute in Decoded after the verified numeric prefix ends.
pub(super) fn transition(opcode: u16) -> bool {
    matches!(opcode,
        0x08 | 0x0a | 0x10..=0x15 | 0x1f |
        0xd0..=0xd6 | 0xe0..=0xe6 | 0xfb00..=0xfbff)
}
pub(super) fn prepare(
    module: &Module,
    body: usize,
    budget: &mut Budget,
) -> Option<LoweredFunction> {
    let code = &module.data.inner.decoded().code.get(body)?.instructions;
    // Unsupported memory-only functions retain their whole-function fallback.
    let mut transaction = budget.clone();
    transaction.reserve(code.len(), 0).ok()?;
    if !code
        .iter()
        .any(|instruction| transition(instruction.opcode))
    {
        return None;
    }
    let mut lowered = LoweredFunction::lower_prefix(module, body, &mut transaction).ok()?;
    lowered.work = lowered.work.checked_add(code.len())?;
    *budget = transaction;
    Some(lowered)
}
pub(super) fn handoff(frame: &mut Frame, stack: &mut Vec<Value>, store: &mut Store) -> Result<()> {
    execute::materialize(frame, stack, store)?;
    frame.register = None;
    #[cfg(feature = "execution-reference")]
    {
        store.register_exits = store.register_exits.saturating_add(1);
    }
    Ok(())
}
