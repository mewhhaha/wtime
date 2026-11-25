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
    CompositeType, FeatureSet, FuncType, HeapType, IndexType, ResourceLimits, ValType,
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
}

/// Exact Wasm feature profile enforced right at the front door of `wtime_validate::validate`,
/// disabling unsupported proposals (GC, threads, function-references, custom page sizes/descriptors,
/// stack switching) before decoding/validation.
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
        ..FeatureSet::mvp()
    }
}

/// Resource limits passed to `wtime_validate::validate` (matching `ResourceLimits::default()`
/// except allowing `memory64` declared page limits up to `2^48`, enforced strictly at admission).
pub fn x64_validate_limits() -> ResourceLimits {
    ResourceLimits {
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
pub const MAX_GLOBALS: usize = 65_536;
pub const MAX_EXPORTS: usize = 65_536;
pub const MAX_MODULE_TYPES: usize = 100_000;
pub const MAX_CTX_BYTES: usize = 16 << 20; // 16 MiB (far below i32::MAX)
pub const MAX_AGGREGATE_CI_TABLE_BYTES: usize = 64 << 20; // 64 MiB
pub const MAX_AGGREGATE_BR_TABLE_ENTRIES: usize = 4_194_304; // 16 MiB jump tables
pub const MAX_LOCALS_PER_FN: usize = 65_536;
pub const MAX_TOTAL_LOCALS: usize = 1_000_000;
pub const MAX_TOTAL_SIG_ARITY: usize = 1_000_000; // params + results across decls, blocks, and call_indirect sites
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

/// Constant value folded from an immutable/never-written non-exported global initializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X64ConstVal {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    V128([u8; 16]),
    RefNull,
}

/// Static `call_indirect` dispatch metadata for one canonical `(table, canonical_sig_id)` pair.
#[derive(Clone, Debug)]
pub struct X64CiSig {
    #[allow(dead_code)]
    pub table: u32,
    pub table_len: u32,
    /// Representative index into `X64Module::module_types` (zero `FuncType` duplication).
    pub type_idx: u32,
    pub min_slot: u32,
    pub span: u32,
    /// Sorted `(table_slot, func_index)` entries whose function signature equals `module_types[type_idx]`.
    pub targets: Vec<(u32, u32)>,
}

/// Metadata for an additional linear memory (`memory_index >= 1`, Wasm `multi-memory`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtraMemMeta {
    pub min_pages: u64,
    pub max_pages: u64,
    pub memory64: bool,
}

/// Lean module metadata consumed by `x64/{ir,lower,mod}.rs`.
pub struct X64Module {
    /// Imported functions: `(module_name, import_name)`.
    pub imports: Vec<(String, String)>,
    /// Index into `module_types` for every function (`0..imports.len() + code.len()`) — zero per-function signature cloning.
    pub func_type_idx: Vec<u32>,
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
    /// Exported globals: `(name, global_index, val_type, mutable)`.
    pub exported_globals: Vec<(String, usize, ValType, bool)>,
    /// Exported functions: `(name, func_index)`.
    pub exported_funcs: Vec<(String, usize)>,
    /// Maps `(table_idx, raw_type_idx)` -> index into `ci_sigs` (`O(log N)` deterministic `BTreeMap` lookup).
    pub ci_index: BTreeMap<(u32, u32), usize>,
    /// Deduplicated `call_indirect` dispatch descriptors.
    pub ci_sigs: Vec<X64CiSig>,
    /// Mutable `externref` tables: `table_idx -> exact_len`.
    pub mut_extern_tables: BTreeMap<u32, u32>,
    /// Memory 0 declared limits: `Some((min_pages, declared_max_pages))` or `None` if 0 memories.
    pub memory: Option<(u64, u64)>,
    /// `true` iff memory 0 uses 64-bit (`i64`) addressing (`memory64`).
    pub memory64: bool,
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
    /// Active element segment bounds checks `(table_len, base_off, elem_len)` for `wt_initialize`.
    pub active_elem_checks: Vec<(u32, u32, u32)>,
    /// Active data segments `(mem_idx, mem_off, decoded.data index)` for `wt_initialize` (in declaration order).
    pub active_data_segs: Vec<(u32, u64, usize)>,
    /// Optional start function index.
    pub start: Option<u32>,
}

impl X64Module {
    /// Borrows the canonical `FuncType` for function index `func_index` without cloning.
    #[inline]
    pub fn func_type(&self, func_index: usize) -> Option<&FuncType> {
        let &ti = self.func_type_idx.get(func_index)?;
        self.module_types.get(ti as usize)
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
            let ty = super::ir::Ty::from_val(vt);
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

fn eval_const_expr(expr: &[Instruction], prior: &[X64ConstVal]) -> Res<X64ConstVal> {
    let mut stk: Vec<X64ConstVal> = Vec::new();
    for i in expr {
        let v = match (i.opcode, &i.immediate) {
            (0x0b, _) => break,
            (0x41, Immediate::I32(k)) => X64ConstVal::I32(*k),
            (0x42, Immediate::I64(k)) => X64ConstVal::I64(*k),
            (0x43, Immediate::F32(b)) => X64ConstVal::F32(*b),
            (0x44, Immediate::F64(b)) => X64ConstVal::F64(*b),
            (0xfd0c, Immediate::Bytes(bytes)) => {
                let arr: [u8; 16] = bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| "invalid v128.const")?;
                X64ConstVal::V128(arr)
            }
            (0xd0, _) => X64ConstVal::RefNull,
            (0x23, Immediate::Index(g)) => *prior
                .get(*g as usize)
                .ok_or("const global.get out of range")?,
            (0x6a..=0x6c, _) => match (stk.pop(), stk.pop()) {
                (Some(X64ConstVal::I32(b)), Some(X64ConstVal::I32(a))) => {
                    X64ConstVal::I32(match i.opcode {
                        0x6a => a.wrapping_add(b),
                        0x6b => a.wrapping_sub(b),
                        _ => a.wrapping_mul(b),
                    })
                }
                _ => return Err("invalid i32 const expr".into()),
            },
            (0x7c..=0x7e, _) => match (stk.pop(), stk.pop()) {
                (Some(X64ConstVal::I64(b)), Some(X64ConstVal::I64(a))) => {
                    X64ConstVal::I64(match i.opcode {
                        0x7c => a.wrapping_add(b),
                        0x7d => a.wrapping_sub(b),
                        _ => a.wrapping_mul(b),
                    })
                }
                _ => return Err("invalid i64 const expr".into()),
            },
            (op, _) => return Err(format!("x64: unsupported const expression opcode {op:#x}")),
        };
        stk.push(v);
    }
    match stk.as_slice() {
        [v] => Ok(*v),
        _ => Err("const expression did not yield one value".into()),
    }
}

fn check_val_type(vt: ValType) -> Res<()> {
    match vt {
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64 | ValType::V128 => Ok(()),
        ValType::Ref(rt) if rt.nullable && matches!(rt.heap, HeapType::Extern) => Ok(()),
        ValType::Ref(rt) if matches!(rt.heap, HeapType::Exn) => Ok(()),
        other => Err(format!("x64: unsupported value type {other:?}")),
    }
}

fn check_block_imm(
    imm: &Immediate,
    type_arity: &[usize],
    type_admitted: &[bool],
    total_sig_arity: &mut usize,
) -> Res<()> {
    match imm {
        Immediate::Block(-64 | -1 | -2 | -3 | -4 | -5 | -17 | -23) => Ok(()),
        Immediate::Block(i) if *i >= 0 => {
            let idx = *i as usize;
            if !*type_admitted
                .get(idx)
                .ok_or("x64: block type index out of range")?
            {
                return Err(format!(
                    "x64: block type index {idx} uses unsupported value type"
                ));
            }
            *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
            if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                return Err(format!(
                    "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
                ));
            }
            Ok(())
        }
        Immediate::BlockType(vt) => check_val_type(*vt),
        other => Err(format!("x64: unsupported block type {other:?}")),
    }
}

fn check_supported_opcode(
    op: u16,
    imm: &Immediate,
    func_type_idx: &[u32],
    tag_type_idx: &[u32],
    type_arity: &[usize],
    type_admitted: &[bool],
    total_sig_arity: &mut usize,
) -> Res<()> {
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
        | 0xd1 => Ok(()),
        0x08 => match imm {
            Immediate::Index(ti) => {
                let &fty = tag_type_idx
                    .get(*ti as usize)
                    .ok_or_else(|| format!("x64: throw tag index {ti} out of range"))?;
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[fty as usize]);
                if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed throw immediate".into()),
        },
        0x1f => match imm {
            Immediate::TryTable(tt) => {
                check_block_imm(&tt.block, type_arity, type_admitted, total_sig_arity)?;
                for c in &tt.catches {
                    if let Some(tag) = c.tag {
                        let &fty = tag_type_idx
                            .get(tag as usize)
                            .ok_or_else(|| format!("x64: catch tag index {tag} out of range"))?;
                        *total_sig_arity = total_sig_arity.saturating_add(type_arity[fty as usize]);
                        if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                            return Err(format!(
                                "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
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
                if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed call immediate".into()),
        },
        0x02..=0x04 => check_block_imm(imm, type_arity, type_admitted, total_sig_arity),
        0x11 | 0x13 => match imm {
            Immediate::Pair(ti, _) => {
                let idx = *ti as usize;
                if !*type_admitted
                    .get(idx)
                    .ok_or("x64: call_indirect type index out of range")?
                {
                    return Err(format!(
                        "x64: call_indirect type index {idx} uses unsupported value type"
                    ));
                }
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
                if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                    return Err(format!(
                        "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
                    ));
                }
                Ok(())
            }
            _ => Err("x64: malformed call_indirect immediate".into()),
        },
        0x1c => match imm {
            Immediate::Types(tys) => {
                for &vt in tys.iter() {
                    check_val_type(vt)?;
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
            Immediate::Heap(HeapType::Extern | HeapType::Exn) => Ok(()),
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
        0xfc0a => match imm {
            Immediate::Pair(_, _) | Immediate::None => Ok(()),
            _ => Err("x64: malformed memory.copy immediate".into()),
        },
        0xfc0b => match imm {
            Immediate::Index(_) | Immediate::None => Ok(()),
            _ => Err("x64: malformed memory.fill immediate".into()),
        },
        0xfd0c..=0xfd53 | 0xfd5e..=0xfdff | 0xfa00..=0xfa13 => Ok(()),
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
            "gs_os_clock_ns" | "bench_clock" | "bench_input" | "gs_os_time_ns" | "gs_os_arg_count"
            | "gs_os_resource_dir",
        ) => ty.params.is_empty() && ty.results.len() == 1 && is_int(ty.results[0]),
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
    pub sig_arity: usize,
    pub br_table_entries: usize,
    pub g_reads: Vec<(u32, u32)>,
    pub g_writes: Vec<(u32, u32)>,
    pub ci_pairs: Vec<(u32, u32)>,
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
    for group in &decoded.type_groups {
        if group.explicit {
            return Err("x64: GC recursive type groups are not supported".into());
        }
        for st in &group.types {
            if !matches!(st.composite, CompositeType::Func(_))
                || !st.final_
                || st.supertype.is_some()
                || st.descriptor.is_some()
                || st.describes.is_some()
            {
                return Err(
                    "x64: GC/subtyping/non-final/descriptor types are not supported".into(),
                );
            }
        }
    }
    if decoded.memories.len() > 1 + MAX_EXTRA_MEMORIES {
        return Err(format!(
            "x64: memory count ({}) exceeds cap {}",
            decoded.memories.len(),
            1 + MAX_EXTRA_MEMORIES
        ));
    }
    let check_mem = |mi: usize, mt: &wtime_types::MemoryType| -> Res<(u64, u64, bool)> {
        let is_mem64 = mt.index_type == IndexType::I64;
        if mt.shared {
            return Err(format!("x64: shared memory {mi} is not supported"));
        }
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
        Ok((mt.limits.min, max_p, is_mem64))
    };
    let (memory, memory64, fixed32) = if let Some(mt) = decoded.memories.first() {
        let (min_p, max_p, is_mem64) = check_mem(0, mt)?;
        let fixed = if min_p == max_p {
            min_p.checked_mul(65_536)
        } else {
            None
        };
        (Some((min_p, max_p)), is_mem64, fixed)
    } else {
        (None, false, None)
    };
    let mut extra_memories = Vec::with_capacity(decoded.memories.len().saturating_sub(1));
    for (mi, mt) in decoded.memories.iter().enumerate().skip(1) {
        let (min_pages, max_pages, m64) = check_mem(mi, mt)?;
        extra_memories.push(ExtraMemMeta {
            min_pages,
            max_pages,
            memory64: m64,
        });
    }

    // Pre-validate and intern module types ONCE into canonical `u32` IDs + cached arity/admissibility
    // so every function declaration, block type, table slot, and `call_indirect` site runs in O(1)
    // with zero per-function or per-slot `FuncType` cloning/re-scanning. Also bound total
    // declared type arity upfront before cloning `decoded.types` into `X64Module`.
    if decoded.types.len() > MAX_MODULE_TYPES {
        return Err(format!(
            "x64: module declares {} types (max {MAX_MODULE_TYPES})",
            decoded.types.len()
        ));
    }
    let mut sig_to_canon: HashMap<&FuncType, u32> = HashMap::with_capacity(decoded.types.len());
    let mut type_canon: Vec<u32> = Vec::with_capacity(decoded.types.len());
    let mut type_arity: Vec<usize> = Vec::with_capacity(decoded.types.len());
    let mut type_admitted: Vec<bool> = Vec::with_capacity(decoded.types.len());
    let mut declared_type_arity: usize = 0;
    for (ti, fty) in decoded.types.iter().enumerate() {
        if let Some(b) = budget
            && (ti & 4095) == 4095
        {
            b.check_deadline()?;
        }
        let arity = fty.params.len().saturating_add(fty.results.len());
        declared_type_arity = declared_type_arity.saturating_add(arity);
        if declared_type_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!(
                "x64: total type-section signature arity exceeds cap ({MAX_TOTAL_SIG_ARITY})"
            ));
        }
        let next_id = sig_to_canon.len() as u32;
        let cid = *sig_to_canon.entry(fty).or_insert(next_id);
        type_canon.push(cid);
        type_arity.push(arity);
        type_admitted.push(
            fty.params
                .iter()
                .chain(&fty.results)
                .all(|&vt| check_val_type(vt).is_ok()),
        );
    }

    let mut tags: Vec<u32> = Vec::with_capacity(decoded.tags.len());
    for (ti, tag) in decoded.tags.iter().enumerate() {
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
        if precheck_total_locals > MAX_TOTAL_LOCALS {
            return Err(format!(
                "x64: module total locals exceeds cap {MAX_TOTAL_LOCALS}"
            ));
        }
        total_sig_arity = total_sig_arity.saturating_add(type_arity[ti]);
        if total_sig_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!(
                "x64: module expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
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
        if total_sig_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!(
                "x64: module expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
            ));
        }
        match target {
            X64Target::BlotCdylib => check_blot_import_sig(&imp.module, &imp.name, fty)?,
            X64Target::StandaloneExec => {
                check_standalone_import_sig(&imp.module, &imp.name, fty, memory64)?;
            }
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

    // Tables: static funcref tables + mutable externref tables (32-bit only, aggregate-capped).
    let mut funcref_tables: BTreeMap<u32, Vec<Option<u32>>> = BTreeMap::new();
    let mut mut_extern_tables: BTreeMap<u32, u32> = BTreeMap::new();
    let mut total_funcref_slots: usize = 0;
    let mut total_extern_slots: usize = 0;
    for (ti, tt) in decoded.tables.iter().enumerate() {
        let ti_u32 = u32::try_from(ti).map_err(|_| "too many tables")?;
        if tt.index_type != IndexType::I32 {
            return Err(format!("x64: table64 (table {ti}) is not supported"));
        }
        if decoded
            .table_inits
            .get(ti)
            .and_then(|o| o.as_ref())
            .is_some()
        {
            return Err(format!(
                "x64: explicit table initializer on table {ti} (function-references) is not supported in Stage 1"
            ));
        }
        match tt.element.heap {
            HeapType::Func if tt.element.nullable => {
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
                funcref_tables.insert(ti_u32, vec![None; len as usize]);
            }
            HeapType::Extern if tt.element.nullable => {
                let max = tt.limits.max.unwrap_or(u64::MAX);
                if tt.limits.min != max || tt.limits.min > u64::from(MAX_EXTERN_TABLE_SLOTS) {
                    return Err(format!(
                        "x64: externref table {ti} must be fixed-size <= {MAX_EXTERN_TABLE_SLOTS} (min={}, max={:?})",
                        tt.limits.min, tt.limits.max
                    ));
                }
                total_extern_slots = total_extern_slots.saturating_add(tt.limits.min as usize);
                if total_extern_slots > MAX_TOTAL_EXTERN_SLOTS {
                    return Err(format!(
                        "x64: aggregate externref table slots ({total_extern_slots}) exceed cap {MAX_TOTAL_EXTERN_SLOTS}"
                    ));
                }
                mut_extern_tables.insert(ti_u32, tt.limits.min as u32);
            }
            other => {
                return Err(format!(
                    "x64: unsupported table element type {other:?} on table {ti}"
                ));
            }
        }
    }

    if decoded.globals.len() > MAX_GLOBALS {
        return Err(format!(
            "x64: module global count ({}) exceeds cap {MAX_GLOBALS}",
            decoded.globals.len()
        ));
    }
    let worst_ctx_bytes = super::CTX_HEADER_BYTES
        .saturating_add(decoded.globals.len().saturating_mul(16))
        .saturating_add(total_extern_slots.saturating_mul(8))
        .saturating_add(extra_memories.len().saturating_mul(32))
        .saturating_add(32);
    if worst_ctx_bytes > MAX_CTX_BYTES {
        return Err(format!(
            "x64: Ctx storage ({worst_ctx_bytes} B) exceeds cap {MAX_CTX_BYTES} B"
        ));
    }

    // Globals: evaluate all initializers; track which globals are written or exported-mutable.
    let mut globals = Vec::with_capacity(decoded.globals.len());
    let mut global_inits = Vec::with_capacity(decoded.globals.len());
    for (gi, g) in decoded.globals.iter().enumerate() {
        if let Some(b) = budget
            && (gi & 4095) == 4095
        {
            b.check_deadline()?;
        }
        check_val_type(g.ty.value)?;
        let init = eval_const_expr(&g.init, &global_inits)?;
        globals.push((g.ty.value, g.ty.mutable));
        global_inits.push(init);
    }

    // Exports: admit functions, globals, and memory 0 (`blot:memory` etc.); reject table/tag exports.
    if decoded.exports.len() > MAX_EXPORTS {
        return Err(format!(
            "x64: module export count ({}) exceeds cap {MAX_EXPORTS}",
            decoded.exports.len()
        ));
    }
    let mut exported_funcs = Vec::new();
    let mut exported_globals = Vec::new();
    let mut exported_mut_globals = vec![false; globals.len()];
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
                if mutable {
                    exported_mut_globals[gi] = true;
                }
                exported_globals.push((exp.name.clone(), gi, vt, mutable));
            }
            ExternalKind::Memory => {
                if (exp.index as usize) >= decoded.memories.len() {
                    return Err(format!(
                        "x64: unsupported memory export {:?} (index {})",
                        exp.name, exp.index
                    ));
                }
            }
            ExternalKind::Table | ExternalKind::Tag => {
                return Err(format!(
                    "x64: unsupported export {:?} ({:?})",
                    exp.name, exp.kind
                ));
            }
        }
    }

    // Active element segments: validate forms and populate static funcref tables (recording bounds
    // checks so `wt_initialize` reports `TRAP_TABLE_OOB` at instantiation time per Wasm spec).
    let mut active_elem_checks = Vec::new();
    let mut elem_steps: usize = 0;
    for elem in &decoded.elements {
        match &elem.mode {
            SegmentMode::Active { index, offset } => {
                let off_val = eval_const_expr(offset, &global_inits)?;
                let X64ConstVal::I32(off_i32) = off_val else {
                    return Err("x64: active element offset must be i32".into());
                };
                let Some(slots) = funcref_tables.get_mut(index) else {
                    return Err(format!(
                        "x64: active element segment targets non-funcref table {index}"
                    ));
                };
                let mut entries = Vec::with_capacity(elem.init.len());
                for expr in &elem.init {
                    elem_steps += 1;
                    if let Some(b) = budget
                        && (elem_steps & 4095) == 0
                    {
                        b.check_deadline()?;
                    }
                    match expr.as_slice() {
                        [
                            Instruction {
                                opcode: 0xd2,
                                immediate: Immediate::Index(f),
                            },
                            Instruction { opcode: 0x0b, .. },
                        ] => {
                            entries.push(Some(*f));
                        }
                        [
                            Instruction { opcode: 0xd0, .. },
                            Instruction { opcode: 0x0b, .. },
                        ] => {
                            entries.push(None);
                        }
                        _ => return Err("x64: unsupported active element init expression".into()),
                    }
                }
                let base_u32 = off_i32 as u32;
                let elem_len =
                    u32::try_from(entries.len()).map_err(|_| "active element segment too large")?;
                active_elem_checks.push((slots.len() as u32, base_u32, elem_len));
                let base = base_u32 as usize;
                if let Some(end) = base.checked_add(entries.len())
                    && end <= slots.len()
                {
                    slots[base..end].copy_from_slice(&entries);
                }
            }
            SegmentMode::Passive | SegmentMode::Declarative => {
                return Err("x64: passive/declarative element segments are not supported".into());
            }
        }
    }

    // Active data segments only (passive segments rejected).
    let mut active_data_segs = Vec::new();
    for (di, seg) in decoded.data.iter().enumerate() {
        if let Some(b) = budget
            && (di & 4095) == 4095
        {
            b.check_deadline()?;
        }
        match &seg.mode {
            SegmentMode::Active { index, offset } => {
                let m_idx = *index as usize;
                if m_idx >= decoded.memories.len() {
                    return Err(format!(
                        "x64: active data segment on out-of-range memory {index}"
                    ));
                }
                let is_m64 = if m_idx == 0 {
                    memory64
                } else {
                    extra_memories[m_idx - 1].memory64
                };
                let off_u64 = if is_m64 {
                    let X64ConstVal::I64(off_i64) = eval_const_expr(offset, &global_inits)? else {
                        return Err("x64: memory64 active data offset must be i64".into());
                    };
                    off_i64 as u64
                } else {
                    let X64ConstVal::I32(off_i32) = eval_const_expr(offset, &global_inits)? else {
                        return Err("x64: active data offset must be i32".into());
                    };
                    u64::from(off_i32 as u32)
                };
                active_data_segs.push((*index, off_u64, di));
            }
            SegmentMode::Passive | SegmentMode::Declarative => {
                return Err("x64: passive data segments are not supported".into());
            }
        }
    }

    // Pre-index all static funcref table entries ONCE in O(total_slots) by `(table, canon_sig_id)`
    // (zero `FuncType` cloning/hashing per slot); `ci_index` uses deterministic `O(log N)` `BTreeMap` lookups.
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
                funcref_by_sig
                    .entry((tbl, csig))
                    .or_default()
                    .push((slot as u32, f));
            }
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
        global_reads: Vec<usize>,
        global_writes: Vec<usize>,
        ci_pairs: Vec<(u32, u32)>,
        body_summaries: Vec<BodyAdmitSummary>,
    }

    let n_globals = globals.len();
    let scan_chunk = |base_bi: usize, bodies: &[wtime_validate::FunctionBody]| -> Res<ChunkScan> {
        let mut sig_arity = 0usize;
        let mut br_table_entries = 0usize;
        let mut chunk_uses_exceptions = false;
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
            let mut b_imm_elems = 0usize;
            let mut b_saw_data_count_op = false;
            let mut b_uses_exceptions = false;
            for &vt in &body.locals {
                check_val_type(vt)?;
                if matches!(vt, ValType::Ref(rt) if rt.heap == HeapType::Exn) {
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
                    &mut sig_arity,
                )?;
                if matches!(insn.opcode, 0x08 | 0x0a | 0x1f)
                    || matches!(insn.immediate, Immediate::Heap(HeapType::Exn))
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
                        if let Immediate::Index(t) = insn.immediate
                            && !mut_extern_tables.contains_key(&t)
                        {
                            return Err(format!("x64: table.get/set on unsupported table {t}"));
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
                    _ => {}
                }
            }
            chunk_uses_exceptions |= b_uses_exceptions;
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
            .min(16)
            .min(n_bodies / 64)
            .max(1)
    } else {
        1
    };
    let chunk_results: Vec<Res<ChunkScan>> = if n_chunks <= 1 {
        vec![scan_chunk(0, &decoded.code)]
    } else {
        let per = n_bodies.div_ceil(n_chunks);
        let slices: Vec<(usize, &[wtime_validate::FunctionBody])> = decoded
            .code
            .chunks(per)
            .enumerate()
            .map(|(i, sl)| (i * per, sl))
            .collect();
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
    let mut global_reads = vec![0usize; n_globals];
    let mut global_writes = vec![0usize; n_globals];
    let mut ci_canon: HashMap<(u32, u32), usize> = HashMap::new();
    let mut ci_index: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut ci_sigs: Vec<X64CiSig> = Vec::new();
    let mut total_ci_bytes = 0usize;
    let mut all_body_summaries = if collect_summaries {
        Vec::with_capacity(n_bodies)
    } else {
        Vec::new()
    };

    for chunk_res in chunk_results {
        let chunk = chunk_res?;
        uses_exceptions |= chunk.uses_exceptions;
        if collect_summaries {
            all_body_summaries.extend(chunk.body_summaries);
        }
        total_sig_arity = total_sig_arity.saturating_add(chunk.sig_arity);
        if total_sig_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!(
                "x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"
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
        for (tbl, ti) in chunk.ci_pairs {
            if ci_index.contains_key(&(tbl, ti)) {
                continue;
            }
            let &csig = type_canon
                .get(ti as usize)
                .ok_or("call_indirect type index")?;
            let ci_id = if let Some(&existing) = ci_canon.get(&(tbl, csig)) {
                existing
            } else {
                let &table_len = funcref_table_lens
                    .get(&tbl)
                    .ok_or_else(|| format!("x64: call_indirect on unsupported table {tbl}"))?;
                let targets = funcref_by_sig.remove(&(tbl, csig)).unwrap_or_default();
                let (min_slot, span) = if let (Some(&(first, _)), Some(&(last, _))) =
                    (targets.first(), targets.last())
                {
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
                    min_slot,
                    span,
                    targets,
                });
                ci_canon.insert((tbl, csig), id);
                id
            };
            ci_index.insert((tbl, ti), ci_id);
        }
    }

    // Fold globals that are immutable OR never written by Wasm AND not exported-mutable.
    let mut global_consts = Vec::with_capacity(globals.len());
    for (k, &(_, mutable)) in globals.iter().enumerate() {
        let can_fold = !mutable || (global_writes[k] == 0 && !exported_mut_globals[k]);
        global_consts.push(if can_fold {
            Some(global_inits[k])
        } else {
            None
        });
    }

    // Pin the hottest mutable scalar I32/I64 global in `R12` ONLY when its static access density
    // justifies giving up a callee-saved GP register across every function (e.g. LLVM/Rust shadow
    // `$__stack_pointer` averages >= 3 accesses/fn; Blot modules average ~1 access/fn and win much
    // more from having `R12` as a 3rd general-purpose callee-saved register!).
    let min_pin_accesses = decoded.code.len().saturating_mul(3).max(16);
    let pinned_global = (0..globals.len())
        .filter(|&k| {
            global_consts[k].is_none()
                && globals[k].1
                && matches!(globals[k].0, ValType::I32 | ValType::I64)
                && (global_reads[k] + global_writes[k]) >= min_pin_accesses
        })
        .max_by_key(|&k| (global_reads[k] + global_writes[k], std::cmp::Reverse(k)))
        .map(|k| k as u32);

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

    Ok((
        X64Module {
            imports,
            func_type_idx,
            module_types: decoded.types.clone(),
            globals,
            global_inits,
            global_consts,
            pinned_global,
            exported_globals,
            exported_funcs,
            ci_index,
            ci_sigs,
            mut_extern_tables,
            memory,
            memory64,
            fixed32,
            extra_memories,
            tags,
            uses_exceptions,
            target,
            standalone_start,
            active_elem_checks,
            active_data_segs,
            start,
        },
        all_body_summaries,
    ))
}
