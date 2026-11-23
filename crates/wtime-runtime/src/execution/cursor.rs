//! Development handoff at the decoded fetch boundary. frame.pc is advanced only
//! after this boundary; branch/exception/resume helpers install the successor.
//! Fuel for the pending operation is already charged. Restore stays in this
//! iteration, before fetch, and must not repeat the accounting boundary.
use super::*;
use crate::continuation::Prompt;
const FRAME_LIMIT: usize = 64;
const VALUE_LIMIT: usize = 64;
const MEMORY_LIMIT: usize = 8;
/// Guest instruction position captured at a diagnostic handoff boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionCursor {
    /// Store-internal instance index owning the executing function.
    pub instance: usize,
    /// Index of the defined function body within the instance's module.
    pub function_body: usize,
    /// Source instruction index in the decoded function body.
    pub source_pc: usize,
    /// Prepared-tier instruction index aligned with `source_pc`.
    pub prepared_pc: usize,
}
/// Normalized, store-independent representation of a runtime [`Value`] for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticValue {
    /// 32-bit integer value.
    I32(i32),
    /// 64-bit integer value.
    I64(i64),
    /// Raw IEEE 754 bit pattern of an `f32` value.
    F32(u32),
    /// Raw IEEE 754 bit pattern of an `f64` value.
    F64(u64),
    /// Little-endian 16-byte representation of a `v128` value.
    V128([u8; 16]),
    /// Normalized reference value avoiding raw store handles.
    Reference {
        /// Reference category label (`"null"`, `"func"`, `"struct"`, `"array"`, `"continuation"`, `"i31"`, `"exception"`, or `"extern"`).
        category: &'static str,
        /// Snapshot-local deduplicated identity index for non-null references, or `None` for null.
        identity: Option<usize>,
    },
}
/// Diagnostic snapshot of a single active guest call frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticFrame {
    /// Instruction position within the frame.
    pub cursor: ExecutionCursor,
    /// Operand-stack base index belonging to this frame.
    pub base: usize,
    /// Total number of locals in the frame (before any diagnostic truncation).
    pub local_count: usize,
    /// Normalized prefix of frame local values.
    pub locals: Vec<DiagnosticValue>,
    /// (start, end, height, params, results, loop, catch count).
    pub labels: Vec<(usize, usize, usize, usize, usize, bool, usize)>,
    /// Total number of open control labels in the frame.
    pub label_count: usize,
    /// Context identity for diagnostics only, never an admission/cache key.
    pub module_fingerprint: u64,
}
/// Bounded diagnostic snapshot of execution state captured at a materialization boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSnapshot {
    /// Execution cursor of the innermost active frame.
    pub cursor: ExecutionCursor,
    /// Normalized prefix of active guest call frames.
    pub frames: Vec<DiagnosticFrame>,
    /// Total number of active guest call frames.
    pub frame_count: usize,
    /// Normalized prefix of the operand stack.
    pub operands: Vec<DiagnosticValue>,
    /// Total number of values on the operand stack.
    pub operand_count: usize,
    /// Prefix of active continuation prompt `(frame_depth, operand_depth)` pairs.
    pub prompt_depths: Vec<(usize, usize)>,
    /// Total number of active continuation prompts.
    pub prompt_count: usize,
    /// Remaining instruction fuel at the boundary, if fuel metering is active.
    pub remaining_fuel: Option<u64>,
    /// Whether accounting for the pending instruction at `cursor` has already been charged.
    pub pending_boundary_charged: bool,
    /// Leading byte prefixes of the store's linear memories.
    pub memory_prefixes: Vec<Vec<u8>>,
    /// Total byte lengths of the sampled linear memories.
    pub memory_lengths: Vec<usize>,
    /// Normalized prefix of the store's global variable values.
    pub globals: Vec<DiagnosticValue>,
    /// Normalized leading element prefixes of the store's tables.
    pub table_prefixes: Vec<Vec<DiagnosticValue>>,
    /// Number of active host-call root groups currently pushed on the store.
    pub active_root_groups: usize,
    /// Number of values currently retained in the store's published-root list.
    pub published_value_count: usize,
    /// Explicit caps: 64 frames/values/labels/prompts/globals, 8 memory/table
    /// prefixes, 64 bytes/elements per prefix. Truncated views are not full digests.
    pub truncated: bool,
}
/// Sealed owned state, including all locals, catches, prompts and reference
/// values. No callbacks/collections occur while detached; restore precedes any
/// guest/host work. Module contexts remain in the owning Store's instances.
pub(super) struct MaterializedState {
    owner: u64,
    frames: Vec<Frame>,
    operands: Vec<Value>,
    prompts: Vec<Prompt>,
    fuel: Option<u64>,
}
impl MaterializedState {
    fn take(
        store: &Store,
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
        fuel: &mut Option<u64>,
    ) -> Self {
        Self {
            owner: store.id,
            frames: std::mem::take(frames),
            operands: std::mem::take(operands),
            prompts: std::mem::take(prompts),
            fuel: fuel.take(),
        }
    }
    fn restore(
        self,
        store: &Store,
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        if self.owner != store.id
            || !frames.is_empty()
            || !operands.is_empty()
            || !prompts.is_empty()
            || fuel.is_some()
        {
            return Err(err(
                ErrorKind::Host,
                "invalid materialized-state destination",
            ));
        }
        *frames = self.frames;
        *operands = self.operands;
        *prompts = self.prompts;
        *fuel = self.fuel;
        Ok(())
    }
    fn snapshot(&self, store: &Store) -> Result<DiagnosticSnapshot> {
        let mut identities = Vec::new();
        let mut truncated = self.frames.len() > FRAME_LIMIT
            || self.operands.len() > VALUE_LIMIT
            || self.prompts.len() > VALUE_LIMIT
            || store.mems.len() > MEMORY_LIMIT
            || store.tabs.len() > MEMORY_LIMIT
            || store.globals.len() > VALUE_LIMIT;
        let mut frames = Vec::new();
        for frame in self.frames.iter().take(FRAME_LIMIT) {
            let module = &store.instances[frame.function.instance].module;
            truncated |= frame.locals.len() > VALUE_LIMIT || frame.labels.len() > VALUE_LIMIT;
            let mut fingerprint = 0xcbf29ce484222325u64;
            // Bounded context fingerprint, diagnostic only. Include full length.
            let bytes = &module.data.inner.decoded().bytes;
            for byte in bytes.iter().take(256) {
                fingerprint = (fingerprint ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
            fingerprint ^= bytes.len() as u64;
            frames.push(DiagnosticFrame {
                cursor: cursor(store, frame)?,
                base: frame.base,
                local_count: frame.locals.len(),
                locals: normalized(
                    store,
                    frame.locals.iter().copied().take(VALUE_LIMIT),
                    &mut identities,
                )?,
                labels: frame
                    .labels
                    .iter()
                    .take(VALUE_LIMIT)
                    .map(|l| {
                        (
                            l.start,
                            l.end,
                            l.height,
                            l.params,
                            l.results,
                            l.loop_,
                            l.catches.len(),
                        )
                    })
                    .collect(),
                label_count: frame.labels.len(),
                module_fingerprint: fingerprint,
            });
        }
        let mut memory_prefixes = Vec::new();
        let mut memory_lengths = Vec::new();
        for mem in store.mems.iter().take(MEMORY_LIMIT) {
            let len = mem.len()?;
            truncated |= len > VALUE_LIMIT;
            memory_lengths.push(len);
            memory_prefixes.push(mem.read(0, len.min(VALUE_LIMIT))?);
        }
        let mut table_prefixes = Vec::new();
        for table in store.tabs.iter().take(MEMORY_LIMIT) {
            truncated |= table.values.len() > VALUE_LIMIT;
            table_prefixes.push(normalized(
                store,
                table.values.iter().copied().take(VALUE_LIMIT),
                &mut identities,
            )?);
        }
        let cursor = cursor(
            store,
            self.frames
                .last()
                .ok_or_else(|| trap("materialized state has no frame"))?,
        )?;
        Ok(DiagnosticSnapshot {
            cursor,
            frames,
            frame_count: self.frames.len(),
            operands: normalized(
                store,
                self.operands.iter().copied().take(VALUE_LIMIT),
                &mut identities,
            )?,
            operand_count: self.operands.len(),
            prompt_depths: self
                .prompts
                .iter()
                .take(VALUE_LIMIT)
                .map(|p| (p.frame_depth, p.operand_depth))
                .collect(),
            prompt_count: self.prompts.len(),
            remaining_fuel: self.fuel,
            pending_boundary_charged: true,
            memory_prefixes,
            memory_lengths,
            globals: normalized(
                store,
                store.globals.iter().take(VALUE_LIMIT).map(|g| g.value),
                &mut identities,
            )?,
            table_prefixes,
            active_root_groups: store.active_roots.len(),
            published_value_count: store.published.len(),
            truncated,
        })
    }
}
fn cursor(store: &Store, frame: &Frame) -> Result<ExecutionCursor> {
    let instance = frame.function.instance;
    let m = &store
        .instances
        .get(instance)
        .ok_or_else(|| trap("cursor instance"))?
        .module;
    let body = m
        .data
        .inner
        .decoded()
        .code
        .get(frame.function.body)
        .ok_or_else(|| trap("cursor function"))?;
    if frame.pc >= body.instructions.len() {
        return Err(trap("instruction pointer out of bounds"));
    }
    Ok(ExecutionCursor {
        instance,
        function_body: frame.function.body,
        source_pc: frame.pc,
        prepared_pc: frame.pc,
    })
}
fn normalized(
    store: &Store,
    values: impl Iterator<Item = Value>,
    identities: &mut Vec<RefValue>,
) -> Result<Vec<DiagnosticValue>> {
    values
        .map(|v| {
            Ok(match v {
                Value::I32(n) => DiagnosticValue::I32(n),
                Value::I64(n) => DiagnosticValue::I64(n),
                Value::F32(n) => DiagnosticValue::F32(n),
                Value::F64(n) => DiagnosticValue::F64(n),
                Value::V128(n) => DiagnosticValue::V128(n.0),
                Value::Ref(r) => {
                    store.validate_value(v)?;
                    let category = match r {
                        RefValue::Null(_) => "null",
                        RefValue::Func(_) => "func",
                        RefValue::Gc(g) => match store.gc.get(g)?.kind {
                            gc::ObjectKind::Struct => "struct",
                            gc::ObjectKind::Array => "array",
                            gc::ObjectKind::Continuation => "continuation",
                        },
                        RefValue::I31(_) => "i31",
                        RefValue::Exn(_) => "exception",
                        _ => "extern",
                    };
                    let identity = if matches!(r, RefValue::Null(_)) {
                        None
                    } else {
                        Some(match identities.iter().position(|old| *old == r) {
                            Some(i) => i,
                            None => {
                                let i = identities.len();
                                identities.push(r);
                                i
                            }
                        })
                    };
                    DiagnosticValue::Reference { category, identity }
                }
            })
        })
        .collect()
}
impl Store {
    /// Request one development-only move/restore at the next matching decoded
    /// fetch boundary. No executable state or raw handle can be supplied.
    pub fn request_materialization_roundtrip(
        &mut self,
        function: Func,
        source_pc: usize,
    ) -> Result<()> {
        self.request_materialization_roundtrip_after_visits(function, source_pc, 0)
    }
    /// Skip a bounded number of matching dynamic source visits before the
    /// one-shot roundtrip. A request remains pending until its visit is reached
    /// or another request replaces it; skipped visits do not capture state.
    pub fn request_materialization_roundtrip_after_visits(
        &mut self,
        function: Func,
        source_pc: usize,
        skip_visits: usize,
    ) -> Result<()> {
        let k = self.check(function.0, &self.funcs)?;
        let fun = &self.funcs[k];
        if fun.host.is_some() {
            return Err(err(ErrorKind::Host, "host function has no guest cursor"));
        }
        let code = &self.instances[fun.instance]
            .module
            .data
            .inner
            .decoded()
            .code[fun.body];
        if source_pc >= code.instructions.len() {
            return Err(err(ErrorKind::Host, "source cursor out of bounds"));
        }
        if skip_visits > 100_000 {
            return Err(err(
                ErrorKind::ResourceLimit,
                "diagnostic source visit bound exceeded",
            ));
        }
        self.materialization_request = Some((fun.instance, fun.body, source_pc, skip_visits));
        self.execution_snapshot = None;
        Ok(())
    }
    /// Returns the most recently captured diagnostic execution snapshot, if any.
    pub fn execution_snapshot(&self) -> Option<&DiagnosticSnapshot> {
        self.execution_snapshot.as_ref()
    }
    /// Returns the total number of completed materialization roundtrips performed on this store.
    pub fn materialization_roundtrips(&self) -> u64 {
        self.materialization_roundtrips
    }
    pub(super) fn materialization_boundary(
        &mut self,
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let Some(frame) = frames.last() else {
            return Ok(());
        };
        let Some((instance, body, pc, remaining)) = self.materialization_request.as_mut() else {
            return Ok(());
        };
        if (*instance, *body, *pc) != (frame.function.instance, frame.function.body, frame.pc) {
            return Ok(());
        }
        if *remaining > 0 {
            *remaining -= 1;
            return Ok(());
        }
        self.materialization_request = None;
        let state = MaterializedState::take(self, frames, operands, prompts, fuel);
        let view = state.snapshot(self);
        // Restore even if observing poisoned shared memory or a bad view fails.
        state.restore(self, frames, operands, prompts, fuel)?;
        self.execution_snapshot = Some(view?);
        self.materialization_roundtrips = self.materialization_roundtrips.saturating_add(1);
        Ok(())
    }
}
