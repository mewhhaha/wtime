//! Lean, self-contained Wasm admission & metadata analysis for `--backend x64`.
//!
//! Decoupled from `codegen::Module` so `--backend x64` skips C/Rust-only analyses
//! (global-preservation fixpoints, callee read/write sets, outline planning) and
//! enforces an explicit Stage 1 admission contract before emitting any IR or code.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use wtime_types::{CompositeType, FuncType, HeapType, IndexType, ValType};
use wtime_validate::{DecodedModule, ExternalKind, Immediate, ImportKind, Instruction, SegmentMode};

pub type Res<T> = Result<T, String>;

/// Admission & compilation caps preventing OOM, peak-memory spikes, or integer overflow on adversarial modules.
pub const MAX_MEM_PAGES: u64 = 65_536;
pub const MAX_FUNCREF_TABLE_SLOTS: u32 = 1_048_576;
pub const MAX_TOTAL_FUNCREF_SLOTS: usize = 2_097_152;
pub const MAX_EXTERN_TABLE_SLOTS: u32 = 65_536;
pub const MAX_TOTAL_EXTERN_SLOTS: usize = 65_536;
pub const MAX_GLOBALS: usize = 65_536;
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
/// peak concurrent liveness memory, and aggregate emitted code/rodata limits BEFORE growth.
pub struct CompileBudget {
    deadline: Option<Instant>,
    in_flight_liveness: AtomicUsize,
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
        self.budget.in_flight_liveness.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl CompileBudget {
    pub fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            in_flight_liveness: AtomicUsize::new(0),
            total_code_bytes: AtomicUsize::new(0),
            total_rodata_bytes: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn check_deadline(&self) -> Res<()> {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err("x64: compilation time limit exceeded".into());
        }
        Ok(())
    }

    /// Atomically reserves `bytes` against `MAX_CONCURRENT_LIVENESS_BYTES` BEFORE allocating
    /// liveness bitset matrices across parallel workers.
    pub fn reserve_liveness(&self, bytes: usize) -> Res<LivenessBudgetGuard<'_>> {
        let mut cur = self.in_flight_liveness.load(Ordering::Acquire);
        loop {
            let next = cur
                .checked_add(bytes)
                .ok_or("x64: concurrent liveness memory overflow")?;
            if next > MAX_CONCURRENT_LIVENESS_BYTES {
                return Err(format!(
                    "x64: concurrent liveness memory ({next} B) exceeds peak cap {MAX_CONCURRENT_LIVENESS_BYTES} B"
                ));
            }
            match self.in_flight_liveness.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(LivenessBudgetGuard { budget: self, bytes }),
                Err(actual) => cur = actual,
            }
        }
    }

    /// Atomically charges `delta` emitted machine-code bytes against `MAX_TOTAL_CODE_BYTES`.
    pub fn charge_code(&self, delta: usize) -> Res<()> {
        if delta == 0 {
            return Ok(());
        }
        let mut cur = self.total_code_bytes.load(Ordering::Acquire);
        loop {
            let next = cur.checked_add(delta).ok_or("x64: aggregate code size overflow")?;
            if next > MAX_TOTAL_CODE_BYTES {
                return Err(format!(
                    "x64: aggregate code size ({next} B) exceeds cap {MAX_TOTAL_CODE_BYTES} B"
                ));
            }
            match self.total_code_bytes.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
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
            let next = cur.checked_add(delta).ok_or("x64: aggregate rodata size overflow")?;
            if next > MAX_TOTAL_RODATA_BYTES {
                return Err(format!(
                    "x64: aggregate rodata size ({next} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
                ));
            }
            match self.total_rodata_bytes.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
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
    pub table: u32,
    pub table_len: u32,
    /// Representative index into `X64Module::module_types` (zero `FuncType` duplication).
    pub type_idx: u32,
    pub min_slot: u32,
    pub span: u32,
    /// Sorted `(table_slot, func_index)` entries whose function signature equals `module_types[type_idx]`.
    pub targets: Vec<(u32, u32)>,
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
    /// `Some(fixed_bytes)` iff memory 0 exists with `min == max`.
    pub fixed32: Option<u64>,
    /// Active element segment bounds checks `(table_len, base_off, elem_len)` for `wt_initialize`.
    pub active_elem_checks: Vec<(u32, u32, u32)>,
    /// Active data segments `(mem_off, decoded.data index)` for `wt_initialize` (in declaration order).
    pub active_data_segs: Vec<(u32, usize)>,
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
                let arr: [u8; 16] = bytes.as_ref().try_into().map_err(|_| "invalid v128.const")?;
                X64ConstVal::V128(arr)
            }
            (0xd0, _) => X64ConstVal::RefNull,
            (0x23, Immediate::Index(g)) => {
                *prior.get(*g as usize).ok_or("const global.get out of range")?
            }
            (0x6a..=0x6c, _) => match (stk.pop(), stk.pop()) {
                (Some(X64ConstVal::I32(b)), Some(X64ConstVal::I32(a))) => X64ConstVal::I32(match i.opcode {
                    0x6a => a.wrapping_add(b),
                    0x6b => a.wrapping_sub(b),
                    _ => a.wrapping_mul(b),
                }),
                _ => return Err("invalid i32 const expr".into()),
            },
            (0x7c..=0x7e, _) => match (stk.pop(), stk.pop()) {
                (Some(X64ConstVal::I64(b)), Some(X64ConstVal::I64(a))) => X64ConstVal::I64(match i.opcode {
                    0x7c => a.wrapping_add(b),
                    0x7d => a.wrapping_sub(b),
                    _ => a.wrapping_mul(b),
                }),
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
        other => Err(format!("x64: unsupported value type {other:?}")),
    }
}

fn check_supported_opcode(
    op: u16,
    imm: &Immediate,
    type_arity: &[usize],
    type_admitted: &[bool],
    total_sig_arity: &mut usize,
) -> Res<()> {
    match op {
        0x00 | 0x01 | 0x05 | 0x0b..=0x10 | 0x1a | 0x1b | 0x20..=0x26 | 0x41..=0x44 | 0x45..=0xc4 | 0xd1 => Ok(()),
        0x02..=0x04 => match imm {
            Immediate::Block(-64 | -1 | -2 | -3 | -4 | -5 | -17) => Ok(()),
            Immediate::Block(i) if *i >= 0 => {
                let idx = *i as usize;
                if !*type_admitted.get(idx).ok_or("x64: block type index out of range")? {
                    return Err(format!("x64: block type index {idx} uses unsupported value type"));
                }
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
                if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                    return Err(format!("x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"));
                }
                Ok(())
            }
            Immediate::BlockType(vt) => check_val_type(*vt),
            other => Err(format!("x64: unsupported block type {other:?}")),
        },
        0x11 => match imm {
            Immediate::Pair(ti, _) => {
                let idx = *ti as usize;
                if !*type_admitted.get(idx).ok_or("x64: call_indirect type index out of range")? {
                    return Err(format!("x64: call_indirect type index {idx} uses unsupported value type"));
                }
                *total_sig_arity = total_sig_arity.saturating_add(type_arity[idx]);
                if *total_sig_arity > MAX_TOTAL_SIG_ARITY {
                    return Err(format!("x64: expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"));
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
            Immediate::Index(0) | Immediate::None => Ok(()),
            _ => Err("x64: memory.size/grow only supports memory 0".into()),
        },
        0xd0 => match imm {
            Immediate::Heap(HeapType::Extern) => Ok(()),
            other => Err(format!("x64: unsupported ref.null heap type {other:?}")),
        },
        0x28..=0x3e | 0xfd00 | 0xfd0b => match imm {
            Immediate::MemArg { memory: 0, .. } => Ok(()),
            _ => Err("x64: only memory index 0 is supported".into()),
        },
        0xfc00..=0xfc07 => Ok(()),
        0xfc0a => match imm {
            Immediate::Pair(0, 0) | Immediate::None => Ok(()),
            _ => Err("x64: memory.copy only supports memory 0".into()),
        },
        0xfc0b => match imm {
            Immediate::Index(0) | Immediate::None => Ok(()),
            _ => Err("x64: memory.fill only supports memory 0".into()),
        },
        0xfd0c | 0xfd0f..=0xfd12 | 0xfd1d | 0xfd1e => Ok(()),
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
        return Err(format!("x64: Blot import {mod_name:?}::{name:?} has unsupported signature {ty:?}"));
    }
    Ok(())
}

/// Enforces the Stage 1 admission contract and builds lean `X64Module` metadata.
pub fn admit_x64_module(decoded: &DecodedModule) -> Res<X64Module> {
    admit_x64_module_with_budget(decoded, None)
}

/// Enforces the Stage 1 admission contract while cooperatively polling `budget`'s deadline.
pub fn admit_x64_module_with_budget(decoded: &DecodedModule, budget: Option<&CompileBudget>) -> Res<X64Module> {
    if let Some(b) = budget {
        b.check_deadline()?;
    }
    if !decoded.tags.is_empty() {
        return Err("x64: exception tags are not supported".into());
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
                return Err("x64: GC/subtyping/non-final/descriptor types are not supported".into());
            }
        }
    }
    if decoded.memories.len() > 1 {
        return Err(format!("x64: multi-memory ({}) is not supported", decoded.memories.len()));
    }
    let (memory, fixed32) = if let Some(mt) = decoded.memories.first() {
        if mt.index_type != IndexType::I32 {
            return Err("x64: memory64 is not supported in Stage 1".into());
        }
        if mt.shared {
            return Err("x64: shared memory is not supported".into());
        }
        if mt.page_size_log2 != 16 {
            return Err(format!("x64: custom memory page size (2^{}) is not supported", mt.page_size_log2));
        }
        let max_p = mt.limits.max.unwrap_or(MAX_MEM_PAGES).min(MAX_MEM_PAGES);
        if mt.limits.min > max_p {
            return Err(format!("x64: memory min ({}) exceeds max ({max_p})", mt.limits.min));
        }
        let fixed = (mt.limits.min == max_p).then_some(mt.limits.min * 65_536);
        (Some((mt.limits.min, max_p)), fixed)
    } else {
        (None, None)
    };

    // Pre-validate and intern module types ONCE into canonical `u32` IDs + cached arity/admissibility
    // so every function declaration, block type, table slot, and `call_indirect` site runs in O(1)
    // with zero per-function or per-slot `FuncType` cloning/re-scanning.
    let mut sig_to_canon: HashMap<&FuncType, u32> = HashMap::with_capacity(decoded.types.len());
    let mut type_canon: Vec<u32> = Vec::with_capacity(decoded.types.len());
    let mut type_arity: Vec<usize> = Vec::with_capacity(decoded.types.len());
    let mut type_admitted: Vec<bool> = Vec::with_capacity(decoded.types.len());
    for fty in &decoded.types {
        let next_id = sig_to_canon.len() as u32;
        let cid = *sig_to_canon.entry(fty).or_insert(next_id);
        type_canon.push(cid);
        type_arity.push(fty.params.len().saturating_add(fty.results.len()));
        type_admitted.push(fty.params.iter().chain(&fty.results).all(|&vt| check_val_type(vt).is_ok()));
    }

    // Charge per-function/total locals AND aggregate signature arity (params + results) BEFORE allocating.
    let mut precheck_total_locals = 0usize;
    let mut total_sig_arity = 0usize;
    for (bi, body) in decoded.code.iter().enumerate() {
        let ti = decoded.functions.get(bi).ok_or("function/code count mismatch")?.0 as usize;
        let fty = decoded.types.get(ti).ok_or("function type index out of range")?;
        let n_locals = fty.params.len().saturating_add(body.locals.len());
        if n_locals > MAX_LOCALS_PER_FN {
            return Err(format!("x64: function body {bi} has {n_locals} locals (cap {MAX_LOCALS_PER_FN})"));
        }
        precheck_total_locals = precheck_total_locals.saturating_add(n_locals);
        if precheck_total_locals > MAX_TOTAL_LOCALS {
            return Err(format!("x64: module total locals exceeds cap {MAX_TOTAL_LOCALS}"));
        }
        total_sig_arity = total_sig_arity.saturating_add(type_arity[ti]);
        if total_sig_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!("x64: module expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"));
        }
    }

    // Imports: plain function imports only (zero per-function `FuncType` cloning).
    let mut imports = Vec::new();
    let mut func_type_idx: Vec<u32> = Vec::with_capacity(decoded.imports.len() + decoded.functions.len());
    let mut func_canon_sig: Vec<u32> = Vec::with_capacity(decoded.imports.len() + decoded.functions.len());
    for imp in &decoded.imports {
        let ti_u32 = match imp.kind {
            ImportKind::Func(t) => t.0,
            _ => return Err(format!("x64: only plain function imports are supported (got {:?}::{:?})", imp.module, imp.name)),
        };
        let ti = ti_u32 as usize;
        let fty = decoded.types.get(ti).ok_or("import type index out of range")?;
        if !type_admitted[ti] {
            return Err(format!("x64: import {:?}::{:?} uses unsupported value type", imp.module, imp.name));
        }
        total_sig_arity = total_sig_arity.saturating_add(type_arity[ti]);
        if total_sig_arity > MAX_TOTAL_SIG_ARITY {
            return Err(format!("x64: module expanded signature arity exceeds cap {MAX_TOTAL_SIG_ARITY}"));
        }
        check_blot_import_sig(&imp.module, &imp.name, fty)?;
        imports.push((imp.module.clone(), imp.name.clone()));
        func_type_idx.push(ti_u32);
        func_canon_sig.push(type_canon[ti]);
    }

    for &ti in &decoded.functions {
        let ti_u = ti.0 as usize;
        if !*type_admitted.get(ti_u).ok_or("function type index out of range")? {
            return Err(format!("x64: function type index {ti_u} uses unsupported value type"));
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
        if let Some(init_expr) = decoded.table_inits.get(ti).and_then(|o| o.as_ref()) {
            let is_matching_null = matches!(
                init_expr.as_slice(),
                [Instruction { opcode: 0xd0, immediate: Immediate::Heap(ht) }, Instruction { opcode: 0x0b, .. }]
                    if *ht == tt.element.heap
            );
            if !is_matching_null {
                return Err(format!("x64: non-null table initializer on table {ti} is not supported"));
            }
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
            other => return Err(format!("x64: unsupported table element type {other:?} on table {ti}")),
        }
    }

    if decoded.globals.len() > MAX_GLOBALS {
        return Err(format!("x64: module global count ({}) exceeds cap {MAX_GLOBALS}", decoded.globals.len()));
    }
    let worst_ctx_bytes = 128usize
        .saturating_add(decoded.globals.len().saturating_mul(16))
        .saturating_add(total_extern_slots.saturating_mul(8));
    if worst_ctx_bytes > MAX_CTX_BYTES {
        return Err(format!("x64: Ctx storage ({worst_ctx_bytes} B) exceeds cap {MAX_CTX_BYTES} B"));
    }

    // Globals: evaluate all initializers; track which globals are written or exported-mutable.
    let mut globals = Vec::with_capacity(decoded.globals.len());
    let mut global_inits = Vec::with_capacity(decoded.globals.len());
    for g in &decoded.globals {
        check_val_type(g.ty.value)?;
        let init = eval_const_expr(&g.init, &global_inits)?;
        globals.push((g.ty.value, g.ty.mutable));
        global_inits.push(init);
    }

    // Exports: admit functions, globals, and memory 0 (`blot:memory` etc.); reject table/tag exports.
    let mut exported_funcs = Vec::new();
    let mut exported_globals = Vec::new();
    let mut exported_mut_globals = vec![false; globals.len()];
    for exp in &decoded.exports {
        match exp.kind {
            ExternalKind::Func => {
                exported_funcs.push((exp.name.clone(), exp.index as usize));
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
                if exp.index != 0 || memory.is_none() {
                    return Err(format!("x64: unsupported memory export {:?} (index {})", exp.name, exp.index));
                }
            }
            ExternalKind::Table | ExternalKind::Tag => {
                return Err(format!("x64: unsupported export {:?} ({:?})", exp.name, exp.kind));
            }
        }
    }

    // Active element segments: validate forms and populate static funcref tables (recording bounds
    // checks so `wt_initialize` reports `TRAP_TABLE_OOB` at instantiation time per Wasm spec).
    let mut active_elem_checks = Vec::new();
    for elem in &decoded.elements {
        match &elem.mode {
            SegmentMode::Active { index, offset } => {
                let off_val = eval_const_expr(offset, &global_inits)?;
                let X64ConstVal::I32(off_i32) = off_val else {
                    return Err("x64: active element offset must be i32".into());
                };
                let Some(slots) = funcref_tables.get_mut(index) else {
                    return Err(format!("x64: active element segment targets non-funcref table {index}"));
                };
                let mut entries = Vec::with_capacity(elem.init.len());
                for expr in &elem.init {
                    match expr.as_slice() {
                        [Instruction { opcode: 0xd2, immediate: Immediate::Index(f) }, Instruction { opcode: 0x0b, .. }] => {
                            entries.push(Some(*f));
                        }
                        [Instruction { opcode: 0xd0, .. }, Instruction { opcode: 0x0b, .. }] => {
                            entries.push(None);
                        }
                        _ => return Err("x64: unsupported active element init expression".into()),
                    }
                }
                let base_u32 = off_i32 as u32;
                let elem_len = u32::try_from(entries.len()).map_err(|_| "active element segment too large")?;
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
        match &seg.mode {
            SegmentMode::Active { index: 0, offset } => {
                let X64ConstVal::I32(off_i32) = eval_const_expr(offset, &global_inits)? else {
                    return Err("x64: active data offset must be i32".into());
                };
                active_data_segs.push((off_i32 as u32, di));
            }
            SegmentMode::Active { index, .. } => {
                return Err(format!("x64: active data segment on unsupported memory {index}"));
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
    for (tbl, slots) in funcref_tables {
        funcref_table_lens.insert(tbl, slots.len() as u32);
        for (slot, entry) in slots.into_iter().enumerate() {
            if let Some(f) = entry
                && let Some(&csig) = func_canon_sig.get(f as usize)
            {
                funcref_by_sig.entry((tbl, csig)).or_default().push((slot as u32, f));
            }
        }
    }

    // Single linear scan over ALL function bodies (including unreachable code) to:
    // 1. Enforce opcode, local-count, value-type, expanded-signature, and br_table aggregate allowlists;
    // 2. Count global reads/writes to detect constant globals & pick `pinned_global`;
    // 3. Collect canonical `(table, canon_sig_id)` `call_indirect` signatures.
    let mut total_br_table_entries = 0usize;
    let mut global_reads = vec![0usize; globals.len()];
    let mut global_writes = vec![0usize; globals.len()];
    let mut ci_canon: HashMap<(u32, u32), usize> = HashMap::new();
    let mut ci_index: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut ci_sigs: Vec<X64CiSig> = Vec::new();
    let mut total_ci_bytes = 0usize;

    for (bi, body) in decoded.code.iter().enumerate() {
        if let Some(b) = budget
            && (bi & 63) == 0
        {
            b.check_deadline()?;
        }
        for &vt in &body.locals {
            check_val_type(vt)?;
        }
        for insn in &body.instructions {
            check_supported_opcode(insn.opcode, &insn.immediate, &type_arity, &type_admitted, &mut total_sig_arity)?;
            match insn.opcode {
                0x0e => {
                    if let Immediate::BrTable(ref raw_targets, _) = insn.immediate {
                        total_br_table_entries = total_br_table_entries.saturating_add(raw_targets.len());
                        if total_br_table_entries > MAX_AGGREGATE_BR_TABLE_ENTRIES {
                            return Err(format!(
                                "x64: aggregate br_table entries ({total_br_table_entries}) exceed cap {MAX_AGGREGATE_BR_TABLE_ENTRIES}"
                            ));
                        }
                    }
                }
                0x23 => {
                    if let Immediate::Index(g) = insn.immediate {
                        let slot = global_reads
                            .get_mut(g as usize)
                            .ok_or_else(|| format!("x64: global.get index {g} out of range"))?;
                        *slot += 1;
                    }
                }
                0x24 => {
                    if let Immediate::Index(g) = insn.immediate {
                        let slot = global_writes
                            .get_mut(g as usize)
                            .ok_or_else(|| format!("x64: global.set index {g} out of range"))?;
                        *slot += 1;
                    }
                }
                0x25 | 0x26 => {
                    if let Immediate::Index(t) = insn.immediate
                        && !mut_extern_tables.contains_key(&t)
                    {
                        return Err(format!("x64: table.get/set on unsupported table {t}"));
                    }
                }
                0x11 => {
                    let Immediate::Pair(ti, tbl) = insn.immediate else {
                        return Err("x64: malformed call_indirect immediate".into());
                    };
                    if ci_index.contains_key(&(tbl, ti)) {
                        continue;
                    }
                    let &csig = type_canon.get(ti as usize).ok_or("call_indirect type index")?;
                    let ci_id = if let Some(&existing) = ci_canon.get(&(tbl, csig)) {
                        existing
                    } else {
                        let &table_len = funcref_table_lens
                            .get(&tbl)
                            .ok_or_else(|| format!("x64: call_indirect on unsupported table {tbl}"))?;
                        let targets = funcref_by_sig.remove(&(tbl, csig)).unwrap_or_default();
                        let (min_slot, span) = if let (Some(&(first, _)), Some(&(last, _))) = (targets.first(), targets.last()) {
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
                _ => {}
            }
        }
    }

    // Fold globals that are immutable OR never written by Wasm AND not exported-mutable.
    let mut global_consts = Vec::with_capacity(globals.len());
    for (k, &(_, mutable)) in globals.iter().enumerate() {
        let can_fold = !mutable || (global_writes[k] == 0 && !exported_mut_globals[k]);
        global_consts.push(if can_fold { Some(global_inits[k]) } else { None });
    }

    // Pick single hottest non-const mutable I32/I64 global for `R12` pinning.
    let pinned_global = (0..globals.len())
        .filter(|&k| {
            global_consts[k].is_none()
                && globals[k].1
                && matches!(globals[k].0, ValType::I32 | ValType::I64)
                && (global_reads[k] + global_writes[k]) > 0
        })
        .max_by_key(|&k| (global_reads[k] + global_writes[k], std::cmp::Reverse(k)))
        .map(|k| k as u32);

    Ok(X64Module {
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
        fixed32,
        active_elem_checks,
        active_data_segs,
        start: decoded.start.map(|s| s.0),
    })
}
