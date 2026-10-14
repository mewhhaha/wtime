//! One-shot continuations own detached explicit frames. No native frame is captured.
use super::*;
use crate::execution::Frame;
use crate::gc::{Object, ObjectKind};
pub(super) struct Prompt {
    pub frame_depth: usize,
    pub operand_depth: usize,
    handlers: Vec<(Tag, Option<u32>)>,
    instance: usize,
}
struct Segment {
    fuel: Option<u64>,
    frames: Vec<Frame>,
    operands: Vec<Value>,
    prompts: Vec<Prompt>,
}
enum Body {
    Start(Func),
    Segment(Segment),
}
pub(super) struct ContinuationState {
    body: Option<Body>,
    signature: FuncType,
    type_ids: (Vec<Option<CanonicalTypeId>>, Vec<Option<CanonicalTypeId>>),
    bound: Vec<Value>,
}
pub(super) fn retire_prompts(depth: usize, prompts: &mut Vec<Prompt>) {
    while prompts.last().is_some_and(|p| p.frame_depth >= depth) {
        prompts.pop();
    }
}
fn handle(s: &mut Vec<Value>) -> Result<GcRef> {
    match pop(s)? {
        Value::Ref(RefValue::Gc(r)) => Ok(r),
        _ => Err(trap("null or invalid continuation")),
    }
}
fn args(s: &mut Vec<Value>, count: usize) -> Result<Vec<Value>> {
    let base = s
        .len()
        .checked_sub(count)
        .ok_or_else(|| trap("continuation argument underflow"))?;
    Ok(s.split_off(base))
}
fn type_ids(types: &[ValType], module: Option<&Module>) -> Result<Vec<Option<CanonicalTypeId>>> {
    let mut ids = Vec::new();
    ids.try_reserve_exact(types.len())
        .map_err(|_| err(ErrorKind::ResourceLimit, "continuation type allocation"))?;
    for ty in types {
        ids.push(match ty {
            ValType::Ref(RefType {
                heap: HeapType::Defined(i) | HeapType::ExactDefined(i),
                ..
            }) => module
                .and_then(|m| m.data.canonical.get(i.as_usize()))
                .copied()
                .map(Some)
                .ok_or_else(|| err(ErrorKind::Host, "missing continuation type context")),
            _ => Ok(None),
        }?);
    }
    // Header admission charges one context entry per declared value. Retain
    // exactly that capacity even if the allocator over-reserved the draft.
    Ok(ids.into_boxed_slice().into_vec())
}
fn signature_bytes(signature: &FuncType) -> usize {
    signature
        .params
        .capacity()
        .saturating_add(signature.results.capacity())
        .saturating_mul(std::mem::size_of::<ValType>())
}
fn capture_bytes(frames: &[Frame], operands: usize, prompts: &[Prompt]) -> usize {
    let mut bytes = frames
        .len()
        .saturating_mul(std::mem::size_of::<Frame>())
        .saturating_add(operands.saturating_mul(std::mem::size_of::<Value>()))
        .saturating_add(prompts.len().saturating_mul(std::mem::size_of::<Prompt>()));
    for frame in frames {
        bytes = bytes
            .saturating_add(
                frame
                    .locals
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Value>()),
            )
            .saturating_add(
                frame
                    .labels
                    .capacity()
                    .saturating_mul(std::mem::size_of::<crate::execution::Label>()),
            )
            .saturating_add(signature_bytes(&frame.function.ty));
        for label in &frame.labels {
            bytes = bytes.saturating_add(
                label
                    .catches
                    .capacity()
                    .saturating_mul(std::mem::size_of::<wtime_validate::Catch>()),
            );
        }
    }
    for prompt in prompts {
        bytes = bytes.saturating_add(
            prompt
                .handlers
                .capacity()
                .saturating_mul(std::mem::size_of::<(Tag, Option<u32>)>()),
        );
    }
    bytes
}
impl Store {
    pub fn continuation_bytes(&self) -> usize {
        #[cfg(feature = "performance-counters")]
        self.perf.record(
            perf::Event::ContinuationScan,
            u64::try_from(self.continuations.len()).unwrap_or(u64::MAX),
        );
        self.continuations.values().fold(0usize, |total, state| {
            let body = match state.body.as_ref() {
                Some(Body::Segment(s)) => {
                    capture_bytes(&s.frames, s.operands.capacity(), &s.prompts)
                        .saturating_add(
                            s.frames
                                .capacity()
                                .saturating_sub(s.frames.len())
                                .saturating_mul(std::mem::size_of::<Frame>()),
                        )
                        .saturating_add(
                            s.prompts
                                .capacity()
                                .saturating_sub(s.prompts.len())
                                .saturating_mul(std::mem::size_of::<Prompt>()),
                        )
                }
                _ => 0,
            };
            total
                .saturating_add(std::mem::size_of::<ContinuationState>())
                .saturating_add(body)
                .saturating_add(signature_bytes(&state.signature).saturating_add(
                    (state.type_ids.0.capacity() + state.type_ids.1.capacity())
                        * std::mem::size_of::<Option<CanonicalTypeId>>(),
                ))
                .saturating_add(
                    state
                        .bound
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Value>()),
                )
        })
    }
    fn check_continuation_bytes(&self, additional: usize) -> Result<()> {
        self.continuation_bytes()
            .checked_add(additional)
            .filter(|n| *n <= self.engine.config.limits.max_continuation_bytes)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "continuation byte budget"))?;
        Ok(())
    }
    fn continuation_header_bytes(signature: &FuncType) -> usize {
        std::mem::size_of::<ContinuationState>()
            .saturating_add(signature_bytes(signature))
            .saturating_add(
                (signature.params.len() + signature.results.len())
                    .saturating_mul(std::mem::size_of::<Option<CanonicalTypeId>>()),
            )
    }

    fn continuation_type(&self, ii: usize, n: usize) -> Result<(CanonicalTypeId, FuncType)> {
        let module = &self.instances[ii].module;
        let ty = module
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(n)
            .ok_or_else(|| trap("continuation type index"))?;
        let CompositeType::Cont(t) = ty.composite else {
            return Err(trap("expected continuation type"));
        };
        Ok((
            module.data.canonical[n],
            module.data.inner.decoded().types[t.function.as_usize()].clone(),
        ))
    }
    pub(super) fn validate_host_suspension(
        &self,
        function: &Function,
        tag: Tag,
        payload: &[Value],
        r: GcRef,
    ) -> Result<()> {
        let tag_index = self.check(tag.0, &self.tags)?;
        let state = self.continuation_ready(r).map_err(|_| {
            err(
                ErrorKind::Host,
                "host suspension requires a live ready continuation",
            )
        })?;
        let signature = &self.tags[tag_index];
        if payload.len() != signature.params.len() {
            return Err(err(ErrorKind::Host, "suspension payload count"));
        }
        for (value, ty) in payload.iter().zip(&signature.params) {
            self.validate_value(*value)?;
            let valid = match ty {
                ValType::Ref(reference) => {
                    if let Some(module) = self.tag_modules.get(&tag_index) {
                        self.gc_matches_module(module, *value, *reference)?
                    } else {
                        self.gc_matches(
                            self.tag_instances
                                .get(&tag_index)
                                .copied()
                                .unwrap_or(usize::MAX),
                            *value,
                            *reference,
                        )?
                    }
                }
                _ => matches_type(*value, *ty),
            };
            if !valid {
                return Err(err(ErrorKind::Host, "suspension payload type"));
            }
        }
        let tag_owner = self.tag_modules.get(&tag_index).or_else(|| {
            self.tag_instances
                .get(&tag_index)
                .and_then(|i| self.instances.get(*i).map(|i| &i.module))
        });
        let host_owner = function.host_type_module.as_ref().or_else(|| {
            function
                .host_type_instance
                .and_then(|i| self.instances.get(i).map(|i| &i.module))
        });
        for (actual, ids, expected, expected_owner) in [
            (
                &state.signature.params,
                &state.type_ids.0,
                &signature.results,
                tag_owner,
            ),
            (
                &state.signature.results,
                &state.type_ids.1,
                &function.ty.results,
                host_owner,
            ),
        ] {
            if actual.len() != expected.len() {
                return Err(err(ErrorKind::Host, "suspension continuation signature"));
            }
            for (position, (a, b)) in actual.iter().zip(expected).enumerate() {
                let equivalent = match (a, b) {
                    (ValType::Ref(a), ValType::Ref(b)) if a.nullable == b.nullable => {
                        match (a.heap, b.heap) {
                            (HeapType::Defined(_), HeapType::Defined(b))
                            | (HeapType::ExactDefined(_), HeapType::ExactDefined(b)) => ids
                                [position]
                                .zip(
                                    expected_owner
                                        .and_then(|m| m.data.canonical.get(b.as_usize()))
                                        .copied(),
                                )
                                .is_some_and(|(a, b)| a == b),
                            (HeapType::Defined(_) | HeapType::ExactDefined(_), _)
                            | (_, HeapType::Defined(_) | HeapType::ExactDefined(_)) => false,
                            _ => a == b,
                        }
                    }
                    _ => a == b,
                };
                if !equivalent {
                    return Err(err(ErrorKind::Host, "suspension continuation signature"));
                }
            }
        }
        Ok(())
    }
    fn continuation_ready(&self, r: GcRef) -> Result<&ContinuationState> {
        self.gc.get(r)?;
        let state = self
            .continuations
            .get(&r)
            .ok_or_else(|| trap("expected continuation"))?;
        if state.body.is_none() {
            return Err(trap("continuation already consumed"));
        }
        Ok(state)
    }
    fn reserve_continuation(&mut self, ty: CanonicalTypeId, roots: Vec<Value>) -> Result<GcRef> {
        self.continuations
            .try_reserve(1)
            .map_err(|_| err(ErrorKind::ResourceLimit, "continuation registry allocation"))?;
        self.reserve_objects(1)?;
        self.gc.allocate(
            Object {
                ty,
                kind: ObjectKind::Continuation,
                fields: roots,
                descriptor: None,
            },
            self.engine.config.limits.max_table_elements as usize,
        )
    }
    fn segment_roots(frames: &[Frame], operands: &[Value]) -> Vec<Value> {
        let mut roots = operands.to_vec();
        for f in frames {
            roots.extend(&f.locals)
        }
        roots
    }
    fn capture(
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
        p: usize,
        fuel: Option<u64>,
    ) -> Segment {
        let fd = prompts[p].frame_depth;
        let od = prompts[p].operand_depth;
        let mut nested = prompts.split_off(p + 1);
        let mut detached = frames.split_off(fd);
        let stack = operands.split_off(od);
        for f in &mut detached {
            f.base -= od;
            for l in &mut f.labels {
                l.height -= od;
            }
        }
        for prompt in &mut nested {
            prompt.frame_depth -= fd;
            prompt.operand_depth -= od;
        }
        Segment {
            fuel,
            frames: detached,
            operands: stack,
            prompts: nested,
        }
    }
    fn install(
        &mut self,
        r: GcRef,
        arguments: Vec<Value>,
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
    ) -> Result<Option<ExceptionRef>> {
        let state = self.continuation_ready(r)?;
        let count = match state.body.as_ref().unwrap() {
            Body::Start(_) => 1,
            Body::Segment(s) => s.frames.len(),
        };
        if frames
            .len()
            .checked_add(count)
            .is_none_or(|n| n > self.engine.config.limits.max_call_depth)
        {
            return Err(err(ErrorKind::ResourceLimit, "continuation frame budget"));
        }
        frames
            .try_reserve(count)
            .map_err(|_| err(ErrorKind::ResourceLimit, "continuation frame allocation"))?;
        let state = self.continuations.get_mut(&r).unwrap();
        let body = state.body.take().unwrap();
        let mut bound = std::mem::take(&mut state.bound);
        bound.extend(arguments);
        self.gc.get_mut(r)?.fields.clear();
        match body {
            Body::Start(f) => {
                let fi = self.check(f.0, &self.funcs)?;
                if self.funcs[fi].host.is_some() {
                    self.active_roots
                        .push(Self::segment_roots(frames, operands));
                    let result = self.invoke(f, &bound);
                    self.active_roots.pop();
                    match result? {
                        RunOutcome::Returned(values) => operands.extend(values),
                        RunOutcome::Exception(ex) => return Ok(Some(ex)),
                        RunOutcome::Suspended {
                            tag,
                            payload,
                            continuation,
                        } => {
                            self.pending_suspension = Some((tag, payload, continuation));
                            frames.clear();
                            operands.clear();
                            return Ok(None);
                        }
                    }
                } else {
                    frames.push(self.frame(f, bound, operands.len())?)
                }
            }
            Body::Segment(mut segment) => {
                self.resume_fuel = Some(segment.fuel);
                let fd = frames.len();
                let od = operands.len();
                for f in &mut segment.frames {
                    f.base += od;
                    for l in &mut f.labels {
                        l.height += od
                    }
                }
                for p in &mut segment.prompts {
                    p.frame_depth += fd;
                    p.operand_depth += od
                }
                operands.extend(segment.operands);
                operands.extend(bound);
                frames.extend(segment.frames);
                prompts.extend(segment.prompts)
            }
        }
        Ok(None)
    }
    fn handler_type(
        &self,
        prompt: &Prompt,
        frames: &[Frame],
        label: u32,
    ) -> Result<(CanonicalTypeId, FuncType)> {
        let frame = frames
            .get(prompt.frame_depth - 1)
            .ok_or_else(|| trap("missing prompt frame"))?;
        let module = &self.instances[prompt.instance].module;
        let results = if label as usize == frame.labels.len() {
            frame.function.ty.results.clone()
        } else {
            let block = frame
                .labels
                .get(
                    frame
                        .labels
                        .len()
                        .checked_sub(label as usize + 1)
                        .ok_or_else(|| trap("handler label depth"))?,
                )
                .ok_or_else(|| trap("handler label"))?;
            let instruction = &module.data.inner.decoded().code[frame.function.body].instructions
                [block.start - 1];
            let immediate = match &instruction.immediate {
                Immediate::TryTable { block, .. } => block.as_ref(),
                x => x,
            };
            match immediate {
                Immediate::Block(n) if *n >= 0 => {
                    let t = &module.data.inner.decoded().types[*n as usize];
                    if block.loop_ {
                        t.params.clone()
                    } else {
                        t.results.clone()
                    }
                }
                Immediate::BlockType(t) => vec![*t],
                _ => vec![],
            }
        };
        let Some(ValType::Ref(RefType {
            heap: HeapType::Defined(n) | HeapType::ExactDefined(n),
            ..
        })) = results.last()
        else {
            return Err(trap("handler requires continuation result"));
        };
        self.continuation_type(prompt.instance, n.as_usize())
    }
    pub(super) fn branch_handler(
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        depth: usize,
    ) -> Result<()> {
        let frame = frames
            .last_mut()
            .ok_or_else(|| trap("handler frame missing"))?;
        if depth == frame.labels.len() {
            let n = frame.function.ty.results.len();
            crate::execution::keep(operands, frame.base, n)?;
            frames.pop();
            return Ok(());
        }
        let pos = frame
            .labels
            .len()
            .checked_sub(depth + 1)
            .ok_or_else(|| trap("handler depth"))?;
        let label = &frame.labels[pos];
        crate::execution::keep(
            operands,
            label.height,
            if label.loop_ {
                label.params
            } else {
                label.results
            },
        )?;
        frame.pc = if label.loop_ {
            label.start
        } else {
            label.end + 1
        };
        frame.labels.truncate(pos + usize::from(label.loop_));
        Ok(())
    }
    fn unhandled_continuation(
        &mut self,
        _ii: usize,
        event: (Tag, Vec<ValType>, Vec<Value>),
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
    ) -> Result<()> {
        let (tag, parameters, payload) = event;
        let result_types = frames
            .first()
            .map(|f| f.function.ty.results.clone())
            .unwrap_or_default();
        let resume_signature = FuncType::new(parameters, result_types);
        let param_owner = self.tag_modules.get(&tag.0.index).or_else(|| {
            self.tag_instances
                .get(&tag.0.index)
                .and_then(|n| self.instances.get(*n).map(|i| &i.module))
        });
        let result_owner = frames
            .first()
            .and_then(|f| self.instances.get(f.function.instance).map(|i| &i.module));
        let ids = (
            type_ids(&resume_signature.params, param_owner)?,
            type_ids(&resume_signature.results, result_owner)?,
        );
        let mut lowered = resume_signature.clone();
        let mut preceding = Vec::new();
        for (types, contexts) in [
            (&mut lowered.params, &ids.0),
            (&mut lowered.results, &ids.1),
        ] {
            for (ty, id) in types.iter_mut().zip(contexts) {
                if let Some(id) = id {
                    let index = TypeIdx(u32::try_from(preceding.len()).map_err(|_| {
                        err(ErrorKind::ResourceLimit, "continuation type relocation")
                    })?);
                    preceding.push(*id);
                    if let ValType::Ref(reference) = ty {
                        reference.heap = if matches!(reference.heap, HeapType::ExactDefined(_)) {
                            HeapType::ExactDefined(index)
                        } else {
                            HeapType::Defined(index)
                        };
                    }
                }
            }
        }
        let mut registry = self
            .engine
            .canonical
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?;
        let function = registry.intern_group(
            &TypeGroup {
                explicit: false,
                types: vec![SubType::final_type(CompositeType::Func(lowered))],
            },
            &preceding,
            self.engine.config.limits.max_type_work,
        )?[0];
        let function_index = preceding.len();
        preceding.push(function);
        let ty = registry.intern_group(
            &TypeGroup {
                explicit: false,
                types: vec![SubType::final_type(CompositeType::Cont(ContType {
                    function: TypeIdx(function_index as u32),
                }))],
            },
            &preceding,
            self.engine.config.limits.max_type_work,
        )?[0];
        drop(registry);
        self.check_continuation_bytes(
            Self::continuation_header_bytes(&resume_signature)
                .saturating_add(capture_bytes(frames, operands.capacity(), prompts))
                .saturating_add(
                    frames
                        .capacity()
                        .saturating_sub(frames.len())
                        .saturating_mul(std::mem::size_of::<Frame>()),
                )
                .saturating_add(
                    prompts
                        .capacity()
                        .saturating_sub(prompts.len())
                        .saturating_mul(std::mem::size_of::<Prompt>()),
                ),
        )?;
        let roots = Self::segment_roots(frames, operands);
        let r = self.reserve_continuation(ty, roots)?;
        let segment = Segment {
            fuel: self.current_fuel,
            frames: std::mem::take(frames),
            operands: std::mem::take(operands),
            prompts: std::mem::take(prompts),
        };
        self.continuations.insert(
            r,
            ContinuationState {
                body: Some(Body::Segment(segment)),
                signature: resume_signature,
                type_ids: ids,
                bound: vec![],
            },
        );
        self.pending_suspension = Some((tag, payload, r));
        Ok(())
    }
    pub(super) fn cont_step(
        &mut self,
        ii: usize,
        i: &Instruction,
        frames: &mut Vec<Frame>,
        operands: &mut Vec<Value>,
        prompts: &mut Vec<Prompt>,
    ) -> Result<Option<ExceptionRef>> {
        match i.opcode {
            0xe0 => {
                let Immediate::Index(n) = i.immediate else {
                    return Err(trap("cont.new immediate"));
                };
                let (ty, signature) = self.continuation_type(ii, n as usize)?;
                let Value::Ref(RefValue::Func(f)) = pop(operands)? else {
                    return Err(trap("null continuation function"));
                };
                self.check(f.0, &self.funcs)?;
                self.check_continuation_bytes(Self::continuation_header_bytes(&signature))?;
                let r = self.reserve_continuation(ty, vec![])?;
                self.continuations.insert(
                    r,
                    ContinuationState {
                        body: Some(Body::Start(f)),
                        type_ids: (
                            type_ids(&signature.params, Some(&self.instances[ii].module))?,
                            type_ids(&signature.results, Some(&self.instances[ii].module))?,
                        ),
                        signature,
                        bound: vec![],
                    },
                );
                operands.push(Value::Ref(RefValue::Gc(r)))
            }
            0xe1 => {
                let Immediate::Pair(_, target) = i.immediate else {
                    return Err(trap("cont.bind immediate"));
                };
                let (ty, signature) = self.continuation_type(ii, target as usize)?;
                let r = handle(operands)?;
                let state = self.continuation_ready(r)?;
                let n = state
                    .signature
                    .params
                    .len()
                    .checked_sub(signature.params.len())
                    .ok_or_else(|| trap("invalid bind signature"))?;
                self.check_continuation_bytes(
                    Self::continuation_header_bytes(&signature)
                        .saturating_add(n.saturating_mul(std::mem::size_of::<Value>()))
                        // Vec::extend may double the transferred argument buffer.
                        .saturating_add(
                            state
                                .bound
                                .capacity()
                                .saturating_add(n)
                                .saturating_mul(std::mem::size_of::<Value>()),
                        ),
                )?;
                let arguments = args(operands, n)?;
                let mut roots = self.gc.get(r)?.fields.clone();
                roots.extend(&arguments);
                let new = self.reserve_continuation(ty, roots)?;
                let old = self.continuations.get_mut(&r).unwrap();
                let body = old.body.take();
                let mut bound = std::mem::take(&mut old.bound);
                bound.extend(arguments);
                self.gc.get_mut(r)?.fields.clear();
                self.continuations.insert(
                    new,
                    ContinuationState {
                        body,
                        type_ids: (
                            type_ids(&signature.params, Some(&self.instances[ii].module))?,
                            type_ids(&signature.results, Some(&self.instances[ii].module))?,
                        ),
                        signature,
                        bound,
                    },
                );
                operands.push(Value::Ref(RefValue::Gc(new)))
            }
            0xe3..=0xe5 => {
                let Immediate::Resume { ty, tag, handlers } = &i.immediate else {
                    return Err(trap("resume immediate"));
                };
                let (_, signature) = self.continuation_type(ii, *ty as usize)?;
                let r = handle(operands)?;
                self.continuation_ready(r)?;
                let mut exception = None;
                let arguments = match i.opcode {
                    0xe3 => args(operands, signature.params.len())?,
                    0xe4 => {
                        let tag = self.instances[ii].tags
                            [tag.ok_or_else(|| trap("resume throw tag"))? as usize];
                        let payload = args(operands, self.tags[tag.0.index].params.len())?;
                        exception = Some(self.allocate_exception(tag, &payload)?);
                        vec![]
                    }
                    _ => {
                        let Value::Ref(RefValue::Exn(ex)) = pop(operands)? else {
                            return Err(trap("null exception reference"));
                        };
                        self.exception_tag(ex)?;
                        exception = Some(ex);
                        vec![]
                    }
                };
                let table = handlers
                    .iter()
                    .map(|(tag, label)| {
                        Ok((
                            *self.instances[ii]
                                .tags
                                .get(*tag as usize)
                                .ok_or_else(|| trap("resume handler tag"))?,
                            *label,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                prompts
                    .try_reserve(1)
                    .map_err(|_| err(ErrorKind::ResourceLimit, "prompt allocation"))?;
                let prompt = Prompt {
                    frame_depth: frames.len(),
                    operand_depth: operands.len(),
                    handlers: table,
                    instance: ii,
                };
                prompts.push(prompt);
                let initial_throw = exception.is_some()
                    && matches!(self.continuation_ready(r)?.body, Some(Body::Start(_)));
                if initial_throw {
                    self.continuations.get_mut(&r).unwrap().body.take();
                    self.gc.get_mut(r)?.fields.clear();
                    prompts.pop();
                } else {
                    if let Some(ex) = self.install(r, arguments, frames, operands, prompts)? {
                        exception = Some(ex);
                    }
                }
                if let Some(ex) = exception
                    && !self.unwind_exception(ex, frames, operands)?
                {
                    // Installation arms the saved segment fuel for its next
                    // source boundary. An unhandled initial throw ends this
                    // invocation before that boundary, so it must not debit a
                    // later independent invocation.
                    self.resume_fuel = None;
                    return Ok(Some(ex));
                }
                if frames.is_empty() {
                    // A catch targeting the function label can also return
                    // immediately without executing another source boundary.
                    self.resume_fuel = None;
                }
            }
            0xe2 => {
                let Immediate::Index(n) = i.immediate else {
                    return Err(trap("suspend immediate"));
                };
                let tag = self.instances[ii].tags[n as usize];
                let signature = self.tags[tag.0.index].clone();
                let handler = prompts.iter().enumerate().rev().find_map(|(p, prompt)| {
                    prompt
                        .handlers
                        .iter()
                        .find_map(|(t, l)| if *t == tag { l.map(|l| (p, l)) } else { None })
                });
                let Some((p, label)) = handler else {
                    let payload = args(operands, signature.params.len())?;
                    self.unhandled_continuation(
                        ii,
                        (tag, signature.results, payload),
                        frames,
                        operands,
                        prompts,
                    )?;
                    return Ok(None);
                };

                let (ty, cont_signature) = self.handler_type(&prompts[p], frames, label)?;
                let payload = args(operands, signature.params.len())?;
                self.check_continuation_bytes(
                    Self::continuation_header_bytes(&cont_signature).saturating_add(capture_bytes(
                        &frames[prompts[p].frame_depth..],
                        operands.len().saturating_sub(prompts[p].operand_depth),
                        &prompts[p + 1..],
                    )),
                )?;
                let roots = Self::segment_roots(
                    &frames[prompts[p].frame_depth..],
                    &operands[prompts[p].operand_depth..],
                );
                let ids = (
                    type_ids(
                        &cont_signature.params,
                        Some(&self.instances[prompts[p].instance].module),
                    )?,
                    type_ids(
                        &cont_signature.results,
                        Some(&self.instances[prompts[p].instance].module),
                    )?,
                );
                let r = self.reserve_continuation(ty, roots)?;
                let segment = Self::capture(frames, operands, prompts, p, self.current_fuel);
                prompts.pop();
                self.continuations.insert(
                    r,
                    ContinuationState {
                        body: Some(Body::Segment(segment)),
                        signature: cont_signature,
                        type_ids: ids,
                        bound: vec![],
                    },
                );
                operands.extend(payload);
                operands.push(Value::Ref(RefValue::Gc(r)));
                Self::branch_handler(frames, operands, label as usize)?;
            }
            0xe6 => {
                let Immediate::Pair(ty, tag) = i.immediate else {
                    return Err(trap("switch immediate"));
                };
                let (_, signature) = self.continuation_type(ii, ty as usize)?;
                let peer = handle(operands)?;
                self.continuation_ready(peer)?;
                let tag = self.instances[ii].tags[tag as usize];
                let selected = prompts
                    .iter()
                    .rposition(|p| p.handlers.iter().any(|(t, l)| *t == tag && l.is_none()));
                let Some(p) = selected else {
                    operands.push(Value::Ref(RefValue::Gc(peer)));
                    if let Some(frame) = frames.last_mut() {
                        frame.pc = frame.pc.saturating_sub(1);
                    }
                    self.unhandled_continuation(
                        ii,
                        (tag, vec![], vec![]),
                        frames,
                        operands,
                        prompts,
                    )?;
                    return Ok(None);
                };

                let Some(ValType::Ref(RefType {
                    heap: HeapType::Defined(out) | HeapType::ExactDefined(out),
                    ..
                })) = signature.params.last()
                else {
                    return Err(trap("switch requires continuation parameter"));
                };
                let (out_ty, out_signature) = self.continuation_type(ii, out.as_usize())?;
                let mut arguments = args(operands, signature.params.len() - 1)?;
                self.check_continuation_bytes(
                    Self::continuation_header_bytes(&out_signature).saturating_add(capture_bytes(
                        &frames[prompts[p].frame_depth..],
                        operands.len().saturating_sub(prompts[p].operand_depth),
                        &prompts[p + 1..],
                    )),
                )?;
                let roots = Self::segment_roots(
                    &frames[prompts[p].frame_depth..],
                    &operands[prompts[p].operand_depth..],
                );
                let outgoing = self.reserve_continuation(out_ty, roots)?;
                let segment = Self::capture(frames, operands, prompts, p, self.current_fuel);
                self.continuations.insert(
                    outgoing,
                    ContinuationState {
                        body: Some(Body::Segment(segment)),
                        type_ids: (
                            type_ids(&out_signature.params, Some(&self.instances[ii].module))?,
                            type_ids(&out_signature.results, Some(&self.instances[ii].module))?,
                        ),
                        signature: out_signature,
                        bound: vec![],
                    },
                );
                arguments.push(Value::Ref(RefValue::Gc(outgoing)));
                if let Some(ex) = self.install(peer, arguments, frames, operands, prompts)?
                    && !self.unwind_exception(ex, frames, operands)?
                {
                    return Ok(Some(ex));
                }
            }
            _ => return Err(trap("unknown continuation instruction")),
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instantiate(bytes: &[u8]) -> (Store, Instance) {
        let e = Engine::new(Config::default()).unwrap();
        let m = e.compile(bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        (s, i)
    }
    fn invoke(s: &mut Store, i: Instance, name: &str) -> Result<RunOutcome> {
        let External::Func(f) = i.export(s, name)? else {
            panic!()
        };
        s.invoke(f, &[])
    }
    const GENERATOR: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 20, 5, 96, 0, 1, 127, 93, 0, 96, 1, 127, 1, 127, 93, 2, 96,
        0, 2, 127, 100, 3, 3, 6, 5, 0, 0, 0, 0, 0, 13, 3, 1, 0, 2, 7, 23, 3, 3, 114, 117, 110, 0,
        1, 5, 116, 119, 105, 99, 101, 0, 3, 5, 98, 111, 117, 110, 100, 0, 4, 9, 9, 2, 3, 0, 1, 0,
        3, 0, 1, 2, 10, 97, 5, 6, 0, 65, 7, 226, 0, 11, 29, 1, 1, 99, 3, 2, 4, 210, 0, 224, 1, 227,
        1, 1, 0, 0, 0, 15, 11, 33, 0, 26, 65, 9, 32, 0, 227, 3, 0, 11, 4, 0, 65, 5, 11, 20, 1, 1,
        99, 1, 210, 2, 224, 1, 34, 0, 227, 1, 0, 26, 32, 0, 227, 1, 0, 11, 32, 1, 1, 99, 3, 2, 4,
        210, 0, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0, 26, 65, 19, 32, 0, 225, 3, 1, 227, 1, 0,
        11,
    ];
    const NESTED: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 19, 5, 96, 0, 1, 127, 93, 0, 96, 1, 127, 1, 127, 93, 2, 96,
        0, 1, 100, 3, 3, 4, 3, 0, 0, 0, 13, 3, 1, 0, 0, 7, 7, 1, 3, 114, 117, 110, 0, 2, 9, 6, 1,
        3, 0, 2, 0, 1, 10, 47, 3, 4, 0, 226, 0, 11, 11, 0, 210, 0, 224, 1, 227, 1, 1, 1, 0, 11, 28,
        1, 1, 99, 3, 2, 4, 210, 1, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0, 65, 39, 32, 0, 227,
        3, 0, 11,
    ];
    const THROW: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 23, 6, 96, 0, 1, 127, 93, 0, 96, 1, 127, 1, 127, 93, 2, 96,
        0, 1, 100, 3, 96, 1, 127, 0, 3, 3, 2, 0, 0, 13, 5, 2, 0, 0, 0, 5, 7, 7, 1, 3, 114, 117,
        110, 0, 1, 9, 5, 1, 3, 0, 1, 0, 10, 46, 2, 14, 0, 2, 127, 31, 127, 1, 0, 1, 0, 226, 0, 11,
        11, 11, 29, 1, 1, 99, 3, 2, 4, 210, 0, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0, 65, 29,
        32, 0, 228, 3, 1, 0, 11,
    ];
    const GC: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 26, 7, 96, 0, 1, 127, 93, 0, 96, 1, 127, 1, 127, 93, 2, 96,
        0, 1, 100, 3, 95, 1, 127, 0, 96, 0, 0, 2, 15, 1, 3, 101, 110, 118, 7, 99, 111, 108, 108,
        101, 99, 116, 0, 6, 3, 3, 2, 0, 0, 13, 3, 1, 0, 0, 7, 7, 1, 3, 114, 117, 110, 0, 2, 9, 5,
        1, 3, 0, 1, 1, 10, 55, 2, 22, 1, 1, 99, 5, 65, 205, 0, 251, 0, 5, 33, 0, 226, 0, 26, 32, 0,
        251, 2, 5, 0, 11, 30, 1, 1, 99, 3, 2, 4, 210, 1, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0,
        16, 0, 65, 0, 32, 0, 227, 3, 0, 11,
    ];

    #[test]
    fn t142_generator_resume_and_bind() {
        let (mut s, i) = instantiate(GENERATOR);
        assert_eq!(
            invoke(&mut s, i, "run").unwrap(),
            RunOutcome::Returned(vec![Value::I32(9)])
        );
        assert_eq!(
            invoke(&mut s, i, "bound").unwrap(),
            RunOutcome::Returned(vec![Value::I32(19)])
        );
    }
    #[test]
    fn t140_second_resume_traps() {
        let (mut s, i) = instantiate(GENERATOR);
        assert_eq!(
            invoke(&mut s, i, "twice").unwrap_err().kind,
            ErrorKind::Trap
        );
    }
    #[test]
    fn t141_suspend_skips_same_tag_switch_prompt() {
        let (mut s, i) = instantiate(NESTED);
        assert_eq!(
            invoke(&mut s, i, "run").unwrap(),
            RunOutcome::Returned(vec![Value::I32(39)])
        );
    }
    #[test]
    fn t144_resume_throw_uses_captured_exception_handler() {
        let (mut s, i) = instantiate(THROW);
        assert_eq!(
            invoke(&mut s, i, "run").unwrap(),
            RunOutcome::Returned(vec![Value::I32(29)])
        );
    }
    #[test]
    fn t145_detached_frame_keeps_gc_local_alive() {
        let e = Engine::new(Config::default()).unwrap();
        let m = e.compile(GC).unwrap();
        let mut s = Store::new(&e);
        let host = s
            .create_host_func_with_caller(FuncType::default(), |caller, _| {
                assert!(caller.collect()?.live >= 2);
                Ok(vec![])
            })
            .unwrap();
        let mut linker = Linker::new(&e);
        linker
            .define("env", "collect", External::Func(host))
            .unwrap();
        let i = linker.instantiate(&mut s, &m).unwrap();
        assert_eq!(
            invoke(&mut s, i, "run").unwrap(),
            RunOutcome::Returned(vec![Value::I32(77)])
        );
        assert_eq!(s.collect().unwrap().live, 0);
        assert!(s.continuations.is_empty());
    }
    #[test]
    fn t007_host_identity_rejects_foreign_reference() {
        let e = Engine::new(Config::default()).unwrap();
        let mut a = Store::new(&e);
        let f = a
            .create_host_func(FuncType::default(), |_| Ok(vec![]))
            .unwrap();
        let mut b = Store::new(&e);
        let identity = b
            .create_host_func(
                FuncType::new(vec![ValType::FUNCREF], vec![ValType::FUNCREF]),
                |args| Ok(args.to_vec()),
            )
            .unwrap();
        assert_eq!(
            b.invoke(identity, &[Value::Ref(RefValue::Func(f))])
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
    }
    const SWITCH: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 15, 4, 96, 0, 1, 127, 93, 0, 96, 1, 100, 1, 1, 127, 93, 2,
        3, 4, 3, 2, 0, 0, 13, 3, 1, 0, 0, 7, 7, 1, 3, 114, 117, 110, 0, 2, 9, 6, 1, 3, 0, 2, 0, 1,
        10, 42, 3, 10, 0, 32, 0, 227, 1, 0, 65, 2, 106, 11, 17, 1, 1, 127, 65, 7, 33, 0, 210, 0,
        224, 3, 230, 3, 0, 32, 0, 11, 11, 0, 210, 1, 224, 1, 227, 1, 1, 1, 0, 11,
    ];

    #[test]
    fn t143_switch_preserves_outgoing_local() {
        let (mut s, i) = instantiate(SWITCH);
        assert_eq!(
            invoke(&mut s, i, "run").unwrap(),
            RunOutcome::Returned(vec![Value::I32(9)])
        );
    }
    #[test]
    fn t140_failed_resume_reservation_does_not_consume() {
        let (mut s, _) = instantiate(GENERATOR);
        let f = s.instances[0].funcs[2];
        let mut values = vec![Value::Ref(RefValue::Func(f))];
        let mut frames = vec![];
        let mut prompts = vec![];
        s.cont_step(
            0,
            &Instruction {
                opcode: 0xe0,
                immediate: Immediate::Index(1),
                span: Span::default(),
            },
            &mut frames,
            &mut values,
            &mut prompts,
        )
        .unwrap();
        let Value::Ref(RefValue::Gc(r)) = values[0] else {
            panic!()
        };
        s.engine.config.limits.max_call_depth = 0;
        assert_eq!(
            s.cont_step(
                0,
                &Instruction {
                    opcode: 0xe3,
                    immediate: Immediate::Resume {
                        ty: 1,
                        tag: None,
                        handlers: vec![]
                    },
                    span: Span::default()
                },
                &mut frames,
                &mut values,
                &mut prompts
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        assert!(s.continuation_ready(r).is_ok());
    }
    const UNHANDLED: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 18, 4, 96, 0, 1, 127, 96, 1, 127, 1, 127, 93, 1, 96, 1,
        100, 2, 1, 127, 3, 3, 2, 0, 3, 13, 3, 1, 0, 1, 7, 16, 2, 3, 114, 117, 110, 0, 0, 6, 114,
        101, 115, 117, 109, 101, 0, 1, 10, 18, 2, 6, 0, 65, 7, 226, 0, 11, 9, 0, 65, 9, 32, 0, 227,
        2, 0, 11,
    ];

    #[test]
    fn t146_unhandled_suspension_is_resumable() {
        let (mut s, i) = instantiate(UNHANDLED);
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = invoke(&mut s, i, "run").unwrap()
        else {
            panic!("expected suspension")
        };
        assert_eq!(payload, vec![Value::I32(7)]);
        s.collect().unwrap();
        let External::Func(f) = i.export(&s, "resume").unwrap() else {
            panic!()
        };
        assert_eq!(
            s.invoke(f, &[Value::Ref(RefValue::Gc(continuation))])
                .unwrap(),
            RunOutcome::Returned(vec![Value::I32(9)])
        );
    }
    #[test]
    fn t145_suspension_carries_remaining_fuel() {
        let (mut s, i) = instantiate(UNHANDLED);
        s.engine.config.limits.fuel = Some(2);
        let RunOutcome::Suspended { continuation, .. } = invoke(&mut s, i, "run").unwrap() else {
            panic!()
        };
        s.engine.config.limits.fuel = Some(100);
        let External::Func(f) = i.export(&s, "resume").unwrap() else {
            panic!()
        };
        let error = s
            .invoke(f, &[Value::Ref(RefValue::Gc(continuation))])
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert!(error.message.contains("fuel"));
    }
    #[test]
    fn t140_aggregate_bytes_fail_before_capture_and_release_after_drop() {
        let (mut store, instance) = instantiate(UNHANDLED);
        store.engine.config.limits.max_continuation_bytes = 0;
        assert_eq!(
            invoke(&mut store, instance, "run").unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert!(store.continuations.is_empty());
        store.engine.config.limits.max_continuation_bytes = 1024 * 1024;
        let RunOutcome::Suspended {
            continuation,
            payload,
            ..
        } = invoke(&mut store, instance, "run").unwrap()
        else {
            panic!()
        };
        assert!(store.continuation_bytes() > std::mem::size_of::<ContinuationState>());
        assert!(store.continuation_ready(continuation).is_ok());
        store.release_published(&payload);
        store.release_published(&[Value::Ref(RefValue::Gc(continuation))]);
        store.collect().unwrap();
        assert_eq!(store.continuation_bytes(), 0);
    }
    #[test]
    fn t147_repeated_suspension_release_and_cancel_do_not_retain_segments() {
        let (mut store, instance) = instantiate(UNHANDLED);
        for _ in 0..64 {
            let RunOutcome::Suspended {
                continuation,
                payload,
                ..
            } = invoke(&mut store, instance, "run").unwrap()
            else {
                panic!()
            };
            let value = Value::Ref(RefValue::Gc(continuation));
            let token = store.root_value(value).unwrap();
            store.release_published(&payload);
            store.release_published(&[value]);
            store.collect().unwrap();
            assert!(store.continuation_ready(continuation).is_ok());
            store.cancellation_token().cancel();
            assert_eq!(
                invoke(&mut store, instance, "run").unwrap_err().kind,
                ErrorKind::ResourceLimit
            );
            store.cancellation_token().reset();
            drop(token);
            store.collect().unwrap();
            assert_eq!(store.continuation_bytes(), 0);
            assert!(store.continuations.is_empty());
            assert!(store.active_roots.is_empty());
        }
    }
    #[test]
    fn t145_resume_throw_ref_uncaught_consumes_and_cleans_segment() {
        let (mut store, instance) = instantiate(UNHANDLED);
        let RunOutcome::Suspended { continuation, .. } =
            invoke(&mut store, instance, "run").unwrap()
        else {
            panic!()
        };
        let tag = store
            .create_tag(FuncType::new(vec![ValType::I32], vec![]))
            .unwrap();
        let ex = store.create_exception(tag, &[Value::I32(23)]).unwrap();
        let mut values = vec![
            Value::Ref(RefValue::Exn(ex)),
            Value::Ref(RefValue::Gc(continuation)),
        ];
        let mut frames = vec![];
        let mut prompts = vec![];
        let outcome = store
            .cont_step(
                instance.0.index,
                &Instruction {
                    opcode: 0xe5,
                    immediate: Immediate::Resume {
                        ty: 2,
                        tag: None,
                        handlers: vec![],
                    },
                    span: Span::default(),
                },
                &mut frames,
                &mut values,
                &mut prompts,
            )
            .unwrap();
        assert_eq!(outcome, Some(ex));
        assert!(frames.is_empty());
        assert!(store.continuation_ready(continuation).is_err());
        store.release_published(&[
            Value::Ref(RefValue::Gc(continuation)),
            Value::Ref(RefValue::Exn(ex)),
        ]);
        store.collect().unwrap();
        assert_eq!(store.continuation_bytes(), 0);
        assert!(store.exceptions.get(ex.0.index).is_none());
    }
    #[test]
    fn t141_multiple_bind_preserves_order_and_invalidates_old_capabilities() {
        let (mut store, instance) = instantiate(GENERATOR);
        let canonical = store.instances[instance.0.index].module.data.canonical[1];
        let f = store.instances[instance.0.index].funcs[2];
        let old = store.reserve_continuation(canonical, vec![]).unwrap();
        store.continuations.insert(
            old,
            ContinuationState {
                body: Some(Body::Start(f)),
                signature: FuncType::new(vec![ValType::I32, ValType::I32], vec![ValType::I32]),
                type_ids: (vec![None, None], vec![None]),
                bound: vec![],
            },
        );
        let mut values = vec![Value::I32(11), Value::Ref(RefValue::Gc(old))];
        let mut frames = vec![];
        let mut prompts = vec![];
        let bind = |target| Instruction {
            opcode: 0xe1,
            immediate: Immediate::Pair(1, target),
            span: Span::default(),
        };
        store
            .cont_step(
                instance.0.index,
                &bind(3),
                &mut frames,
                &mut values,
                &mut prompts,
            )
            .unwrap();
        let Value::Ref(RefValue::Gc(mid)) = values.pop().unwrap() else {
            panic!()
        };
        assert!(store.continuation_ready(old).is_err());
        values.extend([Value::I32(22), Value::Ref(RefValue::Gc(mid))]);
        store
            .cont_step(
                instance.0.index,
                &bind(1),
                &mut frames,
                &mut values,
                &mut prompts,
            )
            .unwrap();
        let Value::Ref(RefValue::Gc(new)) = values.pop().unwrap() else {
            panic!()
        };
        assert_eq!(
            store.continuation_ready(new).unwrap().bound,
            vec![Value::I32(11), Value::I32(22)]
        );
        assert!(store.continuation_ready(mid).is_err());
        values.push(Value::Ref(RefValue::Gc(old)));
        assert!(
            store
                .cont_step(
                    instance.0.index,
                    &bind(1),
                    &mut frames,
                    &mut values,
                    &mut prompts
                )
                .is_err()
        );
    }
    #[test]
    fn t146_host_suspension_forward_validates_capability_and_payload() {
        let (mut store, instance) = instantiate(UNHANDLED);
        let outcome = invoke(&mut store, instance, "run").unwrap();
        let RunOutcome::Suspended {
            tag,
            payload,
            continuation,
        } = outcome.clone()
        else {
            panic!()
        };
        let host = store
            .create_host_func_with_outcome(
                FuncType::new(vec![], vec![ValType::I32]),
                move |_, _| Ok(outcome.clone()),
            )
            .unwrap();
        assert!(matches!(
            store.invoke(host, &[]).unwrap(),
            RunOutcome::Suspended { .. }
        ));
        let reject = |store: &mut Store, outcome: RunOutcome| {
            let host = store
                .create_host_func_with_outcome(
                    FuncType::new(vec![], vec![ValType::I32]),
                    move |_, _| Ok(outcome.clone()),
                )
                .unwrap();
            assert_eq!(store.invoke(host, &[]).unwrap_err().kind, ErrorKind::Host);
        };
        let (mut foreign, foreign_instance) = instantiate(UNHANDLED);
        let RunOutcome::Suspended {
            tag: foreign_tag,
            continuation: foreign_cap,
            ..
        } = invoke(&mut foreign, foreign_instance, "run").unwrap()
        else {
            panic!()
        };
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag,
                payload: payload.clone(),
                continuation: foreign_cap,
            },
        );
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag: foreign_tag,
                payload: payload.clone(),
                continuation,
            },
        );
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag,
                payload: vec![Value::I64(7)],
                continuation,
            },
        );
        let invalid = store
            .gc
            .allocate(
                gc::Object {
                    ty: store.gc.get(continuation).unwrap().ty,
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag,
                payload: payload.clone(),
                continuation: invalid,
            },
        );
        store.collect().unwrap();
        assert!(store.gc.get(invalid).is_err());
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag,
                payload: payload.clone(),
                continuation: invalid,
            },
        );
        let ref_tag = store
            .create_tag(FuncType::new(vec![ValType::FUNCREF], vec![ValType::I32]))
            .unwrap();
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag: ref_tag,
                payload: vec![Value::Ref(RefValue::Null(HeapType::Extern))],
                continuation,
            },
        );
        let wrong_signature = store
            .create_tag(FuncType::new(vec![ValType::I32], vec![ValType::I64]))
            .unwrap();
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag: wrong_signature,
                payload: payload.clone(),
                continuation,
            },
        );
        let External::Func(resume) = instance.export(&store, "resume").unwrap() else {
            panic!()
        };
        assert_eq!(
            store
                .invoke(resume, &[Value::Ref(RefValue::Gc(continuation))])
                .unwrap(),
            RunOutcome::Returned(vec![Value::I32(9)])
        );
        reject(
            &mut store,
            RunOutcome::Suspended {
                tag,
                payload,
                continuation,
            },
        );
    }
    #[test]
    fn t146_guest_call_rejects_host_suspension_without_capturing_outer_frames() {
        let (mut store, nested) = instantiate(UNHANDLED);
        let External::Func(run) = nested.export(&store, "run").unwrap() else {
            panic!()
        };
        let host = store
            .create_host_func_with_outcome(
                FuncType::new(vec![], vec![ValType::I32]),
                move |caller, _| caller.invoke(run, &[]),
            )
            .unwrap();
        // Imported h.f()->i32; outer f calls it. Native callback/outerguest frames cannot be captured.
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 2, 7, 1, 1, b'h', 1, b'f', 0, 0,
            3, 2, 1, 0, 7, 5, 1, 1, b'f', 0, 1, 10, 6, 1, 4, 0, 16, 0, 11,
        ];
        let module = store.engine.compile(&bytes).unwrap();
        let mut linker = Linker::new(&store.engine);
        linker.define("h", "f", External::Func(host)).unwrap();
        let outer = linker.instantiate(&mut store, &module).unwrap();
        assert_eq!(
            invoke(&mut store, outer, "f").unwrap_err().kind,
            ErrorKind::Host
        );
        assert!(store.active_roots.is_empty());
    }
}
