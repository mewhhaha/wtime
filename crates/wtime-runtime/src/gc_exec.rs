//! GC instructions operate on arena handles, never Rust pointer identity.
use super::*;
use crate::gc::{Object, ObjectKind};
fn index(i: &Instruction) -> Result<usize> {
    match i.immediate {
        Immediate::Index(n) => Ok(n as usize),
        Immediate::Pair(n, _) => Ok(n as usize),
        _ => Err(trap("missing GC type index")),
    }
}
fn pair(i: &Instruction) -> Result<(usize, usize)> {
    match i.immediate {
        Immediate::Pair(a, b) => Ok((a as usize, b as usize)),
        _ => Err(trap("missing GC immediate pair")),
    }
}
fn integer(s: &mut Vec<Value>) -> Result<usize> {
    match pop(s)? {
        Value::I32(n) => Ok(n as u32 as usize),
        _ => Err(trap("expected i32")),
    }
}
fn reference(s: &mut Vec<Value>) -> Result<GcRef> {
    match pop(s)? {
        Value::Ref(RefValue::Gc(r)) => Ok(r),
        Value::Ref(RefValue::Null(_)) => Err(trap("null reference")),
        _ => Err(trap("expected GC reference")),
    }
}
fn packed(v: Value, storage: StorageType) -> Value {
    match (v, storage) {
        (Value::I32(x), StorageType::I8) => Value::I32(x & 255),
        (Value::I32(x), StorageType::I16) => Value::I32(x & 65535),
        _ => v,
    }
}
fn unpack(v: Value, storage: StorageType, signed: bool) -> Value {
    match (v, storage, signed) {
        (Value::I32(x), StorageType::I8, true) => Value::I32(x as i8 as i32),
        (Value::I32(x), StorageType::I16, true) => Value::I32(x as i16 as i32),
        _ => v,
    }
}
impl Store {
    /// Tests a reflected abstract heap type without exposing guest heap handles.
    pub fn reference_matches(&self, value: Value, heap: HeapType) -> Result<bool> {
        self.validate_value(value)?;
        if matches!(heap, HeapType::Defined(_) | HeapType::ExactDefined(_)) {
            return Err(err(
                ErrorKind::Host,
                "defined reflection requires a module type context",
            ));
        }
        self.gc_matches(0, value, RefType::new(false, heap))
    }
    /// Returns the host-assigned external identity for an `externref` (including one internalized via `any.convert_extern`), or `None` for other values.
    pub fn extern_identity(&self, value: Value) -> Result<Option<u32>> {
        self.validate_value(value)?;
        if let Value::Ref(RefValue::Extern(r) | RefValue::InternalExtern(r)) = value {
            Ok(self
                .externs
                .iter()
                .find_map(|(id, handle)| (*handle == r).then_some(*id)))
        } else {
            Ok(None)
        }
    }
    fn gc_type(&self, ii: usize, n: usize) -> Result<SubType> {
        self.instances[ii]
            .module
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(n)
            .cloned()
            .ok_or_else(|| trap("GC type out of bounds"))
    }
    fn gc_field(&self, ii: usize, n: usize) -> Result<FieldType> {
        match self.gc_type(ii, n)?.composite {
            CompositeType::Array(a) => Ok(a.field),
            _ => Err(trap("expected array type")),
        }
    }
    fn gc_alloc(
        &mut self,
        ii: usize,
        n: usize,
        kind: ObjectKind,
        fields: Vec<Value>,
    ) -> Result<Value> {
        let ty = *self.instances[ii]
            .module
            .data
            .canonical
            .get(n)
            .ok_or_else(|| trap("missing canonical type"))?;
        let limit = self.engine.config.limits.max_table_elements as usize;
        self.reserve_objects(1)?;
        let r = self.gc.allocate(
            Object {
                ty,
                kind,
                fields,
                descriptor: None,
            },
            limit,
        )?;
        Ok(Value::Ref(RefValue::Gc(r)))
    }
    pub(super) fn gc_matches(&self, ii: usize, v: Value, t: RefType) -> Result<bool> {
        self.gc_matches_in_context(
            self.instances.get(ii).map(|instance| &instance.module),
            v,
            t,
        )
    }
    pub(super) fn gc_matches_module(&self, module: &Module, v: Value, t: RefType) -> Result<bool> {
        self.gc_matches_in_context(Some(module), v, t)
    }
    fn gc_matches_in_context(&self, module: Option<&Module>, v: Value, t: RefType) -> Result<bool> {
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::Subtype, 1);
        if let Value::Ref(RefValue::Null(source)) = v {
            let family = |heap| -> Option<u8> {
                Some(match heap {
                    HeapType::Any
                    | HeapType::Eq
                    | HeapType::I31
                    | HeapType::Struct
                    | HeapType::Array
                    | HeapType::None => 0,
                    HeapType::Func | HeapType::NoFunc => 1,
                    HeapType::Extern | HeapType::NoExtern => 2,
                    HeapType::Exn | HeapType::NoExn => 3,
                    HeapType::Cont | HeapType::NoCont => 4,
                    HeapType::Defined(i) | HeapType::ExactDefined(i) => {
                        match &module?
                            .data
                            .inner
                            .decoded()
                            .type_groups
                            .iter()
                            .flat_map(|g| &g.types)
                            .nth(i.as_usize())?
                            .composite
                        {
                            CompositeType::Struct(_) | CompositeType::Array(_) => 0,
                            CompositeType::Func(_) => 1,
                            CompositeType::Cont(_) => 4,
                        }
                    }
                })
            };
            return Ok(t.nullable && family(source).is_some() && family(source) == family(t.heap));
        }
        Ok(match t.heap {
            HeapType::None
            | HeapType::NoFunc
            | HeapType::NoExtern
            | HeapType::NoExn
            | HeapType::NoCont => false,
            HeapType::Any => match v {
                Value::Ref(RefValue::Gc(r)) => {
                    matches!(self.gc.get(r)?.kind, ObjectKind::Struct | ObjectKind::Array)
                }
                Value::Ref(RefValue::I31(_) | RefValue::InternalExtern(_)) => true,
                _ => false,
            },
            HeapType::Eq => match v {
                Value::Ref(RefValue::I31(_)) => true,
                Value::Ref(RefValue::Gc(r)) => {
                    matches!(self.gc.get(r)?.kind, ObjectKind::Struct | ObjectKind::Array)
                }
                _ => false,
            },
            HeapType::I31 => matches!(v, Value::Ref(RefValue::I31(_))),
            HeapType::Exn => {
                if let Value::Ref(RefValue::Exn(ex)) = v {
                    self.exception_tag(ex)?;
                    true
                } else {
                    false
                }
            }
            HeapType::Struct | HeapType::Array | HeapType::Cont => {
                if let Value::Ref(RefValue::Gc(r)) = v {
                    matches!(
                        (&t.heap, &self.gc.get(r)?.kind),
                        (HeapType::Struct, ObjectKind::Struct)
                            | (HeapType::Array, ObjectKind::Array)
                            | (HeapType::Cont, ObjectKind::Continuation)
                    )
                } else {
                    false
                }
            }
            HeapType::Func => matches!(v, Value::Ref(RefValue::Func(_))),
            HeapType::Extern => matches!(
                v,
                Value::Ref(RefValue::Extern(_) | RefValue::ExternI31(_) | RefValue::ExternGc(_))
            ),
            HeapType::Defined(n) | HeapType::ExactDefined(n) => {
                let target = *module
                    .ok_or_else(|| {
                        err(
                            ErrorKind::Host,
                            "defined reference requires module type context",
                        )
                    })?
                    .data
                    .canonical
                    .get(n.as_usize())
                    .ok_or_else(|| trap("cast type index"))?;
                let actual = match v {
                    Value::Ref(RefValue::Gc(r)) => Some(self.gc.get(r)?.ty),
                    Value::Ref(RefValue::Func(f)) => {
                        Some(self.funcs[self.check(f.0, &self.funcs)?].canonical)
                    }
                    _ => None,
                };
                if let Some(actual) = actual {
                    self.engine
                        .canonical
                        .lock()
                        .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
                        .is_subtype(
                            actual,
                            target,
                            matches!(t.heap, HeapType::ExactDefined(_)),
                            self.engine.config.limits.max_type_work,
                        )?
                } else {
                    false
                }
            }
        })
    }
    pub(super) fn gc_descriptor_matches(
        &self,
        ii: usize,
        value: Value,
        descriptor: GcRef,
        ty: RefType,
    ) -> Result<bool> {
        self.gc.get(descriptor)?;
        if matches!(value, Value::Ref(RefValue::Null(_))) {
            return self.gc_matches(ii, value, ty);
        }
        Ok(self.gc_matches(ii, value, ty)?
            && match value {
                Value::Ref(RefValue::Gc(r)) => self.gc.get(r)?.descriptor == Some(descriptor),
                _ => false,
            })
    }
    pub(super) fn gc_exec(&mut self, ii: usize, i: &Instruction, s: &mut Vec<Value>) -> Result<()> {
        let op = i.opcode - 0xfb00;
        match op {
            0 | 1 => {
                let n = index(i)?;
                let CompositeType::Struct(t) = self.gc_type(ii, n)?.composite else {
                    return Err(trap("expected struct type"));
                };
                self.gc.check_payload(t.fields.len())?;
                let mut fields = Vec::new();
                fields
                    .try_reserve_exact(t.fields.len())
                    .map_err(|_| err(ErrorKind::ResourceLimit, "struct allocation failed"))?;
                if op == 0 {
                    for f in t.fields.iter().rev() {
                        fields.push(packed(pop(s)?, f.storage))
                    }
                    fields.reverse()
                } else {
                    for f in &t.fields {
                        fields.push(Value::zero(f.storage.unpacked()))
                    }
                }
                let value = self.gc_alloc(ii, n, ObjectKind::Struct, fields)?;
                s.push(value)
            }
            2..=5 => {
                let (n, f) = pair(i)?;
                let CompositeType::Struct(t) = self.gc_type(ii, n)?.composite else {
                    return Err(trap("expected struct type"));
                };
                let storage = t
                    .fields
                    .get(f)
                    .ok_or_else(|| trap("struct field out of bounds"))?
                    .storage;
                let value = if op == 5 {
                    Some(packed(pop(s)?, storage))
                } else {
                    None
                };
                let r = reference(s)?;
                if let Some(v) = value {
                    let object = self.gc.get_mut(r)?;
                    *object
                        .fields
                        .get_mut(f)
                        .ok_or_else(|| trap("struct field out of bounds"))? = v
                } else {
                    s.push(unpack(
                        *self
                            .gc
                            .get(r)?
                            .fields
                            .get(f)
                            .ok_or_else(|| trap("struct field out of bounds"))?,
                        storage,
                        op == 3,
                    ))
                }
            }
            6..=8 => {
                let n = index(i)?;
                let field = self.gc_field(ii, n)?;
                let len = if op == 8 { pair(i)?.1 } else { integer(s)? };
                if len > self.engine.config.limits.max_table_elements as usize {
                    return Err(err(ErrorKind::ResourceLimit, "array length budget"));
                }
                self.gc.check_payload(len)?;
                let fields = if op == 8 {
                    let start = s
                        .len()
                        .checked_sub(len)
                        .ok_or_else(|| trap("array operand underflow"))?;
                    s.drain(start..).map(|v| packed(v, field.storage)).collect()
                } else {
                    let value = if op == 6 {
                        packed(pop(s)?, field.storage)
                    } else {
                        Value::zero(field.storage.unpacked())
                    };
                    let mut v = Vec::new();
                    v.try_reserve_exact(len)
                        .map_err(|_| err(ErrorKind::ResourceLimit, "array allocation failed"))?;
                    v.resize(len, value);
                    v
                };
                let v = self.gc_alloc(ii, n, ObjectKind::Array, fields)?;
                s.push(v)
            }
            9 | 10 | 18 | 19 => {
                let (n, segment) = pair(i)?;
                let field = self.gc_field(ii, n)?;
                let len = integer(s)?;
                let from = integer(s)?;
                let destination = if op >= 18 {
                    let at = integer(s)?;
                    Some((reference(s)?, at))
                } else {
                    None
                };
                if let Some((r, at)) = destination {
                    range(at as u64, len, self.gc.get(r)?.fields.len())?;
                }
                let values = if op == 9 || op == 18 {
                    let bytes = self.instances[ii]
                        .data
                        .get(segment)
                        .ok_or_else(|| trap("data segment index"))?
                        .as_deref()
                        .unwrap_or(&[]);
                    let width = match field.storage {
                        StorageType::I8 => 1,
                        StorageType::I16 => 2,
                        StorageType::Val(ValType::I32 | ValType::F32) => 4,
                        StorageType::Val(ValType::I64 | ValType::F64) => 8,
                        StorageType::Val(ValType::V128) => 16,
                        _ => return Err(trap("invalid data array storage")),
                    };
                    let size = len
                        .checked_mul(width)
                        .ok_or_else(|| trap("data length overflow"))?;
                    let bytes = &bytes[range(from as u64, size, bytes.len())?];
                    if len > self.engine.config.limits.max_table_elements as usize {
                        return Err(err(ErrorKind::ResourceLimit, "array length budget"));
                    }
                    self.gc.check_payload(len)?;
                    let mut values = Vec::new();
                    values
                        .try_reserve_exact(len)
                        .map_err(|_| err(ErrorKind::ResourceLimit, "array allocation failed"))?;
                    for chunk in bytes.chunks_exact(width) {
                        let mut raw = [0u8; 16];
                        raw[..width].copy_from_slice(chunk);
                        let bits = u64::from_le_bytes(raw[..8].try_into().unwrap());
                        values.push(match field.storage {
                            StorageType::I8 | StorageType::I16 | StorageType::Val(ValType::I32) => {
                                Value::I32(bits as i32)
                            }
                            StorageType::Val(ValType::I64) => Value::I64(bits as i64),
                            StorageType::Val(ValType::F32) => Value::F32(bits as u32),
                            StorageType::Val(ValType::F64) => Value::F64(bits),
                            StorageType::Val(ValType::V128) => Value::V128(V128(raw)),
                            _ => unreachable!(),
                        })
                    }
                    values
                } else {
                    let values = self.instances[ii]
                        .elems
                        .get(segment)
                        .ok_or_else(|| trap("element segment index"))?
                        .as_deref()
                        .unwrap_or(&[]);
                    let source = &values[range(from as u64, len, values.len())
                        .map_err(|_| trap("out of bounds table access"))?];
                    if len > self.engine.config.limits.max_table_elements as usize {
                        return Err(err(ErrorKind::ResourceLimit, "array length budget"));
                    }
                    self.gc.check_payload(len)?;
                    source.to_vec()
                };
                if let Some((r, at)) = destination {
                    let object = self.gc.get_mut(r)?;
                    let target = range(at as u64, len, object.fields.len())?;
                    object.fields[target].copy_from_slice(&values)
                } else {
                    let value = self.gc_alloc(ii, n, ObjectKind::Array, values)?;
                    s.push(value)
                }
            }
            11..=14 => {
                let n = index(i)?;
                let field = self.gc_field(ii, n)?;
                let value = if op == 14 {
                    Some(packed(pop(s)?, field.storage))
                } else {
                    None
                };
                let at = integer(s)?;
                let r = reference(s)?;
                if let Some(v) = value {
                    *self
                        .gc
                        .get_mut(r)?
                        .fields
                        .get_mut(at)
                        .ok_or_else(|| trap("array out of bounds"))? = v
                } else {
                    s.push(unpack(
                        *self
                            .gc
                            .get(r)?
                            .fields
                            .get(at)
                            .ok_or_else(|| trap("array out of bounds"))?,
                        field.storage,
                        op == 12,
                    ))
                }
            }
            15 => {
                let r = reference(s)?;
                s.push(Value::I32(self.gc.get(r)?.fields.len() as i32))
            }
            16 => {
                let n = index(i)?;
                let field = self.gc_field(ii, n)?;
                let len = integer(s)?;
                let v = packed(pop(s)?, field.storage);
                let at = integer(s)?;
                let r = reference(s)?;
                let object = self.gc.get_mut(r)?;
                let range = range(at as u64, len, object.fields.len())?;
                object.fields[range].fill(v)
            }
            17 => {
                let len = integer(s)?;
                let from = integer(s)?;
                let src = reference(s)?;
                let to = integer(s)?;
                let dst = reference(s)?;
                let src_range = range(from as u64, len, self.gc.get(src)?.fields.len())?;
                let dst_range = range(to as u64, len, self.gc.get(dst)?.fields.len())?;
                if src == dst {
                    self.gc.get_mut(dst)?.fields.copy_within(src_range, to)
                } else {
                    let values = self.gc.get(src)?.fields[src_range].to_vec();
                    self.gc.get_mut(dst)?.fields[dst_range].copy_from_slice(&values)
                }
            }
            20..=23 => {
                let Immediate::Heap(heap) = i.immediate else {
                    return Err(trap("missing cast type"));
                };
                let v = pop(s)?;
                let yes = self.gc_matches(ii, v, RefType::new(op % 2 == 1, heap))?;
                if op <= 21 {
                    s.push(Value::I32(yes as i32))
                } else if yes {
                    s.push(v)
                } else {
                    return Err(trap("reference cast failed"));
                }
            }
            26 | 27 => {
                let v = pop(s)?;
                self.validate_value(v)?;
                let converted = match (op, v) {
                    (26, Value::Ref(RefValue::Null(_))) => RefValue::Null(HeapType::Any),
                    (27, Value::Ref(RefValue::Null(_))) => RefValue::Null(HeapType::Extern),
                    (26, Value::Ref(RefValue::Extern(r))) => RefValue::InternalExtern(r),
                    (26, Value::Ref(RefValue::ExternI31(n))) => RefValue::I31(n),
                    (26, Value::Ref(RefValue::ExternGc(r))) => RefValue::Gc(r),
                    (27, Value::Ref(RefValue::InternalExtern(r))) => RefValue::Extern(r),
                    (27, Value::Ref(RefValue::I31(n))) => RefValue::ExternI31(n),
                    (27, Value::Ref(RefValue::Gc(r))) => RefValue::ExternGc(r),
                    _ => return Err(trap("invalid extern conversion reference")),
                };
                s.push(Value::Ref(converted));
            }
            28 => {
                let x = integer(s)? as u32;
                s.push(Value::Ref(RefValue::I31(((x as i32) << 1) >> 1)))
            }
            29 | 30 => {
                let v = pop(s)?;
                match v {
                    Value::Ref(RefValue::I31(x)) => {
                        s.push(Value::I32(if op == 29 { x } else { x & 0x7fffffff }))
                    }
                    Value::Ref(RefValue::Null(_)) => return Err(trap("null i31 reference")),
                    _ => return Err(trap("expected i31")),
                }
            }
            32 | 33 => {
                let n = index(i)?;
                let t = self.gc_type(ii, n)?;
                let descriptor_type = t.descriptor.ok_or_else(|| trap("type has no descriptor"))?;
                let descriptor = reference(s)?;
                if !self.gc_matches(
                    ii,
                    Value::Ref(RefValue::Gc(descriptor)),
                    RefType::new(false, HeapType::ExactDefined(descriptor_type)),
                )? {
                    return Err(trap("descriptor type mismatch"));
                }
                let CompositeType::Struct(t) = t.composite else {
                    return Err(trap("expected struct type"));
                };
                self.gc.check_payload(t.fields.len())?;
                let mut fields = Vec::new();
                fields
                    .try_reserve_exact(t.fields.len())
                    .map_err(|_| err(ErrorKind::ResourceLimit, "struct allocation failed"))?;
                if op == 32 {
                    for f in t.fields.iter().rev() {
                        fields.push(packed(pop(s)?, f.storage))
                    }
                    fields.reverse()
                } else {
                    for f in &t.fields {
                        fields.push(Value::zero(f.storage.unpacked()))
                    }
                }
                let ty = self.instances[ii].module.data.canonical[n];
                self.reserve_objects(1)?;
                let r = self.gc.allocate(
                    Object {
                        ty,
                        kind: ObjectKind::Struct,
                        fields,
                        descriptor: Some(descriptor),
                    },
                    self.engine.config.limits.max_table_elements as usize,
                )?;
                s.push(Value::Ref(RefValue::Gc(r)));
            }
            34 => {
                let r = reference(s)?;
                let desc = self
                    .gc
                    .get(r)?
                    .descriptor
                    .ok_or_else(|| trap("object has no descriptor"))?;
                s.push(Value::Ref(RefValue::Gc(desc)))
            }
            35 | 36 => {
                let descriptor = reference(s)?;
                let value = pop(s)?;
                let Immediate::Heap(heap) = i.immediate else {
                    return Err(trap("missing descriptor cast type"));
                };
                if !self.gc_descriptor_matches(
                    ii,
                    value,
                    descriptor,
                    RefType::new(op == 36, heap),
                )? {
                    return Err(trap("descriptor cast failed"));
                }
                s.push(value)
            }
            _ => return Err(trap("unsupported GC instruction")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        let e = Engine::new(Config::default()).unwrap();
        let bytes = b"\0asm\x01\0\0\0\x01\x08\x02\x5f\x01\x78\x01\x5e\x77\x01";
        let m = e.compile(bytes).unwrap();
        let mut s = Store::new(&e);
        Linker::new(&e).instantiate(&mut s, &m).unwrap();
        s
    }
    fn op(store: &mut Store, opcode: u16, immediate: Immediate, s: &mut Vec<Value>) {
        store
            .gc_exec(0, &Instruction { opcode, immediate }, s)
            .unwrap()
    }
    #[test]
    fn t115_struct_packed_reads_and_rooted_collection() {
        let mut store = store();
        let mut s = vec![Value::I32(255)];
        op(&mut store, 0xfb00, Immediate::Index(0), &mut s);
        let r = s[0];
        op(&mut store, 0xfb03, Immediate::Pair(0, 0), &mut s);
        assert_eq!(s, vec![Value::I32(-1)]);
        let Value::Ref(RefValue::Gc(h)) = r else {
            panic!()
        };
        let token = store.root(h).unwrap();
        assert_eq!(store.collect().unwrap().live, 1);
        drop(token);
        assert_eq!(store.collect().unwrap().live, 0);
    }
    #[test]
    fn t116_array_overlap_and_bounds() {
        let mut store = store();
        let mut s = vec![Value::I32(1), Value::I32(2), Value::I32(3)];
        op(&mut store, 0xfb08, Immediate::Pair(1, 3), &mut s);
        let a = s[0];
        s = vec![a, Value::I32(1), a, Value::I32(0), Value::I32(2)];
        op(&mut store, 0xfb11, Immediate::Pair(1, 1), &mut s);
        s = vec![a, Value::I32(2)];
        op(&mut store, 0xfb0d, Immediate::Index(1), &mut s);
        assert_eq!(s, vec![Value::I32(2)]);
        s = vec![a, Value::I32(3)];
        assert!(
            store
                .gc_exec(
                    0,
                    &Instruction {
                        opcode: 0xfb0d,
                        immediate: Immediate::Index(1),
                    },
                    &mut s
                )
                .is_err()
        );
    }
    #[test]
    fn t119_i31_sign_and_unsigned_views() {
        let mut store = store();
        let mut s = vec![Value::I32(i32::MAX)];
        op(&mut store, 0xfb1c, Immediate::None, &mut s);
        let v = s[0];
        op(&mut store, 0xfb1d, Immediate::None, &mut s);
        assert_eq!(s, vec![Value::I32(-1)]);
        s = vec![v];
        op(&mut store, 0xfb1e, Immediate::None, &mut s);
        assert_eq!(s, vec![Value::I32(i32::MAX)]);
    }
    #[test]
    fn t156_descriptor_identity_is_not_structural_equality() {
        let e = Engine::new(Config::default()).unwrap();
        let m = e
            .compile(b"\0asm\x01\0\0\0\x01\x0b\x01\x4e\x02\x4d\x01\x5f\x00\x4c\x00\x5f\x00")
            .unwrap();
        let mut store = Store::new(&e);
        Linker::new(&e).instantiate(&mut store, &m).unwrap();
        let mut s = vec![];
        op(&mut store, 0xfb01, Immediate::Index(1), &mut s);
        let d1 = s[0];
        op(&mut store, 0xfb20, Immediate::Index(0), &mut s);
        let object = s[0];
        op(&mut store, 0xfb22, Immediate::Index(0), &mut s);
        assert_eq!(s[0], d1);
        s.clear();
        op(&mut store, 0xfb01, Immediate::Index(1), &mut s);
        let d2 = s[0];
        assert_ne!(d1, d2);
        let Value::Ref(RefValue::Gc(d1)) = d1 else {
            panic!()
        };
        let Value::Ref(RefValue::Gc(d2)) = d2 else {
            panic!()
        };
        let target = RefType::new(false, HeapType::Defined(TypeIdx(0)));
        assert!(store.gc_descriptor_matches(0, object, d1, target).unwrap());
        assert!(!store.gc_descriptor_matches(0, object, d2, target).unwrap());
    }
    #[test]
    fn t119_internalized_host_reference_is_any_but_not_eq() {
        let mut s = store();
        let host = s.create_extern_ref(37).unwrap();
        let mut values = vec![host];
        op(&mut s, 0xfb1a, Immediate::None, &mut values);
        let v = values[0];
        assert!(s.reference_matches(v, HeapType::Any).unwrap());
        assert!(!s.reference_matches(v, HeapType::Eq).unwrap());
        assert_eq!(s.extern_identity(v).unwrap(), Some(37));
        op(&mut s, 0xfb1b, Immediate::None, &mut values);
        assert_eq!(values[0], host);
    }
    #[test]
    fn t113_each_store_provider_is_a_sole_root() {
        for provider in 0..4 {
            let mut store = store();
            let mut values = vec![Value::I32(1)];
            op(&mut store, 0xfb00, Immediate::Index(0), &mut values);
            let value = values[0];
            let Value::Ref(RefValue::Gc(r)) = value else {
                panic!()
            };
            let mut global = None;
            let mut table = None;
            match provider {
                0 => {
                    global = Some(
                        store
                            .create_global(
                                GlobalType {
                                    value: ValType::Ref(RefType::new(true, HeapType::Any)),
                                    mutable: true,
                                },
                                value,
                            )
                            .unwrap(),
                    );
                }
                1 => {
                    let t = store
                        .create_table(TableType {
                            element: RefType::new(true, HeapType::Any),
                            limits: Limits::new(1, Some(1)),
                            index_type: IndexType::I32,
                        })
                        .unwrap();
                    store.write_table(t, 0, value).unwrap();
                    table = Some(t);
                }
                2 => store.active_roots.push(vec![value]),
                _ => store.published.push(value),
            }
            assert_eq!(store.collect().unwrap().live, 1);
            assert!(store.gc.get(r).is_ok());
            match provider {
                0 => store
                    .set_global(global.unwrap(), Value::Ref(RefValue::Null(HeapType::Any)))
                    .unwrap(),
                1 => store
                    .write_table(table.unwrap(), 0, Value::Ref(RefValue::Null(HeapType::Any)))
                    .unwrap(),
                2 => {
                    store.active_roots.pop();
                }
                _ => store.release_published(&[value]),
            }
            assert_eq!(store.collect().unwrap().live, 0);
        }
    }
    #[test]
    fn t113_nested_host_reentry_retains_arguments_and_pending_results() {
        let mut store = store();
        let mut values = vec![Value::I32(1)];
        op(&mut store, 0xfb00, Immediate::Index(0), &mut values);
        let value = values[0];
        let inner = store
            .create_host_func_with_caller(
                FuncType::new(vec![value.ty()], vec![value.ty()]),
                |caller, args| {
                    assert_eq!(caller.collect()?.live, 1);
                    Ok(args.to_vec())
                },
            )
            .unwrap();
        let outer = store
            .create_host_func_with_caller(
                FuncType::new(vec![value.ty()], vec![value.ty()]),
                move |caller, args| {
                    let RunOutcome::Returned(result) = caller.invoke(inner, args)? else {
                        panic!()
                    };
                    assert_eq!(caller.collect()?.live, 1);
                    caller.store().release_published(&result);
                    Ok(result)
                },
            )
            .unwrap();
        let RunOutcome::Returned(result) = store.invoke(outer, &[value]).unwrap() else {
            panic!()
        };
        assert_eq!(store.collect().unwrap().live, 1);
        store.release_published(&result);
        assert_eq!(store.collect().unwrap().live, 0);
    }
    #[test]
    fn t109_host_null_references_preserve_family_boundaries() {
        let engine = Engine::new(Config::default()).unwrap();
        let mut store = Store::new(&engine);
        let expected = [
            HeapType::Any,
            HeapType::Func,
            HeapType::Extern,
            HeapType::Exn,
            HeapType::Cont,
        ];
        let nulls = [
            HeapType::None,
            HeapType::NoFunc,
            HeapType::NoExtern,
            HeapType::NoExn,
            HeapType::NoCont,
        ];
        for (family, heap) in expected.into_iter().enumerate() {
            let function = store
                .create_host_func_with_caller(
                    FuncType::new(vec![ValType::Ref(RefType::new(true, heap))], vec![]),
                    |_, _| Ok(vec![]),
                )
                .unwrap();
            for (source_family, null) in nulls.into_iter().enumerate() {
                let result = store.invoke(function, &[Value::Ref(RefValue::Null(null))]);
                if family == source_family {
                    assert!(result.is_ok(), "family {family}: {result:?}");
                } else {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Host);
                }
            }
        }
    }
}
