//! `Config`, `Engine`, `EngineWeak`, feature reporting, and pooling/caching configuration.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::error::{Error, PoolConcurrencyLimitError, Result};
use crate::types::CanonicalTypeRegistry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Collector {
    Auto,
    DeferredReferenceCounting,
    Null,
    Copying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptLevel {
    None,
    Speed,
    SpeedAndSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegallocAlgorithm {
    Backtracking,
    SinglePass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Inlining {
    No,
    Yes,
    Intrinsics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enabled {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmBacktraceDetails {
    Disable,
    Enable,
    Environment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleVersionStrategy {
    WasmtimeVersion,
    Custom(String),
    None,
}

/// Trait matching `wasmtime::CacheStore` for optional incremental/artifact cache hooks.
pub trait CacheStore: Send + Sync + std::fmt::Debug {
    fn get(&self, key: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>>;
    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool;
}

#[derive(Clone, Debug, Default)]
pub struct CacheConfig {
    directory: Option<std::path::PathBuf>,
}

impl CacheConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_directory(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.directory = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    pub fn with_files_total_size_soft_limit(&mut self, _bytes: u64) -> &mut Self {
        self
    }

    pub fn with_files_total_size_limit_percent_if_deleting(&mut self, _pct: u8) -> &mut Self {
        self
    }

    pub fn with_file_count_limit_percent_if_deleting(&mut self, _pct: u8) -> &mut Self {
        self
    }

    pub fn with_cleanup_interval(&mut self, _interval: std::time::Duration) -> &mut Self {
        self
    }

    pub fn from_file(_path: Option<&Path>) -> Result<Self> {
        Ok(Self::default())
    }
}

#[derive(Clone, Debug)]
pub struct Cache {
    config: CacheConfig,
    hits: Arc<AtomicUsize>,
    misses: Arc<AtomicUsize>,
}

impl Cache {
    pub fn new(config: CacheConfig) -> Result<Self> {
        Ok(Self {
            config,
            hits: Arc::new(AtomicUsize::new(0)),
            misses: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn from_file(path: Option<&Path>) -> Result<Self> {
        Self::new(CacheConfig::from_file(path)?)
    }

    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    pub fn cache_hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn cache_misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }
}

/// Metrics handle returned by `Engine::pooling_allocator_metrics()`.
#[derive(Clone, Debug, Default)]
pub struct PoolingAllocatorMetrics {
    pub(crate) active_stores: usize,
}

impl PoolingAllocatorMetrics {
    pub fn core_instances(&self) -> isize {
        self.active_stores as isize
    }
}

/// Pooling allocator configuration (concurrency limit + resident reset policy).
#[derive(Clone, Debug)]
pub struct PoolingAllocationConfig {
    pub(crate) total_memories: u32,
    pub(crate) total_gc_heaps: u32,
    pub(crate) total_core_instances: u32,
    pub(crate) max_memory_size: usize,
    pub(crate) max_core_instance_size: usize,
    pub(crate) memory_keep_resident: usize,
    pub(crate) gc_heap_keep_resident: usize,
    pub(crate) total_tables: u32,
    pub(crate) table_elements: usize,
    pub(crate) max_memories_per_module: u32,
    pub(crate) max_tables_per_module: u32,
}

impl Default for PoolingAllocationConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PoolingAllocationConfig {
    pub fn new() -> Self {
        Self {
            total_memories: 1000,
            total_gc_heaps: 1000,
            total_core_instances: 1000,
            max_memory_size: 1 << 30,
            max_core_instance_size: 64 << 20,
            memory_keep_resident: 0,
            gc_heap_keep_resident: 0,
            total_tables: 1000,
            table_elements: 100_000,
            max_memories_per_module: 4,
            max_tables_per_module: 4,
        }
    }

    pub fn is_pagemap_scan_available() -> bool {
        false
    }

    pub fn total_memories(&mut self, count: u32) -> &mut Self {
        self.total_memories = count;
        self
    }

    pub fn total_gc_heaps(&mut self, count: u32) -> &mut Self {
        self.total_gc_heaps = count;
        self
    }

    pub fn total_core_instances(&mut self, count: u32) -> &mut Self {
        self.total_core_instances = count;
        self
    }

    pub fn max_memory_size(&mut self, bytes: usize) -> &mut Self {
        self.max_memory_size = bytes;
        self
    }

    pub fn max_core_instance_size(&mut self, bytes: usize) -> &mut Self {
        self.max_core_instance_size = bytes;
        self
    }

    pub fn linear_memory_keep_resident(&mut self, bytes: usize) -> &mut Self {
        self.memory_keep_resident = bytes;
        self
    }

    pub fn table_keep_resident(&mut self, _bytes: usize) -> &mut Self {
        self
    }

    pub fn total_tables(&mut self, count: u32) -> &mut Self {
        self.total_tables = count;
        self
    }

    pub fn table_elements(&mut self, count: usize) -> &mut Self {
        self.table_elements = count;
        self
    }

    pub fn max_memories_per_module(&mut self, count: u32) -> &mut Self {
        self.max_memories_per_module = count;
        self
    }

    pub fn max_tables_per_module(&mut self, count: u32) -> &mut Self {
        self.max_tables_per_module = count;
        self
    }

    pub fn max_unused_warm_slots(&mut self, _count: u32) -> &mut Self {
        self
    }

    pub fn total_stacks(&mut self, _count: u32) -> &mut Self {
        self
    }

    pub fn async_stack_keep_resident(&mut self, _bytes: usize) -> &mut Self {
        self
    }

    pub fn max_gc_heap_size(&mut self, bytes: usize) -> &mut Self {
        self.max_memory_size = bytes;
        self
    }

    pub fn gc_heap_keep_resident(&mut self, bytes: usize) -> &mut Self {
        self.gc_heap_keep_resident = bytes;
        self
    }

    pub fn max_memory_protection_keys(&mut self, _count: usize) -> &mut Self {
        self
    }

    pub fn memory_protection_keys(&mut self, _enabled: Enabled) -> &mut Self {
        self
    }

    pub fn pagemap_scan(&mut self, _enabled: Enabled) -> &mut Self {
        self
    }
}

#[derive(Clone, Debug)]
pub enum InstanceAllocationStrategy {
    OnDemand,
    Pooling(PoolingAllocationConfig),
}

/// Snapshot of enabled Wasm proposals returned by `Engine::get_wasm_features()`.
#[derive(Clone, Copy, Debug)]
pub struct WasmFeatures {
    pub reference_types: bool,
    pub function_references: bool,
    pub gc: bool,
    pub gc_types: bool,
    pub exceptions: bool,
    pub threads: bool,
    pub shared_memory: bool,
    pub multi_memory: bool,
    pub tail_call: bool,
    pub branch_hinting: bool,
    pub simd: bool,
    pub relaxed_simd: bool,
    pub memory64: bool,
}

impl WasmFeatures {
    pub fn reference_types(&self) -> bool {
        self.reference_types
    }
    pub fn function_references(&self) -> bool {
        self.function_references
    }
    pub fn gc(&self) -> bool {
        self.gc
    }
    pub fn gc_types(&self) -> bool {
        self.gc_types
    }
    pub fn exceptions(&self) -> bool {
        self.exceptions
    }
    pub fn threads(&self) -> bool {
        self.threads
    }
    pub fn multi_memory(&self) -> bool {
        self.multi_memory
    }
    pub fn tail_call(&self) -> bool {
        self.tail_call
    }
    pub fn branch_hinting(&self) -> bool {
        self.branch_hinting
    }
    pub fn simd(&self) -> bool {
        self.simd
    }
}

/// Configuration builder for `Engine`.
#[derive(Clone, Debug)]
pub struct Config {
    pub(crate) opt_level: OptLevel,
    pub(crate) compiler_enabled: bool,
    pub(crate) regalloc_algorithm: RegallocAlgorithm,
    pub(crate) inlining: Inlining,
    pub(crate) collector: Option<Collector>,
    pub(crate) gc_support: bool,
    pub(crate) wasm_reference_types: bool,
    pub(crate) wasm_function_references: bool,
    pub(crate) wasm_gc: bool,
    pub(crate) wasm_exceptions: bool,
    pub(crate) wasm_threads: bool,
    pub(crate) shared_memory: bool,
    pub(crate) wasm_multi_memory: bool,
    pub(crate) wasm_tail_call: bool,
    pub(crate) wasm_branch_hinting: bool,
    pub(crate) wasm_simd: bool,
    pub(crate) wasm_relaxed_simd: bool,
    pub(crate) wasm_memory64: bool,
    pub(crate) gc_heap_initial_size: u64,
    pub(crate) gc_heap_reservation: u64,
    pub(crate) gc_heap_guard_size: u64,
    pub(crate) max_wasm_stack: usize,
    pub(crate) async_stack_size: usize,
    pub(crate) memory_reservation: u64,
    pub(crate) memory_guard_size: u64,
    pub(crate) allocation_strategy: InstanceAllocationStrategy,
    pub(crate) module_version: ModuleVersionStrategy,
    pub(crate) parallel_compilation: bool,
    pub(crate) backtrace_details: WasmBacktraceDetails,
    pub(crate) cache: Option<Cache>,
    pub(crate) consume_fuel: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            opt_level: OptLevel::Speed,
            compiler_enabled: true,
            regalloc_algorithm: RegallocAlgorithm::Backtracking,
            inlining: Inlining::No,
            collector: Some(Collector::Copying),
            gc_support: true,
            wasm_reference_types: true,
            wasm_function_references: true,
            wasm_gc: true,
            wasm_exceptions: true,
            wasm_threads: true,
            shared_memory: true,
            wasm_multi_memory: true,
            wasm_tail_call: true,
            wasm_branch_hinting: true,
            wasm_simd: true,
            wasm_relaxed_simd: true,
            wasm_memory64: true,
            gc_heap_initial_size: 4 << 20,
            gc_heap_reservation: crate::gc::SEMI_SPACE_RESERVE_BYTES as u64,
            gc_heap_guard_size: 32 << 20,
            max_wasm_stack: 32 << 20,
            async_stack_size: 64 << 20,
            memory_reservation: 1 << 32,
            memory_guard_size: 32 << 20,
            allocation_strategy: InstanceAllocationStrategy::OnDemand,
            module_version: ModuleVersionStrategy::WasmtimeVersion,
            parallel_compilation: true,
            backtrace_details: WasmBacktraceDetails::Disable,
            cache: None,
            consume_fuel: false,
        }
    }

    pub fn cranelift_opt_level(&mut self, level: OptLevel) -> &mut Self {
        self.opt_level = level;
        self
    }

    pub fn enable_compiler(&mut self, enable: bool) -> &mut Self {
        self.compiler_enabled = enable;
        self
    }

    pub fn cranelift_regalloc_algorithm(&mut self, alg: RegallocAlgorithm) -> &mut Self {
        self.regalloc_algorithm = alg;
        self
    }

    /// # Safety
    /// Matches Wasmtime's unsafe flag-setting API.
    pub unsafe fn cranelift_flag_set(&mut self, _name: &str, _value: &str) -> &mut Self {
        self
    }

    /// # Safety
    /// Matches Wasmtime's unsafe flag-enabling API.
    pub unsafe fn cranelift_flag_enable(&mut self, _name: &str) -> &mut Self {
        self
    }

    pub fn compiler_inlining(&mut self, inlining: Inlining) -> &mut Self {
        self.inlining = inlining;
        self
    }

    pub fn collector(&mut self, collector: Collector) -> &mut Self {
        self.collector = Some(collector);
        self
    }

    pub fn gc_support(&mut self, enable: bool) -> &mut Self {
        self.gc_support = enable;
        self
    }

    pub fn wasm_reference_types(&mut self, enable: bool) -> &mut Self {
        self.wasm_reference_types = enable;
        self
    }

    pub fn wasm_function_references(&mut self, enable: bool) -> &mut Self {
        self.wasm_function_references = enable;
        self
    }

    pub fn wasm_gc(&mut self, enable: bool) -> &mut Self {
        self.wasm_gc = enable;
        self
    }

    pub fn wasm_exceptions(&mut self, enable: bool) -> &mut Self {
        self.wasm_exceptions = enable;
        self
    }

    pub fn wasm_threads(&mut self, enable: bool) -> &mut Self {
        self.wasm_threads = enable;
        self
    }

    pub fn shared_memory(&mut self, enable: bool) -> &mut Self {
        self.shared_memory = enable;
        self
    }

    pub fn wasm_multi_memory(&mut self, enable: bool) -> &mut Self {
        self.wasm_multi_memory = enable;
        self
    }

    pub fn wasm_tail_call(&mut self, enable: bool) -> &mut Self {
        self.wasm_tail_call = enable;
        self
    }

    pub fn wasm_branch_hinting(&mut self, enable: bool) -> &mut Self {
        self.wasm_branch_hinting = enable;
        self
    }

    pub fn wasm_simd(&mut self, enable: bool) -> &mut Self {
        self.wasm_simd = enable;
        self
    }

    pub fn wasm_relaxed_simd(&mut self, enable: bool) -> &mut Self {
        self.wasm_relaxed_simd = enable;
        self
    }

    pub fn wasm_memory64(&mut self, enable: bool) -> &mut Self {
        self.wasm_memory64 = enable;
        self
    }

    pub fn gc_heap_initial_size(&mut self, bytes: u64) -> &mut Self {
        self.gc_heap_initial_size = bytes.max(64 * 1024);
        self
    }

    pub fn gc_heap_reservation(&mut self, bytes: u64) -> &mut Self {
        self.gc_heap_reservation = bytes;
        self
    }

    pub fn gc_heap_guard_size(&mut self, bytes: u64) -> &mut Self {
        self.gc_heap_guard_size = bytes;
        self
    }

    pub fn gc_heap_reservation_for_growth(&mut self, _bytes: u64) -> &mut Self {
        self
    }

    pub fn gc_heap_may_move(&mut self, _enable: bool) -> &mut Self {
        self
    }

    pub fn max_wasm_stack(&mut self, bytes: usize) -> &mut Self {
        self.max_wasm_stack = bytes;
        self
    }

    pub fn async_stack_size(&mut self, bytes: usize) -> &mut Self {
        self.async_stack_size = bytes;
        self
    }

    pub fn memory_reservation(&mut self, bytes: u64) -> &mut Self {
        self.memory_reservation = bytes;
        self
    }

    pub fn memory_reservation_for_growth(&mut self, _bytes: u64) -> &mut Self {
        self
    }

    pub fn memory_may_move(&mut self, _enable: bool) -> &mut Self {
        self
    }

    pub fn memory_guard_size(&mut self, bytes: u64) -> &mut Self {
        self.memory_guard_size = bytes;
        self
    }

    pub fn guard_before_linear_memory(&mut self, _enable: bool) -> &mut Self {
        self
    }

    pub fn epoch_interruption(&mut self, _enable: bool) -> &mut Self {
        self
    }

    pub fn wasm_backtrace_details(&mut self, details: WasmBacktraceDetails) -> &mut Self {
        self.backtrace_details = details;
        self
    }

    pub fn parallel_compilation(&mut self, enable: bool) -> &mut Self {
        self.parallel_compilation = enable;
        self
    }

    pub fn allocation_strategy(&mut self, strategy: InstanceAllocationStrategy) -> &mut Self {
        self.allocation_strategy = strategy;
        self
    }

    pub fn module_version(&mut self, strategy: ModuleVersionStrategy) -> Result<&mut Self> {
        if let ModuleVersionStrategy::Custom(ref s) = strategy
            && s.len() > 255
        {
            return Err(Error::msg("custom module version exceeds 255 bytes"));
        }
        self.module_version = strategy;
        Ok(self)
    }

    pub fn cache(&mut self, cache: Option<Cache>) -> &mut Self {
        self.cache = cache;
        self
    }

    pub fn enable_incremental_compilation(
        &mut self,
        _cache: Arc<dyn CacheStore>,
    ) -> Result<&mut Self> {
        Ok(self)
    }

    pub fn debug_info(&mut self, _enable: bool) -> &mut Self {
        self
    }

    pub fn profiler(&mut self, _profiler: ()) -> &mut Self {
        self
    }

    pub fn consume_fuel(&mut self, enable: bool) -> &mut Self {
        self.consume_fuel = enable;
        self
    }

    pub fn target(&mut self, _triple: &str) -> Result<&mut Self> {
        Ok(self)
    }
}

/// Global code range entry for return-address safepoint lookup and signal classification.
#[derive(Clone, Copy, Debug)]
pub struct RegisteredCodeRange {
    pub code_start: usize,
    pub code_end: usize,
    pub safepoint_header: *const crate::abi::SafepointHeader,
}

// SAFETY: `safepoint_header` points into immutable `.rodata` of a loaded ELF `.so` that stays
// mapped while registered.
unsafe impl Send for RegisteredCodeRange {}
unsafe impl Sync for RegisteredCodeRange {}

pub(crate) struct EngineInner {
    pub(crate) id: u64,
    pub(crate) config: Config,
    pub(crate) types: Mutex<CanonicalTypeRegistry>,
    /// Sorted by `code_start` for O(log M) binary search during GC stack walking.
    pub(crate) code_ranges: Mutex<Vec<RegisteredCodeRange>>,
    /// Active concurrent Stores when using `InstanceAllocationStrategy::Pooling`.
    pub(crate) active_stores: AtomicUsize,
    /// Optional pool concurrency cap (`None` for OnDemand).
    pub(crate) max_pooled_stores: Option<usize>,
    /// Cached compiled `.so` modules keyed by blake/fnv/sha hash when needed.
    pub(crate) _extra: Mutex<HashMap<u64, ()>>,
}

impl std::fmt::Debug for EngineInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("id", &self.id).finish()
    }
}

static NEXT_ENGINE_ID: AtomicU64 = AtomicU64::new(1);

/// Compilation and type-canonicalization context shared by `Module`s, `Linker`s, and `Store`s.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<EngineInner>,
}

/// Weak handle to an `Engine` (does not keep compiled resources alive).
#[derive(Clone)]
pub struct EngineWeak {
    inner: Weak<EngineInner>,
}

impl EngineWeak {
    pub fn upgrade(&self) -> Option<Engine> {
        self.inner.upgrade().map(|inner| Engine { inner })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(&Config::default()).expect("default wtime-aot-host Engine")
    }
}

impl Engine {
    pub fn new(config: &Config) -> Result<Self> {
        let max_pooled_stores = match &config.allocation_strategy {
            InstanceAllocationStrategy::OnDemand => None,
            InstanceAllocationStrategy::Pooling(p) => Some(p.total_gc_heaps.max(1) as usize),
        };
        let id = NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            inner: Arc::new(EngineInner {
                id,
                config: config.clone(),
                types: Mutex::new(CanonicalTypeRegistry::new()),
                code_ranges: Mutex::new(Vec::new()),
                active_stores: AtomicUsize::new(0),
                max_pooled_stores,
                _extra: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn same(a: &Engine, b: &Engine) -> bool {
        a.inner.id == b.inner.id
    }

    pub fn weak(&self) -> EngineWeak {
        EngineWeak {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn get_collector(&self) -> Option<Collector> {
        self.inner.config.collector
    }

    pub fn get_shared_memory(&self) -> bool {
        self.inner.config.shared_memory
    }

    pub fn get_cranelift_opt_level(&self) -> Option<OptLevel> {
        self.inner
            .config
            .compiler_enabled
            .then_some(self.inner.config.opt_level)
    }

    pub fn get_cranelift_regalloc_algorithm(&self) -> Option<RegallocAlgorithm> {
        self.inner
            .config
            .compiler_enabled
            .then_some(self.inner.config.regalloc_algorithm)
    }

    pub fn get_compiler_inlining(&self) -> Inlining {
        self.inner.config.inlining
    }

    pub fn get_gc_heap_initial_size(&self) -> u64 {
        self.inner.config.gc_heap_initial_size
    }

    pub fn get_parallel_compilation(&self) -> bool {
        self.inner.config.parallel_compilation
    }

    pub fn get_max_wasm_stack(&self) -> usize {
        self.inner.config.max_wasm_stack
    }

    pub fn get_async_stack_size(&self) -> usize {
        self.inner.config.async_stack_size
    }

    pub fn get_wasm_features(&self) -> WasmFeatures {
        let c = &self.inner.config;
        WasmFeatures {
            reference_types: c.wasm_reference_types,
            function_references: c.wasm_function_references,
            gc: c.wasm_gc,
            gc_types: c.gc_support,
            exceptions: c.wasm_exceptions,
            threads: c.wasm_threads,
            shared_memory: c.shared_memory,
            multi_memory: c.wasm_multi_memory,
            tail_call: c.wasm_tail_call,
            branch_hinting: c.wasm_branch_hinting,
            simd: c.wasm_simd,
            relaxed_simd: c.wasm_relaxed_simd,
            memory64: c.wasm_memory64,
        }
    }

    /// Deterministic hash covering target ABI, optimization level, features, and module version.
    pub fn precompile_compatibility_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        // Stable FNV-1a 64-bit hash so identical configs across processes produce the exact same key.
        struct Fnv(u64);
        impl Hasher for Fnv {
            fn write(&mut self, bytes: &[u8]) {
                for &b in bytes {
                    self.0 ^= u64::from(b);
                    self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            fn finish(&self) -> u64 {
                self.0
            }
        }
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        std::env::consts::ARCH.as_bytes().hash(&mut h);
        wtime_aot::x64::store_rt::STORE_ABI_VERSION.hash(&mut h);
        let c = &self.inner.config;
        (c.opt_level as u8).hash(&mut h);
        c.collector.map(|v| v as u8).hash(&mut h);
        c.gc_support.hash(&mut h);
        c.wasm_reference_types.hash(&mut h);
        c.wasm_function_references.hash(&mut h);
        c.wasm_gc.hash(&mut h);
        c.wasm_exceptions.hash(&mut h);
        c.wasm_threads.hash(&mut h);
        c.shared_memory.hash(&mut h);
        c.wasm_multi_memory.hash(&mut h);
        c.wasm_tail_call.hash(&mut h);
        c.wasm_branch_hinting.hash(&mut h);
        c.wasm_simd.hash(&mut h);
        c.wasm_relaxed_simd.hash(&mut h);
        c.wasm_memory64.hash(&mut h);
        c.consume_fuel.hash(&mut h);
        match &c.module_version {
            ModuleVersionStrategy::WasmtimeVersion => {
                0u8.hash(&mut h);
            }
            ModuleVersionStrategy::Custom(s) => {
                1u8.hash(&mut h);
                s.as_bytes().hash(&mut h);
            }
            ModuleVersionStrategy::None => {
                2u8.hash(&mut h);
            }
        }
        h.finish()
    }

    /// Precompiles Wasm bytes into a serialized `wtime-aot-host` native image without loading it.
    pub fn precompile_module(&self, wasm: &[u8]) -> Result<Vec<u8>> {
        let so_bytes = crate::module::compile_wasm_so(self, wasm)?;
        Ok(crate::module::serialize_so_bytes(self, &so_bytes))
    }

    pub fn pooling_allocator_metrics(&self) -> Option<PoolingAllocatorMetrics> {
        self.inner
            .max_pooled_stores
            .map(|_| PoolingAllocatorMetrics {
                active_stores: self.inner.active_stores.load(Ordering::Relaxed),
            })
    }

    /// Acquires a pooled Store slot if pooling is configured.
    pub(crate) fn acquire_store_slot(&self) -> Result<Option<PoolStoreGuard>> {
        let Some(limit) = self.inner.max_pooled_stores else {
            return Ok(None);
        };
        let mut cur = self.inner.active_stores.load(Ordering::Acquire);
        loop {
            if cur >= limit {
                return Err(PoolConcurrencyLimitError::new(limit, "stores").into());
            }
            match self.inner.active_stores.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Some(PoolStoreGuard {
                        engine: Arc::clone(&self.inner),
                    }));
                }
                Err(actual) => cur = actual,
            }
        }
    }

    pub(crate) fn register_code_range(&self, range: RegisteredCodeRange) {
        let mut ranges = self
            .inner
            .code_ranges
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pos = ranges.partition_point(|r| r.code_start < range.code_start);
        ranges.insert(pos, range);
        crate::signal::register_global_code_range(range.code_start, range.code_end);
    }

    pub(crate) fn unregister_code_range(&self, code_start: usize, code_end: usize) {
        let mut ranges = self
            .inner
            .code_ranges
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = ranges
            .iter()
            .position(|r| r.code_start == code_start && r.code_end == code_end)
        {
            ranges.remove(pos);
        }
        crate::signal::unregister_global_code_range(code_start, code_end);
    }
}

pub(crate) struct PoolStoreGuard {
    engine: Arc<EngineInner>,
}

impl Drop for PoolStoreGuard {
    fn drop(&mut self) {
        self.engine.active_stores.fetch_sub(1, Ordering::AcqRel);
    }
}
