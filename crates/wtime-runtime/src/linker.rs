use super::*;
impl Linker {
    pub fn new(e: &Engine) -> Self {
        Self {
            engine: e.id,
            defs: HashMap::new(),
        }
    }
    pub fn define(&mut self, m: &str, n: &str, v: External) -> Result<()> {
        if self.defs.contains_key(&(m.into(), n.into())) {
            return Err(err(ErrorKind::Link, "duplicate definition"));
        }
        self.defs.insert((m.into(), n.into()), v);
        Ok(())
    }
    fn instantiate_inner(&self, s: &mut Store, m: &Module) -> Result<Instance> {
        if self.engine != s.engine.id || m.data.engine != self.engine {
            return Err(err(ErrorKind::Link, "engine mismatch"));
        }
        let d = m.data.inner.decoded();
        s.reserve_objects(
            1 + d.functions.len()
                + d.memories.len()
                + d.tables.len()
                + d.globals.len()
                + d.tags.len(),
        )?;
        let ii = s.instances.len();
        let mut inst = Inst {
            #[cfg(feature = "compact-executor")]
            memory_bindings: None,
            #[cfg(feature = "compact-executor")]
            call_sites: None,
            #[cfg(feature = "compact-executor")]
            call_bindings: None,
            tags: vec![],
            module: m.clone(),
            funcs: vec![],
            mems: vec![],
            tabs: vec![],
            globals: vec![],
            exports: HashMap::new(),
            data: vec![],
            elems: vec![],
        };
        for im in &d.imports {
            let x = *self
                .defs
                .get(&(im.module.clone(), im.name.clone()))
                .ok_or_else(|| {
                    err(
                        ErrorKind::Link,
                        format!("missing import {}.{}", im.module, im.name),
                    )
                })?;
            match (&im.kind, x) {
                (ImportKind::Tag(t), External::Tag(tag)) => {
                    let k = s.check(tag.0, &s.tags)?;
                    let type_matches = if let Some(actual) = s.tag_canonical.get(&k) {
                        *actual == m.data.canonical[t.function.as_usize()]
                    } else {
                        s.tags[k] == d.types[t.function.as_usize()]
                    };
                    if !type_matches {
                        return Err(err(ErrorKind::Link, "tag signature mismatch"));
                    }
                    inst.tags.push(tag)
                }
                (ImportKind::Func(t) | ImportKind::ExactFunc(t), External::Func(f)) => {
                    let k = s.check(f.0, &s.funcs)?;
                    let actual = s.funcs[k].canonical;
                    let expected = m.data.canonical[t.as_usize()];
                    let exact = matches!(&im.kind, ImportKind::ExactFunc(_));
                    if !s
                        .engine
                        .canonical
                        .lock()
                        .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                        .is_subtype(
                            actual,
                            expected,
                            exact,
                            s.engine.config.limits.max_type_work,
                        )?
                    {
                        return Err(err(ErrorKind::Link, "function import type mismatch"));
                    }
                    inst.funcs.push(f)
                }
                (ImportKind::Global(t), External::Global(g)) => {
                    let k = s.check(g.0, &s.globals)?;
                    let actual = &s.globals[k];
                    let owner = actual.instance.map(|ii| &s.instances[ii].module);
                    if actual.ty.mutable != t.mutable
                        || !import_subtype(s, actual.ty.value, owner, t.value, m)?
                        || (t.mutable
                            && !import_subtype(
                                s,
                                t.value,
                                Some(m),
                                actual.ty.value,
                                owner.unwrap_or(m),
                            )?)
                    {
                        return Err(err(ErrorKind::Link, "global import mismatch"));
                    }
                    inst.globals.push(g)
                }
                (ImportKind::Memory(t), External::Memory(v)) => {
                    let k = s.check(v.0, &s.mems)?;
                    let mut a = s.mems[k].ty;
                    a.limits.min = (s.mems[k].len()? as u64) >> a.page_size_log2;
                    if a.index_type != t.index_type
                        || a.shared != t.shared
                        || a.page_size_log2 != t.page_size_log2
                        || !limits_match(a.limits, t.limits)
                    {
                        return Err(err(ErrorKind::Link, "memory import mismatch"));
                    }
                    inst.mems.push(v)
                }
                (ImportKind::Table(t), External::Table(v)) => {
                    let k = s.check(v.0, &s.tabs)?;
                    let mut a = s.tabs[k].ty;
                    a.limits.min = s.tabs[k].values.len() as u64;
                    let owner = s.tabs[k].instance.map(|ii| &s.instances[ii].module);
                    if !import_subtype(
                        s,
                        ValType::Ref(a.element),
                        owner,
                        ValType::Ref(t.element),
                        m,
                    )? || !import_subtype(
                        s,
                        ValType::Ref(t.element),
                        Some(m),
                        ValType::Ref(a.element),
                        owner.unwrap_or(m),
                    )? || a.index_type != t.index_type
                        || !limits_match(a.limits, t.limits)
                    {
                        return Err(err(ErrorKind::Link, "table import mismatch"));
                    }
                    inst.tabs.push(v)
                }
                _ => return Err(err(ErrorKind::Link, "import kind mismatch")),
            }
        }
        for t in &d.tags {
            let tag = s.create_module_tag(m, t.function)?;
            s.tag_instances.insert(tag.0.index, ii);
            s.tag_canonical
                .insert(tag.0.index, m.data.canonical[t.function.as_usize()]);
            inst.tags.push(tag)
        }
        for (body, t) in d.functions.iter().enumerate() {
            let f = Func(s.h(s.funcs.len()));
            s.funcs.push(Function {
                host_type_instance: None,
                host_type_module: None,
                canonical: m.data.canonical[t.as_usize()],
                ty: d.types[t.as_usize()].clone(),
                instance: ii,
                body,
                host: None,
            });
            inst.funcs.push(f)
        }
        #[cfg(feature = "compact-executor")]
        if s.engine.policy == ExecutionPolicy::Compact
            && m.data.compact.as_ref().is_some_and(|program| program.needs_bindings()) {
            inst.call_bindings = execution::compact::CallBindings::prepare(s, m, &inst.funcs).map(Arc::new);
        }
        for t in &d.memories {
            let n = t
                .limits
                .min
                .checked_mul(1u64.checked_shl(t.page_size_log2 as u32).unwrap_or(0))
                .and_then(|x| usize::try_from(x).ok())
                .ok_or_else(|| err(ErrorKind::ResourceLimit, "memory allocation overflow"))?;
            if t.limits.min > s.engine.config.limits.max_memory_pages
                || n as u64 > s.engine.config.limits.max_memory_bytes
            {
                return Err(err(ErrorKind::ResourceLimit, "memory limit"));
            }

            s.reserve_memory(n as u64)?;
            inst.mems.push(Memory(s.h(s.mems.len())));
            s.mems.push(Mem::new(*t, n)?)
        }
        for (table_index, t) in d.tables.iter().enumerate() {
            if t.limits.min > s.engine.config.limits.max_table_elements {
                return Err(err(ErrorKind::ResourceLimit, "table limit"));
            }
            s.reserve_table(t.limits.min)?;
            let n = usize::try_from(t.limits.min)
                .map_err(|_| err(ErrorKind::ResourceLimit, "table allocation overflow"))?;
            inst.tabs.push(Table(s.h(s.tabs.len())));
            let init = if let Some(expr) = d.table_inits.get(table_index).and_then(|x| x.as_ref()) {
                eval_const(s, &inst, expr)?
            } else {
                Value::zero(ValType::Ref(t.element))
            };
            s.tabs.push(Tab {
                instance: Some(ii),
                ty: *t,
                values: vec![init; n],
            })
        }
        for g in &d.globals {
            let v = eval_const(s, &inst, &g.init)?;
            inst.globals.push(Global(s.h(s.globals.len())));
            s.globals.push(Glob {
                instance: Some(ii),
                ty: g.ty,
                value: v,
            })
        }
        inst.data = d
            .data
            .iter()
            .map(|x| {
                if matches!(x.mode, SegmentMode::Passive) {
                    Some(d.bytes[x.bytes.start..x.bytes.end].to_vec())
                } else {
                    None
                }
            })
            .collect();
        inst.elems = d
            .elements
            .iter()
            .map(|x| {
                if matches!(x.mode, SegmentMode::Passive) {
                    x.init
                        .iter()
                        .map(|e| eval_const(s, &inst, e))
                        .collect::<Result<Vec<_>>>()
                        .map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        #[cfg(feature = "compact-executor")]
        if s.engine.policy == ExecutionPolicy::Compact {
            inst.call_sites = execution::compact::CallSites::prepare(s, m);
        }
        #[cfg(feature = "compact-executor")]
        if s.engine.policy == ExecutionPolicy::Compact
            && m.data.compact.as_ref().is_some_and(|program| program.needs_memory_bindings()) {
            inst.memory_bindings = execution::compact::MemoryBindings::prepare(s, m, &inst.mems).map(Arc::new);
        }
        s.instances.push(inst.clone());
        for (elem_index, el) in d.elements.iter().enumerate() {
            let vals = el
                .init
                .iter()
                .map(|x| eval_const(s, &inst, x))
                .collect::<Result<Vec<_>>>()?;
            match &el.mode {
                SegmentMode::Active { index, offset } => {
                    let off = address(eval_const(s, &inst, offset)?)?;
                    let tab = &mut s.tabs[inst.tabs[*index as usize].0.index];
                    let r = range(off, vals.len(), tab.values.len())
                        .map_err(|_| trap("table out of bounds"))?;
                    tab.values[r].copy_from_slice(&vals);
                    inst.elems[elem_index] = None
                }
                SegmentMode::Passive => inst.elems[elem_index] = Some(vals),
                SegmentMode::Declarative => inst.elems[elem_index] = None,
            }
        }
        for (data_index, data) in d.data.iter().enumerate() {
            let bytes = d.bytes[data.bytes.start..data.bytes.end].to_vec();
            if let SegmentMode::Active { index, offset } = &data.mode {
                let off = address(eval_const(s, &inst, offset)?)?;
                let mem = &mut s.mems[inst.mems[*index as usize].0.index];
                let r = range(off, bytes.len(), mem.len()?)?;
                let _ = r;
                mem.write(off, &bytes)?;
                inst.data[data_index] = None
            } else {
                inst.data[data_index] = Some(bytes)
            }
        }
        for e in &d.exports {
            let x = match e.kind {
                ExternalKind::Tag => External::Tag(inst.tags[e.index as usize]),
                ExternalKind::Func => External::Func(inst.funcs[e.index as usize]),
                ExternalKind::Memory => External::Memory(inst.mems[e.index as usize]),
                ExternalKind::Table => External::Table(inst.tabs[e.index as usize]),
                ExternalKind::Global => External::Global(inst.globals[e.index as usize]),
            };
            inst.exports.insert(e.name.clone(), x);
        }
        let start = d.start.map(|x| inst.funcs[x.as_usize()]);
        s.instances[ii] = inst;
        let out = Instance(s.h(ii));
        if let Some(f) = start {
            match s.invoke(f, &[])? {
                RunOutcome::Returned(_) => {}
                RunOutcome::Exception(_) => {
                    return Err(trap("uncaught exception in start function"));
                }
                RunOutcome::Suspended { .. } => {
                    return Err(err(ErrorKind::Host, "start function suspended"));
                }
            }
        }
        Ok(out)
    }
}

impl Linker {
    pub fn instantiate(&self, s: &mut Store, m: &Module) -> Result<Instance> {
        let checkpoint = (
            s.funcs.len(),
            s.mems.len(),
            s.tabs.len(),
            s.globals.len(),
            s.instances.len(),
            s.tags.len(),
            s.exceptions.len(),
        );
        let result = self.instantiate_inner(s, m);
        if result.is_err() {
            // References published by the start function keep their instance alive.
            let mut pending: Vec<Value> = (0..checkpoint.2)
                .filter_map(|i| s.tabs.get(i))
                .flat_map(|t| t.values.iter())
                .chain(
                    (0..checkpoint.3)
                        .filter_map(|i| s.globals.get(i))
                        .map(|g| &g.value),
                )
                .chain(s.published.iter())
                .copied()
                .collect();
            let mut escaped = false;
            if let Ok(roots) = s.gc.registered_roots() {
                pending.extend(roots.into_iter().map(|r| Value::Ref(RefValue::Gc(r))));
            } else {
                escaped = true;
            }
            if let Ok(roots) = s.value_roots.lock() {
                pending.extend(roots.iter().filter_map(|r| r.upgrade()).map(|r| *r));
            } else {
                escaped = true;
            }
            let mut gc_seen = std::collections::HashSet::new();
            let mut exn_seen = std::collections::HashSet::new();
            let mut work = 0usize;
            while let Some(value) = pending.pop() {
                work += 1;
                if work > s.engine.config.limits.max_gc_mark_work {
                    escaped = true;
                    break;
                }
                match value {
                    Value::Ref(RefValue::Func(f))
                        if f.0.store == s.id && f.0.index >= checkpoint.0 =>
                    {
                        escaped = true;
                        break;
                    }
                    Value::Ref(RefValue::Gc(r) | RefValue::ExternGc(r)) if gc_seen.insert(r) => {
                        if let Ok(object) = s.gc.get(r) {
                            pending.extend_from_slice(&object.fields);
                            if let Some(r) = object.descriptor {
                                pending.push(Value::Ref(RefValue::Gc(r)));
                            }
                        }
                    }
                    Value::Ref(RefValue::Exn(r)) if exn_seen.insert(r) => {
                        if let Ok(i) = s.check(r.0, &s.exceptions) {
                            pending.extend_from_slice(&s.exceptions[i].payload);
                        }
                    }
                    _ => {}
                }
            }
            if !escaped {
                s.funcs.retire_from(checkpoint.0);
                s.mems.retire_from(checkpoint.1);
                s.tabs.retire_from(checkpoint.2);
                s.globals.retire_from(checkpoint.3);
                s.instances.retire_from(checkpoint.4);
                s.tags.retire_from(checkpoint.5);
                s.tag_instances.retain(|k, _| *k < checkpoint.5);
                s.tag_modules.retain(|k, _| *k < checkpoint.5);
                s.tag_canonical.retain(|k, _| *k < checkpoint.5);
                s.exceptions.retire_from(checkpoint.6);
            }
        }
        result
    }
}

fn import_subtype(
    store: &Store,
    actual: ValType,
    owner: Option<&Module>,
    expected: ValType,
    target: &Module,
) -> Result<bool> {
    if actual == expected
        && !matches!(
            actual,
            ValType::Ref(RefType {
                heap: HeapType::Defined(_) | HeapType::ExactDefined(_),
                ..
            })
        )
    {
        return Ok(true);
    }
    let (ValType::Ref(a), ValType::Ref(b)) = (actual, expected) else {
        return Ok(false);
    };
    if a.nullable && !b.nullable {
        return Ok(false);
    }
    let actual_index = match a.heap {
        HeapType::Defined(i) | HeapType::ExactDefined(i) => Some(i.as_usize()),
        _ => None,
    };
    if actual_index.is_some() && owner.is_none() {
        return Ok(false);
    }
    let expected_index = match b.heap {
        HeapType::Defined(i) | HeapType::ExactDefined(i) => Some(i.as_usize()),
        _ => None,
    };
    if let (Some(ai), Some(bi), Some(source)) = (actual_index, expected_index, owner) {
        return store
            .engine
            .canonical
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
            .is_subtype(
                source.data.canonical[ai],
                target.data.canonical[bi],
                matches!(b.heap, HeapType::ExactDefined(_)),
                store.engine.config.limits.max_type_work,
            );
    }
    let heap = if let (Some(ai), Some(source)) = (actual_index, owner) {
        match &source
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(ai)
            .ok_or_else(|| err(ErrorKind::Link, "import type index"))?
            .composite
        {
            CompositeType::Func(_) => HeapType::Func,
            CompositeType::Struct(_) => HeapType::Struct,
            CompositeType::Array(_) => HeapType::Array,
            CompositeType::Cont(_) => HeapType::Cont,
        }
    } else {
        a.heap
    };
    Ok(heap == b.heap
        || matches!(
            (heap, b.heap),
            (
                HeapType::Struct | HeapType::Array | HeapType::I31,
                HeapType::Eq | HeapType::Any
            ) | (HeapType::Eq, HeapType::Any)
                | (
                    HeapType::None,
                    HeapType::Any
                        | HeapType::Eq
                        | HeapType::Struct
                        | HeapType::Array
                        | HeapType::I31
                )
                | (HeapType::NoFunc, HeapType::Func)
                | (HeapType::NoExtern, HeapType::Extern)
                | (HeapType::NoExn, HeapType::Exn)
                | (HeapType::NoCont, HeapType::Cont)
        ))
}
#[cfg(test)]
mod rollback_escape_tests {
    use super::*;
    #[test]
    fn t043_failed_start_preserves_gc_and_exception_wrapped_function_escape() {
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 2, 7, 1, 1, b'h', 1, b'f', 0, 0, 3, 2,
            1, 0, 8, 1, 1, 10, 6, 1, 4, 0, 16, 0, 11,
        ];
        for exception in [false, true] {
            let e = Engine::new(Config::default()).unwrap();
            let module = e.compile(&bytes).unwrap();
            let mut s = Store::new(&e);
            let saved = Arc::new(std::sync::Mutex::new(None));
            let target = saved.clone();
            let host = s
                .create_host_func_with_caller(FuncType::default(), move |caller, _| {
                    let guest = Func(caller.store.h(caller.store.funcs.len() - 1));
                    let value = Value::Ref(RefValue::Func(guest));
                    let wrapper = if exception {
                        let tag = caller
                            .store
                            .create_tag(FuncType::new(vec![ValType::FUNCREF], vec![]))?;
                        Value::Ref(RefValue::Exn(caller.store.create_exception(tag, &[value])?))
                    } else {
                        let ty = caller.store.funcs[guest.0.index].canonical;
                        let r = caller.store.gc.allocate(
                            gc::Object {
                                ty,
                                kind: gc::ObjectKind::Struct,
                                fields: vec![value],
                                descriptor: None,
                            },
                            10,
                        )?;
                        let wrapped = Value::Ref(RefValue::Gc(r));
                        caller.store.publish_values(&[wrapped]);
                        wrapped
                    };
                    *target.lock().unwrap() = Some(wrapper);
                    Err(err(ErrorKind::Host, "start abort after escape"))
                })
                .unwrap();
            let mut linker = Linker::new(&e);
            linker.define("h", "f", External::Func(host)).unwrap();
            assert!(linker.instantiate(&mut s, &module).is_err());
            s.collect().unwrap();
            let value = saved.lock().unwrap().unwrap();
            let guest = match value {
                Value::Ref(RefValue::Gc(r)) => s.gc.get(r).unwrap().fields[0],
                Value::Ref(RefValue::Exn(r)) => s.exception_payload(r).unwrap()[0],
                _ => panic!(),
            };
            s.validate_value(guest).unwrap();
            assert_eq!(s.instances.live_len(), 1);
        }
    }
}
#[cfg(test)]
mod exact_host_import_tests {
    use super::*;
    #[test]
    fn t152_exact_import_rejects_host_proper_subtype() {
        // Type0=(sub (func)); type1=(sub 0 (func)); same shape, distinct nominal subtype.
        let types = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 12, 2, 0x50, 0, 0x60, 0, 0, 0x4f, 1, 0, 0x60, 0, 0,
        ];
        let e = Engine::new(Config::default()).unwrap();
        let context = e.compile(&types).unwrap();
        let mut s = Store::new(&e);
        let subtype = s
            .create_module_host_func(&context, TypeIdx(1), |_, _| {
                Ok(RunOutcome::Returned(vec![]))
            })
            .unwrap();
        let exact = s
            .create_module_host_func(&context, TypeIdx(0), |_, _| {
                Ok(RunOutcome::Returned(vec![]))
            })
            .unwrap();
        for (kind, function, success) in [
            (0, subtype, true),
            (0x20, subtype, false),
            (0x20, exact, true),
        ] {
            let mut bytes = types.to_vec();
            bytes.extend([2, 7, 1, 1, b'h', 1, b'f', kind, 0]);
            let module = e.compile(&bytes).unwrap();
            let mut linker = Linker::new(&e);
            linker.define("h", "f", External::Func(function)).unwrap();
            let result = linker.instantiate(&mut s, &module);
            if success {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Link);
            }
        }
    }
}
