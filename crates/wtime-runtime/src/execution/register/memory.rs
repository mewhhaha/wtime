//! Scalar memory32 accesses. No memory slice or length survives an operation.
use super::ir::{MemoryArgument, NumericType};
use crate::{ImportKind, IndexType, Inst, Mem, MemoryType, Module, Result, range, trap};

pub(super) fn signature(op: u16) -> Option<(NumericType, usize, bool)> {
    let ty = match op {
        0x28 | 0x2c..=0x2f | 0x36 | 0x3a..=0x3b => NumericType::I32,
        0x29 | 0x30..=0x35 | 0x37 | 0x3c..=0x3e => NumericType::I64,
        0x2a | 0x38 => NumericType::F32,
        0x2b | 0x39 => NumericType::F64,
        _ => return None,
    };
    let width = match op {
        0x28 | 0x2a | 0x34..=0x36 | 0x38 | 0x3e => 4,
        0x29 | 0x2b | 0x37 | 0x39 => 8,
        0x2c | 0x2d | 0x30 | 0x31 | 0x3a | 0x3c => 1,
        _ => 2,
    };
    Some((ty, width, op >= 0x36))
}
pub(super) fn eligible(ty: MemoryType) -> bool {
    ty.index_type == IndexType::I32 && !ty.shared && ty.page_size_log2 == 16
}
// Every caller charges the complete import scan against preparation work.
pub(super) fn declared(module: &Module, index: u32) -> Option<MemoryType> {
    let decoded = module.data.inner.decoded();
    let mut remaining = index as usize;
    for import in &decoded.imports {
        if let ImportKind::Memory(ty) = import.kind {
            if remaining == 0 {
                return Some(ty);
            }
            remaining -= 1;
        }
    }
    decoded.memories.get(remaining).copied()
}
pub(super) fn access(
    instance: &Inst,
    memories: &mut crate::arena::Arena<Mem>,
    argument: &MemoryArgument,
    opcode: u16,
    address: u64,
    value: u64,
) -> Result<Option<u64>> {
    let (_, width, store) = signature(opcode).ok_or_else(|| trap("register memory opcode"))?;
    let memory = instance
        .mems
        .get(argument.memory as usize)
        .ok_or_else(|| trap("register memory binding"))?;
    let index = memory.0.index;
    if memories.get(index).is_none() {
        return Err(trap("register memory binding"));
    }
    let memory = &mut memories[index];
    if !eligible(memory.ty) || memory.shared.is_some() {
        return Err(trap("unadmitted register memory binding"));
    }
    let offset = (address as u32 as u64)
        .checked_add(argument.offset)
        .ok_or_else(|| trap("address overflow"))?;
    let range = range(offset, width, memory.bytes.len())?;
    if store {
        memory
            .bytes
            .write(range.start, &value.to_le_bytes()[..width])
            .map_err(crate::allocation_failed)?;
        Ok(None)
    } else {
        let mut buffer = [0; 8];
        memory.bytes.read(range.start, &mut buffer[..width]);
        let value = u64::from_le_bytes(buffer);
        Ok(Some(match opcode {
            0x2c => value as i8 as i32 as u32 as u64,
            0x2e => value as i16 as i32 as u32 as u64,
            0x30 => value as i8 as i64 as u64,
            0x32 => value as i16 as i64 as u64,
            0x34 => value as i32 as i64 as u64,
            _ => value,
        }))
    }
}
