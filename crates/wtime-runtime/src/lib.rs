#![forbid(unsafe_code)]
//! Portable, explicitly framed WebAssembly execution.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
pub use wtime_types::*;
use wtime_validate::{
    ExternalKind, Immediate, ImportKind, Instruction, SegmentMode, ValidatedModule,
};
#[cfg(feature = "performance-counters")]
mod perf;
#[cfg(feature = "performance-counters")]
pub use perf::CounterSnapshot;
#[cfg(all(feature = "register-executor", feature = "execution-reference"))]
mod register_cache_diagnostics;
#[cfg(all(feature = "register-executor", feature = "execution-reference"))]
mod register_activation_diagnostics;
#[cfg(all(feature = "register-executor", feature = "execution-reference"))]
pub use register_activation_diagnostics::RegisterActivationSnapshot;
#[cfg(all(feature = "register-executor", feature = "execution-reference"))]
pub use register_cache_diagnostics::{
    RegisterCacheEntrySnapshot, RegisterCacheEntryState, RegisterCacheLifecycle,
    RegisterCacheSnapshot,
};
mod continuation;
mod exceptions;
mod linear;
mod gc;
mod gc_exec;
mod numeric;
mod shared_memory;
mod simd;
mod wide;
pub use exceptions::{ExceptionRef, Tag};
mod arena;
mod canonical;
pub use canonical::CanonicalTypeId;
pub use gc::{CollectionStats, GcRef, RootToken};
static IDS: AtomicU64 = AtomicU64::new(1);
fn err(k: ErrorKind, s: impl Into<String>) -> Error {
    Error::new(k, s)
}
fn trap(s: impl Into<String>) -> Error {
    let message = s.into();
    let code = if message == "unreachable" {
        TrapCode::Unreachable
    } else if message.contains("divide by zero") {
        TrapCode::IntegerDivisionByZero
    } else if message.contains("integer overflow") {
        TrapCode::IntegerOverflow
    } else if message.contains("integer conversion") {
        TrapCode::InvalidConversionToInteger
    } else if message.contains("indirect call type") || message.contains("call_ref type") {
        TrapCode::IndirectCallTypeMismatch
    } else if message.contains("null") {
        TrapCode::NullReference
    } else if message.contains("table out of bounds") {
        TrapCode::TableOutOfBounds
    } else if message.contains("out of bounds") || message.contains("address overflow") {
        TrapCode::MemoryOutOfBounds
    } else {
        TrapCode::Other
    };
    Error::trap(code, message)
}
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub features: FeatureSet,
    pub limits: ResourceLimits,
}
/// Explicit execution request. Unimplemented tiers fail during configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecutionPolicy {
    #[default]
    Decoded,
    Reference,
    Compact,
    Register,
    Experimental,
    Native,
    /// The selected portable tier; currently decoded.
    Portable,
}
#[derive(Clone)]
pub struct Engine {
    id: u64,
    policy: ExecutionPolicy,
    #[cfg(feature = "register-executor")]
    register_entry_threshold: Option<u64>,
    config: Config,
    canonical: Arc<std::sync::Mutex<canonical::CanonicalTypes>>,
}
impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Engine").field("id", &self.id).finish()
    }
}
impl Engine {
    pub fn new(config: Config) -> Result<Self> {
        Self::with_execution_policy(config, ExecutionPolicy::Decoded)
    }
    pub fn with_execution_policy(config: Config, policy: ExecutionPolicy) -> Result<Self> {
        let policy = match policy {
            ExecutionPolicy::Portable => ExecutionPolicy::Decoded,
            other => other,
        };
        match policy {
            ExecutionPolicy::Decoded => {}
            #[cfg(feature = "compact-executor")]
            ExecutionPolicy::Compact => {}
            #[cfg(feature = "register-executor")]
            ExecutionPolicy::Register => {}
            #[cfg(feature = "execution-reference")]
            ExecutionPolicy::Reference => {}
            _ => {
                return Err(err(
                    ErrorKind::DisabledFeature,
                    format!("unavailable execution policy: {policy:?}"),
                ));
            }
        }
        config.features.validate()?;
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let mut canonical = canonical::CanonicalTypes::new(id);
        canonical.set_limit(config.limits.max_canonical_types);
        canonical.set_byte_limit(config.limits.max_canonical_type_bytes);
        Ok(Self {
            id,
            policy,
            #[cfg(feature = "register-executor")]
            register_entry_threshold: None,
            config,
            canonical: Arc::new(std::sync::Mutex::new(canonical)),
        })
    }
    pub fn execution_policy(&self) -> ExecutionPolicy {
        self.policy
    }
    /// Opt-in Register preparation at function entry. A positive threshold
    /// counts entries across Stores sharing a Module; cold entries use decoded
    /// execution. The ordinary Register constructor prepares eagerly.
    #[cfg(feature = "register-executor")]
    pub fn with_register_entry_threshold(config: Config, threshold: u64) -> Result<Self> {
        if threshold == 0 {
            return Err(err(ErrorKind::Host, "register entry threshold must be positive"));
        }
        let mut engine = Self::with_execution_policy(config, ExecutionPolicy::Register)?;
        engine.register_entry_threshold = Some(threshold);
        Ok(engine)
    }
    pub fn compile(&self, bytes: &[u8]) -> Result<Module> {
        let validated =
            wtime_validate::validate(bytes, &self.config.features, &self.config.limits)?;
        let mut ids = vec![];
        let mut registry = self
            .canonical
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?;
        for group in &validated.decoded().type_groups {
            ids.extend(registry.intern_group(group, &ids, self.config.limits.max_type_work)?)
        }
        drop(registry);
        let control = lower_control(&validated)?;
        #[cfg(not(feature = "execution-reference"))]
        let lowered = (0..validated.decoded().code.len()).map(|_| std::sync::OnceLock::new()).collect();
        let plans = execution::plan::FunctionPlans::prepare(&validated);
        let module = Module {
            data: Arc::new(ModuleData {
                engine: self.id,
                plans,
                #[cfg(feature = "compact-executor")]
                compact: None,
                #[cfg(feature = "register-executor")]
                register: None,
                #[cfg(feature = "register-executor")]
                register_cache: None,
                control,
                #[cfg(not(feature = "execution-reference"))]
                lowered,
                canonical: ids,
                inner: validated,
            }),
        };
        #[cfg(feature = "compact-executor")]
        let module = {
            let mut module = module;
            if self.policy == ExecutionPolicy::Compact {
                let prepared = execution::compact::PreparedModule::prepare(&module);
                // Prepared code retains source bytes, never this ModuleData Arc.
                if let Some(data) = Arc::get_mut(&mut module.data) {
                    data.compact = prepared;
                }
            }
            module
        };
        #[cfg(feature = "register-executor")]
        let module = {
            let mut module = module;
            if self.policy == ExecutionPolicy::Register {
                if let Some(threshold) = self.register_entry_threshold {
                    let cache = execution::register::Cache::with_threshold(&module, threshold);
                    if let Some(data) = Arc::get_mut(&mut module.data) { data.register_cache = cache; }
                } else {
                    let prepared = execution::register::PreparedModule::prepare(&module);
                    if let Some(data) = Arc::get_mut(&mut module.data) { data.register = prepared; }
                }
            }
            module
        };
        Ok(module)
    }
}

/// For each function body and instruction: where a block, loop, if,
/// try_table or try that starts there ends, and where its else, catch or
/// delegate is. Stored as two u32s per instruction (8 bytes, not the 24 of
/// `Option<(usize, Option<usize>)>`): a large module holds millions.
#[derive(Debug)]
struct ControlMap {
    bodies: Vec<Box<[ControlEntry]>>,
}
#[derive(Clone, Copy, Debug)]
struct ControlEntry {
    end: u32,
    other: u32,
}
impl ControlEntry {
    const NONE: u32 = u32::MAX;
    const EMPTY: Self = Self { end: Self::NONE, other: Self::NONE };
}
impl ControlMap {
    /// The end and the else/catch position of the block starting at `pc` of
    /// `body`; `None` when no block starts there.
    #[inline]
    fn get(&self, body: usize, pc: usize) -> Option<(usize, Option<usize>)> {
        let entry = self.bodies.get(body)?.get(pc)?;
        (entry.end != ControlEntry::NONE)
            .then(|| (entry.end as usize, (entry.other != ControlEntry::NONE).then_some(entry.other as usize)))
    }
    fn bytes(&self) -> usize {
        self.bodies.iter().map(|body| std::mem::size_of_val::<[ControlEntry]>(body)).sum()
    }
    /// Test support: overwrite one entry, to corrupt metadata on purpose.
    #[cfg(test)]
    #[allow(dead_code)]
    fn set(&mut self, body: usize, pc: usize, value: Option<(usize, Option<usize>)>) {
        self.bodies[body][pc] = match value {
            Some((end, other)) => ControlEntry {
                end: u32::try_from(end).expect("test control position fits u32"),
                other: other.map_or(ControlEntry::NONE, |other| u32::try_from(other).expect("test control position fits u32")),
            },
            None => ControlEntry::EMPTY,
        };
    }
}
#[derive(Debug)]
struct ModuleData {
    engine: u64,
    plans: Option<execution::plan::FunctionPlans>,
    #[cfg(feature = "compact-executor")]
    compact: Option<execution::compact::PreparedModule>,
    #[cfg(feature = "register-executor")]
    register: Option<execution::register::PreparedModule>,
    #[cfg(feature = "register-executor")]
    register_cache: Option<Arc<execution::register::Cache>>,
    control: ControlMap,
    /// Lowered bodies for straight-line runs, built on first use and shared
    /// by every store using the module.
    #[cfg(not(feature = "execution-reference"))]
    lowered: Box<[std::sync::OnceLock<Box<[execution::lowered::Op]>>]>,
    canonical: Vec<CanonicalTypeId>,
    inner: ValidatedModule,
}
/// An immutable Engine-bound executable payload. Clones retain the same storage.
#[derive(Clone, Debug)]
pub struct Module {
    data: Arc<ModuleData>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Handle {
    store: u64,
    index: usize,
    generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Func(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Memory(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Table(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Global(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Instance(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExternRef(Handle);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefValue {
    Null(HeapType),
    Func(Func),
    Gc(GcRef),
    I31(i32),
    Extern(ExternRef),
    InternalExtern(ExternRef),
    ExternI31(i32),
    ExternGc(GcRef),
    Exn(ExceptionRef),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    V128(V128),
    Ref(RefValue),
}
impl Value {
    pub fn ty(self) -> ValType {
        match self {
            Self::I32(_) => ValType::I32,
            Self::I64(_) => ValType::I64,
            Self::F32(_) => ValType::F32,
            Self::F64(_) => ValType::F64,
            Self::V128(_) => ValType::V128,
            Self::Ref(RefValue::Null(h)) => ValType::Ref(RefType::new(true, h)),
            Self::Ref(RefValue::Func(_)) => ValType::FUNCREF,
            Self::Ref(RefValue::Gc(_)) => ValType::Ref(RefType::new(false, HeapType::Any)),
            Self::Ref(RefValue::Exn(_)) => ValType::Ref(RefType::new(false, HeapType::Exn)),
            Self::Ref(RefValue::Extern(_) | RefValue::ExternI31(_) | RefValue::ExternGc(_)) => {
                ValType::EXTERNREF
            }
            Self::Ref(RefValue::InternalExtern(_)) => {
                ValType::Ref(RefType::new(false, HeapType::Any))
            }
            Self::Ref(RefValue::I31(_)) => ValType::Ref(RefType::new(false, HeapType::I31)),
        }
    }
    fn zero(t: ValType) -> Self {
        match t {
            ValType::I32 => Self::I32(0),
            ValType::I64 => Self::I64(0),
            ValType::F32 => Self::F32(0),
            ValType::F64 => Self::F64(0),
            ValType::V128 => Self::V128(V128::default()),
            ValType::Ref(r) => Self::Ref(RefValue::Null(r.heap)),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum External {
    Tag(Tag),
    Func(Func),
    Memory(Memory),
    Table(Table),
    Global(Global),
}
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Suspended {
        tag: Tag,
        payload: Vec<Value>,
        continuation: GcRef,
    },
    Returned(Vec<Value>),
    Exception(ExceptionRef),
}
type HostCallback = Arc<dyn Fn(&mut Store, &[Value]) -> Result<RunOutcome> + Send + Sync>;
#[derive(Clone)]
struct Function {
    host_type_instance: Option<usize>,
    host_type_module: Option<Module>,
    canonical: CanonicalTypeId,
    /// Shared so that a frame's copy of its function is cheap to make.
    ty: Arc<FuncType>,
    instance: usize,
    body: usize,
    host: Option<HostCallback>,
}
struct Mem {
    ty: MemoryType,
    bytes: linear::LinearBytes,
    shared: Option<shared_memory::SharedMemory>,
}
impl Mem {
    fn len(&self) -> Result<usize> {
        if let Some(m) = &self.shared {
            m.len()
        } else {
            Ok(self.bytes.len())
        }
    }
    fn new(ty: MemoryType, initial: usize) -> Result<Self> {
        let shared = if ty.shared {
            Some(shared_memory::SharedMemory::new(
                initial,
                ty.limits
                    .max
                    .unwrap_or(65536)
                    .checked_mul(1u64 << ty.page_size_log2)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| err(ErrorKind::ResourceLimit, "shared memory maximum"))?,
            )?)
        } else {
            None
        };
        let bytes = if shared.is_none() {
            linear::LinearBytes::zeroed(initial)
                .ok_or_else(|| err(ErrorKind::ResourceLimit, "memory allocation failed"))?
        } else {
            linear::LinearBytes::default()
        };
        Ok(Self { ty, bytes, shared })
    }
    /// Grows an unshared memory to `bytes`; false when it cannot be allocated.
    fn grow_unshared(&mut self, bytes: usize) -> bool {
        self.bytes.grow_to(bytes)
    }
    fn read(&self, off: u64, n: usize) -> Result<Vec<u8>> {
        if let Some(m) = &self.shared {
            m.read(off, n)
        } else {
            let r = range(off, n, self.bytes.len())?;
            let mut out = vec![0; n];
            self.bytes.read(r.start, &mut out);
            Ok(out)
        }
    }
    /// Reads `out.len()` bytes at `off` without allocating.
    #[inline]
    fn read_into(&self, off: u64, out: &mut [u8]) -> Result<()> {
        if let Some(m) = &self.shared {
            out.copy_from_slice(&m.read(off, out.len())?);
        } else {
            let r = range(off, out.len(), self.bytes.len())?;
            self.bytes.read(r.start, out);
        }
        Ok(())
    }
    fn write(&mut self, off: u64, b: &[u8]) -> Result<()> {
        if let Some(m) = &self.shared {
            m.write(off, b)
        } else {
            let r = range(off, b.len(), self.bytes.len())?;
            self.bytes.write(r.start, b).map_err(allocation_failed)
        }
    }
}
/// Host storage for a linear memory could not be allocated.
fn allocation_failed(_: linear::AllocationFailed) -> Error {
    err(ErrorKind::ResourceLimit, "memory allocation failed")
}
struct Tab {
    instance: Option<usize>,
    ty: TableType,
    values: Vec<Value>,
}
struct Glob {
    instance: Option<usize>,
    ty: GlobalType,
    value: Value,
}
#[derive(Clone)]
struct Inst {
    #[cfg(feature = "compact-executor")]
    memory_bindings: Option<Arc<execution::compact::MemoryBindings>>,
    #[cfg(feature = "compact-executor")]
    call_sites: Option<execution::compact::CallSites>,
    #[cfg(feature = "compact-executor")]
    call_bindings: Option<Arc<execution::compact::CallBindings>>,
    tags: Vec<Tag>,
    module: Module,
    funcs: Vec<Func>,
    mems: Vec<Memory>,
    tabs: Vec<Table>,
    globals: Vec<Global>,
    exports: HashMap<String, External>,
    data: Vec<Option<Vec<u8>>>,
    elems: Vec<Option<Vec<Value>>>,
}
/// An affine host root; clones retain the same registration until the last drop.
#[derive(Clone)]
pub struct ValueRootToken(Arc<Value>);
impl ValueRootToken {
    pub fn value(&self) -> Value {
        *self.0
    }
}

pub struct Store {
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    register_activation: register_activation_diagnostics::Diagnostics,
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    register_dispatches: u64,
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    register_exits: u64,
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    register_fallback_frames: std::cell::Cell<u64>,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_dispatches: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_prefix_dispatches: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_fused_groups: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_inline_leaf_calls: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_inline_declines: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_exits: [u64; 4],
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_last_exit: Option<(usize, u16)>,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_bound_calls: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_memory_accesses: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_indirect_hits: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_indirect_misses: u64,
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    compact_indirect_evictions: u64,
    #[cfg(feature = "execution-reference")]
    reference_dispatches: u64,
    #[cfg(feature = "execution-reference")]
    materialization_request: Option<(usize, usize, usize, usize)>,
    #[cfg(feature = "execution-reference")]
    execution_snapshot: Option<DiagnosticSnapshot>,
    #[cfg(feature = "execution-reference")]
    materialization_roundtrips: u64,
    #[cfg(feature = "performance-counters")]
    perf: perf::Counters,
    current_fuel: Option<u64>,
    resume_fuel: Option<Option<u64>>,
    cancellation: CancellationToken,
    #[cfg(test)]
    bulk_cancel_after: Option<usize>,
    pending_suspension: Option<(Tag, Vec<Value>, GcRef)>,
    tag_instances: HashMap<usize, usize>,
    tag_modules: HashMap<usize, Module>,
    tag_canonical: HashMap<usize, CanonicalTypeId>,
    continuations: HashMap<GcRef, continuation::ContinuationState>,
    tags: arena::Arena<FuncType>,
    exceptions: arena::Arena<exceptions::Exception>,
    externs: HashMap<u32, ExternRef>,
    extern_objects: arena::Arena<(u32, Option<Arc<dyn std::any::Any + Send + Sync>>)>,
    active_roots: Vec<Vec<Value>>,
    published: Vec<Value>,
    value_roots: std::sync::Mutex<Vec<std::sync::Weak<Value>>>,
    gc: gc::GcArena,
    id: u64,
    engine: Engine,
    funcs: arena::Arena<Function>,
    mems: arena::Arena<Mem>,
    tabs: arena::Arena<Tab>,
    globals: arena::Arena<Glob>,
    instances: arena::Arena<Inst>,
}
impl Store {
    /// Working-vector capacity sampled at source and callback boundaries.
    /// Excludes compiled code, generic frame locals, and allocator overhead.
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn register_activation_snapshot(&self) -> RegisterActivationSnapshot {
        self.register_activation.snapshot()
    }
    /// Start a new observation only when no decoded invocation scope is active.
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn reset_register_activation_snapshot(&mut self) -> Result<()> {
        self.register_activation.reset()
    }
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn register_dispatches(&self) -> u64 { self.register_dispatches }
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn register_exits(&self) -> u64 { self.register_exits }
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn register_fallback_frames(&self) -> u64 { self.register_fallback_frames.get() }
    /// Development-only proof of executed compact source events.
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_dispatches(&self) -> u64 { self.compact_dispatches }
    /// Development-only count of executed, verified arithmetic expansions.
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_fused_groups(&self) -> u64 { self.compact_fused_groups }
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_inline_leaf_calls(&self) -> u64 { self.compact_inline_leaf_calls }
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_inline_declines(&self) -> u64 { self.compact_inline_declines }
    /// Diagnostic prefix events, GC/EH/continuation/other exits, and last source exit.
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_prefix_counts(&self) -> (u64, [u64; 4], Option<(usize, u16)>) {
        (self.compact_prefix_dispatches, self.compact_exits, self.compact_last_exit)
    }
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_bound_calls(&self) -> u64 { self.compact_bound_calls }
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_memory_accesses(&self) -> u64 { self.compact_memory_accesses }
    #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
    pub fn compact_indirect_cache_counts(&self) -> (u64, u64, u64) {
        (self.compact_indirect_hits, self.compact_indirect_misses, self.compact_indirect_evictions)
    }
    /// Development-only proof that reference source instructions executed.
    #[cfg(feature = "execution-reference")]
    pub fn reference_dispatches(&self) -> u64 {
        self.reference_dispatches
    }
    #[cfg(feature = "performance-counters")]
    pub fn performance_counters(&self) -> CounterSnapshot {
        self.perf.snapshot()
    }
    #[cfg(feature = "performance-counters")]
    pub fn reset_performance_counters(&self) {
        self.perf.reset();
    }
    pub fn new(e: &Engine) -> Self {
        {
            let id = IDS.fetch_add(1, Ordering::Relaxed);
            let mut gc = gc::GcArena::new(id);
            gc.set_limits(
                e.config.limits.max_gc_bytes,
                e.config.limits.max_gc_mark_work,
            );
            Self {
                #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
                register_activation: register_activation_diagnostics::Diagnostics::default(),
                #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
                register_dispatches: 0,
                #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
                register_exits: 0,
                #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
                register_fallback_frames: std::cell::Cell::new(0),
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_dispatches: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_prefix_dispatches: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_fused_groups: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_inline_leaf_calls: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_inline_declines: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_exits: [0; 4],
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_last_exit: None,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_bound_calls: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_memory_accesses: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_indirect_hits: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_indirect_misses: 0,
                #[cfg(all(feature = "compact-executor", feature = "execution-reference"))]
                compact_indirect_evictions: 0,
                #[cfg(feature = "execution-reference")]
                reference_dispatches: 0,
                #[cfg(feature = "execution-reference")]
                materialization_request: None,
                #[cfg(feature = "execution-reference")]
                execution_snapshot: None,
                #[cfg(feature = "execution-reference")]
                materialization_roundtrips: 0,
                #[cfg(feature = "performance-counters")]
                perf: perf::Counters::default(),
                current_fuel: None,
                resume_fuel: None,
                cancellation: CancellationToken::default(),
                #[cfg(test)]
                bulk_cancel_after: None,
                pending_suspension: None,
                tag_instances: HashMap::new(),
                tag_modules: HashMap::new(),
                tag_canonical: HashMap::new(),
                continuations: HashMap::new(),
                tags: arena::Arena::new(),
                exceptions: arena::Arena::new(),
                externs: HashMap::new(),
                extern_objects: arena::Arena::new(),
                active_roots: vec![],
                published: vec![],
                value_roots: std::sync::Mutex::new(vec![]),
                gc,
                id,
                engine: e.clone(),
                funcs: arena::Arena::new(),
                mems: arena::Arena::new(),
                tabs: arena::Arena::new(),
                globals: arena::Arena::new(),
                instances: arena::Arena::new(),
            }
        }
    }
    fn h(&self, index: usize) -> Handle {
        Handle {
            store: self.id,
            index,
            generation: 0,
        }
    }
    fn check<T>(&self, h: Handle, arena: &arena::Arena<T>) -> Result<usize> {
        if h.store != self.id || arena.generation(h.index) != Some(h.generation) {
            Err(err(ErrorKind::Host, "foreign or stale handle"))
        } else {
            Ok(h.index)
        }
    }
    pub fn get_global(&self, g: Global) -> Result<Value> {
        Ok(self.globals[self.check(g.0, &self.globals)?].value)
    }
    /// `get_global` with one arena lookup.
    #[cfg(not(feature = "execution-reference"))]
    #[inline(always)]
    fn global_value(&self, g: Global) -> Result<Value> {
        match self.globals.get_live(g.0.index, g.0.generation) {
            Some(slot) if g.0.store == self.id => Ok(slot.value),
            _ => Err(err(ErrorKind::Host, "foreign or stale handle")),
        }
    }
    pub fn set_global(&mut self, g: Global, v: Value) -> Result<()> {
        self.validate_value(v)?;
        let i = self.check(g.0, &self.globals)?;
        let matches = if let (Some(ii), ValType::Ref(r)) =
            (self.globals[i].instance, self.globals[i].ty.value)
        {
            self.gc_matches(ii, v, r)?
        } else {
            matches_type(v, self.globals[i].ty.value)
        };
        if !self.globals[i].ty.mutable || !matches {
            return Err(err(ErrorKind::Host, "global type or mutability mismatch"));
        }
        self.globals[i].value = v;
        Ok(())
    }
    pub fn read_memory(&self, m: Memory, offset: u64, len: usize) -> Result<Vec<u8>> {
        #[cfg(feature = "performance-counters")]
        self.perf.record(perf::Event::ReadVec, 1);
        let i = self.check(m.0, &self.mems)?;
        self.mems[i].read(offset, len)
    }
}
impl Instance {
    pub fn export(self, s: &Store, name: &str) -> Result<External> {
        let i = s.check(self.0, &s.instances)?;
        s.instances[i]
            .exports
            .get(name)
            .copied()
            .ok_or_else(|| err(ErrorKind::Link, "unknown export"))
    }
    pub fn exports(self, s: &Store) -> Result<Vec<(String, External)>> {
        let i = s.check(self.0, &s.instances)?;
        Ok(s.instances[i]
            .exports
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect())
    }
}
pub struct Linker {
    engine: u64,
    defs: HashMap<(String, String), External>,
}
mod linker;

fn limits_match(a: Limits, b: Limits) -> bool {
    a.min >= b.min && b.max.is_none_or(|m| a.max.is_some_and(|x| x <= m))
}
fn matches_type(v: Value, t: ValType) -> bool {
    v.ty() == t
        || matches!(
            (v, t),
            (
                Value::Ref(RefValue::Func(_)),
                ValType::Ref(RefType {
                    heap: HeapType::Func,
                    ..
                })
            )
        )
}
fn range(off: u64, n: usize, len: usize) -> Result<std::ops::Range<usize>> {
    let a = usize::try_from(off).map_err(|_| trap("out of bounds"))?;
    let b = a.checked_add(n).ok_or_else(|| trap("out of bounds"))?;
    if b > len {
        return Err(trap("out of bounds"));
    }
    Ok(a..b)
}
fn address(v: Value) -> Result<u64> {
    match v {
        Value::I32(x) => Ok(x as u32 as u64),
        Value::I64(x) => Ok(x as u64),
        _ => Err(trap("expected integer address")),
    }
}
fn pop(s: &mut Vec<Value>) -> Result<Value> {
    s.pop().ok_or_else(|| trap("operand stack underflow"))
}
fn eval_const(s: &mut Store, i: &Inst, ops: &[Instruction]) -> Result<Value> {
    let mut v = vec![];
    for op in ops {
        #[cfg(feature = "opcode-coverage")]
        crate::coverage::record(op.opcode);
        match (&op.opcode, &op.immediate) {
            (0x41, Immediate::I32(x)) => v.push(Value::I32(*x)),
            (0x42, Immediate::I64(x)) => v.push(Value::I64(*x)),
            (0x43, Immediate::F32(x)) => v.push(Value::F32(*x)),
            (0x44, Immediate::F64(x)) => v.push(Value::F64(*x)),
            (0x23, Immediate::Index(x)) => v.push(s.get_global(i.globals[*x as usize])?),
            (0xd0, Immediate::Heap(h)) => v.push(Value::Ref(RefValue::Null(*h))),
            (0xd2, Immediate::Index(x)) => v.push(Value::Ref(RefValue::Func(i.funcs[*x as usize]))),
            (0xfb00..=0xfb26, _) => {
                let ii = s.instances.len();
                s.instances.push(i.clone());
                let result = s.gc_exec(ii, op, &mut v);
                s.instances.pop_unpublished();
                result?;
            }
            (0xb, _) => {}
            (n, _) if n & 0xff00 == 0xfd00 => simd::execute(*n, &op.immediate, &mut v)?,
            _ => numeric::exec(op.opcode, &mut v)?,
        }
    }
    if v.len() != 1 {
        return Err(trap("invalid constant expression result"));
    }
    Ok(v[0])
}
mod execution;
#[cfg(feature = "execution-reference")]
mod reference;
#[cfg(feature = "execution-reference")]
pub use execution::cursor::{
    DiagnosticFrame, DiagnosticSnapshot, DiagnosticValue, ExecutionCursor,
};

impl Store {
    pub fn create_host_func(
        &mut self,
        ty: FuncType,
        callback: impl Fn(&[Value]) -> Result<Vec<Value>> + Send + Sync + 'static,
    ) -> Result<Func> {
        self.reserve_objects(1)?;
        let f = Func(self.h(self.funcs.len()));
        self.funcs.push(Function {
            host_type_instance: None,
            host_type_module: None,
            canonical: self.canonical_func(&ty)?,
            ty: Arc::new(ty),
            instance: 0,
            body: 0,
            host: Some(Arc::new(move |_, args| {
                callback(args).map(RunOutcome::Returned)
            })),
        });
        Ok(f)
    }
    pub fn create_global(&mut self, ty: GlobalType, value: Value) -> Result<Global> {
        if matches!(
            ty.value,
            ValType::Ref(RefType {
                heap: HeapType::Defined(_) | HeapType::ExactDefined(_),
                ..
            })
        ) {
            return Err(err(
                ErrorKind::Host,
                "host global requires an abstract reference type",
            ));
        }
        self.reserve_objects(1)?;
        self.validate_value(value)?;
        let valid = match ty.value {
            ValType::Ref(r) => self.gc_matches(usize::MAX, value, r)?,
            _ => matches_type(value, ty.value),
        };
        if !valid {
            return Err(err(ErrorKind::Host, "global type mismatch"));
        }
        let g = Global(self.h(self.globals.len()));
        self.globals.push(Glob {
            instance: None,
            ty,
            value,
        });
        Ok(g)
    }
    pub fn create_memory(&mut self, ty: MemoryType) -> Result<Memory> {
        self.reserve_objects(1)?;
        let features = self.engine.config.features;
        if (ty.shared && !features.threads)
            || (ty.index_type == IndexType::I64 && !features.memory64)
            || (ty.page_size_log2 != 16 && !features.custom_page_sizes)
        {
            return Err(err(
                ErrorKind::DisabledFeature,
                "host memory feature disabled",
            ));
        }
        if ty.shared && ty.limits.max.is_none() {
            return Err(err(ErrorKind::Host, "shared memory requires maximum"));
        }
        if !matches!(ty.page_size_log2, 0 | 16)
            || !ty.limits.is_valid()
            || ty.limits.min > self.engine.config.limits.max_memory_pages
        {
            return Err(err(ErrorKind::ResourceLimit, "memory limits"));
        }
        let n = ty
            .limits
            .min
            .checked_mul(1u64.checked_shl(ty.page_size_log2 as u32).unwrap_or(0))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "memory size"))?;
        self.reserve_memory(n as u64)?;
        if n as u64 > self.engine.config.limits.max_memory_bytes {
            return Err(err(ErrorKind::ResourceLimit, "memory byte limit"));
        }

        let m = Memory(self.h(self.mems.len()));
        self.mems.push(Mem::new(ty, n)?);
        Ok(m)
    }
    pub fn read_table(&self, table: Table, index: u64) -> Result<Value> {
        let i = self.check(table.0, &self.tabs)?;
        self.tabs[i]
            .values
            .get(usize::try_from(index).map_err(|_| trap("table out of bounds"))?)
            .copied()
            .ok_or_else(|| trap("table out of bounds"))
    }
    pub fn write_table(&mut self, table: Table, index: u64, value: Value) -> Result<()> {
        let i = self.check(table.0, &self.tabs)?;
        self.validate_value(value)?;
        let valid = self.gc_matches(
            self.tabs[i].instance.unwrap_or(usize::MAX),
            value,
            self.tabs[i].ty.element,
        )?;
        if !valid {
            return Err(err(ErrorKind::Host, "table value type mismatch"));
        }
        let slot = self.tabs[i]
            .values
            .get_mut(usize::try_from(index).map_err(|_| trap("table out of bounds"))?)
            .ok_or_else(|| trap("table out of bounds"))?;
        *slot = value;
        Ok(())
    }
    pub fn create_table(&mut self, ty: TableType) -> Result<Table> {
        if matches!(
            ty.element.heap,
            HeapType::Defined(_) | HeapType::ExactDefined(_)
        ) {
            return Err(err(
                ErrorKind::Host,
                "host table requires an abstract reference type",
            ));
        }
        self.reserve_objects(1)?;
        let features = self.engine.config.features;
        if (ty.index_type == IndexType::I64 && !features.memory64)
            || (ty.element.heap != HeapType::Func && !features.reference_types)
            || (matches!(
                ty.element.heap,
                HeapType::Defined(_) | HeapType::ExactDefined(_)
            ) && !features.function_references)
        {
            return Err(err(
                ErrorKind::DisabledFeature,
                "host table feature disabled",
            ));
        }
        if !ty.element.nullable {
            return Err(err(ErrorKind::Host, "table element has no default value"));
        }
        if !ty.limits.is_valid() || ty.limits.min > self.engine.config.limits.max_table_elements {
            return Err(err(ErrorKind::ResourceLimit, "table limits"));
        }
        self.reserve_table(ty.limits.min)?;
        let mut values = Vec::new();
        let n = usize::try_from(ty.limits.min)
            .map_err(|_| err(ErrorKind::ResourceLimit, "table size"))?;
        values
            .try_reserve_exact(n)
            .map_err(|_| err(ErrorKind::ResourceLimit, "table allocation"))?;
        values.resize(n, Value::zero(ValType::Ref(ty.element)));
        let t = Table(self.h(self.tabs.len()));
        self.tabs.push(Tab {
            instance: None,
            ty,
            values,
        });
        Ok(t)
    }
    pub fn write_memory(&mut self, m: Memory, offset: u64, bytes: &[u8]) -> Result<()> {
        let i = self.check(m.0, &self.mems)?;
        let r = range(offset, bytes.len(), self.mems[i].len()?)?;
        let _ = r;
        self.mems[i].write(offset, bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn module(code: &[u8], params: &[u8], results: &[u8]) -> Vec<u8> {
        fn sec(out: &mut Vec<u8>, id: u8, p: &[u8]) {
            out.push(id);
            out.push(p.len() as u8);
            out.extend(p)
        }
        let mut out = b"\0asm\x01\0\0\0".to_vec();
        let mut t = vec![1, 0x60, params.len() as u8];
        t.extend(params);
        t.push(results.len() as u8);
        t.extend(results);
        sec(&mut out, 1, &t);
        sec(&mut out, 3, &[1, 0]);
        sec(&mut out, 7, &[1, 1, b'f', 0, 0]);
        let mut c = vec![1, (code.len() + 1) as u8, 0];
        c.extend(code);
        sec(&mut out, 10, &c);
        out
    }
    fn run(bytes: &[u8], args: &[Value]) -> Result<RunOutcome> {
        let e = Engine::new(Config::default())?;
        let m = e.compile(bytes)?;
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m)?;
        let External::Func(f) = i.export(&s, "f")? else {
            panic!()
        };
        s.invoke(f, args)
    }
    #[test]
    fn t050_integer_add_wraps() {
        assert_eq!(
            run(
                &module(&[0x20, 0, 0x41, 1, 0x6a, 0xb], &[0x7f], &[0x7f]),
                &[Value::I32(i32::MAX)]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(i32::MIN)])
        );
    }
    fn run_with(bytes: &[u8], args: &[Value], limits: ResourceLimits) -> Result<RunOutcome> {
        let e = Engine::new(Config { limits, ..Config::default() })?;
        let m = e.compile(bytes)?;
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m)?;
        let External::Func(f) = i.export(&s, "f")? else {
            panic!()
        };
        s.invoke(f, args)
    }
    /// Counting loop: block, loop, then per iteration local.get, i32.eqz,
    /// br_if 1, local.get, i32.const 1, i32.sub, local.set, br 0; the exit
    /// takes local.get, i32.eqz, br_if 1, then local.get and the function end.
    /// Every executed instruction costs one unit of fuel, whichever path of
    /// the interpreter runs it.
    #[test]
    fn straight_line_runs_charge_fuel_per_instruction() {
        let code = [
            0x02, 0x40, 0x03, 0x40, 0x20, 0, 0x45, 0x0d, 1, 0x20, 0, 0x41, 1, 0x6b, 0x21, 0, 0x0c, 0,
            0x0b, 0x0b, 0x20, 0, 0x0b,
        ];
        let bytes = module(&code, &[0x7f], &[0x7f]);
        for n in [0u64, 1, 5] {
            let needed = 8 * n + 7;
            for fuel in needed.saturating_sub(3)..=needed + 2 {
                let limits = ResourceLimits { fuel: Some(fuel), ..ResourceLimits::default() };
                let outcome = run_with(&bytes, &[Value::I32(n as i32)], limits);
                if fuel >= needed {
                    assert_eq!(outcome.unwrap(), RunOutcome::Returned(vec![Value::I32(0)]), "n={n} fuel={fuel}");
                } else {
                    let error = outcome.unwrap_err();
                    assert_eq!(error.kind, ErrorKind::ResourceLimit, "n={n} fuel={fuel}");
                    assert!(error.message.contains("fuel"), "n={n} fuel={fuel}: {error}");
                }
            }
        }
    }
    #[test]
    fn t045_branch_preserves_results() {
        assert_eq!(
            run(
                &module(&[2, 0x7f, 0x41, 7, 0xc, 0, 0x41, 9, 0xb, 0xb], &[], &[0x7f]),
                &[]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(7)])
        );
    }
    #[test]
    fn t050_division_traps() {
        assert_eq!(
            run(&module(&[0x41, 1, 0x41, 0, 0x6d, 0xb], &[], &[0x7f]), &[])
                .unwrap_err()
                .kind,
            ErrorKind::Trap
        );
    }
    #[test]
    fn t007_foreign_handles_rejected() {
        let e = Engine::new(Config::default()).unwrap();
        let mut a = Store::new(&e);
        let b = Store::new(&e);
        let g = a
            .create_global(
                GlobalType {
                    value: ValType::I32,
                    mutable: false,
                },
                Value::I32(3),
            )
            .unwrap();
        assert_eq!(b.get_global(g).unwrap_err().kind, ErrorKind::Host);
    }
    #[test]
    fn t054_calls_use_explicit_frames() {
        let mut cfg = Config::default();
        cfg.limits.max_call_depth = 32;
        let e = Engine::new(cfg).unwrap();
        let m = e.compile(&module(&[0x10, 0, 0xb], &[], &[])).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let External::Func(f) = i.export(&s, "f").unwrap() else {
            panic!()
        };
        assert_eq!(s.invoke(f, &[]).unwrap_err().kind, ErrorKind::ResourceLimit);
    }
    #[test]
    fn t052_float_signed_zero_min() {
        let mut s = vec![Value::F32(0), Value::F32(0x80000000)];
        numeric::exec(0x96, &mut s).unwrap();
        assert_eq!(s, vec![Value::F32(0x80000000)]);
    }
    #[test]
    fn t043_memory_ranges_checked() {
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let m = s
            .create_memory(MemoryType {
                limits: Limits::new(1, Some(1)),
                ..Default::default()
            })
            .unwrap();
        s.write_memory(m, 65535, &[42]).unwrap();
        assert_eq!(s.read_memory(m, 65535, 1).unwrap(), [42]);
        assert!(s.read_memory(m, u64::MAX, 2).is_err());
    }
}

impl Store {
    pub fn root_value(&self, value: Value) -> Result<ValueRootToken> {
        self.validate_value(value)?;
        let mut roots = self
            .value_roots
            .lock()
            .map_err(|_| err(ErrorKind::Host, "host roots poisoned"))?;
        roots.retain(|r| r.strong_count() != 0);
        if roots.len() >= self.engine.config.limits.max_gc_mark_work {
            return Err(err(ErrorKind::ResourceLimit, "host root limit"));
        }
        roots
            .try_reserve(1)
            .map_err(|_| err(ErrorKind::ResourceLimit, "host root allocation"))?;
        let root = Arc::new(value);
        roots.push(Arc::downgrade(&root));
        Ok(ValueRootToken(root))
    }
    pub fn gc_payload_bytes(&self) -> usize {
        self.gc.bytes()
    }
    pub fn root(&self, r: GcRef) -> Result<RootToken> {
        self.gc.root(r)
    }
    pub fn collect(&mut self) -> Result<CollectionStats> {
        let mut values = Vec::new();
        let cap = self.engine.config.limits.max_gc_mark_work;
        let mut add = |value: Value| -> Result<()> {
            if !matches!(value, Value::Ref(_)) {
                return Ok(());
            }
            if values.len() >= cap {
                return Err(err(ErrorKind::ResourceLimit, "GC root provider work limit"));
            }
            values
                .try_reserve(1)
                .map_err(|_| err(ErrorKind::ResourceLimit, "GC roots allocation"))?;
            values.push(value);
            Ok(())
        };
        for value in self
            .active_roots
            .iter()
            .flatten()
            .chain(self.published.iter())
        {
            add(*value)?;
        }
        {
            let mut registry = self
                .value_roots
                .lock()
                .map_err(|_| err(ErrorKind::Host, "host roots poisoned"))?;
            registry.retain(|r| r.strong_count() != 0);
            for root in registry.iter().filter_map(|r| r.upgrade()) {
                add(*root)?;
            }
        }
        for global in &self.globals {
            add(global.value)?;
        }
        for table in &self.tabs {
            for value in &table.values {
                add(*value)?;
            }
        }
        for instance in &self.instances {
            for segment in instance.elems.iter().flatten() {
                for value in segment {
                    add(*value)?;
                }
            }
        }
        let roots = self.trace_exceptions(&values)?;
        let stats = self.gc.collect(roots)?;
        self.continuations.retain(|r, _| self.gc.get(*r).is_ok());
        let mut external = std::collections::HashSet::new();
        for value in values
            .iter()
            .chain(self.gc.live_values())
            .chain(self.exceptions.iter().flat_map(|e| &e.payload))
        {
            if let Value::Ref(RefValue::Extern(r) | RefValue::InternalExtern(r)) = value {
                external.insert(*r);
            }
        }
        let unreachable: Vec<_> = self
            .externs
            .iter()
            .filter(|(_, r)| !external.contains(r))
            .map(|(identity, r)| (*identity, *r))
            .collect();
        for (identity, r) in unreachable {
            self.externs.remove(&identity);
            self.extern_objects.remove(r.0.index, r.0.generation);
        }

        Ok(stats)
    }
}

pub struct Caller<'a> {
    store: &'a mut Store,
}
impl Caller<'_> {
    pub fn store(&mut self) -> &mut Store {
        self.store
    }
    pub fn invoke(&mut self, f: Func, args: &[Value]) -> Result<RunOutcome> {
        self.store.invoke(f, args)
    }
    pub fn collect(&mut self) -> Result<CollectionStats> {
        self.store.collect()
    }
}
impl Store {
    pub fn create_host_func_with_caller(
        &mut self,
        ty: FuncType,
        callback: impl Fn(&mut Caller<'_>, &[Value]) -> Result<Vec<Value>> + Send + Sync + 'static,
    ) -> Result<Func> {
        self.reserve_objects(1)?;
        let f = Func(self.h(self.funcs.len()));
        self.funcs.push(Function {
            host_type_instance: None,
            host_type_module: None,
            canonical: self.canonical_func(&ty)?,
            ty: Arc::new(ty),
            instance: 0,
            body: 0,
            host: Some(Arc::new(move |store, args| {
                callback(&mut Caller { store }, args).map(RunOutcome::Returned)
            })),
        });
        Ok(f)
    }
}

impl Store {
    /// Own a host resource until its external reference becomes unreachable.
    pub fn create_owned_extern_ref(
        &mut self,
        identity: u32,
        resource: impl std::any::Any + Send + Sync + 'static,
    ) -> Result<Value> {
        if self
            .externs
            .get(&identity)
            .is_some_and(|r| self.extern_objects[r.0.index].1.is_some())
        {
            return Err(err(
                ErrorKind::Host,
                "external identity already owns a resource",
            ));
        }
        let value = self.create_extern_ref(identity)?;
        let Value::Ref(RefValue::Extern(r)) = value else {
            unreachable!()
        };
        self.extern_objects[r.0.index].1 = Some(Arc::new(resource));
        Ok(value)
    }
    pub fn create_extern_ref(&mut self, identity: u32) -> Result<Value> {
        let handle = if let Some(r) = self.externs.get(&identity) {
            *r
        } else {
            self.reserve_objects(1)?;
            let (index, generation) = self.extern_objects.insert((identity, None));
            let r = ExternRef(Handle {
                store: self.id,
                index,
                generation,
            });
            self.externs.insert(identity, r);
            r
        };
        let value = Value::Ref(RefValue::Extern(handle));
        self.publish_values(&[value]);
        Ok(value)
    }
}

impl Store {
    fn canonical_func(&self, ty: &FuncType) -> Result<CanonicalTypeId> {
        if ty.params.iter().chain(&ty.results).any(|t| {
            matches!(
                t,
                ValType::Ref(RefType {
                    heap: HeapType::Defined(_) | HeapType::ExactDefined(_),
                    ..
                })
            )
        }) {
            return Err(err(
                ErrorKind::Host,
                "concrete host reference types require create_typed_host_func with an Instance",
            ));
        }
        let group = TypeGroup {
            explicit: false,
            types: vec![SubType::final_type(CompositeType::Func(ty.clone()))],
        };
        let mut registry = self
            .engine
            .canonical
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?;
        Ok(registry.intern_group(&group, &[], self.engine.config.limits.max_type_work)?[0])
    }
}

impl Store {
    pub fn create_host_func_with_outcome(
        &mut self,
        ty: FuncType,
        callback: impl Fn(&mut Caller<'_>, &[Value]) -> Result<RunOutcome> + Send + Sync + 'static,
    ) -> Result<Func> {
        self.reserve_objects(1)?;
        let f = Func(self.h(self.funcs.len()));
        self.funcs.push(Function {
            host_type_instance: None,
            host_type_module: None,
            canonical: self.canonical_func(&ty)?,
            ty: Arc::new(ty),
            instance: 0,
            body: 0,
            host: Some(Arc::new(move |store, args| {
                callback(&mut Caller { store }, args)
            })),
        });
        Ok(f)
    }
}

fn lower_control(module: &ValidatedModule) -> Result<ControlMap> {
    let mut bodies = vec![];
    for body in &module.decoded().code {
        // Positions must fit below the u32 sentinel.
        if body.instructions.len() >= ControlEntry::NONE as usize {
            return Err(err(ErrorKind::ResourceLimit, "function body too large for control map"));
        }
        let mut map = vec![ControlEntry::EMPTY; body.instructions.len()];
        let mut open: Vec<(usize, Option<usize>)> = vec![];
        for (pc, i) in body.instructions.iter().enumerate() {
            match i.opcode {
                2..=4 | 6 | 0x1f => open.push((pc, None)),
                5 | 7 | 0x19 => {
                    if let Some(top) = open.last_mut() {
                        top.1 = Some(pc)
                    }
                }
                0xb => {
                    if let Some((start, other)) = open.pop() {
                        map[start] = ControlEntry {
                            end: pc as u32,
                            other: other.map_or(ControlEntry::NONE, |other| other as u32),
                        }
                    }
                }
                _ => {}
            }
        }
        if !open.is_empty() {
            return Err(err(ErrorKind::Validation, "unclosed control block"));
        }
        bodies.push(map.into_boxed_slice())
    }
    Ok(ControlMap { bodies })
}

impl Store {
    fn validate_value(&self, value: Value) -> Result<()> {
        match value {
            Value::Ref(RefValue::Func(f)) => {
                self.check(f.0, &self.funcs)?;
            }
            Value::Ref(RefValue::Gc(r) | RefValue::ExternGc(r)) => {
                self.gc.get(r)?;
            }
            Value::Ref(RefValue::Exn(r)) => {
                self.check(r.0, &self.exceptions)?;
            }
            Value::Ref(RefValue::Extern(r) | RefValue::InternalExtern(r)) => {
                self.check(r.0, &self.extern_objects)?;
            }
            _ => {}
        }
        Ok(())
    }
}
/// A shared memory capability that can be imported into another Store.
#[derive(Clone)]
pub struct ShareableMemory {
    engine: u64,
    ty: MemoryType,
    backend: shared_memory::SharedMemory,
}
impl Store {
    pub fn share_memory(&self, memory: Memory) -> Result<ShareableMemory> {
        let i = self.check(memory.0, &self.mems)?;
        let m = &self.mems[i];
        if !m.ty.shared {
            return Err(err(ErrorKind::Host, "memory is not shared"));
        }
        let backend = m
            .shared
            .clone()
            .ok_or_else(|| err(ErrorKind::Host, "memory is not shared"))?;
        Ok(ShareableMemory {
            engine: self.engine.id,
            ty: m.ty,
            backend,
        })
    }
    pub fn import_shared_memory(&mut self, memory: &ShareableMemory) -> Result<Memory> {
        self.reserve_objects(1)?;
        if self.engine.id != memory.engine {
            return Err(err(ErrorKind::Host, "shared memory engine mismatch"));
        }
        if memory.backend.len()? as u64 > self.engine.config.limits.max_memory_bytes {
            return Err(err(ErrorKind::ResourceLimit, "memory byte limit"));
        }
        if !self.mems.iter().any(|m| {
            m.shared
                .as_ref()
                .is_some_and(|s| s.same_backing(&memory.backend))
        }) {
            self.reserve_memory(memory.backend.len()? as u64)?;
        }
        let h = Memory(self.h(self.mems.len()));
        let mut ty = memory.ty;
        ty.limits.min = (memory.backend.len()? as u64) >> ty.page_size_log2;
        self.mems.push(Mem {
            ty,
            bytes: linear::LinearBytes::default(),
            shared: Some(memory.backend.clone()),
        });
        Ok(h)
    }
}
#[cfg(test)]
mod shared_store_tests {
    use super::*;
    #[test]
    fn t134_two_host_threads_share_memory_without_foreign_handles() {
        let engine = Engine::new(Config::default()).unwrap();
        let mut first = Store::new(&engine);
        let m = first
            .create_memory(MemoryType {
                limits: Limits::new(1, Some(1)),
                shared: true,
                ..Default::default()
            })
            .unwrap();
        let token = first.share_memory(m).unwrap();
        let other_engine = engine.clone();
        std::thread::spawn(move || {
            let mut other = Store::new(&other_engine);
            let imported = other.import_shared_memory(&token).unwrap();
            other.write_memory(imported, 7, &[42]).unwrap();
            assert!(other.read_memory(m, 0, 1).is_err());
        })
        .join()
        .unwrap();
        assert_eq!(first.read_memory(m, 7, 1).unwrap(), [42]);
    }
    #[test]
    fn t005_feature_dependencies_checked_at_engine_boundary() {
        let mut cfg = Config::default();
        cfg.features.simd = false;
        cfg.features.relaxed_simd = true;
        assert_eq!(
            Engine::new(cfg).unwrap_err().kind,
            ErrorKind::DisabledFeature
        );
    }
    #[test]
    fn t052_signaling_nan_is_quieted_by_rounding() {
        for (op, initial, mask) in [
            (0x8d, Value::F32(0x7f800001), 0x00400000u64),
            (0x9d, Value::F64(0x7ff0000000000001), 0x0008000000000000),
        ] {
            let mut stack = vec![initial];
            numeric::exec(op, &mut stack).unwrap();
            let bits = match stack[0] {
                Value::F32(x) => x as u64,
                Value::F64(x) => x,
                _ => panic!(),
            };
            assert_ne!(bits & mask, 0);
        }
    }
}
#[cfg(test)]
mod host_tests {
    use super::*;
    #[test]
    fn t113_reentrant_host_collection_roots_arguments() {
        let engine = Engine::new(Config::default()).unwrap();
        let mut store = Store::new(&engine);
        let r = store
            .gc
            .allocate(
                gc::Object {
                    ty: canonical::CanonicalTypeId::test(engine.id, 0),
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        let callback = store
            .create_host_func_with_caller(
                FuncType::new(
                    vec![ValType::Ref(RefType::new(false, HeapType::Any))],
                    vec![],
                ),
                |caller, args| {
                    caller.collect()?;
                    let Value::Ref(RefValue::Gc(r)) = args[0] else {
                        panic!()
                    };
                    caller.store().gc.get(r)?;
                    Ok(vec![])
                },
            )
            .unwrap();
        assert_eq!(
            store
                .invoke(callback, &[Value::Ref(RefValue::Gc(r))])
                .unwrap(),
            RunOutcome::Returned(vec![])
        );
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalType {
    Func(FuncType),
    Memory(MemoryType),
    Table(TableType),
    Global(GlobalType),
    Tag(FuncType),
}
impl Store {
    pub fn external_type(&self, item: External) -> Result<ExternalType> {
        Ok(match item {
            External::Func(f) => {
                ExternalType::Func(self.funcs[self.check(f.0, &self.funcs)?].ty.as_ref().clone())
            }
            External::Memory(m) => {
                let m = &self.mems[self.check(m.0, &self.mems)?];
                let mut ty = m.ty;
                ty.limits.min = (m.len()? as u64) >> ty.page_size_log2;
                ExternalType::Memory(ty)
            }
            External::Table(t) => {
                let t = &self.tabs[self.check(t.0, &self.tabs)?];
                let mut ty = t.ty;
                ty.limits.min = t.values.len() as u64;
                ExternalType::Table(ty)
            }
            External::Global(g) => {
                ExternalType::Global(self.globals[self.check(g.0, &self.globals)?].ty)
            }
            External::Tag(t) => ExternalType::Tag(self.tags[self.check(t.0, &self.tags)?].clone()),
        })
    }
}
#[cfg(test)]
mod rollback_tests {
    use super::*;
    #[test]
    fn t043_start_trap_retires_private_allocations_preserving_import_writes() {
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 10, 1, 1, b'm', 3, b'm', b'e',
            b'm', 2, 0, 1, 3, 2, 1, 0, 8, 1, 0, 10, 12, 1, 10, 0, 0x41, 0, 0x41, 42, 0x3a, 0, 0, 0,
            0x0b,
        ];
        let e = Engine::new(Config::default()).unwrap();
        let module = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let memory = s
            .create_memory(MemoryType {
                limits: Limits::new(1, None),
                ..Default::default()
            })
            .unwrap();
        let mut linker = Linker::new(&e);
        linker.define("m", "mem", External::Memory(memory)).unwrap();
        for _ in 0..5 {
            assert!(linker.instantiate(&mut s, &module).is_err());
            assert_eq!(s.funcs.live_len(), 0);
            assert_eq!(s.instances.live_len(), 0);
            assert_eq!(s.mems.live_len(), 1);
            assert_eq!(s.read_memory(memory, 0, 1).unwrap(), [42]);
        }
    }
}

impl Module {
    /// Observe bounded lazy-cache state without running guest code or pinning artifacts.
    /// Returns None for eager/default modules, unavailable bookkeeping or wrong owners.
    #[cfg(all(feature = "register-executor", feature = "execution-reference"))]
    pub fn register_cache_snapshot(&self) -> Option<RegisterCacheSnapshot> {
        self.data.register_cache.as_ref()?.snapshot(self)
    }

    pub fn branch_hints(&self) -> wtime_validate::BranchHints {
        wtime_validate::inspect_branch_hints(self.data.inner.decoded())
    }

    /// The module's function imports in declaration order: import module,
    /// name, type index (for `Store::create_module_host_func`) and signature.
    /// Imports of other kinds are omitted.
    pub fn function_imports(&self) -> Vec<(&str, &str, TypeIdx, &FuncType)> {
        let decoded = self.data.inner.decoded();
        let mut types = decoded.type_groups.iter().flat_map(|group| &group.types);
        let types: Vec<&CompositeType> = types.by_ref().map(|t| &t.composite).collect();
        decoded
            .imports
            .iter()
            .filter_map(|import| {
                let (ImportKind::Func(index) | ImportKind::ExactFunc(index)) = import.kind else {
                    return None;
                };
                let CompositeType::Func(ty) = types.get(index.as_usize())? else {
                    return None;
                };
                Some((import.module.as_str(), import.name.as_str(), index, ty))
            })
            .collect()
    }
}
impl Instance {
    pub fn function(self, store: &Store, index: u32) -> Result<Func> {
        let i = store.check(self.0, &store.instances)?;
        store.instances[i]
            .funcs
            .get(index as usize)
            .copied()
            .ok_or_else(|| err(ErrorKind::Host, "function index out of bounds"))
    }
}

impl Store {
    fn publish_values(&mut self, values: &[Value]) {
        self.published.extend(
            values
                .iter()
                .copied()
                .filter(|v| matches!(v,Value::Ref(r) if !matches!(r,RefValue::Null(_)))),
        )
    }
    /// Release one Store-owned registration for each returned reference. RootToken registrations remain live.
    pub fn release_published(&mut self, values: &[Value]) {
        for value in values {
            if let Some(i) = self.published.iter().position(|v| v == value) {
                self.published.swap_remove(i);
            }
        }
    }
}
#[cfg(test)]
mod publication_tests {
    use super::*;
    #[test]
    fn t007_scalar_results_do_not_accumulate_roots() {
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let f = s
            .create_host_func(FuncType::new(vec![], vec![ValType::I32]), |_| {
                Ok(vec![Value::I32(7)])
            })
            .unwrap();
        for _ in 0..1000 {
            s.invoke(f, &[]).unwrap();
        }
        assert!(s.published.is_empty());
    }
    #[test]
    fn t113_releasing_published_result_permits_collection() {
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let r =
            s.gc.allocate(
                gc::Object {
                    ty: canonical::CanonicalTypeId::test(e.id, 0),
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        let v = Value::Ref(RefValue::Gc(r));
        let f = s
            .create_host_func(FuncType::new(vec![], vec![v.ty()]), move |_| Ok(vec![v]))
            .unwrap();
        let RunOutcome::Returned(values) = s.invoke(f, &[]).unwrap() else {
            panic!()
        };
        assert_eq!(s.collect().unwrap().live, 1);
        s.release_published(&values);
        assert_eq!(s.collect().unwrap().live, 0);
    }
}
#[cfg(test)]
mod initialization_tests {
    use super::*;
    #[test]
    fn t043_earlier_data_writes_survive_later_segment_trap() {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        let import = [1, 1, b'm', 3, b'm', b'e', b'm', 2, 0, 1];
        bytes.extend([2, import.len() as u8]);
        bytes.extend(import);
        let data = [
            2, 0, 0x41, 0, 0xb, 3, b'a', b'b', b'c', 0, 0x41, 0x80, 0x80, 4, 0xb, 1, b'z',
        ];
        bytes.extend([11, data.len() as u8]);
        bytes.extend(data);
        let e = Engine::new(Config::default()).unwrap();
        let module = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let memory = s
            .create_memory(MemoryType {
                limits: Limits::new(1, None),
                ..Default::default()
            })
            .unwrap();
        let mut linker = Linker::new(&e);
        linker.define("m", "mem", External::Memory(memory)).unwrap();
        assert!(linker.instantiate(&mut s, &module).is_err());
        assert_eq!(s.read_memory(memory, 0, 3).unwrap(), b"abc");
    }
}

fn table_range(off: u64, n: usize, len: usize) -> Result<std::ops::Range<usize>> {
    range(off, n, len).map_err(|_| Error::trap(TrapCode::TableOutOfBounds, "table out of bounds"))
}

/// Thread-safe, explicit cancellation for a Store's current execution.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::Release)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
impl Store {
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn t045_cancelled_execution_releases_frames_and_store_can_resume() {
        let e = Engine::new(Config::default()).unwrap();
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 3, 2, 1, 0, 7, 5, 1, 1, b'f', 0, 0,
            10, 9, 1, 7, 0, 3, 0x40, 0xc, 0, 0xb, 0xb,
        ];
        let m = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let External::Func(f) = i.export(&s, "f").unwrap() else {
            panic!()
        };
        let cancel = s.cancellation_token();
        let thread = std::thread::spawn(move || {
            let error = s.invoke(f, &[]).unwrap_err();
            assert_eq!(error.kind, ErrorKind::ResourceLimit);
            assert!(s.active_roots.is_empty());
            s
        });
        cancel.cancel();
        let mut s = thread.join().unwrap();
        cancel.reset();
        let host = s
            .create_host_func(FuncType::default(), |_| Ok(vec![]))
            .unwrap();
        assert_eq!(s.invoke(host, &[]).unwrap(), RunOutcome::Returned(vec![]));
    }
}

#[cfg(feature = "opcode-coverage")]
mod coverage;
#[cfg(feature = "opcode-coverage")]
pub use coverage::{clear_opcode_coverage, opcode_coverage};

impl Module {
    /// Development-only accepted/declined bodies, attempted work and retained
    /// artifact budget bytes (capacity-based, including metadata headers).
    /// None means no artifact was published; discarded work is unavailable.
    #[cfg(all(feature = "compact-executor", feature = "performance-counters"))]
    pub fn compact_preparation_counts(&self) -> Option<(usize, usize, usize, usize)> {
        self.data.compact.as_ref().map(|prepared| prepared.counts())
    }
    /// Bytes used by precomputed control destinations, excluding shared validated code.
    pub fn lowered_control_bytes(&self) -> usize {
        self.data.control.bytes()
    }
    /// Retained optional function hints, shared once across Module clones.
    pub fn function_plan_bytes(&self) -> usize {
        self.data.inner.function_bounds_bytes()
            + self.data.plans.as_ref().map_or(0, |p| p.retained_bytes)
    }
    pub fn source_bytes(&self) -> usize {
        self.data.inner.decoded().bytes.len()
    }
}

impl Store {
    fn memory_usage(&self) -> Result<u64> {
        let mut total = 0u64;
        let mut seen: Vec<&shared_memory::SharedMemory> = vec![];
        for memory in &self.mems {
            if let Some(shared) = &memory.shared {
                if seen.iter().any(|old| old.same_backing(shared)) {
                    continue;
                }
                seen.push(shared)
            }
            total = total
                .checked_add(memory.len()? as u64)
                .ok_or_else(|| err(ErrorKind::ResourceLimit, "Store memory accounting overflow"))?;
        }
        Ok(total)
    }
    fn table_usage(&self) -> u64 {
        self.tabs.iter().map(|t| t.values.len() as u64).sum()
    }
    fn reserve_objects(&self, count: usize) -> Result<()> {
        let current = self.funcs.live_len()
            + self.mems.live_len()
            + self.tabs.live_len()
            + self.globals.live_len()
            + self.instances.live_len()
            + self.tags.live_len()
            + self.exceptions.live_len()
            + self.externs.len()
            + self.gc.live();
        if current
            .checked_add(count)
            .is_none_or(|n| n > self.engine.config.limits.max_store_objects)
        {
            return Err(err(ErrorKind::ResourceLimit, "Store object budget"));
        }
        Ok(())
    }
    fn reserve_memory(&self, bytes: u64) -> Result<()> {
        if self
            .memory_usage()?
            .checked_add(bytes)
            .is_none_or(|n| n > self.engine.config.limits.max_store_memory_bytes)
        {
            return Err(err(ErrorKind::ResourceLimit, "Store memory byte budget"));
        }
        Ok(())
    }
    fn reserve_table(&self, elements: u64) -> Result<()> {
        if self
            .table_usage()
            .checked_add(elements)
            .is_none_or(|n| n > self.engine.config.limits.max_store_table_elements)
        {
            return Err(err(ErrorKind::ResourceLimit, "Store table budget"));
        }
        Ok(())
    }
}
#[cfg(test)]
mod aggregate_budget_tests {
    use super::*;
    #[test]
    fn t036_live_object_budget_releases_with_retired_slot() {
        let mut cfg = Config::default();
        cfg.limits.max_store_objects = 2;
        let e = Engine::new(cfg).unwrap();
        let mut s = Store::new(&e);
        let ty = GlobalType {
            value: ValType::I32,
            mutable: false,
        };
        let a = s.create_global(ty, Value::I32(1)).unwrap();
        s.create_global(ty, Value::I32(2)).unwrap();
        assert_eq!(
            s.create_global(ty, Value::I32(3)).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        s.globals.remove(a.0.index, a.0.generation).unwrap();
        assert!(s.get_global(a).is_err());
        assert!(s.create_global(ty, Value::I32(4)).is_ok());
    }
    #[test]
    fn t037_aggregate_memory_counts_shared_backing_once() {
        let mut cfg = Config::default();
        cfg.limits.max_store_memory_bytes = 2 * 65536;
        let e = Engine::new(cfg).unwrap();
        let mut s = Store::new(&e);
        let ty = MemoryType {
            limits: Limits::new(1, Some(1)),
            shared: true,
            ..Default::default()
        };
        let m = s.create_memory(ty).unwrap();
        let shared = s.share_memory(m).unwrap();
        s.import_shared_memory(&shared).unwrap();
        assert_eq!(s.memory_usage().unwrap(), 65536);
        s.create_memory(ty).unwrap();
        assert_eq!(
            s.create_memory(ty).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(s.memory_usage().unwrap(), 2 * 65536);
    }
    #[test]
    fn t038_aggregate_table_budget_and_nondefault_type() {
        let mut cfg = Config::default();
        cfg.limits.max_store_table_elements = 3;
        let e = Engine::new(cfg).unwrap();
        let mut s = Store::new(&e);
        let mut ty = TableType {
            limits: Limits::new(2, Some(4)),
            index_type: IndexType::I64,
            element: RefType::FUNCREF,
        };
        s.create_table(ty).unwrap();
        assert_eq!(
            s.create_table(ty).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        ty.limits.min = 1;
        assert!(s.create_table(ty).is_ok());
        ty.element.nullable = false;
        assert_eq!(s.create_table(ty).unwrap_err().kind, ErrorKind::Host);
    }
}
#[cfg(test)]
mod embedding_tests {
    use super::*;
    #[test]
    fn t035_immutable_modules_recompile_and_share_across_threads() {
        let e = Engine::new(Config::default()).unwrap();
        let a = e.compile(b"\0asm\x01\0\0\0").unwrap();
        let b = e.compile(b"\0asm\x01\0\0\0").unwrap();
        assert_eq!(a.source_bytes(), b.source_bytes());
        let threads = (0..2)
            .map(|_| {
                let e = e.clone();
                let m = a.clone();
                std::thread::spawn(move || {
                    let mut s = Store::new(&e);
                    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
                    assert!(i.exports(&s).unwrap().is_empty());
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        let other = Engine::new(Config::default()).unwrap();
        assert_eq!(
            Linker::new(&other)
                .instantiate(&mut Store::new(&other), &a)
                .unwrap_err()
                .kind,
            ErrorKind::Link
        );
    }
    #[test]
    fn t040_wrong_host_result_shape_is_host_failure() {
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let f = s
            .create_host_func(FuncType::default(), |_| Ok(vec![Value::I32(1)]))
            .unwrap();
        assert_eq!(s.invoke(f, &[]).unwrap_err().kind, ErrorKind::Host);
    }
    #[test]
    fn t040_nested_host_to_guest_preserves_ordered_results() {
        let e = Engine::new(Config::default()).unwrap();
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 6, 1, 0x60, 0, 2, 0x7f, 0x7e, 3, 2, 1, 0, 7, 5, 1, 1,
            b'f', 0, 0, 10, 8, 1, 6, 0, 0x41, 7, 0x42, 9, 0xb,
        ];
        let m = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let External::Func(guest) = i.export(&s, "f").unwrap() else {
            panic!()
        };
        let f = s
            .create_host_func_with_caller(
                FuncType::new(vec![], vec![ValType::I32, ValType::I64]),
                move |caller, _| {
                    let RunOutcome::Returned(values) = caller.invoke(guest, &[])? else {
                        return Err(err(ErrorKind::Host, "unexpected outcome"));
                    };
                    Ok(values)
                },
            )
            .unwrap();
        assert_eq!(
            s.invoke(f, &[]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
        );
    }
}

impl Store {
    /// Bind a host function to a validated type in an existing instance.
    pub fn create_typed_host_func(
        &mut self,
        instance: Instance,
        ty: TypeIdx,
        callback: impl Fn(&mut Caller<'_>, &[Value]) -> Result<RunOutcome> + Send + Sync + 'static,
    ) -> Result<Func> {
        self.reserve_objects(1)?;
        let ii = self.check(instance.0, &self.instances)?;
        let module = &self.instances[ii].module;
        let source = module
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(ty.as_usize())
            .ok_or_else(|| err(ErrorKind::Host, "host function type index"))?;
        let CompositeType::Func(signature) = &source.composite else {
            return Err(err(ErrorKind::Host, "host function requires function type"));
        };
        let function = Function {
            host_type_instance: Some(ii),
            host_type_module: None,
            canonical: module.data.canonical[ty.as_usize()],
            ty: Arc::new(signature.clone()),
            instance: ii,
            body: 0,
            host: Some(Arc::new(move |store, args| {
                callback(&mut Caller { store }, args)
            })),
        };
        let f = Func(self.h(self.funcs.len()));
        self.funcs.push(function);
        Ok(f)
    }
    /// Create an importable host function using a validated module type context.
    pub fn create_module_host_func(
        &mut self,
        module: &Module,
        ty: TypeIdx,
        callback: impl Fn(&mut Caller<'_>, &[Value]) -> Result<RunOutcome> + Send + Sync + 'static,
    ) -> Result<Func> {
        if module.data.engine != self.engine.id {
            return Err(err(ErrorKind::Host, "module belongs to another engine"));
        }
        self.reserve_objects(1)?;
        let source = module
            .data
            .inner
            .decoded()
            .type_groups
            .iter()
            .flat_map(|g| &g.types)
            .nth(ty.as_usize())
            .ok_or_else(|| err(ErrorKind::Host, "host function type index"))?;
        let CompositeType::Func(signature) = &source.composite else {
            return Err(err(ErrorKind::Host, "host function requires function type"));
        };
        let function = Function {
            host_type_instance: None,
            host_type_module: Some(module.clone()),
            canonical: module.data.canonical[ty.as_usize()],
            ty: Arc::new(signature.clone()),
            instance: 0,
            body: 0,
            host: Some(Arc::new(move |store, args| {
                callback(&mut Caller { store }, args)
            })),
        };
        let f = Func(self.h(self.funcs.len()));
        self.funcs.push(function);
        Ok(f)
    }
    fn check_function_values(
        &self,
        function: &Function,
        values: &[Value],
        types: &[ValType],
    ) -> Result<()> {
        if values.len() != types.len() {
            return Err(err(ErrorKind::Host, "function value count mismatch"));
        }
        for (value, ty) in values.iter().zip(types) {
            self.validate_value(*value)?;
            let valid = match ty {
                ValType::Ref(reference) => {
                    if let Some(module) = &function.host_type_module {
                        self.gc_matches_module(module, *value, *reference)?
                    } else if let Some(ii) = function.host_type_instance {
                        self.gc_matches(ii, *value, *reference)?
                    } else {
                        self.gc_matches(usize::MAX, *value, *reference)?
                    }
                }
                _ => matches_type(*value, *ty),
            };
            if !valid {
                return Err(err(ErrorKind::Host, "function value type mismatch"));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod typed_host_tests {
    use super::*;
    #[test]
    fn t040_concrete_host_refs_require_validated_instance_type() {
        let e = Engine::new(Config::default()).unwrap();
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 0x5f, 0, 0x60, 1, 0x63, 0, 0,
        ];
        let m = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let signature = FuncType::new(
            vec![ValType::Ref(RefType::new(
                true,
                HeapType::Defined(TypeIdx(0)),
            ))],
            vec![],
        );
        assert_eq!(
            s.create_host_func(signature, |_| Ok(vec![]))
                .unwrap_err()
                .kind,
            ErrorKind::Host
        );
        let f = s
            .create_typed_host_func(i, TypeIdx(1), |_, _| Ok(RunOutcome::Returned(vec![])))
            .unwrap();
        let r =
            s.gc.allocate(
                gc::Object {
                    ty: m.data.canonical[0],
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        assert_eq!(
            s.invoke(f, &[Value::Ref(RefValue::Gc(r))]).unwrap(),
            RunOutcome::Returned(vec![])
        );
        assert_eq!(
            s.invoke(f, &[Value::I32(0)]).unwrap_err().kind,
            ErrorKind::Host
        );
    }
}
#[cfg(test)]
mod owned_resource_tests {
    use super::*;
    #[test]
    fn t113_owned_external_drop_is_reachable_and_generational() {
        struct Resource(Arc<std::sync::atomic::AtomicUsize>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let e = Engine::new(Config::default()).unwrap();
        let mut s = Store::new(&e);
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let old = s
            .create_owned_extern_ref(9, Resource(count.clone()))
            .unwrap();
        s.collect().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let token = s.root_value(old).unwrap();
        let clone = token.clone();
        s.release_published(&[old]);
        s.collect().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        drop(token);
        s.collect().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        drop(clone);
        s.collect().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(s.validate_value(old).is_err());
        let new = s.create_extern_ref(9).unwrap();
        assert_ne!(old, new);
        assert!(s.validate_value(new).is_ok());
    }
    #[test]
    fn t162_host_memory_cannot_bypass_feature_profile() {
        let e = Engine::new(Config {
            features: FeatureSet::mvp(),
            ..Config::default()
        })
        .unwrap();
        let mut s = Store::new(&e);
        for ty in [
            MemoryType {
                shared: true,
                limits: Limits::new(0, Some(1)),
                ..Default::default()
            },
            MemoryType {
                index_type: IndexType::I64,
                ..Default::default()
            },
            MemoryType {
                page_size_log2: 0,
                ..Default::default()
            },
        ] {
            assert_eq!(
                s.create_memory(ty).unwrap_err().kind,
                ErrorKind::DisabledFeature
            );
        }
        assert_eq!(s.mems.live_len(), 0);
    }
    #[test]
    fn t040_module_bound_host_can_be_instantiated_as_typed_import() {
        let e = Engine::new(Config::default()).unwrap();
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 0x5f, 0, 0x60, 1, 0x63, 0, 0, 2, 7, 1, 1, b'h',
            1, b'f', 0, 1,
        ];
        let module = e.compile(&bytes).unwrap();
        let mut s = Store::new(&e);
        let host = s
            .create_module_host_func(&module, TypeIdx(1), |_, _| Ok(RunOutcome::Returned(vec![])))
            .unwrap();
        let mut linker = Linker::new(&e);
        linker.define("h", "f", External::Func(host)).unwrap();
        let i = linker.instantiate(&mut s, &module).unwrap();
        assert_eq!(i.function(&s, 0).unwrap(), host);
        assert_eq!(
            s.invoke(host, &[Value::Ref(RefValue::Null(HeapType::None))])
                .unwrap(),
            RunOutcome::Returned(vec![])
        );
    }
}

#[cfg(test)]
mod lowering_and_bulk_acceptance {
    use super::*;
    #[test]
    fn t044_retained_lowering_bytes_share_validated_body() {
        // (module (func (export "f") (loop (br 0))))
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 7, 5, 1, 1, b'f', 0, 0, 10,
            9, 1, 7, 0, 3, 64, 12, 0, 11, 11,
        ];
        let e = Engine::new(Config::default()).unwrap();
        let m = e.compile(&bytes).unwrap();
        let clone = m.clone();
        assert!(Arc::ptr_eq(&m.data, &clone.data));
        assert!(std::ptr::eq(
            m.data.inner.decoded().code[0].instructions.as_ptr(),
            clone.data.inner.decoded().code[0].instructions.as_ptr()
        ));
        let expected = m.data.control.bodies[0].len() * std::mem::size_of::<ControlEntry>();
        assert_eq!(m.lowered_control_bytes(), expected);
        assert!(expected > 0);
        assert_eq!(m.source_bytes(), bytes.len());
    }
    #[test]
    fn t046_cancel_during_bulk_after_committed_first_chunk() {
        // (module (memory (export "m") 2) (func (export "f") i32.const 0 i32.const 42 i32.const 131072 memory.fill))
        let bytes = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 3, 1, 0, 2, 7, 9, 2, 1,
            b'm', 2, 0, 1, b'f', 0, 0, 10, 15, 1, 13, 0, 65, 0, 65, 42, 65, 128, 128, 8, 252, 11,
            0, 11,
        ];
        let e = Engine::new(Config::default()).unwrap();
        let module = e.compile(&bytes).unwrap();
        let mut store = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut store, &module).unwrap();
        let External::Func(f) = i.export(&store, "f").unwrap() else {
            panic!()
        };
        let External::Memory(m) = i.export(&store, "m").unwrap() else {
            panic!()
        };
        store.bulk_cancel_after = Some(65536);
        let error = store.invoke(f, &[]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert!(error.message.contains("cancel"));
        assert_eq!(store.read_memory(m, 65535, 2).unwrap(), vec![42, 0]);
        assert!(store.active_roots.is_empty());
        store.cancellation_token().reset();
        store.invoke(f, &[]).unwrap();
        assert_eq!(store.read_memory(m, 131071, 1).unwrap(), vec![42]);
    }
}

#[cfg(test)]
mod performance_module_ownership {
    use super::*;
    #[test]
    fn p017_large_clone_shares_all_payload_storage() {
        let e = Engine::new(Config::default()).unwrap();
        let m = e
            .compile(include_bytes!(
                "../../../tools/wtime-bench/fixtures/prepare_large.wasm"
            ))
            .unwrap();
        let clone = m.clone();
        assert!(Arc::ptr_eq(&m.data, &clone.data));
        assert!(std::ptr::eq(
            m.data.control.bodies.as_ptr(),
            clone.data.control.bodies.as_ptr()
        ));
        assert!(std::ptr::eq(
            m.data.canonical.as_ptr(),
            clone.data.canonical.as_ptr()
        ));
        assert!(std::ptr::eq(
            m.data.inner.decoded().code.as_ptr(),
            clone.data.inner.decoded().code.as_ptr()
        ));
        assert_eq!(Arc::strong_count(&m.data), 2);
        assert_eq!(m.lowered_control_bytes(), clone.lowered_control_bytes());
    }
    #[test]
    fn p017_defined_host_type_context_survives_module_drop() {
        let e = Engine::new(Config::default()).unwrap();
        let m = e
            .compile(&[
                0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 2, 0x5f, 0, 0x60, 1, 0x63, 0, 0,
            ])
            .unwrap();
        let mut s = Store::new(&e);
        let f = s
            .create_module_host_func(&m, TypeIdx(1), |_, _| Ok(RunOutcome::Returned(vec![])))
            .unwrap();
        let r =
            s.gc.allocate(
                gc::Object {
                    ty: m.data.canonical[0],
                    kind: gc::ObjectKind::Struct,
                    fields: vec![],
                    descriptor: None,
                },
                10,
            )
            .unwrap();
        let retained = Arc::downgrade(&m.data);
        drop(m);
        assert!(retained.upgrade().is_some());
        assert_eq!(
            s.invoke(f, &[Value::Ref(RefValue::Gc(r))]).unwrap(),
            RunOutcome::Returned(vec![])
        );
        drop(s);
        assert!(retained.upgrade().is_none());
    }
}
