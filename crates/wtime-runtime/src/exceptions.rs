//! Store-owned exception identities; guest throws never use native unwinding.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tag(pub(super) Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExceptionRef(pub(super) Handle);
#[derive(Clone, Debug)]
pub(super) struct Exception {
    pub(super) tag: Tag,
    pub(super) payload: Vec<Value>,
}
impl Store {
    pub fn create_tag(&mut self, ty: FuncType) -> Result<Tag> {
        if ty.params.iter().chain(&ty.results).any(|t| {
            matches!(
                t,
                ValType::Ref(RefType {
                    heap: HeapType::Defined(_) | HeapType::ExactDefined(_),
                    ..
                })
            )
        }) {
            return Err(err(
                ErrorKind::Host,
                "concrete tag references require create_module_tag",
            ));
        }
        if !self.engine.config.features.exception_handling
            && !self.engine.config.features.stack_switching
        {
            return Err(err(ErrorKind::DisabledFeature, "host tag feature disabled"));
        }
        self.reserve_objects(1)?;
        if !ty.results.is_empty() && !self.engine.config.features.stack_switching {
            return Err(err(ErrorKind::Host, "tag type must have no results"));
        }
        let tag = Tag(self.h(self.tags.len()));
        self.tags.push(ty);
        Ok(tag)
    }
    pub fn create_module_tag(&mut self, module: &Module, ty: TypeIdx) -> Result<Tag> {
        if module.data.engine != self.engine.id {
            return Err(err(ErrorKind::Host, "tag module engine mismatch"));
        }
        if !self.engine.config.features.exception_handling
            && !self.engine.config.features.stack_switching
        {
            return Err(err(ErrorKind::DisabledFeature, "host tag feature disabled"));
        }
        let definition = module
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(ty.as_usize())
            .ok_or_else(|| err(ErrorKind::Host, "tag type index"))?;
        let CompositeType::Func(signature) = &definition.composite else {
            return Err(err(ErrorKind::Host, "tag requires function type"));
        };
        if !signature.results.is_empty() && !self.engine.config.features.stack_switching {
            return Err(err(ErrorKind::Host, "tag results disabled"));
        }
        self.reserve_objects(1)?;
        let tag = Tag(self.h(self.tags.len()));
        self.tags.push(signature.clone());
        self.tag_canonical
            .insert(tag.0.index, module.data.canonical[ty.as_usize()]);
        self.tag_modules.insert(tag.0.index, module.clone());
        Ok(tag)
    }
    pub fn create_exception(&mut self, tag: Tag, payload: &[Value]) -> Result<ExceptionRef> {
        let ex = self.allocate_exception(tag, payload)?;
        self.publish_values(&[Value::Ref(RefValue::Exn(ex))]);
        Ok(ex)
    }
    pub(super) fn allocate_exception(
        &mut self,
        tag: Tag,
        payload: &[Value],
    ) -> Result<ExceptionRef> {
        self.reserve_objects(1)?;
        for value in payload {
            self.validate_value(*value)?;
        }
        let ix = self.check(tag.0, &self.tags)?;
        let ty = &self.tags[ix];
        if !ty.results.is_empty() {
            return Err(err(ErrorKind::Host, "exception tag must have no results"));
        }
        if payload.len() != ty.params.len()
            || !payload.iter().zip(&ty.params).all(|(v, t)| match t {
                ValType::Ref(r) => {
                    if let Some(module) = self.tag_modules.get(&ix) {
                        return self.gc_matches_module(module, *v, *r).unwrap_or(false);
                    }
                    let owner = self.tag_instances.get(&ix).copied();
                    if owner.is_some()
                        || !matches!(r.heap, HeapType::Defined(_) | HeapType::ExactDefined(_))
                    {
                        matches_type(*v, *t)
                            || self.gc_matches(owner.unwrap_or(0), *v, *r).unwrap_or(false)
                    } else {
                        matches_type(*v, *t)
                    }
                }
                _ => matches_type(*v, *t),
            })
        {
            return Err(err(ErrorKind::Host, "exception payload type mismatch"));
        }
        let (index, generation) = self.exceptions.insert(Exception {
            tag,
            payload: payload.to_vec(),
        });
        let ex = ExceptionRef(Handle {
            store: self.id,
            index,
            generation,
        });
        Ok(ex)
    }
    pub fn exception_tag(&self, ex: ExceptionRef) -> Result<Tag> {
        Ok(self.exceptions[self.check(ex.0, &self.exceptions)?].tag)
    }
    pub fn exception_payload(&self, ex: ExceptionRef) -> Result<&[Value]> {
        Ok(&self.exceptions[self.check(ex.0, &self.exceptions)?].payload)
    }
}

impl Store {
    /// Jointly trace GC objects and exception payloads; unreachable cycles mark neither side.
    pub(super) fn trace_exceptions(&mut self, roots: &[Value]) -> Result<Vec<GcRef>> {
        let mut pending = roots.to_vec();
        pending.extend(
            self.gc
                .registered_roots()?
                .into_iter()
                .map(|r| Value::Ref(RefValue::Gc(r))),
        );
        let mut gc_seen = std::collections::HashSet::new();
        let mut ex_seen = std::collections::HashSet::new();
        while let Some(value) = pending.pop() {
            match value {
                Value::Ref(RefValue::Exn(ex)) => {
                    let index = self.check(ex.0, &self.exceptions)?;
                    if ex_seen.insert(index) {
                        pending.extend_from_slice(&self.exceptions[index].payload);
                    }
                }
                Value::Ref(RefValue::Gc(r)) if gc_seen.insert(r) => {
                    let object = self.gc.get(r)?;
                    pending.extend_from_slice(&object.fields);
                    if let Some(r) = object.descriptor {
                        pending.push(Value::Ref(RefValue::Gc(r)));
                    }
                }
                _ => {}
            }
        }
        for index in 0..self.exceptions.len() {
            if !ex_seen.contains(&index)
                && let Some(generation) = self.exceptions.generation(index)
            {
                self.exceptions.remove(index, generation);
            }
        }
        Ok(gc_seen.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (Store, Tag, GcRef) {
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let tag = s
            .create_tag(FuncType {
                params: vec![ValType::Ref(RefType::new(true, HeapType::Any))],
                results: vec![],
            })
            .unwrap();
        let r =
            s.gc.allocate(
                gc::Object {
                    ty: CanonicalTypeId::test(e.id, 0),
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        (s, tag, r)
    }
    #[test]
    fn t124_unreachable_exception_payload_cycles_reclaimed() {
        let (mut s, tag, r) = setup();
        let ex = s
            .allocate_exception(tag, &[Value::Ref(RefValue::Gc(r))])
            .unwrap();
        s.gc.get_mut(r)
            .unwrap()
            .fields
            .push(Value::Ref(RefValue::Exn(ex)));
        assert_eq!(s.collect().unwrap().reclaimed, 1);
        assert!(s.exception_tag(ex).is_err());
        let replacement = s
            .allocate_exception(tag, &[Value::Ref(RefValue::Null(HeapType::Any))])
            .unwrap();
        assert_eq!(replacement.0.index, ex.0.index);
        assert_ne!(replacement, ex);
        assert!(s.exception_tag(ex).is_err());
    }
    #[test]
    fn t124_rooted_gc_reaches_exception_payload() {
        let (mut s, tag, r) = setup();
        let ex = s
            .allocate_exception(tag, &[Value::Ref(RefValue::Gc(r))])
            .unwrap();
        s.gc.get_mut(r)
            .unwrap()
            .fields
            .push(Value::Ref(RefValue::Exn(ex)));
        let root = s.root(r).unwrap();
        assert_eq!(s.collect().unwrap().live, 1);
        assert!(s.exception_tag(ex).is_ok());
        drop(root);
        assert_eq!(s.collect().unwrap().reclaimed, 1);
        assert!(s.exception_tag(ex).is_err());
    }
    #[test]
    fn t124_host_published_exception_preserves_payload() {
        let (mut s, tag, r) = setup();
        let ex = s
            .create_exception(tag, &[Value::Ref(RefValue::Gc(r))])
            .unwrap();
        assert_eq!(s.collect().unwrap().live, 1);
        assert!(s.exception_tag(ex).is_ok());
        s.release_published(&[Value::Ref(RefValue::Exn(ex))]);
        assert_eq!(s.collect().unwrap().reclaimed, 1);
        assert!(s.exception_tag(ex).is_err());
    }
}
