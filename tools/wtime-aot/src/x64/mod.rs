//! Self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm -> Linux x86-64 LP64
//! ELF64 `.so` compiler (`wtime-aot --backend x64`).

pub mod asm;
pub mod elf;
pub mod ir;
pub mod lower;
pub mod module;
pub mod opt;
pub mod regalloc;
pub mod runtime;
#[cfg(test)]
mod tests_abi_faults;
#[cfg(test)]
mod tests_oracle;
#[cfg(test)]
mod tests_unit;
pub mod verify;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use wtime_types::{ErrorKind, ValType};
use wtime_validate::DecodedModule;

use asm::{Asm, Reloc, RelocTarget};
use elf::{ElfLayout, ExportDef};
use ir::{IrFunc, lower_wasm_to_ir_with_budget};
use lower::{TargetIsa, lower_ir_to_x64_with_budget};
use module::{
    CompileBudget, MAX_CTX_BYTES, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES, X64ConstVal,
    X64Module, admit_x64_module_with_budget, x64_validate_features, x64_validate_limits,
};
use regalloc::{AllocConfig, AllocResult, allocate_with_config};
use runtime::{
    EXPORT_NAMES, RuntimeTables, RuntimeTestHooks, TRAP_MESSAGES, emit_all_exports,
    emit_all_helpers, emit_import_trampoline, export_val_kind, global_kind_code,
};
#[cfg(test)]
pub use verify::verify_alloc;
pub use verify::{verify_alloc_with_config, verify_ir};

/// Dedicated CLI exit code emitted when `--backend x64` cleanly rejects a module on admission
/// (distinct from exit code `1` for usage/I/O/timeout/compiler errors).
pub const X64_ADMISSION_REJECTED_EXIT_CODE: i32 = 4;

/// Maximum Wasm input module size accepted by `--backend x64` (64 MiB).
pub const MAX_WASM_BYTES_X64: usize = 64 << 20;

/// Maximum synchronous `blot_aot_active_*` callback reentry depth.
pub const MAX_ACTIVE_REENTRY_DEPTH: u32 = 16;

/// Dedicated guest stack geometry: 64 KiB `PROT_NONE` guard + 8 MiB usable RW stack,
/// with 128 KiB headroom above the guard page.
pub const GUEST_STACK_GUARD_BYTES: usize = 64 << 10;
pub const GUEST_STACK_USABLE_BYTES: usize = 8 << 20;
pub const GUEST_STACK_TOTAL_BYTES: usize = GUEST_STACK_GUARD_BYTES + GUEST_STACK_USABLE_BYTES;
pub const GUEST_STACK_FLOOR_HEADROOM: usize = 128 << 10;

// ---- `Ctx` (`BlotHandle`) byte-for-byte fixed header offsets ----
pub const OFF_MEM_BASE: i32 = 0;
pub const OFF_MEM_LEN: i32 = 8;
pub const OFF_MEM_MAX_PAGES: i32 = 16;
pub const OFF_MEM_RESERVE_BYTES: i32 = 24;
pub const OFF_HANDLE_ALLOC_BYTES: i32 = 32;
pub const OFF_STACK_FLOOR: i32 = 40;
pub const OFF_HOST_USER_CTX: i32 = 48;
pub const OFF_HOST_FN: i32 = 56;
pub const OFF_SCRATCH_TMP: i32 = 64;
pub const OFF_RET_SPILL_PTR: i32 = 72;
pub const OFF_GUEST_STACK_BASE: i32 = 80;
pub const OFF_GUEST_STACK_TOP: i32 = 88;
pub const OFF_HOST_RSP: i32 = 96;
pub const OFF_ACTIVE_GUEST_RSP: i32 = 104;
pub const OFF_IN_CALL: i32 = 112;
pub const OFF_CPU_HAS_CET_SS: i32 = 113;
pub const OFF_ACTIVE_DEPTH: i32 = 116;
pub const CTX_HEADER_BYTES: usize = 128;

// ---- Canonical trap codes `1..=22` ----
pub const TRAP_UNREACHABLE: u8 = 1;
pub const TRAP_OOB: u8 = 2;
pub const TRAP_TABLE_OOB: u8 = 3;
pub const TRAP_INDIRECT_CALL: u8 = 4;
pub const TRAP_DIV_ZERO: u8 = 5;
pub const TRAP_INT_OVERFLOW: u8 = 6;
pub const TRAP_INVALID_CONV: u8 = 7;
pub const TRAP_CALL_STACK: u8 = 8;
pub const TRAP_MEM_TOO_LARGE: u8 = 9;
#[allow(dead_code)]
pub const TRAP_MEM_ALLOC_OVERFLOW: u8 = 10;
pub const TRAP_MEM_LIMIT: u8 = 11;
pub const TRAP_ALLOC_FAILED: u8 = 12;
pub const TRAP_HOST: u8 = 19;
#[allow(dead_code)]
pub const TRAP_EXIT: u8 = 20;
#[allow(dead_code)]
pub const TRAP_STOP_ITER: u8 = 21;
pub const TRAP_UNSUPPORTED_ISA: u8 = 22;

// ---- Shared runtime helper IDs ----
pub const HELPER_TRUNC_BASE: u32 = 0; // 0..=15
pub const HELPER_MEM_GROW: u32 = 16;
pub const HELPER_MEM_COPY: u32 = 17;
pub const HELPER_MEM_FILL: u32 = 18;
pub(crate) const HELPER_HOST_CALL: u32 = 19;
pub(crate) const HELPER_GROW_CORE: u32 = 20;
pub(crate) const HELPER_INITIALIZE: u32 = 21;
pub(crate) const HELPER_READ_GLOBAL: u32 = 22;
pub(crate) const NUM_HELPERS: usize = 23;

/// Per-signature `call_indirect` dispatch table metadata in `.rodata`.
#[derive(Clone, Copy, Debug)]
pub struct CiTableLayout {
    pub table_len: u32,
    pub min_slot: u32,
    pub span: u32,
    pub rodata_off: u32,
}

/// Checked `Ctx` layout consumed by `lower.rs` and `runtime.rs`.
#[derive(Clone, Debug)]
pub struct CtxLayout {
    pub pinned_global: Option<u32>,
    /// `Some(fixed_bytes)` iff memory 0 is fixed-size (`min == max`), freeing `R13` as a 4th callee-saved GP reg.
    pub fixed_mem_bytes: Option<u64>,
    pub global_offs: Vec<i32>,
    pub extern_tables: BTreeMap<u32, (i32, u32)>,
    pub ci_tables: Vec<CiTableLayout>,
    pub total_bytes: usize,
}

impl CtxLayout {
    /// Computes checked byte offsets for all non-folded globals and mutable `externref` tables,
    /// rounding `total_bytes` up to a 4 KiB page and enforcing `total_bytes <= MAX_CTX_BYTES`.
    pub fn compute(module: &X64Module, ci_tables: Vec<CiTableLayout>) -> Result<Self, String> {
        let mut cur = CTX_HEADER_BYTES;
        let mut global_offs = Vec::with_capacity(module.globals.len());
        for (k, &(vt, _)) in module.globals.iter().enumerate() {
            if module.global_consts[k].is_some() {
                global_offs.push(-1);
                continue;
            }
            let (align, size) = if vt == ValType::V128 {
                (16usize, 16usize)
            } else {
                (8usize, 8usize)
            };
            cur = align_up_checked(cur, align)?;
            let off = i32::try_from(cur).map_err(|_| "x64: Ctx global offset exceeds i32")?;
            cur = cur.checked_add(size).ok_or("x64: Ctx size overflow")?;
            if cur > MAX_CTX_BYTES {
                return Err(format!(
                    "x64: Ctx layout ({cur} B) exceeds cap {MAX_CTX_BYTES} B"
                ));
            }
            global_offs.push(off);
        }
        let mut extern_tables = BTreeMap::new();
        for (&tbl, &len) in &module.mut_extern_tables {
            cur = align_up_checked(cur, 8)?;
            let off = i32::try_from(cur).map_err(|_| "x64: Ctx extern table offset exceeds i32")?;
            let bytes = (len as usize)
                .checked_mul(8)
                .ok_or("x64: Ctx extern table size overflow")?;
            cur = cur.checked_add(bytes).ok_or("x64: Ctx size overflow")?;
            if cur > MAX_CTX_BYTES {
                return Err(format!(
                    "x64: Ctx layout ({cur} B) exceeds cap {MAX_CTX_BYTES} B"
                ));
            }
            extern_tables.insert(tbl, (off, len));
        }
        let total_bytes = align_up_checked(cur, 4096)?;
        if total_bytes > MAX_CTX_BYTES {
            return Err(format!(
                "x64: page-aligned Ctx size ({total_bytes} B) exceeds cap {MAX_CTX_BYTES} B"
            ));
        }
        Ok(Self {
            pinned_global: module.pinned_global,
            fixed_mem_bytes: module.fixed32,
            global_offs,
            extern_tables,
            ci_tables,
            total_bytes,
        })
    }
}

/// Shared `.rodata` scalar/vector constants.
///
/// Invariant: `f32_abs`, `f32_neg`, `f64_abs`, `f64_neg` are each 16-byte aligned and 16 bytes
/// wide because `lower.rs` emits 128-bit RIP-relative `andps`/`xorps`/`andpd`/`xorpd` operands.
#[derive(Clone, Copy, Debug)]
pub struct RodataConsts {
    pub f32_abs: u32,
    pub f32_neg: u32,
    pub f64_abs: u32,
    pub f64_neg: u32,
    pub f32_qnan: u32,
    pub f64_qnan: u32,
    pub mxcsr_wasm: u32,
    // Truncation helper boundary constants:
    pub f32_neg_one: u32,
    pub f32_neg_2p31: u32,
    pub f32_2p31: u32,
    pub f32_2p32: u32,
    pub f32_neg_2p63: u32,
    pub f32_2p63: u32,
    pub f32_2p64: u32,
    pub f64_neg_one: u32,
    pub f64_neg_2p31_m1: u32,
    pub f64_2p31: u32,
    pub f64_2p32: u32,
    pub f64_neg_2p63: u32,
    pub f64_2p63: u32,
    pub f64_2p64: u32,
}

/// Typed error distinguishing clean Stage 1 admission rejections (CLI exit code `4`) from
/// usage/I/O/timeout/invalid-Wasm/internal errors (CLI exit code `1`).
#[derive(Clone, Debug)]
pub enum X64CompileError {
    AdmissionRejected(String),
    Failed(String),
}

impl std::fmt::Display for X64CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            X64CompileError::AdmissionRejected(s) | X64CompileError::Failed(s) => f.write_str(s),
        }
    }
}

/// Compilation options for `compile_wasm_to_x64_so`.
#[derive(Clone, Copy, Debug)]
pub struct X64CompileOptions {
    pub isa: TargetIsa,
    /// Worker count: `0` = auto (`available_parallelism().min(64)`), explicit clamped `1..=64`.
    pub jobs: usize,
    pub deadline: Option<Instant>,
    /// Enable SSA IR optimizations (`false` exercises `lower_wasm_to_ir_unopt` for differential tests).
    pub opt: bool,
    /// Run `verify_ir` and `verify_alloc` on every defined function during compilation.
    pub verify: bool,
    pub hooks: RuntimeTestHooks,
}

impl Default for X64CompileOptions {
    fn default() -> Self {
        Self {
            isa: TargetIsa {
                has_abm_bmi1: false,
            },
            jobs: 0,
            deadline: None,
            opt: true,
            verify: true,
            hooks: RuntimeTestHooks::default(),
        }
    }
}

/// Validates `--target-cpu` syntax independently of host OS/architecture, matching Pen's
/// `blot-host::wtime_aot::parse_x64_target_cpu_syntax` 1:1.
pub fn parse_x64_target_cpu_syntax(target_cpu: &str) -> Result<Option<&'static str>, String> {
    match target_cpu {
        "native" => Ok(None),
        "x86-64-v2" | "generic" => Ok(Some("x64-v2")),
        "x86-64-v3" | "x86-64-v4" | "haswell" | "broadwell" | "skylake" | "skylake-avx512"
        | "cascadelake" | "cooperlake" | "cannonlake" | "icelake-client" | "icelake-server"
        | "tigerlake" | "rocketlake" | "alderlake" | "raptorlake" | "sapphirerapids"
        | "emeraldrapids" | "graniterapids" => Ok(Some("x64-v3")),
        other
            if other
                .strip_prefix("znver")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) =>
        {
            Ok(Some("x64-v3"))
        }
        other => Err(format!(
            "unsupported --target-cpu {other:?} for --backend x64 (expected native, x86-64-v2, generic, x86-64-v3, x86-64-v4, znver<N>, or haswell+)"
        )),
    }
}

/// Resolves `--target-cpu` to the canonical internal ISA profile tag (`"x64-v2"` or `"x64-v3"`)
/// and `TargetIsa`, enforcing Linux x86-64 LP64 host support after syntax validation.
pub fn resolve_x64_isa(target_cpu: &str) -> Result<(&'static str, TargetIsa), String> {
    let explicit = parse_x64_target_cpu_syntax(target_cpu)?;
    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    )))]
    {
        let _ = explicit;
        Err("wtime-aot --backend x64 is only supported on Linux x86_64 LP64 hosts".into())
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    {
        let tag = match explicit {
            Some(t) => t,
            None => {
                if std::is_x86_feature_detected!("lzcnt") && std::is_x86_feature_detected!("bmi1") {
                    "x64-v3"
                } else {
                    "x64-v2"
                }
            }
        };
        Ok((
            tag,
            TargetIsa {
                has_abm_bmi1: tag == "x64-v3",
            },
        ))
    }
}

pub fn resolve_jobs(jobs: usize) -> usize {
    if jobs == 0 {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 64)
    } else {
        jobs.clamp(1, 64)
    }
}

#[inline]
fn align_up_checked(v: usize, align: usize) -> Result<usize, String> {
    let mask = align - 1;
    v.checked_add(mask)
        .map(|x| x & !mask)
        .ok_or_else(|| "x64: layout size overflow".to_string())
}

struct RodataBuilder {
    buf: Vec<u8>,
}

impl RodataBuilder {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
        }
    }

    fn align(&mut self, align: usize) -> Result<u32, String> {
        let next = align_up_checked(self.buf.len(), align)?;
        if next > MAX_TOTAL_RODATA_BYTES {
            return Err(format!(
                "x64: .rodata ({next} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
            ));
        }
        self.buf.resize(next, 0);
        u32::try_from(self.buf.len()).map_err(|_| "x64: .rodata offset exceeds u32".into())
    }

    fn push_bytes(&mut self, align: usize, bytes: &[u8]) -> Result<u32, String> {
        let off = self.align(align)?;
        let end = self
            .buf
            .len()
            .checked_add(bytes.len())
            .ok_or("x64: .rodata size overflow")?;
        if end > MAX_TOTAL_RODATA_BYTES {
            return Err(format!(
                "x64: .rodata ({end} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
            ));
        }
        self.buf.extend_from_slice(bytes);
        Ok(off)
    }

    fn push_zeros(&mut self, align: usize, len: usize) -> Result<u32, String> {
        let off = self.align(align)?;
        let end = self
            .buf
            .len()
            .checked_add(len)
            .ok_or("x64: .rodata size overflow")?;
        if end > MAX_TOTAL_RODATA_BYTES {
            return Err(format!(
                "x64: .rodata ({end} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
            ));
        }
        self.buf.resize(end, 0);
        Ok(off)
    }

    fn push_u32(&mut self, v: u32) -> Result<u32, String> {
        self.push_bytes(4, &v.to_le_bytes())
    }

    fn push_u64(&mut self, v: u64) -> Result<u32, String> {
        self.push_bytes(8, &v.to_le_bytes())
    }
}

struct CallExportPatch {
    table_rodata_off: u32,
    entry_fn_rel_off: u32,
    func_idx: usize,
}

struct SharedRodata {
    rodata: Vec<u8>,
    consts: RodataConsts,
    tables: RuntimeTables,
    ci_tables: Vec<CiTableLayout>,
    call_export_patches: Vec<CallExportPatch>,
}

fn build_shared_rodata(
    decoded: &DecodedModule,
    module: &X64Module,
    layout_global_offs: &[i32],
    budget: &CompileBudget,
) -> Result<SharedRodata, String> {
    let mut rb = RodataBuilder::new();

    // 1. Fixed vector & scalar constants (all 128-bit masks 16-byte aligned and 16 bytes wide!).
    let mut f32_abs_bytes = [0u8; 16];
    let mut f32_neg_bytes = [0u8; 16];
    for lane in 0..4 {
        f32_abs_bytes[lane * 4..lane * 4 + 4].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        f32_neg_bytes[lane * 4..lane * 4 + 4].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    }
    let mut f64_abs_bytes = [0u8; 16];
    let mut f64_neg_bytes = [0u8; 16];
    for lane in 0..2 {
        f64_abs_bytes[lane * 8..lane * 8 + 8]
            .copy_from_slice(&0x7fff_ffff_ffff_ffffu64.to_le_bytes());
        f64_neg_bytes[lane * 8..lane * 8 + 8]
            .copy_from_slice(&0x8000_0000_0000_0000u64.to_le_bytes());
    }
    let f32_abs = rb.push_bytes(16, &f32_abs_bytes)?;
    let f32_neg = rb.push_bytes(16, &f32_neg_bytes)?;
    let f64_abs = rb.push_bytes(16, &f64_abs_bytes)?;
    let f64_neg = rb.push_bytes(16, &f64_neg_bytes)?;
    let f32_qnan = rb.push_u32(0x7fc0_0000)?;
    let f64_qnan = rb.push_u64(0x7ff8_0000_0000_0000)?;
    let mxcsr_wasm = rb.push_u32(0x0000_1f80)?;

    // Exact IEEE-754 truncation boundary constants.
    let f32_neg_one = rb.push_u32((-1.0f32).to_bits())?;
    let f32_neg_2p31 = rb.push_u32((-2147483648.0f32).to_bits())?;
    let f32_2p31 = rb.push_u32((2147483648.0f32).to_bits())?;
    let f32_2p32 = rb.push_u32((4294967296.0f32).to_bits())?;
    let f32_neg_2p63 = rb.push_u32((-9223372036854775808.0f32).to_bits())?;
    let f32_2p63 = rb.push_u32((9223372036854775808.0f32).to_bits())?;
    let f32_2p64 = rb.push_u32((18446744073709551616.0f32).to_bits())?;

    let f64_neg_one = rb.push_u64((-1.0f64).to_bits())?;
    let f64_neg_2p31_m1 = rb.push_u64((-2147483649.0f64).to_bits())?;
    let f64_2p31 = rb.push_u64((2147483648.0f64).to_bits())?;
    let f64_2p32 = rb.push_u64((4294967296.0f64).to_bits())?;
    let f64_neg_2p63 = rb.push_u64((-9223372036854775808.0f64).to_bits())?;
    let f64_2p63 = rb.push_u64((9223372036854775808.0f64).to_bits())?;
    let f64_2p64 = rb.push_u64((18446744073709551616.0f64).to_bits())?;

    let ro = RodataConsts {
        f32_abs,
        f32_neg,
        f64_abs,
        f64_neg,
        f32_qnan,
        f64_qnan,
        mxcsr_wasm,
        f32_neg_one,
        f32_neg_2p31,
        f32_2p31,
        f32_2p32,
        f32_neg_2p63,
        f32_2p63,
        f32_2p64,
        f64_neg_one,
        f64_neg_2p31_m1,
        f64_2p31,
        f64_2p32,
        f64_neg_2p63,
        f64_2p63,
        f64_2p64,
    };

    // Valid non-null `.rodata` sentinel byte for empty strings (no NULL pointers ever returned).
    let empty_str_off = rb.push_bytes(1, &[0u8])?;
    let push_str = |rb: &mut RodataBuilder, s: &str| -> Result<(u32, u32), String> {
        if s.is_empty() {
            Ok((empty_str_off, 0))
        } else {
            let len = u32::try_from(s.len()).map_err(|_| "x64: string exceeds u32")?;
            let off = rb.push_bytes(1, s.as_bytes())?;
            Ok((off, len))
        }
    };

    // 2. Trap messages & 22-entry table: (rel_off: i32, len: u32).
    let mut trap_str_offs = [(0u32, 0u32); TRAP_MESSAGES.len()];
    for (i, msg) in TRAP_MESSAGES.iter().enumerate() {
        trap_str_offs[i] = push_str(&mut rb, msg)?;
    }
    let trap_table_off = rb.align(4)?;
    for &(soff, slen) in &trap_str_offs {
        let rel = i32::try_from(i64::from(soff) - i64::from(trap_table_off))
            .map_err(|_| "x64: trap string rel32 overflow")?;
        rb.push_bytes(1, &rel.to_le_bytes())?;
        rb.push_bytes(1, &slen.to_le_bytes())?;
    }

    // 3. Import strings & table: (mod_rel: i32, mod_len: u32, name_rel: i32, name_len: u32).
    let mut imp_offs = Vec::with_capacity(module.imports.len());
    for (i, (m, n)) in module.imports.iter().enumerate() {
        if (i & 4095) == 4095 {
            budget.check_deadline()?;
        }
        imp_offs.push((push_str(&mut rb, m)?, push_str(&mut rb, n)?));
    }
    let imp_table_off = rb.align(4)?;
    for &((moff, mlen), (noff, nlen)) in &imp_offs {
        let mrel = i32::try_from(i64::from(moff) - i64::from(imp_table_off))
            .map_err(|_| "x64: import mod rel32 overflow")?;
        let nrel = i32::try_from(i64::from(noff) - i64::from(imp_table_off))
            .map_err(|_| "x64: import name rel32 overflow")?;
        rb.push_bytes(1, &mrel.to_le_bytes())?;
        rb.push_bytes(1, &mlen.to_le_bytes())?;
        rb.push_bytes(1, &nrel.to_le_bytes())?;
        rb.push_bytes(1, &nlen.to_le_bytes())?;
    }
    let imp_count =
        u32::try_from(module.imports.len()).map_err(|_| "x64: import count exceeds u32")?;

    // 4. `blot_aot_call_export` table (16 B entries):
    //    +0: name_rel: i32, +4: name_len: u32, +8: fn_rel: i32 (patched), +12: arg_kind: u8, +13: ret_kind: u8
    let mut call_export_items = Vec::new();
    for (i, (name, fi)) in module.exported_funcs.iter().enumerate() {
        if (i & 4095) == 4095 {
            budget.check_deadline()?;
        }
        let Some(fty) = module.func_type(*fi) else {
            continue;
        };
        let (Some(ak), Some(rk)) = (export_val_kind(&fty.params), export_val_kind(&fty.results))
        else {
            continue;
        };
        let (noff, nlen) = push_str(&mut rb, name)?;
        call_export_items.push((noff, nlen, *fi, ak as u8, rk as u8));
    }
    let call_export_table_off = rb.align(4)?;
    let mut call_export_patches = Vec::with_capacity(call_export_items.len());
    for &(noff, nlen, fi, ak, rk) in &call_export_items {
        let nrel = i32::try_from(i64::from(noff) - i64::from(call_export_table_off))
            .map_err(|_| "x64: export name rel32 overflow")?;
        rb.push_bytes(1, &nrel.to_le_bytes())?;
        rb.push_bytes(1, &nlen.to_le_bytes())?;
        let fn_rel_off = rb.push_bytes(1, &0i32.to_le_bytes())?;
        rb.push_bytes(1, &[ak, rk, 0, 0])?;
        call_export_patches.push(CallExportPatch {
            table_rodata_off: call_export_table_off,
            entry_fn_rel_off: fn_rel_off,
            func_idx: fi,
        });
    }
    let call_export_count =
        u32::try_from(call_export_items.len()).map_err(|_| "x64: export count exceeds u32")?;

    // 5. `blot_aot_get_global` / `blot_aot_active_get_global` table (24 B entries, 8-byte aligned):
    //    +0: name_rel: i32, +4: name_len: u32, +8: kind: u8, +9: is_const: u8, +10..12: pad,
    //    +12: ctx_off: i32, +16: const_bits: u64
    let mut get_global_items = Vec::new();
    let mut set_global_items = Vec::new();
    for (i, (name, gi, vt, mutable)) in module.exported_globals.iter().enumerate() {
        if (i & 4095) == 4095 {
            budget.check_deadline()?;
        }
        let Some(kind) = global_kind_code(*vt) else {
            continue; // V128 excluded from scalar/ref global API
        };
        let (noff, nlen) = push_str(&mut rb, name)?;
        match module.global_consts[*gi] {
            Some(c) => {
                // Store I32/F32 constants 32-bit zero-extended (never sign-extended for negative I32!).
                let bits: u64 = match c {
                    X64ConstVal::I32(v) => u64::from(v as u32),
                    X64ConstVal::I64(v) => v as u64,
                    X64ConstVal::F32(b) => u64::from(b),
                    X64ConstVal::F64(b) => b,
                    X64ConstVal::RefNull => 0,
                    X64ConstVal::V128(_) => continue,
                };
                get_global_items.push((noff, nlen, kind, 1u8, 0i32, bits));
            }
            None => {
                let ctx_off = layout_global_offs[*gi];
                get_global_items.push((noff, nlen, kind, 0u8, ctx_off, 0u64));
                if *mutable {
                    let is64 = u8::from(!matches!(vt, ValType::I32 | ValType::F32));
                    set_global_items.push((noff, nlen, ctx_off, is64));
                }
            }
        }
    }
    let get_global_table_off = rb.align(8)?;
    for &(noff, nlen, kind, is_const, ctx_off, const_bits) in &get_global_items {
        let nrel = i32::try_from(i64::from(noff) - i64::from(get_global_table_off))
            .map_err(|_| "x64: get_global name rel32 overflow")?;
        rb.push_bytes(1, &nrel.to_le_bytes())?;
        rb.push_bytes(1, &nlen.to_le_bytes())?;
        rb.push_bytes(1, &[kind, is_const, 0, 0])?;
        rb.push_bytes(1, &ctx_off.to_le_bytes())?;
        rb.push_bytes(1, &const_bits.to_le_bytes())?;
    }
    let get_global_count = u32::try_from(get_global_items.len())
        .map_err(|_| "x64: global export count exceeds u32")?;

    // 6. `blot_aot_set_global` table (16 B entries):
    //    +0: name_rel: i32, +4: name_len: u32, +8: ctx_off: i32, +12: is64: u8
    let set_global_table_off = rb.align(4)?;
    for &(noff, nlen, ctx_off, is64) in &set_global_items {
        let nrel = i32::try_from(i64::from(noff) - i64::from(set_global_table_off))
            .map_err(|_| "x64: set_global name rel32 overflow")?;
        rb.push_bytes(1, &nrel.to_le_bytes())?;
        rb.push_bytes(1, &nlen.to_le_bytes())?;
        rb.push_bytes(1, &ctx_off.to_le_bytes())?;
        rb.push_bytes(1, &[is64, 0, 0, 0])?;
    }
    let set_global_count = u32::try_from(set_global_items.len())
        .map_err(|_| "x64: mutable global export count exceeds u32")?;

    // 7. Non-zero V128 global initializers (16-byte aligned).
    let mut v128_global_inits = Vec::new();
    for (k, &init) in module.global_inits.iter().enumerate() {
        if module.global_consts[k].is_none()
            && let X64ConstVal::V128(bytes) = init
            && bytes != [0u8; 16]
        {
            let off = rb.push_bytes(16, &bytes)?;
            v128_global_inits.push((k, off));
        }
    }

    // 8. Active data segment payloads (`WT_DATA`), counted in `MAX_TOTAL_RODATA_BYTES`.
    let mut data_segs = Vec::with_capacity(module.active_data_segs.len());
    for (i, &(mem_off, di)) in module.active_data_segs.iter().enumerate() {
        if (i & 1023) == 1023 {
            budget.check_deadline()?;
        }
        let seg = &decoded.data[di];
        let payload = &decoded.bytes[seg.bytes.start..seg.bytes.end];
        let len = u32::try_from(payload.len()).map_err(|_| "x64: data segment exceeds u32")?;
        let ro_off = if payload.is_empty() {
            empty_str_off
        } else {
            rb.push_bytes(1, payload)?
        };
        data_segs.push((mem_off, ro_off, len));
    }

    // 9. `call_indirect` dispatch tables (4-byte aligned, `span * 4` zero bytes, patched at link time).
    let mut ci_tables = Vec::with_capacity(module.ci_sigs.len());
    for ci in &module.ci_sigs {
        let bytes = (ci.span as usize)
            .checked_mul(4)
            .ok_or("x64: call_indirect table size overflow")?;
        let ro_off = if bytes == 0 {
            0
        } else {
            rb.push_zeros(4, bytes)?
        };
        ci_tables.push(CiTableLayout {
            table_len: ci.table_len,
            min_slot: ci.min_slot,
            span: ci.span,
            rodata_off: ro_off,
        });
    }

    budget.charge_rodata(rb.buf.len())?;

    let tables = RuntimeTables {
        trap_table_off,
        imp_table_off,
        imp_count,
        call_export_table_off,
        call_export_count,
        get_global_table_off,
        get_global_count,
        set_global_table_off,
        set_global_count,
        data_segs,
        v128_global_inits,
    };

    Ok(SharedRodata {
        rodata: rb.buf,
        consts: ro,
        tables,
        ci_tables,
        call_export_patches,
    })
}

struct LinkContext<'a> {
    tx_vaddr: u64,
    ro_vaddr: u64,
    func_text_offs: &'a [u64],
    helper_text_offs: &'a [u64],
}

fn patch_relocs(
    text: &mut [u8],
    item_text_off: u64,
    relocs: &[Reloc],
    local_rodata_off: Option<u64>,
    ctx: &LinkContext<'_>,
) -> Result<(), String> {
    let item_vaddr = ctx
        .tx_vaddr
        .checked_add(item_text_off)
        .ok_or("x64: item vaddr overflow")?;
    for r in relocs {
        let target_vaddr: u64 = match r.target {
            RelocTarget::Func(fi) => {
                let off = *ctx
                    .func_text_offs
                    .get(fi as usize)
                    .ok_or_else(|| format!("x64: reloc Func({fi}) out of range"))?;
                ctx.tx_vaddr
                    .checked_add(off)
                    .ok_or("x64: Func target vaddr overflow")?
            }
            RelocTarget::Helper(hi) => {
                let off = *ctx
                    .helper_text_offs
                    .get(hi as usize)
                    .ok_or_else(|| format!("x64: reloc Helper({hi}) out of range"))?;
                ctx.tx_vaddr
                    .checked_add(off)
                    .ok_or("x64: Helper target vaddr overflow")?
            }
            RelocTarget::Rodata(off) => ctx
                .ro_vaddr
                .checked_add(u64::from(off))
                .ok_or("x64: Rodata target vaddr overflow")?,
            RelocTarget::LocalRodata(off) => {
                let base =
                    local_rodata_off.ok_or("x64: LocalRodata reloc without local rodata base")?;
                ctx.ro_vaddr
                    .checked_add(base)
                    .and_then(|v| v.checked_add(u64::from(off)))
                    .ok_or("x64: LocalRodata target vaddr overflow")?
            }
        };
        let next_ip_vaddr = item_vaddr
            .checked_add(u64::from(r.next_ip_off))
            .ok_or("x64: reloc next_ip vaddr overflow")?;
        let rel = i128::from(target_vaddr) - i128::from(next_ip_vaddr);
        let rel32 = i32::try_from(rel)
            .map_err(|_| "x64: rel32 relocation out of 2 GiB range".to_string())?;
        let p = (item_text_off as usize)
            .checked_add(r.patch_off as usize)
            .ok_or("x64: reloc patch offset overflow")?;
        let end = p.checked_add(4).ok_or("x64: reloc patch end overflow")?;
        if end > text.len() {
            return Err(format!(
                "x64: reloc patch extent {p}..{end} exceeds .text len {}",
                text.len()
            ));
        }
        text[p..end].copy_from_slice(&rel32.to_le_bytes());
    }
    Ok(())
}

/// Compiles Wasm module bytes directly into a self-contained Linux x86-64 ELF64 `.so` image.
///
/// Catches unexpected internal panics at the compiler boundary and converts them into
/// `X64CompileError::Failed` so callers always observe a typed error instead of process abort.
pub fn compile_wasm_to_x64_so(
    wasm: &[u8],
    opts: &X64CompileOptions,
) -> Result<Vec<u8>, X64CompileError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_wasm_to_x64_so_inner(wasm, opts)
    })) {
        Ok(res) => res,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unexpected compiler panic".to_string()
            };
            Err(X64CompileError::Failed(format!(
                "x64 internal compiler error: {msg}"
            )))
        }
    }
}

fn compile_wasm_to_x64_so_inner(
    wasm: &[u8],
    opts: &X64CompileOptions,
) -> Result<Vec<u8>, X64CompileError> {
    let t_total = Instant::now();
    let budget = CompileBudget::new(opts.deadline);
    let check_dl = || budget.check_deadline().map_err(X64CompileError::Failed);
    check_dl()?;

    // 1. Validate Wasm under Stage 1's strict feature profile and resource limits.
    let t0 = Instant::now();
    let validated =
        wtime_validate::validate(wasm, &x64_validate_features(), &x64_validate_limits()).map_err(
            |e| match e.kind {
                ErrorKind::DisabledFeature | ErrorKind::ResourceLimit => {
                    X64CompileError::AdmissionRejected(format!("x64: {e}"))
                }
                _ => X64CompileError::Failed(format!("invalid wasm: {e}")),
            },
        )?;
    let decoded = validated.decoded();
    let d_validate = t0.elapsed();
    check_dl()?;

    // 2. Enforce Stage 1 admission contract & build lean X64Module metadata.
    let t0 = Instant::now();
    let module = admit_x64_module_with_budget(decoded, Some(&budget)).map_err(|e| {
        if budget.check_deadline().is_err() {
            X64CompileError::Failed(e)
        } else {
            X64CompileError::AdmissionRejected(e)
        }
    })?;
    let d_admit = t0.elapsed();
    check_dl()?;

    // 3. Compute Ctx global/extern-table offsets first, then build shared `.rodata` + `ci_tables`,
    //    then finalize `CtxLayout` with `ci_tables`.
    let preliminary_layout =
        CtxLayout::compute(&module, Vec::new()).map_err(X64CompileError::AdmissionRejected)?;
    let SharedRodata {
        mut rodata,
        consts: ro,
        tables,
        ci_tables,
        call_export_patches,
    } = build_shared_rodata(decoded, &module, &preliminary_layout.global_offs, &budget).map_err(
        |e| {
            if budget.check_deadline().is_err() {
                X64CompileError::Failed(e)
            } else {
                X64CompileError::AdmissionRejected(e)
            }
        },
    )?;
    let layout = CtxLayout {
        ci_tables,
        ..preliminary_layout
    };

    // 4. Emit shared runtime helpers, `wt_import{k}` trampolines, and all 18 C ABI exports.
    let export_asms = emit_all_exports(&module, &layout, &ro, &tables, opts.isa, opts.hooks)
        .map_err(X64CompileError::Failed)?;
    let helper_asms = emit_all_helpers(&module, &layout, &ro, &tables, opts.hooks)
        .map_err(X64CompileError::Failed)?;
    let mut import_asms = Vec::with_capacity(module.imports.len());
    for k in 0..module.imports.len() {
        import_asms.push(emit_import_trampoline(&module, k).map_err(X64CompileError::Failed)?);
    }
    let runtime_code_bytes: usize = export_asms
        .iter()
        .chain(&helper_asms)
        .chain(&import_asms)
        .map(|a| a.code.len())
        .sum();
    budget
        .charge_code(runtime_code_bytes)
        .map_err(X64CompileError::Failed)?;
    check_dl()?;

    // 5. Compile all defined functions through the Stage 2 multi-phase pipeline (parallel IR lower ->
    //    module-wide inlining + effect analysis -> parallel Stage 2 IR opt + regalloc ->
    //    coupled Step 8a/8b stack & trap-check elision -> parallel x64 codegen) into an indexed
    //    `Vec<Option<Asm>>` for 100% deterministic `.text`/`.rodata` layout.
    let n_imports = module.imports.len();
    let n_defs = decoded.code.len();
    let n_workers = resolve_jobs(opts.jobs).min(n_defs.max(1));
    let ns_ssa_opt = AtomicU64::new(0);
    let ns_regalloc = AtomicU64::new(0);
    let ns_emit = AtomicU64::new(0);
    let mut d_inline_effects = std::time::Duration::ZERO;
    let mut d_elide = std::time::Duration::ZERO;

    let mut compiled: Vec<Option<Asm>> = (0..n_defs).map(|_| None).collect();

    let map_worker_err = |err: String| -> X64CompileError {
        if budget.check_deadline().is_ok()
            && (err.contains("exceeds cap") || err.contains("exceeds frame cap"))
        {
            X64CompileError::AdmissionRejected(err)
        } else {
            X64CompileError::Failed(err)
        }
    };

    if n_defs > 0 {
        // Phase 5a (parallel): Wasm -> initial SSA IR for all defined functions.
        let next_idx = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        let worker_irs: Mutex<Vec<Vec<(usize, IrFunc)>>> =
            Mutex::new(Vec::with_capacity(n_workers));
        std::thread::scope(|s| {
            for _ in 0..n_workers {
                s.spawn(|| {
                    let mut local_out: Vec<(usize, IrFunc)> = Vec::new();
                    let mut loc_ssa = 0u64;
                    loop {
                        if failed.load(Ordering::Acquire) {
                            break;
                        }
                        let bi = next_idx.fetch_add(1, Ordering::Relaxed);
                        if bi >= n_defs {
                            break;
                        }
                        let func_index = n_imports + bi;
                        let ts = Instant::now();
                        let res = lower_wasm_to_ir_with_budget(
                            decoded,
                            &module,
                            func_index,
                            opts.opt,
                            Some(&budget),
                        )
                        .and_then(|mut ir| {
                            if opts.opt {
                                opt::prune_early_block_params(&mut ir, Some(&budget))?;
                            }
                            Ok(ir)
                        });
                        loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                        match res {
                            Ok(ir) => local_out.push((bi, ir)),
                            Err(e) => {
                                failed.store(true, Ordering::Release);
                                let mut guard = first_err.lock().unwrap_or_else(|g| g.into_inner());
                                if guard.as_ref().is_none_or(|(prev_bi, _)| bi < *prev_bi) {
                                    *guard = Some((bi, e));
                                }
                                break;
                            }
                        }
                    }
                    ns_ssa_opt.fetch_add(loc_ssa, Ordering::Relaxed);
                    worker_irs
                        .lock()
                        .unwrap_or_else(|g| g.into_inner())
                        .push(local_out);
                });
            }
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        let mut ir_slots: Vec<Option<IrFunc>> = (0..n_defs).map(|_| None).collect();
        for batch in worker_irs.into_inner().unwrap_or_else(|g| g.into_inner()) {
            for (bi, ir) in batch {
                ir_slots[bi] = Some(ir);
            }
        }
        let mut irs: Vec<IrFunc> = ir_slots
            .into_iter()
            .enumerate()
            .map(|(bi, o)| {
                o.ok_or_else(|| X64CompileError::Failed(format!("x64: missing IR {bi}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        check_dl()?;

        // Phase 5b (module-wide, only when `opts.opt`): budgeted small-leaf inlining + interprocedural effects.
        let effects = if opts.opt {
            let ti = Instant::now();
            opt::inline_small_helpers(&mut irs, &module, Some(&budget)).map_err(map_worker_err)?;
            let eff = opt::compute_module_effects(&irs, &module, Some(&budget))
                .map_err(map_worker_err)?;
            d_inline_effects = ti.elapsed();
            Some(eff)
        } else {
            None
        };
        check_dl()?;

        // Phase 5c (parallel): per-function Stage 2 IR opt (when `opts.opt`), optional `verify_ir`,
        // register allocation, and optional `verify_alloc`.
        let alloc_cfg = AllocConfig {
            allow_r12: layout.pinned_global.is_none(),
            allow_r13: layout.fixed_mem_bytes.is_some(),
        };
        let work_queue: Mutex<Vec<(usize, IrFunc)>> =
            Mutex::new(irs.into_iter().enumerate().collect());
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        let worker_allocs: Mutex<Vec<Vec<(usize, IrFunc, AllocResult)>>> =
            Mutex::new(Vec::with_capacity(n_workers));
        std::thread::scope(|s| {
            for _ in 0..n_workers {
                s.spawn(|| {
                    let mut local_out: Vec<(usize, IrFunc, AllocResult)> = Vec::new();
                    let mut loc_ssa = 0u64;
                    let mut loc_ra = 0u64;
                    loop {
                        if failed.load(Ordering::Acquire) {
                            break;
                        }
                        let item = work_queue.lock().unwrap_or_else(|g| g.into_inner()).pop();
                        let Some((bi, mut ir)) = item else {
                            break;
                        };
                        let step_res: Result<AllocResult, String> = (|| {
                            if let Some(ref eff) = effects {
                                let ts = Instant::now();
                                opt::optimize_func_stage2(&mut ir, &module, eff, Some(&budget))?;
                                loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                            }
                            if opts.verify {
                                verify_ir(&ir, &module)?;
                            }
                            let tr = Instant::now();
                            let alloc = allocate_with_config(&ir, alloc_cfg, Some(&budget))?;
                            loc_ra = loc_ra.saturating_add(tr.elapsed().as_nanos() as u64);
                            if opts.verify {
                                verify_alloc_with_config(&ir, &alloc, alloc_cfg)?;
                            }
                            Ok(alloc)
                        })();
                        match step_res {
                            Ok(alloc) => local_out.push((bi, ir, alloc)),
                            Err(e) => {
                                failed.store(true, Ordering::Release);
                                let mut guard = first_err.lock().unwrap_or_else(|g| g.into_inner());
                                if guard.as_ref().is_none_or(|(prev_bi, _)| bi < *prev_bi) {
                                    *guard = Some((bi, e));
                                }
                                break;
                            }
                        }
                    }
                    ns_ssa_opt.fetch_add(loc_ssa, Ordering::Relaxed);
                    ns_regalloc.fetch_add(loc_ra, Ordering::Relaxed);
                    worker_allocs
                        .lock()
                        .unwrap_or_else(|g| g.into_inner())
                        .push(local_out);
                });
            }
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        let mut ir_slots: Vec<Option<IrFunc>> = (0..n_defs).map(|_| None).collect();
        let mut alloc_slots: Vec<Option<AllocResult>> = (0..n_defs).map(|_| None).collect();
        for batch in worker_allocs
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
        {
            for (bi, ir, alloc) in batch {
                ir_slots[bi] = Some(ir);
                alloc_slots[bi] = Some(alloc);
            }
        }
        let mut irs: Vec<IrFunc> = ir_slots
            .into_iter()
            .enumerate()
            .map(|(bi, o)| {
                o.ok_or_else(|| X64CompileError::Failed(format!("x64: missing opt IR {bi}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let allocs: Vec<AllocResult> = alloc_slots
            .into_iter()
            .enumerate()
            .map(|(bi, o)| {
                o.ok_or_else(|| X64CompileError::Failed(format!("x64: missing alloc {bi}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        check_dl()?;

        // Phase 5d (module-wide, only when `opts.opt`): coupled Step 8a stack-check elision -> Step 8b trap-check elision.
        let fn_can_trap: Option<Vec<bool>> = if opts.opt {
            let te = Instant::now();
            let res = opt::run_stack_and_trap_elision(&mut irs, &allocs, &module, Some(&budget))
                .map_err(map_worker_err)?;
            d_elide = te.elapsed();
            Some(res)
        } else {
            None
        };
        check_dl()?;

        // Phase 5e (parallel): x86-64 machine code lowering.
        let next_idx = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        let worker_results: Mutex<Vec<Vec<(usize, Asm)>>> =
            Mutex::new(Vec::with_capacity(n_workers));
        let fn_can_trap_ref = fn_can_trap.as_deref();
        std::thread::scope(|s| {
            for _ in 0..n_workers {
                s.spawn(|| {
                    let mut local_out: Vec<(usize, Asm)> = Vec::new();
                    let mut loc_em = 0u64;
                    loop {
                        if failed.load(Ordering::Acquire) {
                            break;
                        }
                        let bi = next_idx.fetch_add(1, Ordering::Relaxed);
                        if bi >= n_defs {
                            break;
                        }
                        let te = Instant::now();
                        let res = lower_ir_to_x64_with_budget(
                            &irs[bi],
                            &allocs[bi],
                            &layout,
                            &ro,
                            opts.isa,
                            fn_can_trap_ref,
                            Some(&budget),
                        );
                        loc_em = loc_em.saturating_add(te.elapsed().as_nanos() as u64);
                        match res {
                            Ok(asm) => local_out.push((bi, asm)),
                            Err(e) => {
                                failed.store(true, Ordering::Release);
                                let mut guard = first_err.lock().unwrap_or_else(|g| g.into_inner());
                                if guard.as_ref().is_none_or(|(prev_bi, _)| bi < *prev_bi) {
                                    *guard = Some((bi, e));
                                }
                                break;
                            }
                        }
                    }
                    ns_emit.fetch_add(loc_em, Ordering::Relaxed);
                    worker_results
                        .lock()
                        .unwrap_or_else(|g| g.into_inner())
                        .push(local_out);
                });
            }
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        for batch in worker_results
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
        {
            for (bi, asm) in batch {
                compiled[bi] = Some(asm);
            }
        }
    }
    check_dl()?;

    // 6. Deterministic merge & ELF64 linking.
    let t_link = Instant::now();
    let mut def_asms: Vec<Asm> = Vec::with_capacity(n_defs);
    for (bi, item) in compiled.into_iter().enumerate() {
        let asm = item
            .ok_or_else(|| X64CompileError::Failed(format!("x64: missing compiled body {bi}")))?;
        def_asms.push(asm);
    }

    // 6a. Precompute exact merged `.rodata` offsets for every function's `local_rodata` (16-byte aligned).
    let mut planned_rodata_len = rodata.len();
    let mut fn_local_rodata_offs: Vec<u64> = Vec::with_capacity(n_defs);
    for (bi, asm) in def_asms.iter().enumerate() {
        if (bi & 1023) == 1023 {
            check_dl()?;
        }
        if asm.local_rodata.is_empty() {
            fn_local_rodata_offs.push(0);
        } else {
            let aligned =
                align_up_checked(planned_rodata_len, 16).map_err(X64CompileError::Failed)?;
            let next = aligned.checked_add(asm.local_rodata.len()).ok_or_else(|| {
                X64CompileError::Failed("x64: merged .rodata size overflow".into())
            })?;
            if next > MAX_TOTAL_RODATA_BYTES {
                return Err(X64CompileError::Failed(format!(
                    "x64: complete merged .rodata ({next} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
                )));
            }
            fn_local_rodata_offs.push(aligned as u64);
            planned_rodata_len = next;
        }
    }

    // 6b. Merge `.text` (16-byte aligned per item) in deterministic order:
    //     1. 18 C ABI exports, 2. shared helpers, 3. import trampolines, 4. defined functions.
    let mut text: Vec<u8> = Vec::with_capacity(
        runtime_code_bytes
            .saturating_add(n_defs * 128)
            .min(64 << 20),
    );
    let push_text = |text: &mut Vec<u8>, bytes: &[u8]| -> Result<u64, X64CompileError> {
        let aligned = align_up_checked(text.len(), 16).map_err(X64CompileError::Failed)?;
        let next = aligned
            .checked_add(bytes.len())
            .ok_or_else(|| X64CompileError::Failed("x64: merged .text size overflow".into()))?;
        if next > MAX_TOTAL_CODE_BYTES {
            return Err(X64CompileError::Failed(format!(
                "x64: complete merged .text ({next} B) exceeds cap {MAX_TOTAL_CODE_BYTES} B"
            )));
        }
        // Pad inter-function gaps with INT3 (0xcc) for clean trap behavior if misjumped.
        text.resize(aligned, 0xcc);
        text.extend_from_slice(bytes);
        Ok(aligned as u64)
    };

    let mut export_defs: Vec<ExportDef<'_>> = Vec::with_capacity(EXPORT_NAMES.len());
    let mut export_text_offs: Vec<u64> = Vec::with_capacity(EXPORT_NAMES.len());
    for (i, asm) in export_asms.iter().enumerate() {
        let off = push_text(&mut text, &asm.code)?;
        export_text_offs.push(off);
        export_defs.push(ExportDef {
            name: EXPORT_NAMES[i],
            text_off: off,
            size: asm.code.len() as u64,
        });
    }

    let mut helper_text_offs: Vec<u64> = Vec::with_capacity(helper_asms.len());
    for asm in &helper_asms {
        helper_text_offs.push(push_text(&mut text, &asm.code)?);
    }

    let mut func_text_offs: Vec<u64> = Vec::with_capacity(n_imports + n_defs);
    for asm in &import_asms {
        func_text_offs.push(push_text(&mut text, &asm.code)?);
    }
    for (bi, asm) in def_asms.iter().enumerate() {
        if (bi & 1023) == 1023 {
            check_dl()?;
        }
        func_text_offs.push(push_text(&mut text, &asm.code)?);
    }

    // 6c. Plan ELF64 layout (verifying `.text` start < i32::MAX).
    let elf_layout =
        ElfLayout::plan(&EXPORT_NAMES, planned_rodata_len).map_err(X64CompileError::Failed)?;
    let ro_vaddr = elf_layout.rodata_vaddr as u64;
    let tx_vaddr = elf_layout.text_vaddr as u64;

    // 6d. Patch all jump tables in `def_asms` and append their `local_rodata` into `rodata`.
    rodata.reserve_exact(planned_rodata_len.saturating_sub(rodata.len()));
    for (bi, asm) in def_asms.iter_mut().enumerate() {
        if (bi & 1023) == 1023 {
            check_dl()?;
        }
        if !asm.local_rodata.is_empty() {
            let fn_vaddr = tx_vaddr
                .checked_add(func_text_offs[n_imports + bi])
                .ok_or_else(|| X64CompileError::Failed("x64: fn_vaddr overflow".into()))?;
            let lro_vaddr = ro_vaddr
                .checked_add(fn_local_rodata_offs[bi])
                .ok_or_else(|| {
                    X64CompileError::Failed("x64: local_rodata vaddr overflow".into())
                })?;
            asm.patch_jump_tables(fn_vaddr, lro_vaddr)
                .map_err(X64CompileError::Failed)?;
            let target_off = fn_local_rodata_offs[bi] as usize;
            debug_assert!(target_off >= rodata.len());
            rodata.resize(target_off, 0);
            rodata.extend_from_slice(&asm.local_rodata);
        }
    }
    debug_assert_eq!(rodata.len(), planned_rodata_len);

    // 6e. Patch `call_indirect` dispatch tables (`target_vaddr - table_base_vaddr`).
    for (ci_sig, ci_lay) in module.ci_sigs.iter().zip(&layout.ci_tables) {
        if ci_lay.span == 0 {
            continue;
        }
        let table_base_vaddr = ro_vaddr
            .checked_add(u64::from(ci_lay.rodata_off))
            .ok_or_else(|| X64CompileError::Failed("x64: ci table vaddr overflow".into()))?;
        for &(slot, fi) in &ci_sig.targets {
            let rel_slot = slot
                .checked_sub(ci_lay.min_slot)
                .ok_or_else(|| X64CompileError::Failed("x64: ci slot underflow".into()))?;
            let entry_off = (ci_lay.rodata_off as usize)
                .checked_add((rel_slot as usize) * 4)
                .ok_or_else(|| X64CompileError::Failed("x64: ci entry offset overflow".into()))?;
            let target_off = *func_text_offs.get(fi as usize).ok_or_else(|| {
                X64CompileError::Failed(format!("x64: ci target func {fi} out of range"))
            })?;
            let target_vaddr = tx_vaddr
                .checked_add(target_off)
                .ok_or_else(|| X64CompileError::Failed("x64: ci target vaddr overflow".into()))?;
            let disp = i32::try_from(i128::from(target_vaddr) - i128::from(table_base_vaddr))
                .map_err(|_| X64CompileError::Failed("x64: ci target rel32 overflow".into()))?;
            debug_assert_ne!(
                disp, 0,
                "call_indirect target disp cannot collide with 0 sentinel"
            );
            rodata[entry_off..entry_off + 4].copy_from_slice(&disp.to_le_bytes());
        }
    }

    // 6f. Patch `call_export_table` function displacements (`target_vaddr - table_base_vaddr`).
    for p in &call_export_patches {
        let table_base_vaddr = ro_vaddr
            .checked_add(u64::from(p.table_rodata_off))
            .ok_or_else(|| X64CompileError::Failed("x64: export table vaddr overflow".into()))?;
        let target_off = *func_text_offs.get(p.func_idx).ok_or_else(|| {
            X64CompileError::Failed(format!(
                "x64: export target func {} out of range",
                p.func_idx
            ))
        })?;
        let target_vaddr = tx_vaddr
            .checked_add(target_off)
            .ok_or_else(|| X64CompileError::Failed("x64: export target vaddr overflow".into()))?;
        let disp = i32::try_from(i128::from(target_vaddr) - i128::from(table_base_vaddr))
            .map_err(|_| X64CompileError::Failed("x64: export target rel32 overflow".into()))?;
        let off = p.entry_fn_rel_off as usize;
        rodata[off..off + 4].copy_from_slice(&disp.to_le_bytes());
    }

    // 6g. Patch all instruction relocations across exports, helpers, import trampolines, and defined functions.
    let link_ctx = LinkContext {
        tx_vaddr,
        ro_vaddr,
        func_text_offs: &func_text_offs,
        helper_text_offs: &helper_text_offs,
    };
    for (i, asm) in export_asms.iter().enumerate() {
        patch_relocs(&mut text, export_text_offs[i], &asm.relocs, None, &link_ctx)
            .map_err(X64CompileError::Failed)?;
    }
    for (i, asm) in helper_asms.iter().enumerate() {
        patch_relocs(&mut text, helper_text_offs[i], &asm.relocs, None, &link_ctx)
            .map_err(X64CompileError::Failed)?;
    }
    for (k, asm) in import_asms.iter().enumerate() {
        patch_relocs(&mut text, func_text_offs[k], &asm.relocs, None, &link_ctx)
            .map_err(X64CompileError::Failed)?;
    }
    for (bi, asm) in def_asms.iter().enumerate() {
        if (bi & 1023) == 1023 {
            check_dl()?;
        }
        let lro = (!asm.local_rodata.is_empty()).then_some(fn_local_rodata_offs[bi]);
        patch_relocs(
            &mut text,
            func_text_offs[n_imports + bi],
            &asm.relocs,
            lro,
            &link_ctx,
        )
        .map_err(X64CompileError::Failed)?;
    }
    check_dl()?;

    // 6h. Emit ELF64 shared library image.
    let img = elf_layout
        .emit(&rodata, &text, &export_defs)
        .map_err(X64CompileError::Failed)?;
    let d_link = t_link.elapsed();

    if std::env::var_os("WTIME_AOT_TIMINGS").is_some_and(|v| v == "1") {
        let total_ms = t_total.elapsed().as_secs_f64() * 1000.0;
        let ssa_ms = (ns_ssa_opt.load(Ordering::Relaxed) as f64) / 1e6;
        let ra_ms = (ns_regalloc.load(Ordering::Relaxed) as f64) / 1e6;
        let em_ms = (ns_emit.load(Ordering::Relaxed) as f64) / 1e6;
        eprintln!(
            "wtime-aot x64 timings: validate={:.2}ms admit={:.2}ms ssa_opt={:.2}ms(cpu) inline_eff={:.2}ms regalloc={:.2}ms(cpu) elide={:.2}ms emit={:.2}ms(cpu) link={:.2}ms total={:.2}ms (fns={} workers={} text={}B rodata={}B)",
            d_validate.as_secs_f64() * 1000.0,
            d_admit.as_secs_f64() * 1000.0,
            ssa_ms,
            d_inline_effects.as_secs_f64() * 1000.0,
            ra_ms,
            d_elide.as_secs_f64() * 1000.0,
            em_ms,
            d_link.as_secs_f64() * 1000.0,
            total_ms,
            n_defs,
            n_workers,
            text.len(),
            rodata.len(),
        );
    }

    Ok(img)
}
