use super::*;
fn sequence_subtype(a: &[ValType], b: &[ValType], types: &types::TypeEnvironment) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| value_subtype(*a, *b, types))
}
impl Context<'_> {
    fn cont(&self, n: usize) -> Result<(TypeIdx, &FuncType)> {
        let Some(SubType {
            composite: CompositeType::Cont(c),
            ..
        }) = self.types.get(n)
        else {
            return Err(invalid("expected continuation type"));
        };
        Ok((c.function, self.ty(c.function.0 as usize)?))
    }
    fn contref(n: usize) -> ValType {
        ValType::Ref(RefType::new(true, HeapType::Defined(TypeIdx(n as u32))))
    }
    pub(super) fn continuation(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        use ValType::*;
        self.enabled(self.features.stack_switching)?;
        match i.opcode {
            0xe0 => {
                let n = index(i)?;
                let (ft, _) = self.cont(n)?;
                s.pop(Ref(RefType::new(true, HeapType::Defined(ft))))?;
                s.values.push(Ref(RefType::new(
                    false,
                    HeapType::Defined(TypeIdx(n as u32)),
                )))
            }
            0xe1 => {
                let (a, b) = if let Immediate::Pair(a, b) = i.immediate {
                    (a as usize, b as usize)
                } else {
                    return Err(invalid("invalid continuation bind"));
                };
                let (_, a_type) = self.cont(a)?;
                let (_, b_type) = self.cont(b)?;
                let prefix = a_type
                    .params
                    .len()
                    .checked_sub(b_type.params.len())
                    .ok_or_else(|| invalid("continuation binding arity"))?;
                check(
                    sequence_subtype(&b_type.params, &a_type.params[prefix..], &self.types)
                        && sequence_subtype(&a_type.results, &b_type.results, &self.types),
                    "continuation binding variance",
                )?;
                s.pop(Self::contref(a))?;
                s.pop_all(&a_type.params[..prefix])?;
                s.values.push(Ref(RefType::new(
                    false,
                    HeapType::Defined(TypeIdx(b as u32)),
                )))
            }
            0xe2 => {
                let t = self
                    .tags
                    .get(index(i)?)
                    .ok_or_else(|| invalid("suspend tag index out of bounds"))?;
                s.op(&t.params, &t.results)?
            }
            0xe3..=0xe5 => {
                let (ty, tag, handlers) = if let Immediate::Resume {
                    ty,
                    tag,
                    ref handlers,
                } = i.immediate
                {
                    (ty as usize, tag, handlers)
                } else {
                    return Err(invalid("invalid resume immediate"));
                };
                let (_, ft) = self.cont(ty)?;
                for (tag, label) in handlers {
                    let tag = self
                        .tags
                        .get(*tag as usize)
                        .ok_or_else(|| invalid("handler tag index out of bounds"))?;
                    if let Some(label) = label {
                        let mut target = s.label(*label as usize)?;
                        let cont = match target.pop() {
                            Some(Ref(RefType {
                                heap: HeapType::Defined(t) | HeapType::ExactDefined(t),
                                ..
                            })) => t,
                            _ => return Err(invalid("suspend handler needs continuation result")),
                        };
                        let (_, ct) = self.cont(cont.0 as usize)?;
                        check(
                            sequence_subtype(&tag.params, &target, &self.types),
                            "handler payload types mismatch",
                        )?;
                        check(
                            sequence_subtype(&ct.params, &tag.results, &self.types)
                                && sequence_subtype(&ft.results, &ct.results, &self.types),
                            "handler continuation type mismatch",
                        )?
                    } else {
                        check(
                            tag.params.is_empty()
                                && sequence_subtype(&tag.results, &ft.results, &self.types),
                            "switch handler tag mismatch",
                        )?
                    }
                }
                s.pop(Self::contref(ty))?;
                match i.opcode {
                    0xe3 => s.pop_all(&ft.params)?,
                    0xe4 => {
                        let t = self
                            .tags
                            .get(tag.ok_or_else(|| invalid("resume_throw tag missing"))? as usize)
                            .ok_or_else(|| invalid("resume_throw tag out of bounds"))?;
                        check(t.results.is_empty(), "resume_throw requires exception tag")?;
                        s.pop_all(&t.params)?
                    }
                    _ => s.pop(Ref(RefType::new(true, HeapType::Exn)))?,
                }
                s.values.extend(&ft.results)
            }
            0xe6 => {
                let (ty, tag) = if let Immediate::Pair(a, b) = i.immediate {
                    (a as usize, b as usize)
                } else {
                    return Err(invalid("invalid switch immediate"));
                };
                let tag = self
                    .tags
                    .get(tag)
                    .ok_or_else(|| invalid("switch tag out of bounds"))?;
                check(tag.params.is_empty(), "switch tag must have no parameters")?;
                let (_, ft) = self.cont(ty)?;
                let last = ft
                    .params
                    .last()
                    .ok_or_else(|| invalid("switch continuation requires peer parameter"))?;
                let peer = match last {
                    Ref(RefType {
                        heap: HeapType::Defined(t) | HeapType::ExactDefined(t),
                        ..
                    }) => *t,
                    _ => return Err(invalid("switch peer must be continuation")),
                };
                let (_, peer) = self.cont(peer.0 as usize)?;
                check(
                    sequence_subtype(&ft.results, &tag.results, &self.types)
                        && sequence_subtype(&tag.results, &peer.results, &self.types),
                    "switch result types mismatch",
                )?;
                s.pop(Self::contref(ty))?;
                s.pop_all(&ft.params[..ft.params.len() - 1])?;
                s.values.extend(&peer.params)
            }
            _ => return Err(invalid("unknown continuation operation")),
        }
        Ok(())
    }
}
