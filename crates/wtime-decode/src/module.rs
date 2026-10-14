use crate::{
    Immediate, Instruction, Reader,
    instructions::{count, expression, val},
};
use std::sync::Arc;
use wtime_types::*;
#[derive(Clone, Debug)]
pub struct DecodedModule {
    pub bytes: Arc<[u8]>,
    pub customs: Vec<CustomSection>,
    pub types: Vec<FuncType>,
    pub type_groups: Vec<TypeGroup>,
    pub imports: Vec<Import>,
    pub functions: Vec<TypeIdx>,
    pub tables: Vec<TableType>,
    pub table_inits: Vec<Option<Vec<Instruction>>>,
    pub memories: Vec<MemoryType>,
    pub globals: Vec<Global>,
    pub exports: Vec<Export>,
    pub start: Option<FuncIdx>,
    pub elements: Vec<Element>,
    pub data: Vec<Data>,
    pub code: Vec<FunctionBody>,
    pub data_count: Option<u32>,
    pub tags: Vec<TagType>,
}
#[derive(Clone, Debug)]
pub struct CustomSection {
    pub name: String,
    pub data: Span,
}
#[derive(Clone, Debug)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub kind: ImportKind,
}
#[derive(Clone, Debug)]
pub enum ImportKind {
    Func(TypeIdx),
    ExactFunc(TypeIdx),
    Table(TableType),
    Memory(MemoryType),
    Global(GlobalType),
    Tag(TagType),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalKind {
    Func,
    Table,
    Memory,
    Global,
    Tag,
}
#[derive(Clone, Debug)]
pub struct Export {
    pub name: String,
    pub kind: ExternalKind,
    pub index: u32,
}
#[derive(Clone, Debug)]
pub struct Global {
    pub ty: GlobalType,
    pub init: Vec<Instruction>,
}
#[derive(Clone, Debug)]
pub struct FunctionBody {
    pub locals: Vec<ValType>,
    pub instructions: Vec<Instruction>,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub enum SegmentMode {
    Passive,
    Active {
        index: u32,
        offset: Vec<Instruction>,
    },
    Declarative,
}
#[derive(Clone, Debug)]
pub struct Data {
    pub mode: SegmentMode,
    pub bytes: Span,
}
#[derive(Clone, Debug)]
pub struct Element {
    pub mode: SegmentMode,
    pub ty: RefType,
    pub init: Vec<Vec<Instruction>>,
}
fn boolean(r: &mut Reader) -> Result<bool> {
    match r.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(r.error("invalid boolean")),
    }
}
fn limits(r: &mut Reader, memory: bool) -> Result<(Limits, IndexType, bool, u8)> {
    let f = r.byte()? as u32;
    let mask = if memory { 15 } else { 5 };
    if f & !mask != 0 {
        return Err(r.error("invalid limits flags"));
    }
    let wide = f & 4 != 0;
    // With memory64 enabled the grammar uses u64 limits even for i32
    // memories and tables; address-width bounds are semantic validation.
    let min = r.memory_offset()?;
    let max = if f & 1 != 0 {
        Some(r.memory_offset()?)
    } else {
        None
    };
    let page = if f & 8 != 0 {
        let p = r.u32()?;
        if p >= 64 {
            return Err(r.error("custom page size overflows u64"));
        }
        p as u8
    } else {
        16
    };
    r.feature(!wide || r.features().memory64, "memory64")?;
    r.feature(
        f & 8 == 0 || r.features().custom_page_sizes,
        "custom page sizes",
    )?;
    Ok((
        Limits { min, max },
        if wide { IndexType::I64 } else { IndexType::I32 },
        f & 2 != 0,
        page,
    ))
}
fn table(r: &mut Reader) -> Result<TableType> {
    let ValType::Ref(element) = val(r)? else {
        return Err(r.error("table element must be reference type"));
    };
    let (limits, index_type, _, _) = limits(r, false)?;
    Ok(TableType {
        limits,
        index_type,
        element,
    })
}
fn memory(r: &mut Reader) -> Result<MemoryType> {
    let (limits, index_type, shared, page_size_log2) = limits(r, true)?;
    Ok(MemoryType {
        limits,
        index_type,
        shared,
        page_size_log2,
    })
}
fn global(r: &mut Reader) -> Result<GlobalType> {
    Ok(GlobalType {
        value: val(r)?,
        mutable: boolean(r)?,
    })
}
fn tag(r: &mut Reader) -> Result<TagType> {
    if r.byte()? != 0 {
        return Err(r.error("invalid tag attribute"));
    }
    Ok(TagType {
        function: TypeIdx(r.u32()?),
    })
}
fn kind(r: &mut Reader) -> Result<ExternalKind> {
    Ok(match r.byte()? {
        0 => ExternalKind::Func,
        1 => ExternalKind::Table,
        2 => ExternalKind::Memory,
        3 => ExternalKind::Global,
        4 => ExternalKind::Tag,
        _ => return Err(r.error("invalid external kind")),
    })
}
fn import_kind(s: &mut Reader) -> Result<ImportKind> {
    if s.peek()? == 0x20 {
        s.feature(s.features().custom_descriptors, "exact function imports")?;
        s.byte()?;
        return Ok(ImportKind::ExactFunc(TypeIdx(s.u32()?)));
    }
    Ok(match kind(s)? {
        ExternalKind::Func => ImportKind::Func(TypeIdx(s.u32()?)),
        ExternalKind::Table => ImportKind::Table(table(s)?),
        ExternalKind::Memory => ImportKind::Memory(memory(s)?),
        ExternalKind::Global => ImportKind::Global(global(s)?),
        ExternalKind::Tag => ImportKind::Tag(tag(s)?),
    })
}
fn values(r: &mut Reader) -> Result<Vec<ValType>> {
    let n = count(r)?;
    let mut v = Vec::new();
    for _ in 0..n {
        v.push(val(r)?)
    }
    Ok(v)
}
fn subtype(r: &mut Reader) -> Result<SubType> {
    let mut final_ = true;
    let mut supertype = None;
    if matches!(r.peek()?, 0x4f | 0x50) {
        final_ = r.byte()? == 0x4f;
        let n = count(r)?;
        for _ in 0..n {
            supertype = Some(TypeIdx(r.u32()?));
        }
        if n > 1 {
            return Err(Error::new(ErrorKind::Validation, "multiple supertypes").at(r.offset()));
        }
    }
    let describes = if r.peek()? == 0x4c {
        r.feature(r.features().custom_descriptors, "custom descriptors")?;
        r.byte()?;
        Some(TypeIdx(r.u32()?))
    } else {
        None
    };
    let descriptor = if r.peek()? == 0x4d {
        r.feature(r.features().custom_descriptors, "custom descriptors")?;
        r.byte()?;
        Some(TypeIdx(r.u32()?))
    } else {
        None
    };
    let composite = match r.byte()? {
        0x60 => CompositeType::Func(FuncType {
            params: values(r)?,
            results: values(r)?,
        }),
        0x5f => {
            let n = count(r)?;
            let mut fields = Vec::new();
            for _ in 0..n {
                fields.push(field(r)?)
            }
            CompositeType::Struct(StructType { fields })
        }
        0x5e => CompositeType::Array(ArrayType { field: field(r)? }),
        0x5d => CompositeType::Cont(ContType {
            function: TypeIdx(
                u32::try_from(r.signed(33)?)
                    .map_err(|_| r.error("invalid continuation function type"))?,
            ),
        }),
        _ => return Err(r.error("unknown composite type")),
    };
    Ok(SubType {
        final_,
        supertype,
        composite,
        descriptor,
        describes,
    })
}
fn field(r: &mut Reader) -> Result<FieldType> {
    let storage = match r.peek()? {
        0x78 => {
            r.byte()?;
            StorageType::I8
        }
        0x77 => {
            r.byte()?;
            StorageType::I16
        }
        _ => StorageType::Val(val(r)?),
    };
    Ok(FieldType {
        storage,
        mutable: boolean(r)?,
    })
}
pub fn decode(
    bytes: &[u8],
    _features: &FeatureSet,
    policy: &ResourceLimits,
) -> Result<DecodedModule> {
    if bytes.len() > policy.max_module_bytes {
        return Err(Error::new(ErrorKind::ResourceLimit, "module byte limit"));
    }
    let mut r = Reader::new(bytes).with_features(*_features);
    if r.take(8)? != b"\0asm\x01\0\0\0" {
        return Err(r.error("invalid WebAssembly magic or version"));
    }
    let mut m = DecodedModule {
        bytes: Arc::from(bytes),
        customs: vec![],
        types: vec![],
        type_groups: vec![],
        imports: vec![],
        functions: vec![],
        tables: vec![],
        table_inits: vec![],
        memories: vec![],
        globals: vec![],
        exports: vec![],
        start: None,
        elements: vec![],
        data: vec![],
        code: vec![],
        data_count: None,
        tags: vec![],
    };
    let mut last = 0;
    let mut seen = 0u16;
    while r.remaining() > 0 {
        let id = r.byte()?;
        let len = r.u32()? as usize;
        let mut s = r.child(len)?;
        if id == 0 {
            let name = s.name()?;
            let start = s.offset();
            s.take(s.remaining())?;
            m.customs.push(CustomSection {
                name,
                data: Span::new(start, s.offset()),
            });
            continue;
        }
        let rank = match id {
            1..=5 => id,
            13 => 6,
            6..=9 => id + 1,
            12 => 11,
            10 => 12,
            11 => 13,
            _ => return Err(s.error("unknown section")),
        };
        if rank < last || seen & (1 << id) != 0 {
            return Err(s.error("duplicate or out-of-order section"));
        }
        last = rank;
        seen |= 1 << id;
        match id {
            1 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let explicit = s.peek()? == 0x4e;
                    if explicit {
                        s.feature(s.features().gc, "recursive type groups")?;
                    }
                    let n = if explicit {
                        s.byte()?;
                        count(&mut s)?
                    } else {
                        1
                    };
                    if n > policy.max_rec_group_size {
                        return Err(Error::new(
                            ErrorKind::ResourceLimit,
                            "recursion group limit",
                        ));
                    }
                    let mut types = Vec::new();
                    for _ in 0..n {
                        let ty = subtype(&mut s)?;
                        m.types.push(match &ty.composite {
                            CompositeType::Func(f) => f.clone(),
                            _ => FuncType::default(),
                        });
                        types.push(ty)
                    }
                    m.type_groups.push(TypeGroup { explicit, types })
                }
            }
            2 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let module = s.name()?;
                    let name = s.name()?;
                    if name.is_empty() && matches!(s.peek()?, 0x7e | 0x7f) {
                        let mode = s.byte()?;
                        if !_features.compact_imports {
                            return Err(Error::new(
                                ErrorKind::DisabledFeature,
                                "compact imports disabled",
                            ));
                        }
                        if mode == 0x7f {
                            let n = count(&mut s)?;
                            for _ in 0..n {
                                m.imports.push(Import {
                                    module: module.clone(),
                                    name: s.name()?,
                                    kind: import_kind(&mut s)?,
                                })
                            }
                        } else {
                            let kind = import_kind(&mut s)?;
                            let n = count(&mut s)?;
                            for _ in 0..n {
                                m.imports.push(Import {
                                    module: module.clone(),
                                    name: s.name()?,
                                    kind: kind.clone(),
                                })
                            }
                        }
                    } else {
                        m.imports.push(Import {
                            module,
                            name,
                            kind: import_kind(&mut s)?,
                        })
                    }
                }
            }
            3 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    m.functions.push(TypeIdx(s.u32()?))
                }
            }
            4 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let has_init = s.peek()? == 0x40;
                    if has_init {
                        s.feature(s.features().function_references, "table initializer")?;
                        s.byte()?;
                        if s.byte()? != 0 {
                            return Err(s.error("invalid table initializer prefix"));
                        }
                    }
                    m.tables.push(table(&mut s)?);
                    m.table_inits.push(if has_init {
                        Some(expression(&mut s)?)
                    } else {
                        None
                    });
                }
            }
            5 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    m.memories.push(memory(&mut s)?)
                }
            }
            6 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    m.globals.push(Global {
                        ty: global(&mut s)?,
                        init: expression(&mut s)?,
                    })
                }
            }
            7 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    m.exports.push(Export {
                        name: s.name()?,
                        kind: kind(&mut s)?,
                        index: s.u32()?,
                    })
                }
            }
            8 => m.start = Some(FuncIdx(s.u32()?)),
            9 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let f = s.u32()?;
                    if f > 7 {
                        return Err(s.error("invalid element flags"));
                    }
                    s.feature(
                        f & 4 == 0 || s.features().reference_types,
                        "expression element segments",
                    )?;
                    s.feature(
                        f & 3 == 0 || s.features().bulk_memory,
                        "bulk element segments",
                    )?;
                    let mode = match f & 3 {
                        0 => SegmentMode::Active {
                            index: 0,
                            offset: expression(&mut s)?,
                        },
                        2 => SegmentMode::Active {
                            index: s.u32()?,
                            offset: expression(&mut s)?,
                        },
                        1 => SegmentMode::Passive,
                        _ => SegmentMode::Declarative,
                    };
                    let expr = f & 4 != 0;
                    let ty = if f == 0 || f == 4 {
                        RefType::FUNCREF
                    } else if expr {
                        let ValType::Ref(t) = val(&mut s)? else {
                            return Err(s.error("invalid element type"));
                        };
                        t
                    } else {
                        if s.byte()? != 0 {
                            return Err(s.error("invalid element kind"));
                        }
                        RefType::FUNCREF
                    };
                    let ty = if expr {
                        ty
                    } else {
                        RefType::new(false, HeapType::Func)
                    };
                    let n = count(&mut s)?;
                    let mut init = Vec::new();
                    for _ in 0..n {
                        init.push(if expr {
                            expression(&mut s)?
                        } else {
                            let start = s.offset();
                            let index = s.u32()?;
                            vec![
                                Instruction {
                                    opcode: 0xd2,
                                    immediate: Immediate::Index(index),
                                    span: Span::new(start, s.offset()),
                                },
                                Instruction {
                                    opcode: 0x0b,
                                    immediate: Immediate::None,
                                    span: Span::new(s.offset(), s.offset()),
                                },
                            ]
                        })
                    }
                    m.elements.push(Element { mode, ty, init })
                }
            }
            10 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let size = s.u32()? as usize;
                    let mut b = s.child(size)?;
                    let start = b.offset();
                    let groups = count(&mut b)?;
                    let mut local_groups = Vec::new();
                    let mut total = 0u32;
                    for _ in 0..groups {
                        let n = b.u32()?;
                        let ty = val(&mut b)?;
                        total = total
                            .checked_add(n)
                            .ok_or_else(|| b.error("local count overflow"))?;
                        local_groups.push((n, ty));
                    }
                    let instructions = expression(&mut b)?;
                    b.finish()?;
                    if total as usize > policy.max_locals {
                        return Err(Error::new(ErrorKind::ResourceLimit, "local limit"));
                    }
                    let mut locals = Vec::new();
                    for (n, ty) in local_groups {
                        locals.resize(locals.len() + n as usize, ty);
                    }
                    m.code.push(FunctionBody {
                        locals,
                        instructions,
                        span: Span::new(start, b.offset()),
                    })
                }
            }
            11 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let flags = s.u32()?;
                    s.feature(flags == 0 || s.features().bulk_memory, "bulk data segments")?;
                    let mode = match flags {
                        0 => SegmentMode::Active {
                            index: 0,
                            offset: expression(&mut s)?,
                        },
                        1 => SegmentMode::Passive,
                        2 => SegmentMode::Active {
                            index: s.u32()?,
                            offset: expression(&mut s)?,
                        },
                        _ => return Err(s.error("invalid data flags")),
                    };
                    let len = s.u32()? as usize;
                    let start = s.offset();
                    s.take(len)?;
                    m.data.push(Data {
                        mode,
                        bytes: Span::new(start, s.offset()),
                    })
                }
            }
            12 => {
                s.feature(s.features().bulk_memory, "data count section")?;
                m.data_count = Some(s.u32()?);
            }
            13 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    m.tags.push(tag(&mut s)?)
                }
            }
            _ => unreachable!(),
        };
        s.finish()?;
    }
    if m.types.len() > policy.max_types
        || m.functions.len() > policy.max_functions
        || m.tables.len() > policy.max_tables
        || m.memories.len() > policy.max_memories
        || m.globals.len() > policy.max_globals
        || m.code.iter().map(|b| b.instructions.len()).sum::<usize>() > policy.max_instructions
    {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "module declaration limit",
        ));
    }
    if m.functions.len() != m.code.len() {
        return Err(r.error("function/code count mismatch"));
    }
    if m.data_count.is_none()
        && m.code.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i.opcode, 0xfc08 | 0xfc09))
        })
    {
        return Err(r.error("data count section required"));
    }
    if m.data_count.is_some_and(|n| n as usize != m.data.len()) {
        return Err(r.error("data count mismatch"));
    }
    Ok(m)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn module(parts: &[(u8, &[u8])]) -> Vec<u8> {
        let mut b = b"\0asm\x01\0\0\0".to_vec();
        for (id, payload) in parts {
            b.push(*id);
            assert!(payload.len() < 128);
            b.push(payload.len() as u8);
            b.extend_from_slice(payload)
        }
        b
    }
    fn parse(b: &[u8]) -> Result<DecodedModule> {
        decode(b, &FeatureSet::default(), &ResourceLimits::default())
    }
    #[test]
    fn t017_sections_and_absolute_offsets() {
        let b = module(&[
            (1, &[1, 0x60, 0, 1, 0x7f]),
            (3, &[1, 0]),
            (10, &[1, 4, 0, 0x41, 42, 0x0b]),
        ]);
        let m = parse(&b).unwrap();
        assert_eq!(m.code[0].instructions[0].immediate, Immediate::I32(42));
        let s = m.code[0].instructions[0].span;
        assert_eq!(&b[s.start..s.end], &[0x41, 42]);
    }
    #[test]
    fn t018_malformed_sections() {
        assert!(parse(&module(&[(1, &[0]), (1, &[0])])).is_err());
        assert!(parse(&module(&[(3, &[0]), (1, &[0])])).is_err());
        assert!(parse(&module(&[(1, &[0, 0])])).is_err());
        assert!(parse(&module(&[(0, &[1, 0xff])])).is_err());
        assert!(parse(&module(&[(3, &[1, 0])])).is_err());
    }
    #[test]
    fn t019_bounded_body_and_locals() {
        assert!(
            parse(&module(&[
                (1, &[1, 0x60, 0, 0]),
                (3, &[1, 0]),
                (10, &[1, 2, 0, 0x41, 0, 0x0b])
            ]))
            .is_err()
        );
        let b = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 4, 1, 10, 0x7f, 0x0b]),
        ]);
        let policy = ResourceLimits {
            max_locals: 9,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&b, &FeatureSet::default(), &policy)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
    }
    #[test]
    fn t024_data_spans() {
        let b = module(&[(11, &[1, 1, 3, 1, 2, 3])]);
        let m = parse(&b).unwrap();
        assert!(matches!(m.data[0].mode, SegmentMode::Passive));
        let span = m.data[0].bytes;
        assert_eq!(&m.bytes[span.start..span.end], &[1, 2, 3]);
    }
    #[test]
    fn t141_compact_imports() {
        let b = module(&[(2, &[1, 1, b'm', 0, 0x7e, 0, 0, 2, 1, b'a', 1, b'b'])]);
        let m = parse(&b).unwrap();
        assert_eq!(m.imports.len(), 2);
        assert_eq!(m.imports[1].name, "b");
        assert_eq!(
            decode(&b, &FeatureSet::mvp(), &ResourceLimits::default())
                .unwrap_err()
                .kind,
            ErrorKind::DisabledFeature
        );
    }
    #[test]
    fn t020_reserved_and_relaxed_opcodes() {
        let mut r = Reader::new(&[0xfd, 0x9a, 1, 0x0b]);
        assert!(expression(&mut r).is_err());
        let mut r = Reader::new(&[0xfd, 0x80, 2, 0x0b]);
        assert_eq!(expression(&mut r).unwrap()[0].opcode, 0xfa00);
        let mut r = Reader::new(&[0xfc, 18, 0x0b]);
        assert!(expression(&mut r).is_err());
    }
    #[test]
    fn t021_padded_signed_constants() {
        let mut r = Reader::new(&[0x41, 0xff, 0xff, 0xff, 0xff, 0x7f, 0x0b]);
        assert_eq!(expression(&mut r).unwrap()[0].immediate, Immediate::I32(-1));
    }
    #[test]
    fn t142_extension_immediates() {
        let mut r = Reader::new(&[0xe3, 0, 1, 0, 2, 3, 0x0b]);
        assert!(
            matches!(&expression(&mut r).unwrap()[0].immediate,Immediate::Resume{ty:0,tag:None,handlers} if handlers==&vec![(2,Some(3))])
        );
        let mut r = Reader::new(&[0xfb, 24, 3, 0, 0x70, 0x6f, 0x0b]);
        assert!(matches!(
            expression(&mut r).unwrap()[0].immediate,
            Immediate::Cast {
                flags: 3,
                label: 0,
                ..
            }
        ));
    }
    #[test]
    fn t145_exact_import_and_table_initializer() {
        let b = module(&[
            (2, &[1, 1, b'm', 1, b'f', 0x20, 0]),
            (4, &[1, 0x40, 0, 0x63, 0, 0, 1, 0xd2, 0, 0x0b]),
        ]);
        let m = parse(&b).unwrap();
        assert!(matches!(
            m.imports[0].kind,
            ImportKind::ExactFunc(TypeIdx(0))
        ));
        assert_eq!(m.table_inits[0].as_ref().unwrap()[0].opcode, 0xd2);
    }
    #[test]
    fn t146_descriptors_and_reference_blocks() {
        let b = module(&[(1, &[1, 0x4e, 2, 0x4d, 1, 0x5f, 0, 0x4c, 0, 0x5f, 0])]);
        let m = parse(&b).unwrap();
        assert_eq!(m.type_groups[0].types[0].descriptor, Some(TypeIdx(1)));
        assert_eq!(m.type_groups[0].types[1].describes, Some(TypeIdx(0)));
        let mut r = Reader::new(&[0x02, 0x63, 0, 0x0b, 0x0b]);
        assert!(matches!(
            expression(&mut r).unwrap()[0].immediate,
            Immediate::BlockType(_)
        ));
        let mut r = Reader::new(&[0xd4, 0x0b]);
        assert_eq!(expression(&mut r).unwrap().len(), 2);
    }
    #[test]
    fn t113_memory_offset_width() {
        let raw = &[0x28, 0, 0x80, 0x80, 0x80, 0x80, 0x80, 0, 0x0b];
        assert!(expression(&mut Reader::new(raw).with_memory64(false)).is_err());
        assert!(expression(&mut Reader::new(raw).with_memory64(true)).is_ok());
    }
    #[test]
    #[ignore = "external fixture audit configured by WTIME_DECODE_CORPUS"]
    fn t024_external_corpus() {
        let path = std::env::var("WTIME_DECODE_CORPUS").unwrap();
        let mut count = 0;
        for entry in std::fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|x| x == "wasm") {
                let bytes = std::fs::read(&p).unwrap();
                parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                count += 1;
            }
        }
        assert!(count > 0);
        eprintln!("decoded {count} external fixtures");
    }
    #[test]
    fn t162_binary_feature_evidence() {
        let mut f = FeatureSet::all_phase3_core();
        f.custom_page_sizes = false;
        let bytes = module(&[(5, &[1, 8, 0, 16])]);
        assert_eq!(
            decode(&bytes, &f, &ResourceLimits::default())
                .unwrap_err()
                .kind,
            ErrorKind::DisabledFeature
        );
        f = FeatureSet::all_phase3_core();
        f.multi_memory = false;
        let mut r = Reader::new(&[0x28, 0x40, 0, 0]).with_features(f);
        assert_eq!(
            crate::decode_instruction(&mut r).unwrap_err().kind,
            ErrorKind::DisabledFeature
        );
        f = FeatureSet::all_phase3_core();
        f.gc = false;
        let bytes = module(&[(1, &[1, 0x4e, 1, 0x60, 0, 0])]);
        assert_eq!(
            decode(&bytes, &f, &ResourceLimits::default())
                .unwrap_err()
                .kind,
            ErrorKind::DisabledFeature
        );
    }

    #[test]
    fn t023_custom_sections_preserved() {
        let bytes = module(&[(0, &[3, b'x', b'y', b'z', 0xff, 0, 1]), (0, &[0, 2])]);
        let m = parse(&bytes).unwrap();
        assert_eq!(m.customs.len(), 2);
        assert_eq!(m.customs[0].name, "xyz");
        let span = m.customs[0].data;
        assert_eq!(&m.bytes[span.start..span.end], &[0xff, 0, 1]);
        assert_eq!(m.customs[1].name, "");
    }
    #[test]
    fn t019_malformed_precedes_local_policy() {
        let b = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (
                10,
                &[1, 10, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x7f, 2, 0x7e, 0x0b],
            ),
        ]);
        assert_eq!(parse(&b).unwrap_err().kind, ErrorKind::Decode);
        let b = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 4, 1, 100, 0x7f, 0x41]),
        ]);
        let p = ResourceLimits {
            max_locals: 2,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&b, &FeatureSet::default(), &p).unwrap_err().kind,
            ErrorKind::Decode
        );
    }
    #[test]
    fn t024_missing_data_count_is_malformed() {
        let b = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 5, 0, 0xfc, 9, 0, 0x0b]),
        ]);
        assert_eq!(parse(&b).unwrap_err().kind, ErrorKind::Decode);
    }
    #[test]
    fn t162_truncated_disabled_feature_is_malformed() {
        let b = module(&[(5, &[1, 8])]);
        assert_eq!(
            decode(&b, &FeatureSet::stable_core(), &ResourceLimits::default())
                .unwrap_err()
                .kind,
            ErrorKind::Decode
        );
    }
    #[test]
    fn t024_empty_group_and_nonnull_legacy_elements() {
        let b = module(&[(1, &[1, 0x4e, 0]), (9, &[1, 1, 0, 0])]);
        let m = parse(&b).unwrap();
        assert!(m.type_groups[0].types.is_empty());
        assert!(!m.elements[0].ty.nullable);
    }
    #[test]
    fn t085_invalid_page_size_is_semantic() {
        let bytes = module(&[(5, &[1, 8, 0, 17])]);
        assert_eq!(parse(&bytes).unwrap().memories[0].page_size_log2, 17);
        let bytes = module(&[(5, &[1, 8, 0, 0x80, 2])]);
        assert_eq!(parse(&bytes).unwrap_err().kind, ErrorKind::Decode);
    }
}
