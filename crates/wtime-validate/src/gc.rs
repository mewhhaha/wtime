use super::*;
fn defaultable(v: ValType) -> bool {
    !matches!(
        v,
        ValType::Ref(RefType {
            nullable: false,
            ..
        })
    )
}
impl Context<'_> {
    fn defined(&self, n: usize) -> Result<&SubType> {
        self.types
            .get(n)
            .ok_or_else(|| invalid("type index out of bounds"))
    }
    fn structure(&self, n: usize) -> Result<&StructType> {
        if let CompositeType::Struct(s) = &self.defined(n)?.composite {
            Ok(s)
        } else {
            Err(invalid("expected struct type"))
        }
    }
    fn array(&self, n: usize) -> Result<&ArrayType> {
        if let CompositeType::Array(a) = &self.defined(n)?.composite {
            Ok(a)
        } else {
            Err(invalid("expected array type"))
        }
    }
    fn new_reference(&self, n: usize) -> ValType {
        ValType::Ref(RefType::new(
            false,
            if self.features.custom_descriptors {
                HeapType::ExactDefined(TypeIdx(n as u32))
            } else {
                HeapType::Defined(TypeIdx(n as u32))
            },
        ))
    }
    fn reference(&self, n: usize, nullable: bool) -> ValType {
        ValType::Ref(RefType::new(nullable, HeapType::Defined(TypeIdx(n as u32))))
    }
    pub(super) fn gc(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        use ValType::*;
        self.enabled(self.features.gc)?;
        let pair = || {
            if let Immediate::Pair(a, b) = i.immediate {
                Ok((a as usize, b as usize))
            } else {
                Err(invalid("expected pair immediate"))
            }
        };
        match i.opcode - 0xfb00 {
            0 | 1 => {
                let n = index(i)?;
                let t = self.structure(n)?;
                check(
                    self.defined(n)?.descriptor.is_none(),
                    "described struct requires descriptor constructor",
                )?;
                if i.opcode == 0xfb00 {
                    s.pop_fields(&t.fields)?;
                } else {
                    self.types.charge(t.fields.len());
                    if self.types.exhausted() {
                        return Err(Error::new(
                            ErrorKind::ResourceLimit,
                            "type validation work budget exhausted",
                        ));
                    }
                    check(
                        t.fields.iter().all(|f| defaultable(f.storage.unpacked())),
                        "nondefaultable struct field",
                    )?
                }
                s.values.push(self.new_reference(n));
            }
            2..=5 => {
                let (n, f) = pair()?;
                let f = self
                    .structure(n)?
                    .fields
                    .get(f)
                    .ok_or_else(|| invalid("struct field index out of bounds"))?;
                if i.opcode == 0xfb05 {
                    check(f.mutable, "immutable struct field")?;
                    s.pop(f.storage.unpacked())?
                } else {
                    check(
                        matches!(f.storage, StorageType::Val(_)) == (i.opcode == 0xfb02),
                        "packed struct read signedness",
                    )?
                }
                s.pop(self.reference(n, true))?;
                if i.opcode != 0xfb05 {
                    s.values.push(f.storage.unpacked())
                }
            }
            6 | 7 => {
                let n = index(i)?;
                let a = self.array(n)?;
                s.pop(I32)?;
                if i.opcode == 0xfb06 {
                    s.pop(a.field.storage.unpacked())?
                } else {
                    check(
                        defaultable(a.field.storage.unpacked()),
                        "nondefaultable array element",
                    )?
                }
                s.values.push(self.new_reference(n))
            }
            8 => {
                let (n, len) = pair()?;
                let a = self.array(n)?;
                let frame = s
                    .controls
                    .last()
                    .ok_or_else(|| invalid("missing control frame"))?;
                let available = s.values.len() - frame.height;
                check(
                    len <= available || frame.unreachable,
                    "array.new_fixed operand underflow",
                )?;
                for _ in 0..len.min(available) {
                    s.pop(a.field.storage.unpacked())?
                }
                s.values.push(self.new_reference(n))
            }
            9 | 10 | 18 | 19 => {
                let (n, seg) = pair()?;
                let a = self.array(n)?;
                if matches!(i.opcode, 0xfb09 | 0xfb12) {
                    check(
                        self.m.data_count.is_some() && seg < self.m.data.len(),
                        "data segment index out of bounds",
                    )?;
                    check(
                        !matches!(a.field.storage, StorageType::Val(Ref(_))),
                        "array data needs numeric storage",
                    )?
                } else {
                    let e = self
                        .m
                        .elements
                        .get(seg)
                        .ok_or_else(|| invalid("element segment index out of bounds"))?;
                    check(
                        value_subtype(Ref(e.ty), a.field.storage.unpacked(), &self.types),
                        "array element segment type mismatch",
                    )?
                }
                s.pop(I32)?;
                s.pop(I32)?;
                if matches!(i.opcode, 0xfb12 | 0xfb13) {
                    check(a.field.mutable, "immutable array initialization")?;
                    s.pop(I32)?;
                    s.pop(self.reference(n, true))?
                } else {
                    s.values.push(self.new_reference(n))
                }
            }
            11..=14 => {
                let n = index(i)?;
                let a = self.array(n)?;
                if i.opcode == 0xfb0e {
                    check(a.field.mutable, "immutable array element")?;
                    s.pop(a.field.storage.unpacked())?
                } else {
                    check(
                        matches!(a.field.storage, StorageType::Val(_)) == (i.opcode == 0xfb0b),
                        "packed array read signedness",
                    )?
                }
                s.pop(I32)?;
                s.pop(self.reference(n, true))?;
                if i.opcode != 0xfb0e {
                    s.values.push(a.field.storage.unpacked())
                }
            }
            15 => s.op(&[Ref(RefType::new(true, HeapType::Array))], &[I32])?,
            16 => {
                let n = index(i)?;
                let a = self.array(n)?;
                check(a.field.mutable, "immutable array fill")?;
                s.op(
                    &[
                        self.reference(n, true),
                        I32,
                        a.field.storage.unpacked(),
                        I32,
                    ],
                    &[],
                )?
            }
            17 => {
                let (a, b) = pair()?;
                let dst = self.array(a)?;
                let src = self.array(b)?;
                check(dst.field.mutable, "immutable array copy destination")?;
                check(
                    value_subtype(
                        src.field.storage.unpacked(),
                        dst.field.storage.unpacked(),
                        &self.types,
                    ),
                    "array copy element mismatch",
                )?;
                check(
                    matches!(src.field.storage, StorageType::Val(_))
                        == matches!(dst.field.storage, StorageType::Val(_))
                        && (matches!(src.field.storage, StorageType::Val(_))
                            || src.field.storage == dst.field.storage),
                    "array copy packed mismatch",
                )?;
                s.op(
                    &[
                        self.reference(a, true),
                        I32,
                        self.reference(b, true),
                        I32,
                        I32,
                    ],
                    &[],
                )?
            }
            20..=23 => {
                let h = if let Immediate::Heap(h) = i.immediate {
                    h
                } else {
                    return Err(invalid("invalid cast heap type"));
                };
                let out = Ref(RefType::new(i.opcode % 2 == 1, h));
                self.valtype(out)?;
                let is_cont =
                    value_subtype(out, Ref(RefType::new(true, HeapType::Cont)), &self.types);
                if self.types.exhausted() {
                    return Err(Error::new(
                        ErrorKind::ResourceLimit,
                        "type validation work budget exhausted",
                    ));
                }
                check(!is_cont, "continuations cannot be cast")?;
                let source = match h {
                    HeapType::Func | HeapType::NoFunc => HeapType::Func,
                    HeapType::Extern | HeapType::NoExtern => HeapType::Extern,
                    HeapType::Exn | HeapType::NoExn => HeapType::Exn,
                    HeapType::Cont | HeapType::NoCont => HeapType::Cont,
                    HeapType::Defined(t) | HeapType::ExactDefined(t) => {
                        match self.defined(t.0 as usize)?.composite {
                            CompositeType::Func(_) => HeapType::Func,
                            CompositeType::Cont(_) => HeapType::Cont,
                            _ => HeapType::Any,
                        }
                    }
                    _ => HeapType::Any,
                };
                s.pop(Ref(RefType::new(true, source)))?;
                s.values.push(if i.opcode <= 0xfb15 { I32 } else { out })
            }
            24 | 25 => {
                let (flags, label, from, to) = if let Immediate::Cast {
                    flags,
                    label,
                    from,
                    to,
                } = i.immediate
                {
                    (flags, label, from, to)
                } else {
                    return Err(invalid("invalid branch cast"));
                };
                let a = Ref(RefType::new(flags & 1 != 0, from));
                let b = Ref(RefType::new(flags & 2 != 0, to));
                self.valtype(a)?;
                self.valtype(b)?;
                let is_cont =
                    value_subtype(b, Ref(RefType::new(true, HeapType::Cont)), &self.types);
                if self.types.exhausted() {
                    return Err(Error::new(
                        ErrorKind::ResourceLimit,
                        "type validation work budget exhausted",
                    ));
                }
                check(!is_cont, "continuations cannot be cast")?;
                check(
                    value_subtype(b, a, &self.types),
                    "cast target is not a subtype",
                )?;
                s.pop(a)?;
                let ts = s.label(label as usize, &self.m.types)?;
                let (&expected, prefix) = ts
                    .as_slice()
                    .split_last()
                    .ok_or_else(|| invalid("cast branch label has no reference"))?;
                let difference = match a {
                    Ref(mut r) => {
                        if flags & 2 != 0 {
                            r.nullable = false;
                        }
                        Ref(r)
                    }
                    _ => a,
                };
                let branch = if i.opcode == 0xfb18 { b } else { difference };
                check(
                    value_subtype(branch, expected, &self.types),
                    "cast branch result mismatch",
                )?;
                s.pop_all(prefix)?;
                s.push_slice(prefix)?;
                s.values
                    .push(if i.opcode == 0xfb18 { difference } else { b })
            }
            26 | 27 => {
                let input = if i.opcode == 0xfb1a {
                    HeapType::Extern
                } else {
                    HeapType::Any
                };
                let output = if i.opcode == 0xfb1a {
                    HeapType::Any
                } else {
                    HeapType::Extern
                };
                let v = s.any()?;
                let nullable = if let Some(v) = v {
                    check(
                        value_subtype(v, Ref(RefType::new(true, input)), &self.types),
                        "reference conversion type mismatch",
                    )?;
                    if let Ref(r) = v { r.nullable } else { true }
                } else {
                    s.unknown.push(s.values.len());
                    true
                };
                s.values.push(Ref(RefType::new(nullable, output)));
            }
            28 => s.op(&[I32], &[Ref(RefType::new(false, HeapType::I31))])?,
            29 | 30 => s.op(&[Ref(RefType::new(true, HeapType::I31))], &[I32])?,
            32 | 33 => {
                self.enabled(self.features.custom_descriptors)?;
                let n = index(i)?;
                let t = self.structure(n)?;
                let d = self
                    .defined(n)?
                    .descriptor
                    .ok_or_else(|| invalid("struct has no descriptor"))?;
                s.pop(Ref(RefType::new(true, HeapType::ExactDefined(d))))?;
                if i.opcode == 0xfb20 {
                    s.pop_fields(&t.fields)?;
                } else {
                    self.types.charge(t.fields.len());
                    if self.types.exhausted() {
                        return Err(Error::new(
                            ErrorKind::ResourceLimit,
                            "type validation work budget exhausted",
                        ));
                    }
                    check(
                        t.fields.iter().all(|f| defaultable(f.storage.unpacked())),
                        "nondefaultable struct field",
                    )?
                }
                s.values.push(self.new_reference(n))
            }
            34 => {
                self.enabled(self.features.custom_descriptors)?;
                let n = index(i)?;
                let d = self
                    .defined(n)?
                    .descriptor
                    .ok_or_else(|| invalid("struct has no descriptor"))?;
                let actual = s.any()?;
                let exact = actual.is_none_or(|v| {
                    value_subtype(
                        v,
                        Ref(RefType::new(
                            true,
                            HeapType::ExactDefined(TypeIdx(n as u32)),
                        )),
                        &self.types,
                    )
                });
                if self.types.exhausted() {
                    return Err(Error::new(
                        ErrorKind::ResourceLimit,
                        "type validation work budget exhausted",
                    ));
                }
                if let Some(v) = actual {
                    check(
                        value_subtype(v, self.reference(n, true), &self.types),
                        "descriptor operand type mismatch",
                    )?
                }
                s.values.push(Ref(RefType::new(
                    false,
                    if exact {
                        HeapType::ExactDefined(d)
                    } else {
                        HeapType::Defined(d)
                    },
                )))
            }
            35 | 36 => {
                self.enabled(self.features.custom_descriptors)?;
                let h = if let Immediate::Heap(h) = i.immediate {
                    h
                } else {
                    return Err(invalid("invalid descriptor cast"));
                };
                let n = match h {
                    HeapType::Defined(n) | HeapType::ExactDefined(n) => n,
                    _ => return Err(invalid("descriptor cast needs defined struct")),
                };
                let d = self
                    .defined(n.0 as usize)?
                    .descriptor
                    .ok_or_else(|| invalid("cast type has no descriptor"))?;
                let exact = matches!(h, HeapType::ExactDefined(_));
                s.pop(Ref(RefType::new(
                    true,
                    if exact {
                        HeapType::ExactDefined(d)
                    } else {
                        HeapType::Defined(d)
                    },
                )))?;
                s.pop(Ref(RefType::new(true, HeapType::Any)))?;
                s.values.push(Ref(RefType::new(i.opcode == 0xfb24, h)))
            }
            37 | 38 => {
                self.enabled(self.features.custom_descriptors)?;
                let (flags, label, from, to) = if let Immediate::Cast {
                    flags,
                    label,
                    from,
                    to,
                } = i.immediate
                {
                    (flags, label, from, to)
                } else {
                    return Err(invalid("invalid descriptor branch cast"));
                };
                let n = match to {
                    HeapType::Defined(n) | HeapType::ExactDefined(n) => n,
                    _ => return Err(invalid("descriptor cast needs defined struct")),
                };
                let d = self
                    .defined(n.0 as usize)?
                    .descriptor
                    .ok_or_else(|| invalid("cast type has no descriptor"))?;
                s.pop(Ref(RefType::new(
                    true,
                    if matches!(to, HeapType::ExactDefined(_)) {
                        HeapType::ExactDefined(d)
                    } else {
                        HeapType::Defined(d)
                    },
                )))?;
                let a = Ref(RefType::new(flags & 1 != 0, from));
                let b = Ref(RefType::new(flags & 2 != 0, to));
                self.valtype(a)?;
                self.valtype(b)?;
                let is_cont =
                    value_subtype(b, Ref(RefType::new(true, HeapType::Cont)), &self.types);
                if self.types.exhausted() {
                    return Err(Error::new(
                        ErrorKind::ResourceLimit,
                        "type validation work budget exhausted",
                    ));
                }
                check(!is_cont, "continuations cannot be cast")?;
                check(
                    value_subtype(a, Ref(RefType::new(true, HeapType::Any)), &self.types),
                    "cast source must have same heap hierarchy",
                )?;
                s.pop(a)?;
                let ts = s.label(label as usize, &self.m.types)?;
                let (&last, prefix) = ts
                    .as_slice()
                    .split_last()
                    .ok_or_else(|| invalid("cast label missing result"))?;
                let difference = match a {
                    Ref(mut r) => {
                        if flags & 2 != 0 {
                            r.nullable = false;
                        }
                        Ref(r)
                    }
                    _ => a,
                };
                check(
                    value_subtype(
                        if i.opcode == 0xfb25 { b } else { difference },
                        last,
                        &self.types,
                    ),
                    "cast branch type mismatch",
                )?;
                s.pop_all(prefix)?;
                s.push_slice(prefix)?;
                s.values
                    .push(if i.opcode == 0xfb25 { difference } else { b })
            }
            _ => return Err(invalid("unknown GC instruction")),
        }
        Ok(())
    }
}
