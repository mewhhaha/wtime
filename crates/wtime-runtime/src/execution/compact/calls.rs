//! Compact-tier call handling. Instance-owned Store handles never enter module IR.
use super::*;

pub(super) const CALL_MAX_WORK: usize = 100_000;
pub(super) const CALL_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct CallType {
    type_index: usize,
    expected: CanonicalTypeId,
    numeric: bool,
}

pub(super) struct CallTypes {
    entries: Vec<CallType>,
    pub(super) work: usize,
    pub(super) bytes: usize,
}

fn numeric(ty: &ValType) -> bool {
    matches!(
        ty,
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64 | ValType::V128
    )
}

fn sized<T>(count: usize, header: usize) -> Option<usize> {
    count
        .checked_mul(std::mem::size_of::<T>())?
        .checked_add(header)
}

impl CallTypes {
    // The temporary map is linear in imports + definitions + signature values.
    // Both module preparation and binding construction charge its complete work.
    pub(super) fn prepare(module: &Module, max_work: usize, max_bytes: usize) -> Option<Self> {
        let decoded = module.data.inner.decoded();
        let max_work = max_work.min(CALL_MAX_WORK);
        let max_bytes = max_bytes.min(CALL_MAX_BYTES);
        let upper = decoded.imports.len().checked_add(decoded.functions.len())?;
        if upper > max_work {
            return None;
        }
        if sized::<CallType>(upper, std::mem::size_of::<Self>())? > max_bytes {
            return None;
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(upper).ok()?;
        let bytes = sized::<CallType>(entries.capacity(), std::mem::size_of::<Self>())?;
        if bytes > max_bytes {
            return None;
        }
        let mut work = upper;
        let indices = decoded
            .imports
            .iter()
            .filter_map(|import| match import.kind {
                ImportKind::Func(index) | ImportKind::ExactFunc(index) => Some(index),
                _ => None,
            })
            .chain(decoded.functions.iter().copied());
        for index in indices {
            let ty = decoded.types.get(index.as_usize())?;
            work = work
                .checked_add(ty.params.len())?
                .checked_add(ty.results.len())?;
            if work > max_work {
                return None;
            }
            entries.push(CallType {
                type_index: index.as_usize(),
                expected: *module.data.canonical.get(index.as_usize())?,
                numeric: ty.params.iter().chain(&ty.results).all(numeric),
            });
        }
        Some(Self {
            entries,
            work,
            bytes,
        })
    }

    pub(super) fn supports(&self, index: u32) -> bool {
        self.entries
            .get(index as usize)
            .is_some_and(|ty| ty.numeric)
    }
}

#[derive(Clone, Copy, Debug)]
struct Binding {
    target: Func,
    expected_type: usize,
    expected: CanonicalTypeId,
    actual: CanonicalTypeId,
    params: usize,
    results: usize,
    host: bool,
}

#[derive(Debug)]
pub(crate) struct CallBindings {
    store: u64,
    engine: u64,
    source: Arc<[u8]>,
    entries: Vec<Option<Binding>>,
    work: usize,
    retained_bytes: usize,
}

impl CallBindings {
    // Bounds are per instance, including the temporary signature map in peak
    // construction storage. Repeated instances each own their own bindings.
    pub(crate) fn prepare(store: &Store, module: &Module, funcs: &[Func]) -> Option<Self> {
        Self::bounded(store, module, funcs, CALL_MAX_WORK, CALL_MAX_BYTES)
    }

    fn bounded(
        store: &Store,
        module: &Module,
        funcs: &[Func],
        max_work: usize,
        max_bytes: usize,
    ) -> Option<Self> {
        let max_work = max_work.min(CALL_MAX_WORK);
        let max_bytes = max_bytes.min(CALL_MAX_BYTES);
        if store.engine.id != module.data.engine {
            return None;
        }
        let types = CallTypes::prepare(module, max_work, max_bytes)?;
        if types.entries.len() != funcs.len() {
            return None;
        }
        let mut work = types
            .work
            .checked_add(funcs.len())?
            .checked_add(module.data.inner.decoded().code.len())?;
        if work > max_work {
            return None;
        }
        let remaining_bytes = max_bytes.checked_sub(types.bytes)?;
        if sized::<Option<Binding>>(funcs.len(), std::mem::size_of::<Self>())? > remaining_bytes {
            return None;
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(funcs.len()).ok()?;
        let retained_bytes =
            sized::<Option<Binding>>(entries.capacity(), std::mem::size_of::<Self>())?;
        if retained_bytes > remaining_bytes {
            return None;
        }
        for (target, expected) in funcs.iter().copied().zip(&types.entries) {
            let actual = &store.funcs[store.check(target.0, &store.funcs).ok()?];
            work = work
                .checked_add(actual.ty.params.len())?
                .checked_add(actual.ty.results.len())?;
            if work > max_work {
                return None;
            }
            if !expected.numeric {
                entries.push(None);
                continue;
            }
            // Normal linker checks establish subtype acceptance before this
            // plan is built. Retain both IDs; never require equality here.
            if !actual
                .ty
                .params
                .iter()
                .chain(&actual.ty.results)
                .all(numeric)
            {
                return None;
            }
            entries.push(Some(Binding {
                target,
                expected_type: expected.type_index,
                expected: expected.expected,
                actual: actual.canonical,
                params: actual.ty.params.len(),
                results: actual.ty.results.len(),
                host: actual.host.is_some(),
            }));
        }
        Some(Self {
            store: store.id,
            engine: store.engine.id,
            source: module.data.inner.decoded().bytes.clone(),
            entries,
            work,
            retained_bytes,
        })
    }

    pub(crate) fn resolve(&self, store: &Store, inst: &Inst, index: u32) -> Result<(Func, usize)> {
        if self.store != store.id
            || self.engine != store.engine.id
            || self.engine != inst.module.data.engine
            || !Arc::ptr_eq(&self.source, &inst.module.data.inner.decoded().bytes)
            || self.entries.len() != inst.funcs.len()
            || self.work > CALL_MAX_WORK
            || self.retained_bytes > CALL_MAX_BYTES
        {
            return Err(trap("compact call binding identity"));
        }
        let binding = self
            .entries
            .get(index as usize)
            .copied()
            .flatten()
            .ok_or_else(|| trap("missing compact call binding"))?;
        if inst.funcs.get(index as usize) != Some(&binding.target) {
            return Err(trap("compact call target identity"));
        }
        if inst.module.data.canonical.get(binding.expected_type) != Some(&binding.expected) {
            return Err(trap("compact call expected signature identity"));
        }
        let actual = &store.funcs[store.check(binding.target.0, &store.funcs)?];
        if actual.canonical != binding.actual
            || actual.ty.params.len() != binding.params
            || actual.ty.results.len() != binding.results
            || actual.host.is_some() != binding.host
        {
            return Err(trap("compact call binding signature"));
        }
        Ok((binding.target, binding.params))
    }
}

// Copy the checked argument range directly into its final local storage. The
// ordinary frame constructor remains authoritative for every decoded call.
pub(crate) fn guest_frame(
    store: &Store,
    target: Func,
    args: &[Value],
    base: usize,
) -> Result<Frame> {
    guest_frame_with_release(store, target, args, base, 0)
}

pub(crate) fn guest_frame_with_release(
    store: &Store,
    target: Func,
    args: &[Value],
    base: usize,
    releasing_slots: usize,
) -> Result<Frame> {
    let function = &store.funcs[store.check(target.0, &store.funcs)?];
    if function.host.is_some() {
        return Err(trap("compact guest frame for host"));
    }
    for value in args {
        store.validate_value(*value)?;
    }
    if args.len() != function.ty.params.len()
        || !args
            .iter()
            .zip(&function.ty.params)
            .all(|(value, ty)| match ty {
                ValType::Ref(reference) => {
                    matches_type(*value, *ty)
                        || store
                            .gc_matches(function.instance, *value, *reference)
                            .unwrap_or(false)
                }
                _ => matches_type(*value, *ty),
            })
    {
        return Err(err(ErrorKind::Host, "argument type mismatch"));
    }
    let instance = &store.instances[function.instance];
    let module = &instance.module;
    let body = &module.data.inner.decoded().code[function.body];
    let layout = module.function_layout(function.body);
    let local_types = layout
        .as_ref()
        .map_or(body.locals.as_slice(), |plan| plan.locals);
    let total = args
        .len()
        .checked_add(local_types.len())
        .ok_or_else(|| err(ErrorKind::ResourceLimit, "compact local count overflow"))?;
    store.check_local_slot_budget(total, releasing_slots)?;
    let mut locals = Vec::new();
    locals
        .try_reserve_exact(total)
        .map_err(|_| err(ErrorKind::ResourceLimit, "compact local allocation"))?;
    locals.extend_from_slice(args);
    locals.extend(local_types.iter().map(|ty| Value::zero(*ty)));
    let compact = store.engine.policy == ExecutionPolicy::Compact
        && module.data.compact.as_ref().is_some_and(|program| {
            program.contains(function.body)
                && (!program.has_calls(function.body) || instance.call_bindings.is_some())
                && (!program.has_memory(function.body) || instance.memory_bindings.is_some())
        });
    #[cfg(feature = "performance-counters")]
    {
        store.perf.record(perf::Event::Frame, 1);
        if store.engine.policy == ExecutionPolicy::Compact && !compact {
            store.perf.record(perf::Event::CompactFallbackFrame, 1);
        }
    }
    Ok(Frame {
        #[cfg(feature = "register-executor")]
        register: None,
        compact,
        function: function.clone(),
        pc: 0,
        locals,
        base,
        labels: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p041_private.rs");
}
