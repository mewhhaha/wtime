//! Two verified i32 expansions. Every original accounting boundary remains.
use super::source_map::SourceMap;
use super::*;
use crate::continuation::Prompt;
use crate::execution::{Frame, accounting};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pattern {
    AddLocals {
        first: usize,
        left: u32,
        right: u32,
    },
    AddConstantSet {
        first: usize,
        input: u32,
        constant: i32,
        output: u32,
    },
}
impl Pattern {
    fn first(self) -> usize {
        match self {
            Self::AddLocals { first, .. } | Self::AddConstantSet { first, .. } => first,
        }
    }
    fn len(self) -> usize {
        match self {
            Self::AddLocals { .. } => 3,
            Self::AddConstantSet { .. } => 4,
        }
    }
    fn op(self, stage: usize) -> CompactOp {
        let (opcode, form, aux, payload) = match (self, stage) {
            (Self::AddLocals { left, .. }, 0) => (0x20, INDEX, left, 0),
            (Self::AddConstantSet { input, .. }, 0) => (0x20, INDEX, input, 0),
            (Self::AddLocals { right, .. }, 1) => (0x20, INDEX, right, 0),
            (Self::AddConstantSet { constant, .. }, 1) => {
                (0x41, I32_BITS, 0, u64::from(constant as u32))
            }
            (_, 2) => (0x6a, NONE, 0, 0),
            (Self::AddConstantSet { output, .. }, 3) => (0x21, INDEX, output, 0),
            _ => unreachable!("sealed fusion stage"),
        };
        CompactOp {
            opcode,
            form,
            aux,
            payload,
        }
    }
    fn eligible(self, frame: &Frame, stack: &[Value], fuel: Option<u64>, store: &Store) -> bool {
        // The shared loop has already charged source 0. Short fuel and a
        // potentially failing logical peak take the original per-op route.
        if fuel.is_some_and(|remaining| remaining < (self.len() - 1) as u64)
            || stack
                .len()
                .checked_add(2)
                .is_none_or(|peak| peak > store.engine.config.limits.max_stack_height)
        {
            return false;
        }
        #[cfg(feature = "execution-reference")]
        if store
            .materialization_request
            .is_some_and(|(instance, body, pc, _)| {
                instance == frame.function.instance
                    && body == frame.function.body
                    && pc > self.first()
                    && pc < self.first() + self.len()
            })
        {
            return false;
        }
        frame.pc == self.first() + 1
    }
}

#[derive(Debug)]
pub(super) struct Fusions {
    starts: Vec<u16>, // zero = ordinary operation; otherwise descriptor + 1
    patterns: Vec<Pattern>,
    pub(super) bytes: usize,
    work: usize,
}

fn local_types<'a>(
    function: &CompactFunction,
    module: &'a Module,
) -> Option<(&'a [ValType], &'a [ValType])> {
    let decoded = module.data.inner.decoded();
    let ty = decoded.types.get(function.type_index)?;
    let body = decoded.code.get(function.body)?;
    Some((&ty.params, &body.locals))
}

fn recognize(
    ops: &[CompactOp],
    pc: usize,
    params: &[ValType],
    locals: &[ValType],
    entries: &[bool],
) -> Option<Pattern> {
    let local = |index: u32| {
        let index = usize::try_from(index).ok()?;
        if index < params.len() {
            params.get(index)
        } else {
            locals.get(index.checked_sub(params.len())?)
        }
    };
    let first = *ops.get(pc)?;
    let second = *ops.get(pc.checked_add(1)?)?;
    let third = *ops.get(pc.checked_add(2)?)?;
    if first.opcode != 0x20
        || first.form != INDEX
        || local(first.aux) != Some(&ValType::I32)
        || third
            != (CompactOp {
                opcode: 0x6a,
                form: NONE,
                aux: 0,
                payload: 0,
            })
        || *entries.get(pc + 1)?
        || *entries.get(pc + 2)?
    {
        return None;
    }
    if second.opcode == 0x20 && second.form == INDEX && local(second.aux) == Some(&ValType::I32) {
        Some(Pattern::AddLocals {
            first: pc,
            left: first.aux,
            right: second.aux,
        })
    } else if second.opcode == 0x41 && second.form == I32_BITS {
        let fourth = *ops.get(pc.checked_add(3)?)?;
        if fourth.opcode != 0x21
            || fourth.form != INDEX
            || local(fourth.aux) != Some(&ValType::I32)
            || *entries.get(pc + 3)?
        {
            return None;
        }
        Some(Pattern::AddConstantSet {
            first: pc,
            input: first.aux,
            constant: second.payload as u32 as i32,
            output: fourth.aux,
        })
    } else {
        None
    }
}

fn entries(
    map: &SourceMap,
    len: usize,
    max_bytes: usize,
) -> std::result::Result<Vec<bool>, Declined> {
    let count = len.checked_add(1).ok_or(Declined::Budget)?;
    if count > max_bytes {
        return Err(Declined::Budget);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| Declined::Allocation)?;
    if vector_bytes::<bool>(entries.capacity()).ok_or(Declined::Budget)? > max_bytes {
        return Err(Declined::Budget);
    }
    entries.resize(count, false);
    for pc in 0..len {
        for target in map.targets(pc).ok_or(Declined::Verification)? {
            // Sequential edges do not turn every source into a block entry.
            if *target != pc + 1 {
                *entries.get_mut(*target).ok_or(Declined::Verification)? = true;
            }
        }
    }
    Ok(entries)
}

impl Fusions {
    pub(super) fn prepare(
        function: &CompactFunction,
        module: &Module,
        map: &SourceMap,
        max_work: usize,
        max_bytes: usize,
    ) -> std::result::Result<Option<Self>, Declined> {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let (params, locals) = local_types(function, module).ok_or(Declined::Verification)?;
        // Reference-bearing frames remain on their previously selected path.
        if params
            .iter()
            .chain(locals)
            .any(|ty| matches!(ty, ValType::Ref(_)))
        {
            return Ok(None);
        }
        let len = function.ops.len();
        let edge_count = (0..len)
            .try_fold(0usize, |count, pc| {
                count.checked_add(map.targets(pc)?.len())
            })
            .ok_or(Declined::Budget)?;
        let work = len
            .checked_mul(16)
            .and_then(|n| n.checked_add(edge_count.checked_mul(2)?))
            .and_then(|n| n.checked_add(params.len().checked_add(locals.len())?.checked_mul(2)?))
            .ok_or(Declined::Budget)?;
        if work > max_work {
            return Err(Declined::Budget);
        }
        let entries = entries(map, len, max_bytes)?;
        let scratch = vector_bytes::<bool>(entries.capacity()).ok_or(Declined::Budget)?;
        let mut pc = 0;
        let mut count = 0usize;
        while pc < len {
            if let Some(pattern) = recognize(&function.ops, pc, params, locals, &entries) {
                count = count.checked_add(1).ok_or(Declined::Budget)?;
                pc += pattern.len();
            } else {
                pc += 1;
            }
        }
        if count == 0 {
            return Ok(None);
        }
        if count > u16::MAX as usize {
            return Err(Declined::Budget);
        }
        let planned = std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<u16>(len).ok_or(Declined::Budget)?)
            .and_then(|n| n.checked_add(vector_bytes::<Pattern>(count)?))
            .and_then(|n| n.checked_add(scratch))
            .ok_or(Declined::Budget)?;
        if planned > max_bytes {
            return Err(Declined::Budget);
        }
        let mut completed = Self {
            starts: vec![],
            patterns: vec![],
            bytes: 0,
            work,
        };
        completed
            .starts
            .try_reserve_exact(len)
            .map_err(|_| Declined::Allocation)?;
        completed
            .patterns
            .try_reserve_exact(count)
            .map_err(|_| Declined::Allocation)?;
        completed.starts.resize(len, 0);
        pc = 0;
        while pc < len {
            if let Some(pattern) = recognize(&function.ops, pc, params, locals, &entries) {
                completed.starts[pc] =
                    u16::try_from(completed.patterns.len() + 1).map_err(|_| Declined::Budget)?;
                completed.patterns.push(pattern);
                pc += pattern.len();
            } else {
                pc += 1;
            }
        }
        completed.bytes = completed.retained().ok_or(Declined::Budget)?;
        if completed
            .bytes
            .checked_add(scratch)
            .ok_or(Declined::Budget)?
            > max_bytes
        {
            return Err(Declined::Budget);
        }
        Ok(Some(completed))
    }
    fn retained(&self) -> Option<usize> {
        std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<u16>(self.starts.capacity())?)?
            .checked_add(vector_bytes::<Pattern>(self.patterns.capacity())?)
    }
    pub(super) fn get(&self, pc: usize) -> Option<Pattern> {
        let index = usize::from(*self.starts.get(pc)?).checked_sub(1)?;
        self.patterns.get(index).copied()
    }
    pub(super) fn verify(
        &self,
        function: &CompactFunction,
        module: &Module,
        map: &SourceMap,
        max_work: usize,
        max_bytes: usize,
    ) -> bool {
        if self.retained() != Some(self.bytes) || self.bytes > max_bytes || self.work > max_work {
            return false;
        }
        // Reconstruct under the original retained + verification scratch cap.
        let Some(remaining) = max_bytes.checked_sub(self.bytes) else {
            return false;
        };
        let Ok(Some(expected)) = Self::prepare(function, module, map, max_work, remaining) else {
            return false;
        };
        self.starts == expected.starts
            && self.patterns == expected.patterns
            && self.work == expected.work
    }
}

/// Source 0 is charged/recorded by the outer loop. All later charges use the
/// original logical height; pending pure values are restored before an error.
pub(crate) fn execute(
    pattern: Pattern,
    frame: &mut Frame,
    stack: &mut Vec<Value>,
    store: &mut Store,
    fuel: &mut Option<u64>,
    frame_depth: usize,
    prompts: &mut Vec<Prompt>,
) -> Result<()> {
    if !pattern.eligible(frame, stack, *fuel, store) {
        super::execute::execute(pattern.op(0), &mut frame.locals, stack, store)?;
        return Ok(());
    }
    #[cfg(feature = "execution-reference")]
    {
        store.compact_fused_groups = store.compact_fused_groups.saturating_add(1);
    }
    let local_i32 = |frame: &Frame, index: u32| -> Result<i32> {
        match frame.locals.get(index as usize) {
            Some(Value::I32(value)) => Ok(*value),
            _ => Err(trap("unverified fusion local type")),
        }
    };
    let left = local_i32(
        frame,
        match pattern {
            Pattern::AddLocals { left, .. } => left,
            Pattern::AddConstantSet { input, .. } => input,
        },
    )?;
    let mut pending = [Value::I32(left), Value::I32(0)];
    let mut height = 1;
    for stage in 1..pattern.len() {
        frame.pc = pattern.first() + stage;
        let logical_height = stack
            .len()
            .checked_add(height)
            .ok_or_else(|| trap("fusion operand height overflow"))?;
        #[cfg(test)]
        CANCEL_AT.with(|slot| {
            if slot.get() == Some(stage) {
                slot.set(None);
                store.cancellation.cancel();
            }
        });
        if let Err(error) =
            accounting::next_source(store, fuel, frame_depth, logical_height, prompts)
        {
            stack.extend_from_slice(&pending[..height]);
            return Err(error);
        }
        super::execute::record(pattern.op(stage), store);
        #[cfg(feature = "execution-reference")]
        {
            store.compact_dispatches = store.compact_dispatches.saturating_add(1);
        }
        frame.pc += 1;
        match stage {
            1 => {
                let right = match pattern {
                    Pattern::AddLocals { right, .. } => local_i32(frame, right)?,
                    Pattern::AddConstantSet { constant, .. } => constant,
                };
                pending[1] = Value::I32(right);
                height = 2;
            }
            2 => {
                let Value::I32(right) = pending[1] else {
                    unreachable!()
                };
                pending[0] = Value::I32(left.wrapping_add(right));
                height = 1;
            }
            3 => {
                let Pattern::AddConstantSet { output, .. } = pattern else {
                    unreachable!()
                };
                *frame
                    .locals
                    .get_mut(output as usize)
                    .ok_or_else(|| trap("fusion output local index"))? = pending[0];
                height = 0;
            }
            _ => unreachable!(),
        }
    }
    stack.extend_from_slice(&pending[..height]);
    Ok(())
}

#[cfg(test)]
thread_local! { static CANCEL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }

#[cfg(test)]
mod tests {
    use super::*;
    const BYTES: &[u8] =
        include_bytes!("../../../tests/fixtures/performance/p047/p047_const_local_set.wasm");
    fn admitted() -> (Engine, Module, CompactFunction, SourceMap) {
        let engine =
            Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap();
        let module = engine.compile(BYTES).unwrap();
        let function = CompactFunction::from_module(&module, 0, MAX_WORK, MAX_BYTES).unwrap();
        let map = SourceMap::bounded(&function, &module, MAX_WORK, MAX_BYTES).unwrap();
        (engine, module, function, map)
    }
    #[test]
    fn p047_corrupt_or_over_budget_declines() {
        let (_, module, function, map) = admitted();
        for case in 0..7 {
            let mut fusions = Fusions::prepare(&function, &module, &map, MAX_WORK, MAX_BYTES)
                .unwrap()
                .unwrap();
            assert!(fusions.verify(&function, &module, &map, MAX_WORK, MAX_BYTES));
            match case {
                0 => fusions.starts[0] = 0,
                1 => fusions.starts[1] = 1,
                2 => fusions.starts[0] = u16::MAX,
                3 => {
                    fusions.patterns[0] = Pattern::AddConstantSet {
                        first: 0,
                        input: 0,
                        constant: 7,
                        output: 1,
                    }
                }
                4 => fusions.work += 1,
                5 => fusions.bytes += 1,
                6 => {
                    fusions.patterns.clear();
                }
                _ => unreachable!(),
            }
            assert!(
                !fusions.verify(&function, &module, &map, MAX_WORK, MAX_BYTES),
                "corruption {case}"
            );
        }
        assert!(matches!(
            Fusions::prepare(&function, &module, &map, 0, MAX_BYTES),
            Err(Declined::Budget)
        ));
        assert!(matches!(
            Fusions::prepare(&function, &module, &map, MAX_WORK, 1),
            Err(Declined::Budget)
        ));
        let fusions = Fusions::prepare(&function, &module, &map, MAX_WORK, MAX_BYTES)
            .unwrap()
            .unwrap();
        assert!(!fusions.verify(&function, &module, &map, MAX_WORK, fusions.bytes));
        // An externally entering interior source must decline recognition.
        let mut entries = vec![false; function.ops.len() + 1];
        assert!(recognize(&function.ops, 0, &[ValType::I32], &[ValType::I32], &entries).is_some());
        for pc in 1..4 {
            entries[pc] = true;
            assert!(
                recognize(&function.ops, 0, &[ValType::I32], &[ValType::I32], &entries).is_none()
            );
            entries[pc] = false;
        }
    }
    #[test]
    fn p047_internal_cancellation_materializes_the_completed_pure_prefix() {
        let (engine, module, _, _) = admitted();
        for stage in 1..4 {
            let mut store = Store::new(&engine);
            let instance = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let External::Func(func) = instance.export(&store, "run").unwrap() else {
                panic!()
            };
            let mut frame = store.frame(func, vec![Value::I32(i32::MAX)], 0).unwrap();
            frame.pc = 1;
            let pattern = Pattern::AddConstantSet {
                first: 0,
                input: 0,
                constant: i32::MAX,
                output: 1,
            };
            let mut stack = vec![Value::I32(91)];
            let mut fuel = Some(10);
            let mut prompts = vec![];
            CANCEL_AT.with(|slot| slot.set(Some(stage)));
            let error = execute(
                pattern,
                &mut frame,
                &mut stack,
                &mut store,
                &mut fuel,
                1,
                &mut prompts,
            )
            .unwrap_err();
            assert_eq!(
                (error.kind, error.message.as_str()),
                (ErrorKind::ResourceLimit, "execution cancelled")
            );
            assert_eq!(frame.pc, stage);
            assert_eq!(fuel, Some(11 - stage as u64));
            let expected = match stage {
                1 => vec![Value::I32(91), Value::I32(i32::MAX)],
                2 => vec![Value::I32(91), Value::I32(i32::MAX), Value::I32(i32::MAX)],
                3 => vec![Value::I32(91), Value::I32(-2)],
                _ => unreachable!(),
            };
            assert_eq!(stack, expected);
            assert_eq!(frame.locals[1], Value::I32(0));
        }
    }
}
