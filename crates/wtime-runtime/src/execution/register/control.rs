//! Immutable simultaneous edge-move schedules. No production execution yet.
use super::{
    ir::{Budget, Declined, NumericType},
    lower::EdgeMove,
};
use std::mem::size_of;
type Checked<T> = Result<T, Declined>;
fn add(a: usize, b: usize) -> Checked<usize> {
    a.checked_add(b).ok_or(Declined::Budget)
}
fn mul(a: usize, b: usize) -> Checked<usize> {
    a.checked_mul(b).ok_or(Declined::Budget)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveSource {
    Slot(u32),
    Scratch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveStep {
    Save {
        from: u32,
        ty: NumericType,
    },
    Copy {
        from: MoveSource,
        to: u32,
        ty: NumericType,
    },
}
#[derive(Clone, Copy)]
struct Pending {
    from: MoveSource,
    to: u32,
    ty: NumericType,
}
pub(super) struct MovePlan {
    steps: Vec<MoveStep>,
    scratch: bool,
    work: usize,
    retained_bytes: usize,
}
impl MovePlan {
    pub(super) fn steps(&self) -> &[MoveStep] {
        &self.steps
    }
    pub(super) fn scratch_slots(&self) -> usize {
        usize::from(self.scratch)
    }
    pub(super) fn costs(&self) -> (usize, usize) {
        (self.work, self.retained_bytes)
    }
    pub(super) fn prepare(requests: &[EdgeMove], slots: u32, budget: &mut Budget) -> Checked<Self> {
        let n = requests.len();
        let square = mul(n, n)?;
        // Upper charge for validation/sorting plus <=2n scheduling iterations,
        // each with <=n*n dependency comparisons. Charge before any allocation.
        let work = add(
            1,
            add(mul(mul(2, square)?, n)?, add(mul(3, square)?, mul(8, n)?)?)?,
        )?;
        let max_steps = mul(2, n)?;
        let peak = add(
            size_of::<Self>(),
            add(
                mul(n, size_of::<Pending>())?,
                mul(max_steps, size_of::<MoveStep>())?,
            )?,
        )?;
        let (remaining_work, remaining_bytes) = budget.remaining();
        if work > remaining_work || peak > remaining_bytes {
            return Err(Declined::Budget);
        }
        for (i, request) in requests.iter().enumerate() {
            if request.from >= slots || request.to >= slots {
                return Err(Declined::Metadata);
            }
            for other in &requests[..i] {
                if request.to == other.to {
                    return Err(Declined::Metadata);
                }
                // Reads of one old value must agree on its type. A later
                // write may reuse the same temporary slot with another type.
                if request.from == other.from && request.ty != other.ty {
                    return Err(Declined::Metadata);
                }
            }
        }
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(n)
            .map_err(|_| Declined::Allocation)?;
        let mut steps = Vec::new();
        steps
            .try_reserve_exact(max_steps)
            .map_err(|_| Declined::Allocation)?;
        let actual_peak = add(
            size_of::<Self>(),
            add(
                mul(pending.capacity(), size_of::<Pending>())?,
                mul(steps.capacity(), size_of::<MoveStep>())?,
            )?,
        )?;
        if actual_peak > remaining_bytes {
            return Err(Declined::Budget);
        }
        pending.extend(
            requests
                .iter()
                .filter(|request| request.from != request.to)
                .map(|request| Pending {
                    from: MoveSource::Slot(request.from),
                    to: request.to,
                    ty: request.ty,
                }),
        );
        pending.sort_unstable_by_key(|request| request.to);
        let mut scratch = false;
        let mut iterations = 0;
        while !pending.is_empty() {
            iterations += 1;
            if iterations > max_steps || steps.len() >= max_steps {
                return Err(Declined::Metadata);
            }
            // Overwrite a destination only after all reads of its old value.
            let ready = pending.iter().position(|request| {
                !pending
                    .iter()
                    .any(|other| other.from == MoveSource::Slot(request.to))
            });
            if let Some(index) = ready {
                let request = pending.remove(index);
                steps.push(MoveStep::Copy {
                    from: request.from,
                    to: request.to,
                    ty: request.ty,
                });
            } else {
                // A saved source always opens a cycle. Finish its reads before
                // reusing the single scratch value for another component.
                if pending
                    .iter()
                    .any(|request| request.from == MoveSource::Scratch)
                {
                    return Err(Declined::Metadata);
                }
                let MoveSource::Slot(source) = pending[0].from else {
                    return Err(Declined::Metadata);
                };
                steps.push(MoveStep::Save {
                    from: source,
                    ty: pending[0].ty,
                });
                for request in &mut pending {
                    if request.from == MoveSource::Slot(source) {
                        request.from = MoveSource::Scratch;
                    }
                }
                scratch = true;
            }
        }
        let retained_bytes = add(
            size_of::<Self>(),
            mul(steps.capacity(), size_of::<MoveStep>())?,
        )?;
        // All remaining operations are infallible publication after reservation.
        budget.reserve(work, retained_bytes)?;
        Ok(Self {
            steps,
            scratch,
            work,
            retained_bytes,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p130_private.rs");
}
