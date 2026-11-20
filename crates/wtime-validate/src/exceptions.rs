use super::*;
impl Context<'_> {
    pub(super) fn add_tag(&mut self, t: TagType) -> Result<()> {
        self.enabled(self.features.exception_handling)?;
        let ty = self.ty(t.function.as_usize())?.clone();
        check(
            ty.results.is_empty() || self.features.stack_switching,
            "tag type must have no results",
        )?;
        self.tags.push(ty);
        Ok(())
    }
    pub(super) fn exception_instruction(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        self.enabled(self.features.exception_handling)?;
        match i.opcode {
            0x08 => {
                let t = self
                    .tags
                    .get(index(i)?)
                    .ok_or_else(|| invalid("tag index out of bounds"))?;
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
                let (params, results) = match block {
                    Immediate::Block(-64) => (vec![], vec![]),
                    Immediate::Block(-1) => (vec![], vec![ValType::I32]),
                    Immediate::Block(-2) => (vec![], vec![ValType::I64]),
                    Immediate::Block(-3) => (vec![], vec![ValType::F32]),
                    Immediate::Block(-4) => (vec![], vec![ValType::F64]),
                    Immediate::Block(-5) => (vec![], vec![ValType::V128]),
                    Immediate::Block(-16) => (vec![], vec![ValType::FUNCREF]),
                    Immediate::Block(-17) => (vec![], vec![ValType::EXTERNREF]),
                    Immediate::BlockType(t) => (vec![], vec![*t]),
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
                        (vec![], vec![ValType::Ref(RefType::new(true, h))])
                    }
                    Immediate::Block(n) if *n >= 0 => {
                        self.enabled(self.features.multi_value)?;
                        let t = self.ty(*n as usize)?;
                        (t.params.clone(), t.results.clone())
                    }
                    _ => return Err(invalid("invalid try_table block type")),
                };
                for t in params.iter().chain(&results) {
                    self.valtype(*t)?;
                }
                for c in catches {
                    let mut delivered = if let Some(tag) = c.tag {
                        {
                            let tag = self
                                .tags
                                .get(tag as usize)
                                .ok_or_else(|| invalid("catch tag index out of bounds"))?;
                            check(tag.results.is_empty(), "catch tag must have no results")?;
                            tag.params.clone()
                        }
                    } else {
                        vec![]
                    };
                    if c.kind == 1 || c.kind == 3 {
                        delivered.push(ValType::Ref(RefType::new(false, HeapType::Exn)))
                    }
                    let target = s.label(c.label as usize)?;
                    check(
                        delivered.len() == target.len()
                            && delivered
                                .iter()
                                .zip(&target)
                                .all(|(a, b)| value_subtype(*a, *b, &self.types)),
                        "catch target type mismatch",
                    )?;
                }
                s.pop_all(&params)?;
                let height = s.values.len();
                s.values.extend(&params);
                s.controls.push(Control {
                    kind: 0x1f,
                    height,
                    params,
                    results,
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
