//! Bounded immutable hints from admitted code. No Store handles or executable
//! constructors; bounds never authorize skipping source resource checks.
use super::*;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_WORK: usize = 100_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotClass {
    Numeric,
    Vector,
    Reference,
}
pub(crate) fn slot_class(t: ValType) -> SlotClass {
    match t {
        ValType::V128 => SlotClass::Vector,
        ValType::Ref(_) => SlotClass::Reference,
        _ => SlotClass::Numeric,
    }
}
/// Borrow original admitted types, including exact nominal reference context.
pub(crate) struct FunctionLayout<'a> {
    pub params: &'a [ValType],
    pub locals: &'a [ValType],
    pub results: &'a [ValType],
}
impl Module {
    /// All views select a plan from this immutable owner. A caller cannot pair
    /// another module's plan with this module's nominal types or destinations.
    pub(crate) fn function_layout(&self, body: usize) -> Option<FunctionLayout<'_>> {
        let p = self.data.plans.as_ref()?.functions.get(body)?;
        let d = self.data.inner.decoded();
        let t = d.types.get(p.type_index)?;
        let b = d.code.get(body)?;
        Some(FunctionLayout {
            params: &t.params,
            locals: &b.locals,
            results: &t.results,
        })
    }
    pub(crate) fn source_effect(&self, body: usize, pc: usize) -> Option<super::effects::Effect> {
        self.data.plans.as_ref()?.functions.get(body)?;
        Some(super::effects::classify(
            self.data
                .inner
                .decoded()
                .code
                .get(body)?
                .instructions
                .get(pc)?
                .opcode,
        ))
    }
    pub(crate) fn handler_range(&self, body: usize, pc: usize) -> Option<std::ops::Range<usize>> {
        self.data.plans.as_ref()?.functions.get(body)?;
        match &self
            .data
            .inner
            .decoded()
            .code
            .get(body)?
            .instructions
            .get(pc)?
            .immediate
        {
            Immediate::TryTable { catches, .. } => Some(0..catches.len()),
            Immediate::Resume { handlers, .. } => Some(0..handlers.len()),
            _ => None,
        }
    }
    pub(crate) fn control_destination(
        &self,
        body: usize,
        label: usize,
    ) -> Option<(usize, Option<usize>)> {
        let p = self.data.plans.as_ref()?.functions.get(body)?;
        let pc = p.labels.get(label)?.source_pc;
        self.data.control.get(body, pc)
    }
}
#[derive(Debug)]
pub(crate) struct LabelPlan {
    pub source_pc: usize,
    pub params: usize,
    pub results: usize,
    // Range into the original immutable immediate, not copied handler handles.
    pub handlers: std::ops::Range<usize>,
}
#[derive(Debug)]
pub(crate) struct FunctionPlan {
    pub body: usize,
    pub type_index: usize,
    pub local_count: usize,
    pub stack_peak: usize,
    pub maximum_nesting: usize,
    pub labels: Vec<LabelPlan>,
}
#[derive(Debug)]
pub(crate) struct FunctionPlans {
    pub functions: Vec<FunctionPlan>,
    pub retained_bytes: usize,
    pub work: usize,
}
fn layout(i: &Immediate, m: &ValidatedModule) -> Option<(usize, usize)> {
    match i {
        Immediate::Block(-64) => Some((0, 0)),
        Immediate::Block(x) if *x < 0 => Some((0, 1)),
        Immediate::BlockType(_) => Some((0, 1)),
        Immediate::Block(x) => {
            let t = m.decoded().types.get(usize::try_from(*x).ok()?)?;
            Some((t.params.len(), t.results.len()))
        }
        _ => None,
    }
}
impl FunctionPlans {
    pub(crate) fn prepare(m: &ValidatedModule) -> Option<Self> {
        Self::bounded(m, MAX_WORK, MAX_BYTES)
    }
    pub(crate) fn bounded(m: &ValidatedModule, max_work: usize, max_bytes: usize) -> Option<Self> {
        let code = &m.decoded().code;
        let mut work = code.len();
        let mut label_count = 0usize;
        for b in code {
            work = work
                .checked_add(b.instructions.len().checked_mul(3)?)?
                .checked_add(b.locals.len())?;
            if work > max_work {
                return None;
            }
            label_count = label_count.checked_add(
                b.instructions
                    .iter()
                    .filter(|i| matches!(i.opcode, 2..=4 | 6 | 0x1f))
                    .count(),
            )?;
            if work > max_work {
                return None;
            }
        }
        let required = code
            .len()
            .checked_mul(std::mem::size_of::<FunctionPlan>())?
            .checked_add(label_count.checked_mul(std::mem::size_of::<LabelPlan>())?)?
            .checked_add(std::mem::size_of::<Self>())?;
        if required > max_bytes {
            return None;
        }
        let mut functions = Vec::new();
        functions.try_reserve_exact(code.len()).ok()?;
        let mut retained_bytes = std::mem::size_of::<Self>().checked_add(
            functions
                .capacity()
                .checked_mul(std::mem::size_of::<FunctionPlan>())?,
        )?;
        for (body, b) in code.iter().enumerate() {
            let type_index = m.decoded().functions.get(body)?.0 as usize;
            let t = m.decoded().types.get(type_index)?;
            let local_count = t.params.len().checked_add(b.locals.len())?;
            work = work
                .checked_add(t.params.len())?
                .checked_add(t.results.len())?;
            if work > max_work {
                return None;
            }
            let bounds = m.function_bounds().get(body)?;
            let mut labels = Vec::new();
            labels
                .try_reserve_exact(
                    b.instructions
                        .iter()
                        .filter(|i| matches!(i.opcode, 2..=4 | 6 | 0x1f))
                        .count(),
                )
                .ok()?;
            retained_bytes = retained_bytes.checked_add(
                labels
                    .capacity()
                    .checked_mul(std::mem::size_of::<LabelPlan>())?,
            )?;
            if retained_bytes > max_bytes {
                return None;
            }
            for (pc, i) in b.instructions.iter().enumerate() {
                if !matches!(i.opcode, 2..=4 | 6 | 0x1f) {
                    continue;
                }
                let (immediate, n) = match &i.immediate {
                    Immediate::TryTable { block, catches } => (block.as_ref(), catches.len()),
                    other => (other, 0),
                };
                let (params, results) = layout(immediate, m)?;
                labels.push(LabelPlan {
                    source_pc: pc,
                    params,
                    results,
                    handlers: 0..n,
                });
            }
            functions.push(FunctionPlan {
                body,
                type_index,
                local_count,
                stack_peak: bounds.stack_peak,
                maximum_nesting: bounds.maximum_nesting,
                labels,
            });
        }
        Some(Self {
            functions,
            retained_bytes,
            work,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    include!("../../tests/performance/p021.rs");
}
