//! Final bounded admission of the assembled module, before publication.
//! Leaf reconstruction uses the remaining peak-storage budget, including all
//! already retained functions and the temporary call/memory declaration maps.
use super::*;

impl PreparedModule {
    pub(super) fn verify(
        &self,
        module: &Module,
        calls: &super::super::super::calls::CallTypes,
        memories: &super::super::memory::MemoryTypes,
    ) -> bool {
        self.checked(module, calls, memories).is_some()
    }

    fn checked(
        &self,
        module: &Module,
        calls: &super::super::super::calls::CallTypes,
        memories: &super::super::memory::MemoryTypes,
    ) -> Option<()> {
        #[cfg(test)]
        if fault(Fault::VerifyAllocation) || fault(Fault::VerifyWork) {
            return None;
        }
        let decoded = module.data.inner.decoded();
        let plans = module.data.plans.as_ref()?;
        let count = decoded.code.len();
        if self.functions.len() != count
            || count > MAX_WORK
            || self.attempted_work > MAX_WORK
            || self.retained_bytes > MAX_BYTES
        {
            return None;
        }
        let mut bytes = vector_bytes::<Option<PreparedFunction>>(self.functions.capacity())?
            .checked_add(std::mem::size_of::<Self>())?
            .checked_add(self.inline.as_ref().map_or(0, |leaves| leaves.bytes))?;
        let mut declined = 0usize;
        let mut accepted = 0usize;
        for prepared in &self.functions {
            let extra = match prepared {
                None => {
                    declined = declined.checked_add(1)?;
                    0
                }
                Some(PreparedFunction::Whole(p)) => {
                    accepted = accepted.checked_add(1)?;
                    p.function
                        .retained_bytes
                        .checked_add(p.map.retained_bytes)?
                        .checked_add(p.control.retained_bytes)?
                        .checked_add(p.memory.as_ref().map_or(0, |m| m.bytes))?
                        .checked_add(p.fusions.as_ref().map_or(0, |f| f.bytes))?
                }
                Some(PreparedFunction::Prefix(p)) => {
                    accepted = accepted.checked_add(1)?;
                    p.bytes
                }
            };
            bytes = bytes.checked_add(extra)?;
            if bytes > MAX_BYTES {
                return None;
            }
        }
        if accepted == 0 || declined != self.declined || bytes != self.retained_bytes {
            return None;
        }
        let scratch = MAX_BYTES
            .checked_sub(bytes)?
            .checked_sub(calls.bytes)?
            .checked_sub(memories.bytes)?;
        let mut work = count.checked_add(calls.work)?.checked_add(memories.work)?;
        if work > MAX_WORK {
            return None;
        }
        let leaf_charge = super::super::inline::Leaves::charge(module)?;
        let inline_work = if leaf_charge <= MAX_WORK.checked_sub(work)? {
            leaf_charge
        } else {
            0
        };
        if let Some(leaves) = &self.inline
            && !leaves.verify(module, inline_work, scratch.checked_add(leaves.bytes)?)
        {
            return None;
        }
        work = work.checked_add(inline_work)?;
        for (body, (code, prepared)) in decoded.code.iter().zip(&self.functions).enumerate() {
            let plan = plans.functions.get(body)?;
            let ty = decoded.types.get(plan.type_index)?;
            if plan.body != body {
                return None;
            }
            let declarations = ty
                .params
                .len()
                .checked_add(ty.results.len())?
                .checked_add(code.locals.len())?;
            let remaining = MAX_WORK.checked_sub(work)?;
            let Some(charge) = preparation_charge(code, declarations, module, remaining) else {
                work = MAX_WORK;
                if prepared.is_some() {
                    return None;
                }
                continue;
            };
            work = work.checked_add(charge)?;
            let whole = whole_eligible(code, ty, module, calls, memories);
            if !whole {
                let extra = code
                    .instructions
                    .len()
                    .checked_mul(8)?
                    .checked_add(declarations.checked_mul(2)?)?
                    .checked_add(8)?;
                if extra > MAX_WORK.checked_sub(work)? {
                    work = MAX_WORK;
                    if prepared.is_some() {
                        return None;
                    }
                    continue;
                }
                work = work.checked_add(extra)?;
            }
            match prepared {
                None => {}
                Some(PreparedFunction::Prefix(p)) => {
                    if whole
                        || !p.belongs_to(body)
                        || !p.verify(module, calls, charge, scratch.checked_add(p.bytes)?)
                    {
                        return None;
                    }
                }
                Some(PreparedFunction::Whole(p)) => {
                    let function = &p.function;
                    if !whole
                        || function.body != body
                        || function.type_index != plan.type_index
                        || !function.verify(module)
                        || p.has_calls
                            != code
                                .instructions
                                .iter()
                                .any(|op| matches!(op.opcode, 0x10 | 0x12))
                        || p.memory.is_some()
                            != code.instructions.iter().any(|op| memories.supports(op))
                    {
                        return None;
                    }
                    // SourceMap::verify accounts for its original map and the
                    // immutable function as well as reconstruction scratch.
                    let map_bytes = scratch
                        .checked_add(function.retained_bytes)?
                        .checked_add(p.map.retained_bytes)?;
                    if !p.map.verify(function, module, charge, map_bytes)
                        || !p.control.verify(
                            function,
                            module,
                            super::super::lower_numeric::value_type,
                            charge,
                            scratch.checked_add(p.control.retained_bytes)?,
                        )
                    {
                        return None;
                    }
                    if let Some(memory) = &p.memory
                        && !memory.verify(function, module, memories, charge, memory.bytes)
                    {
                        return None;
                    }
                    if let Some(fusions) = &p.fusions {
                        if !fusions.verify(
                            function,
                            module,
                            &p.map,
                            charge,
                            scratch.checked_add(fusions.bytes)?,
                        ) {
                            return None;
                        }
                    } else if super::super::fuse::Fusions::prepare(
                        function, module, &p.map, charge, scratch,
                    )
                    .ok()?
                    .is_some()
                    {
                        return None;
                    }
                }
            }
        }
        (work == self.attempted_work).then_some(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Fault {
    LowerAllocation,
    LowerWork,
    VerifyAllocation,
    VerifyWork,
}
#[cfg(test)]
thread_local! { static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn inject(fault: Fault) {
    FAULT.with(|slot| {
        assert!(slot.get().is_none());
        slot.set(Some(fault));
    });
}
#[cfg(test)]
pub(super) fn fault(fault: Fault) -> bool {
    FAULT.with(|slot| {
        if slot.get() == Some(fault) {
            slot.set(None);
            true
        } else {
            false
        }
    })
}
