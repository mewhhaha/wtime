use super::*;
impl Context<'_> {
    pub(super) fn add_tag(&mut self, t: TagType) -> Result<()> {
        self.enabled(self.features.exception_handling)?;
        budget(self.tags.len() < self.limits.max_tags, "tag declaration limit exceeded")?;
        let ti = t.function.as_usize();
        let ty = self.ty(ti)?;
        check(
            ty.results.is_empty() || self.features.stack_switching,
            "tag type must have no results",
        )?;
        self.tags.push(ti);
        Ok(())
    }
    pub(super) fn exception_instruction(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        self.enabled(self.features.exception_handling)?;
        match i.opcode {
            0x08 => {
                let &ti = self
                    .tags
                    .get(index(i)?)
                    .ok_or_else(|| invalid("tag index out of bounds"))?;
                let t = self.ty(ti)?;
                check(t.results.is_empty(), "throw tag must have no results")?;
                s.pop_all(&t.params)?;
                s.dead()
            }
            0x0a => {
                s.pop(ValType::Ref(RefType::new(true, HeapType::Exn)))?;
                s.dead()
            }
            0x1f => {
                let Immediate::TryTable(table) = &i.immediate else {
                    return Err(invalid("invalid try_table immediate"));
                };
                let (block, catches) = (&table.block, &table.catches);
                let sig = match block {
                    Immediate::Block(-64) => BlockSig::Empty,
                    Immediate::Block(-1) => BlockSig::Single(ValType::I32),
                    Immediate::Block(-2) => BlockSig::Single(ValType::I64),
                    Immediate::Block(-3) => BlockSig::Single(ValType::F32),
                    Immediate::Block(-4) => BlockSig::Single(ValType::F64),
                    Immediate::Block(-5) => {
                        self.valtype(ValType::V128)?;
                        BlockSig::Single(ValType::V128)
                    }
                    Immediate::Block(-16) => {
                        self.valtype(ValType::FUNCREF)?;
                        BlockSig::Single(ValType::FUNCREF)
                    }
                    Immediate::Block(-17) => {
                        self.valtype(ValType::EXTERNREF)?;
                        BlockSig::Single(ValType::EXTERNREF)
                    }
                    Immediate::BlockType(t) => {
                        self.valtype(*t)?;
                        BlockSig::Single(*t)
                    }
                    Immediate::Block(n @ (-24..=-18 | -15..=-11)) => {
                        let h = match *n {
                            -18 => HeapType::Any,
                            -19 => HeapType::Eq,
                            -20 => HeapType::I31,
                            -21 => HeapType::Struct,
                            -22 => HeapType::Array,
                            -23 => HeapType::Exn,
                            -24 => HeapType::Cont,
                            -15 => HeapType::None,
                            -14 => HeapType::NoExtern,
                            -13 => HeapType::NoFunc,
                            -12 => HeapType::NoExn,
                            _ => HeapType::NoCont,
                        };
                        let vt = ValType::Ref(RefType::new(true, h));
                        self.valtype(vt)?;
                        BlockSig::Single(vt)
                    }
                    Immediate::Block(n) if *n >= 0 => {
                        self.enabled(self.features.multi_value)?;
                        let idx = *n as usize;
                        self.ty(idx)?;
                        BlockSig::Func(idx)
                    }
                    _ => return Err(invalid("invalid try_table block type")),
                };
                for c in catches {
                    let params: &[ValType] = if let Some(tag) = c.tag {
                        let &ti = self
                            .tags
                            .get(tag as usize)
                            .ok_or_else(|| invalid("catch tag index out of bounds"))?;
                        let tag = self.ty(ti)?;
                        check(tag.results.is_empty(), "catch tag must have no results")?;
                        &tag.params
                    } else {
                        &[]
                    };
                    let exn = (c.kind == 1 || c.kind == 3)
                        .then_some(ValType::Ref(RefType::new(false, HeapType::Exn)));
                    let target = s.label(c.label as usize, &self.m.types)?;
                    let tslice = target.as_slice();
                    let expected_len = params.len() + usize::from(exn.is_some());
                    let matches = expected_len == tslice.len()
                        && params
                            .iter()
                            .chain(exn.iter())
                            .zip(tslice)
                            .all(|(a, b)| value_subtype(*a, *b, &self.types));
                    check(matches, "catch target type mismatch")?;
                }
                let params = sig.params(&self.m.types);
                s.pop_all(params.as_slice())?;
                let height = s.values.len();
                s.values.extend_from_slice(params.as_slice());
                s.controls.push(Control {
                    kind: 0x1f,
                    height,
                    sig,
                    unreachable: false,
                    has_else: false,
                    initialized: s.initialization_changes.len(),
                });
            }
            _ => return Err(invalid("invalid exception instruction")),
        }
        Ok(())
    }
}
