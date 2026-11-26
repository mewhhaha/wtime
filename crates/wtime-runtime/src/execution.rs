mod accounting;
#[cfg(feature = "compact-executor")]
#[allow(dead_code)]
pub(super) mod compact;
#[cfg(feature = "execution-reference")]
pub(crate) mod cursor;
#[allow(dead_code)]
mod effects;
#[cfg(not(feature = "execution-reference"))]
pub(crate) mod lowered;
#[allow(dead_code)]
pub(super) mod plan;
#[cfg(feature = "register-executor")]
#[allow(dead_code)]
pub(super) mod register;
#[cfg(not(feature = "execution-reference"))]
mod straight;
use super::*;
#[derive(Clone)]
pub(super) struct Label {
    /// Source PC of the `try_table` instruction owning this label's catch list
    /// (`None` for ordinary `block`/`loop`/`if` labels). Catch clauses are read
    /// directly from the immutable decoded module instead of cloned per entry.
    pub(super) try_table_pc: Option<usize>,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) height: usize,
    pub(super) params: usize,
    pub(super) results: usize,
    pub(super) loop_: bool,
}
#[derive(Clone)]
pub(super) struct Frame {
    #[cfg(feature = "register-executor")]
    pub(super) register: Option<register::Activation>,
    #[cfg(feature = "compact-executor")]
    pub(super) compact: bool,
    pub(super) function: Function,
    pub(super) pc: usize,
    pub(super) locals: Vec<Value>,
    pub(super) base: usize,
    pub(super) labels: Vec<Label>,
}
fn arity(i: &Immediate, m: &Module) -> (usize, usize) {
    match i {
        Immediate::Block(-64) => (0, 0),
        Immediate::Block(x) if *x < 0 => (0, 1),
        Immediate::BlockType(_) => (0, 1),
        Immediate::Block(x) => {
            let t = &m.data.inner.decoded().types[*x as usize];
            (t.params.len(), t.results.len())
        }
        _ => (0, 0),
    }
}
/// Keeps a returned frame's locals vector, emptied, for a later call that
/// needs exactly its capacity, unless enough are kept or it is large.
#[inline]
fn retire(spare: &mut Vec<Vec<Value>>, frame: Frame) {
    if spare.len() < 16 && frame.locals.capacity() <= 1024 {
        let mut locals = frame.locals;
        locals.clear();
        spare.push(locals);
    }
}
#[inline(always)]
pub(super) fn keep(s: &mut Vec<Value>, height: usize, n: usize) -> Result<()> {
    if s.len() < n || height > s.len() - n {
        return Err(trap("stack underflow"));
    }
    // Remove the values between the label height and the kept results, in
    // place: no temporary vector per branch.
    let start = s.len() - n;
    if height < start {
        s.drain(height..start);
    }
    Ok(())
}
impl Store {
    pub(super) fn frame(&self, f: Func, args: Vec<Value>, base: usize) -> Result<Frame> {
        self.frame_with_release(f, args, base, 0)
    }
    pub(super) fn frame_with_release(
        &self,
        f: Func,
        args: Vec<Value>,
        base: usize,
        releasing_slots: usize,
    ) -> Result<Frame> {
        let k = self.check(f.0, &self.funcs)?;
        let fun = self.funcs[k].clone();
        for value in &args {
            self.validate_value(*value)?;
        }
        if args.len() != fun.ty.params.len()
            || !args.iter().zip(&fun.ty.params).all(|(v, t)| match t {
                ValType::Ref(r) => {
                    matches_type(*v, *t) || self.gc_matches(fun.instance, *v, *r).unwrap_or(false)
                }
                _ => matches_type(*v, *t),
            })
        {
            return Err(err(ErrorKind::Host, "argument type mismatch"));
        }
        let body = &self.instances[fun.instance]
            .module
            .data
            .inner
            .decoded()
            .code[fun.body];
        let total_slots = args
            .len()
            .checked_add(body.locals.len())
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "active local slots limit"))?;
        self.check_local_slot_budget(total_slots, releasing_slots)?;
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::Frame, 1);
        let mut locals = args;
        let module = &self.instances[fun.instance].module;
        // Zero values in runs of one type, reserved fallibly at once.
        locals
            .try_reserve_exact(body.locals.len())
            .map_err(|_| err(ErrorKind::ResourceLimit, "frame allocation failed"))?;
        for &(zero, n) in module.data.local_runs(fun.body) {
            locals.resize(locals.len() + n as usize, zero);
        }
        #[cfg(feature = "compact-executor")]
        let compact = self.engine.policy == ExecutionPolicy::Compact
            && module.data.compact.as_ref().is_some_and(|program| {
                program.contains(fun.body)
                    && (!program.has_calls(fun.body)
                        || self.instances[fun.instance].call_bindings.is_some())
                    && (!program.has_memory(fun.body)
                        || self.instances[fun.instance].memory_bindings.is_some())
            });
        #[cfg(all(feature = "compact-executor", feature = "performance-counters"))]
        if self.engine.policy == ExecutionPolicy::Compact && !compact {
            self.perf.record(perf::Event::CompactFallbackFrame, 1);
        }
        #[allow(unused_mut)]
        let mut labels = vec![];
        #[cfg(all(
            feature = "register-executor",
            any(
                feature = "execution-reference",
                feature = "performance-counters",
                feature = "opcode-coverage"
            )
        ))]
        let register = if self.engine.policy == ExecutionPolicy::Register {
            register::activate_entry(self, fun.instance, fun.body, &locals, &mut labels)
        } else {
            None
        };
        #[cfg(all(
            feature = "register-executor",
            not(any(
                feature = "execution-reference",
                feature = "performance-counters",
                feature = "opcode-coverage"
            ))
        ))]
        let register = if self.engine.policy == ExecutionPolicy::Register
            && self.engine.register_entry_threshold.is_some()
        {
            register::activate_entry(self, fun.instance, fun.body, &locals, &mut labels)
        } else {
            None
        };
        #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
        if self.engine.policy == ExecutionPolicy::Register && register.is_none() {
            self.register_fallback_frames
                .set(self.register_fallback_frames.get().saturating_add(1));
        }
        #[cfg(all(feature = "register-executor", feature = "performance-counters"))]
        if self.engine.policy == ExecutionPolicy::Register && register.is_none() {
            self.perf.record(perf::Event::RegisterFallbackFrame, 1);
        }
        Ok(Frame {
            #[cfg(feature = "register-executor")]
            register,
            #[cfg(feature = "compact-executor")]
            compact,
            function: fun,
            pc: 0,
            locals,
            base,
            labels,
        })
    }
    /// Invokes a guest or host function handle with `args` inside this store.
    pub fn invoke(&mut self, f: Func, args: &[Value]) -> Result<RunOutcome> {
        #[cfg(feature = "execution-reference")]
        if self.engine.policy == ExecutionPolicy::Reference {
            return self.invoke_reference(f, args);
        }
        self.invoke_decoded(f, args)
    }
    pub(super) fn invoke_decoded(&mut self, f: Func, args: &[Value]) -> Result<RunOutcome> {
        let saved_slots = self.active_local_slots;
        let saved_labels = self.active_labels;
        let result = self.invoke_decoded_inner(f, args);
        self.active_local_slots = saved_slots;
        self.active_labels = saved_labels;
        result
    }
    fn invoke_decoded_inner(&mut self, f: Func, args: &[Value]) -> Result<RunOutcome> {
        #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
        let register_capacity_scope = self.register_activation.scope();
        if self.cancellation.is_cancelled() {
            return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
        }
        let k = self.check(f.0, &self.funcs)?;
        if let Some(host) = self.funcs[k].host.clone() {
            let ty = self.funcs[k].ty.clone();
            for value in args {
                self.validate_value(*value)?;
            }
            self.check_function_values(&self.funcs[k], args, &ty.params)?;
            #[cfg(feature = "performance-counters")]
            self.perf
                .record(perf::Event::Roots, u64::from(!args.is_empty()));
            self.active_roots.push(args.to_vec());
            let result = host(self, args);
            self.active_roots.pop();
            let out = match result? {
                RunOutcome::Suspended {
                    tag,
                    payload,
                    continuation,
                } => {
                    self.validate_host_suspension(&self.funcs[k], tag, &payload, continuation)?;
                    self.publish_values(&payload);
                    self.publish_values(&[Value::Ref(RefValue::Gc(continuation))]);
                    return Ok(RunOutcome::Suspended {
                        tag,
                        payload,
                        continuation,
                    });
                }
                RunOutcome::Returned(out) => out,
                RunOutcome::Exception(ex) => {
                    self.exception_tag(ex)?;
                    self.published.push(Value::Ref(RefValue::Exn(ex)));
                    return Ok(RunOutcome::Exception(ex));
                }
            };
            for value in &out {
                self.validate_value(*value)?;
            }
            self.check_function_values(&self.funcs[k], &out, &ty.results)?;
            self.publish_values(&out);
            return Ok(RunOutcome::Returned(out));
        }
        #[cfg(all(
            feature = "register-executor",
            not(any(
                feature = "execution-reference",
                feature = "performance-counters",
                feature = "opcode-coverage"
            ))
        ))]
        if self.engine.policy == ExecutionPolicy::Register
            && self.engine.register_entry_threshold.is_none()
            && self.engine.config.limits.fuel.is_none()
            && let Some(outcome) = register::fast::try_invoke_leaf(self, k, args)
        {
            return Ok(outcome);
        }
        #[cfg(feature = "performance-counters")]
        self.perf
            .record(perf::Event::Arguments, u64::from(!args.is_empty()));
        let first = self.frame(f, args.to_vec(), 0)?;
        self.acquire_local_slots(first.locals.len());
        let mut frames = vec![first];
        // Locals vectors of returned frames, reused by later calls that need
        // the same capacity, so that such a call does not allocate one. The
        // capacity must match: captured continuations are charged by it.
        let mut spare: Vec<Vec<Value>> = Vec::new();
        let mut stack = vec![];
        let mut fuel = self.engine.config.limits.fuel;
        let mut prompts = Vec::new();
        let mut module_cache: Option<(usize, Module)> = None;
        let mut module_stale = true;
        #[cfg(not(feature = "execution-reference"))]
        let mut precharged = false;
        // A straight-line run that hands an instruction back has already
        // counted it in the diagnostic counters.
        #[cfg(all(
            not(feature = "execution-reference"),
            any(feature = "performance-counters", feature = "opcode-coverage")
        ))]
        let mut counted = false;
        loop {
            #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
            register_capacity_scope.observe(&frames);
            #[cfg(feature = "register-executor")]
            let operand_height = if frames.last().is_some_and(|frame| frame.register.is_some()) {
                register::logical_height(frames.last().unwrap(), self)?
            } else {
                stack.len()
            };
            #[cfg(not(feature = "register-executor"))]
            let operand_height = stack.len();
            // A straight-line run may already have charged this instruction.
            #[cfg(not(feature = "execution-reference"))]
            let charge = !std::mem::take(&mut precharged);
            #[cfg(feature = "execution-reference")]
            let charge = true;
            if charge {
                accounting::next_source(
                    self,
                    &mut fuel,
                    frames.len(),
                    operand_height,
                    &mut prompts,
                )?;
            }
            #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
            if self.materialization_request.is_some()
                && frames.last().is_some_and(|frame| frame.register.is_some())
            {
                register::materialize(frames.last_mut().unwrap(), &mut stack, self)?;
            }
            #[cfg(feature = "execution-reference")]
            self.materialization_boundary(&mut frames, &mut stack, &mut prompts, &mut fuel)?;
            #[cfg(all(
                feature = "register-executor",
                not(any(
                    feature = "execution-reference",
                    feature = "performance-counters",
                    feature = "opcode-coverage"
                ))
            ))]
            if self.engine.policy == ExecutionPolicy::Register && charge {
                match register::fast::run(self, &mut frames, &mut stack, &mut fuel)? {
                    register::fast::FastOutcome::Declined => {}
                    register::fast::FastOutcome::Bailed => continue,
                    register::fast::FastOutcome::Returned => {
                        if let Some(f) = frames.pop() {
                            self.release_local_slots(f.locals.len());
                            self.release_labels(f.labels.len());
                            retire(&mut spare, f);
                        }
                        if frames.is_empty() {
                            for value in &stack {
                                self.validate_value(*value)?;
                            }
                            self.publish_values(&stack);
                            return Ok(RunOutcome::Returned(stack));
                        }
                        continue;
                    }
                }
            }
            #[cfg(feature = "compact-executor")]
            let frame_depth = frames.len();
            let frame = frames.last_mut().unwrap();
            let inst_id = frame.function.instance;
            #[cfg(feature = "register-executor")]
            if frame.register.is_some() {
                match register::step(frame, &mut stack, self)? {
                    register::Step::Continue => continue,
                    register::Step::ExitToDecoded => {}
                    register::Step::Return => {
                        if let Some(f) = frames.pop() {
                            self.release_local_slots(f.locals.len());
                            self.release_labels(f.labels.len());
                        }
                        if frames.is_empty() {
                            for value in &stack {
                                self.validate_value(*value)?;
                            }
                            self.publish_values(&stack);
                            return Ok(RunOutcome::Returned(stack));
                        }
                        continue;
                    }
                }
            }
            // A Register call barrier may have released its vectors. Refresh
            // before a host callback can enter another invocation scope.
            #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
            let frame = {
                register_capacity_scope.observe(&frames);
                frames.last_mut().unwrap()
            };
            // Instructions are borrowed from a module handle held outside the
            // store, refreshed only when execution moves to another module, so
            // dispatch (including the experimental compact tier) neither clones
            // the handle nor the instruction on steady-state steps.
            // Instances are append-only while published, so the cache is keyed
            // by instance index. After an instruction that may run host code
            // (which can instantiate or roll back unpublished instances) the
            // handle is compared again before reuse.
            if module_stale || module_cache.as_ref().is_none_or(|(id, _)| *id != inst_id) {
                let current = &self.instances[inst_id].module;
                if !module_cache
                    .as_ref()
                    .is_some_and(|(_, cached)| Arc::ptr_eq(&cached.data, &current.data))
                {
                    module_cache = Some((inst_id, current.clone()));
                } else if let Some(cached) = module_cache.as_mut() {
                    cached.0 = inst_id;
                }
                #[cfg(feature = "compact-executor")]
                {
                    module_stale = false;
                }
            }
            let module: &Module = &module_cache
                .as_ref()
                .expect("module cache was just filled")
                .1;
            #[cfg(feature = "compact-executor")]
            let mut prepared_instruction = None;
            #[cfg(feature = "compact-executor")]
            let mut bound_call = None;
            #[cfg(feature = "compact-executor")]
            if frame.compact {
                let pc = frame.pc;
                frame.pc = pc
                    .checked_add(1)
                    .ok_or_else(|| trap("compact instruction pointer overflow"))?;
                #[cfg(feature = "execution-reference")]
                {
                    self.compact_dispatches = self.compact_dispatches.saturating_add(1);
                    if module
                        .data
                        .compact
                        .as_ref()
                        .is_some_and(|p| p.is_prefix_step(frame.function.body, pc))
                    {
                        self.compact_prefix_dispatches =
                            self.compact_prefix_dispatches.saturating_add(1);
                    }
                }
                let step = module
                    .data
                    .compact
                    .as_ref()
                    .ok_or_else(|| trap("missing admitted compact program"))?
                    .step(frame.function.body, pc, frame, &mut stack, self)?;
                match step {
                    compact::Step::Exit { pc, opcode } => {
                        // next_source has charged this pending source exactly
                        // once. Canonical locals/operands/labels/prompts/fuel
                        // stay in this loop; the decoded handler executes once.
                        frame.compact = false;
                        let original = self.instances[inst_id].module.data.inner.decoded().code
                            [frame.function.body]
                            .instructions
                            .get(pc)
                            .ok_or_else(|| trap("compact exit source cursor"))?;
                        if original.opcode != opcode {
                            return Err(trap("compact exit source identity"));
                        }
                        #[cfg(feature = "execution-reference")]
                        {
                            self.compact_dispatches = self.compact_dispatches.saturating_sub(1);
                            let family = match opcode {
                                0xfb00..=0xfbff | 0xd0..=0xd6 => 0,
                                0x08..=0x0a | 0x1f => 1,
                                0xe0..=0xe6 => 2,
                                _ => 3,
                            };
                            self.compact_exits[family] =
                                self.compact_exits[family].saturating_add(1);
                            self.compact_last_exit = Some((pc, opcode));
                        }
                        let op = original.clone();
                        #[cfg(feature = "performance-counters")]
                        {
                            self.perf.record(perf::Event::Dispatch, 1);
                            self.perf.record(perf::Event::InstructionClone, 1);
                            self.perf.record(
                                perf::Event::InstructionBytes,
                                perf::immediate_copy_bytes(&op.immediate),
                            );
                        }
                        #[cfg(feature = "opcode-coverage")]
                        crate::coverage::record(op.opcode);
                        prepared_instruction = Some((op, pc));
                    }
                    compact::Step::Continue => continue,
                    compact::Step::Fusion(pattern) => {
                        compact::execute_fusion(
                            pattern,
                            frame,
                            &mut stack,
                            self,
                            &mut fuel,
                            frame_depth,
                            &mut prompts,
                        )?;
                        continue;
                    }
                    compact::Step::Memory(access) => {
                        self.compact_memory_access(inst_id, access, &mut stack)?;
                        continue;
                    }
                    compact::Step::Call { index, tail } => {
                        let instance = &self.instances[inst_id];
                        let binding = instance
                            .call_bindings
                            .as_ref()
                            .ok_or_else(|| trap("missing compact instance bindings"))?
                            .resolve(self, instance, index)?;
                        let source = instance.module.data.inner.decoded().code[frame.function.body]
                            .instructions
                            .get(pc)
                            .ok_or_else(|| trap("compact call source cursor"))?;
                        let opcode = if tail { 0x12 } else { 0x10 };
                        if source.opcode != opcode
                            || !matches!(source.immediate, Immediate::Index(value) if value == index)
                        {
                            return Err(trap("compact call source identity"));
                        }
                        prepared_instruction = Some((
                            Instruction {
                                opcode,
                                immediate: Immediate::Index(index),
                            },
                            pc,
                        ));
                        bound_call = Some(binding);
                        #[cfg(feature = "execution-reference")]
                        {
                            self.compact_bound_calls = self.compact_bound_calls.saturating_add(1);
                        }
                        #[cfg(feature = "performance-counters")]
                        self.perf.record(perf::Event::CompactBoundCall, 1);
                    }
                    compact::Step::Return => {
                        let n = frame.function.ty.results.len();
                        let base = frame.base;
                        compact::keep(&mut stack, base, n)?;
                        #[cfg(feature = "performance-counters")]
                        self.perf
                            .record(perf::Event::CompactResultMove, u64::from(n != 0));
                        if let Some(f) = frames.pop() {
                            self.release_local_slots(f.locals.len());
                            self.release_labels(f.labels.len());
                        }
                        if frames.is_empty() {
                            for value in &stack {
                                self.validate_value(*value)?;
                            }
                            self.publish_values(&stack);
                            return Ok(RunOutcome::Returned(stack));
                        }
                        continue;
                    }
                }
            }
            let ops = &module.data.inner.decoded().code[frame.function.body].instructions;
            // Straight-line instructions of a plain Decoded frame run without
            // returning here. The reference build keeps every instruction on
            // this path for its per-boundary diagnostics.
            #[cfg(not(feature = "execution-reference"))]
            {
                #[cfg(feature = "compact-executor")]
                let plain = prepared_instruction.is_none() && !frame.compact;
                #[cfg(not(feature = "compact-executor"))]
                let plain = true;
                #[cfg(feature = "register-executor")]
                let plain = plain && frame.register.is_none();
                // An instruction a run handed back is not a straight-line one.
                if plain && charge {
                    match self.run_straight(inst_id, frame, module, ops, &mut stack, &mut fuel)? {
                        straight::Straight::Declined => {
                            #[cfg(any(
                                feature = "performance-counters",
                                feature = "opcode-coverage"
                            ))]
                            {
                                counted = true;
                            }
                        }
                        straight::Straight::Ran => continue,
                        straight::Straight::Charged => {
                            precharged = true;
                            #[cfg(any(
                                feature = "performance-counters",
                                feature = "opcode-coverage"
                            ))]
                            {
                                counted = true;
                            }
                            continue;
                        }
                    }
                }
            }
            #[cfg(feature = "compact-executor")]
            let (op, pc): (&Instruction, usize) = match &prepared_instruction {
                Some((op, pc)) => (op, *pc),
                None => {
                    let op = ops
                        .get(frame.pc)
                        .ok_or_else(|| trap("instruction pointer out of bounds"))?;
                    #[cfg(all(
                        not(feature = "execution-reference"),
                        any(feature = "performance-counters", feature = "opcode-coverage")
                    ))]
                    let count = !std::mem::take(&mut counted);
                    #[cfg(all(
                        feature = "execution-reference",
                        any(feature = "performance-counters", feature = "opcode-coverage")
                    ))]
                    let count = true;
                    #[cfg(feature = "performance-counters")]
                    if count {
                        self.perf.record(perf::Event::Dispatch, 1);

                        self.perf.record(perf::Event::InstructionClone, 1);
                        self.perf.record(
                            perf::Event::InstructionBytes,
                            perf::immediate_copy_bytes(&op.immediate),
                        );
                    }
                    #[cfg(feature = "opcode-coverage")]
                    if count {
                        crate::coverage::record(op.opcode);
                    }
                    let pc = frame.pc;
                    frame.pc += 1;
                    (op, pc)
                }
            };
            #[cfg(not(feature = "compact-executor"))]
            let (op, pc) = {
                let op = ops
                    .get(frame.pc)
                    .ok_or_else(|| trap("instruction pointer out of bounds"))?;
                #[cfg(all(
                    not(feature = "execution-reference"),
                    any(feature = "performance-counters", feature = "opcode-coverage")
                ))]
                let count = !std::mem::take(&mut counted);
                #[cfg(all(
                    feature = "execution-reference",
                    any(feature = "performance-counters", feature = "opcode-coverage")
                ))]
                let count = true;
                #[cfg(feature = "performance-counters")]
                if count {
                    self.perf.record(perf::Event::Dispatch, 1);

                    self.perf.record(perf::Event::InstructionClone, 1);
                    self.perf.record(
                        perf::Event::InstructionBytes,
                        perf::immediate_copy_bytes(&op.immediate),
                    );
                }
                #[cfg(feature = "opcode-coverage")]
                if count {
                    crate::coverage::record(op.opcode);
                }
                let pc = frame.pc;
                frame.pc += 1;
                (op, pc)
            };
            // Calls may reach host code; continuation resumption may too.
            module_stale = matches!(op.opcode, 0x10..=0x15 | 0xe0..=0xe6);
            match op.opcode {
                0x1f => {
                    let Immediate::TryTable(table) = &op.immediate else {
                        return Err(trap("try_table immediate"));
                    };
                    let (a, r) = arity(&table.block, module);
                    let (end, _) = module
                        .data
                        .control_get(frame.function.body, pc)
                        .ok_or_else(|| trap("missing control boundary"))?;
                    let height = stack
                        .len()
                        .checked_sub(a)
                        .ok_or_else(|| trap("try_table stack underflow"))?;
                    self.push_label(
                        &mut frame.labels,
                        Label {
                            try_table_pc: Some(pc),
                            start: pc + 1,
                            end,
                            height,
                            params: a,
                            results: r,
                            loop_: false,
                        },
                    )?;
                }
                0x08 | 0x0a => {
                    let ex = if op.opcode == 0x08 {
                        let Immediate::Index(n) = op.immediate else {
                            return Err(trap("throw immediate"));
                        };
                        let tag = self.instances[inst_id].tags[n as usize];
                        let arity = self.tags[tag.0.index].params.len();
                        let payload = stack.split_off(
                            stack
                                .len()
                                .checked_sub(arity)
                                .ok_or_else(|| trap("throw payload underflow"))?,
                        );
                        self.allocate_exception(tag, &payload)?
                    } else {
                        match pop(&mut stack)? {
                            Value::Ref(RefValue::Exn(ex)) => {
                                self.exception_tag(ex)?;
                                ex
                            }
                            _ => return Err(trap("null exception reference")),
                        }
                    };
                    if !self.unwind_exception(ex, &mut frames, &mut stack)? {
                        self.published.push(Value::Ref(RefValue::Exn(ex)));
                        return Ok(RunOutcome::Exception(ex));
                    }
                    if frames.is_empty() {
                        for value in &stack {
                            self.validate_value(*value)?;
                        }
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0 => return Err(trap("unreachable")),
                1 => {}
                2..=4 => {
                    let (a, r) = arity(&op.immediate, module);
                    let cond = if op.opcode == 4 {
                        address(pop(&mut stack)?)? != 0
                    } else {
                        true
                    };
                    let (end, other) = module
                        .data
                        .control_get(frame.function.body, pc)
                        .ok_or_else(|| trap("missing control boundary"))?;
                    let height = stack
                        .len()
                        .checked_sub(a)
                        .ok_or_else(|| trap("block stack underflow"))?;
                    self.push_label(
                        &mut frame.labels,
                        Label {
                            try_table_pc: None,
                            start: pc + 1,
                            end,
                            height,
                            params: a,
                            results: r,
                            loop_: op.opcode == 3,
                        },
                    )?;
                    if !cond {
                        frame.pc = other.map_or(end, |x| x + 1)
                    }
                }
                5 => {
                    frame.pc = frame
                        .labels
                        .last()
                        .ok_or_else(|| trap("else without label"))?
                        .end
                }
                0xb => {
                    if let Some(l) = frame.labels.pop() {
                        self.release_labels(1);
                        #[cfg(feature = "performance-counters")]
                        self.perf
                            .record(perf::Event::Branch, u64::from(l.results != 0));
                        keep(&mut stack, l.height, l.results)?;
                    } else {
                        let n = frame.function.ty.results.len();
                        let base = frame.base;
                        #[cfg(feature = "performance-counters")]
                        self.perf.record(perf::Event::Branch, u64::from(n != 0));
                        keep(&mut stack, base, n)?;
                        if let Some(f) = frames.pop() {
                            self.release_local_slots(f.locals.len());
                            self.release_labels(f.labels.len());
                            retire(&mut spare, f)
                        }
                        if frames.is_empty() {
                            for value in &stack {
                                self.validate_value(*value)?;
                            }
                            self.publish_values(&stack);
                            return Ok(RunOutcome::Returned(stack));
                        }
                    }
                }
                0xc..=0xe => {
                    let depth = match &op.immediate {
                        Immediate::Index(x) => {
                            if op.opcode == 0xd && address(pop(&mut stack)?)? == 0 {
                                continue;
                            }
                            *x
                        }
                        Immediate::BrTable(v, d) => {
                            let n = address(pop(&mut stack)?)?;
                            v.get(n as usize).copied().unwrap_or(*d)
                        }
                        _ => return Err(trap("branch immediate")),
                    } as usize;
                    if depth == frame.labels.len() {
                        let n = frame.function.ty.results.len();
                        #[cfg(feature = "performance-counters")]
                        self.perf.record(perf::Event::Branch, u64::from(n != 0));
                        keep(&mut stack, frame.base, n)?;
                        if let Some(f) = frames.pop() {
                            self.release_local_slots(f.locals.len());
                            self.release_labels(f.labels.len());
                            retire(&mut spare, f)
                        }
                        if frames.is_empty() {
                            for value in &stack {
                                self.validate_value(*value)?;
                            }
                            self.publish_values(&stack);
                            return Ok(RunOutcome::Returned(stack));
                        }
                    } else {
                        let pos = frame
                            .labels
                            .len()
                            .checked_sub(depth + 1)
                            .ok_or_else(|| trap("branch depth"))?;
                        let l = &frame.labels[pos];
                        keep(
                            &mut stack,
                            l.height,
                            if l.loop_ { l.params } else { l.results },
                        )?;
                        frame.pc = if l.loop_ { l.start } else { l.end + 1 };
                        let retain = pos + usize::from(l.loop_);
                        let removed = frame.labels.len() - retain;
                        frame.labels.truncate(retain);
                        self.release_labels(removed);
                    }
                }
                0xf => {
                    #[cfg(feature = "performance-counters")]
                    self.perf.record(
                        perf::Event::Branch,
                        u64::from(!frame.function.ty.results.is_empty()),
                    );
                    keep(&mut stack, frame.base, frame.function.ty.results.len())?;
                    if let Some(f) = frames.pop() {
                        self.release_local_slots(f.locals.len());
                        self.release_labels(f.labels.len());
                        retire(&mut spare, f)
                    }
                    if frames.is_empty() {
                        for value in &stack {
                            self.validate_value(*value)?;
                        }
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0x10..=0x15 => {
                    let fun = match op.immediate {
                        Immediate::Index(x) if op.opcode == 0x10 || op.opcode == 0x12 => {
                            #[cfg(feature = "compact-executor")]
                            {
                                bound_call.map_or_else(
                                    || self.instances[inst_id].funcs[x as usize],
                                    |binding| binding.0,
                                )
                            }
                            #[cfg(not(feature = "compact-executor"))]
                            {
                                self.instances[inst_id].funcs[x as usize]
                            }
                        }
                        Immediate::Pair(t, table) => {
                            let ix = address(pop(&mut stack)?)?;
                            let tab = self.instances[inst_id].tabs[table as usize];
                            let val = *self.tabs[tab.0.index]
                                .values
                                .get(usize::try_from(ix).map_err(|_| trap("table out of bounds"))?)
                                .ok_or_else(|| trap("table out of bounds"))?;
                            let Value::Ref(RefValue::Func(f)) = val else {
                                return Err(trap("null indirect call"));
                            };
                            #[cfg(feature = "compact-executor")]
                            let compatible = self.compact_indirect_compatible(
                                inst_id,
                                module,
                                frame.function.body,
                                pc,
                                op.opcode,
                                t as usize,
                                f,
                            )?;
                            #[cfg(not(feature = "compact-executor"))]
                            let compatible = self
                                .engine
                                .canonical
                                .lock()
                                .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                                .is_subtype(
                                    self.funcs[self.check(f.0, &self.funcs)?].canonical,
                                    module.data.canonical[t as usize],
                                    false,
                                    self.engine.config.limits.max_type_work,
                                )?;
                            if !compatible {
                                return Err(trap("indirect call type mismatch"));
                            }
                            f
                        }
                        Immediate::Index(t) => {
                            let Value::Ref(RefValue::Func(f)) = pop(&mut stack)? else {
                                return Err(trap("null call_ref"));
                            };
                            #[cfg(feature = "compact-executor")]
                            let compatible = self.compact_indirect_compatible(
                                inst_id,
                                module,
                                frame.function.body,
                                pc,
                                op.opcode,
                                t as usize,
                                f,
                            )?;
                            #[cfg(not(feature = "compact-executor"))]
                            let compatible = self
                                .engine
                                .canonical
                                .lock()
                                .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                                .is_subtype(
                                    self.funcs[self.check(f.0, &self.funcs)?].canonical,
                                    module.data.canonical[t as usize],
                                    false,
                                    self.engine.config.limits.max_type_work,
                                )?;
                            if !compatible {
                                return Err(trap("call_ref type mismatch"));
                            }
                            f
                        }
                        _ => return Err(trap("call immediate")),
                    };
                    let checked = self.check(fun.0, &self.funcs)?;
                    #[cfg(feature = "compact-executor")]
                    let n =
                        bound_call.map_or(self.funcs[checked].ty.params.len(), |binding| binding.1);
                    #[cfg(not(feature = "compact-executor"))]
                    let n = self.funcs[checked].ty.params.len();
                    if n > stack.len() {
                        return Err(trap("call stack underflow"));
                    }
                    #[cfg(feature = "compact-executor")]
                    if bound_call.is_some() && self.funcs[checked].host.is_none() {
                        let start = stack.len() - n;
                        let tail = matches!(op.opcode, 0x12 | 0x13 | 0x15);
                        let base = if tail { frame.base } else { start };
                        let releasing = if tail { frame.locals.len() } else { 0 };
                        if !tail && frames.len() >= self.engine.config.limits.max_call_depth {
                            return Err(err(ErrorKind::ResourceLimit, "call depth exhausted"));
                        }
                        if !tail {
                            let mut stopped = None;
                            match compact::try_inline_leaf(
                                self,
                                fun,
                                inst_id,
                                &mut stack,
                                &mut fuel,
                                frame_depth,
                                &mut prompts,
                                &mut stopped,
                            ) {
                                Ok(true) => continue,
                                Ok(false) => {}
                                Err(error) => {
                                    if let Some(callee) = stopped {
                                        frames.push(callee);
                                    }
                                    return Err(error);
                                }
                            }
                        }
                        let next = compact::guest_frame_with_release(
                            self,
                            fun,
                            &stack[start..],
                            base,
                            releasing,
                        )?;
                        let added = next.locals.len();
                        if tail {
                            stack.truncate(base);
                            if let Some(popped) = frames.pop() {
                                self.release_local_slots(popped.locals.len());
                                self.release_labels(popped.labels.len());
                            }
                        } else {
                            stack.truncate(start);
                        }
                        self.acquire_local_slots(added);
                        frames.push(next);
                        continue;
                    }
                    #[cfg(feature = "performance-counters")]
                    self.perf.record(perf::Event::Arguments, u64::from(n != 0));
                    // The callee's frame takes this vector as its locals:
                    // reserve room for its declared locals now so that the
                    // frame does not reallocate.
                    let declared = if self.funcs[checked].host.is_none() {
                        let callee = &self.funcs[checked];
                        self.instances[callee.instance]
                            .module
                            .data
                            .inner
                            .decoded()
                            .code
                            .get(callee.body)
                            .map_or(0, |body| body.locals.len())
                    } else {
                        0
                    };
                    let tail = matches!(op.opcode, 0x12 | 0x13 | 0x15);
                    let releasing = if tail { frame.locals.len() } else { 0 };
                    let total_needed = n
                        .checked_add(declared)
                        .ok_or_else(|| err(ErrorKind::ResourceLimit, "active local slots limit"))?;
                    if self.funcs[checked].host.is_none() {
                        self.check_local_slot_budget(total_needed, releasing)?;
                    }
                    let start = stack.len() - n;
                    let mut args = match spare.iter().rposition(|v| v.capacity() == total_needed) {
                        Some(i) => spare.swap_remove(i),
                        None => {
                            let mut v = Vec::new();
                            v.try_reserve_exact(total_needed).map_err(|_| {
                                err(ErrorKind::ResourceLimit, "frame allocation failed")
                            })?;
                            v
                        }
                    };
                    args.extend_from_slice(&stack[start..]);
                    stack.truncate(start);
                    if let Some(host) = self.funcs[fun.0.index].host.clone() {
                        let ty = self.funcs[fun.0.index].ty.clone();
                        for value in &args {
                            self.validate_value(*value)?;
                        }
                        self.check_function_values(&self.funcs[fun.0.index], &args, &ty.params)?;
                        let caller_base = frame.base;
                        let caller_results = frame.function.ty.results.len();
                        // Collection only marks references, so only they are
                        // retained; scalars would make every host call copy the
                        // whole operand stack and every frame's locals.
                        let is_ref = |value: &&Value| matches!(value, Value::Ref(_));
                        let mut roots: Vec<Value> = stack.iter().filter(is_ref).copied().collect();
                        roots.extend(args.iter().filter(is_ref).copied());
                        roots.extend(
                            frames
                                .iter()
                                .flat_map(|f| f.locals.iter().filter(is_ref).copied()),
                        );
                        #[cfg(feature = "performance-counters")]
                        self.perf
                            .record(perf::Event::Roots, u64::from(!roots.is_empty()));
                        self.active_roots.push(roots);
                        let result = host(self, &args);
                        self.active_roots.pop();
                        let out = match result? {
                            RunOutcome::Suspended { .. } => {
                                return Err(err(
                                    ErrorKind::Host,
                                    "host suspension cannot cross an active guest call",
                                ));
                            }
                            RunOutcome::Returned(out) => out,
                            RunOutcome::Exception(ex) => {
                                if !self.unwind_exception(ex, &mut frames, &mut stack)? {
                                    self.published.push(Value::Ref(RefValue::Exn(ex)));
                                    return Ok(RunOutcome::Exception(ex));
                                }
                                if frames.is_empty() {
                                    for value in &stack {
                                        self.validate_value(*value)?;
                                    }
                                    self.publish_values(&stack);
                                    return Ok(RunOutcome::Returned(stack));
                                }
                                continue;
                            }
                        };
                        for value in &out {
                            self.validate_value(*value)?;
                        }
                        self.check_function_values(&self.funcs[fun.0.index], &out, &ty.results)?;
                        stack.extend(out);
                        if matches!(op.opcode, 0x12 | 0x13 | 0x15) {
                            #[cfg(feature = "performance-counters")]
                            self.perf
                                .record(perf::Event::Branch, u64::from(caller_results != 0));
                            keep(&mut stack, caller_base, caller_results)?;
                            if let Some(popped) = frames.pop() {
                                self.release_local_slots(popped.locals.len());
                                self.release_labels(popped.labels.len());
                            }
                            if frames.is_empty() {
                                for value in &stack {
                                    self.validate_value(*value)?;
                                }
                                self.publish_values(&stack);
                                return Ok(RunOutcome::Returned(stack));
                            }
                        }
                        continue;
                    }
                    let base = if tail { frame.base } else { stack.len() };
                    let next = self.frame_with_release(fun, args, base, releasing)?;
                    let added = next.locals.len();
                    if tail {
                        stack.truncate(base);
                        if let Some(popped) = frames.pop() {
                            self.release_local_slots(popped.locals.len());
                            self.release_labels(popped.labels.len());
                        }
                    } else if frames.len() >= self.engine.config.limits.max_call_depth {
                        return Err(err(ErrorKind::ResourceLimit, "call depth exhausted"));
                    }
                    self.acquire_local_slots(added);
                    frames.push(next)
                }
                0x1a => {
                    pop(&mut stack)?;
                }
                0x1b | 0x1c => {
                    let c = address(pop(&mut stack)?)?;
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(if c != 0 { a } else { b })
                }
                // The most frequent instructions, without the shared arm's
                // second dispatch.
                0x20 => {
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("index immediate"));
                    };
                    let value = *frame
                        .locals
                        .get(x as usize)
                        .ok_or_else(|| trap("local index"))?;
                    stack.push(value)
                }
                0x21 => {
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("index immediate"));
                    };
                    let value = pop(&mut stack)?;
                    *frame
                        .locals
                        .get_mut(x as usize)
                        .ok_or_else(|| trap("local index"))? = value
                }
                0x22 => {
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("index immediate"));
                    };
                    let value = *stack.last().ok_or_else(|| trap("empty stack"))?;
                    *frame
                        .locals
                        .get_mut(x as usize)
                        .ok_or_else(|| trap("local index"))? = value
                }
                0x23..=0x26 => {
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("index immediate"));
                    };
                    let x = x as usize;
                    match op.opcode {
                        0x23 => stack.push(self.get_global(self.instances[inst_id].globals[x])?),
                        0x24 => {
                            let v = pop(&mut stack)?;
                            self.set_global(self.instances[inst_id].globals[x], v)?
                        }
                        0x25 => {
                            let ix = address(pop(&mut stack)?)?;
                            let tab = &self.tabs[self.instances[inst_id].tabs[x].0.index];
                            stack.push(
                                *tab.values
                                    .get(ix as usize)
                                    .ok_or_else(|| trap("table out of bounds"))?,
                            )
                        }
                        _ => {
                            let v = pop(&mut stack)?;
                            let ix = address(pop(&mut stack)?)?;
                            let tab = &mut self.tabs[self.instances[inst_id].tabs[x].0.index];
                            *tab.values
                                .get_mut(ix as usize)
                                .ok_or_else(|| trap("table out of bounds"))? = v
                        }
                    }
                }
                0x28..=0x3e => self.load_store(inst_id, op, &mut stack)?,
                0x3f | 0x40 => {
                    let remaining_memory = self
                        .engine
                        .config
                        .limits
                        .max_store_memory_bytes
                        .saturating_sub(self.memory_usage()?);
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("memory index"));
                    };
                    let mem = &mut self.mems[self.instances[inst_id].mems[x as usize].0.index];
                    let old = (mem.len()? as u64) >> mem.ty.page_size_log2;
                    let wide = mem.ty.index_type == IndexType::I64;
                    let mut result = old;
                    if op.opcode == 0x40 {
                        let delta = address(pop(&mut stack)?)?;
                        let new = old.checked_add(delta);
                        if let Some(n) = new.filter(|n| {
                            *n <= mem.ty.limits.max.unwrap_or(u64::MAX)
                                && *n <= mem.ty.addressable_pages()
                                && *n <= self.engine.config.limits.max_memory_pages
                        }) {
                            if let Some(bytes) = n
                                .checked_mul(
                                    1u64.checked_shl(mem.ty.page_size_log2 as u32).unwrap_or(0),
                                )
                                .filter(|n| {
                                    *n <= self.engine.config.limits.max_memory_bytes
                                        && n.saturating_sub(old << mem.ty.page_size_log2)
                                            <= remaining_memory
                                })
                                .and_then(|n| usize::try_from(n).ok())
                            {
                                if let Some(shared) = &mem.shared {
                                    if !shared.grow(bytes)? {
                                        result = u64::MAX
                                    }
                                } else if !mem.grow_unshared(bytes) {
                                    result = u64::MAX
                                }
                            } else {
                                result = u64::MAX
                            }
                        } else {
                            result = u64::MAX
                        }
                    }
                    if op.opcode == 0x40 && result != u64::MAX {
                        mem.ty.limits.min = (mem.len()? as u64) >> mem.ty.page_size_log2;
                    }
                    stack.push(if wide {
                        Value::I64(result as i64)
                    } else {
                        Value::I32(result as i32)
                    })
                }
                0x41 => {
                    if let Immediate::I32(x) = op.immediate {
                        stack.push(Value::I32(x))
                    }
                }
                0x42 => {
                    if let Immediate::I64(x) = op.immediate {
                        stack.push(Value::I64(x))
                    }
                }
                0x43 => {
                    if let Immediate::F32(x) = op.immediate {
                        stack.push(Value::F32(x))
                    }
                }
                0x44 => {
                    if let Immediate::F64(x) = op.immediate {
                        stack.push(Value::F64(x))
                    }
                }
                0xd0 => {
                    if let Immediate::Heap(h) = op.immediate {
                        stack.push(Value::Ref(RefValue::Null(h)))
                    }
                }
                0xd1 => {
                    let v = pop(&mut stack)?;
                    stack.push(Value::I32(matches!(v, Value::Ref(RefValue::Null(_))) as i32))
                }
                0xd2 => {
                    if let Immediate::Index(x) = op.immediate {
                        stack.push(Value::Ref(RefValue::Func(
                            self.instances[inst_id].funcs[x as usize],
                        )))
                    }
                }
                0xd3 => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(Value::I32(
                        (a == b
                            || matches!(
                                (a, b),
                                (Value::Ref(RefValue::Null(_)), Value::Ref(RefValue::Null(_)))
                            )) as i32,
                    ))
                }
                0xd4 => {
                    if matches!(stack.last(), Some(Value::Ref(RefValue::Null(_)))) {
                        return Err(trap("null reference"));
                    }
                }
                0xd5 | 0xd6 => {
                    let Immediate::Index(label) = op.immediate else {
                        return Err(trap("reference branch immediate"));
                    };
                    let value = pop(&mut stack)?;
                    let null = matches!(value, Value::Ref(RefValue::Null(_)));
                    if op.opcode == 0xd6 && !null || op.opcode == 0xd5 && !null {
                        stack.push(value)
                    }
                    if null == (op.opcode == 0xd5) {
                        self.branch_handler(&mut frames, &mut stack, label as usize)?;
                    }
                    if frames.is_empty() {
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0xe0..=0xe6 => {
                    if let Some(ex) =
                        self.cont_step(inst_id, op, &mut frames, &mut stack, &mut prompts)?
                    {
                        self.published.push(Value::Ref(RefValue::Exn(ex)));
                        return Ok(RunOutcome::Exception(ex));
                    }
                    if frames.is_empty() {
                        if let Some((tag, payload, continuation)) = self.pending_suspension.take() {
                            self.publish_values(&payload);
                            self.publish_values(&[Value::Ref(RefValue::Gc(continuation))]);
                            return Ok(RunOutcome::Suspended {
                                tag,
                                payload,
                                continuation,
                            });
                        }
                        for value in &stack {
                            self.validate_value(*value)?;
                        }
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0xfb18 | 0xfb19 | 0xfb25 | 0xfb26 => {
                    let Immediate::Cast {
                        flags, label, to, ..
                    } = op.immediate
                    else {
                        return Err(trap("cast branch immediate"));
                    };
                    let descriptor = if op.opcode >= 0xfb25 {
                        match pop(&mut stack)? {
                            Value::Ref(RefValue::Gc(r)) => Some(r),
                            _ => return Err(trap("null descriptor")),
                        }
                    } else {
                        None
                    };
                    let value = pop(&mut stack)?;
                    let target = RefType::new(flags & 2 != 0, to);
                    let success = if let Some(d) = descriptor {
                        self.gc_descriptor_matches(inst_id, value, d, target)?
                    } else {
                        self.gc_matches(inst_id, value, target)?
                    };
                    stack.push(value);
                    if success == (op.opcode == 0xfb18 || op.opcode == 0xfb25) {
                        self.branch_handler(&mut frames, &mut stack, label as usize)?;
                    }
                    if frames.is_empty() {
                        for value in &stack {
                            self.validate_value(*value)?;
                        }
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0xfb00..=0xfb26 => self.gc_exec(inst_id, op, &mut stack)?,
                0xfe00..=0xfe4e => self.atomic(inst_id, op, &mut stack)?,
                0xfc13..=0xfc16 => crate::wide::execute(op.opcode, &mut stack)?,
                0xfc08..=0xfc11 => self.bulk(inst_id, op, &mut stack)?,
                0xfd00..=0xfd0b | 0xfd54..=0xfd5d => self.simd_memory(inst_id, op, &mut stack)?,
                n if n & 0xff00 == 0xfd00 || n & 0xff00 == 0xfa00 => {
                    crate::simd::execute(n, &op.immediate, &mut stack)?
                }
                n => numeric::exec(n, &mut stack)?,
            }
        }
    }
    pub(super) fn load_store(
        &mut self,
        ii: usize,
        op: &Instruction,
        s: &mut Vec<Value>,
    ) -> Result<()> {
        let Immediate::MemArg { memory, .. } = op.immediate else {
            return Err(trap("memory argument"));
        };
        let index = self.instances[ii].mems[memory as usize].0.index;
        self.memory_access(index, op, s)
    }
    /// A scalar load or store on the memory at store index `index`, which the
    /// caller resolved from the instruction's memory immediate.
    #[inline]
    pub(super) fn memory_access(
        &mut self,
        index: usize,
        op: &Instruction,
        s: &mut Vec<Value>,
    ) -> Result<()> {
        let Immediate::MemArg { offset, .. } = op.immediate else {
            return Err(trap("memory argument"));
        };
        self.memory_access_raw(index, op.opcode, offset, s)
    }
    /// `memory_access` with the opcode and static offset already decoded.
    #[inline(always)]
    pub(super) fn memory_access_raw(
        &mut self,
        index: usize,
        code: u16,
        offset: u64,
        s: &mut Vec<Value>,
    ) -> Result<()> {
        // Fixed-width accesses to an unshared memory that stay within one
        // allocated page; everything else, with every error, falls through.
        if code < 0x36 {
            if let Some(slot) = s.last_mut()
                && let Some(v) = self.load_fast(index, code, *slot, offset)
            {
                *slot = v;
                return Ok(());
            }
        } else if let [.., addr, value] = s.as_slice()
            && self.store_fast(index, code, *addr, *value, offset)
        {
            s.truncate(s.len() - 2);
            return Ok(());
        }
        self.memory_slow(index, code, offset, s)
    }
    /// The value of load `code` at `addr + offset` on the memory at store
    /// index `index`, when the memory is unshared, the access is a common
    /// fixed-width one, in bounds and within one allocated or unwritten page,
    /// and `addr` is an address; otherwise None and nothing is recorded.
    #[inline(always)]
    pub(super) fn load_fast(
        &mut self,
        index: usize,
        code: u16,
        addr: Value,
        offset: u64,
    ) -> Option<Value> {
        let mem = &self.mems[index];
        if mem.shared.is_some() {
            return None;
        }
        let a = match addr {
            Value::I32(x) => x as u32 as u64,
            Value::I64(x) => x as u64,
            _ => return None,
        };
        let at = usize::try_from(a.checked_add(offset)?).ok()?;
        let bytes = &mem.bytes;
        let v = match code {
            0x28 => Value::I32(i32::from_le_bytes(bytes.read_array::<4>(at)?)),
            0x29 => Value::I64(i64::from_le_bytes(bytes.read_array::<8>(at)?)),
            0x2a => Value::F32(u32::from_le_bytes(bytes.read_array::<4>(at)?)),
            0x2b => Value::F64(u64::from_le_bytes(bytes.read_array::<8>(at)?)),
            0x2d => Value::I32(bytes.read_u8_fast(at)? as i32),
            0x2c => Value::I32(bytes.read_u8_fast(at)? as i8 as i32),
            0x2e => Value::I32(i16::from_le_bytes(bytes.read_array::<2>(at)?) as i32),
            0x2f => Value::I32(u16::from_le_bytes(bytes.read_array::<2>(at)?) as i32),
            0x35 => Value::I64(u32::from_le_bytes(bytes.read_array::<4>(at)?) as i64),
            0x34 => Value::I64(i32::from_le_bytes(bytes.read_array::<4>(at)?) as i64),
            0x31 => Value::I64(bytes.read_u8_fast(at)? as i64),
            _ => return None,
        };
        // The load event the general path records.
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::ReadVec, 1);
        Some(v)
    }
    /// Performs store `code` of `value` at `addr + offset` on the memory at
    /// store index `index` under the conditions of `load_fast`, also
    /// requiring `value` to be of the stored type and the page allocated;
    /// false leaves memory unchanged.
    #[inline(always)]
    pub(super) fn store_fast(
        &mut self,
        index: usize,
        code: u16,
        addr: Value,
        value: Value,
        offset: u64,
    ) -> bool {
        let mem = &mut self.mems[index];
        if mem.shared.is_some() {
            return false;
        }
        let a = match addr {
            Value::I32(x) => x as u32 as u64,
            Value::I64(x) => x as u64,
            _ => return false,
        };
        let Some(at) = a.checked_add(offset).and_then(|a| usize::try_from(a).ok()) else {
            return false;
        };
        let bytes = &mut mem.bytes;
        match (code, value) {
            (0x36, Value::I32(x)) => bytes.write_array::<4>(at, x.to_le_bytes()),
            (0x37, Value::I64(x)) => bytes.write_array::<8>(at, x.to_le_bytes()),
            (0x38, Value::F32(x)) => bytes.write_array::<4>(at, x.to_le_bytes()),
            (0x39, Value::F64(x)) => bytes.write_array::<8>(at, x.to_le_bytes()),
            (0x3a, Value::I32(x)) => bytes.write_array::<1>(at, [x as u8]),
            (0x3b, Value::I32(x)) => bytes.write_array::<2>(at, (x as u16).to_le_bytes()),
            (0x3c, Value::I64(x)) => bytes.write_array::<1>(at, [x as u8]),
            (0x3d, Value::I64(x)) => bytes.write_array::<2>(at, (x as u16).to_le_bytes()),
            (0x3e, Value::I64(x)) => bytes.write_array::<4>(at, (x as u32).to_le_bytes()),
            _ => false,
        }
    }
    /// `memory_access_raw` for everything its fast paths leave, with every
    /// error.
    #[inline(never)]
    pub(super) fn memory_slow(
        &mut self,
        index: usize,
        code: u16,
        offset: u64,
        s: &mut Vec<Value>,
    ) -> Result<()> {
        let store = code >= 0x36;
        let value = if store { Some(pop(s)?) } else { None };
        let off = address(pop(s)?)?
            .checked_add(offset)
            .ok_or_else(|| trap("address overflow"))?;
        let n = match code {
            0x28 | 0x2a | 0x34 | 0x35 | 0x36 | 0x38 | 0x3e => 4,
            0x29 | 0x2b | 0x37 | 0x39 => 8,
            0x2c | 0x2d | 0x30 | 0x31 | 0x3a | 0x3c => 1,
            _ => 2,
        };
        let mem = &mut self.mems[index];
        let r = range(off, n, mem.len()?)?;
        if let Some(v) = value {
            let bits = match v {
                Value::I32(x) => x as u32 as u64,
                Value::I64(x) => x as u64,
                Value::F32(x) => x as u64,
                Value::F64(x) => x,
                _ => return Err(trap("invalid store value")),
            };
            let _ = r;
            mem.write(off, &bits.to_le_bytes()[..n])?;
        } else {
            let mut b = [0; 8];
            let _ = r;
            #[cfg(feature = "performance-counters")]
            self.perf.record(perf::Event::ReadVec, 1);
            mem.read_into(off, &mut b[..n])?;
            let x = u64::from_le_bytes(b);
            s.push(match code {
                0x28 => Value::I32(x as i32),
                0x29 => Value::I64(x as i64),
                0x2a => Value::F32(x as u32),
                0x2b => Value::F64(x),
                0x2c => Value::I32(x as i8 as i32),
                0x2d => Value::I32(x as i32),
                0x2e => Value::I32(x as i16 as i32),
                0x2f => Value::I32(x as i32),
                0x30 => Value::I64(x as i8 as i64),
                0x31 => Value::I64(x as i64),
                0x32 => Value::I64(x as i16 as i64),
                0x33 => Value::I64(x as i64),
                0x34 => Value::I64(x as i32 as i64),
                _ => Value::I64(x as i64),
            });
        }
        Ok(())
    }
    pub(super) fn bulk(&mut self, ii: usize, op: &Instruction, s: &mut Vec<Value>) -> Result<()> {
        let (a, b) = match op.immediate {
            Immediate::Pair(a, b) => (a as usize, b as usize),
            Immediate::Index(a) => (a as usize, 0),
            _ => (0, 0),
        };
        match op.opcode {
            0xfc09 => {
                self.instances[ii].data[a] = false;
            }
            0xfc0d => {
                self.instances[ii].elems[a] = None;
            }
            0xfc0f => {
                let remaining_table = self
                    .engine
                    .config
                    .limits
                    .max_store_table_elements
                    .saturating_sub(self.table_usage());
                let n = address(pop(s)?)?;
                let v = pop(s)?;
                let t = &mut self.tabs[self.instances[ii].tabs[a].0.index];
                let old = t.values.len() as u64;
                let mut result = u64::MAX;
                if let Some(new) = old.checked_add(n).filter(|n| {
                    *n <= t.ty.limits.max.unwrap_or(u64::MAX)
                        && *n <= self.engine.config.limits.max_table_elements
                        && n.saturating_sub(old) <= remaining_table
                }) && let Ok(new) = usize::try_from(new)
                    && t.values.try_reserve(new - t.values.len()).is_ok()
                {
                    t.values.resize(new, v);
                    result = old
                }
                s.push(if t.ty.index_type == IndexType::I64 {
                    Value::I64(result as i64)
                } else {
                    Value::I32(result as i32)
                });
            }
            0xfc10 => {
                let t = &self.tabs[self.instances[ii].tabs[a].0.index];
                s.push(if t.ty.index_type == IndexType::I64 {
                    Value::I64(t.values.len() as i64)
                } else {
                    Value::I32(t.values.len() as i32)
                })
            }
            0xfc08 | 0xfc0a | 0xfc0b => {
                let n = usize::try_from(address(pop(s)?)?).map_err(|_| trap("length overflow"))?;
                let src = address(pop(s)?)?;
                let dst = address(pop(s)?)?;
                let mi = if op.opcode == 0xfc08 { b } else { a };
                range(
                    dst,
                    n,
                    self.mems[self.instances[ii].mems[mi].0.index].len()?,
                )?;
                let dest = self.instances[ii].mems[mi].0.index;
                match op.opcode {
                    0xfc08 => {
                        let data = self.instances[ii].data_segment(a).unwrap_or(&[]);
                        let r = range(src, n, data.len())?;
                        self.mems[dest].write(dst, &data[r])?;
                    }
                    0xfc0a => {
                        let source = self.instances[ii].mems[b].0.index;
                        range(src, n, self.mems[source].len()?)?;
                        let same_unshared = source == dest && self.mems[dest].shared.is_none();
                        let mut scratch = Vec::new();
                        let mut done = 0;
                        while done < n {
                            if self.cancellation.is_cancelled() {
                                return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
                            }
                            let amount = (n - done).min(65536);
                            let chunk = if dst > src { n - done - amount } else { done };
                            let from = src as usize + chunk;
                            if same_unshared {
                                // Ranges were checked above; overlap is handled
                                // by the chunk order and copy_within.
                                #[cfg(feature = "performance-counters")]
                                self.perf.record(perf::Event::BulkSliceChunk, 1);
                                self.mems[dest]
                                    .bytes
                                    .copy_within(from, dst as usize + chunk, amount, &mut scratch)
                                    .map_err(allocation_failed)?;
                            } else if self.mems[source].shared.is_none()
                                && self.mems[dest].shared.is_none()
                            {
                                // Two unshared memories: one reused buffer.
                                #[cfg(feature = "performance-counters")]
                                self.perf.record(perf::Event::BulkSliceChunk, 1);
                                scratch
                                    .try_reserve(amount.saturating_sub(scratch.len()))
                                    .map_err(|_| {
                                        err(ErrorKind::ResourceLimit, "memory allocation failed")
                                    })?;
                                scratch.resize(amount, 0);
                                self.mems[source].read_into(from as u64, &mut scratch)?;
                                self.mems[dest].write(dst + chunk as u64, &scratch)?;
                            } else {
                                #[cfg(feature = "performance-counters")]
                                self.perf.record(perf::Event::ReadVec, 1);
                                let bytes = self.mems[source].read(from as u64, amount)?;
                                self.mems[dest].write(dst + chunk as u64, &bytes)?;
                            }
                            done += amount;
                        }
                    }
                    _ => {
                        const FILL_CHUNK: usize = 65536;
                        let unshared = self.mems[dest].shared.is_none();
                        // Shared memories take their writes from a buffer, sized
                        // to the fill rather than to a whole chunk.
                        let buffer = if unshared {
                            Vec::new()
                        } else {
                            vec![src as u8; n.min(FILL_CHUNK)]
                        };
                        let mut done = 0;
                        while done < n {
                            if self.cancellation.is_cancelled() {
                                return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
                            }
                            let amount = (n - done).min(FILL_CHUNK);
                            let at = dst as usize + done;
                            if unshared {
                                #[cfg(feature = "performance-counters")]
                                self.perf.record(perf::Event::BulkSliceChunk, 1);
                                self.mems[dest]
                                    .bytes
                                    .fill(at, amount, src as u8)
                                    .map_err(allocation_failed)?;
                            } else {
                                self.mems[dest].write(at as u64, &buffer[..amount])?;
                            }
                            done += amount;
                            #[cfg(test)]
                            if self
                                .bulk_cancel_after
                                .is_some_and(|threshold| done >= threshold)
                            {
                                self.bulk_cancel_after = None;
                                self.cancellation.cancel();
                            }
                        }
                    }
                }
            }
            0xfc0c | 0xfc0e | 0xfc11 => {
                let n = usize::try_from(address(pop(s)?)?).map_err(|_| trap("length overflow"))?;
                if op.opcode == 0xfc11 {
                    let v = pop(s)?;
                    let dst = address(pop(s)?)?;
                    let t = &mut self.tabs[self.instances[ii].tabs[a].0.index].values;
                    let r = table_range(dst, n, t.len())?;
                    t[r].fill(v)
                } else {
                    let src = address(pop(s)?)?;
                    let dst = address(pop(s)?)?;
                    let dest = if op.opcode == 0xfc0c { b } else { a };
                    table_range(
                        dst,
                        n,
                        self.tabs[self.instances[ii].tabs[dest].0.index]
                            .values
                            .len(),
                    )?;
                    let target = self.instances[ii].tabs[dest].0.index;
                    if op.opcode == 0xfc0c {
                        let values = self.instances[ii].elems[a].as_deref().unwrap_or(&[]);
                        let r = table_range(src, n, values.len())?;
                        let out = table_range(dst, n, self.tabs[target].values.len())?;
                        self.tabs[target].values[out].copy_from_slice(&values[r]);
                    } else {
                        let source = self.instances[ii].tabs[b].0.index;
                        table_range(src, n, self.tabs[source].values.len())?;
                        if target == source {
                            self.tabs[target]
                                .values
                                .copy_within(src as usize..src as usize + n, dst as usize)
                        } else {
                            let mut done = 0;
                            while done < n {
                                if self.cancellation.is_cancelled() {
                                    return Err(err(
                                        ErrorKind::ResourceLimit,
                                        "execution cancelled",
                                    ));
                                }
                                let count = (n - done).min(4096);
                                #[cfg(feature = "performance-counters")]
                                self.perf.record(perf::Event::TableCopySnapshotChunk, 1);
                                let vals = self.tabs[source].values
                                    [src as usize + done..src as usize + done + count]
                                    .to_vec();
                                self.tabs[target].values
                                    [dst as usize + done..dst as usize + done + count]
                                    .copy_from_slice(&vals);
                                done += count;
                            }
                        }
                    }
                }
            }
            _ => return Err(trap("unsupported bulk opcode")),
        }
        Ok(())
    }
}

impl Store {
    pub(super) fn simd_memory(
        &mut self,
        ii: usize,
        op: &Instruction,
        s: &mut Vec<Value>,
    ) -> Result<()> {
        let (align, offset, memory, lane) = match &op.immediate {
            Immediate::MemArg {
                align,
                offset,
                memory,
            } => (*align, *offset, *memory, 0),
            Immediate::MemLane {
                align,
                offset,
                memory,
                lane,
            } => (*align, *offset, *memory, *lane as usize),
            _ => return Err(trap("SIMD memory immediate")),
        };
        let _ = align;
        let sub = op.opcode & 255;
        let lane_op = (0x54..=0x5b).contains(&sub);
        let store = sub == 11 || (0x58..=0x5b).contains(&sub);
        let vector = if store || lane_op {
            let Value::V128(v) = pop(s)? else {
                return Err(trap("expected vector"));
            };
            v
        } else {
            V128::default()
        };
        let addr = address(pop(s)?)?
            .checked_add(offset)
            .ok_or_else(|| trap("address overflow"))?;
        let n = match sub {
            0 | 11 => 16,
            1..=6 => 8,
            7 => 1,
            8 => 2,
            9 | 0x5c => 4,
            10 | 0x5d => 8,
            0x54 | 0x58 => 1,
            0x55 | 0x59 => 2,
            0x56 | 0x5a => 4,
            0x57 | 0x5b => 8,
            _ => return Err(trap("SIMD memory opcode")),
        };
        let m = &mut self.mems[self.instances[ii].mems[memory as usize].0.index];
        range(addr, n, m.len()?)?;
        if store {
            if lane_op {
                let start = lane * n;
                if start + n > 16 {
                    return Err(trap("invalid lane"));
                }
                m.write(addr, &vector.0[start..start + n])?;
            } else {
                m.write(addr, &vector.0)?
            }
            return Ok(());
        }
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::ReadVec, 1);
        let data = m.read(addr, n)?;
        let bytes = &data[..];
        let mut out = vector.0;
        if lane_op {
            let start = lane * n;
            if start + n > 16 {
                return Err(trap("invalid lane"));
            }
            out[start..start + n].copy_from_slice(bytes)
        } else {
            match sub {
                0 => out.copy_from_slice(bytes),
                1..=6 => {
                    let width = 1usize << ((sub - 1) / 2);
                    let signed = sub % 2 == 1;
                    for (i, chunk) in bytes.chunks(width).enumerate() {
                        let mut b = [0u8; 8];
                        b[..width].copy_from_slice(chunk);
                        let mut x = u64::from_le_bytes(b);
                        if signed {
                            x = ((x << (64 - width * 8)) as i64 >> (64 - width * 8)) as u64
                        }
                        out[i * width * 2..(i + 1) * width * 2]
                            .copy_from_slice(&x.to_le_bytes()[..width * 2]);
                    }
                }
                7..=10 => {
                    for c in out.chunks_mut(n) {
                        c.copy_from_slice(bytes)
                    }
                }
                0x5c | 0x5d => out[..n].copy_from_slice(bytes),
                _ => return Err(trap("SIMD load opcode")),
            }
        }
        s.push(Value::V128(V128(out)));
        Ok(())
    }
}

impl Store {
    pub(super) fn atomic(&mut self, ii: usize, op: &Instruction, s: &mut Vec<Value>) -> Result<()> {
        let sub = op.opcode & 255;
        if sub == 3 {
            return shared_memory::SharedMemory::fence();
        }
        let Immediate::MemArg { offset, memory, .. } = op.immediate else {
            return Err(trap("atomic memory argument"));
        };
        let mem = &mut self.mems[self.instances[ii].mems[memory as usize].0.index];
        if (sub == 1 || sub == 2) && !mem.ty.shared {
            return Err(trap("atomic wait needs shared memory"));
        }
        if sub == 0 && !mem.ty.shared {
            let _count = pop(s)?;
            let addr = address(pop(s)?)?
                .checked_add(offset)
                .ok_or_else(|| trap("address overflow"))?;
            if addr % 4 != 0 {
                return Err(trap("unaligned atomic"));
            }
            range(addr, 4, mem.len()?)?;
            s.push(Value::I32(0));
            return Ok(());
        }
        if mem.shared.is_none() {
            let max = mem
                .ty
                .limits
                .max
                .unwrap_or(self.engine.config.limits.max_memory_pages)
                .checked_mul(1u64 << mem.ty.page_size_log2)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| err(ErrorKind::ResourceLimit, "memory maximum"))?;
            let max = max.min(
                usize::try_from(self.engine.config.limits.max_memory_bytes).unwrap_or(usize::MAX),
            );
            let backing = shared_memory::SharedMemory::new(mem.bytes.len(), max)?;
            // Only written pages hold anything; the rest of the new shared
            // backing is already zero.
            for (at, page) in mem.bytes.written_pages() {
                backing.write(at as u64, page)?;
            }
            mem.bytes.clear();
            mem.shared = Some(backing);
        }
        let m = mem.shared.clone().unwrap();
        if sub == 0 {
            let count = address(pop(s)?)? as u32;
            let a = address(pop(s)?)?
                .checked_add(offset)
                .ok_or_else(|| trap("address overflow"))?;
            s.push(Value::I32(m.notify(a, count)? as i32));
            return Ok(());
        }
        if sub == 1 || sub == 2 {
            let Value::I64(timeout) = pop(s)? else {
                return Err(trap("wait timeout"));
            };
            let expected = address(pop(s)?)?;
            let a = address(pop(s)?)?
                .checked_add(offset)
                .ok_or_else(|| trap("address overflow"))?;
            s.push(Value::I32(m.wait_with_cancel(
                a,
                if sub == 1 { 4 } else { 8 },
                expected,
                timeout,
                Some(&self.cancellation),
            )? as i32));
            return Ok(());
        }
        if sub < 0x10 {
            return Err(trap("unknown atomic instruction"));
        }
        let lane = (sub - 0x10) % 7;
        let (width, wide) = match lane {
            0 => (4, false),
            1 => (8, true),
            2 => (1, false),
            3 => (2, false),
            4 => (1, true),
            5 => (2, true),
            _ => (4, true),
        };
        let group = (sub - 0x10) / 7;
        let replacement = if group > 0 {
            Some(address(pop(s)?)?)
        } else {
            None
        };
        let expected = if group == 8 {
            Some(address(pop(s)?)?)
        } else {
            None
        };
        let a = address(pop(s)?)?
            .checked_add(offset)
            .ok_or_else(|| trap("address overflow"))?;
        let v = match group {
            0 => m.load(a, width)?,
            1 => {
                m.store(a, width, replacement.unwrap())?;
                return Ok(());
            }
            2..=7 => m.rmw(
                a,
                width,
                replacement.unwrap(),
                match group {
                    2 => shared_memory::Rmw::Add,
                    3 => shared_memory::Rmw::Sub,
                    4 => shared_memory::Rmw::And,
                    5 => shared_memory::Rmw::Or,
                    6 => shared_memory::Rmw::Xor,
                    _ => shared_memory::Rmw::Exchange,
                },
            )?,
            8 => m.compare_exchange(a, width, expected.unwrap(), replacement.unwrap())?,
            _ => return Err(trap("atomic opcode")),
        };
        s.push(if wide {
            Value::I64(v as i64)
        } else {
            Value::I32(v as i32)
        });
        Ok(())
    }
}

impl Store {
    pub(super) fn unwind_exception(
        &mut self,
        ex: ExceptionRef,
        frames: &mut Vec<Frame>,
        stack: &mut Vec<Value>,
    ) -> Result<bool> {
        let ex_idx = self.check(ex.0, &self.exceptions)?;
        let ex_tag = self.exceptions[ex_idx].tag;
        while let Some(frame) = frames.last_mut() {
            for pos in (0..frame.labels.len()).rev() {
                let Some(try_pc) = frame.labels[pos].try_table_pc else {
                    continue;
                };
                let ii = frame.function.instance;
                let matched = {
                    let Immediate::TryTable(table) =
                        &self.instances[ii].module.data.inner.decoded().code[frame.function.body]
                            .instructions[try_pc]
                            .immediate
                    else {
                        return Err(trap("try_table immediate"));
                    };
                    table
                        .catches
                        .iter()
                        .find(|catch| {
                            catch
                                .tag
                                .is_none_or(|n| self.instances[ii].tags[n as usize] == ex_tag)
                        })
                        .cloned()
                };
                if let Some(catch) = matched {
                    let mut values = if catch.tag.is_some() {
                        self.exceptions[ex_idx].payload.clone()
                    } else {
                        vec![]
                    };
                    if catch.kind == 1 || catch.kind == 3 {
                        values.push(Value::Ref(RefValue::Exn(ex)))
                    }
                    let depth = catch.label as usize;
                    if depth == pos {
                        stack.truncate(frame.base);
                        stack.extend(values);
                        if let Some(popped) = frames.pop() {
                            self.release_local_slots(popped.locals.len());
                            self.release_labels(popped.labels.len());
                        }
                        return Ok(true);
                    }
                    let target = pos
                        .checked_sub(depth + 1)
                        .ok_or_else(|| trap("exception branch depth"))?;
                    let label = &frame.labels[target];
                    stack.truncate(label.height);
                    stack.extend(values);
                    frame.pc = if label.loop_ {
                        label.start
                    } else {
                        label.end + 1
                    };
                    let retain = target + usize::from(label.loop_);
                    let removed = frame.labels.len() - retain;
                    frame.labels.truncate(retain);
                    self.release_labels(removed);
                    return Ok(true);
                }
            }
            let base = frame.base;
            if let Some(popped) = frames.pop() {
                self.release_local_slots(popped.locals.len());
                self.release_labels(popped.labels.len());
            }
            stack.truncate(base);
        }
        Ok(false)
    }
}
