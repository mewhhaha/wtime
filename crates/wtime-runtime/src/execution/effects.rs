//! Conservative metadata only: no second semantic validator and no executable
//! raw-code constructor. Unknown instructions and unproved stack layouts decline.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectFamily {
    Numeric,
    NumericTrap,
    Local,
    ReadMemory,
    WriteMemory,
    Call,
    Control,
    Barrier,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Effect {
    family: EffectFamily,
    may_trap: bool,
    roots: bool,
    materialize: bool,
    logical_call: bool,
}
pub(crate) fn classify(opcode: u16) -> Effect {
    use EffectFamily::*;
    let family = match opcode {
        0x01 | 0x41..=0x44 => Numeric,
        0x6d..=0x70 | 0x7f..=0x82 | 0xa8..=0xab | 0xae..=0xb1 => NumericTrap,
        0x45..=0xc4 | 0xfc00..=0xfc07 => Numeric,
        0x1a..=0x1c | 0x20..=0x22 => Local,
        0x28..=0x35 => ReadMemory,
        0x36..=0x3e => WriteMemory,
        0x10..=0x15 => Call,
        0x02..=0x05 | 0x0b | 0x1f => Control,
        _ => Barrier,
    };
    Effect {
        family,
        may_trap: !matches!(family, Numeric | Local | Control),
        roots: matches!(family, Local | Call | Control | Barrier),
        materialize: family != Numeric,
        logical_call: family == Call,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceOrigin {
    pc: usize,
    opcode: u16,
}
#[derive(Clone, Debug)]
struct SourceEvent {
    origin: Option<SourceOrigin>,
    effect: Effect,
    fuel_charge: u8,
    cancellation_checkpoint: bool,
    materialization_point: bool,
    /// Optional validated logical stack peak for this source instruction.
    transient_stack_peak: Option<usize>,
    /// Existing block/if control destinations. Unresolved branch-depth cases
    /// remain barriers; an in-bounds arbitrary target is not sufficient proof.
    control_destinations: Option<(usize, Option<usize>)>,
}
pub(super) struct PreparedEffect {
    source: Vec<SourceEvent>,
}
fn metadata_bytes(count: usize) -> Option<usize> {
    count
        .checked_mul(std::mem::size_of::<SourceEvent>())?
        .checked_add(std::mem::size_of::<PreparedEffect>())
}
impl PreparedEffect {
    pub(super) fn from_module(
        module: &Module,
        body: usize,
        max_work: usize,
        max_bytes: usize,
    ) -> Option<Self> {
        let instructions = &module.data.inner.decoded().code.get(body)?.instructions;
        if instructions.len().checked_mul(2)? > max_work
            || metadata_bytes(instructions.len())? > max_bytes
        {
            return None;
        }
        let mut source = Vec::new();
        source.try_reserve_exact(instructions.len()).ok()?;
        if metadata_bytes(source.capacity())? > max_bytes {
            return None;
        }
        for (pc, op) in instructions.iter().enumerate() {
            let effect = classify(op.opcode);
            source.push(SourceEvent {
                origin: Some(SourceOrigin {
                    pc,
                    opcode: op.opcode,
                }),
                effect,
                fuel_charge: 1,
                cancellation_checkpoint: true,
                materialization_point: effect.materialize,
                transient_stack_peak: None,
                control_destinations: module.data.control_get(body, pc),
            });
        }
        let completed = Self { source };
        completed.verify(module, body).then_some(completed)
    }
    pub(super) fn verify(&self, module: &Module, body: usize) -> bool {
        let Some(body_code) = module.data.inner.decoded().code.get(body) else {
            return false;
        };
        let ops = &body_code.instructions;
        if self.source.len() != ops.len() {
            return false;
        }
        for (pc, (event, op)) in self.source.iter().zip(ops).enumerate() {
            let expected = classify(op.opcode);
            if event.origin
                != Some(SourceOrigin {
                    pc,
                    opcode: op.opcode,
                })
                || event.effect != expected
                || event.fuel_charge != 1
                || !event.cancellation_checkpoint
                || (expected.materialize && !event.materialization_point)
                || event.control_destinations != module.data.control_get(body, pc)
            {
                return false;
            }
            if event
                .control_destinations
                .is_some_and(|(a, b)| a >= ops.len() || b.is_some_and(|b| b >= ops.len()))
            {
                return false;
            }
            if event.transient_stack_peak.is_some() {
                // No stack certificate producer exists yet; reject fabricated
                // bounds rather than treating arbitrary hints as trusted proof.
                return false;
            }
        }
        true
    }
    pub(super) fn numeric_region_eligible(&self) -> bool {
        !self.source.is_empty()
            && self.source.iter().all(|e| {
                e.effect.family == EffectFamily::Numeric && e.transient_stack_peak.is_some()
            })
    }
}
#[cfg(test)]
mod tests {
    include!("../../tests/performance/p019.rs");
}
