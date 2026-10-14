//! Unexecuted P039 draft. Pure numeric adapters retain canonical stack values.
use super::*;

pub(super) fn value_type(ty: &ValType) -> bool {
    matches!(ty, ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64 | ValType::V128)
}

pub(super) fn supported(instruction: &wtime_validate::Instruction, pc: usize, terminal: usize) -> bool {
    if super::lower_scalar::supported(instruction, pc, terminal) { return true; }
    match instruction.opcode {
        0x1c => matches!(&instruction.immediate, Immediate::Types(types) if types.len() == 1 && value_type(&types[0])),
        0x43 => matches!(instruction.immediate, Immediate::F32(_)),
        0x44 => matches!(instruction.immediate, Immediate::F64(_)),
        0x45..=0xc4 | 0xfc00..=0xfc07 => matches!(instruction.immediate, Immediate::None),
        // Exclude every SIMD memory and lane-memory opcode. Prepared adapter
        // arguments contain neither Store-local handles nor movable roots.
        0xfd0c | 0xfd0d => matches!(&instruction.immediate, Immediate::Bytes(bytes) if bytes.len() == 16),
        0xfd0e..=0xfd53 | 0xfd5e..=0xfdff | 0xfa00..=0xfaff => matches!(instruction.immediate, Immediate::None | Immediate::Index(_)),
        _ => false,
    }
}

pub(super) fn execute(function: &CompactFunction, op: CompactOp, stack: &mut Vec<Value>) -> Result<bool> {
    match op.opcode {
        0x43 => stack.push(Value::F32(op.payload as u32)),
        0x44 => stack.push(Value::F64(op.payload)),
        0x45..=0xc4 | 0xfc00..=0xfc07 => numeric::exec(op.opcode, stack)?,
        0xfd0c | 0xfd0d => {
            let Some(SideImmediate::Vector(bytes)) = function.sides.get(op.aux as usize) else {
                return Err(trap("unverified compact vector immediate"));
            };
            crate::simd::execute_prepared_vector(op.opcode, bytes, stack)?;
        }
        0xfd0e..=0xfd53 | 0xfd5e..=0xfdff | 0xfa00..=0xfaff => {
            // These temporary immediates own no heap storage. Pure helpers see
            // the canonical operand stack after the charged source boundary.
            let immediate = match op.form {
                NONE => Immediate::None,
                INDEX => Immediate::Index(op.aux),
                _ => return Err(trap("unverified compact SIMD adapter immediate")),
            };
            crate::simd::execute(op.opcode, &immediate, stack)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
