//! One original-instruction boundary, never an aggregate group debit. A
//! prepared group calls this before each source event and exits immediately on
//! error; its pending instruction has not run. Bulk/wait checkpoints stay in
//! their existing helpers. The reference dispatcher keeps its archived copy.
use super::*;
use crate::continuation::Prompt;
#[inline]
pub(super) fn next_source(
    store: &mut Store,
    fuel: &mut Option<u64>,
    frame_depth: usize,
    operand_height: usize,
    prompts: &mut Vec<Prompt>,
) -> Result<()> {
    if store.resume_fuel.is_some()
        && let Some(saved) = store.resume_fuel.take()
    {
        *fuel = match (*fuel, saved) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) => a,
            (None, b) => b,
        };
    }
    if store.cancellation.is_cancelled() {
        return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
    }
    crate::continuation::retire_prompts(frame_depth, prompts);
    if let Some(n) = fuel {
        if *n == 0 {
            return Err(err(ErrorKind::ResourceLimit, "fuel exhausted"));
        }
        *n -= 1;
    }
    store.current_fuel = *fuel;
    if operand_height > store.engine.config.limits.max_stack_height {
        return Err(err(ErrorKind::ResourceLimit, "operand stack limit"));
    }
    Ok(())
}
