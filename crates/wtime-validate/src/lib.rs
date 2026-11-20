#![forbid(unsafe_code)]
mod atomics;
mod continuations;
mod exceptions;
mod gc;
mod types;
use std::rc::Rc;
use types::value_subtype;
// Validation is the sole constructor of executable modules.
use std::collections::HashSet;
pub use wtime_decode::*;
use wtime_types::*;
/// An immutable module certified under a retained feature and resource policy.
///
/// The policy cannot be relabeled after validation.
/// ```compile_fail
/// use wtime_validate::validate;
/// use wtime_types::{FeatureSet, ResourceLimits};
/// let module = validate(b"\0asm\x01\0\0\0", &FeatureSet::default(), &ResourceLimits::default()).unwrap();
/// module.features().simd = false;
/// ```
/// ```compile_fail
/// use wtime_validate::validate;
/// use wtime_types::{FeatureSet, ResourceLimits};
/// let module = validate(b"\0asm\x01\0\0\0", &FeatureSet::default(), &ResourceLimits::default()).unwrap();
/// module.decoded().functions.clear();
/// ```
/// ```compile_fail
/// use wtime_validate::{decode, ValidatedModule};
/// use wtime_types::{FeatureSet, ResourceLimits};
/// let features = FeatureSet::default();
/// let limits = ResourceLimits::default();
/// let decoded = decode(b"\0asm\x01\0\0\0", &features, &limits).unwrap();
/// let unchecked = ValidatedModule { decoded, features, limits };
/// ```
/// Bounds observed by the existing validation pass, relative to one body.
/// Unreachable code uses the validator's polymorphic stack; these are allocation
/// hints, not certificates for removing runtime checks across calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunctionBounds {
    pub stack_peak: usize,
    pub maximum_nesting: usize,
}
#[derive(Clone, Debug)]
pub struct ValidatedModule {
    decoded: DecodedModule,
    function_bounds: Vec<FunctionBounds>,
    features: FeatureSet,
    limits: ResourceLimits,
}
impl ValidatedModule {
    pub fn function_bounds_bytes(&self) -> usize {
        self.function_bounds.capacity() * std::mem::size_of::<FunctionBounds>()
    }
    pub fn function_bounds(&self) -> &[FunctionBounds] {
        &self.function_bounds
    }
    pub fn decoded(&self) -> &DecodedModule {
        &self.decoded
    }
    pub fn features(&self) -> &FeatureSet {
        &self.features
    }
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}
fn invalid(s: impl Into<String>) -> Error {
    Error::new(ErrorKind::Validation, s)
}
fn budget(b: bool, s: &str) -> Result<()> {
    if b {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::ResourceLimit, s))
    }
}
#[inline]
fn check(b: bool, s: &str) -> Result<()> {
    if b { Ok(()) } else { Err(invalid(s)) }
}
#[inline]
fn index(i: &Instruction) -> Result<usize> {
    if let Immediate::Index(x) = i.immediate {
        Ok(x as usize)
    } else {
        Err(invalid("expected index"))
    }
}

/// The most frequent instructions in reachable code whose operands have
/// exactly the expected types. Returns false, changing nothing, for anything
/// else, which then takes the general path with all its checks and errors.
#[inline]
fn fast_instruction(
    s: &mut Stack,
    locals: &[ValType],
    globals: &[GlobalType],
    memories: &[MemoryType],
    i: &Instruction,
) -> bool {
    let Some(c) = s.controls.last() else { return false };
    if c.unreachable || !s.unknown.is_empty() {
        return false;
    }
    let floor = c.height;
    let n = s.values.len();
    // Two operands of type `t` above the control frame's floor.
    let two = |s: &Stack, t: ValType| n >= floor + 2 && s.values[n - 1] == t && s.values[n - 2] == t;
    match (i.opcode, &i.immediate) {
        (0x20..=0x22, Immediate::Index(x)) => {
            let x = *x as usize;
            let (Some(&t), Some(true)) = (locals.get(x), s.initialized.get(x)) else { return false };
            match i.opcode {
                0x20 => s.values.push(t),
                0x21 if n > floor && s.values[n - 1] == t => s.values.truncate(n - 1),
                0x22 if n > floor && s.values[n - 1] == t => {}
                _ => return false,
            }
            true
        }
        (0x41, Immediate::I32(_)) => {
            s.values.push(ValType::I32);
            true
        }
        (0x42, Immediate::I64(_)) => {
            s.values.push(ValType::I64);
            true
        }
        (0x46..=0x4f | 0x6a..=0x78, _) if two(s, ValType::I32) => {
            s.values.truncate(n - 1);
            true
        }
        (0x7c..=0x8a, _) if two(s, ValType::I64) => {
            s.values.truncate(n - 1);
            true
        }
        (0x51..=0x5a, _) if two(s, ValType::I64) => {
            s.values.truncate(n - 2);
            s.values.push(ValType::I32);
            true
        }
        (0x23, Immediate::Index(x)) => {
            let Some(t) = globals.get(*x as usize) else { return false };
            s.values.push(t.value);
            true
        }
        (0x24, Immediate::Index(x)) => {
            let Some(t) = globals.get(*x as usize) else { return false };
            if !(t.mutable && n > floor && s.values[n - 1] == t.value) {
                return false;
            }
            s.values.truncate(n - 1);
            true
        }
        (0x28..=0x3e, &Immediate::MemArg { align, memory, offset }) => {
            let Some(m) = memories.get(memory as usize) else { return false };
            let (t, size) = match i.opcode {
                0x28 | 0x36 => (ValType::I32, 2),
                0x29 | 0x37 => (ValType::I64, 3),
                0x2a | 0x38 => (ValType::F32, 2),
                0x2b | 0x39 => (ValType::F64, 3),
                0x2c | 0x2d | 0x3a => (ValType::I32, 0),
                0x2e | 0x2f | 0x3b => (ValType::I32, 1),
                0x30 | 0x31 | 0x3c => (ValType::I64, 0),
                0x32 | 0x33 | 0x3d => (ValType::I64, 1),
                _ => (ValType::I64, 2),
            };
            let address = m.index_type.val_type();
            if align > size || (m.index_type != IndexType::I64 && offset > u32::MAX as u64) {
                return false;
            }
            if i.opcode >= 0x36 {
                if !(n >= floor + 2 && s.values[n - 1] == t && s.values[n - 2] == address) {
                    return false;
                }
                s.values.truncate(n - 2);
            } else {
                if !(n > floor && s.values[n - 1] == address) {
                    return false;
                }
                s.values[n - 1] = t;
            }
            true
        }
        (0x1a, _) if n > floor => {
            s.values.truncate(n - 1);
            true
        }
        // Unary operators: (operand, result).
        (0x45 | 0x50 | 0xa7 | 0xac | 0xad, _) => {
            let (a, r) = match i.opcode {
                0x45 => (ValType::I32, ValType::I32),
                0x50 | 0xa7 => (ValType::I64, ValType::I32),
                _ => (ValType::I32, ValType::I64),
            };
            if !(n > floor && s.values[n - 1] == a) {
                return false;
            }
            s.values[n - 1] = r;
            true
        }
        _ => false,
    }
}
struct Context<'a> {
    tags: Vec<FuncType>,
    types: Rc<types::TypeEnvironment>,
    m: &'a DecodedModule,
    funcs: Vec<FuncType>,
    globals: Vec<GlobalType>,
    tables: Vec<TableType>,
    memories: Vec<MemoryType>,
    imported_globals: usize,
    features: &'a FeatureSet,
    limits: &'a ResourceLimits,
}
#[derive(Clone)]
struct Control {
    kind: u16,
    height: usize,
    params: Vec<ValType>,
    results: Vec<ValType>,
    unreachable: bool,
    has_else: bool,
    initialized: usize,
}
struct Stack {
    initialization_changes: Vec<usize>,
    unknown: HashSet<usize>,
    initialized: Vec<bool>,
    types: Rc<types::TypeEnvironment>,
    values: Vec<ValType>,
    controls: Vec<Control>,
}
impl Stack {
    fn restore_locals(&mut self, n: usize) {
        while self.initialization_changes.len() > n {
            let i = self.initialization_changes.pop().unwrap();
            self.initialized[i] = false;
        }
    }
    fn initialize(&mut self, n: usize) {
        if let Some(v) = self.initialized.get_mut(n)
            && !*v
        {
            *v = true;
            self.initialization_changes.push(n);
        }
    }
    fn pop(&mut self, t: ValType) -> Result<()> {
        let c = self
            .controls
            .last()
            .ok_or_else(|| invalid("instruction after final end"))?;
        if self.values.len() == c.height && c.unreachable {
            return Ok(());
        }
        let v = self
            .values
            .pop()
            .ok_or_else(|| invalid("operand stack underflow"))?;
        // Unknown operands only arise in unreachable code; skip the hash
        // lookup when there are none.
        let unknown = !self.unknown.is_empty() && self.unknown.remove(&self.values.len());
        check(
            self.values.len() >= c.height && (unknown || value_subtype(v, t, &self.types)),
            "operand type mismatch",
        )
    }
    fn any(&mut self) -> Result<Option<ValType>> {
        let c = self
            .controls
            .last()
            .ok_or_else(|| invalid("missing control frame"))?;
        if self.values.len() == c.height && c.unreachable {
            Ok(None)
        } else {
            check(self.values.len() > c.height, "operand stack underflow")?;
            let v = self.values.pop();
            if !self.unknown.is_empty() && self.unknown.remove(&self.values.len()) {
                Ok(None)
            } else {
                Ok(v)
            }
        }
    }
    fn pop_all(&mut self, ts: &[ValType]) -> Result<()> {
        for t in ts.iter().rev() {
            self.pop(*t)?
        }
        Ok(())
    }
    fn op(&mut self, inputs: &[ValType], outputs: &[ValType]) -> Result<()> {
        self.pop_all(inputs)?;
        self.values.extend_from_slice(outputs);
        Ok(())
    }
    fn dead(&mut self) {
        let c = self.controls.last_mut().unwrap();
        self.values.truncate(c.height);
        if !self.unknown.is_empty() {
            self.unknown.retain(|i| *i < c.height);
        }
        c.unreachable = true;
    }
    fn label(&self, n: usize) -> Result<Vec<ValType>> {
        let c = self
            .controls
            .get(
                self.controls
                    .len()
                    .checked_sub(n + 1)
                    .ok_or_else(|| invalid("invalid branch depth"))?,
            )
            .ok_or_else(|| invalid("invalid branch depth"))?;
        Ok(if c.kind == 3 {
            c.params.clone()
        } else {
            c.results.clone()
        })
    }
}
pub fn validate(
    bytes: &[u8],
    features: &FeatureSet,
    limits: &ResourceLimits,
) -> Result<ValidatedModule> {
    features.validate()?;
    let m = decode(bytes, features, limits)?;

    let mut c = Context {
        tags: vec![],
        types: Rc::new(types::TypeEnvironment::new(
            &m.type_groups,
            limits.max_type_work,
        )),
        m: &m,
        funcs: vec![],
        globals: vec![],
        tables: vec![],
        memories: vec![],
        imported_globals: 0,
        features,
        limits,
    };
    let mut function_bounds = Vec::new();
    function_bounds
        .try_reserve_exact(m.code.len())
        .map_err(|_| {
            Error::new(
                ErrorKind::ResourceLimit,
                "function bounds allocation failed",
            )
        })?;
    let validation = (|| -> Result<()> {
        types::validate_types(&c)?;
        for t in &m.types {
            c.enabled(features.multi_value || t.results.len() <= 1)?;
            for v in t.params.iter().chain(&t.results) {
                c.valtype(*v)?
            }
        }
        for im in &m.imports {
            match &im.kind {
                ImportKind::Func(t) => c.funcs.push(c.ty(t.0 as usize)?.clone()),
                ImportKind::ExactFunc(t) => {
                    c.enabled(features.custom_descriptors)?;
                    c.funcs.push(c.ty(t.0 as usize)?.clone())
                }
                ImportKind::Global(t) => {
                    c.enabled(!t.mutable || features.mutable_globals)?;
                    c.globals.push(*t);
                    c.imported_globals += 1
                }
                ImportKind::Table(t) => c.tables.push(*t),
                ImportKind::Memory(t) => c.memories.push(*t),
                ImportKind::Tag(t) => c.add_tag(*t)?,
            }
        }
        for t in &m.tags {
            c.add_tag(*t)?;
        }
        for t in &m.functions {
            c.funcs.push(c.ty(t.0 as usize)?.clone())
        }
        c.tables.extend(&m.tables);
        c.memories.extend(&m.memories);
        c.enabled(features.multi_memory || c.memories.len() <= 1)?;
        c.enabled(features.reference_types || c.tables.len() <= 1)?;
        budget(
            c.funcs.len() <= limits.max_functions
                && c.globals.len() + m.globals.len() <= limits.max_globals
                && c.tables.len() <= limits.max_tables
                && c.memories.len() <= limits.max_memories,
            "imported declaration limits exceeded",
        )?;
        for t in &c.memories {
            c.enabled(t.index_type == IndexType::I32 || features.memory64)?;
            c.enabled(!t.shared || features.threads)?;
            c.enabled(t.page_size_log2 == 16 || features.custom_page_sizes)?;
            check(t.limits.is_valid(), "invalid memory limits")?;
            check(
                t.page_size_log2 == 0 || t.page_size_log2 == 16,
                "invalid page size",
            )?;
            let max = if t.index_type == IndexType::I32 {
                (1u64 << (32 - t.page_size_log2)).min(u32::MAX as u64)
            } else {
                if t.page_size_log2 == 0 {
                    u64::MAX
                } else {
                    1u64 << (64 - t.page_size_log2)
                }
            };
            check(
                t.limits.min <= max && t.limits.max.is_none_or(|v| v <= max),
                "memory exceeds address space",
            )?;
            check(
                !t.shared || t.limits.max.is_some(),
                "shared memory requires maximum",
            )?;
        }
        for t in &c.tables {
            c.enabled(t.index_type == IndexType::I32 || features.memory64)?;
            check(
                t.index_type != IndexType::I32
                    || (t.limits.min <= u32::MAX as u64
                        && t.limits.max.is_none_or(|n| n <= u32::MAX as u64)),
                "table limits overflow",
            )?;
            check(t.limits.is_valid(), "invalid table limits")?;
            if t.element != RefType::FUNCREF {
                c.valtype(ValType::Ref(t.element))?;
            }
        }
        for t in &c.globals {
            c.valtype(t.value)?;
        }
        for (n, init) in m.table_inits.iter().enumerate() {
            if let Some(init) = init {
                c.constant(init, ValType::Ref(m.tables[n].element))?
            } else {
                check(
                    m.tables[n].element.nullable,
                    "nonnullable table requires initializer",
                )?;
            }
        }
        for g in &m.globals {
            c.valtype(g.ty.value)?;
            c.constant(&g.init, g.ty.value)?;
            c.globals.push(g.ty)
        }
        let mut names = HashSet::new();
        for e in &m.exports {
            if e.kind == ExternalKind::Global
                && let Some(g) = c.globals.get(e.index as usize)
            {
                c.enabled(!g.mutable || features.mutable_globals)?;
            }
            check(names.insert(&e.name), "duplicate export")?;
            let n = match e.kind {
                ExternalKind::Func => c.funcs.len(),
                ExternalKind::Table => c.tables.len(),
                ExternalKind::Memory => c.memories.len(),
                ExternalKind::Global => c.globals.len(),
                ExternalKind::Tag => c.tags.len(),
            };
            check((e.index as usize) < n, "export index out of bounds")?;
        }
        if let Some(s) = m.start {
            let t = c
                .funcs
                .get(s.0 as usize)
                .ok_or_else(|| invalid("start index out of bounds"))?;
            check(
                t.params.is_empty() && t.results.is_empty(),
                "start function must have empty signature",
            )?
        }
        c.enabled(m.data_count.is_none() || features.bulk_memory)?;
        for d in &m.data {
            c.enabled(matches!(d.mode, SegmentMode::Active { .. }) || features.bulk_memory)?;
            if let SegmentMode::Active { index, offset } = &d.mode {
                let t = c
                    .memories
                    .get(*index as usize)
                    .ok_or_else(|| invalid("data memory index out of bounds"))?;
                c.constant(offset, t.index_type.val_type())?
            }
        }
        for e in &m.elements {
            if features.reference_types {
                c.valtype(ValType::Ref(e.ty))?;
            }
            c.enabled(matches!(e.mode, SegmentMode::Active { .. }) || features.bulk_memory)?;
            if let SegmentMode::Active { index, offset } = &e.mode {
                let t = c
                    .tables
                    .get(*index as usize)
                    .ok_or_else(|| invalid("element table index out of bounds"))?;
                check(
                    value_subtype(ValType::Ref(e.ty), ValType::Ref(t.element), &c.types),
                    "element table type mismatch",
                )?;
                c.constant(offset, t.index_type.val_type())?
            }
            for x in &e.init {
                if !features.reference_types
                    && x.len() == 2
                    && x[0].opcode == 0xd2
                    && x[1].opcode == 0x0b
                {
                    check(
                        index(&x[0])? < c.funcs.len(),
                        "element function index out of bounds",
                    )?;
                } else {
                    c.constant(x, ValType::Ref(e.ty))?;
                }
            }
        }
        check(
            m.functions.len() == m.code.len(),
            "function/code count mismatch",
        )?;
        for (t, b) in m.functions.iter().zip(&m.code) {
            function_bounds.push(c.body(c.ty(t.0 as usize)?, b)?)
        }
        Ok(())
    })();
    if c.types.exhausted() {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            "type validation work budget exhausted",
        ));
    }
    validation?;
    Ok(ValidatedModule {
        decoded: m,
        function_bounds,
        features: *features,
        limits: limits.clone(),
    })
}
impl Context<'_> {
    fn ty(&self, n: usize) -> Result<&FuncType> {
        check(
            self.types
                .get(n)
                .is_some_and(|t| matches!(t.composite, CompositeType::Func(_))),
            "expected function type",
        )?;
        self.m
            .types
            .get(n)
            .ok_or_else(|| invalid("type index out of bounds"))
    }
    fn valtype(&self, v: ValType) -> Result<()> {
        if v == ValType::V128 {
            self.enabled(self.features.simd)?;
        }
        if let ValType::Ref(r) = v {
            self.enabled(self.features.reference_types)?;
            match r.heap {
                HeapType::Defined(i) | HeapType::ExactDefined(i) => {
                    check(
                        (i.0 as usize) < self.types.len(),
                        "heap type index out of bounds",
                    )?;
                    self.enabled(self.features.function_references || self.features.gc)?;
                    if matches!(r.heap, HeapType::ExactDefined(_)) {
                        self.enabled(self.features.custom_descriptors)?
                    }
                }
                HeapType::Func | HeapType::Extern => {}
                HeapType::Exn | HeapType::NoExn => {
                    self.enabled(self.features.exception_handling)?
                }
                HeapType::Cont | HeapType::NoCont => self.enabled(self.features.stack_switching)?,
                _ => self.enabled(self.features.gc)?,
            }
        }
        Ok(())
    }

    fn constant(&self, is: &[Instruction], expected: ValType) -> Result<()> {
        let mut s = Stack {
            initialization_changes: vec![],
            unknown: HashSet::new(),
            initialized: vec![],
            types: self.types.clone(),
            values: vec![],
            controls: vec![Control {
                kind: 0,
                height: 0,
                params: vec![],
                results: vec![expected],
                unreachable: false,
                has_else: false,
                initialized: 0,
            }],
        };
        for i in is {
            match i.opcode {
                0x41..=0x44
                | 0xd0
                | 0xd2
                | 0xfd0c
                | 0xfb00
                | 0xfb01
                | 0xfb06..=0xfb08
                | 0xfb1c
                | 0xfb1a
                | 0xfb1b
                | 0xfb20
                | 0xfb21
                | 0x0b => {}
                0x23 => {
                    let n = index(i)?;
                    check(
                        n < self.globals.len()
                            && (n < self.imported_globals || self.features.extended_const)
                            && !self.globals[n].mutable,
                        "constant global must be immutable import",
                    )?
                }
                0x6a | 0x6b | 0x6c | 0x7c | 0x7d | 0x7e => {
                    self.enabled(self.features.extended_const)?;
                }
                _ => return Err(invalid("invalid constant expression instruction")),
            }
            self.instruction(&mut s, &[], i)?;
        }
        check(s.controls.is_empty(), "unterminated constant expression")
    }
    fn body(&self, t: &FuncType, b: &FunctionBody) -> Result<FunctionBounds> {
        let mut locals = t.params.clone();
        locals.extend(&b.locals);
        budget(locals.len() <= self.limits.max_locals, "too many locals")?;
        for v in &locals {
            self.valtype(*v)?
        }
        let mut s = Stack {
            initialization_changes: vec![],
            unknown: HashSet::new(),
            initialized: vec![],
            types: self.types.clone(),
            values: vec![],
            controls: vec![Control {
                kind: 0,
                height: 0,
                params: vec![],
                results: t.results.clone(),
                unreachable: false,
                has_else: false,
                initialized: 0,
            }],
        };
        s.initialized = locals
            .iter()
            .enumerate()
            .map(|(n, v)| {
                n < t.params.len()
                    || !matches!(
                        v,
                        ValType::Ref(RefType {
                            nullable: false,
                            ..
                        })
                    )
            })
            .collect();
        let mut bounds = FunctionBounds {
            stack_peak: 0,
            maximum_nesting: 1,
        };
        for (n, i) in b.instructions.iter().enumerate() {
            // Absolute module offset for error reports; a body built without
            // offsets reports its own start.
            let at = b.offsets.get(n).map_or(b.span.start, |&offset| offset as usize);
            if i.opcode == 0xd2 {
                let n = index(i)? as u32;
                check(
                    self.m
                        .exports
                        .iter()
                        .any(|e| e.kind == ExternalKind::Func && e.index == n)
                        || self
                            .m
                            .elements
                            .iter()
                            .flat_map(|e| e.init.iter().flatten())
                            .chain(self.m.globals.iter().flat_map(|g| g.init.iter()))
                            .chain(
                                self.m
                                    .table_inits
                                    .iter()
                                    .filter_map(|v| v.as_ref())
                                    .flatten(),
                            )
                            .any(|j| {
                                j.opcode == 0xd2
                                    && matches!(j.immediate,Immediate::Index(x) if x==n)
                            }),
                    "undeclared function reference",
                )?;
            }
            if !fast_instruction(&mut s, &locals, &self.globals, &self.memories, i) {
                self.instruction(&mut s, &locals, i)
                    .map_err(|e| e.at(at))?;
            }
            bounds.stack_peak = bounds.stack_peak.max(s.values.len());
            bounds.maximum_nesting = bounds.maximum_nesting.max(s.controls.len());
            budget(
                s.values.len() <= self.limits.max_stack_height
                    && s.controls.len() <= self.limits.max_stack_height,
                "validation stack limit exceeded",
            )?;
        }
        check(s.controls.is_empty(), "unterminated function")?;
        Ok(bounds)
    }
    fn enabled(&self, b: bool) -> Result<()> {
        if b {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::DisabledFeature,
                "instruction feature disabled",
            ))
        }
    }
    fn instruction(&self, s: &mut Stack, locals: &[ValType], i: &Instruction) -> Result<()> {
        use ValType::*;
        check(!s.controls.is_empty(), "instructions after end")?;
        match i.opcode {
            0x12 | 0x13 => self.enabled(self.features.tail_call)?,
            0x14 | 0x15 | 0xd4..=0xd6 => self.enabled(self.features.function_references)?,
            0x1c | 0x25 | 0x26 | 0xd0..=0xd2 => self.enabled(self.features.reference_types)?,
            0xc0..=0xc4 => self.enabled(self.features.sign_extension)?,
            0xfc00..=0xfc07 => self.enabled(self.features.saturating_float_to_int)?,
            0xfc08..=0xfc11 => self.enabled(self.features.bulk_memory)?,
            _ => {}
        }
        match i.opcode {
            0 => s.dead(),
            1 => {}
            2..=4 => {
                if i.opcode == 4 {
                    s.pop(I32)?
                }
                let (params, results) = match i.immediate {
                    Immediate::BlockType(t) => (vec![], vec![t]),
                    Immediate::Block(-64) => (vec![], vec![]),
                    Immediate::Block(-1) => (vec![], vec![I32]),
                    Immediate::Block(-2) => (vec![], vec![I64]),
                    Immediate::Block(-3) => (vec![], vec![F32]),
                    Immediate::Block(-4) => (vec![], vec![F64]),
                    Immediate::Block(-5) => (vec![], vec![V128]),
                    Immediate::Block(-16) => (vec![], vec![ValType::FUNCREF]),
                    Immediate::Block(-17) => (vec![], vec![ValType::EXTERNREF]),
                    Immediate::Block(n @ (-24..=-18 | -15..=-11)) => {
                        let h = match n {
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
                        (vec![], vec![Ref(RefType::new(true, h))])
                    }
                    Immediate::Block(n) if n >= 0 => {
                        self.enabled(self.features.multi_value)?;
                        let t = self.ty(n as usize)?;
                        (t.params.clone(), t.results.clone())
                    }
                    _ => return Err(invalid("invalid block type")),
                };
                for v in params.iter().chain(&results) {
                    self.valtype(*v)?;
                }
                s.pop_all(&params)?;
                let h = s.values.len();
                s.values.extend(&params);
                s.controls.push(Control {
                    kind: i.opcode,
                    height: h,
                    params,
                    results,
                    unreachable: false,
                    has_else: false,
                    initialized: s.initialization_changes.len(),
                })
            }
            5 => {
                let c = s.controls.last().unwrap().clone();
                check(
                    c.kind == 4 && !c.has_else,
                    "else outside if or duplicate else",
                )?;
                s.pop_all(&c.results)?;
                check(s.values.len() == c.height, "extra stack operands at else")?;
                s.values.extend(&c.params);
                s.restore_locals(c.initialized);
                let c = s.controls.last_mut().unwrap();
                c.unreachable = false;
                c.has_else = true;
            }
            0x0b => {
                let c = s.controls.last().unwrap().clone();
                if c.kind == 4 && !c.has_else {
                    check(c.params == c.results, "if without else type mismatch")?
                }
                s.pop_all(&c.results)?;
                check(s.values.len() == c.height, "extra operands at end")?;
                s.restore_locals(c.initialized);
                s.controls.pop();
                s.values.extend(c.results)
            }
            0x0c | 0x0d => {
                let ts = s.label(index(i)?)?;
                if i.opcode == 0x0d {
                    s.pop(I32)?;
                    s.pop_all(&ts)?;
                    s.values.extend(ts)
                } else {
                    s.pop_all(&ts)?;
                    s.dead()
                }
            }
            0x0e => {
                if let Immediate::BrTable(ref ns, d) = i.immediate {
                    s.pop(I32)?;
                    let ts = s.label(d as usize)?;
                    for n in ns {
                        let target = s.label(*n as usize)?;
                        check(target.len() == ts.len(), "branch table arity mismatch")?;
                        let values = s.values.clone();
                        let unknown = s.unknown.clone();
                        s.pop_all(&target)?;
                        s.values = values;
                        s.unknown = unknown;
                    }
                    s.pop_all(&ts)?;
                    s.dead()
                } else {
                    return Err(invalid("invalid br_table"));
                }
            }
            0x0f => {
                let ts = s.controls[0].results.clone();
                s.pop_all(&ts)?;
                s.dead()
            }
            0x10 | 0x12 => {
                let t = self
                    .funcs
                    .get(index(i)?)
                    .ok_or_else(|| invalid("function index out of bounds"))?;
                s.op(&t.params, &t.results)?;
                if i.opcode == 0x12 {
                    check(
                        t.results.len() == s.controls[0].results.len()
                            && t.results
                                .iter()
                                .zip(&s.controls[0].results)
                                .all(|(a, b)| value_subtype(*a, *b, &self.types)),
                        "tail call result mismatch",
                    )?;
                    s.dead()
                }
            }
            0x11 | 0x13 => {
                let (t, n) = if let Immediate::Pair(t, n) = i.immediate {
                    (self.ty(t as usize)?, n)
                } else {
                    return Err(invalid("invalid indirect call"));
                };
                let table = self
                    .tables
                    .get(n as usize)
                    .ok_or_else(|| invalid("table index out of bounds"))?;
                check(
                    value_subtype(Ref(table.element), ValType::FUNCREF, &self.types),
                    "call_indirect requires funcref table",
                )?;
                s.pop(table.index_type.val_type())?;
                s.op(&t.params, &t.results)?;
                if i.opcode == 0x13 {
                    check(
                        t.results.len() == s.controls[0].results.len()
                            && t.results
                                .iter()
                                .zip(&s.controls[0].results)
                                .all(|(a, b)| value_subtype(*a, *b, &self.types)),
                        "tail call result mismatch",
                    )?;
                    s.dead()
                }
            }
            0x14 | 0x15 => {
                let n = index(i)?;
                let t = self.ty(n)?;
                s.pop(Ref(RefType::new(
                    true,
                    HeapType::Defined(TypeIdx(n as u32)),
                )))?;
                s.op(&t.params, &t.results)?;
                if i.opcode == 0x15 {
                    self.enabled(self.features.tail_call)?;
                    check(
                        t.results.len() == s.controls[0].results.len()
                            && t.results
                                .iter()
                                .zip(&s.controls[0].results)
                                .all(|(a, b)| value_subtype(*a, *b, &self.types)),
                        "tail call result mismatch",
                    )?;
                    s.dead()
                }
            }
            0x1a => {
                s.any()?;
            }
            0x1b | 0x1c => {
                s.pop(I32)?;
                if let Immediate::Types(ref ts) = i.immediate {
                    check(ts.len() == 1, "select requires one type")?;
                    self.valtype(ts[0])?;
                    s.op(&[ts[0], ts[0]], ts)?
                } else {
                    let b = s.any()?;
                    let a = s.any()?;
                    for value in [a, b].into_iter().flatten() {
                        check(value.is_numeric() || value == V128, "invalid select types")?;
                    }
                    if let (Some(a), Some(b)) = (a, b) {
                        check(a == b, "invalid select types")?
                    }
                    if a.is_none() && b.is_none() {
                        s.unknown.insert(s.values.len());
                    }
                    s.values.push(a.or(b).unwrap_or(I32));
                }
            }
            0x20..=0x22 => {
                let t = *locals
                    .get(index(i)?)
                    .ok_or_else(|| invalid("local index out of bounds"))?;
                match i.opcode {
                    0x20 => {
                        check(
                            s.initialized.get(index(i)?).copied().unwrap_or(true),
                            "uninitialized nonnullable local",
                        )?;
                        s.values.push(t)
                    }
                    0x21 => {
                        s.pop(t)?;
                        s.initialize(index(i)?)
                    }
                    _ => {
                        s.pop(t)?;
                        s.initialize(index(i)?);
                        s.values.push(t)
                    }
                }
            }
            0x23 | 0x24 => {
                let t = self
                    .globals
                    .get(index(i)?)
                    .ok_or_else(|| invalid("global index out of bounds"))?;
                if i.opcode == 0x23 {
                    s.values.push(t.value)
                } else {
                    check(t.mutable, "immutable global write")?;
                    s.pop(t.value)?
                }
            }
            0x25 | 0x26 => {
                let t = self
                    .tables
                    .get(index(i)?)
                    .ok_or_else(|| invalid("table index out of bounds"))?;
                if i.opcode == 0x25 {
                    s.op(&[t.index_type.val_type()], &[Ref(t.element)])?
                } else {
                    s.op(&[t.index_type.val_type(), Ref(t.element)], &[])?
                }
            }
            0x28..=0x3e => {
                let (align, memory) = if let Immediate::MemArg { align, memory, .. } = i.immediate {
                    (align, memory)
                } else {
                    return Err(invalid("invalid memory immediate"));
                };
                let m = self
                    .memories
                    .get(memory as usize)
                    .ok_or_else(|| invalid("memory index out of bounds"))?;
                let (t, size) = match i.opcode {
                    0x28 | 0x36 => (I32, 2),
                    0x29 | 0x37 => (I64, 3),
                    0x2a | 0x38 => (F32, 2),
                    0x2b | 0x39 => (F64, 3),
                    0x2c | 0x2d | 0x3a => (I32, 0),
                    0x2e | 0x2f | 0x3b => (I32, 1),
                    0x30 | 0x31 | 0x3c => (I64, 0),
                    0x32 | 0x33 | 0x3d => (I64, 1),
                    _ => (I64, 2),
                };
                check(align <= size, "memory alignment too large")?;
                if let Immediate::MemArg { offset, .. } = i.immediate {
                    check(
                        m.index_type == IndexType::I64 || offset <= u32::MAX as u64,
                        "memory offset overflow",
                    )?;
                };
                if i.opcode >= 0x36 {
                    s.op(&[m.index_type.val_type(), t], &[])?
                } else {
                    s.op(&[m.index_type.val_type()], &[t])?
                }
            }
            0x3f | 0x40 => {
                let m = self
                    .memories
                    .get(index(i)?)
                    .ok_or_else(|| invalid("memory index out of bounds"))?;
                let t = m.index_type.val_type();
                if i.opcode == 0x40 {
                    s.pop(t)?
                }
                s.values.push(t)
            }
            0x41 => s.values.push(I32),
            0x42 => s.values.push(I64),
            0x43 => s.values.push(F32),
            0x44 => s.values.push(F64),
            0x45 => s.op(&[I32], &[I32])?,
            0x46..=0x4f => s.op(&[I32, I32], &[I32])?,
            0x50 => s.op(&[I64], &[I32])?,
            0x51..=0x5a => s.op(&[I64, I64], &[I32])?,
            0x5b..=0x60 => s.op(&[F32, F32], &[I32])?,
            0x61..=0x66 => s.op(&[F64, F64], &[I32])?,
            0x67..=0x69 => s.op(&[I32], &[I32])?,
            0x6a..=0x78 => s.op(&[I32, I32], &[I32])?,
            0x79..=0x7b => s.op(&[I64], &[I64])?,
            0x7c..=0x8a => s.op(&[I64, I64], &[I64])?,
            0x8b..=0x91 => s.op(&[F32], &[F32])?,
            0x92..=0x98 => s.op(&[F32, F32], &[F32])?,
            0x99..=0x9f => s.op(&[F64], &[F64])?,
            0xa0..=0xa6 => s.op(&[F64, F64], &[F64])?,
            0xa7..=0xbf => {
                let (a, b) = match i.opcode {
                    0xa7 => (I64, I32),
                    0xa8 | 0xa9 => (F32, I32),
                    0xaa | 0xab => (F64, I32),
                    0xac | 0xad => (I32, I64),
                    0xae | 0xaf => (F32, I64),
                    0xb0 | 0xb1 => (F64, I64),
                    0xb2 | 0xb3 => (I32, F32),
                    0xb4 | 0xb5 => (I64, F32),
                    0xb6 => (F64, F32),
                    0xb7 | 0xb8 => (I32, F64),
                    0xb9 | 0xba => (I64, F64),
                    0xbb => (F32, F64),
                    0xbc => (F32, I32),
                    0xbd => (F64, I64),
                    0xbe => (I32, F32),
                    _ => (I64, F64),
                };
                s.op(&[a], &[b])?
            }
            0xc0 | 0xc1 => s.op(&[I32], &[I32])?,
            0xc2..=0xc4 => s.op(&[I64], &[I64])?,
            0xd0 => {
                if let Immediate::Heap(h) = i.immediate {
                    self.valtype(Ref(RefType::new(true, h)))?;
                    s.values.push(Ref(RefType::new(true, h)))
                } else {
                    return Err(invalid("invalid ref.null"));
                }
            }
            0xd3 => {
                self.enabled(self.features.gc)?;
                s.op(
                    &[
                        Ref(RefType::new(true, HeapType::Eq)),
                        Ref(RefType::new(true, HeapType::Eq)),
                    ],
                    &[I32],
                )?
            }
            0xd4 | 0xd5 => {
                let v = s.any()?;
                let r = match v {
                    Some(Ref(r)) => r,
                    None => RefType::FUNCREF,
                    _ => return Err(invalid("expected reference")),
                };
                if i.opcode == 0xd5 {
                    let ts = s.label(index(i)?)?;
                    s.pop_all(&ts)?;
                    s.values.extend(ts)
                }
                if v.is_none() {
                    s.unknown.insert(s.values.len());
                }
                s.values.push(Ref(RefType::new(false, r.heap)))
            }
            0xd6 => {
                let mut ts = s.label(index(i)?)?;
                let r = match ts.pop() {
                    Some(Ref(r)) => r,
                    _ => return Err(invalid("branch target must end in reference")),
                };
                s.pop(Ref(RefType::new(true, r.heap)))?;
                s.pop_all(&ts)?;
                s.values.extend(ts)
            }
            0xd1 => {
                if let Some(v) = s.any()? {
                    check(matches!(v, Ref(_)), "ref.is_null requires reference")?
                }
                s.values.push(I32)
            }
            0xd2 => {
                check(index(i)? < self.funcs.len(), "ref.func index out of bounds")?;
                let n = index(i)?;
                if self.features.function_references {
                    let ty = self
                        .m
                        .imports
                        .iter()
                        .filter_map(|im| {
                            if let ImportKind::Func(t) | ImportKind::ExactFunc(t) = im.kind {
                                Some(t)
                            } else {
                                None
                            }
                        })
                        .chain(self.m.functions.iter().copied())
                        .nth(n)
                        .ok_or_else(|| invalid("function index out of bounds"))?;
                    let imported = self
                        .m
                        .imports
                        .iter()
                        .filter(|i| {
                            matches!(i.kind, ImportKind::Func(_) | ImportKind::ExactFunc(_))
                        })
                        .collect::<Vec<_>>();
                    let exact = self.features.custom_descriptors
                        && (n >= imported.len()
                            || matches!(imported[n].kind, ImportKind::ExactFunc(_)));
                    s.values.push(Ref(RefType::new(
                        false,
                        if exact {
                            HeapType::ExactDefined(ty)
                        } else {
                            HeapType::Defined(ty)
                        },
                    )))
                } else {
                    s.values.push(ValType::FUNCREF)
                }
            }
            0xfc13..=0xfc16 => {
                self.enabled(self.features.wide_arithmetic)?;
                if i.opcode <= 0xfc14 {
                    s.op(&[I64, I64, I64, I64], &[I64, I64])?
                } else {
                    s.op(&[I64, I64], &[I64, I64])?
                }
            }
            0xfc00..=0xfc07 => {
                let sub = i.opcode - 0xfc00;
                let a = if sub % 4 < 2 { F32 } else { F64 };
                let b = if sub < 4 { I32 } else { I64 };
                s.op(&[a], &[b])?
            }
            0xfc08 | 0xfc0a => {
                let (a, b) = if let Immediate::Pair(a, b) = i.immediate {
                    (a as usize, b as usize)
                } else {
                    return Err(invalid("invalid bulk memory immediate"));
                };
                let (dst, src) = if i.opcode == 0xfc08 {
                    check(
                        self.m.data_count.is_some(),
                        "memory.init requires data count",
                    )?;
                    check(a < self.m.data.len(), "data index out of bounds")?;
                    let m = self
                        .memories
                        .get(b)
                        .ok_or_else(|| invalid("memory index out of bounds"))?;
                    (m.index_type.val_type(), I32)
                } else {
                    let a = self
                        .memories
                        .get(a)
                        .ok_or_else(|| invalid("memory index out of bounds"))?;
                    let b = self
                        .memories
                        .get(b)
                        .ok_or_else(|| invalid("memory index out of bounds"))?;
                    (a.index_type.val_type(), b.index_type.val_type())
                };
                let len = if dst == I64 && src == I64 { I64 } else { I32 };
                s.op(&[dst, src, len], &[])?
            }
            0xfc09 => check(
                self.m.data_count.is_some() && index(i)? < self.m.data.len(),
                "invalid data.drop index",
            )?,
            0xfc0b => {
                let m = self
                    .memories
                    .get(index(i)?)
                    .ok_or_else(|| invalid("memory index out of bounds"))?;
                let t = m.index_type.val_type();
                s.op(&[t, I32, t], &[])?
            }
            0xfc0c | 0xfc0e => {
                let (a, b) = if let Immediate::Pair(a, b) = i.immediate {
                    (a as usize, b as usize)
                } else {
                    return Err(invalid("invalid bulk table immediate"));
                };
                let (dst, src) = if i.opcode == 0xfc0c {
                    let e = self
                        .m
                        .elements
                        .get(a)
                        .ok_or_else(|| invalid("element index out of bounds"))?;
                    let t = self
                        .tables
                        .get(b)
                        .ok_or_else(|| invalid("table index out of bounds"))?;
                    check(
                        value_subtype(Ref(e.ty), Ref(t.element), &self.types),
                        "table.init type mismatch",
                    )?;
                    (t.index_type.val_type(), I32)
                } else {
                    let a = self
                        .tables
                        .get(a)
                        .ok_or_else(|| invalid("table index out of bounds"))?;
                    let b = self
                        .tables
                        .get(b)
                        .ok_or_else(|| invalid("table index out of bounds"))?;
                    check(
                        value_subtype(Ref(b.element), Ref(a.element), &self.types),
                        "table.copy type mismatch",
                    )?;
                    (a.index_type.val_type(), b.index_type.val_type())
                };
                s.op(
                    &[dst, src, if dst == I64 && src == I64 { I64 } else { I32 }],
                    &[],
                )?
            }
            0xfc0d => check(
                index(i)? < self.m.elements.len(),
                "element index out of bounds",
            )?,
            0xfc0f..=0xfc11 => {
                let t = self
                    .tables
                    .get(index(i)?)
                    .ok_or_else(|| invalid("table index out of bounds"))?;
                let n = t.index_type.val_type();
                match i.opcode {
                    0xfc0f => s.op(&[Ref(t.element), n], &[n])?,
                    0xfc10 => s.values.push(n),
                    _ => s.op(&[n, Ref(t.element), n], &[])?,
                }
            }
            0x08 | 0x0a | 0x1f => self.exception_instruction(s, i)?,
            0xe0..=0xe6 => self.continuation(s, i)?,
            0xfe00..=0xfe4e => self.atomic(s, i)?,
            0xfb00..=0xfb26 => self.gc(s, i)?,
            0xfd00..=0xfdff | 0xfa00..=0xfa13 => self.simd(s, i)?,
            _ => return Err(invalid(format!("unsupported instruction {:#x}", i.opcode))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn module(result: u8, ops: &[u8]) -> Vec<u8> {
        let mut b = b"\0asm\x01\0\0\0".to_vec();
        b.extend([1, 5, 1, 0x60, 0, 1, result, 3, 2, 1, 0]);
        b.extend([10, (ops.len() + 4) as u8, 1, (ops.len() + 2) as u8, 0]);
        b.extend(ops);
        b.push(0x0b);
        b
    }
    fn valid(b: &[u8]) -> Result<ValidatedModule> {
        validate(b, &FeatureSet::default(), &ResourceLimits::default())
    }
    #[test]
    fn t033_empty_module() {
        assert!(valid(b"\0asm\x01\0\0\0").is_ok())
    }
    #[test]
    fn t033_numeric_result() {
        assert!(valid(&module(0x7f, &[0x41, 7])).is_ok());
    }
    #[test]
    fn t033_wrong_result() {
        assert!(valid(&module(0x7f, &[0x42, 7])).is_err());
    }
    #[test]
    fn t033_stack_underflow() {
        assert!(valid(&module(0x7f, &[0x6a])).is_err());
    }
    #[test]
    fn t033_unreachable_polymorphism() {
        assert!(valid(&module(0x7f, &[0, 0x6a])).is_ok());
        assert!(valid(&module(0x7c, &[0, 0x1b, 0x9a])).is_ok());
    }
    #[test]
    fn t033_local_bounds() {
        assert!(valid(&module(0x7f, &[0x20, 0])).is_err());
    }
    #[test]
    fn t033_branch_bounds() {
        assert!(valid(&module(0x7f, &[0x0c, 1])).is_err());
    }
    #[test]
    fn t033_block_result() {
        assert!(valid(&module(0x7f, &[2, 0x7f, 0x41, 1, 0x0b])).is_ok());
    }
    #[test]
    fn t033_if_needs_else() {
        assert!(valid(&module(0x7f, &[0x41, 1, 4, 0x7f, 0x41, 1, 0x0b])).is_err());
    }
    #[test]
    fn t033_extra_operand() {
        assert!(valid(&module(0x7f, &[0x41, 1, 0x41, 2])).is_err());
    }
    #[test]
    fn t033_unknown_opcode() {
        assert!(valid(&module(0x7f, &[0xff])).is_err());
    }
}

impl Context<'_> {
    fn simd(&self, s: &mut Stack, i: &Instruction) -> Result<()> {
        use ValType::*;
        self.enabled(self.features.simd)?;
        let n = if i.opcode >= 0xfd00 {
            i.opcode - 0xfd00
        } else {
            self.enabled(self.features.relaxed_simd)?;
            i.opcode - 0xfa00 + 0x100
        };
        match n {
            0..=11 | 84..=93 => {
                let (align, memory, lane) = match &i.immediate {
                    Immediate::MemArg { align, memory, .. } => (*align, *memory, None),
                    Immediate::MemLane {
                        align,
                        memory,
                        lane,
                        ..
                    } => (*align, *memory, Some(*lane)),
                    Immediate::Bytes(v) if v.len() == 17 => (
                        u32::from_le_bytes(v[0..4].try_into().unwrap()),
                        u32::from_le_bytes(v[12..16].try_into().unwrap()),
                        Some(v[16]),
                    ),
                    _ => return Err(invalid("invalid SIMD memory immediate")),
                };
                let natural = match n {
                    0 | 11 => 4,
                    1..=6 => 3,
                    7 => 0,
                    8 => 1,
                    9 | 92 => 2,
                    10 | 93 => 3,
                    84..=91 => (n - 84) % 4,
                    _ => unreachable!(),
                };
                check(align <= natural as u32, "SIMD memory alignment too large")?;
                if let Some(l) = lane {
                    check(
                        (l as usize) < (16usize >> natural),
                        "SIMD lane out of bounds",
                    )?;
                }
                let memory_type = self
                    .memories
                    .get(memory as usize)
                    .ok_or_else(|| invalid("SIMD memory index out of bounds"))?;
                let offset = match i.immediate {
                    Immediate::MemArg { offset, .. } | Immediate::MemLane { offset, .. } => offset,
                    _ => 0,
                };
                check(
                    memory_type.index_type == IndexType::I64 || offset <= u32::MAX as u64,
                    "SIMD memory offset overflow",
                )?;
                let t = memory_type.index_type.val_type();
                if n == 11 || (84..=91).contains(&n) {
                    s.op(
                        &[t, V128],
                        if (88..=91).contains(&n) || n == 11 {
                            &[]
                        } else {
                            &[V128]
                        },
                    )?
                } else {
                    s.op(&[t], &[V128])?
                }
            }
            12 => {
                check(
                    matches!(&i.immediate,Immediate::Bytes(v) if v.len()==16),
                    "invalid SIMD constant",
                )?;
                s.values.push(V128)
            }
            13 => {
                check(
                    matches!(&i.immediate,Immediate::Bytes(v) if v.len()==16 && v.iter().all(|x|*x<32)),
                    "SIMD shuffle lane out of bounds",
                )?;
                s.op(&[V128, V128], &[V128])?
            }
            14 => s.op(&[V128, V128], &[V128])?,
            15..=20 => {
                let t = match n {
                    15..=17 => I32,
                    18 => I64,
                    19 => F32,
                    _ => F64,
                };
                s.op(&[t], &[V128])?
            }
            21..=34 => {
                let (lanes, t) = match n {
                    21..=23 => (16, I32),
                    24..=26 => (8, I32),
                    27..=28 => (4, I32),
                    29..=30 => (2, I64),
                    31..=32 => (4, F32),
                    _ => (2, F64),
                };
                check(index(i)? < lanes, "SIMD lane out of bounds")?;
                if matches!(n, 23 | 26 | 28 | 30 | 32 | 34) {
                    s.op(&[V128, t], &[V128])?
                } else {
                    s.op(&[V128], &[t])?
                }
            }
            0x23 | 0x24 | 0x25 | 0x26 | 0x27 | 0x28 | 0x29 | 0x2a | 0x2b | 0x2c | 0x2d | 0x2e
            | 0x2f | 0x30 | 0x31 | 0x32 | 0x33 | 0x34 | 0x35 | 0x36 | 0x37 | 0x38 | 0x39 | 0x3a
            | 0x3b | 0x3c | 0x3d | 0x3e | 0x3f | 0x40 | 0x41 | 0x42 | 0x43 | 0x44 | 0x45 | 0x46
            | 0x47 | 0x48 | 0x49 | 0x4a | 0x4b | 0x4c | 0x4e | 0x4f | 0x50 | 0x51 | 0x65 | 0x66
            | 0x6e | 0x6f | 0x70 | 0x71 | 0x72 | 0x73 | 0x76 | 0x77 | 0x78 | 0x79 | 0x7b | 0x82
            | 0x85 | 0x86 | 0x8e | 0x8f | 0x90 | 0x91 | 0x92 | 0x93 | 0x95 | 0x96 | 0x97 | 0x98
            | 0x99 | 0x9b | 0x9c | 0x9d | 0x9e | 0x9f | 0xae | 0xb1 | 0xb5 | 0xb6 | 0xb7 | 0xb8
            | 0xb9 | 0xba | 0xbc | 0xbd | 0xbe | 0xbf | 0xce | 0xd1 | 0xd5 | 0xd6 | 0xd7 | 0xd8
            | 0xd9 | 0xda | 0xdb | 0xdc | 0xdd | 0xde | 0xdf | 0xe4 | 0xe5 | 0xe6 | 0xe7 | 0xe8
            | 0xe9 | 0xea | 0xeb | 0xf0 | 0xf1 | 0xf2 | 0xf3 | 0xf4 | 0xf5 | 0xf6 | 0xf7
            | 0x100 | 0x10d | 0x10e | 0x10f | 0x110 | 0x111 | 0x112 => {
                s.op(&[V128, V128], &[V128])?
            }
            0x4d | 0x5e | 0x5f | 0x60 | 0x61 | 0x62 | 0x67 | 0x68 | 0x69 | 0x6a | 0x74 | 0x75
            | 0x7a | 0x7c | 0x7d | 0x7e | 0x7f | 0x80 | 0x81 | 0x87 | 0x88 | 0x89 | 0x8a | 0x94
            | 0xa0 | 0xa1 | 0xa7 | 0xa8 | 0xa9 | 0xaa | 0xc0 | 0xc1 | 0xc7 | 0xc8 | 0xc9 | 0xca
            | 0xe0 | 0xe1 | 0xe3 | 0xec | 0xed | 0xef | 0xf8 | 0xf9 | 0xfa | 0xfb | 0xfc | 0xfd
            | 0xfe | 0xff | 0x101 | 0x102 | 0x103 | 0x104 => s.op(&[V128], &[V128])?,
            0x52 | 0x105 | 0x106 | 0x107 | 0x108 | 0x109 | 0x10a | 0x10b | 0x10c | 0x113 => {
                s.op(&[V128, V128, V128], &[V128])?
            }
            0x53 | 0x63 | 0x64 | 0x83 | 0x84 | 0xa3 | 0xa4 | 0xc3 | 0xc4 => {
                s.op(&[V128], &[I32])?
            }
            0x6b | 0x6c | 0x6d | 0x8b | 0x8c | 0x8d | 0xab | 0xac | 0xad | 0xcb | 0xcc | 0xcd => {
                s.op(&[V128, I32], &[V128])?
            }
            _ => return Err(invalid("unknown SIMD instruction")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;
    // (module (type $a (sub (struct (field i32)))) (type $b (sub $a (struct (field i32) (field i64)))))
    #[test]
    fn t107_case_01() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 16, 2, 80, 0, 95, 1, 127, 0, 80, 1, 0, 95, 2, 127, 0,
            126, 0, 0, 14, 4, 110, 97, 109, 101, 4, 7, 2, 0, 1, 97, 1, 1, 98,
        ];
        assert!(validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (sub (struct (field (mut (ref null any)))))) (type $b (sub $a (struct (field (mut (ref null eq)))))))
    #[test]
    fn t107_case_02() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 14, 2, 80, 0, 95, 1, 110, 1, 80, 1, 0, 95, 1, 109, 1,
            0, 14, 4, 110, 97, 109, 101, 4, 7, 2, 0, 1, 97, 1, 1, 98,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (struct)) (type $b (sub $a (struct))))
    #[test]
    fn t107_case_03() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 95, 0, 80, 1, 0, 95, 0, 0, 14, 4, 110, 97, 109,
            101, 4, 7, 2, 0, 1, 97, 1, 1, 98,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (struct (field i32))) (func (param (ref null $a)) local.get 0 i32.const 0 struct.set $a 0))
    #[test]
    fn t111_case_01() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 10, 2, 95, 1, 127, 0, 96, 1, 99, 0, 0, 3, 2, 1, 1, 10,
            12, 1, 10, 0, 32, 0, 65, 0, 251, 5, 0, 0, 11, 0, 11, 4, 110, 97, 109, 101, 4, 4, 1, 0,
            1, 97,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (array (mut (ref null func)))) (elem externref (ref.null extern)) (func i32.const 0 i32.const 0 array.new_elem $a 0 drop))
    #[test]
    fn t111_case_02() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 7, 2, 94, 112, 1, 96, 0, 0, 3, 2, 1, 1, 9, 7, 1, 5,
            111, 1, 208, 111, 11, 10, 13, 1, 11, 0, 65, 0, 65, 0, 251, 10, 0, 0, 26, 11, 0, 11, 4,
            110, 97, 109, 101, 4, 4, 1, 0, 1, 97,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (struct (field (ref func)))) (func struct.new_default $a drop))
    #[test]
    fn t111_case_03() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 9, 2, 95, 1, 100, 112, 0, 96, 0, 0, 3, 2, 1, 1, 10, 8,
            1, 6, 0, 251, 1, 0, 26, 11, 0, 11, 4, 110, 97, 109, 101, 4, 4, 1, 0, 1, 97,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (struct (field i8))) (func (param (ref null $a)) local.get 0 struct.get $a 0 drop))
    #[test]
    fn t111_packed() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 10, 2, 95, 1, 120, 0, 96, 1, 99, 0, 0, 3, 2, 1, 1, 10,
            11, 1, 9, 0, 32, 0, 251, 2, 0, 0, 26, 11, 0, 11, 4, 110, 97, 109, 101, 4, 4, 1, 0, 1,
            97,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (type $a (struct (field i32))) (func i32.const 7 struct.new $a struct.get $a 0 drop))
    #[test]
    fn t111_valid() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 95, 1, 127, 0, 96, 0, 0, 3, 2, 1, 1, 10, 14, 1,
            12, 0, 65, 7, 251, 0, 0, 251, 2, 0, 0, 26, 11, 0, 11, 4, 110, 97, 109, 101, 4, 4, 1, 0,
            1, 97,
        ];
        assert!(validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (func (result v128) v128.const i32x4 1 2 3 4 v128.const i32x4 5 6 7 8 i32x4.add))
    #[test]
    fn t099_simd_valid() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 43, 1, 41, 0, 253,
            12, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 253, 12, 5, 0, 0, 0, 6, 0, 0, 0, 7,
            0, 0, 0, 8, 0, 0, 0, 253, 174, 1, 11,
        ];
        assert!(validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
    // (module (func (result i32) v128.const i32x4 1 2 3 4 i32x4.extract_lane 4))
    #[test]
    fn t099_simd_bad_lane() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 10, 25, 1, 23, 0, 253,
            12, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 253, 27, 4, 11,
        ];
        assert!(!validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
}

#[cfg(test)]
mod more_proposal_tests {
    use super::*;
    // (module (memory 1 1 shared) (func (result i32) i32.const 0 i32.atomic.load))
    #[test]
    fn t130_atomic_valid() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 5, 4, 1, 3, 1, 1, 10,
            10, 1, 8, 0, 65, 0, 254, 16, 2, 0, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    // (module (memory 1 1 shared) (func (result i32) i32.const 0 i32.atomic.load align=1))
    #[test]
    fn t130_atomic_alignment() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 5, 4, 1, 3, 1, 1, 10,
            10, 1, 8, 0, 65, 0, 254, 16, 0, 0, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (memory 1 1 shared) (func (result i32) i64.const 0 i32.atomic.load))
    #[test]
    fn t130_atomic_type() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 5, 4, 1, 3, 1, 1, 10,
            10, 1, 8, 0, 66, 0, 254, 16, 2, 0, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (descriptor $d) (struct)) (type $d (describes $a) (struct))) (func struct.new $d struct.new_desc $a drop))
    #[test]
    fn t150_descriptor_valid() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 14, 2, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 0, 0, 3,
            2, 1, 2, 10, 11, 1, 9, 0, 251, 0, 1, 251, 32, 0, 26, 11, 0, 14, 4, 110, 97, 109, 101,
            4, 7, 2, 0, 1, 97, 1, 1, 100,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (descriptor $d) (struct)) (type $d (describes $a) (struct))) (func (param (ref $d)) local.get 0 struct.new_desc $a drop))
    #[test]
    fn t150_descriptor_requires_exact() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 16, 2, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 1, 100,
            1, 0, 3, 2, 1, 2, 10, 10, 1, 8, 0, 32, 0, 251, 32, 0, 26, 11, 0, 14, 4, 110, 97, 109,
            101, 4, 7, 2, 0, 1, 97, 1, 1, 100,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (descriptor $d) (struct)) (type $d (describes $a) (struct))) (func struct.new $a drop))
    #[test]
    fn t150_descriptor_normal_constructor() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 14, 2, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 0, 0, 3,
            2, 1, 2, 10, 8, 1, 6, 0, 251, 0, 0, 26, 11, 0, 14, 4, 110, 97, 109, 101, 4, 7, 2, 0, 1,
            97, 1, 1, 100,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (func (local (ref func)) local.get 0 drop))
    #[test]
    fn t111_nondefaultable_local() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 10, 10, 1, 8, 1, 1, 100,
            112, 32, 0, 26, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (func $f (export "f") (local (ref func)) ref.func $f local.set 0 local.get 0 drop))
    #[test]
    fn t111_initialized_local() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 7, 5, 1, 1, 102, 0, 0, 10,
            14, 1, 12, 1, 1, 100, 112, 210, 0, 33, 0, 32, 0, 26, 11, 0, 11, 4, 110, 97, 109, 101,
            1, 4, 1, 0, 1, 102,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    #[test]
    fn t139_generator_bind() {
        let b: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 20, 5, 96, 0, 1, 127, 93, 0, 96, 1, 127, 1, 127, 93, 2,
            96, 0, 2, 127, 100, 3, 3, 6, 5, 0, 0, 0, 0, 0, 13, 3, 1, 0, 2, 7, 23, 3, 3, 114, 117,
            110, 0, 1, 5, 116, 119, 105, 99, 101, 0, 3, 5, 98, 111, 117, 110, 100, 0, 4, 9, 9, 2,
            3, 0, 1, 0, 3, 0, 1, 2, 10, 97, 5, 6, 0, 65, 7, 226, 0, 11, 29, 1, 1, 99, 3, 2, 4, 210,
            0, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0, 26, 65, 9, 32, 0, 227, 3, 0, 11, 4, 0,
            65, 5, 11, 20, 1, 1, 99, 1, 210, 2, 224, 1, 34, 0, 227, 1, 0, 26, 32, 0, 227, 1, 0, 11,
            32, 1, 1, 99, 3, 2, 4, 210, 0, 224, 1, 227, 1, 1, 0, 0, 0, 15, 11, 33, 0, 26, 65, 19,
            32, 0, 225, 3, 1, 227, 1, 0, 11, 0, 98, 4, 110, 97, 109, 101, 1, 22, 2, 0, 9, 103, 101,
            110, 101, 114, 97, 116, 111, 114, 2, 8, 99, 111, 110, 115, 116, 97, 110, 116, 2, 24, 3,
            1, 1, 0, 1, 99, 3, 1, 0, 5, 115, 97, 118, 101, 100, 4, 1, 0, 5, 115, 97, 118, 101, 100,
            4, 31, 5, 0, 1, 102, 1, 1, 99, 2, 5, 121, 105, 101, 108, 100, 3, 6, 114, 101, 115, 117,
            109, 101, 4, 7, 104, 97, 110, 100, 108, 101, 114, 11, 8, 1, 0, 5, 121, 105, 101, 108,
            100,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    #[test]
    fn t033_type_work_budget() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 10, 6, 1, 4, 0, 65, 0,
            11,
        ];
        let limits = ResourceLimits {
            max_type_work: 0,
            ..ResourceLimits::default()
        };
        assert_eq!(
            validate(b, &FeatureSet::default(), &limits)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
    }
    #[test]
    fn t033_stack_budget() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 10, 6, 1, 4, 0, 65, 0,
            11,
        ];
        let limits = ResourceLimits {
            max_stack_height: 0,
            ..ResourceLimits::default()
        };
        assert_eq!(
            validate(b, &FeatureSet::default(), &limits)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
    }
}

#[cfg(test)]
mod corpus_regressions {
    use super::*;
    #[test]
    fn t073_tail_result_covariance() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 23, 5, 96, 0, 0, 96, 0, 1, 100, 0, 96, 0, 1, 99, 0, 96,
            0, 1, 100, 112, 96, 0, 1, 112, 3, 10, 9, 1, 2, 2, 3, 3, 4, 4, 4, 4, 9, 8, 1, 3, 0, 4,
            0, 2, 4, 8, 10, 64, 9, 6, 0, 210, 0, 21, 1, 11, 6, 0, 210, 0, 21, 1, 11, 6, 0, 210, 2,
            21, 2, 11, 6, 0, 210, 0, 21, 1, 11, 6, 0, 210, 4, 21, 3, 11, 6, 0, 210, 0, 21, 1, 11,
            6, 0, 210, 2, 21, 2, 11, 6, 0, 210, 4, 21, 3, 11, 6, 0, 210, 8, 21, 4, 11, 0, 75, 4,
            110, 97, 109, 101, 1, 46, 9, 0, 3, 102, 49, 49, 1, 3, 102, 50, 49, 2, 3, 102, 50, 50,
            3, 3, 102, 51, 49, 4, 3, 102, 51, 51, 5, 3, 102, 52, 49, 6, 3, 102, 52, 50, 7, 3, 102,
            52, 51, 8, 3, 102, 52, 52, 4, 20, 5, 0, 1, 116, 1, 2, 116, 49, 2, 2, 116, 50, 3, 2,
            116, 51, 4, 2, 116, 52,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    #[test]
    fn t111_large_polymorphic_array() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 9, 2, 94, 127, 0, 96, 0, 1, 100, 0, 3, 2, 1, 1, 10, 13,
            1, 11, 0, 0, 251, 8, 0, 255, 255, 255, 255, 15, 11, 0, 11, 4, 110, 97, 109, 101, 4, 4,
            1, 0, 1, 97,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
}

#[cfg(test)]
mod mvp_regressions {
    use super::*;
    #[test]
    fn t033_mvp_legacy_element() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 4, 5, 1, 112, 1, 1, 1, 9,
            7, 1, 0, 65, 0, 11, 1, 0, 10, 4, 1, 2, 0, 11,
        ];
        let r = validate(b, &FeatureSet::mvp(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
}

#[cfg(test)]
mod corpus_validation_regressions {
    use super::*;
    // (module (rec (type $f1 (func)) (type (struct (field (ref $f1))))) (rec (type $f2 (func)) (type (struct (field (ref $f1))))) (func $f (type $f2)) (global (ref $f1) (ref.func $f)))
    #[test]
    fn t107_internal_external_distinction() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 21, 2, 78, 2, 96, 0, 0, 95, 1, 100, 0, 0, 78, 2, 96, 0,
            0, 95, 1, 100, 0, 0, 3, 2, 1, 2, 6, 7, 1, 100, 0, 0, 210, 0, 11, 10, 4, 1, 2, 0, 11, 0,
            22, 4, 110, 97, 109, 101, 1, 4, 1, 0, 1, 102, 4, 9, 2, 0, 2, 102, 49, 2, 2, 102, 50,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (table 0 (ref func)))
    #[test]
    fn t033_nonnullable_table_requires_initializer() {
        let b = &[0, 97, 115, 109, 1, 0, 0, 0, 4, 5, 1, 100, 112, 0, 0];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (elem (ref 1)))
    #[test]
    fn t033_empty_element_type_checked() {
        let b = &[0, 97, 115, 109, 1, 0, 0, 0, 9, 5, 1, 5, 100, 1, 0];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (func) (table 1 (ref func) (ref.func 0)) (elem (i32.const 0) func 0))
    #[test]
    fn t033_nonnullable_table_legacy_element() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 4, 10, 1, 64, 0, 100, 112,
            0, 1, 210, 0, 11, 9, 7, 1, 0, 65, 0, 11, 1, 0, 10, 4, 1, 2, 0, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    // (module (memory 1) (func i32.const 0 v128.load offset=4294967296 drop))
    #[test]
    fn t097_simd_offset_overflow() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 3, 1, 0, 1, 10, 15, 1,
            13, 0, 65, 0, 253, 0, 4, 128, 128, 128, 128, 16, 26, 11,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (memory i64 0 281474976710656))
    #[test]
    fn t088_memory64_boundary() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 5, 10, 1, 5, 0, 128, 128, 128, 128, 128, 128, 64,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
}

#[cfg(test)]
mod empty_rec_regression {
    use super::*;
    #[test]
    fn t107_empty_rec_group_valid() {
        let b = &[0, 97, 115, 109, 1, 0, 0, 0, 1, 3, 1, 78, 0];
        assert!(validate(b, &FeatureSet::default(), &ResourceLimits::default()).is_ok());
    }
}

#[cfg(test)]
mod descriptor_regressions {
    use super::*;
    // (module (rec (type $a (descriptor $b) (struct)) (type $b (describes $a) (struct))) (func (result (ref (exact $b))) unreachable ref.get_desc $a))
    #[test]
    fn t150_exact_descriptor_bottom() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 17, 2, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 0, 1,
            100, 98, 1, 3, 2, 1, 2, 10, 8, 1, 6, 0, 0, 251, 34, 0, 11, 0, 14, 4, 110, 97, 109, 101,
            4, 7, 2, 0, 1, 97, 1, 1, 98,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (sub (descriptor $b) (struct))) (type $b (sub (describes $a) (struct))) (type $c (sub $a (descriptor $d) (struct))) (type $d (sub $b (describes $c) (struct)))) (func (param (ref (exact $c))) (result (ref (exact $b))) local.get 0 ref.get_desc $a))
    #[test]
    fn t150_exact_descriptor_subtype() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 38, 2, 78, 4, 80, 0, 77, 1, 95, 0, 80, 0, 76, 0, 95, 0,
            80, 1, 0, 77, 3, 95, 0, 80, 1, 1, 76, 2, 95, 0, 96, 1, 100, 98, 2, 1, 100, 98, 1, 3, 2,
            1, 4, 10, 9, 1, 7, 0, 32, 0, 251, 34, 0, 11, 0, 20, 4, 110, 97, 109, 101, 4, 13, 4, 0,
            1, 97, 1, 1, 98, 2, 1, 99, 3, 1, 100,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(!r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (descriptor $b) (struct)) (type $b (describes $a) (struct))) (func (param (ref null any) (ref null $b)) (result i32 (ref null $a)) (block (result i32 (ref any)) i32.const 42 local.get 0 local.get 1 br_on_cast_desc_eq 1 (ref null any) (ref null $a)) unreachable))
    #[test]
    fn t150_desc_branch_refines_null() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 26, 3, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 2, 110,
            99, 1, 2, 127, 99, 0, 96, 0, 2, 127, 100, 110, 3, 2, 1, 2, 10, 20, 1, 18, 0, 2, 3, 65,
            42, 32, 0, 32, 1, 251, 37, 3, 1, 110, 0, 11, 0, 11, 0, 14, 4, 110, 97, 109, 101, 4, 7,
            2, 0, 1, 97, 1, 1, 98,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
    // (module (rec (type $a (descriptor $b) (struct)) (type $b (describes $a) (struct))) (func (param (ref null any) (ref null $b)) (result i32 (ref any)) (block (result i32 (ref null $a)) i32.const 42 local.get 0 local.get 1 br_on_cast_desc_eq_fail 1 (ref null any) (ref null $a)) unreachable))
    #[test]
    fn t150_desc_fail_refines_null() {
        let b = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 26, 3, 78, 2, 77, 1, 95, 0, 76, 0, 95, 0, 96, 2, 110,
            99, 1, 2, 127, 100, 110, 96, 0, 2, 127, 99, 0, 3, 2, 1, 2, 10, 20, 1, 18, 0, 2, 3, 65,
            42, 32, 0, 32, 1, 251, 38, 3, 1, 110, 0, 11, 0, 11, 0, 14, 4, 110, 97, 109, 101, 4, 7,
            2, 0, 1, 97, 1, 1, 98,
        ];
        let r = validate(b, &FeatureSet::default(), &ResourceLimits::default());
        assert!(r.is_ok(), "{r:?}");
    }
}

#[cfg(test)]
mod acceptance;
