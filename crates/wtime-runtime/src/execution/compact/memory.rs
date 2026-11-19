//! Source-only P043 draft: checked scalar memory operands and affine bindings.
//! Views and lengths never survive an individual source instruction.
use super::*;

const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarKind {
    I32,
    I64,
    F32,
    F64,
    I32S8,
    I32U8,
    I32S16,
    I32U16,
    I64S8,
    I64U8,
    I64S16,
    I64U16,
    I64S32,
    I64U32,
    Store32,
    Store64,
    StoreF32,
    StoreF64,
    Store32U8,
    Store32U16,
    Store64U8,
    Store64U16,
    Store64U32,
}
impl ScalarKind {
    fn from_opcode(opcode: u16) -> Option<Self> {
        Some(match opcode {
            0x28 => Self::I32,
            0x29 => Self::I64,
            0x2a => Self::F32,
            0x2b => Self::F64,
            0x2c => Self::I32S8,
            0x2d => Self::I32U8,
            0x2e => Self::I32S16,
            0x2f => Self::I32U16,
            0x30 => Self::I64S8,
            0x31 => Self::I64U8,
            0x32 => Self::I64S16,
            0x33 => Self::I64U16,
            0x34 => Self::I64S32,
            0x35 => Self::I64U32,
            0x36 => Self::Store32,
            0x37 => Self::Store64,
            0x38 => Self::StoreF32,
            0x39 => Self::StoreF64,
            0x3a => Self::Store32U8,
            0x3b => Self::Store32U16,
            0x3c => Self::Store64U8,
            0x3d => Self::Store64U16,
            0x3e => Self::Store64U32,
            _ => return None,
        })
    }
    fn width(self) -> usize {
        match self {
            Self::I32
            | Self::F32
            | Self::I64S32
            | Self::I64U32
            | Self::Store32
            | Self::StoreF32
            | Self::Store64U32 => 4,
            Self::I64 | Self::F64 | Self::Store64 | Self::StoreF64 => 8,
            Self::I32S8
            | Self::I32U8
            | Self::I64S8
            | Self::I64U8
            | Self::Store32U8
            | Self::Store64U8 => 1,
            _ => 2,
        }
    }
    fn store(self) -> bool {
        matches!(
            self,
            Self::Store32
                | Self::Store64
                | Self::StoreF32
                | Self::StoreF64
                | Self::Store32U8
                | Self::Store32U16
                | Self::Store64U8
                | Self::Store64U16
                | Self::Store64U32
        )
    }
    fn value(self, bits: u64) -> Value {
        match self {
            Self::I32 => Value::I32(bits as i32),
            Self::I64 => Value::I64(bits as i64),
            Self::F32 => Value::F32(bits as u32),
            Self::F64 => Value::F64(bits),
            Self::I32S8 => Value::I32(bits as i8 as i32),
            Self::I32U8 => Value::I32(bits as i32),
            Self::I32S16 => Value::I32(bits as i16 as i32),
            Self::I32U16 => Value::I32(bits as i32),
            Self::I64S8 => Value::I64(bits as i8 as i64),
            Self::I64U8 => Value::I64(bits as i64),
            Self::I64S16 => Value::I64(bits as i16 as i64),
            Self::I64U16 => Value::I64(bits as i64),
            Self::I64S32 => Value::I64(bits as i32 as i64),
            Self::I64U32 => Value::I64(bits as i64),
            _ => unreachable!("store access cannot decode a load"),
        }
    }
}

pub(super) struct MemoryTypes {
    engine: u64,
    source: Arc<[u8]>,
    entries: Vec<MemoryType>,
    pub(super) work: usize,
    pub(super) bytes: usize,
}
fn size<T>(capacity: usize, header: usize) -> Option<usize> {
    capacity
        .checked_mul(std::mem::size_of::<T>())?
        .checked_add(header)
}
impl MemoryTypes {
    pub(super) fn prepare(module: &Module, max_work: usize, max_bytes: usize) -> Option<Self> {
        let decoded = module.data.inner.decoded();
        let work = decoded.imports.len().checked_add(decoded.memories.len())?;
        if work > max_work.min(MAX_WORK)
            || size::<MemoryType>(work, std::mem::size_of::<Self>())? > max_bytes.min(MAX_BYTES)
        {
            return None;
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(work).ok()?;
        let bytes = size::<MemoryType>(entries.capacity(), std::mem::size_of::<Self>())?;
        if bytes > max_bytes.min(MAX_BYTES) {
            return None;
        }
        entries.extend(
            decoded
                .imports
                .iter()
                .filter_map(|import| match import.kind {
                    ImportKind::Memory(ty) => Some(ty),
                    _ => None,
                })
                .chain(decoded.memories.iter().copied()),
        );
        Some(Self {
            engine: module.data.engine,
            source: decoded.bytes.clone(),
            entries,
            work,
            bytes,
        })
    }
    pub(super) fn supports(&self, instruction: &Instruction) -> bool {
        ScalarKind::from_opcode(instruction.opcode).is_some()
            && matches!(instruction.immediate, Immediate::MemArg { memory, .. } if self.entries.get(memory as usize).is_some())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemoryOp {
    kind: ScalarKind,
    width: u8,
    memory: u32,
    offset: u64,
    index_type: IndexType,
}

#[derive(Debug)]
pub(super) struct MemoryAccesses {
    entries: Vec<Option<MemoryOp>>,
    pub(super) bytes: usize,
    work: usize,
}
impl MemoryAccesses {
    #[cfg(test)]
    pub(super) fn p045_corrupt(&mut self) { self.entries.clear(); }
    pub(super) fn prepare(
        function: &CompactFunction,
        module: &Module,
        types: &MemoryTypes,
        max_work: usize,
        max_bytes: usize,
    ) -> std::result::Result<Option<Self>, Declined> {
        let code = module
            .data
            .inner
            .decoded()
            .code
            .get(function.body)
            .ok_or(Declined::Plan)?;
        let work = code
            .instructions
            .len()
            .checked_mul(3)
            .ok_or(Declined::Budget)?;
        if work > max_work.min(MAX_WORK) {
            return Err(Declined::Budget);
        }
        if !code
            .instructions
            .iter()
            .any(|instruction| ScalarKind::from_opcode(instruction.opcode).is_some())
        {
            return Ok(None);
        }
        let max_bytes = max_bytes.min(MAX_BYTES);
        let total = size::<Option<MemoryOp>>(code.instructions.len(), std::mem::size_of::<Self>())
            .ok_or(Declined::Budget)?;
        if total > max_bytes {
            return Err(Declined::Budget);
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(code.instructions.len())
            .map_err(|_| Declined::Allocation)?;
        let bytes = size::<Option<MemoryOp>>(entries.capacity(), std::mem::size_of::<Self>())
            .ok_or(Declined::Budget)?;
        if bytes > max_bytes {
            return Err(Declined::Budget);
        }
        for instruction in &code.instructions {
            let access = match ScalarKind::from_opcode(instruction.opcode) {
                None => None,
                Some(kind) => {
                    let Immediate::MemArg { memory, offset, .. } = instruction.immediate else {
                        return Err(Declined::Unsupported);
                    };
                    let index_type = types
                        .entries
                        .get(memory as usize)
                        .ok_or(Declined::Plan)?
                        .index_type;
                    Some(MemoryOp {
                        kind,
                        width: kind.width() as u8,
                        memory,
                        offset,
                        index_type,
                    })
                }
            };
            entries.push(access);
        }
        let result = Self {
            entries,
            bytes,
            work,
        };
        if !result.verify(function, module, types, max_work, max_bytes) {
            return Err(Declined::Verification);
        }
        Ok(Some(result))
    }
    pub(super) fn get(&self, pc: usize) -> Option<MemoryOp> {
        self.entries.get(pc).copied().flatten()
    }
    pub(super) fn verify(
        &self,
        function: &CompactFunction,
        module: &Module,
        types: &MemoryTypes,
        max_work: usize,
        max_bytes: usize,
    ) -> bool {
        if function.engine != module.data.engine
            || !Arc::ptr_eq(&function.source, &module.data.inner.decoded().bytes)
            || types.engine != module.data.engine
            || !Arc::ptr_eq(&types.source, &module.data.inner.decoded().bytes)
            || self.work > max_work.min(MAX_WORK)
            || self.bytes > max_bytes.min(MAX_BYTES)
            || size::<Option<MemoryOp>>(self.entries.capacity(), std::mem::size_of::<Self>())
                != Some(self.bytes)
        {
            return false;
        }
        let Some(code) = module.data.inner.decoded().code.get(function.body) else {
            return false;
        };
        if self.entries.len() != code.instructions.len()
            || self.entries.len() != function.ops.len()
            || code.instructions.len().checked_mul(3) != Some(self.work)
        {
            return false;
        }
        for (pc, (instruction, access)) in code.instructions.iter().zip(&self.entries).enumerate() {
            match (ScalarKind::from_opcode(instruction.opcode), access) {
                (None, None) => {}
                (Some(kind), Some(access)) => {
                    let Immediate::MemArg {
                        memory,
                        offset,
                        align,
                    } = instruction.immediate
                    else {
                        return false;
                    };
                    let Some(ty) = types.entries.get(memory as usize) else {
                        return false;
                    };
                    let Some(op) = function.ops.get(pc) else {
                        return false;
                    };
                    if op.opcode != instruction.opcode
                        || op.form != SIDE
                        || function.sides.get(op.aux as usize)
                            != Some(&SideImmediate::Memory {
                                align,
                                offset,
                                memory,
                            })
                        || access.kind != kind
                        || access.width as usize != kind.width()
                        || access.memory != memory
                        || access.offset != offset
                        || access.index_type != ty.index_type
                    {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
struct Binding {
    memory: Memory,
    index_type: IndexType,
    shared: bool,
    page_size_log2: u8,
}
#[derive(Debug)]
pub(crate) struct MemoryBindings {
    store: u64,
    engine: u64,
    source: Arc<[u8]>,
    entries: Vec<Binding>,
    work: usize,
    bytes: usize,
}
impl MemoryBindings {
    pub(crate) fn prepare(store: &Store, module: &Module, memories: &[Memory]) -> Option<Self> {
        Self::bounded(store, module, memories, MAX_WORK, MAX_BYTES)
    }
    fn bounded(
        store: &Store,
        module: &Module,
        memories: &[Memory],
        max_work: usize,
        max_bytes: usize,
    ) -> Option<Self> {
        if store.engine.id != module.data.engine {
            return None;
        }
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let types = MemoryTypes::prepare(module, max_work, max_bytes)?;
        if types.entries.len() != memories.len() {
            return None;
        }
        let work = types.work.checked_add(memories.len())?;
        if work > max_work {
            return None;
        }
        let available = max_bytes.checked_sub(types.bytes)?;
        if size::<Binding>(memories.len(), std::mem::size_of::<Self>())? > available {
            return None;
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(memories.len()).ok()?;
        let bytes = size::<Binding>(entries.capacity(), std::mem::size_of::<Self>())?;
        if bytes > available {
            return None;
        }
        for (memory, expected) in memories.iter().copied().zip(&types.entries) {
            let actual = &store.mems[store.check(memory.0, &store.mems).ok()?];
            if actual.ty.index_type != expected.index_type
                || actual.ty.shared != expected.shared
                || actual.ty.page_size_log2 != expected.page_size_log2
                || (actual.ty.shared && actual.shared.is_none())
            {
                return None;
            }
            entries.push(Binding {
                memory,
                index_type: actual.ty.index_type,
                shared: actual.ty.shared,
                page_size_log2: actual.ty.page_size_log2,
            });
        }
        Some(Self {
            store: store.id,
            engine: store.engine.id,
            source: module.data.inner.decoded().bytes.clone(),
            entries,
            work,
            bytes,
        })
    }
    fn resolve(&self, store: &Store, instance: &Inst, access: MemoryOp) -> Result<Memory> {
        if self.store != store.id
            || self.engine != store.engine.id
            || self.engine != instance.module.data.engine
            || !Arc::ptr_eq(&self.source, &instance.module.data.inner.decoded().bytes)
            || self.entries.len() != instance.mems.len()
            || self.work > MAX_WORK
            || self.bytes > MAX_BYTES
            || size::<Binding>(self.entries.capacity(), std::mem::size_of::<Self>()) != Some(self.bytes)
            || instance.module.data.inner.decoded().imports.len().checked_add(instance.module.data.inner.decoded().memories.len()).and_then(|n| n.checked_add(self.entries.len())) != Some(self.work)
            || access.width as usize != access.kind.width()
        {
            return Err(trap("compact memory binding identity"));
        }
        let binding = self
            .entries
            .get(access.memory as usize)
            .ok_or_else(|| trap("compact memory local index"))?;
        if instance.mems.get(access.memory as usize) != Some(&binding.memory)
            || binding.index_type != access.index_type
        {
            return Err(trap("compact memory target identity"));
        }
        let memory = &store.mems[store.check(binding.memory.0, &store.mems)?];
        if memory.ty.index_type != binding.index_type
            || memory.ty.shared != binding.shared
            || memory.ty.page_size_log2 != binding.page_size_log2
            || (binding.shared && memory.shared.is_none())
        {
            return Err(trap("compact memory binding class"));
        }
        Ok(binding.memory)
    }
}

impl Store {
    pub(crate) fn compact_memory_access(
        &mut self,
        instance: usize,
        access: MemoryOp,
        stack: &mut Vec<Value>,
    ) -> Result<()> {
        // Preserve decoded ordering: pop store value, then address and checked
        // offset addition, then resolve/check the whole range, then store bits.
        let value = if access.kind.store() {
            Some(pop(stack)?)
        } else {
            None
        };
        let offset = address(pop(stack)?)?
            .checked_add(access.offset)
            .ok_or_else(|| trap("address overflow"))?;
        let inst = &self.instances[instance];
        let memory = inst
            .memory_bindings
            .as_ref()
            .ok_or_else(|| trap("missing compact memory bindings"))?
            .resolve(self, inst, access)?;
        let live = self.check(memory.0, &self.mems)?;
        let mem = &mut self.mems[live];
        let width = access.width as usize;
        let whole = range(offset, width, mem.len()?)?;
        #[cfg(feature = "execution-reference")]
        {
            self.compact_memory_accesses = self.compact_memory_accesses.saturating_add(1);
        }
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::CompactMemoryAccess, 1);
        if let Some(value) = value {
            let bits = match value {
                Value::I32(value) => value as u32 as u64,
                Value::I64(value) => value as u64,
                Value::F32(value) => value as u64,
                Value::F64(value) => value,
                _ => return Err(trap("invalid store value")),
            };
            if mem.shared.is_some() {
                mem.write(offset, &bits.to_le_bytes()[..width])?;
                #[cfg(feature = "performance-counters")]
                self.perf.record(perf::Event::CompactSharedMemoryAccess, 1);
            } else {
                mem.bytes.write(whole.start, &bits.to_le_bytes()[..width]);
            }
        } else {
            let mut bytes = [0u8; 8];
            if mem.shared.is_some() {
                bytes[..width].copy_from_slice(&mem.read(offset, width)?);
                #[cfg(feature = "performance-counters")]
                {
                    self.perf.record(perf::Event::ReadVec, 1);
                    self.perf.record(perf::Event::CompactSharedMemoryAccess, 1);
                }
            } else {
                mem.bytes.read(whole.start, &mut bytes[..width]);
                #[cfg(feature = "performance-counters")]
                self.perf.record(perf::Event::ReadFixed, 1);
            }
            stack.push(access.kind.value(u64::from_le_bytes(bytes)));
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "execution-reference"))]
mod p043_private {
    use super::*;
    include!("../../../tests/performance/p043_private.rs");
}
