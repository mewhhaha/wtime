//! Reference dispatcher seeded from the immutable P001 source archive.
//! Shares admission, frames, memory/numeric/GC/continuation helpers deliberately;
//! it is not an independent oracle for bugs in those helpers. External/frozen
//! binaries remain necessary for differential validation of shared semantics.
use super::*;
use crate::execution::Label;
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
pub(super) fn keep(s: &mut Vec<Value>, height: usize, n: usize) -> Result<()> {
    if s.len() < n || height > s.len() - n {
        return Err(trap("stack underflow"));
    }
    let tail = s.split_off(s.len() - n);
    s.truncate(height);
    s.extend(tail);
    Ok(())
}
impl Store {
    pub(super) fn invoke_reference(&mut self, f: Func, args: &[Value]) -> Result<RunOutcome> {
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
        let mut frames = vec![self.frame(f, args.to_vec(), 0)?];
        let mut stack = vec![];
        let mut fuel = self.engine.config.limits.fuel;
        let mut prompts = Vec::new();
        loop {
            if let Some(saved) = self.resume_fuel.take() {
                fuel = match (fuel, saved) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, None) => a,
                    (None, b) => b,
                };
            }
            if self.cancellation.is_cancelled() {
                return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
            }
            crate::continuation::retire_prompts(frames.len(), &mut prompts);
            if let Some(n) = &mut fuel {
                if *n == 0 {
                    return Err(err(ErrorKind::ResourceLimit, "fuel exhausted"));
                }
                *n -= 1
            }
            self.current_fuel = fuel;
            if stack.len() > self.engine.config.limits.max_stack_height {
                return Err(err(ErrorKind::ResourceLimit, "operand stack limit"));
            }
            let frame = frames.last_mut().unwrap();
            let inst_id = frame.function.instance;
            let module = self.instances[inst_id].module.clone();
            let ops = &module.data.inner.decoded().code[frame.function.body].instructions;
            self.reference_dispatches = self.reference_dispatches.saturating_add(1);
            let op = ops
                .get(frame.pc)
                .ok_or_else(|| trap("instruction pointer out of bounds"))?
                .clone();
            #[cfg(feature = "opcode-coverage")]
            crate::coverage::record(op.opcode);
            let pc = frame.pc;
            frame.pc += 1;
            match op.opcode {
                0x1f => {
                    let Immediate::TryTable(table) = &op.immediate else {
                        return Err(trap("try_table immediate"));
                    };
                    let (block, catches) = (&table.block, &table.catches);
                    let (a, r) = arity(block, &module);
                    let (end, _) = module.data.control.get(frame.function.body, pc)
                        .ok_or_else(|| trap("missing control boundary"))?;
                    let height = stack
                        .len()
                        .checked_sub(a)
                        .ok_or_else(|| trap("try_table stack underflow"))?;
                    frame.labels.push(Label {
                        catches: catches.clone(),
                        start: pc + 1,
                        end,
                        height,
                        params: a,
                        results: r,
                        loop_: false,
                    });
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
                    let (a, r) = arity(&op.immediate, &module);
                    let cond = if op.opcode == 4 {
                        address(pop(&mut stack)?)? != 0
                    } else {
                        true
                    };
                    let (end, other) = module.data.control.get(frame.function.body, pc)
                        .ok_or_else(|| trap("missing control boundary"))?;
                    let height = stack
                        .len()
                        .checked_sub(a)
                        .ok_or_else(|| trap("block stack underflow"))?;
                    frame.labels.push(Label {
                        catches: vec![],
                        start: pc + 1,
                        end,
                        height,
                        params: a,
                        results: r,
                        loop_: op.opcode == 3,
                    });
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
                        keep(&mut stack, l.height, l.results)?;
                    } else {
                        let n = frame.function.ty.results.len();
                        let base = frame.base;
                        keep(&mut stack, base, n)?;
                        frames.pop();
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
                        keep(&mut stack, frame.base, n)?;
                        frames.pop();
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
                        frame.labels.truncate(retain)
                    }
                }
                0xf => {
                    keep(&mut stack, frame.base, frame.function.ty.results.len())?;
                    frames.pop();
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
                            self.instances[inst_id].funcs[x as usize]
                        }
                        Immediate::Pair(t, table) => {
                            let ix = address(pop(&mut stack)?)?;
                            let tab = self.instances[inst_id].tabs[table as usize];
                            let val = *self.tabs[tab.0.index]
                                .values
                                .get(ix as usize)
                                .ok_or_else(|| trap("table out of bounds"))?;
                            let Value::Ref(RefValue::Func(f)) = val else {
                                return Err(trap("null indirect call"));
                            };
                            if !self
                                .engine
                                .canonical
                                .lock()
                                .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                                .is_subtype(
                                    self.funcs[self.check(f.0, &self.funcs)?].canonical,
                                    module.data.canonical[t as usize],
                                    false,
                                    self.engine.config.limits.max_type_work,
                                )?
                            {
                                return Err(trap("indirect call type mismatch"));
                            }
                            f
                        }
                        Immediate::Index(t) => {
                            let Value::Ref(RefValue::Func(f)) = pop(&mut stack)? else {
                                return Err(trap("null call_ref"));
                            };
                            if !self
                                .engine
                                .canonical
                                .lock()
                                .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                                .is_subtype(
                                    self.funcs[self.check(f.0, &self.funcs)?].canonical,
                                    module.data.canonical[t as usize],
                                    false,
                                    self.engine.config.limits.max_type_work,
                                )?
                            {
                                return Err(trap("call_ref type mismatch"));
                            }
                            f
                        }
                        _ => return Err(trap("call immediate")),
                    };
                    let n = self.funcs[self.check(fun.0, &self.funcs)?].ty.params.len();
                    if n > stack.len() {
                        return Err(trap("call stack underflow"));
                    }
                    let args = stack.split_off(stack.len() - n);
                    if let Some(host) = self.funcs[fun.0.index].host.clone() {
                        let ty = self.funcs[fun.0.index].ty.clone();
                        for value in &args {
                            self.validate_value(*value)?;
                        }
                        self.check_function_values(&self.funcs[fun.0.index], &args, &ty.params)?;
                        let caller_base = frame.base;
                        let caller_results = frame.function.ty.results.len();
                        let mut roots = stack.clone();
                        roots.extend_from_slice(&args);
                        roots.extend(frames.iter().flat_map(|f| f.locals.iter().copied()));
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
                            keep(&mut stack, caller_base, caller_results)?;
                            frames.pop();
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
                    let tail = matches!(op.opcode, 0x12 | 0x13 | 0x15);
                    let base = if tail { frame.base } else { stack.len() };
                    let next = self.frame(fun, args, base)?;
                    if tail {
                        stack.truncate(base);
                        frames.pop();
                    } else if frames.len() >= self.engine.config.limits.max_call_depth {
                        return Err(err(ErrorKind::ResourceLimit, "call depth exhausted"));
                    }
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
                0x20..=0x26 => {
                    let Immediate::Index(x) = op.immediate else {
                        return Err(trap("index immediate"));
                    };
                    let x = x as usize;
                    match op.opcode {
                        0x20 => stack.push(frame.locals[x]),
                        0x21 => frame.locals[x] = pop(&mut stack)?,
                        0x22 => {
                            frame.locals[x] = *stack.last().ok_or_else(|| trap("empty stack"))?
                        }
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
                0x28..=0x3e => self.load_store(inst_id, &op, &mut stack)?,
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
                        Self::branch_handler(&mut frames, &mut stack, label as usize)?;
                    }
                    if frames.is_empty() {
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0xe0..=0xe6 => {
                    if let Some(ex) =
                        self.cont_step(inst_id, &op, &mut frames, &mut stack, &mut prompts)?
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
                        Self::branch_handler(&mut frames, &mut stack, label as usize)?;
                    }
                    if frames.is_empty() {
                        for value in &stack {
                            self.validate_value(*value)?;
                        }
                        self.publish_values(&stack);
                        return Ok(RunOutcome::Returned(stack));
                    }
                }
                0xfb00..=0xfb26 => self.gc_exec(inst_id, &op, &mut stack)?,
                0xfe00..=0xfe4e => self.atomic(inst_id, &op, &mut stack)?,
                0xfc13..=0xfc16 => crate::wide::execute(op.opcode, &mut stack)?,
                0xfc08..=0xfc11 => self.bulk(inst_id, &op, &mut stack)?,
                0xfd00..=0xfd0b | 0xfd54..=0xfd5d => self.simd_memory(inst_id, &op, &mut stack)?,
                n if n & 0xff00 == 0xfd00 || n & 0xff00 == 0xfa00 => {
                    crate::simd::execute(n, &op.immediate, &mut stack)?
                }
                n => numeric::exec(n, &mut stack)?,
            }
        }
    }
}
