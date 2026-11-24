#[cfg(test)]
use crate::instructions::expression;
use crate::{
    Immediate, Instruction, Reader,
    instructions::{
        AlignedAtomicUsize, DecodeWorkerBudget, body_expression_bounded, count, expression_bounded,
        val,
    },
};
use std::sync::Arc;
use wtime_types::*;
/// Decoded representation of a WebAssembly binary module.
#[derive(Clone, Debug)]
pub struct DecodedModule {
    /// Original owned module bytes referenced by all decoded [`Span`]s.
    pub bytes: Arc<[u8]>,
    /// Custom sections in appearance order.
    pub customs: Vec<CustomSection>,
    /// Flattened function signatures indexed by module type index (non-function types use an empty placeholder).
    pub types: Vec<FuncType>,
    /// Decoded recursion groups from the type section.
    pub type_groups: Vec<TypeGroup>,
    /// Module imports in declaration order (with compact imports expanded).
    pub imports: Vec<Import>,
    /// Declared type index for each defined (non-imported) function.
    pub functions: Vec<TypeIdx>,
    /// Defined table types.
    pub tables: Vec<TableType>,
    /// Optional explicit initializer expressions for each defined table.
    pub table_inits: Vec<Option<Vec<Instruction>>>,
    /// Defined linear memory types.
    pub memories: Vec<MemoryType>,
    /// Defined global variables and their initializer expressions.
    pub globals: Vec<Global>,
    /// Module exports in declaration order.
    pub exports: Vec<Export>,
    /// Optional start function index.
    pub start: Option<FuncIdx>,
    /// Element segments from the element section.
    pub elements: Vec<Element>,
    /// Data segments from the data section.
    pub data: Vec<Data>,
    /// Decoded function bodies corresponding to `functions`.
    pub code: Vec<FunctionBody>,
    /// Optional declared data segment count from the data count section.
    pub data_count: Option<u32>,
    /// Defined tag types.
    pub tags: Vec<TagType>,
}
/// A custom section decoded from a WebAssembly binary.
#[derive(Clone, Debug)]
pub struct CustomSection {
    /// UTF-8 custom section name.
    pub name: String,
    /// Payload byte span inside [`DecodedModule::bytes`].
    pub data: Span,
}
/// A single import declaration from the import section.
#[derive(Clone, Debug)]
pub struct Import {
    /// Imported module name.
    pub module: String,
    /// Imported entity name within `module`.
    pub name: String,
    /// Kind and descriptor of the imported entity.
    pub kind: ImportKind,
}
/// Descriptor specifying the kind and type of an [`Import`].
#[derive(Clone, Debug)]
pub enum ImportKind {
    /// Function import by type index.
    Func(TypeIdx),
    /// Exact function import (`custom-descriptors` proposal) by type index.
    ExactFunc(TypeIdx),
    /// Table import.
    Table(TableType),
    /// Linear memory import.
    Memory(MemoryType),
    /// Global variable import.
    Global(GlobalType),
    /// Tag import.
    Tag(TagType),
}
/// Kind of an exported or external WebAssembly entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalKind {
    /// Function.
    Func,
    /// Table.
    Table,
    /// Linear memory.
    Memory,
    /// Global variable.
    Global,
    /// Exception or continuation tag.
    Tag,
}
/// A single export declaration from the export section.
#[derive(Clone, Debug)]
pub struct Export {
    /// UTF-8 export name.
    pub name: String,
    /// Entity kind being exported.
    pub kind: ExternalKind,
    /// Index in the corresponding entity index space.
    pub index: u32,
}
/// A defined global variable from the global section.
#[derive(Clone, Debug)]
pub struct Global {
    /// Value type and mutability of the global.
    pub ty: GlobalType,
    /// Constant initializer expression instructions.
    pub init: Vec<Instruction>,
}
/// A decoded function body from the code section.
#[derive(Clone, Debug)]
pub struct FunctionBody {
    /// Expanded declared local variable types (excluding parameters).
    pub locals: Vec<ValType>,
    /// Decoded instruction stream, ending with the function's `end` opcode.
    pub instructions: Vec<Instruction>,
    /// The absolute module offset at which each instruction starts.
    pub offsets: Vec<u32>,
    /// Byte span of the function body payload within [`DecodedModule::bytes`].
    pub span: Span,
}
/// Initialization mode of a data or element segment.
#[derive(Clone, Debug)]
pub enum SegmentMode {
    /// Retained for explicit runtime initialization (`memory.init` / `table.init`).
    Passive,
    /// Automatically copied into a memory or table during instantiation.
    Active {
        /// Target memory or table index.
        index: u32,
        /// Constant expression computing the destination offset.
        offset: Vec<Instruction>,
    },
    /// Declarative element segment used only to declare function references.
    Declarative,
}
/// A data segment from the data section.
#[derive(Clone, Debug)]
pub struct Data {
    /// Segment initialization mode.
    pub mode: SegmentMode,
    /// Byte span of the raw data payload within [`DecodedModule::bytes`].
    pub bytes: Span,
}
/// An element segment from the element section.
#[derive(Clone, Debug)]
pub struct Element {
    /// Segment initialization mode.
    pub mode: SegmentMode,
    /// Element reference type.
    pub ty: RefType,
    /// Constant initializer expressions for each element item.
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
fn values_bounded(
    r: &mut Reader,
    type_items: &mut usize,
    max_type_items: usize,
    keep: bool,
) -> Result<(Vec<ValType>, bool)> {
    let n = count(r)?;
    *type_items = type_items.saturating_add(n);
    let over = *type_items > max_type_items;
    if keep && !over {
        let mut v = Vec::new();
        let _ = v.try_reserve_exact(n);
        for _ in 0..n {
            v.push(val(r)?);
        }
        Ok((v, false))
    } else {
        for _ in 0..n {
            let _ = val(r)?;
        }
        Ok((Vec::new(), over))
    }
}
fn subtype_bounded(
    r: &mut Reader,
    type_items: &mut usize,
    max_type_items: usize,
    keep: bool,
) -> Result<(Option<SubType>, bool)> {
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
    let mut over = *type_items > max_type_items;
    let composite = match r.byte()? {
        0x60 => {
            let (params, o1) = values_bounded(r, type_items, max_type_items, keep && !over)?;
            over |= o1;
            let (results, o2) = values_bounded(r, type_items, max_type_items, keep && !over)?;
            over |= o2;
            CompositeType::Func(FuncType { params, results })
        }
        0x5f => {
            let n = count(r)?;
            *type_items = type_items.saturating_add(n);
            if *type_items > max_type_items {
                over = true;
            }
            let mut fields = Vec::new();
            if keep && !over {
                let _ = fields.try_reserve_exact(n);
                for _ in 0..n {
                    fields.push(field(r)?);
                }
            } else {
                for _ in 0..n {
                    let _ = field(r)?;
                }
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
    if keep && !over {
        Ok((
            Some(SubType {
                final_,
                supertype,
                composite,
                descriptor,
                describes,
            }),
            false,
        ))
    } else {
        Ok((None, over))
    }
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
/// Decodes a WebAssembly binary module under the given feature policy and resource limits.
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
    let mut expr_insns: usize = 0;
    let mut expr_imm_elems: usize = 0;
    let mut total_elem_items: u64 = 0;
    let mut decoded_func_count: usize = 0;
    let mut decoded_code_count: usize = 0;
    let mut decoded_data_count: usize = 0;
    let mut saw_data_count_op = false;
    let mut decl_limit_exceeded = false;
    let mut rec_group_limit_exceeded = false;
    let mut expr_limit_exceeded = false;
    let mut local_limit_exceeded = false;
    let mut total_local_limit_exceeded = false;
    let mut code_insn_limit_exceeded = false;
    let mut imm_limit_exceeded = false;
    let max_exports = policy
        .max_functions
        .saturating_add(policy.max_tables)
        .saturating_add(policy.max_memories)
        .saturating_add(policy.max_globals)
        .saturating_add(policy.max_tags)
        .max(65_536);
    while r.remaining() > 0 {
        let id = r.byte()?;
        let len = r.u32()? as usize;
        let mut s = r.child(len)?;
        if id == 0 {
            let name = s.name()?;
            let start = s.offset();
            s.take(s.remaining())?;
            if m.customs.len() < max_exports {
                m.customs.push(CustomSection {
                    name,
                    data: Span::new(start, s.offset()),
                });
            }
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
                let n_groups = count(&mut s)?;
                let mut total_types: usize = 0;
                let mut total_groups: usize = 0;
                let mut type_items: usize = 0;
                let mut type_over = false;
                for _ in 0..n_groups {
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
                        rec_group_limit_exceeded = true;
                        type_over = true;
                    }
                    total_groups = total_groups.saturating_add(1);
                    total_types = total_types.saturating_add(n);
                    if total_types > policy.max_types || total_groups > policy.max_types {
                        type_over = true;
                    }
                    let mut types = Vec::new();
                    for _ in 0..n {
                        let (maybe_ty, items_over) = subtype_bounded(
                            &mut s,
                            &mut type_items,
                            policy.max_instructions,
                            !type_over,
                        )?;
                        if items_over {
                            type_over = true;
                            types.clear();
                        } else if let Some(ty) = maybe_ty {
                            m.types.push(match &ty.composite {
                                CompositeType::Func(f) => f.clone(),
                                _ => FuncType::default(),
                            });
                            types.push(ty);
                        }
                    }
                    if !type_over {
                        m.type_groups.push(TypeGroup { explicit, types });
                    }
                }
                if type_over && !rec_group_limit_exceeded {
                    decl_limit_exceeded = true;
                }
            }
            2 => {
                let n = count(&mut s)?;
                let max_imports = policy
                    .max_functions
                    .saturating_add(policy.max_tables)
                    .saturating_add(policy.max_memories)
                    .saturating_add(policy.max_globals)
                    .saturating_add(policy.max_tags);
                let mut total_imports: usize = 0;
                let mut compact_expansion_bytes: usize = 0;
                let mut import_limit_exceeded = false;
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
                                let item_name = s.name()?;
                                let kind = import_kind(&mut s)?;
                                total_imports = total_imports.saturating_add(1);
                                compact_expansion_bytes = compact_expansion_bytes
                                    .saturating_add(module.len().max(1))
                                    .saturating_add(item_name.len().max(1))
                                    .saturating_add(48);
                                if total_imports > max_imports
                                    || compact_expansion_bytes > policy.max_module_bytes
                                {
                                    import_limit_exceeded = true;
                                } else {
                                    m.imports.push(Import {
                                        module: module.clone(),
                                        name: item_name,
                                        kind,
                                    })
                                }
                            }
                        } else {
                            let kind = import_kind(&mut s)?;
                            let n = count(&mut s)?;
                            for _ in 0..n {
                                let item_name = s.name()?;
                                total_imports = total_imports.saturating_add(1);
                                compact_expansion_bytes = compact_expansion_bytes
                                    .saturating_add(module.len().max(1))
                                    .saturating_add(item_name.len().max(1))
                                    .saturating_add(48);
                                if total_imports > max_imports
                                    || compact_expansion_bytes > policy.max_module_bytes
                                {
                                    import_limit_exceeded = true;
                                } else {
                                    m.imports.push(Import {
                                        module: module.clone(),
                                        name: item_name,
                                        kind: kind.clone(),
                                    })
                                }
                            }
                        }
                    } else {
                        let kind = import_kind(&mut s)?;
                        total_imports = total_imports.saturating_add(1);
                        if total_imports > max_imports {
                            import_limit_exceeded = true;
                        } else {
                            m.imports.push(Import { module, name, kind })
                        }
                    }
                }
                if import_limit_exceeded {
                    decl_limit_exceeded = true;
                }
            }
            3 => {
                let n = count(&mut s)?;
                decoded_func_count = n;
                for i in 0..n {
                    let idx = TypeIdx(s.u32()?);
                    if i < policy.max_functions {
                        m.functions.push(idx);
                    } else {
                        decl_limit_exceeded = true;
                    }
                }
            }
            4 => {
                let n = count(&mut s)?;
                for i in 0..n {
                    let has_init = s.peek()? == 0x40;
                    if has_init {
                        s.feature(s.features().function_references, "table initializer")?;
                        s.byte()?;
                        if s.byte()? != 0 {
                            return Err(s.error("invalid table initializer prefix"));
                        }
                    }
                    let tt = table(&mut s)?;
                    let keep = i < policy.max_tables && !expr_limit_exceeded;
                    let init = if has_init {
                        let (expr, over) = expression_bounded(
                            &mut s,
                            &mut expr_insns,
                            &mut expr_imm_elems,
                            policy.max_instructions,
                            keep,
                        )?;
                        if over {
                            expr_limit_exceeded = true;
                        }
                        Some(expr)
                    } else {
                        None
                    };
                    if i < policy.max_tables {
                        m.tables.push(tt);
                        m.table_inits.push(init);
                    } else {
                        decl_limit_exceeded = true;
                    }
                }
            }
            5 => {
                let n = count(&mut s)?;
                for i in 0..n {
                    let mem = memory(&mut s)?;
                    if i < policy.max_memories {
                        m.memories.push(mem);
                    } else {
                        decl_limit_exceeded = true;
                    }
                }
            }
            6 => {
                let n = count(&mut s)?;
                for i in 0..n {
                    let ty = global(&mut s)?;
                    let keep = i < policy.max_globals && !expr_limit_exceeded;
                    let (init, over) = expression_bounded(
                        &mut s,
                        &mut expr_insns,
                        &mut expr_imm_elems,
                        policy.max_instructions,
                        keep,
                    )?;
                    if over {
                        expr_limit_exceeded = true;
                    }
                    if i < policy.max_globals {
                        m.globals.push(Global { ty, init });
                    } else {
                        decl_limit_exceeded = true;
                    }
                }
            }
            7 => {
                let n = count(&mut s)?;
                for i in 0..n {
                    let exp = Export {
                        name: s.name()?,
                        kind: kind(&mut s)?,
                        index: s.u32()?,
                    };
                    if i < max_exports {
                        m.exports.push(exp);
                    } else {
                        decl_limit_exceeded = true;
                    }
                }
            }
            8 => m.start = Some(FuncIdx(s.u32()?)),
            9 => {
                let n = count(&mut s)?;
                if n > policy.max_elements
                    || (!decl_limit_exceeded
                        && !rec_group_limit_exceeded
                        && !expr_limit_exceeded
                        && m.elements.try_reserve_exact(n).is_err())
                {
                    decl_limit_exceeded = true;
                }
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
                    expr_insns = expr_insns.saturating_add(1);
                    if expr_insns > policy.max_instructions {
                        expr_limit_exceeded = true;
                    }
                    let keep_seg = !decl_limit_exceeded
                        && !rec_group_limit_exceeded
                        && !expr_limit_exceeded
                        && m.elements.len() < policy.max_elements;
                    let mode = match f & 3 {
                        0 => {
                            let (offset, over) = expression_bounded(
                                &mut s,
                                &mut expr_insns,
                                &mut expr_imm_elems,
                                policy.max_instructions,
                                keep_seg,
                            )?;
                            if over {
                                expr_limit_exceeded = true;
                            }
                            SegmentMode::Active { index: 0, offset }
                        }
                        2 => {
                            let index = s.u32()?;
                            let (offset, over) = expression_bounded(
                                &mut s,
                                &mut expr_insns,
                                &mut expr_imm_elems,
                                policy.max_instructions,
                                keep_seg,
                            )?;
                            if over {
                                expr_limit_exceeded = true;
                            }
                            SegmentMode::Active { index, offset }
                        }
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
                    } else if s.features().function_references {
                        RefType::new(false, HeapType::Func)
                    } else {
                        RefType::FUNCREF
                    };
                    let n_items = count(&mut s)?;
                    let mut init = Vec::new();
                    let min_insns_per_item = if expr { 1 } else { 2 };
                    let keep_items = keep_seg
                        && !expr_limit_exceeded
                        && total_elem_items
                            .saturating_add(u64::try_from(n_items).unwrap_or(u64::MAX))
                            <= policy.max_table_elements
                        && n_items
                            .checked_mul(min_insns_per_item)
                            .and_then(|d| expr_insns.checked_add(d))
                            .is_some_and(|t| t <= policy.max_instructions)
                        && n_items
                            .checked_mul(4)
                            .and_then(|d| expr_imm_elems.checked_add(d))
                            .is_some_and(|t| t <= policy.max_instructions);
                    if keep_items && init.try_reserve_exact(n_items).is_err() {
                        expr_limit_exceeded = true;
                    }
                    for _ in 0..n_items {
                        total_elem_items = total_elem_items.saturating_add(1);
                        expr_imm_elems = expr_imm_elems.saturating_add(4);
                        if total_elem_items > policy.max_table_elements
                            || expr_imm_elems > policy.max_instructions
                        {
                            expr_limit_exceeded = true;
                            init = Vec::new();
                        }
                        let retain = keep_items && !expr_limit_exceeded;
                        if expr {
                            let (item, over) = expression_bounded(
                                &mut s,
                                &mut expr_insns,
                                &mut expr_imm_elems,
                                policy.max_instructions,
                                retain,
                            )?;
                            if over {
                                expr_limit_exceeded = true;
                                init = Vec::new();
                            } else if retain {
                                init.push(item);
                            }
                        } else {
                            let index = s.u32()?;
                            expr_insns = expr_insns.saturating_add(2);
                            if expr_insns > policy.max_instructions {
                                expr_limit_exceeded = true;
                                init = Vec::new();
                            } else if retain {
                                init.push(vec![
                                    Instruction {
                                        opcode: 0xd2,
                                        immediate: Immediate::Index(index),
                                    },
                                    Instruction {
                                        opcode: 0x0b,
                                        immediate: Immediate::None,
                                    },
                                ]);
                            }
                        }
                    }
                    if keep_seg && !expr_limit_exceeded {
                        m.elements.push(Element { mode, ty, init });
                    }
                }
            }
            10 => {
                let n = count(&mut s)?;
                decoded_code_count = n;
                if n > policy.max_functions {
                    // Stream-validate every body without retaining any `Reader`, `Instruction`,
                    // local-group, or immediate buffers so malformed size/syntax still precedes
                    // `ResourceLimit` in O(1) RAM.
                    for _ in 0..n {
                        let size = s.u32()?;
                        let b = s.child(size as usize)?;
                        let parsed = parse_function_body(b, policy, None, false)?;
                        saw_data_count_op |= parsed.saw_data_count_op;
                    }
                    decl_limit_exceeded = true;
                } else {
                    // Each body's bytes first, then the bodies, in parallel when
                    // the section is large; errors are reported in body order, a
                    // malformed size only after the bodies before it.
                    let mut bodies = Vec::with_capacity(n);
                    let mut sizes = Ok(());
                    for _ in 0..n {
                        match s.u32().and_then(|size| s.child(size as usize)) {
                            Ok(b) => bodies.push(b),
                            Err(e) => {
                                sizes = Err(e);
                                break;
                            }
                        }
                    }
                    let threads = policy.body_threads(bodies.iter().map(|b| b.remaining()).sum());
                    let keep_bodies =
                        !decl_limit_exceeded && !rec_group_limit_exceeded && !expr_limit_exceeded;
                    // Share cache-line-isolated module-wide pre-reserved budgets across all workers
                    // via per-worker batched banks so parallel decoding never retains past `policy`
                    // limits while avoiding per-body atomic contention.
                    let shared_insns = AlignedAtomicUsize::new(expr_insns);
                    let shared_imm_elems = AlignedAtomicUsize::new(expr_imm_elems);
                    let shared_locals = AlignedAtomicUsize::new(0);
                    let decoded = wtime_types::parallel::map_ordered(
                        &bodies,
                        threads,
                        || {
                            DecodeWorkerBudget::new(
                                &shared_insns,
                                &shared_imm_elems,
                                &shared_locals,
                                policy.max_instructions,
                                policy.max_total_locals,
                                threads,
                            )
                        },
                        |wb, b| parse_function_body(b.clone(), policy, Some(wb), keep_bodies),
                    );
                    let mut parsed_bodies = Vec::with_capacity(decoded.len());
                    let mut total_decoded_locals: usize = 0;
                    let mut total_decoded_insns: usize = expr_insns;
                    let mut total_decoded_imm_elems: usize = expr_imm_elems;
                    for (idx, body) in decoded.into_iter().enumerate() {
                        let parsed = body?;
                        saw_data_count_op |= parsed.saw_data_count_op;
                        if parsed.total_locals as usize > policy.max_locals {
                            local_limit_exceeded = true;
                        }
                        total_decoded_locals =
                            total_decoded_locals.saturating_add(parsed.total_locals as usize);
                        if total_decoded_locals > policy.max_total_locals {
                            total_local_limit_exceeded = true;
                        }
                        total_decoded_insns = total_decoded_insns.saturating_add(parsed.insn_count);
                        if total_decoded_insns > policy.max_instructions {
                            code_insn_limit_exceeded = true;
                        }
                        total_decoded_imm_elems =
                            total_decoded_imm_elems.saturating_add(parsed.imm_elems);
                        if parsed.imm_limit_exceeded
                            || total_decoded_imm_elems > policy.max_instructions
                        {
                            imm_limit_exceeded = true;
                        }
                        if keep_bodies
                            && !local_limit_exceeded
                            && !total_local_limit_exceeded
                            && !code_insn_limit_exceeded
                            && !imm_limit_exceeded
                        {
                            parsed_bodies.push((idx, parsed));
                        } else if !parsed_bodies.is_empty() {
                            parsed_bodies.clear();
                        }
                    }
                    sizes?;
                    // Carry cumulative instruction and immediate-element counts into Section 11
                    // (`Data`) so data-segment headers and active offset expressions budget against
                    // the live already-retained code section rather than resetting to pre-code counts.
                    expr_insns = total_decoded_insns;
                    expr_imm_elems = total_decoded_imm_elems;
                    if keep_bodies
                        && !local_limit_exceeded
                        && !total_local_limit_exceeded
                        && !code_insn_limit_exceeded
                        && !imm_limit_exceeded
                    {
                        m.code.reserve_exact(parsed_bodies.len());
                        for (idx, parsed) in parsed_bodies {
                            // If a valid module near `max_instructions` or `max_total_locals`
                            // transiently dropped one body due to sibling threads holding unspent
                            // reservations, re-decode just that body sequentially with exact retention.
                            let parsed = if parsed.dropped
                                && (parsed.insn_count > 0 || parsed.total_locals > 0)
                            {
                                parse_function_body(bodies[idx].clone(), policy, None, true)?
                            } else {
                                parsed
                            };
                            m.code.push(FunctionBody {
                                locals: parsed.locals,
                                instructions: parsed.instructions,
                                offsets: parsed.offsets,
                                span: parsed.span,
                            });
                        }
                    }
                }
            }
            11 => {
                let n = count(&mut s)?;
                decoded_data_count = n;
                if n > policy.max_data_segments {
                    decl_limit_exceeded = true;
                }
                let data_keep_base = !decl_limit_exceeded
                    && !rec_group_limit_exceeded
                    && !local_limit_exceeded
                    && !total_local_limit_exceeded
                    && !code_insn_limit_exceeded
                    && !imm_limit_exceeded
                    && !expr_limit_exceeded
                    && expr_insns <= policy.max_instructions
                    && expr_imm_elems <= policy.max_instructions;
                if data_keep_base && m.data.try_reserve_exact(n).is_err() {
                    decl_limit_exceeded = true;
                }
                let data_keep_base = data_keep_base && !decl_limit_exceeded;
                for _ in 0..n {
                    let flags = s.u32()?;
                    s.feature(flags == 0 || s.features().bulk_memory, "bulk data segments")?;
                    expr_insns = expr_insns.saturating_add(1);
                    if expr_insns > policy.max_instructions {
                        expr_limit_exceeded = true;
                    }
                    let keep_data = data_keep_base && !expr_limit_exceeded;
                    let mode = match flags {
                        0 => {
                            let (offset, over) = expression_bounded(
                                &mut s,
                                &mut expr_insns,
                                &mut expr_imm_elems,
                                policy.max_instructions,
                                keep_data,
                            )?;
                            if over {
                                expr_limit_exceeded = true;
                            }
                            SegmentMode::Active { index: 0, offset }
                        }
                        1 => SegmentMode::Passive,
                        2 => {
                            let index = s.u32()?;
                            let (offset, over) = expression_bounded(
                                &mut s,
                                &mut expr_insns,
                                &mut expr_imm_elems,
                                policy.max_instructions,
                                keep_data,
                            )?;
                            if over {
                                expr_limit_exceeded = true;
                            }
                            SegmentMode::Active { index, offset }
                        }
                        _ => return Err(s.error("invalid data flags")),
                    };
                    let len = s.u32()? as usize;
                    let start = s.offset();
                    s.take(len)?;
                    if keep_data && !expr_limit_exceeded {
                        m.data.push(Data {
                            mode,
                            bytes: Span::new(start, s.offset()),
                        });
                    }
                }
            }
            12 => {
                s.feature(s.features().bulk_memory, "data count section")?;
                m.data_count = Some(s.u32()?);
            }
            13 => {
                let n = count(&mut s)?;
                for _ in 0..n {
                    let t = tag(&mut s)?;
                    if m.tags.len() >= policy.max_tags {
                        decl_limit_exceeded = true;
                    } else {
                        m.tags.push(t);
                    }
                }
            }
            _ => unreachable!(),
        };
        s.finish()?;
    }
    // Check cross-section structural `Decode` errors BEFORE policy `ResourceLimit` exits using
    // the exact scalar counters recorded even on over-limit modules, so malformed syntax always
    // precedes policy errors.
    if decoded_func_count != decoded_code_count {
        return Err(r.error("function/code count mismatch"));
    }
    if m.data_count.is_none() && saw_data_count_op {
        return Err(r.error("data count section required"));
    }
    if m.data_count
        .is_some_and(|n| n as usize != decoded_data_count)
    {
        return Err(r.error("data count mismatch"));
    }
    if rec_group_limit_exceeded {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "recursion group limit",
        ));
    }
    if local_limit_exceeded {
        return Err(Error::new(ErrorKind::ResourceLimit, "local limit"));
    }
    if total_local_limit_exceeded {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "module total local limit",
        ));
    }
    if code_insn_limit_exceeded
        || expr_limit_exceeded
        || imm_limit_exceeded
        || expr_insns > policy.max_instructions
        || expr_imm_elems > policy.max_instructions
    {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "module instruction limit",
        ));
    }
    if decl_limit_exceeded
        || m.types.len() > policy.max_types
        || m.functions.len() > policy.max_functions
        || m.tables.len() > policy.max_tables
        || m.memories.len() > policy.max_memories
        || m.globals.len() > policy.max_globals
        || m.tags.len() > policy.max_tags
        || m.elements.len() > policy.max_elements
        || m.data.len() > policy.max_data_segments
    {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "module declaration limit",
        ));
    }
    Ok(m)
}
struct ParsedBody {
    locals: Vec<ValType>,
    total_locals: u32,
    insn_count: usize,
    imm_elems: usize,
    dropped: bool,
    imm_limit_exceeded: bool,
    saw_data_count_op: bool,
    instructions: Vec<Instruction>,
    offsets: Vec<u32>,
    span: Span,
}

/// One function body of the code section, expanding bounded `locals` in parallel
/// once reserved against `worker_budget` so aggregate retained memory never
/// exceeds `policy` caps while full syntax checking still runs.
fn parse_function_body(
    mut b: Reader,
    policy: &ResourceLimits,
    mut worker_budget: Option<&mut DecodeWorkerBudget<'_>>,
    keep: bool,
) -> Result<ParsedBody> {
    let start = b.offset();
    let groups = count(&mut b)?;
    let mut locals = Vec::new();
    let mut total = 0u32;
    let mut reserved_locals = 0usize;
    let mut keep_locals = keep;
    let mut dropped_locals = !keep;
    for _ in 0..groups {
        let n = b.u32()?;
        let ty = val(&mut b)?;
        total = total
            .checked_add(n)
            .ok_or_else(|| b.error("local count overflow"))?;
        if n > 0 && keep_locals {
            if (total as usize) > policy.max_locals {
                if reserved_locals > 0
                    && let Some(wb) = worker_budget.as_deref_mut()
                {
                    wb.release_locals(reserved_locals);
                    reserved_locals = 0;
                }
                locals = Vec::new();
                keep_locals = false;
                dropped_locals = true;
            } else {
                let reserved = match worker_budget.as_deref_mut() {
                    Some(wb) => wb.try_reserve_locals(n as usize),
                    None => (total as usize) <= policy.max_total_locals,
                };
                if reserved && locals.try_reserve(n as usize).is_ok() {
                    reserved_locals += n as usize;
                    locals.resize(locals.len() + n as usize, ty);
                } else {
                    if reserved && let Some(wb) = worker_budget.as_deref_mut() {
                        wb.release_locals(reserved_locals + n as usize);
                        reserved_locals = 0;
                    } else if reserved_locals > 0
                        && let Some(wb) = worker_budget.as_deref_mut()
                    {
                        wb.release_locals(reserved_locals);
                        reserved_locals = 0;
                    }
                    locals = Vec::new();
                    keep_locals = false;
                    dropped_locals = true;
                }
            }
        }
    }
    let keep_body = keep && (total as usize) <= policy.max_locals;
    let parsed_body = body_expression_bounded(
        &mut b,
        worker_budget.as_deref_mut(),
        policy.max_instructions,
        keep_body,
    )?;
    b.finish()?;
    let dropped = dropped_locals || parsed_body.dropped;
    if dropped && !locals.is_empty() {
        if reserved_locals > 0
            && let Some(wb) = worker_budget
        {
            wb.release_locals(reserved_locals);
        }
        locals = Vec::new();
    }
    Ok(ParsedBody {
        locals,
        total_locals: total,
        insn_count: parsed_body.count,
        imm_elems: parsed_body.imm_elems,
        dropped,
        imm_limit_exceeded: parsed_body.imm_exceeded,
        saw_data_count_op: parsed_body.saw_data_count_op,
        instructions: parsed_body.instructions,
        offsets: parsed_body.offsets,
        span: Span::new(start, b.offset()),
    })
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
        let start = m.code[0].offsets[0] as usize;
        assert_eq!(&b[start..start + 2], &[0x41, 42]);
        assert_eq!(m.code[0].offsets.len(), m.code[0].instructions.len());
        assert_eq!(b[m.code[0].offsets[1] as usize], 0x0b);
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
            matches!(&expression(&mut r).unwrap()[0].immediate,Immediate::Resume(resume) if resume.ty == 0 && resume.tag.is_none() && resume.handlers==vec![(2,Some(3))])
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
        // Aggregate local cap (`max_total_locals`) rejects multi-body expansion without OOM,
        // while malformed bytecode in a later body still precedes policy errors.
        let ok_bodies = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[2, 0, 0]),
            (10, &[2, 4, 1, 5, 0x7f, 0x0b, 4, 1, 5, 0x7f, 0x0b]),
        ]);
        let tight_total = ResourceLimits {
            max_locals: 10,
            max_total_locals: 8,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&ok_bodies, &FeatureSet::default(), &tight_total)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        let malformed_second = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[2, 0, 0]),
            (10, &[2, 4, 1, 5, 0x7f, 0x0b, 4, 1, 5, 0x7f, 0x41]),
        ]);
        assert_eq!(
            decode(&malformed_second, &FeatureSet::default(), &tight_total)
                .unwrap_err()
                .kind,
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
    #[test]
    fn t163_bounded_decode_all_sections_and_skip_immediates() {
        // Zero-count local groups do not accumulate or exceed local limits.
        let mut body = vec![4, 0, 0x7f, 0, 0x7e, 0, 0x7d, 0, 0x7c, 0x0b];
        let mut code = vec![1, body.len() as u8];
        code.append(&mut body);
        let zero_locals = module(&[(1, &[1, 0x60, 0, 0]), (3, &[1, 0]), (10, &code)]);
        let m = decode(
            &zero_locals,
            &FeatureSet::default(),
            &ResourceLimits {
                max_locals: 0,
                max_total_locals: 0,
                ..ResourceLimits::default()
            },
        )
        .unwrap();
        assert!(m.code[0].locals.is_empty());

        // Over-limit body with heap-backed `br_table` & `select` immediates still reports
        // `ResourceLimit` when well-formed and `Decode` when malformed, without retaining.
        let over_body = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (
                10,
                &[1, 12, 0, 0x01, 0x01, 0x0e, 2, 0, 0, 0, 0x1c, 1, 0x7f, 0x0b],
            ),
        ]);
        let tight_insn = ResourceLimits {
            max_instructions: 2,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&over_body, &FeatureSet::default(), &tight_insn)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        let malformed_over_body = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (
                10,
                &[1, 12, 0, 0x01, 0x01, 0x0e, 2, 0, 0, 0, 0x1c, 1, 0x7f, 0xff],
            ),
        ]);
        assert_eq!(
            decode(&malformed_over_body, &FeatureSet::default(), &tight_insn)
                .unwrap_err()
                .kind,
            ErrorKind::Decode
        );

        // Over-limit types & legacy element entries reject with `ResourceLimit`, while
        // malformed syntax in a later section still reports `Decode`.
        let many_types = module(&[(1, &[3, 0x60, 0, 0, 0x60, 0, 0, 0x60, 0, 0])]);
        let tight_types = ResourceLimits {
            max_types: 1,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&many_types, &FeatureSet::default(), &tight_types)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        let many_types_then_bad_mem = module(&[
            (1, &[3, 0x60, 0, 0, 0x60, 0, 0, 0x60, 0, 0]),
            (5, &[1, 0xff]),
        ]);
        assert_eq!(
            decode(
                &many_types_then_bad_mem,
                &FeatureSet::default(),
                &tight_types
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );
        // Legacy element items charge 2 instructions & 4 immediate elements each, and skip
        // allocation/retention upfront (`keep_items == false`) when over limit while still
        // preserving malformed-precedes-policy ordering.
        let elem_over = module(&[(9, &[1, 1, 0, 3, 0, 0, 0])]);
        assert_eq!(
            decode(
                &elem_over,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 4,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        let mut legacy_many = vec![1, 1, 0, 50];
        // 50 legacy func indices (well over `max_instructions: 32` -> `keep_items == false`).
        legacy_many.extend(std::iter::repeat_n(0u8, 50));
        let legacy_many_ok = module(&[(9, &legacy_many)]);
        assert_eq!(
            decode(
                &legacy_many_ok,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 32,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(
            decode(
                &legacy_many_ok,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_table_elements: 10,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        // Truncate the last index byte to an unterminated LEB128 so malformed syntax still wins.
        *legacy_many.last_mut().unwrap() = 0x80;
        let legacy_many_bad = module(&[(9, &legacy_many)]);
        assert_eq!(
            decode(
                &legacy_many_bad,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 32,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );

        // Parallel decode right at `max_instructions` boundary preserves all bodies cleanly.
        let mut sec3 = vec![4u8];
        sec3.extend([0, 0, 0, 0]);
        let mut sec10 = vec![4u8];
        for _ in 0..4 {
            sec10.extend([3, 0, 0x01, 0x0b]); // 2 instructions each -> 8 total
        }
        let exact_par = module(&[(1, &[1, 0x60, 0, 0]), (3, &sec3), (10, &sec10)]);
        let m = decode(
            &exact_par,
            &FeatureSet::default(),
            &ResourceLimits {
                max_instructions: 8,
                compile_threads: 4,
                ..ResourceLimits::default()
            },
        )
        .unwrap();
        assert_eq!(m.code.len(), 4);
        assert!(m.code.iter().all(|b| b.instructions.len() == 2));

        // Cumulative variable-length immediate elements across multiple instructions are capped
        // against `max_instructions` (3x `select [i32, i32, i32]` = 9 > 8), while malformed syntax
        // in a trailing byte still reports `Decode`.
        let cum_imm_ok = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (
                10,
                &[
                    1, 17, 0, // 0 locals
                    0x1c, 3, 0x7f, 0x7f, 0x7f, // 3 types
                    0x1c, 3, 0x7f, 0x7f, 0x7f, // +3 = 6
                    0x1c, 3, 0x7f, 0x7f, 0x7f, // +3 = 9 > 8
                    0x0b,
                ],
            ),
        ]);
        assert_eq!(
            decode(
                &cum_imm_ok,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 8,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        let cum_imm_bad = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (
                10,
                &[
                    1, 17, 0, 0x1c, 3, 0x7f, 0x7f, 0x7f, 0x1c, 3, 0x7f, 0x7f, 0x7f, 0x1c, 3, 0x7f,
                    0x7f, 0x7f, 0xff,
                ],
            ),
        ]);
        assert_eq!(
            decode(
                &cum_imm_bad,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 8,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );

        // Over-limit `max_rec_group_size` is deferred so malformed syntax in a later section
        // still reports `Decode`, while well-formed modules report `ResourceLimit`.
        let rec_over = module(&[(1, &[1, 0x4e, 3, 0x60, 0, 0, 0x60, 0, 0, 0x60, 0, 0])]);
        let tight_rec = ResourceLimits {
            max_rec_group_size: 1,
            ..ResourceLimits::default()
        };
        assert_eq!(
            decode(&rec_over, &FeatureSet::default(), &tight_rec)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        let rec_over_then_bad = module(&[
            (1, &[1, 0x4e, 3, 0x60, 0, 0, 0x60, 0, 0, 0x60, 0, 0]),
            (5, &[1, 0xff]),
        ]);
        assert_eq!(
            decode(&rec_over_then_bad, &FeatureSet::default(), &tight_rec)
                .unwrap_err()
                .kind,
            ErrorKind::Decode
        );

        // Section 10 -> Section 11 handoff: combined instructions AND combined immediate elements
        // across Code + Data must enforce `max_instructions` without resetting counters.
        let code_plus_data_insns = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 5, 0, 0x01, 0x01, 0x01, 0x0b]), // 4 insns
            (11, &[3, 1, 0, 1, 0, 1, 0]),             // +3 data headers = 7 > 6
        ]);
        assert_eq!(
            decode(
                &code_plus_data_insns,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 6,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        let code_plus_data_imm = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 7, 0, 0x1c, 3, 0x7f, 0x7f, 0x7f, 0x0b]), // 2 insns, 3 imm_elems
            (11, &[1, 0, 0x1c, 3, 0x7f, 0x7f, 0x7f, 0x0b, 0]), // +1 header +2 insns (5<=5), +3 imm_elems = 6 > 5
        ]);
        assert_eq!(
            decode(
                &code_plus_data_imm,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 5,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );

        // Cross-section structural `Decode` checks ALWAYS precede `ResourceLimit` even when
        // retention was dropped due to over-limit declarations or instructions:
        // (a) function/code count mismatch on over-limit function section;
        let func_code_mismatch_over = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[3, 0, 0, 0]),
            (10, &[2, 2, 0, 0x0b, 2, 0, 0x0b]),
        ]);
        assert_eq!(
            decode(
                &func_code_mismatch_over,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_functions: 1,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );
        // (b) missing data count section for `data.drop` (`0xfc 0x09`) inside an over-limit body;
        let missing_dc_over = module(&[
            (1, &[1, 0x60, 0, 0]),
            (3, &[1, 0]),
            (10, &[1, 8, 0, 0x01, 0x01, 0x01, 0xfc, 9, 0, 0x0b]),
        ]);
        assert_eq!(
            decode(
                &missing_dc_over,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_instructions: 2,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );
        // (c) data count mismatch on an over-limit data section.
        let dc_mismatch_over = module(&[(12, &[5]), (11, &[3, 1, 0, 1, 0, 1, 0])]);
        assert_eq!(
            decode(
                &dc_mismatch_over,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_data_segments: 1,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::Decode
        );
        // (d) over-limit elements / data segments reject with `ResourceLimit` when well-formed.
        let many_data = module(&[(11, &[3, 1, 0, 1, 0, 1, 0])]);
        assert_eq!(
            decode(
                &many_data,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_data_segments: 2,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
        let many_elems = module(&[(9, &[3, 1, 0, 0, 1, 0, 0, 1, 0, 0])]);
        assert_eq!(
            decode(
                &many_elems,
                &FeatureSet::default(),
                &ResourceLimits {
                    max_elements: 2,
                    ..ResourceLimits::default()
                },
            )
            .unwrap_err()
            .kind,
            ErrorKind::ResourceLimit
        );
    }
}
