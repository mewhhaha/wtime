use crate::Reader;
use wtime_types::*;
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub opcode: u16,
    pub immediate: Immediate,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    pub kind: u8,
    pub tag: Option<u32>,
    pub label: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Immediate {
    TryTable {
        block: Box<Immediate>,
        catches: Vec<Catch>,
    },
    Resume {
        ty: u32,
        tag: Option<u32>,
        handlers: Vec<(u32, Option<u32>)>,
    },
    Cast {
        flags: u8,
        label: u32,
        from: HeapType,
        to: HeapType,
    },
    None,
    Index(u32),
    Pair(u32, u32),
    Block(i64),
    BlockType(ValType),
    BrTable(Vec<u32>, u32),
    MemLane {
        align: u32,
        offset: u64,
        memory: u32,
        lane: u8,
    },
    MemArg {
        align: u32,
        offset: u64,
        memory: u32,
    },
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    Types(Vec<ValType>),
    Heap(HeapType),
    Bytes(Vec<u8>),
}
pub(crate) fn val(r: &mut Reader) -> Result<ValType> {
    if matches!(r.peek()?,0x68..=0x6e|0x71..=0x75) {
        return Ok(ValType::Ref(RefType::new(true, heap(r)?)));
    }
    Ok(match r.byte()? {
        0x7f => ValType::I32,
        0x7e => ValType::I64,
        0x7d => ValType::F32,
        0x7c => ValType::F64,
        0x7b => ValType::V128,
        0x70 => ValType::FUNCREF,
        0x6f => ValType::EXTERNREF,
        0x63 => ValType::Ref(RefType::new(true, heap(r)?)),
        0x64 => ValType::Ref(RefType::new(false, heap(r)?)),
        _ => return Err(r.error("invalid value type")),
    })
}
pub(crate) fn heap(r: &mut Reader) -> Result<HeapType> {
    if r.peek()? == 0x62 {
        r.feature(r.features().custom_descriptors, "exact heap types")?;
        r.byte()?;
        return Ok(HeapType::ExactDefined(TypeIdx(r.u32()?)));
    }
    Ok(match r.signed(33)? {
        n if n >= 0 => HeapType::Defined(TypeIdx(n as u32)),
        -16 => HeapType::Func,
        -17 => HeapType::Extern,
        -18 => HeapType::Any,
        -19 => HeapType::Eq,
        -20 => HeapType::I31,
        -21 => HeapType::Struct,
        -22 => HeapType::Array,
        -15 => HeapType::None,
        -13 => HeapType::NoFunc,
        -14 => HeapType::NoExtern,
        -23 => HeapType::Exn,
        -12 => HeapType::NoExn,
        -24 => HeapType::Cont,
        -11 => HeapType::NoCont,
        _ => return Err(r.error("invalid heap type")),
    })
}
pub(crate) fn count(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > r.remaining() {
        return Err(r.error("vector count exceeds remaining bytes"));
    }
    Ok(n)
}
fn memarg(r: &mut Reader) -> Result<Immediate> {
    let flags = r.u32()?;
    if flags & !127 != 0 {
        return Err(r.error("invalid memory alignment flags"));
    }
    r.feature(
        flags & 64 == 0 || r.features().multi_memory,
        "explicit memory index",
    )?;
    let memory = if flags & 64 != 0 { r.u32()? } else { 0 };
    Ok(Immediate::MemArg {
        align: flags & 63,
        offset: r.memory_offset()?,
        memory,
    })
}
pub(crate) fn expression(r: &mut Reader) -> Result<Vec<Instruction>> {
    expression_with_capacity(r, 0)
}
/// A function body: the reader holds just the body, and instructions average
/// about three bytes, so reserving for that avoids most regrowth copies.
pub(crate) fn body_expression(r: &mut Reader) -> Result<Vec<Instruction>> {
    let estimate = r.remaining() / 3;
    expression_with_capacity(r, estimate)
}
fn expression_with_capacity(r: &mut Reader, capacity: usize) -> Result<Vec<Instruction>> {
    let mut out = Vec::new();
    // A hint only: a failed reservation leaves growth to push.
    let _ = out.try_reserve_exact(capacity);
    let mut depth = 1usize;
    while depth > 0 {
        out.push(read_instruction(r, &mut depth)?);
    }
    Ok(out)
}
/// Decode one instruction and its immediates, without validating control nesting.
/// The returned span is absolute within the reader's original module.
pub fn decode_instruction(r: &mut Reader) -> Result<Instruction> {
    read_instruction(r, &mut 1)
}
fn read_instruction(r: &mut Reader, depth: &mut usize) -> Result<Instruction> {
    let start = r.offset();
    let first = r.byte()?;
    let mut opcode = first as u16;
    let immediate = match first {
        0x02..=0x04 => {
            *depth += 1;
            if matches!(r.peek()?, 0x63 | 0x64) {
                Immediate::BlockType(val(r)?)
            } else {
                Immediate::Block(r.signed(33)?)
            }
        }
        0x0b => {
            *depth -= 1;
            Immediate::None
        }
        0x0c | 0x0d | 0x10 | 0x12 | 0x14 | 0x15 | 0x20..=0x26 | 0xd2 | 0xd5 | 0xd6 | 0x08 => {
            Immediate::Index(r.u32()?)
        }
        0x0e => {
            let n = count(r)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(r.u32()?)
            }
            Immediate::BrTable(v, r.u32()?)
        }
        0x11 | 0x13 => Immediate::Pair(r.u32()?, r.u32()?),
        0x1c => {
            let n = count(r)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(val(r)?)
            }
            Immediate::Types(v)
        }
        0x28..=0x3e => memarg(r)?,
        0x3f | 0x40 => Immediate::Index(r.u32()?),
        0x41 => Immediate::I32(r.signed(32)? as i32),
        0x42 => Immediate::I64(r.signed(64)?),
        0x43 => Immediate::F32(u32::from_le_bytes(r.take(4)?.try_into().unwrap())),
        0x44 => Immediate::F64(u64::from_le_bytes(r.take(8)?.try_into().unwrap())),
        0xd0 => Immediate::Heap(heap(r)?),
        0xfc => {
            let sub = r.u32()?;
            if sub > 22 || sub == 18 {
                return Err(r.error("unknown miscellaneous opcode"));
            }
            opcode = 0xfc00 | sub as u16;
            match sub {
                0..=7 | 19..=22 => Immediate::None,
                8 | 10 | 12 | 14 => Immediate::Pair(r.u32()?, r.u32()?),
                _ => Immediate::Index(r.u32()?),
            }
        }
        0xfd => {
            let sub = r.u32()?;
            if sub > 0x113
                || matches!(
                    sub,
                    154 | 162
                        | 165
                        | 166
                        | 175
                        | 176
                        | 178
                        | 179
                        | 180
                        | 187
                        | 194
                        | 197
                        | 198
                        | 207
                        | 208
                        | 210
                        | 211
                        | 212
                        | 226
                        | 238
                )
            {
                return Err(r.error("unsupported SIMD opcode"));
            }
            opcode = if sub >= 256 {
                0xfa00 | (sub as u16 - 256)
            } else {
                0xfd00 | sub as u16
            };
            match sub {
                0..=11 | 92 | 93 => memarg(r)?,
                12 | 13 => Immediate::Bytes(r.take(16)?.to_vec()),
                21..=34 => Immediate::Index(r.byte()? as u32),
                84..=91 => {
                    let m = memarg(r)?;
                    let lane = r.byte()?;
                    if let Immediate::MemArg {
                        align,
                        offset,
                        memory,
                    } = m
                    {
                        Immediate::MemLane {
                            align,
                            offset,
                            memory,
                            lane,
                        }
                    } else {
                        return Err(r.error("invalid memory lane"));
                    }
                }
                14..=20 | 35..=83 | 94..=255 | 256..=275 => Immediate::None,
                _ => return Err(r.error("unknown SIMD opcode")),
            }
        }
        0xfb => {
            let sub = r.u32()?;
            if sub > 38 || sub == 31 {
                return Err(r.error("unknown GC opcode"));
            }
            opcode = 0xfb00 | sub as u16;
            match sub {
                0 | 1 | 6 | 7 | 11..=14 | 16 | 32..=34 => Immediate::Index(r.u32()?),
                2..=5 | 8..=10 | 17..=19 => Immediate::Pair(r.u32()?, r.u32()?),
                20..=23 | 35 | 36 => Immediate::Heap(heap(r)?),
                24 | 25 | 37 | 38 => {
                    let flags = r.byte()?;
                    if flags > 3 {
                        return Err(r.error("invalid cast flags"));
                    }
                    Immediate::Cast {
                        flags,
                        label: r.u32()?,
                        from: heap(r)?,
                        to: heap(r)?,
                    }
                }
                15 | 26..=30 => Immediate::None,
                _ => return Err(r.error("unknown GC opcode")),
            }
        }
        0xe0 | 0xe2 => Immediate::Index(r.u32()?),
        0xe1 | 0xe6 => Immediate::Pair(r.u32()?, r.u32()?),
        0xe3..=0xe5 => {
            let ty = r.u32()?;
            let tag = if first == 0xe4 { Some(r.u32()?) } else { None };
            let n = count(r)?;
            let mut handlers = Vec::new();
            for _ in 0..n {
                let switch = r.byte()?;
                if switch > 1 {
                    return Err(r.error("invalid continuation handler"));
                }
                let t = r.u32()?;
                handlers.push((t, if switch == 0 { Some(r.u32()?) } else { None }));
            }
            Immediate::Resume { ty, tag, handlers }
        }
        0x1f => {
            *depth += 1;
            let block = if matches!(r.peek()?, 0x63 | 0x64) {
                Immediate::BlockType(val(r)?)
            } else {
                Immediate::Block(r.signed(33)?)
            };
            let n = count(r)?;
            let mut catches = Vec::new();
            for _ in 0..n {
                let kind = r.byte()?;
                if kind > 3 {
                    return Err(r.error("invalid catch kind"));
                }
                catches.push(Catch {
                    kind,
                    tag: if kind < 2 { Some(r.u32()?) } else { None },
                    label: r.u32()?,
                });
            }
            Immediate::TryTable {
                block: Box::new(block),
                catches,
            }
        }
        0x0a => Immediate::None,
        0xfe => {
            let sub = r.u32()?;
            opcode = 0xfe00 | sub as u16;
            match sub {
                0..=2 | 16..=78 => memarg(r)?,
                3 => {
                    if r.byte()? != 0 {
                        return Err(r.error("invalid atomic fence immediate"));
                    }
                    Immediate::None
                }
                _ => return Err(r.error("unknown atomic opcode")),
            }
        }
        0x00 | 0x01 | 0x05 | 0x0f | 0x1a | 0x1b | 0x45..=0xc4 | 0xd1 | 0xd3 | 0xd4 => {
            Immediate::None
        }
        _ => return Err(r.error("unknown opcode")),
    };
    Ok(Instruction {
        opcode,
        immediate,
        span: Span::new(start, r.offset()),
    })
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    fn encoding(key: &str) -> Vec<u8> {
        let mut pieces = key.split(':');
        let mut bytes = vec![u8::from_str_radix(pieces.next().unwrap(), 16).unwrap()];
        if let Some(sub) = pieces.next() {
            let mut n: u32 = sub.parse().unwrap();
            loop {
                let b = (n & 127) as u8;
                n >>= 7;
                bytes.push(b | if n > 0 { 128 } else { 0 });
                if n == 0 {
                    break;
                }
            }
        }
        bytes.extend([0; 64]);
        bytes
    }
    #[test]
    #[ignore = "requires opcode ledger environment from opcode_coverage.py"]
    fn t008_actual_dispatch_coverage() {
        let Ok(assigned) = std::env::var("WTIME_ASSIGNED_OPCODES") else {
            return;
        };
        let reserved = std::env::var("WTIME_RESERVED_OPCODES").unwrap();
        for key in assigned.split(',').filter(|s| !s.is_empty()) {
            decode_instruction(&mut Reader::new(&encoding(key)))
                .unwrap_or_else(|e| panic!("missing decoder handler for {key}: {e}"));
        }
        for key in reserved.split(',').filter(|s| !s.is_empty()) {
            assert!(
                decode_instruction(&mut Reader::new(&encoding(key))).is_err(),
                "reserved opcode {key} acquired a decoder handler"
            );
        }
    }
}
