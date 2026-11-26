//! Logical labels remain source-equivalent under checked control plans.
use super::*;
use crate::execution::{Frame, Label};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockRecord {
    params: u32,
    results: u32,
    end: u32,
    other: u32,
}
const NO_ELSE: u32 = u32::MAX;

#[derive(Debug)]
pub(super) struct ControlPlans {
    engine: u64,
    source: Arc<[u8]>,
    body: usize,
    blocks: Vec<Option<BlockRecord>>,
    pub(super) retained_bytes: usize,
    pub(super) work: usize,
}

fn arities(
    immediate: &Immediate,
    module: &Module,
    allowed: fn(&ValType) -> bool,
    remaining: &mut usize,
) -> std::result::Result<(u32, u32), Declined> {
    let (params, results) = match immediate {
        Immediate::Block(-64) => return Ok((0, 0)),
        Immediate::Block(value) if *value < 0 => {
            let ty = match *value {
                -1 => ValType::I32,
                -2 => ValType::I64,
                -3 => ValType::F32,
                -4 => ValType::F64,
                -5 => ValType::V128,
                _ => return Err(Declined::Unsupported),
            };
            return if allowed(&ty) {
                Ok((0, 1))
            } else {
                Err(Declined::Unsupported)
            };
        }
        Immediate::BlockType(ty) => {
            return if allowed(ty) {
                Ok((0, 1))
            } else {
                Err(Declined::Unsupported)
            };
        }
        Immediate::Block(index) => {
            let ty = module
                .data
                .inner
                .decoded()
                .types
                .get(usize::try_from(*index).map_err(|_| Declined::Plan)?)
                .ok_or(Declined::Plan)?;
            let count = ty
                .params
                .len()
                .checked_add(ty.results.len())
                .ok_or(Declined::Budget)?;
            *remaining = remaining.checked_sub(count).ok_or(Declined::Budget)?;
            if !ty.params.iter().chain(&ty.results).all(allowed) {
                return Err(Declined::Unsupported);
            }
            (ty.params.len(), ty.results.len())
        }
        _ => return Err(Declined::Unsupported),
    };
    Ok((
        u32::try_from(params).map_err(|_| Declined::Budget)?,
        u32::try_from(results).map_err(|_| Declined::Budget)?,
    ))
}

impl ControlPlans {
    #[cfg(test)]
    pub(super) fn p045_corrupt(&mut self) {
        self.blocks.clear();
    }
    pub(super) fn prepare(
        function: &CompactFunction,
        module: &Module,
        allowed: fn(&ValType) -> bool,
        max_work: usize,
        max_bytes: usize,
    ) -> std::result::Result<Self, Declined> {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let code = module
            .data
            .inner
            .decoded()
            .code
            .get(function.body)
            .ok_or(Declined::Plan)?;
        let initial_work = code
            .instructions
            .len()
            .checked_mul(2)
            .ok_or(Declined::Budget)?;
        let mut remaining_work = max_work.checked_sub(initial_work).ok_or(Declined::Budget)?;
        let planned = vector_bytes::<Option<BlockRecord>>(code.instructions.len())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .ok_or(Declined::Budget)?;
        if planned > max_bytes {
            return Err(Declined::Budget);
        }
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(code.instructions.len())
            .map_err(|_| Declined::Allocation)?;
        let retained_bytes = vector_bytes::<Option<BlockRecord>>(blocks.capacity())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .ok_or(Declined::Budget)?;
        if retained_bytes > max_bytes {
            return Err(Declined::Budget);
        }
        for (pc, instruction) in code.instructions.iter().enumerate() {
            let record = if matches!(instruction.opcode, 2..=4) {
                let (params, results) =
                    arities(&instruction.immediate, module, allowed, &mut remaining_work)?;
                let (end, other) = module
                    .data
                    .control_get(function.body, pc)
                    .ok_or(Declined::Plan)?;
                if end < pc
                    || end >= code.instructions.len()
                    || other.is_some_and(|other| other <= pc || other >= end)
                {
                    return Err(Declined::Plan);
                }
                Some(BlockRecord {
                    params,
                    results,
                    end: u32::try_from(end).map_err(|_| Declined::Budget)?,
                    other: match other {
                        Some(other) => u32::try_from(other).map_err(|_| Declined::Budget)?,
                        None => NO_ELSE,
                    },
                })
            } else {
                None
            };
            blocks.push(record);
        }
        Ok(Self {
            engine: function.engine,
            source: function.source.clone(),
            body: function.body,
            blocks,
            retained_bytes,
            work: max_work - remaining_work,
        })
    }

    pub(super) fn verify(
        &self,
        function: &CompactFunction,
        module: &Module,
        allowed: fn(&ValType) -> bool,
        max_work: usize,
        max_bytes: usize,
    ) -> bool {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let bytes = vector_bytes::<Option<BlockRecord>>(self.blocks.capacity())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()));
        if self.engine != function.engine
            || self.engine != module.data.engine
            || self.body != function.body
            || !Arc::ptr_eq(&self.source, &function.source)
            || !Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
            || bytes != Some(self.retained_bytes)
            || self.work > max_work
        {
            return false;
        }
        let Some(remaining_bytes) = max_bytes.checked_sub(self.retained_bytes) else {
            return false;
        };
        let Some(remaining_work) = max_work
            .checked_sub(self.blocks.len())
            .and_then(|work| work.checked_sub(self.work))
        else {
            return false;
        };
        let Ok(expected) =
            Self::prepare(function, module, allowed, remaining_work, remaining_bytes)
        else {
            return false;
        };
        self.work == expected.work && self.blocks == expected.blocks
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute(
        &self,
        function: &CompactFunction,
        map: &super::source_map::SourceMap,
        op: CompactOp,
        source_pc: usize,
        frame: &mut Frame,
        stack: &mut Vec<Value>,
        store: &mut Store,
    ) -> Result<Option<super::execute::Step>> {
        match op.opcode {
            0 => return Err(trap("unreachable")),
            2..=4 => {
                let block = self
                    .blocks
                    .get(source_pc)
                    .copied()
                    .flatten()
                    .ok_or_else(|| trap("unverified compact control record"))?;
                let condition = op.opcode != 4 || address(pop(stack)?)? != 0;
                let height = stack
                    .len()
                    .checked_sub(block.params as usize)
                    .ok_or_else(|| trap("block stack underflow"))?;
                store.push_label(
                    &mut frame.labels,
                    Label {
                        try_table_pc: None,
                        start: source_pc + 1,
                        end: block.end as usize,
                        height,
                        params: block.params as usize,
                        results: block.results as usize,
                        loop_: op.opcode == 3,
                    },
                )?;
                if !condition {
                    frame.pc = if block.other == NO_ELSE {
                        block.end as usize
                    } else {
                        block.other as usize + 1
                    };
                }
            }
            5 => {
                frame.pc = frame
                    .labels
                    .last()
                    .ok_or_else(|| trap("else without label"))?
                    .end
            }
            0x0b => {
                if let Some(label) = frame.labels.pop() {
                    store.release_labels(1);
                    keep(stack, label.height, label.results)?;
                    #[cfg(feature = "performance-counters")]
                    store.perf.record(
                        perf::Event::CompactResultMove,
                        u64::from(label.results != 0),
                    );
                } else {
                    return Ok(Some(super::execute::Step::Return));
                }
            }
            0x0c..=0x0e => {
                let (depth, edge) = if op.opcode == 0x0e {
                    let Some(SideImmediate::Branches { targets, default }) =
                        function.sides.get(op.aux as usize)
                    else {
                        return Err(trap("unverified compact branch table"));
                    };
                    let index = address(pop(stack)?)?;
                    let position = usize::try_from(index)
                        .ok()
                        .filter(|index| *index < targets.len());
                    (
                        position.map_or(*default, |position| targets[position]),
                        position.unwrap_or(targets.len()),
                    )
                } else {
                    if op.opcode == 0x0d && address(pop(stack)?)? == 0 {
                        return Ok(Some(super::execute::Step::Continue));
                    }
                    (op.aux, usize::from(op.opcode == 0x0d))
                };
                let depth = depth as usize;
                let destination = *map
                    .targets(source_pc)
                    .and_then(|targets| targets.get(edge))
                    .ok_or_else(|| trap("unverified compact branch target"))?;
                if depth == frame.labels.len() {
                    return Ok(Some(super::execute::Step::Return));
                }
                let position = frame
                    .labels
                    .len()
                    .checked_sub(depth.checked_add(1).ok_or_else(|| trap("branch depth"))?)
                    .ok_or_else(|| trap("branch depth"))?;
                let label = &frame.labels[position];
                keep(
                    stack,
                    label.height,
                    if label.loop_ {
                        label.params
                    } else {
                        label.results
                    },
                )?;
                #[cfg(feature = "performance-counters")]
                store.perf.record(
                    perf::Event::CompactResultMove,
                    u64::from(if label.loop_ {
                        label.params != 0
                    } else {
                        label.results != 0
                    }),
                );
                frame.pc = destination;
                let retain = position + usize::from(label.loop_);
                let removed = frame.labels.len() - retain;
                frame.labels.truncate(retain);
                store.release_labels(removed);
            }
            0x0f => return Ok(Some(super::execute::Step::Return)),
            _ => return Ok(None),
        }
        Ok(Some(super::execute::Step::Continue))
    }
}

// Same checked result contract as decoded keep, with overlap-safe movement.
pub(crate) fn keep(stack: &mut Vec<Value>, height: usize, count: usize) -> Result<()> {
    if stack.len() < count || height > stack.len() - count {
        return Err(trap("stack underflow"));
    }
    let first = stack.len() - count;
    stack.copy_within(first.., height);
    stack.truncate(height + count);
    Ok(())
}

#[cfg(test)]
include!("../../../tests/performance/p040_private.rs");
