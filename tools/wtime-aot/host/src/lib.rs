//! `wtime-aot-host`: Multi-module Wasm Store, Cheney GC, Weak/Ephemeron, Threads,
//! Exceptions, and Fault-Based Interrupt runtime for `wtime-aot --backend x64` (`StoreCdylib`).

pub mod abi;
pub mod config;
pub mod error;
pub mod gc;
pub mod instance;
pub mod memory;
pub mod module;
pub mod signal;
pub mod store;
pub mod types;

pub use config::{
    Cache, CacheConfig, CacheStore, Collector, Config, Enabled, Engine, EngineWeak, Inlining,
    InstanceAllocationStrategy, ModuleVersionStrategy, OptLevel, PoolingAllocationConfig,
    PoolingAllocatorMetrics, RegallocAlgorithm, WasmBacktraceDetails, WasmFeatures,
};
pub use error::{Error, GcHeapOutOfMemory, PoolConcurrencyLimitError, Result, Trap};
pub use gc::{
    AnyRef, ArrayRef, ArrayRefPre, EqRef, ExnRef, ExnRefPre, ExternRef, GcEphemeron,
    GcEphemeronUpdate, GcRefType, I31, OwnedRooted, RawGcEphemerons, RootScope, Rooted, RootedRef,
    StructRef, StructRefPre, Val, ValRaw, WeakAnyRef,
};
pub use instance::{
    Caller, Extern, Func, Global, Instance, InstancePre, IntoFunc, Linker, Memory, Table, Tag,
    TypedFunc, WasmParams, WasmResults, WasmRet, WasmTy,
};
pub use memory::SharedMemory;
pub use module::Module;
pub use signal::InterruptHandle;
pub use store::{
    AsContext, AsContextMut, Store, StoreContext, StoreContextMut, StoreLimits, StoreLimitsBuilder,
};
pub use types::{
    ArrayType, ExnType, ExportType, ExternType, FieldType, FuncType, GlobalType, HeapType,
    ImportType, MemoryType, ModuleExport, Mutability, RefType, StorageType, StructType, TableType,
    TagType, ValType,
};
