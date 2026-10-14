use super::*;
impl Context<'_> {
    pub(super) fn atomic(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        use ValType::*;
        self.enabled(self.features.threads)?;
        let n = i.opcode - 0xfe00;
        if n == 3 {
            return Ok(());
        }
        let (align, offset, memory) = if let Immediate::MemArg {
            align,
            offset,
            memory,
        } = i.immediate
        {
            (align, offset, memory)
        } else {
            return Err(invalid("invalid atomic immediate"));
        };
        let m = self
            .memories
            .get(memory as usize)
            .ok_or_else(|| invalid("atomic memory index out of bounds"))?;
        check(
            m.index_type == IndexType::I64 || offset <= u32::MAX as u64,
            "atomic memory offset overflow",
        )?;
        let addr = m.index_type.val_type();
        if n <= 2 {
            check(
                align == if n == 2 { 3 } else { 2 },
                "atomic alignment must equal natural alignment",
            )?;
            return match n {
                0 => s.op(&[addr, I32], &[I32]),
                1 => s.op(&[addr, I32, I64], &[I32]),
                _ => s.op(&[addr, I64, I64], &[I32]),
            };
        }
        check((16..=78).contains(&n), "unknown atomic opcode")?;
        let lane = (n - 16) % 7;
        let t = if matches!(lane, 0 | 2 | 3) { I32 } else { I64 };
        let width = match lane {
            0 => 2,
            1 => 3,
            2 | 4 => 0,
            3 | 5 => 1,
            _ => 2,
        };
        check(
            align == width,
            "atomic alignment must equal natural alignment",
        )?;
        match n {
            16..=22 => s.op(&[addr], &[t]),
            23..=29 => s.op(&[addr, t], &[]),
            72..=78 => s.op(&[addr, t, t], &[t]),
            _ => s.op(&[addr, t], &[t]),
        }
    }
}
