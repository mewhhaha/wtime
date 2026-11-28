//! Lean, self-contained Wasm admission & metadata analysis for `--backend x64`.
//!
//! Decoupled from `codegen::Module` so `--backend x64` skips C/Rust-only analyses
//! (global-preservation fixpoints, callee read/write sets, outline planning) and
//! enforces an explicit Stage 1 admission contract before emitting any IR or code.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use wtime_types::{
    CompositeType, FeatureSet, FieldType, FuncType, HeapType, IndexType, ResourceLimits,
    StorageType, SubType, TypeIdx, ValType,
};
use wtime_validate::{
    DecodedModule, ExternalKind, Immediate, ImportKind, Instruction, SegmentMode,
};

pub type Res<T> = Result<T, String>;

/// Output artifact target for `--backend x64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X64Target {
    /// Blot ABI 8 shared library (`ET_DYN`, `library.so`).
    BlotCdylib,
    /// Self-contained static Linux x86-64 executable (`ET_EXEC`).
    StandaloneExec,
    /// Multi-module Store-hosted shared library (`ET_DYN`) with arbitrary imports/exports,
    /// shared Cheney GC heap, and cross-module canonical type remapping.
    StoreCdylib,
}

/// Exact Wasm feature profile enforced right at the front door of `wtime_validate::validate`,
/// disabling unsupported proposals (threads, custom page sizes/descriptors, stack switching)
/// before decoding/validation.
pub const fn x64_validate_features() -> FeatureSet {
    FeatureSet {
        mutable_globals: true,
        saturating_float_to_int: true,
        sign_extension: true,
        multi_value: true,
        reference_types: true,
        bulk_memory: true,
        simd: true,
        relaxed_simd: true,
        tail_call: true,
        memory64: true,
        multi_memory: true,
        extended_const: true,
        wide_arithmetic: true,
        exception_handling: true,
        function_references: true,
        gc: true,
        threads: true,
        branch_hinting: true,
        ..FeatureSet::mvp()
    }
}

/// Resource limits passed to `wtime_validate::validate` (matching `ResourceLimits::default()`
/// except allowing `memory64` declared page limits up to `2^48` and large compiled guest modules).
pub fn x64_validate_limits() -> ResourceLimits {
    ResourceLimits {
        max_module_bytes: 256 * 1024 * 1024,
        max_instructions: 100_000_000,
        max_total_locals: 4_000_000,
        max_memory_pages: 1 << 48,
        max_memory_bytes: u64::MAX,
        ..ResourceLimits::default()
    }
}

/// Admission & compilation caps preventing OOM, peak-memory spikes, or integer overflow on adversarial modules.
pub const MAX_MEM_PAGES: u64 = 65_536;
pub const MAX_MEM64_PAGES: u64 = 1 << 48;
pub const MAX_EXTRA_MEMORIES: usize = 64;
pub const MAX_TAGS: usize = 4_096;
pub const MAX_FUNCREF_TABLE_SLOTS: u32 = 1_048_576;
pub const MAX_TOTAL_FUNCREF_SLOTS: usize = 2_097_152;
pub const MAX_EXTERN_TABLE_SLOTS: u32 = 65_536;
pub const MAX_TOTAL_EXTERN_SLOTS: usize = 65_536;
pub const MAX_DYN_TABLE_SLOTS: u32 = 131_072;
pub const MAX_ELEM_SEG_ENTRIES: usize = 2_097_152;
pub const MAX_GLOBALS: usize = 65_536;
pub const MAX_EXPORTS: usize = 65_536;
pub const MAX_MODULE_TYPES: usize = 100_000;
pub const MAX_CTX_BYTES: usize = 16 << 20; // 16 MiB (far below i32::MAX)
pub const MAX_AGGREGATE_CI_TABLE_BYTES: usize = 64 << 20; // 64 MiB
pub const MAX_AGGREGATE_BR_TABLE_ENTRIES: usize = 4_194_304; // 16 MiB jump tables
pub const MAX_LOCALS_PER_FN: usize = 65_536;
pub const MAX_TOTAL_LOCALS: usize = 1_000_000;
pub const MAX_TOTAL_SIG_ARITY: usize = 1_000_000; // params + results across decls, blocks, and call_indirect sites
pub const STORE_MAX_TOTAL_LOCALS: usize = 8_000_000;
pub const STORE_MAX_TOTAL_SIG_ARITY: usize = 16_000_000;
pub const MAX_BLOCKS_PER_FN: usize = 262_144;
pub const MAX_FRAME_BYTES: i32 = 4 << 20; // 4 MiB (well below 8 MiB dedicated guest stack)
pub const MAX_LIVENESS_BYTES: usize = 16 << 20; // 16 MiB per function
pub const MAX_CONCURRENT_LIVENESS_BYTES: usize = 256 << 20; // 256 MiB peak across all parallel workers
pub const MAX_LIVENESS_WORK_UNITS: usize = 32_000_000;
pub const MAX_FRAME_SCAN_WRITES: usize = 4_000_000;
pub const MAX_IF_SNAPSHOT_ENTRIES: usize = 4_000_000;
pub const MAX_IR_VALS_PER_FN: usize = 1_000_000;
pub const MAX_VAL_POOL_PER_FN: usize = 2_000_000;
pub const MAX_FN_CODE_BYTES: usize = 64 << 20; // 64 MiB
pub const MAX_FN_LOCAL_RODATA_BYTES: usize = 16 << 20; // 16 MiB
pub const MAX_TOTAL_CODE_BYTES: usize = 512 << 20; // 512 MiB aggregate across workers
pub const MAX_TOTAL_RODATA_BYTES: usize = 128 << 20; // 128 MiB aggregate across workers

/// Shared cross-worker compilation budget enforcing cooperative `--time-limit` checks,
/// peak concurrent liveness scheduling (deterministic serial/parallel acceptance), and
/// aggregate emitted code/rodata limits BEFORE growth.
pub struct CompileBudget {
    deadline: Option<Instant>,
    liveness_mu: Mutex<usize>,
    liveness_cv: Condvar,
    total_code_bytes: AtomicUsize,
    total_rodata_bytes: AtomicUsize,
}

/// RAII release guard for an in-flight liveness matrix reservation.
pub struct LivenessBudgetGuard<'a> {
    budget: &'a CompileBudget,
    bytes: usize,
}

impl Drop for LivenessBudgetGuard<'_> {
    fn drop(&mut self) {
        let mut cur = self
            .budget
            .liveness_mu
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cur = cur.saturating_sub(self.bytes);
        self.budget.liveness_cv.notify_all();
    }
}

impl CompileBudget {
    pub fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            liveness_mu: Mutex::new(0),
            liveness_cv: Condvar::new(),
            total_code_bytes: AtomicUsize::new(0),
            total_rodata_bytes: AtomicUsize::new(0),
        }
    }

    /// Forks a fresh budget inheriting the current shared code and rodata charges so
    /// speculative incremental compilation enforces the exact combined caps while discarding
    /// warm-only charges cleanly if it falls back to cold compilation.
    pub fn fork_snapshot(&self) -> Self {
        Self {
            deadline: self.deadline,
            liveness_mu: Mutex::new(0),
            liveness_cv: Condvar::new(),
            total_code_bytes: AtomicUsize::new(self.total_code_bytes.load(Ordering::Acquire)),
            total_rodata_bytes: AtomicUsize::new(self.total_rodata_bytes.load(Ordering::Acquire)),
        }
    }

    #[inline]
    pub fn check_deadline(&self) -> Res<()> {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err("x64: compilation time limit exceeded".into());
        }
        Ok(())
    }

    /// Schedules parallel workers against `MAX_CONCURRENT_LIVENESS_BYTES` BEFORE allocating
    /// liveness bitset matrices, waiting cleanly rather than spuriously rejecting a valid module
    /// due to worker overlap (deterministic rejection stays solely on per-function `MAX_LIVENESS_BYTES`).
    pub fn reserve_liveness(&self, bytes: usize) -> Res<LivenessBudgetGuard<'_>> {
        if bytes > MAX_LIVENESS_BYTES || bytes > MAX_CONCURRENT_LIVENESS_BYTES {
            return Err(format!(
                "x64: liveness matrix ({bytes} B) exceeds cap {MAX_LIVENESS_BYTES} B"
            ));
        }
        let mut cur = self.liveness_mu.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            self.check_deadline()?;
            if cur
                .checked_add(bytes)
                .is_some_and(|next| next <= MAX_CONCURRENT_LIVENESS_BYTES)
            {
                *cur += bytes;
                return Ok(LivenessBudgetGuard {
                    budget: self,
                    bytes,
                });
            }
            cur = match self.deadline {
                Some(dl) => {
                    let rem = dl.saturating_duration_since(Instant::now());
                    if rem.is_zero() {
                        return Err("x64: compilation time limit exceeded".into());
                    }
                    let (guard, _) = self
                        .liveness_cv
                        .wait_timeout(cur, rem)
                        .unwrap_or_else(|e| e.into_inner());
                    guard
                }
                None => self
                    .liveness_cv
                    .wait(cur)
                    .unwrap_or_else(|e| e.into_inner()),
            };
        }
    }

    /// Atomically charges `delta` emitted machine-code bytes against `MAX_TOTAL_CODE_BYTES`.
    pub fn charge_code(&self, delta: usize) -> Res<()> {
        if delta == 0 {
            return Ok(());
        }
        let mut cur = self.total_code_bytes.load(Ordering::Acquire);
        loop {
            let next = cur
                .checked_add(delta)
                .ok_or("x64: aggregate code size overflow")?;
            if next > MAX_TOTAL_CODE_BYTES {
                return Err(format!(
                    "x64: aggregate code size ({next} B) exceeds cap {MAX_TOTAL_CODE_BYTES} B"
                ));
            }
            match self.total_code_bytes.compare_exchange_weak(
                cur,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => cur = actual,
            }
        }
    }

    /// Atomically charges `delta` emitted local-rodata bytes against `MAX_TOTAL_RODATA_BYTES`.
    pub fn charge_rodata(&self, delta: usize) -> Res<()> {
        if delta == 0 {
            return Ok(());
        }
        let mut cur = self.total_rodata_bytes.load(Ordering::Acquire);
        loop {
            let next = cur
                .checked_add(delta)
                .ok_or("x64: aggregate rodata size overflow")?;
            if next > MAX_TOTAL_RODATA_BYTES {
                return Err(format!(
                    "x64: aggregate rodata size ({next} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
                ));
            }
            match self.total_rodata_bytes.compare_exchange_weak(
                cur,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => cur = actual,
            }
        }
    }
}

/// Inline Cohen supertype-display length embedded directly in each 64-byte `.rodata` `TypeDesc`.
pub const GC_DISPLAY_LEN: usize = 10;
/// Object header tag bits (`bits 1..2`, with `bit 0 == 0` for live objects, `bit 0 == 1` for Cheney forwarding ptr).
pub const GC_HDR_KIND_STRUCT: u32 = 0;
pub const GC_HDR_KIND_ARRAY: u32 = 2;
pub const GC_HDR_KIND_INTERNAL_EXTERN: u32 = 4;
/// Reserved high 32-bit sentinel tag distinguishing `extern.convert_any(i31)` from opaque host `externref`s.
pub const GC_EXT_I31_TAG_HI: u64 = 0x5754_4933_0000_0000;
/// Admission caps for Wasm GC types and constant initializers.
pub const MAX_GC_STRUCT_FIELDS: usize = 4_096;
pub const MAX_GC_CONST_NODES: usize = 1_000_000;
pub const MAX_GC_SUBTYPE_DEPTH: usize = 1_024;
/// Default demand-paged initial semi-space size (4 MiB) and per-semi-space growth cap (256 MiB).
pub const DEFAULT_GC_INIT_HEAP_BYTES: u64 = 4 << 20;
pub const DEFAULT_GC_MAX_HEAP_BYTES: u64 = 256 << 20;

/// Disjoint Wasm reference hierarchy family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GcRefFamily {
    /// `any`, `eq`, `i31`, `struct`, `array`, `none`, concrete struct/array types (unconditionally GC-traced if non-null & non-i31).
    Any,
    /// `extern`, `noextern` (GC-traced only if pointing inside active GC from-space).
    Extern,
    /// `func`, `nofunc`, concrete function types (`0` = null, `f + 1` = `ref.func f`).
    Func,
    /// `exn`, `noexn` (`0` = null, offset into exception slab).
    Exn,
}

impl GcRefFamily {
    #[inline]
    pub const fn is_traced(self) -> bool {
        matches!(self, GcRefFamily::Any | GcRefFamily::Extern)
    }
}

/// Physical storage kind of a GC struct field or array element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GcStorageKind {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
    F32 = 4,
    F64 = 5,
    V128 = 6,
    GcRef = 7,
    ExternRef = 8,
    OtherRef = 9,
}

impl GcStorageKind {
    #[inline]
    pub const fn byte_size(self) -> u32 {
        match self {
            GcStorageKind::I8 => 1,
            GcStorageKind::I16 => 2,
            GcStorageKind::I32 | GcStorageKind::F32 => 4,
            GcStorageKind::I64
            | GcStorageKind::F64
            | GcStorageKind::GcRef
            | GcStorageKind::ExternRef
            | GcStorageKind::OtherRef => 8,
            GcStorageKind::V128 => 16,
        }
    }

    #[inline]
    pub const fn align(self) -> u32 {
        match self {
            GcStorageKind::I8 => 1,
            GcStorageKind::I16 => 2,
            GcStorageKind::I32 | GcStorageKind::F32 => 4,
            GcStorageKind::I64
            | GcStorageKind::F64
            | GcStorageKind::V128
            | GcStorageKind::GcRef
            | GcStorageKind::ExternRef
            | GcStorageKind::OtherRef => 8,
        }
    }
}

/// Laid-out field inside a Wasm GC struct (byte offset from object base `+0`, after the 8-byte header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcStructField {
    pub offset: u32,
    pub storage: GcStorageKind,
    pub mutable: bool,
    pub non_null: bool,
}

/// Canonical composite payload of a `GcCanonType`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GcCompositeInfo {
    Func,
    Struct {
        fields: Vec<GcStructField>,
        /// 8-byte aligned total object byte size including the 8-byte header (`>= 8`).
        total_bytes: u32,
        /// Byte offsets of fields holding `GcRef` (`any` hierarchy).
        gc_ref_offs: Vec<u32>,
        /// Byte offsets of fields holding `ExternRef` (`extern` hierarchy).
        ext_ref_offs: Vec<u32>,
    },
    Array {
        elem: GcStorageKind,
        mutable: bool,
        elem_non_null: bool,
    },
}

/// Canonicalized Wasm GC type metadata indexed by `canon_id`.
#[derive(Clone, Debug)]
pub struct GcCanonType {
    pub canon_id: u32,
    pub is_final: bool,
    pub supertype: Option<u32>,
    pub depth: u32,
    /// Inline supertype chain `[root, ..., self]`, padded with `u32::MAX` beyond `depth`.
    pub display: [u32; GC_DISPLAY_LEN],
    /// Full supertype chain `[root, ..., self]` (length `depth + 1`) for `depth >= GC_DISPLAY_LEN`.
    pub supertypes_all: Vec<u32>,
    pub composite: GcCompositeInfo,
}

impl GcCanonType {
    /// Header word (`u32`) stored at `[obj + 0]` for live instances of this type.
    #[inline]
    pub const fn header_word(&self) -> u32 {
        let kind = match self.composite {
            GcCompositeInfo::Struct { .. } => GC_HDR_KIND_STRUCT,
            GcCompositeInfo::Array { .. } => GC_HDR_KIND_ARRAY,
            GcCompositeInfo::Func => 0,
        };
        (self.canon_id << 3) | kind
    }
}

/// Constant value folded from an immutable/never-written non-exported global initializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X64ConstVal {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    V128([u8; 16]),
    RefNull,
    FuncRef(u32),
    /// Encoded `ref.i31` (`((x as u32) << 1) | 1`), stored zero-extended in 64 bits.
    I31(u32),
    /// Encoded `extern.convert_any(ref.i31)` (`GC_EXT_I31_TAG_HI | enc`).
    ExternI31(u32),
    /// Placeholder indicating a runtime GC-allocated initializer evaluated in `wt_initialize`.
    RuntimeGc,
}

/// Runtime-evaluated Wasm GC constant expression tree (for globals, table inits, and elem segments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GcConstExpr {
    Scalar(X64ConstVal),
    GlobalGet(u32),
    StructNew {
        ty: u32,
        fields: Vec<GcConstExpr>,
    },
    StructNewDefault(u32),
    ArrayNew {
        ty: u32,
        init: Box<GcConstExpr>,
        len: Box<GcConstExpr>,
    },
    ArrayNewDefault {
        ty: u32,
        len: Box<GcConstExpr>,
    },
    ArrayNewFixed {
        ty: u32,
        elems: Vec<GcConstExpr>,
    },
    RefI31(Box<GcConstExpr>),
    AnyConvertExtern(Box<GcConstExpr>),
    ExternConvertAny(Box<GcConstExpr>),
    I32Bin(u16, Box<GcConstExpr>, Box<GcConstExpr>),
    I64Bin(u16, Box<GcConstExpr>, Box<GcConstExpr>),
}

/// Static or dynamic `call_indirect` (or typed `call_ref`) dispatch metadata.
#[derive(Clone, Debug)]
pub struct X64CiSig {
    pub table: u32,
    pub table_len: u32,
    /// Representative index into `X64Module::module_types` (zero `FuncType` duplication).
    pub type_idx: u32,
    /// Canonical deduplicated signature ID (for dynamic funcref table `call_indirect` checks).
    pub canon_sig: u32,
    /// `true` iff `table` is dynamically mutable/growable (`dyn_tables`).
    pub is_dynamic: bool,
    /// `true` iff this entry dispatches a typed `call_ref` / `return_call_ref` (no table lookup).
    pub is_call_ref: bool,
    /// `true` iff `table` uses 64-bit (`table64`) indexing.
    pub table_is64: bool,
    pub min_slot: u32,
    pub span: u32,
    /// Sorted `(table_slot, func_index)` entries whose function signature equals `module_types[type_idx]`.
    pub targets: Vec<(u32, u32)>,
}

/// Metadata for a dynamically mutable/growable table stored via a 24-byte `Ctx` descriptor
/// `(ptr: u64, cur_len: u32, max_len: u32, reserve_bytes: u64)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynTableMeta {
    pub min_len: u32,
    pub eff_max_len: u32,
    pub is_funcref: bool,
    pub is_table64: bool,
    /// Reference hierarchy family stored in this table (controls GC root tracing).
    pub ref_family: GcRefFamily,
    /// Encoded initial slot value (`0` = null, `f + 1` = `ref.func f`, or encoded i31).
    pub init_val: u64,
}

/// Decoded element segment initializer (`0` = null, `f + 1` = `ref.func f`, or GC const exprs).
#[derive(Clone, Debug)]
pub struct ElemSegInit {
    pub is_passive: bool,
    pub active_target: Option<(u32, u64)>,
    pub entries: Vec<u32>,
    /// Reference hierarchy family of elements in this segment.
    pub ref_family: GcRefFamily,
    /// Present for element segments with GC/extern/exn or runtime-allocated expressions.
    pub gc_entries: Option<Vec<GcConstExpr>>,
}

/// Metadata for an additional linear memory (`memory_index >= 1`, Wasm `multi-memory`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtraMemMeta {
    pub min_pages: u64,
    pub max_pages: u64,
    pub memory64: bool,
    pub shared: bool,
}

/// Imported memory metadata `(module, name, min_pages, eff_max_pages, declared_max_pages, is64, shared)`.
pub type ImportedMemMeta = (String, String, u64, u64, Option<u64>, bool, bool);

/// Lean module metadata consumed by `x64/{ir,lower,mod}.rs`.
pub struct X64Module {
    /// Imported functions: `(module_name, import_name)`.
    pub imports: Vec<(String, String)>,
    /// Index into `module_types` for every function (`0..imports.len() + code.len()`) — zero per-function signature cloning.
    pub func_type_idx: Vec<u32>,
    /// Canonical structural signature ID for every function (`0..imports.len() + code.len()`).
    pub func_canon_sig: Vec<u32>,
    /// All declared module types (referenced by `func_type_idx`, `X64CiSig::type_idx`, and `Immediate::Block(idx)`).
    pub module_types: Vec<FuncType>,
    /// Globals: `(ValType, mutable)` for every global index.
    pub globals: Vec<(ValType, bool)>,
    /// Evaluated initializer value for every global (for runtime init of non-folded globals).
    pub global_inits: Vec<X64ConstVal>,
    /// `Some(c)` iff global `k` is proven constant AND not an exported mutable global.
    /// Every global with `global_consts[k].is_none()` gets a checked storage slot in `Ctx`.
    pub global_consts: Vec<Option<X64ConstVal>>,
    /// Single hottest mutable non-const scalar (`I32`/`I64`) global to pin in `R12`.
    pub pinned_global: Option<u32>,
    /// Second hottest mutable non-const scalar (`I32`/`I64`) global to pin in `R13` when `fixed32.is_some()`.
    pub pinned_global_2: Option<u32>,
    /// Exported globals: `(name, global_index, val_type, mutable)`.
    pub exported_globals: Vec<(String, usize, ValType, bool)>,
    /// Exported functions: `(name, func_index)`.
    pub exported_funcs: Vec<(String, usize)>,
    /// Maps `(table_idx, raw_type_idx)` -> index into `ci_sigs` (`O(log N)` deterministic `BTreeMap` lookup).
    pub ci_index: BTreeMap<(u32, u32), usize>,
    /// Deduplicated `call_indirect` dispatch descriptors.
    pub ci_sigs: Vec<X64CiSig>,
    /// `true` iff any `call_indirect` targets a dynamic `funcref` table (`dyn_tables`).
    pub uses_dyn_ci: bool,
    /// `true` iff any function uses `call_ref` or `return_call_ref`.
    pub uses_call_ref: bool,
    /// Declared initial (`min`) length for every table `0..n_tables`.
    pub table_lens: Vec<u32>,
    /// `true` iff table `t` uses 64-bit (`table64`) indexing (`0..n_tables`).
    pub table_is64: Vec<bool>,
    /// Fixed-size inline mutable `externref` tables: `table_idx -> exact_len`.
    pub mut_extern_tables: BTreeMap<u32, u32>,
    /// Dynamically mutable/growable (`funcref` or `externref`) tables backed by `Ctx` descriptors + mmap.
    pub dyn_tables: BTreeMap<u32, DynTableMeta>,
    /// Memory 0 declared limits: `Some((min_pages, declared_max_pages))` or `None` if 0 memories.
    pub memory: Option<(u64, u64)>,
    /// `true` iff memory 0 uses 64-bit (`i64`) addressing (`memory64`).
    pub memory64: bool,
    /// `true` iff memory 0 is declared `shared` (`threads`).
    pub memory_shared: bool,
    /// `Some(fixed_bytes)` iff memory 0 exists with `min == max`.
    pub fixed32: Option<u64>,
    /// Additional linear memories `1..N` (`multi-memory`; empty for all 0/1-memory modules).
    pub extra_memories: Vec<ExtraMemMeta>,
    /// Defined exception tag type indices (`exception_handling`; empty for non-EH modules).
    pub tags: Vec<u32>,
    /// `true` iff the module declares any exception tags or uses `try_table`/`throw`/`throw_ref`/`exnref`.
    pub uses_exceptions: bool,
    /// Compilation target (`BlotCdylib` vs `StandaloneExec`).
    pub target: X64Target,
    /// Exported `_start` function for `StandaloneExec`: `Some((func_idx, returns_i32))`.
    pub standalone_start: Option<(u32, bool)>,
    /// Exported `goose.reserve(control, count) -> ptr` for `StandaloneExec`: `Some((func_idx, is_i64))`.
    pub standalone_goose_reserve: Option<(u32, bool)>,
    /// Active element segment bounds checks `(table_len, base_off, elem_len)` for static funcref tables.
    pub active_elem_checks: Vec<(u32, u64, u32)>,
    /// Active data segments `(mem_idx, mem_off, decoded.data index)` for `wt_initialize` (in declaration order).
    pub active_data_segs: Vec<(u32, u64, usize)>,
    /// Total declared data segments (`decoded.data.len()`).
    pub data_seg_count: usize,
    /// Indices of passive data segments (`decoded.data` indices).
    pub passive_data_segs: Vec<usize>,
    /// `true` iff any function uses `memory.init` or `data.drop`.
    pub uses_bulk_data: bool,
    /// All declared element segments `0..decoded.elements.len()`.
    pub elem_segs: Vec<ElemSegInit>,
    /// `true` iff any function uses `table.init`, `elem.drop`, `array.new_elem`, or `array.init_elem`.
    pub uses_bulk_elem: bool,
    /// `true` iff the module uses any Wasm GC feature (rec groups, struct/array types, subtyping,
    /// GC heap types, GC const expressions, or GC opcodes `0xd3`/`0xfb00..=0xfb1e`).
    pub uses_gc: bool,
    /// `true` iff the module declares any shared memory or uses any atomic opcode (`0xfe00..=0xfe4e`).
    pub uses_threads: bool,
    /// Canonical type ID (`0..gc_canon_types.len()`) for every declared module type `0..module_types.len()`.
    pub type_canon: Vec<u32>,
    /// Canonical Wasm GC type descriptors (populated for all canonical types when `uses_gc`).
    pub gc_canon_types: Vec<GcCanonType>,
    /// Kind byte for every declared module type (`0 = Func, 1 = Struct, 2 = Array`).
    pub type_kinds: Vec<u8>,
    /// Runtime-evaluated GC global initializers `(global_index, expr)` in declaration order.
    pub runtime_global_inits: Vec<(usize, GcConstExpr)>,
    /// Runtime-evaluated GC explicit table initializers `(table_index, expr)` in declaration order.
    pub runtime_table_inits: Vec<(u32, GcConstExpr)>,
    /// Multi-module StoreCdylib imported metadata (empty/zero for `BlotCdylib` and `StandaloneExec`).
    pub imported_globals: Vec<(String, String, ValType, bool)>,
    pub imported_mems: Vec<ImportedMemMeta>,
    pub imported_tables: Vec<(String, String, wtime_types::TableType)>,
    pub imported_tags: Vec<(String, String, u32)>,
    /// Original Wasm import declaration order `(kind, kind_local_idx)` where
    /// `0=Func, 1=Table, 2=Mem, 3=Global, 4=Tag` (populated for `StoreCdylib`).
    pub import_order: Vec<(u8, u32)>,
    /// Declared optional maximum pages for every memory `0..total_mems` (populated for `StoreCdylib`).
    pub all_mem_declared_max: Vec<Option<u64>>,
    /// Declared `TableType` for every table `0..total_tables` (populated for `StoreCdylib`).
    pub all_table_types: Vec<wtime_types::TableType>,
    /// Multi-module StoreCdylib exported non-func/non-global metadata.
    pub exported_mems: Vec<(String, u32)>,
    pub exported_tables: Vec<(String, u32)>,
    pub exported_tags: Vec<(String, u32)>,
    /// Raw recursion groups preserved for cross-module `CanonicalTypeRegistry` in `StoreCdylib`.
    pub raw_type_groups: Vec<wtime_types::TypeGroup>,
    /// Optional start function index.
    pub start: Option<u32>,
}

impl X64Module {
    /// Returns `true` iff linear memory `mem` is declared `shared` (Wasm Threads).
    #[inline]
    pub fn mem_is_shared(&self, mem: u32) -> bool {
        if mem == 0 {
            self.memory_shared
        } else {
            self.extra_memories
                .get((mem - 1) as usize)
                .is_some_and(|em| em.shared)
        }
    }

    /// Borrows the canonical `FuncType` for function index `func_index` without cloning.
    #[inline]
    pub fn func_type(&self, func_index: usize) -> Option<&FuncType> {
        let &ti = self.func_type_idx.get(func_index)?;
        self.module_types.get(ti as usize)
    }

    /// Classifies a `HeapType` into its disjoint reference family.
    #[inline]
    pub fn heap_ref_family(&self, heap: HeapType) -> GcRefFamily {
        match heap {
            HeapType::Func | HeapType::NoFunc => GcRefFamily::Func,
            HeapType::Extern | HeapType::NoExtern => GcRefFamily::Extern,
            HeapType::Exn | HeapType::NoExn => GcRefFamily::Exn,
            HeapType::Any
            | HeapType::Eq
            | HeapType::I31
            | HeapType::Struct
            | HeapType::Array
            | HeapType::None => GcRefFamily::Any,
            HeapType::Defined(ti) | HeapType::ExactDefined(ti) => {
                if self.type_kinds.get(ti.0 as usize).copied().unwrap_or(0) == 0 {
                    GcRefFamily::Func
                } else {
                    GcRefFamily::Any
                }
            }
            HeapType::Cont | HeapType::NoCont => {
                unreachable!("x64: stack-switching cont types are rejected at admission")
            }
        }
    }

    /// Returns `true` iff canonical type `actual` is a subtype of `expected`.
    #[cfg(test)]
    #[inline]
    pub fn is_canon_subtype(&self, actual: u32, expected: u32) -> bool {
        if actual == expected {
            return true;
        }
        let Some(act) = self.gc_canon_types.get(actual as usize) else {
            return false;
        };
        let Some(exp) = self.gc_canon_types.get(expected as usize) else {
            return false;
        };
        if exp.is_final || act.depth < exp.depth {
            return false;
        }
        let d = exp.depth as usize;
        if d < GC_DISPLAY_LEN {
            act.display[d] == expected
        } else {
            act.supertypes_all.get(d).copied() == Some(expected)
        }
    }

    /// Returns the canonical `GcCanonType` for module type index `ti`.
    #[inline]
    pub fn gc_type(&self, ti: u32) -> &GcCanonType {
        let cid = self.type_canon[ti as usize] as usize;
        &self.gc_canon_types[cid]
    }

    /// Maps a Wasm `ValType` to its SSA IR `ValRefKind` (always `None` when `!self.uses_gc`).
    #[inline]
    pub fn val_ref_kind(&self, vt: ValType) -> super::ir::ValRefKind {
        if !self.uses_gc {
            return super::ir::ValRefKind::None;
        }
        match vt {
            ValType::Ref(rt) => match self.heap_ref_family(rt.heap) {
                GcRefFamily::Any => super::ir::ValRefKind::GcRef,
                GcRefFamily::Extern => super::ir::ValRefKind::ExternRef,
                GcRefFamily::Func | GcRefFamily::Exn => super::ir::ValRefKind::None,
            },
            _ => super::ir::ValRefKind::None,
        }
    }

    /// Maps a `GcStorageKind` to its SSA IR `ValRefKind`.
    #[inline]
    pub const fn storage_ref_kind(sk: GcStorageKind) -> super::ir::ValRefKind {
        match sk {
            GcStorageKind::GcRef => super::ir::ValRefKind::GcRef,
            GcStorageKind::ExternRef => super::ir::ValRefKind::ExternRef,
            _ => super::ir::ValRefKind::None,
        }
    }

    /// Maps a `GcStorageKind` to its unpacked SSA IR `Ty`.
    #[inline]
    pub const fn storage_to_ty(sk: GcStorageKind) -> super::ir::Ty {
        match sk {
            GcStorageKind::I8 | GcStorageKind::I16 | GcStorageKind::I32 => super::ir::Ty::I32,
            GcStorageKind::I64
            | GcStorageKind::GcRef
            | GcStorageKind::ExternRef
            | GcStorageKind::OtherRef => super::ir::Ty::I64,
            GcStorageKind::F32 => super::ir::Ty::F32,
            GcStorageKind::F64 => super::ir::Ty::F64,
            GcStorageKind::V128 => super::ir::Ty::V128,
        }
    }

    /// Maps a Wasm `ValType` to its SSA IR `Ty`.
    #[inline]
    pub fn val_type_to_ty(&self, vt: ValType) -> super::ir::Ty {
        super::ir::Ty::from_val(vt)
    }

    /// Returns `true` iff linear memory `mem` uses 64-bit (`memory64`) addressing.
    #[inline]
    pub fn mem_is_64(&self, mem: u32) -> bool {
        if mem == 0 {
            self.memory64
        } else {
            self.extra_memories
                .get((mem as usize).saturating_sub(1))
                .is_some_and(|m| m.memory64)
        }
    }

    /// Returns `true` iff table `tbl` uses 64-bit (`table64`) indexing.
    #[inline]
    pub fn table_is_64(&self, tbl: u32) -> bool {
        self.table_is64.get(tbl as usize).copied().unwrap_or(false)
    }

    /// Total number of declared linear memories (`0..=1 + MAX_EXTRA_MEMORIES`).
    #[inline]
    pub fn num_memories(&self) -> usize {
        usize::from(self.memory.is_some()) + self.extra_memories.len()
    }

    /// Computes deterministic 8B/16B-aligned payload field offsets and 16B-aligned total record size
    /// for exception tag `tag` (header at `+0: tag_idx: u32, +4: reserved: u32`, payload from `+8`).
    pub fn tag_payload_layout(&self, tag: u32) -> (Vec<(super::ir::Ty, i32)>, u32) {
        let ti = self.tags[tag as usize] as usize;
        let params = &self.module_types[ti].params;
        let mut off: i32 = 8;
        let mut fields = Vec::with_capacity(params.len());
        for &vt in params {
            let ty = self.val_type_to_ty(vt);
            let (sz, align) = if ty == super::ir::Ty::V128 {
                (16, 16)
            } else {
                (8, 8)
            };
            off = (off + (align - 1)) & !(align - 1);
            fields.push((ty, off));
            off += sz;
        }
        let total = ((off + 15) & !15).max(16) as u32;
        (fields, total)
    }
}

impl GcConstExpr {
    #[inline]
    pub fn as_scalar(&self) -> Option<X64ConstVal> {
        match *self {
            GcConstExpr::Scalar(v) if v != X64ConstVal::RuntimeGc => Some(v),
            _ => None,
        }
    }

    pub fn uses_gc(&self) -> bool {
        match self {
            GcConstExpr::Scalar(
                X64ConstVal::I31(_) | X64ConstVal::ExternI31(_) | X64ConstVal::RuntimeGc,
            ) => true,
            GcConstExpr::Scalar(_) | GcConstExpr::GlobalGet(_) => false,
            GcConstExpr::StructNew { .. }
            | GcConstExpr::StructNewDefault(_)
            | GcConstExpr::ArrayNew { .. }
            | GcConstExpr::ArrayNewDefault { .. }
            | GcConstExpr::ArrayNewFixed { .. }
            | GcConstExpr::RefI31(_)
            | GcConstExpr::AnyConvertExtern(_)
            | GcConstExpr::ExternConvertAny(_) => true,
            GcConstExpr::I32Bin(_, a, b) | GcConstExpr::I64Bin(_, a, b) => {
                a.uses_gc() || b.uses_gc()
            }
        }
    }
}

fn eval_const_expr_gc(
    expr: &[Instruction],
    prior: &[X64ConstVal],
    type_canon: &[u32],
    gc_canon_types: &[GcCanonType],
    const_nodes: &mut usize,
) -> Res<GcConstExpr> {
    let mut stk: Vec<GcConstExpr> = Vec::new();
    for i in expr {
        *const_nodes = const_nodes.saturating_add(1);
        if *const_nodes > MAX_GC_CONST_NODES {
            return Err(format!(
                "x64: aggregate const expression nodes exceed cap {MAX_GC_CONST_NODES}"
            ));
        }
        let v = match (i.opcode, &i.immediate) {
            (0x0b, _) => break,
            (0x41, Immediate::I32(k)) => GcConstExpr::Scalar(X64ConstVal::I32(*k)),
            (0x42, Immediate::I64(k)) => GcConstExpr::Scalar(X64ConstVal::I64(*k)),
            (0x43, Immediate::F32(b)) => GcConstExpr::Scalar(X64ConstVal::F32(*b)),
            (0x44, Immediate::F64(b)) => GcConstExpr::Scalar(X64ConstVal::F64(*b)),
            (0xfd0c, Immediate::Bytes(bytes)) => {
                let arr: [u8; 16] = bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| "invalid v128.const")?;
                GcConstExpr::Scalar(X64ConstVal::V128(arr))
            }
            (0xd0, _) => GcConstExpr::Scalar(X64ConstVal::RefNull),
            (0xd2, Immediate::Index(f)) => GcConstExpr::Scalar(X64ConstVal::FuncRef(*f)),
            (0x23, Immediate::Index(g)) => {
                let pv = *prior
                    .get(*g as usize)
                    .ok_or("const global.get out of range")?;
                if pv == X64ConstVal::RuntimeGc {
                    GcConstExpr::GlobalGet(*g)
                } else {
                    GcConstExpr::Scalar(pv)
                }
            }
            (0x6a..=0x6c, _) => {
                let b = stk.pop().ok_or("invalid i32 const expr")?;
                let a = stk.pop().ok_or("invalid i32 const expr")?;
                match (a.as_scalar(), b.as_scalar()) {
                    (Some(X64ConstVal::I32(av)), Some(X64ConstVal::I32(bv))) => {
                        GcConstExpr::Scalar(X64ConstVal::I32(match i.opcode {
                            0x6a => av.wrapping_add(bv),
                            0x6b => av.wrapping_sub(bv),
                            _ => av.wrapping_mul(bv),
                        }))
                    }
                    _ => GcConstExpr::I32Bin(i.opcode, Box::new(a), Box::new(b)),
                }
            }
            (0x7c..=0x7e, _) => {
                let b = stk.pop().ok_or("invalid i64 const expr")?;
                let a = stk.pop().ok_or("invalid i64 const expr")?;
                match (a.as_scalar(), b.as_scalar()) {
                    (Some(X64ConstVal::I64(av)), Some(X64ConstVal::I64(bv))) => {
                        GcConstExpr::Scalar(X64ConstVal::I64(match i.opcode {
                            0x7c => av.wrapping_add(bv),
                            0x7d => av.wrapping_sub(bv),
                            _ => av.wrapping_mul(bv),
                        }))
                    }
                    _ => GcConstExpr::I64Bin(i.opcode, Box::new(a), Box::new(b)),
                }
            }
            (0xfb1c, _) => {
                let x = stk.pop().ok_or("invalid ref.i31 const expr")?;
                if let Some(X64ConstVal::I32(k)) = x.as_scalar() {
                    let enc = ((k as u32) << 1) | 1;
                    GcConstExpr::Scalar(X64ConstVal::I31(enc))
                } else {
                    GcConstExpr::RefI31(Box::new(x))
                }
            }
            (0xfb1a, _) => {
                let x = stk.pop().ok_or("invalid any.convert_extern const expr")?;
                match x.as_scalar() {
                    Some(X64ConstVal::RefNull) => GcConstExpr::Scalar(X64ConstVal::RefNull),
                    Some(X64ConstVal::ExternI31(enc)) => GcConstExpr::Scalar(X64ConstVal::I31(enc)),
                    _ => GcConstExpr::AnyConvertExtern(Box::new(x)),
                }
            }
            (0xfb1b, _) => {
                let x = stk.pop().ok_or("invalid extern.convert_any const expr")?;
                match x.as_scalar() {
                    Some(X64ConstVal::RefNull) => GcConstExpr::Scalar(X64ConstVal::RefNull),
                    Some(X64ConstVal::I31(enc)) => GcConstExpr::Scalar(X64ConstVal::ExternI31(enc)),
                    _ => GcConstExpr::ExternConvertAny(Box::new(x)),
                }
            }
            (0xfb00, Immediate::Index(ty)) => {
                let cid = *type_canon
                    .get(*ty as usize)
                    .ok_or("struct.new const type out of range")?
                    as usize;
                let n_fields = match &gc_canon_types[cid].composite {
                    GcCompositeInfo::Struct { fields, .. } => fields.len(),
                    _ => return Err("struct.new const on non-struct type".into()),
                };
                if stk.len() < n_fields {
                    return Err("struct.new const stack underflow".into());
                }
                let fields = stk.split_off(stk.len() - n_fields);
                GcConstExpr::StructNew { ty: *ty, fields }
            }
            (0xfb01, Immediate::Index(ty)) => GcConstExpr::StructNewDefault(*ty),
            (0xfb06, Immediate::Index(ty)) => {
                let len = stk.pop().ok_or("array.new const stack underflow")?;
                let init = stk.pop().ok_or("array.new const stack underflow")?;
                GcConstExpr::ArrayNew {
                    ty: *ty,
                    init: Box::new(init),
                    len: Box::new(len),
                }
            }
            (0xfb07, Immediate::Index(ty)) => {
                let len = stk.pop().ok_or("array.new_default const stack underflow")?;
                GcConstExpr::ArrayNewDefault {
                    ty: *ty,
                    len: Box::new(len),
                }
            }
            (0xfb08, Immediate::Pair(ty, n)) => {
                let count = *n as usize;
                if stk.len() < count {
                    return Err("array.new_fixed const stack underflow".into());
                }
                let elems = stk.split_off(stk.len() - count);
                GcConstExpr::ArrayNewFixed { ty: *ty, elems }
            }
            (op, _) => return Err(format!("x64: unsupported const expression opcode {op:#x}")),
        };
        stk.push(v);
    }
    if stk.len() == 1 {
        Ok(stk.pop().unwrap())
    } else {
        Err("const expression did not yield one value".into())
    }
}

fn is_gc_heap_type(ht: HeapType, type_kinds: &[u8]) -> bool {
    match ht {
        HeapType::Any
        | HeapType::Eq
        | HeapType::I31
        | HeapType::Struct
        | HeapType::Array
        | HeapType::None
        | HeapType::NoFunc
        | HeapType::NoExtern
        | HeapType::NoExn => true,
        HeapType::Defined(ti) => type_kinds.get(ti.0 as usize).copied().unwrap_or(0) != 0,
        _ => false,
    }
}

fn is_gc_val_type(vt: ValType, type_kinds: &[u8]) -> bool {
    matches!(vt, ValType::Ref(rt) if is_gc_heap_type(rt.heap, type_kinds))
}

fn check_heap_type(ht: HeapType, n_types: usize) -> Res<()> {
    match ht {
        HeapType::Extern
        | HeapType::Func
        | HeapType::Exn
        | HeapType::Any
        | HeapType::Eq
        | HeapType::I31
        | HeapType::Struct
        | HeapType::Array
        | HeapType::None
        | HeapType::NoFunc
        | HeapType::NoExtern
        | HeapType::NoExn => Ok(()),
        HeapType::Defined(ti) if (ti.0 as usize) < n_types => Ok(()),
        _ => Err(format!("x64: unsupported heap type {ht:?}")),
    }
}

fn check_val_type(vt: ValType, n_types: usize) -> Res<()> {
    match vt {
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64 | ValType::V128 => Ok(()),
        ValType::Ref(rt) => check_heap_type(rt.heap, n_types),
    }
}

fn check_block_imm(
    imm: &Immediate,
    type_arity: &[usize],
    type_admitted: &[bool],
    type_kinds: &[u8],
    total_sig_arity: &mut usize,
    max_total_sig_arity: usize,
    uses_gc: &mut bool,
) -> Res<()> {
    let n_types = type_admitted.len();
    match imm {
        Immediate::Block(-64 | -1 | -2 | -3 | -4 | -5 | -16 | -17 | -23) => Ok(()),
        Immediate::Block(-22..=-18 | -15..=-12) => {
            *uses_gc = true;
            Ok(())
        }
        Immediate::Block(i) if *i >= 0 => {
            let idx = *i as usize;
            if !*type_admitted
                .get(idx)
                .ok_or("x64: block type index out of range")?
                || type_kinds.get(idx).copied() != Some(0)
            {
                return Err(format!(
                    "x64: block type index {idx} uses unsupported value type"
                ));
            }
            *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
            if *total_sig_arity > max_total_sig_arity {
                return Err(format!(
                    "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                ));
            }
            Ok(())
        }
        Immediate::BlockType(vt) => {
            check_val_type(*vt, n_types)?;
            if is_gc_val_type(*vt, type_kinds) {
                *uses_gc = true;
            }
            Ok(())
        }
        other => Err(format!("x64: unsupported block type {other:?}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn check_supported_opcode(
    op: u16,
    imm: &Immediate,
    func_type_idx: &[u32],
    tag_type_idx: &[u32],
    type_arity: &[usize],
    type_admitted: &[bool],
    type_kinds: &[u8],
    total_sig_arity: &mut usize,
    max_total_sig_arity: usize,
    uses_gc: &mut bool,
) -> Res<()> {
    let n_types = type_admitted.len();
    match op {
        0x00
        | 0x01
        | 0x05
        | 0x0a
        | 0x0b..=0x0f
        | 0x1a
        | 0x1b
        | 0x20..=0x26
        | 0x41..=0x44
        | 0x45..=0xc4
        | 0xd1
        | 0xd2
        | 0xd4
        | 0xd5
        | 0xd6 => Ok(()),
        0xd3 => {
            *uses_gc = true;
            Ok(())
        }
        0x08 => match imm {
            Immediate::Index(ti) => {
                let &fty = tag_type_idx
                    .get(*ti as usize)
                    .ok_or_else(|| format!("x64: throw tag index {ti} out of range"))?;
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[fty as usize]);
                if *total_sig_arity > max_total_sig_arity {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed throw immediate".into()),
        },
        0x1f => match imm {
            Immediate::TryTable(tt) => {
                check_block_imm(
                    &tt.block,
                    type_arity,
                    type_admitted,
                    type_kinds,
                    total_sig_arity,
                    max_total_sig_arity,
                    uses_gc,
                )?;
                for c in &tt.catches {
                    if let Some(tag) = c.tag {
                        let &fty = tag_type_idx
                            .get(tag as usize)
                            .ok_or_else(|| format!("x64: catch tag index {tag} out of range"))?;
                        *total_sig_arity = total_sig_arity.saturating_add(type_arity[fty as usize]);
                        if *total_sig_arity > max_total_sig_arity {
                            return Err(format!(
                                "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                            ));
                        }
                    }
                }
                Ok(())
            }
            _ => Err("x64: malformed try_table immediate".into()),
        },
        0x10 | 0x12 => match imm {
            Immediate::Index(fi) => {
                let &ti = func_type_idx
                    .get(*fi as usize)
                    .ok_or_else(|| format!("x64: call function index {fi} out of range"))?;
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[ti as usize]);
                if *total_sig_arity > max_total_sig_arity {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed call immediate".into()),
        },
        0x14 | 0x15 => match imm {
            Immediate::Index(ti) => {
                let idx = *ti as usize;
                if !*type_admitted
                    .get(idx)
                    .ok_or("x64: call_ref type index out of range")?
                    || type_kinds.get(idx).copied() != Some(0)
                {
                    return Err(format!(
                        "x64: call_ref type index {idx} uses unsupported value type"
                    ));
                }
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
                if *total_sig_arity > max_total_sig_arity {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed call_ref immediate".into()),
        },
        0x02..=0x04 => check_block_imm(
            imm,
            type_arity,
            type_admitted,
            type_kinds,
            total_sig_arity,
            max_total_sig_arity,
            uses_gc,
        ),
        0x11 | 0x13 => match imm {
            Immediate::Pair(ti, _) => {
                let idx = *ti as usize;
                if !*type_admitted
                    .get(idx)
                    .ok_or("x64: call_indirect type index out of range")?
                    || type_kinds.get(idx).copied() != Some(0)
                {
                    return Err(format!(
                        "x64: call_indirect type index {idx} uses unsupported value type"
                    ));
                }
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
                if *total_sig_arity > max_total_sig_arity {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed call_indirect immediate".into()),
        },
        0x1c => match imm {
            Immediate::Types(tys) => {
                for &vt in tys.iter() {
                    check_val_type(vt, n_types)?;
                    if is_gc_val_type(vt, type_kinds) {
                        *uses_gc = true;
                    }
                }
                Ok(())
            }
            _ => Err("x64: malformed typed select immediate".into()),
        },
        0x3f | 0x40 => match imm {
            Immediate::Index(_) | Immediate::None => Ok(()),
            _ => Err("x64: malformed memory.size/grow immediate".into()),
        },
        0xd0 => match imm {
            Immediate::Heap(ht) => {
                check_heap_type(*ht, n_types)?;
                if is_gc_heap_type(*ht, type_kinds) {
                    *uses_gc = true;
                }
                Ok(())
            }
            other => Err(format!("x64: unsupported ref.null heap type {other:?}")),
        },
        0x28..=0x3e | 0xfd00..=0xfd0b | 0xfd5c | 0xfd5d => match imm {
            Immediate::MemArg { .. } => Ok(()),
            _ => Err("x64: malformed memory load/store immediate".into()),
        },
        0xfd54..=0xfd5b => match imm {
            Immediate::MemLane { .. } => Ok(()),
            Immediate::Bytes(b) if b.len() == 17 => Ok(()),
            _ => Err("x64: malformed SIMD lane load/store immediate".into()),
        },
        0xfc00..=0xfc07 | 0xfc13..=0xfc16 => Ok(()),
        0xfc08 | 0xfc0c | 0xfc0e => match imm {
            Immediate::Pair(_, _) => Ok(()),
            _ => Err("x64: malformed bulk memory/table pair immediate".into()),
        },
        0xfc09 | 0xfc0d | 0xfc0f..=0xfc11 => match imm {
            Immediate::Index(_) => Ok(()),
            _ => Err("x64: malformed bulk memory/table index immediate".into()),
        },
        0xfc0a => match imm {
            Immediate::Pair(_, _) | Immediate::None => Ok(()),
            _ => Err("x64: malformed memory.copy immediate".into()),
        },
        0xfc0b => match imm {
            Immediate::Index(_) | Immediate::None => Ok(()),
            _ => Err("x64: malformed memory.fill immediate".into()),
        },
        // Wasm GC (`0xfb00..=0xfb1e`)
        0xfb00 | 0xfb01 | 0xfb06 | 0xfb07 | 0xfb0b..=0xfb0e | 0xfb10 => {
            *uses_gc = true;
            match imm {
                Immediate::Index(ti) if (*ti as usize) < n_types => Ok(()),
                _ => Err(format!("x64: malformed GC index immediate for {op:#x}")),
            }
        }
        0xfb02..=0xfb05 | 0xfb08..=0xfb0a | 0xfb11..=0xfb13 => {
            *uses_gc = true;
            match imm {
                Immediate::Pair(ti, _) if (*ti as usize) < n_types => Ok(()),
                _ => Err(format!("x64: malformed GC pair immediate for {op:#x}")),
            }
        }
        0xfb0f | 0xfb1a..=0xfb1e => {
            *uses_gc = true;
            Ok(())
        }
        0xfb14..=0xfb17 => {
            *uses_gc = true;
            match imm {
                Immediate::Heap(ht) => check_heap_type(*ht, n_types),
                _ => Err(format!(
                    "x64: malformed GC ref.test/cast immediate for {op:#x}"
                )),
            }
        }
        0xfb18 | 0xfb19 => {
            *uses_gc = true;
            match imm {
                Immediate::Cast { from, to, .. } => {
                    check_heap_type(*from, n_types)?;
                    check_heap_type(*to, n_types)
                }
                _ => Err(format!("x64: malformed br_on_cast immediate for {op:#x}")),
            }
        }
        0xfd0c..=0xfd53 | 0xfd5e..=0xfdff | 0xfa00..=0xfa13 => Ok(()),
        // Wasm Threads / Atomics (`0xfe00..=0xfe03`, `0xfe10..=0xfe4e`)
        0xfe03 => Ok(()),
        0xfe00..=0xfe02 | 0xfe10..=0xfe4e => match imm {
            Immediate::MemArg { .. } => Ok(()),
            _ => Err(format!("x64: malformed atomic immediate for {op:#x}")),
        },
        _ => Err(format!("x64: unsupported opcode {op:#x}")),
    }
}

fn check_blot_import_sig(mod_name: &str, name: &str, ty: &FuncType) -> Res<()> {
    let ok_params = matches!(
        ty.params.as_slice(),
        [] | [ValType::Ref(_)]
            | [ValType::I32]
            | [ValType::F32]
            | [ValType::Ref(_), ValType::I32]
            | [ValType::Ref(_), ValType::I64]
            | [ValType::Ref(_), ValType::F32]
            | [ValType::Ref(_), ValType::F64]
            | [ValType::Ref(_), ValType::Ref(_)]
    );
    let ok_results = matches!(
        ty.results.as_slice(),
        [] | [ValType::I32] | [ValType::I64] | [ValType::F32] | [ValType::F64] | [ValType::Ref(_)]
    );
    if !ok_params || !ok_results {
        return Err(format!(
            "x64: Blot import {mod_name:?}::{name:?} has unsupported signature {ty:?}"
        ));
    }
    Ok(())
}

/// Checks whether an import is supported natively by the standalone Linux syscall runtime.
pub fn check_standalone_import_sig(
    mod_name: &str,
    name: &str,
    ty: &FuncType,
    memory64: bool,
) -> Res<()> {
    use ValType::{F32, F64, I32, I64};
    let is_int = |t: ValType| matches!(t, I32 | I64);
    let ok = match (mod_name, name) {
        ("wasi_snapshot_preview1", "proc_exit") => {
            ty.params.len() == 1 && is_int(ty.params[0]) && ty.results.is_empty()
        }
        ("wasi_snapshot_preview1", "fd_write" | "fd_read") => {
            ty.params.len() == 4
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("wasi_snapshot_preview1", "fd_close") => {
            ty.params.len() == 1
                && is_int(ty.params[0])
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("wasi_snapshot_preview1", "fd_seek") => {
            ty.params.len() == 4
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        (
            "wasi_snapshot_preview1",
            "args_sizes_get" | "args_get" | "environ_sizes_get" | "environ_get",
        ) => {
            ty.params.len() == 2
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("wasi_snapshot_preview1", "clock_time_get") => {
            ty.params.len() == 3
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("goose", "write") => {
            ty.params.len() == 2 && ty.params.iter().all(|&t| is_int(t)) && ty.results.is_empty()
        }
        ("goose", "print_i64" | "print_u64" | "print_bool" | "exit") => {
            ty.params.len() == 1 && is_int(ty.params[0]) && ty.results.is_empty()
        }
        ("goose", "print_f64") => {
            ty.params.len() == 1 && matches!(ty.params[0], F64 | F32) && ty.results.is_empty()
        }
        ("goose", "print_f32") => {
            ty.params.len() == 1 && matches!(ty.params[0], F32 | F64) && ty.results.is_empty()
        }
        ("goose", "format_f64") => {
            ty.params.len() == 2
                && is_int(ty.params[0])
                && matches!(ty.params[1], F64 | F32)
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("goose", "format_f32") => {
            ty.params.len() == 2
                && is_int(ty.params[0])
                && matches!(ty.params[1], F32 | F64)
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("goose", "newline") => ty.params.is_empty() && ty.results.is_empty(),
        ("goose", "abort") => {
            ty.params.len() == 3 && ty.params.iter().all(|&t| is_int(t)) && ty.results.is_empty()
        }
        ("env", "gs_os_write_stdout" | "gs_os_write_stderr") => {
            let expected = if memory64 { 2 } else { 1 };
            (ty.params.len() == expected || ty.params.len() == 2)
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() <= 1
                && ty.results.iter().all(|&t| is_int(t))
        }
        (
            "env",
            "gs_os_clock_ns" | "bench_clock" | "bench_input" | "gs_os_time_ns" | "gs_os_arg_count",
        ) => ty.params.is_empty() && ty.results.len() == 1 && is_int(ty.results[0]),
        ("env", "gs_os_resource_dir") => {
            ty.params.len() <= 1
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("env", "gs_os_arg") => {
            ty.params.len() == 2 && ty.params.iter().all(|&t| is_int(t)) && ty.results.is_empty()
        }
        ("env", "gs_os_file_exists" | "gs_os_is_dir") => {
            matches!(ty.params.len(), 1 | 2)
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("env", "gs_os_read_file" | "gs_os_getenv" | "gs_find_any") => {
            matches!(ty.params.len(), 2 | 3)
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("env", "gs_os_write_file" | "gs_os_append_file") => {
            matches!(ty.params.len(), 2 | 4)
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("env", "gs_find_pair") => {
            matches!(ty.params.len(), 4 | 5)
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        ("env", "bench_input_f64") => {
            ty.params.is_empty() && ty.results.len() == 1 && matches!(ty.results[0], F64 | F32)
        }
        ("env", "gs_os_flush_stdout" | "bench_sink" | "emscripten_notify_memory_growth") => {
            ty.params.len() <= 1
                && ty
                    .params
                    .iter()
                    .all(|&t| matches!(t, I32 | I64 | F32 | F64))
                && ty.results.is_empty()
        }
        ("env", "bench_barrier") => {
            ty.params.len() == 1
                && ty.results == ty.params
                && matches!(ty.params[0], I32 | I64 | F32 | F64)
        }
        ("env", "system") => {
            ty.params.len() <= 2
                && ty.params.iter().all(|&t| is_int(t))
                && ty.results.len() == 1
                && is_int(ty.results[0])
        }
        _ => false,
    };
    if !ok {
        return Err(format!(
            "x64: standalone executable import {mod_name:?}::{name:?} ({ty:?}) is not supported"
        ));
    }
    Ok(())
}

/// Per-body admission summary cached in `ValIndexData` so incremental builds skip scanning
/// unchanged function bodies while producing bit-for-bit identical `pinned_global`,
/// `global_consts`, `ci_sigs`, and aggregate caps.
#[derive(Clone, Debug, Default)]
pub struct BodyAdmitSummary {
    pub total_locals: u32,
    pub insn_count: u32,
    pub imm_elems: u32,
    pub saw_data_count_op: bool,
    pub uses_exceptions: bool,
    pub uses_bulk_data: bool,
    pub uses_bulk_elem: bool,
    pub uses_gc: bool,
    pub uses_threads: bool,
    pub dyn_table_ops: Vec<u32>,
    pub sig_arity: usize,
    pub br_table_entries: usize,
    pub g_reads: Vec<(u32, u32)>,
    pub g_writes: Vec<(u32, u32)>,
    pub ci_pairs: Vec<(u32, u32)>,
}

fn rewrite_subtype_indices(
    ty: &mut SubType,
    base: usize,
    end: usize,
    preceding: &[u32],
) -> Res<()> {
    let map_idx = |idx: TypeIdx| -> Res<TypeIdx> {
        let n = idx.0 as usize;
        if n < base {
            Ok(TypeIdx(preceding[n]))
        } else if n < end {
            let local = u32::try_from(n - base).map_err(|_| "x64: rec group too large")?;
            Ok(TypeIdx(0x8000_0000 | local))
        } else {
            Err("x64: type reference outside recursion group".into())
        }
    };
    if let Some(ref mut s) = ty.supertype {
        *s = map_idx(*s)?;
    }
    let map_val = |v: &mut ValType| -> Res<()> {
        if let ValType::Ref(r) = v
            && let HeapType::Defined(ref mut i) | HeapType::ExactDefined(ref mut i) = r.heap
        {
            *i = map_idx(*i)?;
        }
        Ok(())
    };
    let map_field = |f: &mut FieldType| -> Res<()> {
        if let StorageType::Val(ref mut v) = f.storage {
            map_val(v)?;
        }
        Ok(())
    };
    match &mut ty.composite {
        CompositeType::Func(ft) => {
            for v in ft.params.iter_mut().chain(&mut ft.results) {
                map_val(v)?;
            }
        }
        CompositeType::Struct(st) => {
            for f in &mut st.fields {
                map_field(f)?;
            }
        }
        CompositeType::Array(at) => map_field(&mut at.field)?,
        CompositeType::Cont(_) => {
            return Err("x64: stack-switching continuation types are not supported".into());
        }
    }
    Ok(())
}

fn storage_to_gc_kind(st: StorageType, type_kinds: &[u8]) -> Res<GcStorageKind> {
    Ok(match st {
        StorageType::I8 => GcStorageKind::I8,
        StorageType::I16 => GcStorageKind::I16,
        StorageType::Val(vt) => match vt {
            ValType::I32 => GcStorageKind::I32,
            ValType::I64 => GcStorageKind::I64,
            ValType::F32 => GcStorageKind::F32,
            ValType::F64 => GcStorageKind::F64,
            ValType::V128 => GcStorageKind::V128,
            ValType::Ref(rt) => match rt.heap {
                HeapType::Any
                | HeapType::Eq
                | HeapType::I31
                | HeapType::Struct
                | HeapType::Array
                | HeapType::None => GcStorageKind::GcRef,
                HeapType::Extern | HeapType::NoExtern => GcStorageKind::ExternRef,
                HeapType::Func | HeapType::NoFunc | HeapType::Exn | HeapType::NoExn => {
                    GcStorageKind::OtherRef
                }
                HeapType::Defined(ti) => {
                    if type_kinds.get(ti.0 as usize).copied().unwrap_or(0) == 0 {
                        GcStorageKind::OtherRef
                    } else {
                        GcStorageKind::GcRef
                    }
                }
                _ => return Err(format!("x64: unsupported GC field heap type {:?}", rt.heap)),
            },
        },
    })
}

fn ref_family_from_heap(ht: HeapType, type_kinds: &[u8]) -> Res<GcRefFamily> {
    Ok(match ht {
        HeapType::Func | HeapType::NoFunc => GcRefFamily::Func,
        HeapType::Extern | HeapType::NoExtern => GcRefFamily::Extern,
        HeapType::Exn | HeapType::NoExn => GcRefFamily::Exn,
        HeapType::Any
        | HeapType::Eq
        | HeapType::I31
        | HeapType::Struct
        | HeapType::Array
        | HeapType::None => GcRefFamily::Any,
        HeapType::Defined(ti) => {
            if type_kinds.get(ti.0 as usize).copied().unwrap_or(0) == 0 {
                GcRefFamily::Func
            } else {
                GcRefFamily::Any
            }
        }
        _ => return Err(format!("x64: unsupported heap type {ht:?}")),
    })
}

/// Enforces the admission contract and builds lean `X64Module` metadata (Blot cdylib target).
#[allow(dead_code)]
pub fn admit_x64_module(decoded: &DecodedModule) -> Res<X64Module> {
    admit_x64_module_for_target(decoded, X64Target::BlotCdylib, None)
}

/// Enforces the admission contract for `target` while cooperatively polling `budget`'s deadline.
pub fn admit_x64_module_for_target(
    decoded: &DecodedModule,
    target: X64Target,
    budget: Option<&CompileBudget>,
) -> Res<X64Module> {
    admit_x64_module_for_target_with_summaries(decoded, target, budget, None, false).map(|(m, _)| m)
}

/// Enforces the admission contract for `target`, optionally reusing per-body `ValBodyEntry`
/// summaries directly by reference for unchanged bodies (zero `Vec` cloning!) and/or collecting
/// per-body summaries for caching. When `pre_vidx` skips a body, the corresponding slot in the
/// returned `Vec<BodyAdmitSummary>` is left `Default` so the caller can move the cached
/// `ValBodyEntry` directly without cloning.
pub fn admit_x64_module_for_target_with_summaries(
    decoded: &DecodedModule,
    target: X64Target,
    budget: Option<&CompileBudget>,
    pre_vidx: Option<(&[super::inc_cache::ValBodyEntry], &[bool])>,
    collect_summaries: bool,
) -> Res<(X64Module, Vec<BodyAdmitSummary>)> {
    if let Some(b) = budget {
        b.check_deadline()?;
    }
    if decoded.tags.len() > MAX_TAGS {
        return Err(format!(
            "x64: module declares {} exception tags (max {MAX_TAGS})",
            decoded.tags.len()
        ));
    }
    let mut uses_gc = false;
    let n_types = decoded.types.len();
    if n_types > MAX_MODULE_TYPES {
        return Err(format!(
            "x64: module declares {n_types} types (max {MAX_MODULE_TYPES})"
        ));
    }
    let synthetic_type_groups: Vec<wtime_types::TypeGroup> =
        if decoded.type_groups.is_empty() && !decoded.types.is_empty() {
            decoded
                .types
                .iter()
                .map(|ft| wtime_types::TypeGroup {
                    explicit: false,
                    types: vec![SubType {
                        final_: true,
                        supertype: None,
                        descriptor: None,
                        describes: None,
                        composite: CompositeType::Func(ft.clone()),
                    }],
                })
                .collect()
        } else {
            Vec::new()
        };
    let type_groups: &[wtime_types::TypeGroup] = if decoded.type_groups.is_empty() {
        &synthetic_type_groups
    } else {
        &decoded.type_groups
    };
    let mut type_kinds: Vec<u8> = Vec::with_capacity(n_types);
    for group in type_groups {
        if group.explicit {
            uses_gc = true;
        }
        for st in &group.types {
            if st.descriptor.is_some() || st.describes.is_some() {
                return Err("x64: custom descriptor types are not supported".into());
            }
            if !st.final_ || st.supertype.is_some() {
                uses_gc = true;
            }
            match &st.composite {
                CompositeType::Func(_) => type_kinds.push(0),
                CompositeType::Struct(s) => {
                    if s.fields.len() > MAX_GC_STRUCT_FIELDS {
                        return Err(format!(
                            "x64: GC struct field count ({}) exceeds cap {MAX_GC_STRUCT_FIELDS}",
                            s.fields.len()
                        ));
                    }
                    uses_gc = true;
                    type_kinds.push(1);
                }
                CompositeType::Array(_) => {
                    uses_gc = true;
                    type_kinds.push(2);
                }
                CompositeType::Cont(_) => {
                    return Err("x64: stack-switching continuation types are not supported".into());
                }
            }
        }
    }
    if type_kinds.len() != n_types {
        return Err("x64: decoded type_groups count mismatch with types".into());
    }
    let is_store = target == X64Target::StoreCdylib;
    // Pre-partition non-function imports (admitted ONLY for `StoreCdylib`).
    let mut imp_mem_defs: Vec<(String, String, wtime_types::MemoryType)> = Vec::new();
    let mut imp_table_defs: Vec<(String, String, wtime_types::TableType)> = Vec::new();
    let mut imp_global_defs: Vec<(String, String, wtime_types::GlobalType)> = Vec::new();
    let mut imp_tag_defs: Vec<(String, String, wtime_types::TagType)> = Vec::new();
    let mut import_order: Vec<(u8, u32)> = Vec::new();
    let mut imp_func_ord: u32 = 0;
    for imp in &decoded.imports {
        match &imp.kind {
            ImportKind::Func(_) => {
                if is_store {
                    import_order.push((0, imp_func_ord));
                    imp_func_ord += 1;
                }
            }
            ImportKind::Table(tt) if is_store => {
                import_order.push((1, imp_table_defs.len() as u32));
                imp_table_defs.push((imp.module.clone(), imp.name.clone(), *tt));
            }
            ImportKind::Memory(mt) if is_store => {
                import_order.push((2, imp_mem_defs.len() as u32));
                imp_mem_defs.push((imp.module.clone(), imp.name.clone(), *mt));
            }
            ImportKind::Global(gt) if is_store => {
                import_order.push((3, imp_global_defs.len() as u32));
                imp_global_defs.push((imp.module.clone(), imp.name.clone(), *gt));
            }
            ImportKind::Tag(tt) if is_store => {
                import_order.push((4, imp_tag_defs.len() as u32));
                imp_tag_defs.push((imp.module.clone(), imp.name.clone(), *tt));
            }
            _ => {
                return Err(format!(
                    "x64: only plain function imports are supported (got {:?}::{:?})",
                    imp.module, imp.name
                ));
            }
        }
    }
    let total_mems = imp_mem_defs.len() + decoded.memories.len();
    if total_mems > 1 + MAX_EXTRA_MEMORIES {
        return Err(format!(
            "x64: memory count ({total_mems}) exceeds cap {}",
            1 + MAX_EXTRA_MEMORIES
        ));
    }
    let check_mem = |mi: usize, mt: &wtime_types::MemoryType| -> Res<(u64, u64, bool, bool)> {
        let is_mem64 = mt.index_type == IndexType::I64;
        if mt.page_size_log2 != 16 {
            return Err(format!(
                "x64: custom memory page size (2^{}) on memory {mi} is not supported",
                mt.page_size_log2
            ));
        }
        let max_limit = if is_mem64 {
            MAX_MEM64_PAGES
        } else {
            MAX_MEM_PAGES
        };
        let max_p = match mt.limits.max {
            Some(m) if m > max_limit => {
                return Err(format!(
                    "x64: memory {mi} max ({m}) exceeds {} limit ({max_limit})",
                    if is_mem64 { "memory64" } else { "Wasm32" }
                ));
            }
            Some(m) => m,
            None => max_limit,
        };
        if mt.limits.min > max_p {
            return Err(format!(
                "x64: memory {mi} min ({}) exceeds max ({max_p})",
                mt.limits.min
            ));
        }
        Ok((mt.limits.min, max_p, is_mem64, mt.shared))
    };
    let mut uses_threads = false;
    let mut all_mems_checked: Vec<(u64, u64, bool, bool)> = Vec::with_capacity(total_mems);
    let mut all_mem_declared_max: Vec<Option<u64>> = Vec::with_capacity(total_mems);
    let mut imported_mems = Vec::with_capacity(imp_mem_defs.len());
    for (mi, (m_mod, m_name, mt)) in imp_mem_defs.iter().enumerate() {
        let (min_p, max_p, m64, is_shared) = check_mem(mi, mt)?;
        uses_threads |= is_shared;
        all_mems_checked.push((min_p, max_p, m64, is_shared));
        all_mem_declared_max.push(mt.limits.max);
        imported_mems.push((
            m_mod.clone(),
            m_name.clone(),
            min_p,
            max_p,
            mt.limits.max,
            m64,
            is_shared,
        ));
    }
    for (di, mt) in decoded.memories.iter().enumerate() {
        let mi = imp_mem_defs.len() + di;
        let (min_p, max_p, m64, is_shared) = check_mem(mi, mt)?;
        uses_threads |= is_shared;
        all_mems_checked.push((min_p, max_p, m64, is_shared));
        all_mem_declared_max.push(mt.limits.max);
    }
    let (memory, memory64, memory_shared, fixed32) =
        if let Some(&(min_p, max_p, is_mem64, is_shared)) = all_mems_checked.first() {
            let fixed = if !is_store && min_p == max_p {
                min_p.checked_mul(65_536)
            } else {
                None
            };
            (Some((min_p, max_p)), is_mem64, is_shared, fixed)
        } else {
            (None, false, false, None)
        };
    let mut extra_memories = Vec::with_capacity(all_mems_checked.len().saturating_sub(1));
    for &(min_pages, max_pages, m64, is_shared) in all_mems_checked.iter().skip(1) {
        extra_memories.push(ExtraMemMeta {
            min_pages,
            max_pages,
            memory64: m64,
            shared: is_shared,
        });
    }

    // Canonicalize all `decoded.type_groups` into structural rec-group canonical IDs
    // (exact match with `wtime-runtime/src/canonical.rs`) and compute `GcCanonType` layouts
    // and Cohen supertype displays.
    let mut group_to_canon: HashMap<Vec<SubType>, Vec<u32>> =
        HashMap::with_capacity(type_groups.len());
    let mut type_canon: Vec<u32> = Vec::with_capacity(n_types);
    let mut gc_canon_types: Vec<GcCanonType> = Vec::new();
    let mut type_arity: Vec<usize> = Vec::with_capacity(n_types);
    let mut type_admitted: Vec<bool> = Vec::with_capacity(n_types);
    let mut declared_type_arity: usize = 0;
    let mut has_func_subtyping = false;
    let mut flat_ti = 0usize;
    for group in type_groups {
        if let Some(b) = budget
            && (flat_ti & 4095) == 4095
        {
            b.check_deadline()?;
        }
        let base = type_canon.len();
        let end = base + group.types.len();
        let mut normalized = group.types.clone();
        for st in &mut normalized {
            rewrite_subtype_indices(st, base, end, &type_canon)?;
        }
        let ids = if let Some(existing) = group_to_canon.get(&normalized) {
            existing.clone()
        } else {
            let first = gc_canon_types.len() as u32;
            let ids: Vec<u32> = (0..group.types.len() as u32).map(|i| first + i).collect();
            let resolve_norm = |idx: TypeIdx| -> u32 {
                if (idx.0 & 0x8000_0000) != 0 {
                    first + (idx.0 & 0x7fff_ffff)
                } else {
                    idx.0
                }
            };
            // First push skeletons with resolved supertype & composite, then fill supertype displays.
            for (local_i, (orig_st, norm_st)) in group.types.iter().zip(&normalized).enumerate() {
                let cid = first + local_i as u32;
                let super_cid = norm_st.supertype.map(resolve_norm);
                let composite = match &orig_st.composite {
                    CompositeType::Func(_) => {
                        if !orig_st.final_ || super_cid.is_some() {
                            has_func_subtyping = true;
                        }
                        GcCompositeInfo::Func
                    }
                    CompositeType::Struct(s) => {
                        let mut off: u32 = 8;
                        let mut fields = Vec::with_capacity(s.fields.len());
                        let mut gc_ref_offs = Vec::new();
                        let mut ext_ref_offs = Vec::new();
                        for f in &s.fields {
                            let sk = storage_to_gc_kind(f.storage, &type_kinds)?;
                            let align = sk.align();
                            off = off
                                .checked_add(align - 1)
                                .map(|v| v & !(align - 1))
                                .ok_or("x64: GC struct offset overflow")?;
                            if sk == GcStorageKind::GcRef {
                                gc_ref_offs.push(off);
                            } else if sk == GcStorageKind::ExternRef {
                                ext_ref_offs.push(off);
                            }
                            fields.push(GcStructField {
                                offset: off,
                                storage: sk,
                                mutable: f.mutable,
                                non_null: matches!(
                                    f.storage,
                                    StorageType::Val(ValType::Ref(rt)) if !rt.nullable
                                ),
                            });
                            off = off
                                .checked_add(sk.byte_size())
                                .ok_or("x64: GC struct size overflow")?;
                        }
                        let total_bytes = off
                            .checked_add(7)
                            .map(|v| (v & !7).max(8))
                            .ok_or("x64: GC struct aligned size overflow")?;
                        GcCompositeInfo::Struct {
                            fields,
                            total_bytes,
                            gc_ref_offs,
                            ext_ref_offs,
                        }
                    }
                    CompositeType::Array(at) => GcCompositeInfo::Array {
                        elem: storage_to_gc_kind(at.field.storage, &type_kinds)?,
                        mutable: at.field.mutable,
                        elem_non_null: matches!(
                            at.field.storage,
                            StorageType::Val(ValType::Ref(rt)) if !rt.nullable
                        ),
                    },
                    CompositeType::Cont(_) => unreachable!(),
                };
                gc_canon_types.push(GcCanonType {
                    canon_id: cid,
                    is_final: orig_st.final_,
                    supertype: super_cid,
                    depth: 0,
                    display: [u32::MAX; GC_DISPLAY_LEN],
                    supertypes_all: Vec::new(),
                    composite,
                });
            }
            // Compute supertype chain & Cohen display for each newly created canonical type.
            for local_i in 0..group.types.len() {
                let cid = first + local_i as u32;
                let mut rev_chain = Vec::new();
                let mut cur = Some(cid);
                while let Some(id) = cur {
                    if rev_chain.len() > MAX_GC_SUBTYPE_DEPTH {
                        return Err(format!(
                            "x64: GC supertype depth exceeds cap {MAX_GC_SUBTYPE_DEPTH}"
                        ));
                    }
                    rev_chain.push(id);
                    cur = gc_canon_types[id as usize].supertype;
                }
                rev_chain.reverse();
                let depth = (rev_chain.len() - 1) as u32;
                let mut display = [u32::MAX; GC_DISPLAY_LEN];
                for (k, &anc) in rev_chain.iter().take(GC_DISPLAY_LEN).enumerate() {
                    display[k] = anc;
                }
                let entry = &mut gc_canon_types[cid as usize];
                entry.depth = depth;
                entry.display = display;
                entry.supertypes_all = rev_chain;
            }
            group_to_canon.insert(normalized, ids.clone());
            ids
        };
        for (st, cid) in group.types.iter().zip(ids) {
            type_canon.push(cid);
            let (arity, admitted) = match &st.composite {
                CompositeType::Func(fty) => {
                    let ar = fty.params.len().saturating_add(fty.results.len());
                    let ok = fty.params.iter().chain(&fty.results).all(|&vt| {
                        if is_gc_val_type(vt, &type_kinds) {
                            uses_gc = true;
                        }
                        check_val_type(vt, n_types).is_ok()
                    });
                    (ar, ok)
                }
                CompositeType::Struct(s) => {
                    let ok = s.fields.iter().all(|f| match f.storage {
                        StorageType::I8 | StorageType::I16 => true,
                        StorageType::Val(vt) => check_val_type(vt, n_types).is_ok(),
                    });
                    (s.fields.len(), ok)
                }
                CompositeType::Array(a) => {
                    let ok = match a.field.storage {
                        StorageType::I8 | StorageType::I16 => true,
                        StorageType::Val(vt) => check_val_type(vt, n_types).is_ok(),
                    };
                    (1, ok)
                }
                CompositeType::Cont(_) => (0, false),
            };
            let max_total_sig_arity = if is_store {
                STORE_MAX_TOTAL_SIG_ARITY
            } else {
                MAX_TOTAL_SIG_ARITY
            };
            declared_type_arity = declared_type_arity.saturating_add(arity);
            if declared_type_arity > max_total_sig_arity {
                return Err(format!(
                    "x64: total type-section signature arity exceeds cap ({max_total_sig_arity})"
                ));
            }
            type_arity.push(arity);
            type_admitted.push(admitted);
            flat_ti += 1;
        }
    }

    let mut tags: Vec<u32> = Vec::with_capacity(imp_tag_defs.len() + decoded.tags.len());
    let mut imported_tags = Vec::with_capacity(imp_tag_defs.len());
    for (ti, (t_mod, t_name, tag)) in imp_tag_defs.iter().enumerate() {
        let fti = tag.function.0 as usize;
        let fty = decoded
            .types
            .get(fti)
            .ok_or_else(|| format!("x64: imported tag {ti} type index {fti} out of range"))?;
        if !fty.results.is_empty() {
            return Err(format!("x64: imported tag {ti} must have empty results"));
        }
        if !type_admitted[fti] {
            return Err(format!(
                "x64: imported tag {ti} type index {fti} uses unsupported value type"
            ));
        }
        tags.push(tag.function.0);
        imported_tags.push((t_mod.clone(), t_name.clone(), tag.function.0));
    }
    for (di, tag) in decoded.tags.iter().enumerate() {
        let ti = imp_tag_defs.len() + di;
        let fti = tag.function.0 as usize;
        let fty = decoded
            .types
            .get(fti)
            .ok_or_else(|| format!("x64: tag {ti} type index {fti} out of range"))?;
        if !fty.results.is_empty() {
            return Err(format!("x64: tag {ti} must have empty results"));
        }
        if !type_admitted[fti] {
            return Err(format!(
                "x64: tag {ti} type index {fti} uses unsupported value type"
            ));
        }
        tags.push(tag.function.0);
    }

    // Charge per-function/total locals AND aggregate signature arity (params + results) BEFORE allocating.
    if decoded.functions.len() != decoded.code.len() {
        return Err(format!(
            "x64: function/code count mismatch ({} vs {})",
            decoded.functions.len(),
            decoded.code.len()
        ));
    }
    let max_total_locals = if is_store {
        STORE_MAX_TOTAL_LOCALS
    } else {
        MAX_TOTAL_LOCALS
    };
    let max_total_sig_arity = if is_store {
        STORE_MAX_TOTAL_SIG_ARITY
    } else {
        MAX_TOTAL_SIG_ARITY
    };
    let mut precheck_total_locals = 0usize;
    let mut total_sig_arity = 0usize;
    for (bi, body) in decoded.code.iter().enumerate() {
        if let Some(b) = budget
            && (bi & 4095) == 4095
        {
            b.check_deadline()?;
        }
        let ti = decoded
            .functions
            .get(bi)
            .ok_or("function/code count mismatch")?
            .0 as usize;
        let fty = decoded
            .types
            .get(ti)
            .ok_or("function type index out of range")?;
        let body_locals = pre_vidx
            .and_then(|(bs, skip)| {
                skip.get(bi)
                    .copied()
                    .unwrap_or(false)
                    .then(|| bs.get(bi))
                    .flatten()
            })
            .map_or(body.locals.len(), |pre| pre.total_locals as usize);
        let n_locals = fty.params.len().saturating_add(body_locals);
        if n_locals > MAX_LOCALS_PER_FN {
            return Err(format!(
                "x64: function body {bi} has {n_locals} locals (cap {MAX_LOCALS_PER_FN})"
            ));
        }
        precheck_total_locals = precheck_total_locals.saturating_add(n_locals);
        if precheck_total_locals > max_total_locals {
            return Err(format!(
                "x64: module total locals exceeds cap {max_total_locals}"
            ));
        }
        total_sig_arity = total_sig_arity.saturating_add(type_arity[ti]);
        if total_sig_arity > max_total_sig_arity {
            return Err(format!(
                "x64: module expanded signature arity exceeds cap {max_total_sig_arity}"
            ));
        }
    }

    // Imports: plain function imports only (zero per-function `FuncType` cloning).
    let mut imports = Vec::new();
    let mut func_type_idx: Vec<u32> =
        Vec::with_capacity(decoded.imports.len() + decoded.functions.len());
    let mut func_canon_sig: Vec<u32> =
        Vec::with_capacity(decoded.imports.len() + decoded.functions.len());
    for (ii, imp) in decoded.imports.iter().enumerate() {
        if let Some(b) = budget
            && (ii & 4095) == 4095
        {
            b.check_deadline()?;
        }
        let ti_u32 = match imp.kind {
            ImportKind::Func(t) => t.0,
            ImportKind::Memory(_)
            | ImportKind::Table(_)
            | ImportKind::Global(_)
            | ImportKind::Tag(_)
                if is_store =>
            {
                continue;
            }
            _ => {
                return Err(format!(
                    "x64: only plain function imports are supported (got {:?}::{:?})",
                    imp.module, imp.name
                ));
            }
        };
        let ti = ti_u32 as usize;
        let fty = decoded
            .types
            .get(ti)
            .ok_or("import type index out of range")?;
        if !type_admitted[ti] {
            return Err(format!(
                "x64: import {:?}::{:?} uses unsupported value type",
                imp.module, imp.name
            ));
        }
        total_sig_arity = total_sig_arity.saturating_add(type_arity[ti]);
        if total_sig_arity > max_total_sig_arity {
            return Err(format!(
                "x64: module expanded signature arity exceeds cap {max_total_sig_arity}"
            ));
        }
        match target {
            X64Target::BlotCdylib => check_blot_import_sig(&imp.module, &imp.name, fty)?,
            X64Target::StandaloneExec => {
                check_standalone_import_sig(&imp.module, &imp.name, fty, memory64)?;
            }
            X64Target::StoreCdylib => {}
        }
        imports.push((imp.module.clone(), imp.name.clone()));
        func_type_idx.push(ti_u32);
        func_canon_sig.push(type_canon[ti]);
    }

    for &ti in &decoded.functions {
        let ti_u = ti.0 as usize;
        if !*type_admitted
            .get(ti_u)
            .ok_or("function type index out of range")?
        {
            return Err(format!(
                "x64: function type index {ti_u} uses unsupported value type"
            ));
        }
        func_type_idx.push(ti.0);
        func_canon_sig.push(type_canon[ti_u]);
    }

    // Tables: classify metadata first (32-bit/64-bit funcref, typed concrete-func ref, externref,
    // exnref, and Wasm GC ref tables, with optional explicit table initializers); partitioning
    // into static funcref, inline fixed externref, and dynamic mmap-backed tables happens right
    // after scanning bodies.
    let mut const_nodes = 0usize;
    struct RawTableMeta {
        min_len: u32,
        eff_max_len: u32,
        is_funcref: bool,
        is_table64: bool,
        ref_family: GcRefFamily,
        init_val: u64,
        has_runtime_init: bool,
        fixed_inline_extern_eligible: bool,
    }
    let total_tables = imp_table_defs.len() + decoded.tables.len();
    let mut raw_tables: Vec<RawTableMeta> = Vec::with_capacity(total_tables);
    let mut table_lens: Vec<u32> = Vec::with_capacity(total_tables);
    let mut table_is64: Vec<bool> = Vec::with_capacity(total_tables);
    let mut imported_tables = Vec::with_capacity(imp_table_defs.len());
    let mut runtime_table_inits: Vec<(u32, GcConstExpr)> = Vec::new();
    let mut total_funcref_slots: usize = 0;
    let mut total_extern_slots: usize = 0;
    let all_table_types: Vec<(bool, wtime_types::TableType)> = imp_table_defs
        .iter()
        .map(|(m, n, tt)| {
            imported_tables.push((m.clone(), n.clone(), *tt));
            (true, *tt)
        })
        .chain(decoded.tables.iter().map(|tt| (false, *tt)))
        .collect();
    for (ti, (is_imp_tbl, tt)) in all_table_types.iter().enumerate() {
        let ti_u32 = u32::try_from(ti).map_err(|_| "too many tables")?;
        let is_table64 = tt.index_type == IndexType::I64;
        check_heap_type(tt.element.heap, n_types)?;
        if is_gc_heap_type(tt.element.heap, &type_kinds) {
            uses_gc = true;
        }
        let ref_family = ref_family_from_heap(tt.element.heap, &type_kinds)?;
        let (init_val, has_runtime_init) = if !*is_imp_tbl
            && let Some(init_expr) = decoded
                .table_inits
                .get(ti - imp_table_defs.len())
                .and_then(|o| o.as_ref())
        {
            let expr = eval_const_expr_gc(
                init_expr,
                &[],
                &type_canon,
                &gc_canon_types,
                &mut const_nodes,
            )?;
            if expr.uses_gc() {
                uses_gc = true;
            }
            match expr.as_scalar() {
                Some(X64ConstVal::RefNull) => (0u64, false),
                Some(X64ConstVal::FuncRef(f)) => (
                    u64::from(f)
                        .checked_add(1)
                        .ok_or("x64: table init funcref overflow")?,
                    false,
                ),
                Some(X64ConstVal::I31(enc)) => (u64::from(enc), false),
                Some(X64ConstVal::ExternI31(enc)) => (GC_EXT_I31_TAG_HI | u64::from(enc), false),
                None => {
                    runtime_table_inits.push((ti_u32, expr));
                    (0u64, true)
                }
                Some(other) => {
                    return Err(format!(
                        "x64: unsupported explicit table {ti} initializer {other:?}"
                    ));
                }
            }
        } else {
            (0u64, false)
        };
        match ref_family {
            GcRefFamily::Func => {
                let len = u32::try_from(tt.limits.min)
                    .ok()
                    .filter(|&n| n <= MAX_FUNCREF_TABLE_SLOTS)
                    .ok_or_else(|| format!("x64: funcref table {ti} min ({}) exceeds cap {MAX_FUNCREF_TABLE_SLOTS}", tt.limits.min))?;
                total_funcref_slots = total_funcref_slots.saturating_add(len as usize);
                if total_funcref_slots > MAX_TOTAL_FUNCREF_SLOTS {
                    return Err(format!(
                        "x64: aggregate funcref table slots ({total_funcref_slots}) exceed cap {MAX_TOTAL_FUNCREF_SLOTS}"
                    ));
                }
                let eff_max = tt
                    .limits
                    .max
                    .unwrap_or(u64::from(MAX_DYN_TABLE_SLOTS))
                    .min(u64::from(MAX_DYN_TABLE_SLOTS)) as u32;
                table_lens.push(len);
                table_is64.push(is_table64);
                raw_tables.push(RawTableMeta {
                    min_len: len,
                    eff_max_len: eff_max,
                    is_funcref: true,
                    is_table64,
                    ref_family,
                    init_val,
                    has_runtime_init,
                    fixed_inline_extern_eligible: false,
                });
            }
            GcRefFamily::Extern | GcRefFamily::Exn => {
                let len = u32::try_from(tt.limits.min)
                    .ok()
                    .filter(|&n| n <= MAX_DYN_TABLE_SLOTS)
                    .ok_or_else(|| {
                        format!(
                            "x64: externref table {ti} min ({}) exceeds cap {MAX_DYN_TABLE_SLOTS}",
                            tt.limits.min
                        )
                    })?;
                let fixed_inline_ok = init_val == 0
                    && !has_runtime_init
                    && tt.limits.max == Some(tt.limits.min)
                    && tt.limits.min <= u64::from(MAX_EXTERN_TABLE_SLOTS);
                if fixed_inline_ok {
                    total_extern_slots = total_extern_slots.saturating_add(len as usize);
                }
                let eff_max = tt
                    .limits
                    .max
                    .unwrap_or(u64::from(MAX_DYN_TABLE_SLOTS))
                    .min(u64::from(MAX_DYN_TABLE_SLOTS)) as u32;
                table_lens.push(len);
                table_is64.push(is_table64);
                raw_tables.push(RawTableMeta {
                    min_len: len,
                    eff_max_len: eff_max,
                    is_funcref: false,
                    is_table64,
                    ref_family,
                    init_val,
                    has_runtime_init,
                    fixed_inline_extern_eligible: fixed_inline_ok
                        && total_extern_slots <= MAX_TOTAL_EXTERN_SLOTS,
                });
            }
            GcRefFamily::Any => {
                let len = u32::try_from(tt.limits.min)
                    .ok()
                    .filter(|&n| n <= MAX_DYN_TABLE_SLOTS)
                    .ok_or_else(|| {
                        format!(
                            "x64: GC ref table {ti} min ({}) exceeds cap {MAX_DYN_TABLE_SLOTS}",
                            tt.limits.min
                        )
                    })?;
                let eff_max = tt
                    .limits
                    .max
                    .unwrap_or(u64::from(MAX_DYN_TABLE_SLOTS))
                    .min(u64::from(MAX_DYN_TABLE_SLOTS)) as u32;
                table_lens.push(len);
                table_is64.push(is_table64);
                raw_tables.push(RawTableMeta {
                    min_len: len,
                    eff_max_len: eff_max,
                    is_funcref: false,
                    is_table64,
                    ref_family,
                    init_val,
                    has_runtime_init,
                    fixed_inline_extern_eligible: false,
                });
            }
        }
    }

    let total_globals = imp_global_defs.len() + decoded.globals.len();
    if total_globals > MAX_GLOBALS {
        return Err(format!(
            "x64: module global count ({total_globals}) exceeds cap {MAX_GLOBALS}",
        ));
    }

    // Globals: prepend imported globals (StoreCdylib only), then evaluate all defined initializers.
    let mut globals = Vec::with_capacity(total_globals);
    let mut global_inits = Vec::with_capacity(total_globals);
    let mut imported_globals = Vec::with_capacity(imp_global_defs.len());
    let mut runtime_global_inits: Vec<(usize, GcConstExpr)> = Vec::new();
    for (g_mod, g_name, gt) in &imp_global_defs {
        check_val_type(gt.value, n_types)?;
        if is_gc_val_type(gt.value, &type_kinds) {
            uses_gc = true;
        }
        globals.push((gt.value, gt.mutable));
        global_inits.push(X64ConstVal::RuntimeGc);
        imported_globals.push((g_mod.clone(), g_name.clone(), gt.value, gt.mutable));
    }
    for (di, g) in decoded.globals.iter().enumerate() {
        let gi = imp_global_defs.len() + di;
        if let Some(b) = budget
            && (gi & 4095) == 4095
        {
            b.check_deadline()?;
        }
        check_val_type(g.ty.value, n_types)?;
        if is_gc_val_type(g.ty.value, &type_kinds) {
            uses_gc = true;
        }
        let expr = eval_const_expr_gc(
            &g.init,
            &global_inits,
            &type_canon,
            &gc_canon_types,
            &mut const_nodes,
        )?;
        if expr.uses_gc() {
            uses_gc = true;
        }
        globals.push((g.ty.value, g.ty.mutable));
        if let Some(sv) = expr.as_scalar() {
            global_inits.push(sv);
        } else {
            global_inits.push(X64ConstVal::RuntimeGc);
            runtime_global_inits.push((gi, expr));
        }
    }

    // Exports: admit functions, globals, and memories (plus tables & tags in `StoreCdylib`).
    if decoded.exports.len() > MAX_EXPORTS {
        return Err(format!(
            "x64: module export count ({}) exceeds cap {MAX_EXPORTS}",
            decoded.exports.len()
        ));
    }
    let mut exported_funcs = Vec::new();
    let mut exported_globals = Vec::new();
    let mut exported_mems = Vec::new();
    let mut exported_tables = Vec::new();
    let mut exported_tags = Vec::new();
    let mut exported_mut_globals = vec![false; globals.len()];
    let mut exported_any_globals = vec![false; globals.len()];
    for (ei, exp) in decoded.exports.iter().enumerate() {
        if let Some(b) = budget
            && (ei & 4095) == 4095
        {
            b.check_deadline()?;
        }
        match exp.kind {
            ExternalKind::Func => {
                let fi = exp.index as usize;
                if fi >= func_type_idx.len() {
                    return Err(format!("x64: exported function index {fi} out of range"));
                }
                exported_funcs.push((exp.name.clone(), fi));
            }
            ExternalKind::Global => {
                let gi = exp.index as usize;
                let (vt, mutable) = *globals.get(gi).ok_or("exported global out of range")?;
                exported_any_globals[gi] = true;
                if mutable {
                    exported_mut_globals[gi] = true;
                }
                exported_globals.push((exp.name.clone(), gi, vt, mutable));
            }
            ExternalKind::Memory => {
                if (exp.index as usize) >= total_mems {
                    return Err(format!(
                        "x64: unsupported memory export {:?} (index {})",
                        exp.name, exp.index
                    ));
                }
                if is_store {
                    exported_mems.push((exp.name.clone(), exp.index));
                }
            }
            ExternalKind::Table if is_store => {
                if (exp.index as usize) >= raw_tables.len() {
                    return Err(format!(
                        "x64: exported table index {} out of range",
                        exp.index
                    ));
                }
                exported_tables.push((exp.name.clone(), exp.index));
            }
            ExternalKind::Tag if is_store => {
                if (exp.index as usize) >= tags.len() {
                    return Err(format!(
                        "x64: exported tag index {} out of range",
                        exp.index
                    ));
                }
                exported_tags.push((exp.name.clone(), exp.index));
            }
            ExternalKind::Table | ExternalKind::Tag => {
                return Err(format!(
                    "x64: unsupported export {:?} ({:?})",
                    exp.name, exp.kind
                ));
            }
        }
    }

    // Element segments: admit active, passive, and declarative segments (both funcref and GC).
    let mut elem_segs: Vec<ElemSegInit> = Vec::with_capacity(decoded.elements.len());
    let mut has_gc_elem_segs = false;
    let mut table_has_active_elem = vec![false; raw_tables.len()];
    let mut elem_steps: usize = 0;
    let mut total_elem_entries: usize = 0;
    let parse_elem_entries = |inits: &[Vec<Instruction>],
                              ref_family: GcRefFamily,
                              steps: &mut usize,
                              total: &mut usize,
                              c_nodes: &mut usize,
                              saw_gc: &mut bool|
     -> Res<(Vec<u32>, Option<Vec<GcConstExpr>>)> {
        *total = total.saturating_add(inits.len());
        if *total > MAX_ELEM_SEG_ENTRIES {
            return Err(format!(
                "x64: aggregate element entries ({total}) exceed cap {MAX_ELEM_SEG_ENTRIES}"
            ));
        }
        let mut func_entries = if ref_family == GcRefFamily::Func {
            Some(Vec::with_capacity(inits.len()))
        } else {
            None
        };
        let mut gc_entries: Option<Vec<GcConstExpr>> = if ref_family == GcRefFamily::Func {
            None
        } else {
            Some(Vec::with_capacity(inits.len()))
        };
        for expr in inits {
            *steps += 1;
            if let Some(b) = budget
                && (*steps & 4095) == 0
            {
                b.check_deadline()?;
            }
            let e = eval_const_expr_gc(expr, &global_inits, &type_canon, &gc_canon_types, c_nodes)?;
            if e.uses_gc() {
                *saw_gc = true;
            }
            if let Some(ref mut fe) = func_entries {
                match e.as_scalar() {
                    Some(X64ConstVal::RefNull) => fe.push(0),
                    Some(X64ConstVal::FuncRef(f)) => {
                        fe.push(
                            f.checked_add(1)
                                .ok_or("x64: element funcref index overflow")?,
                        );
                    }
                    _ => {
                        // Fall back to runtime GC element evaluation.
                        let mut ge = Vec::with_capacity(inits.len());
                        for &prev in fe.iter() {
                            ge.push(GcConstExpr::Scalar(if prev == 0 {
                                X64ConstVal::RefNull
                            } else {
                                X64ConstVal::FuncRef(prev - 1)
                            }));
                        }
                        ge.push(e);
                        func_entries = None;
                        gc_entries = Some(ge);
                    }
                }
            } else if let Some(ref mut ge) = gc_entries {
                ge.push(e);
            }
        }
        Ok((func_entries.unwrap_or_default(), gc_entries))
    };
    for elem in &decoded.elements {
        check_heap_type(elem.ty.heap, n_types)?;
        if is_gc_heap_type(elem.ty.heap, &type_kinds) {
            uses_gc = true;
        }
        let ref_family = ref_family_from_heap(elem.ty.heap, &type_kinds)?;
        match &elem.mode {
            SegmentMode::Active { index, offset } => {
                if (*index as usize) >= raw_tables.len() {
                    return Err(format!(
                        "x64: active element segment targets out-of-range table {index}"
                    ));
                }
                let off_val = eval_const_expr_gc(
                    offset,
                    &global_inits,
                    &type_canon,
                    &gc_canon_types,
                    &mut const_nodes,
                )?
                .as_scalar()
                .ok_or("x64: active element offset must be constant")?;
                let off_u64 = if raw_tables[*index as usize].is_table64 {
                    let X64ConstVal::I64(off_i64) = off_val else {
                        return Err("x64: table64 active element offset must be i64".into());
                    };
                    off_i64 as u64
                } else {
                    let X64ConstVal::I32(off_i32) = off_val else {
                        return Err("x64: active element offset must be i32".into());
                    };
                    u64::from(off_i32 as u32)
                };
                let (entries, gc_entries) = parse_elem_entries(
                    &elem.init,
                    ref_family,
                    &mut elem_steps,
                    &mut total_elem_entries,
                    &mut const_nodes,
                    &mut uses_gc,
                )?;
                if gc_entries.is_some() {
                    has_gc_elem_segs = true;
                }
                table_has_active_elem[*index as usize] = true;
                elem_segs.push(ElemSegInit {
                    is_passive: false,
                    active_target: Some((*index, off_u64)),
                    entries,
                    ref_family,
                    gc_entries,
                });
            }
            SegmentMode::Passive => {
                let (entries, gc_entries) = parse_elem_entries(
                    &elem.init,
                    ref_family,
                    &mut elem_steps,
                    &mut total_elem_entries,
                    &mut const_nodes,
                    &mut uses_gc,
                )?;
                if gc_entries.is_some() {
                    has_gc_elem_segs = true;
                }
                elem_segs.push(ElemSegInit {
                    is_passive: true,
                    active_target: None,
                    entries,
                    ref_family,
                    gc_entries,
                });
            }
            SegmentMode::Declarative => {
                elem_segs.push(ElemSegInit {
                    is_passive: false,
                    active_target: None,
                    entries: Vec::new(),
                    ref_family,
                    gc_entries: None,
                });
            }
        }
    }

    if has_gc_elem_segs {
        uses_gc = true;
    }

    // Data segments: admit both active and passive data segments.
    let mut active_data_segs = Vec::new();
    let mut passive_data_segs = Vec::new();
    let data_seg_count = decoded.data.len();
    for (di, seg) in decoded.data.iter().enumerate() {
        if let Some(b) = budget
            && (di & 4095) == 4095
        {
            b.check_deadline()?;
        }
        match &seg.mode {
            SegmentMode::Active { index, offset } => {
                let m_idx = *index as usize;
                if m_idx >= total_mems {
                    return Err(format!(
                        "x64: active data segment on out-of-range memory {index}"
                    ));
                }
                let is_m64 = if m_idx == 0 {
                    memory64
                } else {
                    extra_memories[m_idx - 1].memory64
                };
                let off_scalar = eval_const_expr_gc(
                    offset,
                    &global_inits,
                    &type_canon,
                    &gc_canon_types,
                    &mut const_nodes,
                )?
                .as_scalar()
                .ok_or("x64: active data offset must be constant")?;
                let off_u64 = if is_m64 {
                    let X64ConstVal::I64(off_i64) = off_scalar else {
                        return Err("x64: memory64 active data offset must be i64".into());
                    };
                    off_i64 as u64
                } else {
                    let X64ConstVal::I32(off_i32) = off_scalar else {
                        return Err("x64: active data offset must be i32".into());
                    };
                    u64::from(off_i32 as u32)
                };
                active_data_segs.push((*index, off_u64, di));
            }
            SegmentMode::Passive => {
                passive_data_segs.push(di);
            }
            SegmentMode::Declarative => {}
        }
    }

    // Linear scan over ALL function bodies (including unreachable code), parallelized across
    // contiguous chunks on large modules and merged in deterministic body-index order so:
    // 1. Opcode, local-count, value-type, expanded-signature, and br_table caps match identically;
    // 2. Global read/write counts & `pinned_global` selection are bit-for-bit identical;
    // 3. Canonical `(table, canon_sig_id)` `call_indirect` tables are built in identical encounter order.
    struct ChunkScan {
        sig_arity: usize,
        br_table_entries: usize,
        uses_exceptions: bool,
        uses_bulk_data: bool,
        uses_bulk_elem: bool,
        uses_gc: bool,
        uses_threads: bool,
        dyn_table_ops: Vec<u32>,
        global_reads: Vec<usize>,
        global_writes: Vec<usize>,
        ci_pairs: Vec<(u32, u32)>,
        body_summaries: Vec<BodyAdmitSummary>,
    }

    let n_globals = globals.len();
    let raw_table_is_funcref: Vec<bool> = raw_tables.iter().map(|t| t.is_funcref).collect();
    let raw_table_fixed_inline: Vec<bool> = raw_tables
        .iter()
        .map(|t| t.fixed_inline_extern_eligible)
        .collect();
    let scan_chunk = |base_bi: usize, bodies: &[wtime_validate::FunctionBody]| -> Res<ChunkScan> {
        let mut sig_arity = 0usize;
        let mut br_table_entries = 0usize;
        let mut chunk_uses_exceptions = false;
        let mut chunk_uses_bulk_data = false;
        let mut chunk_uses_bulk_elem = false;
        let mut chunk_uses_gc = false;
        let mut chunk_uses_threads = false;
        let mut chunk_dyn_seen: HashSet<u32> = HashSet::new();
        let mut chunk_dyn_tables: Vec<u32> = Vec::new();
        let mut g_reads = vec![0usize; n_globals];
        let mut g_writes = vec![0usize; n_globals];
        let mut ci_seen: HashSet<(u32, u32)> = HashSet::new();
        let mut ci_pairs: Vec<(u32, u32)> = Vec::new();
        let mut body_summaries = if collect_summaries {
            Vec::with_capacity(bodies.len())
        } else {
            Vec::new()
        };
        let mut touched_reads: Vec<u32> = Vec::new();
        let mut touched_writes: Vec<u32> = Vec::new();
        let mut body_g_reads = if collect_summaries {
            vec![0u32; n_globals]
        } else {
            Vec::new()
        };
        let mut body_g_writes = if collect_summaries {
            vec![0u32; n_globals]
        } else {
            Vec::new()
        };
        for (local_bi, body) in bodies.iter().enumerate() {
            let bi = base_bi + local_bi;
            if let Some(b) = budget
                && (local_bi & 63) == 0
            {
                b.check_deadline()?;
            }
            if let Some((bs, skip)) = pre_vidx
                && skip.get(bi).copied().unwrap_or(false)
                && let Some(pre) = bs.get(bi)
            {
                sig_arity = sig_arity.saturating_add(pre.sig_arity as usize);
                br_table_entries = br_table_entries.saturating_add(pre.br_table_entries as usize);
                chunk_uses_exceptions |= pre.uses_exceptions;
                chunk_uses_bulk_data |= pre.uses_bulk_data;
                chunk_uses_bulk_elem |= pre.uses_bulk_elem;
                chunk_uses_gc |= pre.uses_gc;
                chunk_uses_threads |= pre.uses_threads;
                for &t in &pre.dyn_table_ops {
                    if chunk_dyn_seen.insert(t) {
                        chunk_dyn_tables.push(t);
                    }
                }
                if br_table_entries > MAX_AGGREGATE_BR_TABLE_ENTRIES {
                    return Err(format!(
                        "x64: aggregate br_table entries ({br_table_entries}) exceed cap {MAX_AGGREGATE_BR_TABLE_ENTRIES}"
                    ));
                }
                for &(g, cnt) in &pre.g_reads {
                    let slot = g_reads
                        .get_mut(g as usize)
                        .ok_or_else(|| format!("x64: global.get index {g} out of range"))?;
                    *slot += cnt as usize;
                }
                for &(g, cnt) in &pre.g_writes {
                    let slot = g_writes
                        .get_mut(g as usize)
                        .ok_or_else(|| format!("x64: global.set index {g} out of range"))?;
                    *slot += cnt as usize;
                }
                for &pair in &pre.ci_pairs {
                    if ci_seen.insert(pair) {
                        ci_pairs.push(pair);
                    }
                }
                if collect_summaries {
                    body_summaries.push(BodyAdmitSummary::default());
                }
                continue;
            }
            let b_sig_before = sig_arity;
            let b_br_before = br_table_entries;
            let mut b_ci: Vec<(u32, u32)> = Vec::new();
            let mut b_dyn_tables: Vec<u32> = Vec::new();
            let mut b_imm_elems = 0usize;
            let mut b_saw_data_count_op = false;
            let mut b_uses_exceptions = false;
            let mut b_uses_bulk_data = false;
            let mut b_uses_bulk_elem = false;
            let mut b_uses_gc = false;
            let mut b_uses_threads = false;
            let mut mark_dyn_table = |t: u32| -> Res<()> {
                if (t as usize) >= raw_table_is_funcref.len() {
                    return Err(format!("x64: table index {t} out of range"));
                }
                if chunk_dyn_seen.insert(t) {
                    chunk_dyn_tables.push(t);
                }
                if collect_summaries && !b_dyn_tables.contains(&t) {
                    b_dyn_tables.push(t);
                }
                Ok(())
            };
            for &vt in &body.locals {
                check_val_type(vt, n_types)?;
                if is_gc_val_type(vt, &type_kinds) {
                    b_uses_gc = true;
                }
                if matches!(vt, ValType::Ref(rt) if matches!(rt.heap, HeapType::Exn | HeapType::NoExn))
                {
                    b_uses_exceptions = true;
                }
            }
            for (ii, insn) in body.instructions.iter().enumerate() {
                if let Some(b) = budget
                    && (ii & 4095) == 4095
                {
                    b.check_deadline()?;
                }
                check_supported_opcode(
                    insn.opcode,
                    &insn.immediate,
                    &func_type_idx,
                    &tags,
                    &type_arity,
                    &type_admitted,
                    &type_kinds,
                    &mut sig_arity,
                    max_total_sig_arity,
                    &mut b_uses_gc,
                )?;
                if matches!(insn.opcode, 0xfe00..=0xfe03 | 0xfe10..=0xfe4e) {
                    b_uses_threads = true;
                }
                if matches!(insn.opcode, 0x08 | 0x0a | 0x1f)
                    || matches!(
                        insn.immediate,
                        Immediate::Heap(HeapType::Exn | HeapType::NoExn)
                    )
                {
                    b_uses_exceptions = true;
                }
                if collect_summaries {
                    match &insn.immediate {
                        Immediate::BrTable(targets, _) => {
                            b_imm_elems =
                                b_imm_elems.saturating_add(targets.len().saturating_add(1));
                        }
                        Immediate::TryTable(tt) => {
                            b_imm_elems = b_imm_elems.saturating_add(tt.catches.len());
                        }
                        _ => {}
                    }
                    if matches!(insn.opcode, 0xfc08 | 0xfc09 | 0xfb09 | 0xfb12) {
                        b_saw_data_count_op = true;
                    }
                }
                match insn.opcode {
                    0x0e => {
                        if let Immediate::BrTable(ref raw_targets, _) = insn.immediate {
                            br_table_entries = br_table_entries.saturating_add(raw_targets.len());
                            if br_table_entries > MAX_AGGREGATE_BR_TABLE_ENTRIES {
                                return Err(format!(
                                    "x64: aggregate br_table entries ({br_table_entries}) exceed cap {MAX_AGGREGATE_BR_TABLE_ENTRIES}"
                                ));
                            }
                        }
                    }
                    0x23 => {
                        if let Immediate::Index(g) = insn.immediate {
                            let slot = g_reads
                                .get_mut(g as usize)
                                .ok_or_else(|| format!("x64: global.get index {g} out of range"))?;
                            *slot += 1;
                            if collect_summaries {
                                let bs = &mut body_g_reads[g as usize];
                                if *bs == 0 {
                                    touched_reads.push(g);
                                }
                                *bs = bs.saturating_add(1);
                            }
                        }
                    }
                    0x24 => {
                        if let Immediate::Index(g) = insn.immediate {
                            let slot = g_writes
                                .get_mut(g as usize)
                                .ok_or_else(|| format!("x64: global.set index {g} out of range"))?;
                            *slot += 1;
                            if collect_summaries {
                                let bs = &mut body_g_writes[g as usize];
                                if *bs == 0 {
                                    touched_writes.push(g);
                                }
                                *bs = bs.saturating_add(1);
                            }
                        }
                    }
                    0x25 | 0x26 => {
                        if let Immediate::Index(t) = insn.immediate {
                            let ti = t as usize;
                            if ti >= raw_table_is_funcref.len() {
                                return Err(format!(
                                    "x64: table.get/set on out-of-range table {t}"
                                ));
                            }
                            if raw_table_is_funcref[ti] || !raw_table_fixed_inline[ti] {
                                mark_dyn_table(t)?;
                            }
                        }
                    }
                    0xfc08 | 0xfc09 | 0xfb09 | 0xfb12 => {
                        b_uses_bulk_data = true;
                    }
                    0xfb0a | 0xfb13 => {
                        b_uses_bulk_elem = true;
                    }
                    0xfc0c => {
                        b_uses_bulk_elem = true;
                        if let Immediate::Pair(_, t) = insn.immediate {
                            mark_dyn_table(t)?;
                        }
                    }
                    0xfc0d => {
                        b_uses_bulk_elem = true;
                    }
                    0xfc0e => {
                        if let Immediate::Pair(dt, st) = insn.immediate {
                            mark_dyn_table(dt)?;
                            mark_dyn_table(st)?;
                        }
                    }
                    0xfc0f | 0xfc11 => {
                        if let Immediate::Index(t) = insn.immediate {
                            mark_dyn_table(t)?;
                        }
                    }
                    0x11 | 0x13 => {
                        let Immediate::Pair(ti, tbl) = insn.immediate else {
                            return Err("x64: malformed call_indirect immediate".into());
                        };
                        if ci_seen.insert((tbl, ti)) {
                            ci_pairs.push((tbl, ti));
                        }
                        if collect_summaries && !b_ci.contains(&(tbl, ti)) {
                            b_ci.push((tbl, ti));
                        }
                    }
                    0x14 | 0x15 => {
                        let Immediate::Index(ti) = insn.immediate else {
                            return Err("x64: malformed call_ref immediate".into());
                        };
                        if ci_seen.insert((u32::MAX, ti)) {
                            ci_pairs.push((u32::MAX, ti));
                        }
                        if collect_summaries && !b_ci.contains(&(u32::MAX, ti)) {
                            b_ci.push((u32::MAX, ti));
                        }
                    }
                    _ => {}
                }
            }
            chunk_uses_exceptions |= b_uses_exceptions;
            chunk_uses_bulk_data |= b_uses_bulk_data;
            chunk_uses_bulk_elem |= b_uses_bulk_elem;
            chunk_uses_gc |= b_uses_gc;
            chunk_uses_threads |= b_uses_threads;
            if collect_summaries {
                let mut gr = Vec::with_capacity(touched_reads.len());
                for g in touched_reads.drain(..) {
                    let c = body_g_reads[g as usize];
                    body_g_reads[g as usize] = 0;
                    gr.push((g, c));
                }
                let mut gw = Vec::with_capacity(touched_writes.len());
                for g in touched_writes.drain(..) {
                    let c = body_g_writes[g as usize];
                    body_g_writes[g as usize] = 0;
                    gw.push((g, c));
                }
                body_summaries.push(BodyAdmitSummary {
                    total_locals: body.locals.len() as u32,
                    insn_count: body.instructions.len() as u32,
                    imm_elems: b_imm_elems as u32,
                    saw_data_count_op: b_saw_data_count_op,
                    uses_exceptions: b_uses_exceptions,
                    uses_bulk_data: b_uses_bulk_data,
                    uses_bulk_elem: b_uses_bulk_elem,
                    uses_gc: b_uses_gc,
                    uses_threads: b_uses_threads,
                    dyn_table_ops: b_dyn_tables,
                    sig_arity: sig_arity - b_sig_before,
                    br_table_entries: br_table_entries - b_br_before,
                    g_reads: gr,
                    g_writes: gw,
                    ci_pairs: b_ci,
                });
            }
        }
        Ok(ChunkScan {
            sig_arity,
            br_table_entries,
            uses_exceptions: chunk_uses_exceptions,
            uses_bulk_data: chunk_uses_bulk_data,
            uses_bulk_elem: chunk_uses_bulk_elem,
            uses_gc: chunk_uses_gc,
            uses_threads: chunk_uses_threads,
            dyn_table_ops: chunk_dyn_tables,
            global_reads: g_reads,
            global_writes: g_writes,
            ci_pairs,
            body_summaries,
        })
    };

    let n_bodies = decoded.code.len();
    let n_chunks = if n_bodies >= 256 && pre_vidx.is_none() {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8)
            .min(n_bodies / 64)
            .max(1)
    } else {
        1
    };
    let chunk_results: Vec<Res<ChunkScan>> = if n_chunks <= 1 {
        vec![scan_chunk(0, &decoded.code)]
    } else {
        // Partition by cumulative instruction weight so giant functions do not create straggler chunks.
        let total_weight: usize = decoded
            .code
            .iter()
            .map(|b| b.instructions.len().saturating_add(16))
            .sum();
        let target_weight = total_weight.div_ceil(n_chunks).max(1);
        let mut slices: Vec<(usize, &[wtime_validate::FunctionBody])> =
            Vec::with_capacity(n_chunks);
        let mut start = 0usize;
        while start < n_bodies {
            let mut end = start;
            if slices.len() + 1 == n_chunks {
                end = n_bodies;
            } else {
                let mut acc = 0usize;
                while end < n_bodies && (end == start || acc < target_weight) {
                    acc += decoded.code[end].instructions.len().saturating_add(16);
                    end += 1;
                }
            }
            slices.push((start, &decoded.code[start..end]));
            start = end;
        }
        let mut slots: Vec<Option<Res<ChunkScan>>> = (0..slices.len()).map(|_| None).collect();
        let mut unspawned: Vec<usize> = Vec::new();
        std::thread::scope(|s| {
            let mut rem = slots.as_mut_slice();
            for (ci, &(base_bi, sl)) in slices.iter().enumerate() {
                let (head, tail) = rem.split_first_mut().unwrap();
                rem = tail;
                if ci + 1 == slices.len() {
                    *head = Some(scan_chunk(base_bi, sl));
                } else if std::thread::Builder::new()
                    .spawn_scoped(s, move || {
                        *head = Some(scan_chunk(base_bi, sl));
                    })
                    .is_err()
                {
                    unspawned.push(ci);
                }
            }
        });
        for ci in unspawned {
            let (base_bi, sl) = slices[ci];
            slots[ci] = Some(scan_chunk(base_bi, sl));
        }
        slots.into_iter().map(|o| o.unwrap()).collect()
    };

    let mut total_br_table_entries = 0usize;
    let mut uses_exceptions = !tags.is_empty();
    let mut uses_bulk_data = false;
    let mut uses_bulk_elem = false;
    let mut dyn_table_touched: HashSet<u32> = HashSet::new();
    let mut global_reads = vec![0usize; n_globals];
    let mut global_writes = vec![0usize; n_globals];
    let mut merged_ci_pairs: Vec<(u32, u32)> = Vec::new();
    let mut merged_ci_seen: HashSet<(u32, u32)> = HashSet::new();
    let mut all_body_summaries = if collect_summaries {
        Vec::with_capacity(n_bodies)
    } else {
        Vec::new()
    };

    for chunk_res in chunk_results {
        let chunk = chunk_res?;
        uses_exceptions |= chunk.uses_exceptions;
        uses_bulk_data |= chunk.uses_bulk_data;
        uses_bulk_elem |= chunk.uses_bulk_elem;
        uses_gc |= chunk.uses_gc;
        uses_threads |= chunk.uses_threads;
        for t in chunk.dyn_table_ops {
            dyn_table_touched.insert(t);
        }
        if collect_summaries {
            all_body_summaries.extend(chunk.body_summaries);
        }
        total_sig_arity = total_sig_arity.saturating_add(chunk.sig_arity);
        if total_sig_arity > max_total_sig_arity {
            return Err(format!(
                "x64: expanded signature arity exceeds cap {max_total_sig_arity}"
            ));
        }
        total_br_table_entries = total_br_table_entries.saturating_add(chunk.br_table_entries);
        if total_br_table_entries > MAX_AGGREGATE_BR_TABLE_ENTRIES {
            return Err(format!(
                "x64: aggregate br_table entries ({total_br_table_entries}) exceed cap {MAX_AGGREGATE_BR_TABLE_ENTRIES}"
            ));
        }
        for (dst, src) in global_reads.iter_mut().zip(&chunk.global_reads) {
            *dst += *src;
        }
        for (dst, src) in global_writes.iter_mut().zip(&chunk.global_writes) {
            *dst += *src;
        }
        for pair in chunk.ci_pairs {
            if merged_ci_seen.insert(pair) {
                merged_ci_pairs.push(pair);
            }
        }
    }

    // Partition tables into static funcref, inline fixed externref, and dynamic mmap-backed tables.
    let mut funcref_tables: BTreeMap<u32, Vec<Option<u32>>> = BTreeMap::new();
    let mut mut_extern_tables: BTreeMap<u32, u32> = BTreeMap::new();
    let mut dyn_tables: BTreeMap<u32, DynTableMeta> = BTreeMap::new();
    let mut inline_extern_slots: usize = 0;
    for (ti, rt) in raw_tables.iter().enumerate() {
        let t = ti as u32;
        if is_store {
            if rt.min_len > MAX_DYN_TABLE_SLOTS {
                return Err(format!(
                    "x64: StoreCdylib table {t} min ({}) exceeds cap {MAX_DYN_TABLE_SLOTS}",
                    rt.min_len
                ));
            }
            dyn_tables.insert(
                t,
                DynTableMeta {
                    min_len: rt.min_len,
                    eff_max_len: rt.eff_max_len.max(rt.min_len),
                    is_funcref: rt.is_funcref,
                    is_table64: rt.is_table64,
                    ref_family: rt.ref_family,
                    init_val: rt.init_val,
                },
            );
        } else if rt.is_funcref {
            if dyn_table_touched.contains(&t) || rt.has_runtime_init {
                if rt.min_len > MAX_DYN_TABLE_SLOTS {
                    return Err(format!(
                        "x64: dynamic funcref table {t} min ({}) exceeds cap {MAX_DYN_TABLE_SLOTS}",
                        rt.min_len
                    ));
                }
                dyn_tables.insert(
                    t,
                    DynTableMeta {
                        min_len: rt.min_len,
                        eff_max_len: rt.eff_max_len.max(rt.min_len),
                        is_funcref: true,
                        is_table64: rt.is_table64,
                        ref_family: rt.ref_family,
                        init_val: rt.init_val,
                    },
                );
            } else {
                let init_opt = if rt.init_val == 0 {
                    None
                } else {
                    Some((rt.init_val - 1) as u32)
                };
                funcref_tables.insert(t, vec![init_opt; rt.min_len as usize]);
            }
        } else if dyn_table_touched.contains(&t)
            || !rt.fixed_inline_extern_eligible
            || table_has_active_elem[ti]
            || rt.has_runtime_init
        {
            dyn_tables.insert(
                t,
                DynTableMeta {
                    min_len: rt.min_len,
                    eff_max_len: rt.eff_max_len.max(rt.min_len),
                    is_funcref: false,
                    is_table64: rt.is_table64,
                    ref_family: rt.ref_family,
                    init_val: rt.init_val,
                },
            );
        } else {
            inline_extern_slots = inline_extern_slots.saturating_add(rt.min_len as usize);
            mut_extern_tables.insert(t, rt.min_len);
        }
    }

    let worst_ctx_bytes = super::CTX_HEADER_BYTES
        .saturating_add(decoded.globals.len().saturating_mul(16))
        .saturating_add(inline_extern_slots.saturating_mul(8))
        .saturating_add(dyn_tables.len().saturating_mul(24))
        .saturating_add(extra_memories.len().saturating_mul(32))
        .saturating_add(if uses_bulk_data { data_seg_count } else { 0 })
        .saturating_add(if uses_bulk_elem { elem_segs.len() } else { 0 })
        .saturating_add(if uses_gc { 128 } else { 0 })
        .saturating_add(64);
    if worst_ctx_bytes > MAX_CTX_BYTES {
        return Err(format!(
            "x64: Ctx storage ({worst_ctx_bytes} B) exceeds cap {MAX_CTX_BYTES} B"
        ));
    }

    // Apply active element segments targeting static funcref tables at compile time
    // (dynamic table active element segments are applied at runtime in `wt_initialize`).
    let mut active_elem_checks = Vec::new();
    for seg in &elem_segs {
        if let Some((tbl, base_u64)) = seg.active_target
            && let Some(slots) = funcref_tables.get_mut(&tbl)
        {
            let elem_len =
                u32::try_from(seg.entries.len()).map_err(|_| "active element segment too large")?;
            active_elem_checks.push((slots.len() as u32, base_u64, elem_len));
            if let Ok(base) = usize::try_from(base_u64)
                && let Some(end) = base.checked_add(seg.entries.len())
                && end <= slots.len()
            {
                for (dst, &enc) in slots[base..end].iter_mut().zip(&seg.entries) {
                    *dst = if enc == 0 { None } else { Some(enc - 1) };
                }
            }
        }
    }

    // Pre-index all static funcref table entries ONCE in O(total_slots) by `(table, canon_sig_id)`
    // (also propagating to every ancestor supertype when function subtyping is present!).
    let mut funcref_table_lens: BTreeMap<u32, u32> = BTreeMap::new();
    let mut funcref_by_sig: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    let mut slot_steps: usize = 0;
    for (tbl, slots) in funcref_tables {
        funcref_table_lens.insert(tbl, slots.len() as u32);
        for (slot, entry) in slots.into_iter().enumerate() {
            slot_steps += 1;
            if let Some(b) = budget
                && (slot_steps & 4095) == 0
            {
                b.check_deadline()?;
            }
            if let Some(f) = entry
                && let Some(&csig) = func_canon_sig.get(f as usize)
            {
                if has_func_subtyping {
                    for &anc_sig in &gc_canon_types[csig as usize].supertypes_all {
                        funcref_by_sig
                            .entry((tbl, anc_sig))
                            .or_default()
                            .push((slot as u32, f));
                    }
                } else {
                    funcref_by_sig
                        .entry((tbl, csig))
                        .or_default()
                        .push((slot as u32, f));
                }
            }
        }
    }

    let mut ci_canon: HashMap<(u32, u32), usize> = HashMap::new();
    let mut ci_index: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut ci_sigs: Vec<X64CiSig> = Vec::new();
    let mut uses_dyn_ci = false;
    let mut uses_call_ref = false;
    let mut total_ci_bytes = 0usize;
    for (tbl, ti) in merged_ci_pairs {
        if ci_index.contains_key(&(tbl, ti)) {
            continue;
        }
        let &csig = type_canon
            .get(ti as usize)
            .ok_or("call_indirect type index")?;
        let ci_id = if let Some(&existing) = ci_canon.get(&(tbl, csig)) {
            existing
        } else if tbl == u32::MAX {
            uses_call_ref = true;
            let id = ci_sigs.len();
            ci_sigs.push(X64CiSig {
                table: u32::MAX,
                table_len: 0,
                type_idx: ti,
                canon_sig: csig,
                is_dynamic: false,
                is_call_ref: true,
                table_is64: false,
                min_slot: 0,
                span: 0,
                targets: Vec::new(),
            });
            ci_canon.insert((u32::MAX, csig), id);
            id
        } else if dyn_tables.get(&tbl).is_some_and(|m| m.is_funcref) {
            uses_dyn_ci = true;
            let t64 = table_is64.get(tbl as usize).copied().unwrap_or(false);
            let id = ci_sigs.len();
            ci_sigs.push(X64CiSig {
                table: tbl,
                table_len: 0,
                type_idx: ti,
                canon_sig: csig,
                is_dynamic: true,
                is_call_ref: false,
                table_is64: t64,
                min_slot: 0,
                span: 0,
                targets: Vec::new(),
            });
            ci_canon.insert((tbl, csig), id);
            id
        } else {
            let &table_len = funcref_table_lens
                .get(&tbl)
                .ok_or_else(|| format!("x64: call_indirect on unsupported table {tbl}"))?;
            let t64 = table_is64.get(tbl as usize).copied().unwrap_or(false);
            let targets = funcref_by_sig.remove(&(tbl, csig)).unwrap_or_default();
            let (min_slot, span) =
                if let (Some(&(first, _)), Some(&(last, _))) = (targets.first(), targets.last()) {
                    (first, last - first + 1)
                } else {
                    (0, 0)
                };
            let bytes = (span as usize).saturating_mul(4);
            total_ci_bytes = total_ci_bytes.saturating_add(bytes);
            if total_ci_bytes > MAX_AGGREGATE_CI_TABLE_BYTES {
                return Err(format!(
                    "x64: aggregate call_indirect dispatch tables ({total_ci_bytes} B) exceed cap {MAX_AGGREGATE_CI_TABLE_BYTES} B"
                ));
            }
            let id = ci_sigs.len();
            ci_sigs.push(X64CiSig {
                table: tbl,
                table_len,
                type_idx: ti,
                canon_sig: csig,
                is_dynamic: false,
                is_call_ref: false,
                table_is64: t64,
                min_slot,
                span,
                targets,
            });
            ci_canon.insert((tbl, csig), id);
            id
        };
        ci_index.insert((tbl, ti), ci_id);
    }

    // Fold globals that are immutable OR never written by Wasm AND not exported-mutable,
    // EXCEPT runtime-initialized GC globals (which live in traced `Ctx` slots) and imported/exported
    // `StoreCdylib` globals (which require addressable live `Ctx` storage).
    let mut global_consts = Vec::with_capacity(globals.len());
    for (k, &(_, mutable)) in globals.iter().enumerate() {
        let can_fold = !(is_store && (k < imported_globals.len() || exported_any_globals[k]))
            && global_inits[k] != X64ConstVal::RuntimeGc
            && (!mutable || (global_writes[k] == 0 && !exported_mut_globals[k]));
        global_consts.push(if can_fold {
            Some(global_inits[k])
        } else {
            None
        });
    }

    // Pin the hottest mutable scalar I32/I64 global in `R12` ONLY when its static access density
    // justifies giving up a callee-saved GP register across every function (disabled in `StoreCdylib`
    // so cross-module calls and host global reads never see stale register-cached globals).
    let min_pin_accesses = decoded.code.len().saturating_mul(3).max(16);
    let pinned_global = if is_store {
        None
    } else {
        (0..globals.len())
            .filter(|&k| {
                global_consts[k].is_none()
                    && globals[k].1
                    && matches!(globals[k].0, ValType::I32 | ValType::I64)
                    && (global_reads[k] + global_writes[k]) >= min_pin_accesses
            })
            .max_by_key(|&k| (global_reads[k] + global_writes[k], std::cmp::Reverse(k)))
            .map(|k| k as u32)
    };

    let pinned_global_2 = if !is_store && fixed32.is_some() {
        (0..globals.len())
            .filter(|&k| {
                pinned_global != Some(k as u32)
                    && global_consts[k].is_none()
                    && globals[k].1
                    && matches!(globals[k].0, ValType::I32 | ValType::I64)
                    && (global_reads[k] + global_writes[k]) >= min_pin_accesses
            })
            .max_by_key(|&k| (global_reads[k] + global_writes[k], std::cmp::Reverse(k)))
            .map(|k| k as u32)
    } else {
        None
    };

    let start = if let Some(s) = decoded.start {
        let fty = func_type_idx
            .get(s.0 as usize)
            .and_then(|&ti| decoded.types.get(ti as usize))
            .ok_or_else(|| format!("x64: start function index {} out of range", s.0))?;
        if !fty.params.is_empty() || !fty.results.is_empty() {
            return Err(format!(
                "x64: start function {} must have signature () -> ()",
                s.0
            ));
        }
        Some(s.0)
    } else {
        None
    };

    let standalone_start = if target == X64Target::StandaloneExec {
        if let Some((_, fi)) = exported_funcs.iter().find(|(name, _)| name == "_start") {
            let ti = func_type_idx[*fi] as usize;
            let fty = &decoded.types[ti];
            if !fty.params.is_empty() {
                return Err(format!(
                    "x64: standalone _start export must take 0 parameters (got {:?})",
                    fty.params
                ));
            }
            let returns_i32 = match fty.results.as_slice() {
                [] => false,
                [ValType::I32] => true,
                other => {
                    return Err(format!(
                        "x64: standalone _start export must return () or (i32) (got {other:?})"
                    ));
                }
            };
            Some((*fi as u32, returns_i32))
        } else if start.is_some() {
            None
        } else {
            return Err("x64: standalone executable requires an exported `_start` function or start section".into());
        }
    } else {
        None
    };

    let standalone_goose_reserve = if target == X64Target::StandaloneExec {
        if let Some((_, fi)) = exported_funcs
            .iter()
            .find(|(name, _)| name == "goose.reserve")
        {
            let ti = func_type_idx[*fi] as usize;
            let fty = &decoded.types[ti];
            match (fty.params.as_slice(), fty.results.as_slice()) {
                ([ValType::I64, ValType::I64], [ValType::I64]) => Some((*fi as u32, true)),
                ([ValType::I32, ValType::I32], [ValType::I32]) => Some((*fi as u32, false)),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok((
        X64Module {
            imports,
            func_type_idx,
            func_canon_sig,
            module_types: decoded.types.clone(),
            globals,
            global_inits,
            global_consts,
            pinned_global,
            pinned_global_2,
            exported_globals,
            exported_funcs,
            ci_index,
            ci_sigs,
            uses_dyn_ci,
            uses_call_ref,
            table_lens,
            table_is64,
            mut_extern_tables,
            dyn_tables,
            memory,
            memory64,
            memory_shared,
            fixed32,
            extra_memories,
            tags,
            uses_exceptions,
            target,
            standalone_start,
            standalone_goose_reserve,
            active_elem_checks,
            active_data_segs,
            data_seg_count,
            passive_data_segs,
            uses_bulk_data,
            elem_segs,
            uses_bulk_elem,
            uses_gc,
            uses_threads,
            type_canon,
            gc_canon_types: if uses_gc || is_store {
                gc_canon_types
            } else {
                Vec::new()
            },
            type_kinds,
            runtime_global_inits,
            runtime_table_inits,
            imported_globals,
            imported_mems,
            imported_tables,
            imported_tags,
            import_order,
            all_mem_declared_max: if is_store {
                all_mem_declared_max
            } else {
                Vec::new()
            },
            all_table_types: if is_store {
                all_table_types.into_iter().map(|(_, tt)| tt).collect()
            } else {
                Vec::new()
            },
            exported_mems,
            exported_tables,
            exported_tags,
            raw_type_groups: if is_store {
                type_groups.to_vec()
            } else {
                Vec::new()
            },
            start,
        },
        all_body_summaries,
    ))
}
