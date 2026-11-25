//! Self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm -> Linux x86-64 LP64
//! ELF64 `.so` compiler (`wtime-aot --backend x64`).

pub mod asm;
pub mod elf;
pub mod inc_cache;
pub mod ir;
pub mod lower;
pub mod module;
pub mod opt;
pub mod regalloc;
pub mod runtime;
pub mod simd_lower;
pub mod standalone_rt;
#[cfg(test)]
mod tests_abi_faults;
#[cfg(test)]
mod tests_oracle;
#[cfg(test)]
pub(crate) mod tests_unit;
pub mod verify;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use wtime_types::{ErrorKind, ValType};
use wtime_validate::DecodedModule;

use asm::{Asm, Reloc, RelocTarget};
use elf::{ElfExecLayout, ElfLayout, ExportDef};
use ir::{IrFunc, OptIrScratch, lower_wasm_to_ir_with_budget_and_scratch};
use lower::{TargetIsa, lower_ir_to_x64_with_budget};
use module::{
    CompileBudget, MAX_CTX_BYTES, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES, X64ConstVal,
    X64Module, X64Target, x64_validate_features, x64_validate_limits,
};
use regalloc::{AllocConfig, AllocResult, RegallocScratch, allocate_with_config_and_scratch};
use runtime::{
    EXPORT_NAMES, RuntimeTables, RuntimeTestHooks, TRAP_MESSAGES, emit_all_exports,
    emit_all_helpers, emit_import_trampoline, export_val_kind, global_kind_code,
};
use standalone_rt::{emit_standalone_entry, emit_standalone_import_trampoline};
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
/// 16-byte aligned intra-instruction SIMD scratch slot inside `Ctx`.
pub const OFF_SCRATCH_V128: i32 = 128;
/// Standalone executable runtime fields inside `Ctx` (unused by Blot cdylib).
pub const OFF_EXEC_ARGC: i32 = 144;
pub const OFF_EXEC_ARGV: i32 = 152;
pub const OFF_EXEC_CLOCK_ORIGIN: i32 = 160;
pub const OFF_EXEC_EXIT_CODE: i32 = 168;
pub const OFF_EXEC_TIMING_PATH: i32 = 176;
pub const OFF_EXEC_MEM_LIMIT: i32 = 184;
pub const CTX_HEADER_BYTES: usize = 192;

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
pub(crate) const HELPER_GOOSE_FMT_FLOAT: u32 = 23;
pub(crate) const NUM_HELPERS: usize = 24;

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
    /// `true` iff linear memory 0 uses 64-bit (`memory64`) addressing.
    pub memory64: bool,
    /// `true` iff standalone `ET_EXEC` reserves an `8 GiB + 64 KiB` `PROT_NONE` guard region with
    /// a `SIGSEGV` handler for `memory32` (NEVER enabled for `--blot-cdylib` `.so`).
    pub use_guard_pages: bool,
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
            memory64: module.memory64,
            use_guard_pages: module.target == X64Target::StandaloneExec
                && !module.memory64
                && module.memory.is_some(),
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
#[derive(Clone, Copy, Debug, Default)]
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
#[derive(Clone, Debug)]
pub struct X64CompileOptions {
    pub target: X64Target,
    pub isa: TargetIsa,
    /// Worker count: `0` = auto (`available_parallelism().min(64)`), explicit clamped `1..=64`.
    pub jobs: usize,
    pub deadline: Option<Instant>,
    /// Enable SSA IR optimizations (`false` exercises `lower_wasm_to_ir_unopt` for differential tests).
    pub opt: bool,
    /// Run `verify_ir` and `verify_alloc` on every defined function during compilation.
    pub verify: bool,
    /// Optional private cache directory enabling content-addressed per-function incremental reuse.
    pub inc_cache_dir: Option<PathBuf>,
    /// Cryptographic salt (e.g. compiler binary SHA-256) bound into incremental cache keys.
    pub cache_key_salt: Option<[u8; 32]>,
    pub hooks: RuntimeTestHooks,
}

impl Default for X64CompileOptions {
    fn default() -> Self {
        Self {
            target: X64Target::BlotCdylib,
            isa: TargetIsa {
                has_abm_bmi1: false,
                has_bmi2: false,
                has_avx2: false,
            },
            jobs: 0,
            deadline: None,
            opt: true,
            verify: true,
            inc_cache_dir: None,
            cache_key_salt: None,
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
                if std::is_x86_feature_detected!("lzcnt")
                    && std::is_x86_feature_detected!("bmi1")
                    && std::is_x86_feature_detected!("bmi2")
                    && std::is_x86_feature_detected!("avx2")
                {
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
                has_bmi2: tag == "x64-v3",
                has_avx2: tag == "x64-v3",
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

    // 8. `call_indirect` dispatch tables (placed BEFORE data segments so edits to data segment
    //    lengths do not shift `ci_tables` offsets or invalidate incremental function caches).
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

    // 9. Active data segment payloads (`WT_DATA`), counted in `MAX_TOTAL_RODATA_BYTES`.
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

/// Completion handle for a best-effort background incremental cache write started before linking.
pub(crate) type PendingIncCacheSave = std::sync::Arc<(Mutex<bool>, std::sync::Condvar)>;

/// Best-effort bounded wait for a background incremental cache write.
pub(crate) fn wait_pending_inc_cache_save(done: PendingIncCacheSave, deadline: Option<Instant>) {
    let (lk, cv) = &*done;
    let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
    if !*guard {
        match deadline {
            None => {
                drop(cv.wait_while(guard, |d| !*d));
            }
            Some(dl) => {
                let rem = dl.saturating_duration_since(Instant::now());
                let wait = rem
                    .saturating_sub(std::time::Duration::from_millis(150))
                    .min(std::time::Duration::from_millis(250));
                if !wait.is_zero() {
                    drop(cv.wait_timeout_while(guard, wait, |d| !*d));
                }
            }
        }
    }
}

/// Compiles Wasm module bytes directly into a self-contained Linux x86-64 ELF64 `.so` image.
///
/// Catches unexpected internal panics at the compiler boundary and converts them into
/// `X64CompileError::Failed` so callers always observe a typed error instead of process abort.
pub fn compile_wasm_to_x64_so(
    wasm: &[u8],
    opts: &X64CompileOptions,
) -> Result<Vec<u8>, X64CompileError> {
    let (img, pending) = compile_wasm_to_x64_so_with_pending_save(wasm, opts)?;
    if let Some(done) = pending {
        wait_pending_inc_cache_save(done, opts.deadline);
    }
    Ok(img)
}

/// Same as `compile_wasm_to_x64_so`, returning any in-flight incremental cache save handle so
/// `build_x64` can overlap final `.so` staging and SHA-256 hashing with incremental cache I/O.
pub(crate) fn compile_wasm_to_x64_so_with_pending_save(
    wasm: &[u8],
    opts: &X64CompileOptions,
) -> Result<(Vec<u8>, Option<PendingIncCacheSave>), X64CompileError> {
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

/// Fallibly maps `0..n` in parallel across up to `n_workers.min(16)` threads when `n >= 256`,
/// with inline fallback if `spawn_scoped` fails and early abort on any `None` (OOM).
fn parallel_try_map<T: Send, F: Fn(usize) -> Option<T> + Sync>(
    n: usize,
    n_workers: usize,
    f: F,
) -> Option<Vec<T>> {
    if n < 256 || n_workers <= 1 {
        let mut out = inc_cache::try_vec_with_capacity::<T>(n)?;
        for i in 0..n {
            out.push(f(i)?);
        }
        return Some(out);
    }
    let mut slots = inc_cache::try_vec_with_capacity::<Option<T>>(n)?;
    for _ in 0..n {
        slots.push(None);
    }
    let workers = n_workers.min(16).min(n);
    let per_worker = n.div_ceil(workers);
    let failed = AtomicBool::new(false);
    let f_ref = &f;
    let run_range = |start_idx: usize, chunk: &mut [Option<T>]| {
        for (j, slot) in chunk.iter_mut().enumerate() {
            if failed.load(Ordering::Acquire) {
                return;
            }
            match f_ref(start_idx + j) {
                Some(v) => *slot = Some(v),
                None => {
                    failed.store(true, Ordering::Release);
                    return;
                }
            }
        }
    };
    let mut unspawned: [Option<(usize, usize)>; 16] = [None; 16];
    let mut n_unspawned = 0usize;
    std::thread::scope(|s| {
        let mut rem = slots.as_mut_slice();
        let mut idx = 0usize;
        while !rem.is_empty() {
            let take = per_worker.min(rem.len());
            let (chunk, rest) = rem.split_at_mut(take);
            let start_idx = idx;
            rem = rest;
            idx += take;
            if rem.is_empty() {
                run_range(start_idx, chunk);
            } else if std::thread::Builder::new()
                .spawn_scoped(s, move || run_range(start_idx, chunk))
                .is_err()
            {
                unspawned[n_unspawned] = Some((start_idx, take));
                n_unspawned += 1;
            }
        }
    });
    for &(start_idx, take) in unspawned[..n_unspawned].iter().flatten() {
        run_range(start_idx, &mut slots[start_idx..start_idx + take]);
    }
    if failed.load(Ordering::Acquire) {
        return None;
    }
    let mut out = inc_cache::try_vec_with_capacity::<T>(n)?;
    for slot in slots {
        out.push(slot?);
    }
    Some(out)
}

enum ValidatedBacking {
    Full(wtime_validate::ValidatedModule),
    Incremental(wtime_validate::IncrementalModule),
}

impl ValidatedBacking {
    #[inline]
    fn decoded(&self) -> &DecodedModule {
        match self {
            ValidatedBacking::Full(v) => v.decoded(),
            ValidatedBacking::Incremental(v) => v.decoded(),
        }
    }

    #[inline]
    fn is_incremental(&self) -> bool {
        matches!(self, ValidatedBacking::Incremental(_))
    }

    fn ensure_body_validated(&mut self, bi: usize) -> Result<(), wtime_types::Error> {
        match self {
            ValidatedBacking::Full(_) => Ok(()),
            ValidatedBacking::Incremental(v) => v.ensure_body_validated(bi),
        }
    }
}

fn hash_raw_spans_parallel(
    wasm: &[u8],
    spans: &[(usize, usize)],
    n_workers: usize,
) -> Option<Vec<[u8; 32]>> {
    let n_defs = spans.len();
    let mut body_hashes = inc_cache::try_vec_with_capacity::<[u8; 32]>(n_defs)?;
    body_hashes.resize(n_defs, [0u8; 32]);
    if n_defs == 0 {
        return Some(body_hashes);
    }
    let hash_slice = |start_bi: usize, dst: &mut [[u8; 32]]| {
        for (j, slot) in dst.iter_mut().enumerate() {
            let (s, e) = spans[start_bi + j];
            *slot = crate::sha256::digest(&wasm[s..e]);
        }
    };
    let workers = n_workers.min(16).min(n_defs);
    let per_worker = n_defs.div_ceil(workers);
    let mut unspawned: [Option<(usize, usize)>; 16] = [None; 16];
    let mut n_unspawned = 0usize;
    std::thread::scope(|hs| {
        let mut rem = body_hashes.as_mut_slice();
        let mut bi = 0usize;
        while !rem.is_empty() {
            let take = per_worker.min(rem.len());
            let (slice, rest) = rem.split_at_mut(take);
            let start_bi = bi;
            rem = rest;
            bi += take;
            if rem.is_empty() {
                hash_slice(start_bi, slice);
            } else if std::thread::Builder::new()
                .spawn_scoped(hs, move || hash_slice(start_bi, slice))
                .is_err()
            {
                unspawned[n_unspawned] = Some((start_bi, take));
                n_unspawned += 1;
            }
        }
    });
    for &(start_bi, take) in unspawned[..n_unspawned].iter().flatten() {
        hash_slice(start_bi, &mut body_hashes[start_bi..start_bi + take]);
    }
    Some(body_hashes)
}

fn val_body_entry_from_summary(
    admit: module::BodyAdmitSummary,
    raw_body_hash: [u8; 32],
) -> inc_cache::ValBodyEntry {
    inc_cache::ValBodyEntry {
        raw_body_hash,
        total_locals: admit.total_locals,
        insn_count: admit.insn_count,
        imm_elems: admit.imm_elems,
        saw_data_count_op: admit.saw_data_count_op,
        sig_arity: admit.sig_arity as u32,
        br_table_entries: admit.br_table_entries as u32,
        g_reads: admit.g_reads,
        g_writes: admit.g_writes,
        ci_pairs: admit.ci_pairs,
    }
}

fn compile_wasm_to_x64_so_inner(
    wasm: &[u8],
    opts: &X64CompileOptions,
) -> Result<(Vec<u8>, Option<PendingIncCacheSave>), X64CompileError> {
    let t_total = Instant::now();
    let budget = CompileBudget::new(opts.deadline);
    let check_dl = || budget.check_deadline().map_err(X64CompileError::Failed);
    check_dl()?;

    let default_hooks = matches!(opts.hooks.cpuid, runtime::CpuidSource::Real)
        && matches!(opts.hooks.rdssp, runtime::RdsspSource::Real)
        && matches!(opts.hooks.syscall_fault, runtime::SyscallFault::None);
    let inc_enabled =
        opts.inc_cache_dir.is_some() && opts.cache_key_salt.is_some() && opts.opt && default_hooks;

    // 1. Validate Wasm under Stage 1's strict feature profile and resource limits.
    //    Unlock up to 16 parallel decode/validate workers for large modules (>= 256 KiB).
    //    When a Merkle-verified companion `.vidx` cache matches the module shell (`shell_hash`)
    //    and <= 50% of bodies changed, skip decoding/validating unchanged bodies and overlap
    //    loading the `x64-inc` `.bin` file with shell validation + admission!
    let t0 = Instant::now();
    let mut vlimits = x64_validate_limits();
    vlimits.compile_threads = if wasm.len() >= 256 * 1024 {
        resolve_jobs(opts.jobs).min(16)
    } else {
        1
    };
    let mut shell_hash_opt: Option<[u8; 32]> = None;
    let mut body_spans_opt: Option<Vec<(usize, usize)>> = None;
    let mut pre_body_hashes: Option<std::sync::Arc<Vec<[u8; 32]>>> = None;
    let mut pre_vidx: Option<inc_cache::ValIndexData> = None;
    type PreRawWait = std::sync::Arc<(Mutex<Option<Option<Vec<u8>>>>, std::sync::Condvar)>;
    let mut pre_raw_state: Option<([u8; 32], PreRawWait)> = None;
    let mut pre_admit_summaries: Option<Vec<Option<module::BodyAdmitSummary>>> = None;

    if inc_enabled
        && let (Some(cache_dir), Some(salt)) = (opts.inc_cache_dir.as_deref(), opts.cache_key_salt)
        && let Some((sh, spans)) = inc_cache::scan_wasm_shell_and_body_spans(
            wasm,
            &salt,
            opts.isa,
            opts.opt,
            opts.verify,
            opts.target,
        )
    {
        shell_hash_opt = Some(sh);
        let n_defs_hint = spans.len();
        if n_defs_hint > 0 && inc_cache::has_val_index(cache_dir, &sh) {
            let n_hash_workers = resolve_jobs(opts.jobs).min(n_defs_hint);
            // Overlap `.vidx` load + Merkle verify with parallel body hashing!
            let (vidx_opt, hashes_opt) = std::thread::scope(|s| {
                let dir_ref = cache_dir;
                let h_vidx = std::thread::Builder::new()
                    .spawn_scoped(s, move || {
                        inc_cache::load_val_index(dir_ref, &sh, n_defs_hint)
                    })
                    .ok();
                let hashes = hash_raw_spans_parallel(wasm, &spans, n_hash_workers);
                let vidx = h_vidx
                    .and_then(|h| h.join().ok())
                    .unwrap_or_else(|| inc_cache::load_val_index(cache_dir, &sh, n_defs_hint));
                (vidx, hashes)
            });
            if let Some(hashes) = hashes_opt {
                if let Some(vidx) = vidx_opt {
                    let changed_cnt = hashes
                        .iter()
                        .zip(&vidx.bodies)
                        .filter(|(bh, vb)| *bh != &vb.raw_body_hash)
                        .count();
                    if changed_cnt <= n_defs_hint / 2 {
                        // Start loading the 9 MiB `x64-inc` `.bin` cache in the background NOW!
                        let raw_wait = opts.deadline.map(|dl| {
                            dl.saturating_duration_since(Instant::now())
                                .saturating_sub(std::time::Duration::from_millis(250))
                                .min(std::time::Duration::from_millis(250))
                        });
                        if raw_wait.is_none_or(|w| !w.is_zero()) {
                            let st = std::sync::Arc::new((
                                Mutex::new(None::<Option<Vec<u8>>>),
                                std::sync::Condvar::new(),
                            ));
                            let st_bg = std::sync::Arc::clone(&st);
                            let dir_buf = cache_dir.to_path_buf();
                            let abi_hint = vidx.abi_hash;
                            if std::thread::Builder::new()
                                .spawn(move || {
                                    let raw = inc_cache::load_inc_cache_raw(&dir_buf, &abi_hint);
                                    let (lk, cv) = &*st_bg;
                                    *lk.lock().unwrap_or_else(|g| g.into_inner()) = Some(raw);
                                    cv.notify_all();
                                })
                                .is_ok()
                            {
                                pre_raw_state = Some((abi_hint, st));
                            }
                        }
                        pre_vidx = Some(vidx);
                    }
                }
                pre_body_hashes = Some(std::sync::Arc::new(hashes));
            }
        } else if n_defs_hint > 0 {
            // Cold cache (no `.vidx`): defer body hashing so it never blocks validation!
            body_spans_opt = Some(spans);
        }
    }

    let map_val_err = |e: wtime_types::Error| match e.kind {
        ErrorKind::DisabledFeature | ErrorKind::ResourceLimit => {
            X64CompileError::AdmissionRejected(format!("x64: {e}"))
        }
        _ => X64CompileError::Failed(format!("invalid wasm: {e}")),
    };

    let mut validated: ValidatedBacking =
        if let (Some(vidx), Some(bhs)) = (pre_vidx.as_ref(), pre_body_hashes.as_ref()) {
            let bhs_slice = bhs.as_slice();
            let bodies_ref = &vidx.bodies;
            let inc_mod = wtime_validate::validate_incremental(
                wasm,
                &x64_validate_features(),
                &vlimits,
                |idx, _slice| {
                    let vb = bodies_ref.get(idx)?;
                    if bhs_slice.get(idx)? == &vb.raw_body_hash {
                        Some(wtime_validate::CachedBodyMetrics {
                            total_locals: vb.total_locals,
                            insn_count: vb.insn_count as usize,
                            imm_elems: vb.imm_elems as usize,
                            saw_data_count_op: vb.saw_data_count_op,
                        })
                    } else {
                        None
                    }
                },
            )
            .map_err(map_val_err)?;
            let mut summaries = Vec::with_capacity(bodies_ref.len());
            for (idx, vb) in bodies_ref.iter().enumerate() {
                if !inc_mod.is_body_validated(idx) {
                    summaries.push(Some(module::BodyAdmitSummary {
                        total_locals: vb.total_locals,
                        insn_count: vb.insn_count,
                        imm_elems: vb.imm_elems,
                        saw_data_count_op: vb.saw_data_count_op,
                        sig_arity: vb.sig_arity as usize,
                        br_table_entries: vb.br_table_entries as usize,
                        g_reads: vb.g_reads.clone(),
                        g_writes: vb.g_writes.clone(),
                        ci_pairs: vb.ci_pairs.clone(),
                    }));
                } else {
                    summaries.push(None);
                }
            }
            pre_admit_summaries = Some(summaries);
            ValidatedBacking::Incremental(inc_mod)
        } else {
            ValidatedBacking::Full(
                wtime_validate::validate(wasm, &x64_validate_features(), &vlimits)
                    .map_err(map_val_err)?,
            )
        };
    let d_validate = t0.elapsed();
    check_dl()?;

    // 2. Enforce admission contract & build lean X64Module metadata.
    let t0 = Instant::now();
    let (mut module, mut admit_summaries) = module::admit_x64_module_for_target_with_summaries(
        validated.decoded(),
        opts.target,
        Some(&budget),
        pre_admit_summaries.as_deref(),
        inc_enabled,
    )
    .map_err(|e| {
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
    } = build_shared_rodata(
        validated.decoded(),
        &module,
        &preliminary_layout.global_offs,
        &budget,
    )
    .map_err(|e| {
        if budget.check_deadline().is_err() {
            X64CompileError::Failed(e)
        } else {
            X64CompileError::AdmissionRejected(e)
        }
    })?;
    let layout = CtxLayout {
        ci_tables,
        ..preliminary_layout
    };

    // 4. Emit shared runtime helpers, import trampolines, and target entry points / exports.
    let mut export_asms = match opts.target {
        X64Target::BlotCdylib => {
            emit_all_exports(&module, &layout, &ro, &tables, opts.isa, opts.hooks)
                .map_err(X64CompileError::Failed)?
        }
        X64Target::StandaloneExec => vec![
            emit_standalone_entry(&module, &layout, &ro, &tables, opts.isa, opts.hooks)
                .map_err(X64CompileError::Failed)?,
        ],
    };
    let helper_asms = emit_all_helpers(&module, &layout, &ro, &tables, opts.hooks)
        .map_err(X64CompileError::Failed)?;
    let mut import_asms = Vec::with_capacity(module.imports.len());
    for k in 0..module.imports.len() {
        let imp = match opts.target {
            X64Target::BlotCdylib => emit_import_trampoline(&module, k),
            X64Target::StandaloneExec => emit_standalone_import_trampoline(&module, k),
        }
        .map_err(X64CompileError::Failed)?;
        import_asms.push(imp);
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
    let n_defs = validated.decoded().code.len();
    let n_workers = resolve_jobs(opts.jobs).min(n_defs.max(1));
    let ns_ssa_opt = AtomicU64::new(0);
    let ns_regalloc = AtomicU64::new(0);
    let ns_emit = AtomicU64::new(0);
    let mut d_inline_effects = std::time::Duration::ZERO;
    let mut d_elide = std::time::Duration::ZERO;

    let mut compiled: Vec<Option<Asm>> = (0..n_defs).map(|_| None).collect();
    let mut new_inc_cache: Option<std::sync::Arc<inc_cache::IncCacheData>> = None;
    let mut new_val_index: Option<inc_cache::ValIndexData> = None;
    type DeferredValIndex = (
        [u8; 32],
        [u8; 32],
        Vec<usize>,
        Vec<bool>,
        Vec<module::BodyAdmitSummary>,
        std::sync::Arc<Vec<[u8; 32]>>,
    );
    type DeferredExtraDrop = (Vec<Vec<(u32, u32)>>, Option<opt::ModuleEffects>);
    let mut deferred_val_index: Option<DeferredValIndex> = None;
    let mut inc_reused_count: usize = 0;
    let mut deferred_cold_drop: Option<(Vec<IrFunc>, Vec<AllocResult>)> = None;
    let mut deferred_extra_drop: Option<DeferredExtraDrop> = None;

    let map_worker_err = |err: String| -> X64CompileError {
        if budget.check_deadline().is_ok()
            && (err.contains("exceeds cap") || err.contains("exceeds frame cap"))
        {
            X64CompileError::AdmissionRejected(err)
        } else {
            X64CompileError::Failed(err)
        }
    };

    let alloc_cfg = AllocConfig {
        allow_r12: layout.pinned_global.is_none(),
        allow_r13: layout.fixed_mem_bytes.is_some(),
        has_bmi2: opts.isa.has_bmi2,
    };

    type IncCtx = (
        [u8; 32],
        Option<std::sync::Arc<Vec<[u8; 32]>>>,
        Option<inc_cache::IncCacheData>,
    );
    let mut inc_ctx: Option<IncCtx> = if let (Some(cache_dir), Some(salt)) =
        (opts.inc_cache_dir.as_deref(), opts.cache_key_salt)
        && opts.opt
        && default_hooks
        && n_defs > 0
    {
        let abi_hash = inc_cache::compute_abi_hash(
            &salt,
            opts.isa,
            opts.opt,
            opts.verify,
            n_defs,
            &module,
            &layout,
            &ro,
        );
        let global_words = module.globals.len().div_ceil(64);
        let shared_rodata_len = rodata.len();
        let raw_wait = opts.deadline.map(|dl| {
            dl.saturating_duration_since(Instant::now())
                .saturating_sub(std::time::Duration::from_millis(250))
                .min(std::time::Duration::from_millis(250))
        });
        let can_read_cache = raw_wait.is_none_or(|w| !w.is_zero());

        // Reuse pre-computed `body_hashes` and pre-spawned `load_inc_cache_raw` when available!
        // On a cold cache miss where neither `.vidx` nor `.bin` exists on disk, defer body hashing
        // to overlap with Phase 5b instead of blocking Phase 5a!
        if let Some(bhs) = pre_body_hashes.take() {
            let raw = if let Some((hint_abi, st)) = pre_raw_state.take()
                && hint_abi == abi_hash
            {
                let (lk, cv) = &*st;
                let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
                match raw_wait {
                    None => {
                        let mut g = cv
                            .wait_while(guard, |opt| opt.is_none())
                            .unwrap_or_else(|e| e.into_inner());
                        g.take().flatten()
                    }
                    Some(wait) => {
                        let (mut g, _) = cv
                            .wait_timeout_while(guard, wait, |opt| opt.is_none())
                            .unwrap_or_else(|e| e.into_inner());
                        g.take().flatten()
                    }
                }
            } else if can_read_cache {
                inc_cache::load_inc_cache_raw(cache_dir, &abi_hash)
            } else {
                None
            };
            let loaded = raw.and_then(|bytes| {
                inc_cache::decode_verified_inc_cache(
                    std::sync::Arc::new(bytes),
                    &abi_hash,
                    n_imports,
                    n_defs,
                    global_words,
                    shared_rodata_len,
                    &bhs,
                )
            });
            Some((abi_hash, Some(bhs), loaded))
        } else if !can_read_cache || !inc_cache::has_inc_cache(cache_dir, &abi_hash) {
            Some((abi_hash, None, None))
        } else if let Some(mut body_hashes) = inc_cache::try_vec_with_capacity(n_defs) {
            body_hashes.resize(n_defs, [0u8; 32]);
            let raw_state = std::sync::Arc::new((
                Mutex::new(None::<Option<Vec<u8>>>),
                std::sync::Condvar::new(),
            ));
            let spawned_raw = {
                let dir_buf = cache_dir.to_path_buf();
                let st_bg = std::sync::Arc::clone(&raw_state);
                std::thread::Builder::new()
                    .spawn(move || {
                        let raw = inc_cache::load_inc_cache_raw(&dir_buf, &abi_hash);
                        let (lk, cv) = &*st_bg;
                        *lk.lock().unwrap_or_else(|g| g.into_inner()) = Some(raw);
                        cv.notify_all();
                    })
                    .is_ok()
            };

            let dec_ref = validated.decoded();
            let hash_slice = |start_bi: usize, dst: &mut [[u8; 32]]| {
                for (j, slot) in dst.iter_mut().enumerate() {
                    let sp = dec_ref.code[start_bi + j].span;
                    *slot = crate::sha256::digest(&dec_ref.bytes[sp.start..sp.end]);
                }
            };
            let workers = n_workers.min(16).min(n_defs);
            let per_worker = n_defs.div_ceil(workers);
            let mut unspawned: [Option<(usize, usize)>; 16] = [None; 16];
            let mut n_unspawned = 0usize;
            std::thread::scope(|hs| {
                let mut rem = body_hashes.as_mut_slice();
                let mut bi = 0usize;
                while !rem.is_empty() {
                    let take = per_worker.min(rem.len());
                    let (slice, rest) = rem.split_at_mut(take);
                    let start_bi = bi;
                    rem = rest;
                    bi += take;
                    if rem.is_empty() {
                        hash_slice(start_bi, slice);
                    } else if std::thread::Builder::new()
                        .spawn_scoped(hs, move || hash_slice(start_bi, slice))
                        .is_err()
                    {
                        unspawned[n_unspawned] = Some((start_bi, take));
                        n_unspawned += 1;
                    }
                }
            });
            for &(start_bi, take) in unspawned[..n_unspawned].iter().flatten() {
                hash_slice(start_bi, &mut body_hashes[start_bi..start_bi + take]);
            }
            let raw = if spawned_raw {
                let (lk, cv) = &*raw_state;
                let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
                match raw_wait {
                    None => {
                        let mut g = cv
                            .wait_while(guard, |opt| opt.is_none())
                            .unwrap_or_else(|e| e.into_inner());
                        g.take().flatten()
                    }
                    Some(wait) => {
                        let (mut g, _) = cv
                            .wait_timeout_while(guard, wait, |opt| opt.is_none())
                            .unwrap_or_else(|e| e.into_inner());
                        g.take().flatten()
                    }
                }
            } else if raw_wait.is_none() {
                inc_cache::load_inc_cache_raw(cache_dir, &abi_hash)
            } else {
                None
            };
            let body_hashes = std::sync::Arc::new(body_hashes);
            let loaded = raw.and_then(|bytes| {
                inc_cache::decode_verified_inc_cache(
                    std::sync::Arc::new(bytes),
                    &abi_hash,
                    n_imports,
                    n_defs,
                    global_words,
                    shared_rodata_len,
                    &body_hashes,
                )
            });
            Some((abi_hash, Some(body_hashes), loaded))
        } else {
            None
        }
    } else {
        None
    };
    check_dl()?;

    let mut used_incremental = false;
    if let Some((abi_hash, Some(ref body_hashes), ref mut prev_opt)) = inc_ctx
        && let Some(prev_cache) = prev_opt.take()
    {
        // Run speculative incremental compilation on a forked `CompileBudget` inheriting
        // the current shared code and rodata charges so warm emission enforces the exact
        // combined pre-relaxation caps while any fallback (`Ok(None)`) discards warm-only
        // charges cleanly before cold retry.
        let warm_budget = budget.fork_snapshot();
        let prev_topo_cycle = pre_vidx.as_ref().and_then(|v| {
            (v.topo_bottom_up.len() == n_defs && v.in_cycle.len() == n_defs)
                .then_some((v.topo_bottom_up.as_slice(), v.in_cycle.as_slice()))
        });
        if let Some((inc_asms, updated_cache, reused, topo_cycle_opt)) =
            try_compile_defs_incremental(
                &mut validated,
                &module,
                &layout,
                &ro,
                opts,
                alloc_cfg,
                n_workers,
                abi_hash,
                body_hashes.as_slice(),
                prev_cache,
                prev_topo_cycle,
                &warm_budget,
                &ns_ssa_opt,
                &ns_regalloc,
                &ns_emit,
                &mut d_inline_effects,
                &mut d_elide,
            )?
        {
            if let Some(asms) = inc_asms {
                for (bi, asm) in asms.into_iter().enumerate() {
                    compiled[bi] = Some(asm);
                }
            }
            if let (Some(sh), Some((topo_bu, in_cyc))) = (shell_hash_opt, topo_cycle_opt)
                && admit_summaries.len() == n_defs
            {
                let mut v_bodies = Vec::with_capacity(n_defs);
                let mut prev_bodies = pre_vidx.take().map(|v| v.bodies);
                for (bi, admit) in admit_summaries.drain(..).enumerate() {
                    if let Some(ref mut pbs) = prev_bodies
                        && pbs[bi].raw_body_hash == body_hashes[bi]
                    {
                        v_bodies.push(std::mem::replace(
                            &mut pbs[bi],
                            inc_cache::ValBodyEntry {
                                raw_body_hash: [0u8; 32],
                                total_locals: 0,
                                insn_count: 0,
                                imm_elems: 0,
                                saw_data_count_op: false,
                                sig_arity: 0,
                                br_table_entries: 0,
                                g_reads: Vec::new(),
                                g_writes: Vec::new(),
                                ci_pairs: Vec::new(),
                            },
                        ));
                    } else {
                        v_bodies.push(val_body_entry_from_summary(admit, body_hashes[bi]));
                    }
                }
                new_val_index = Some(inc_cache::ValIndexData {
                    shell_hash: sh,
                    abi_hash,
                    topo_bottom_up: topo_bu,
                    in_cycle: in_cyc,
                    bodies: v_bodies,
                });
            }
            new_inc_cache = updated_cache;
            inc_reused_count = reused;
            used_incremental = true;
        }
    }

    if n_defs > 0 && !used_incremental {
        // If incremental validation skipped bodies and the warm path fell back, perform full
        // validation + admission cleanly before cold compilation so every body is decoded.
        if validated.is_incremental() {
            validated = ValidatedBacking::Full(
                wtime_validate::validate(wasm, &x64_validate_features(), &vlimits)
                    .map_err(map_val_err)?,
            );
            let (m_full, sums_full) = module::admit_x64_module_for_target_with_summaries(
                validated.decoded(),
                opts.target,
                Some(&budget),
                None,
                inc_enabled,
            )
            .map_err(|e| {
                if budget.check_deadline().is_err() {
                    X64CompileError::Failed(e)
                } else {
                    X64CompileError::AdmissionRejected(e)
                }
            })?;
            module = m_full;
            admit_summaries = sums_full;
        }
        let decoded = validated.decoded();
        // Phase 5a (parallel, LPT-ordered biggest bodies first): Wasm -> initial SSA IR,
        // fused with scratch-reused early block-param pruning AND L1-hot pre-inline summary collection.
        let mut order_5a: Vec<usize> = (0..n_defs).collect();
        order_5a.sort_unstable_by(|&a, &b| {
            decoded.code[b]
                .instructions
                .len()
                .cmp(&decoded.code[a].instructions.len())
                .then_with(|| a.cmp(&b))
        });
        let next_pos = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        type Phase5aItem = (usize, IrFunc, Option<(usize, bool, Vec<(u32, u32)>)>);
        let worker_irs: Mutex<Vec<Vec<Phase5aItem>>> = Mutex::new(Vec::with_capacity(n_workers));
        let run_5a = || {
            let mut local_out: Vec<Phase5aItem> = Vec::new();
            let mut opt_scratch = OptIrScratch::default();
            let mut cfg_dom = opt::CfgDom::default();
            let mut cfg_sc = opt::CfgScratch::default();
            let mut prune_sc = opt::PruneScratch::default();
            let mut loc_ssa = 0u64;
            loop {
                if failed.load(Ordering::Acquire) {
                    break;
                }
                let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                if pos >= n_defs {
                    break;
                }
                let bi = order_5a[pos];
                let func_index = n_imports + bi;
                let ts = Instant::now();
                let res = lower_wasm_to_ir_with_budget_and_scratch(
                    decoded,
                    &module,
                    func_index,
                    opts.opt,
                    Some(&budget),
                    &mut opt_scratch,
                )
                .and_then(|mut ir| {
                    if opts.opt {
                        opt::prune_early_block_params_with_scratch(
                            &mut ir,
                            Some(&budget),
                            &mut cfg_dom,
                            &mut cfg_sc,
                            &mut prune_sc,
                        )?;
                    }
                    let pre = if opts.opt {
                        opt::try_collect_pre_inline_summary(&ir, n_imports, n_defs)
                    } else {
                        None
                    };
                    Ok((ir, pre))
                });
                loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                match res {
                    Ok((ir, pre)) => local_out.push((bi, ir, pre)),
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
        };
        std::thread::scope(|s| {
            for _ in 1..n_workers {
                let _ = std::thread::Builder::new().spawn_scoped(s, run_5a);
            }
            run_5a();
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        let mut ir_slots: Vec<Option<IrFunc>> = (0..n_defs).map(|_| None).collect();
        let mut pre_inline_callees: Vec<Vec<(u32, u32)>> = if opts.opt {
            vec![Vec::new(); n_defs]
        } else {
            Vec::new()
        };
        let mut pre_inst_counts: Vec<usize> = if opts.opt {
            vec![0usize; n_defs]
        } else {
            Vec::new()
        };
        let mut pre_was_inlinable: Vec<bool> = if opts.opt {
            vec![false; n_defs]
        } else {
            Vec::new()
        };
        let mut pre_summary_complete = opts.opt;
        for batch in worker_irs.into_inner().unwrap_or_else(|g| g.into_inner()) {
            for (bi, ir, pre) in batch {
                ir_slots[bi] = Some(ir);
                if opts.opt {
                    if let Some((ic, inl, cs)) = pre {
                        pre_inst_counts[bi] = ic;
                        pre_was_inlinable[bi] = inl;
                        pre_inline_callees[bi] = cs;
                    } else {
                        pre_summary_complete = false;
                    }
                }
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

        let mut capture_cache = inc_ctx.is_some() && pre_summary_complete;
        if !capture_cache {
            inc_ctx = None;
        }

        // Phase 5b (module-wide, only when `opts.opt`): budgeted small-leaf inlining + interprocedural effects.
        // Overlap deferred cold `body_hashes` computation (when `.vidx`/`.bin` were absent) with Phase 5b!
        let need_deferred_hashes = matches!(&inc_ctx, Some((_, None, _)));
        let mut inline_track: Option<opt::InlineTrack> = None;
        let mut own_effects_vec: Vec<opt::FnOwnEffects> = Vec::new();
        let mut cold_topo_cycle: Option<(Vec<usize>, Vec<bool>)> = None;
        let (effects, deferred_hashes_res) = std::thread::scope(|s_5b| {
            let h_hashes = if need_deferred_hashes {
                let spans_ref = body_spans_opt.as_deref();
                let dec_ref = validated.decoded();
                let n_hw = n_workers.clamp(1, 4);
                std::thread::Builder::new()
                    .spawn_scoped(s_5b, move || {
                        if let Some(spans) = spans_ref
                            && spans.len() == n_defs
                        {
                            hash_raw_spans_parallel(wasm, spans, n_hw)
                        } else {
                            let mut out = inc_cache::try_vec_with_capacity(n_defs)?;
                            out.resize(n_defs, [0u8; 32]);
                            for (bi, slot) in out.iter_mut().enumerate() {
                                let sp = dec_ref.code[bi].span;
                                *slot = crate::sha256::digest(&dec_ref.bytes[sp.start..sp.end]);
                            }
                            Some(out)
                        }
                    })
                    .ok()
            } else {
                None
            };

            let eff_res: Result<Option<opt::ModuleEffects>, X64CompileError> = (|| {
                if opts.opt {
                    let ti = Instant::now();
                    let cache_prep = if capture_cache {
                        (|| {
                            let mut counts = inc_cache::try_vec_with_capacity(n_defs)?;
                            counts.resize(n_defs, 0u32);
                            let mut edges = inc_cache::try_vec_with_capacity(n_defs)?;
                            for cs in &pre_inline_callees {
                                let mut row = inc_cache::try_vec_with_capacity(cs.len())?;
                                for &(c, cnt) in cs {
                                    row.push(c as usize);
                                    counts[c as usize] = counts[c as usize].saturating_add(cnt);
                                }
                                edges.push(row);
                            }
                            let (in_cycle, topo) = opt::try_call_graph_sccs_from_edges(&edges)?;
                            let opt_irs: Vec<Option<IrFunc>> =
                                inc_cache::try_vec_with_capacity(n_defs)?;
                            Some((counts, in_cycle, topo, opt_irs))
                        })()
                    } else {
                        None
                    };
                    if let Some((total_call_counts, in_cycle, topo, mut opt_irs)) = cache_prep {
                        for ir in std::mem::take(&mut irs) {
                            opt_irs.push(Some(ir));
                        }
                        let tr = opt::inline_small_helpers_subset_parallel_with_pre(
                            &mut opt_irs,
                            &in_cycle,
                            &topo,
                            &total_call_counts,
                            &module,
                            0,
                            opt::InlineMode::ColdCaptureCache,
                            n_workers,
                            Some(&pre_inline_callees),
                            Some(&pre_inst_counts),
                            Some(&pre_was_inlinable),
                            Some(&budget),
                        )
                        .map_err(map_worker_err)?
                        .ok_or_else(|| {
                            X64CompileError::Failed("x64: out of memory during inlining".into())
                        })?;
                        irs = opt_irs.into_iter().map(|o| o.unwrap()).collect();
                        if tr.tracking_complete {
                            inline_track = Some(tr);
                        } else {
                            capture_cache = false;
                            inc_ctx = None;
                        }
                        let n_total = n_imports + n_defs;
                        let mut eff_opt: Option<opt::ModuleEffects> = None;
                        if capture_cache
                            && let Some(own_eff) = parallel_try_map(n_defs, n_workers, |bi| {
                                opt::try_compute_fn_own_effects(&irs[bi], &module, n_total)
                            })
                            && let Some(eff) = opt::try_compute_module_effects_from_lookup(
                                own_eff.len(),
                                |i| &own_eff[i],
                                Some(&topo),
                                &module,
                                Some(&budget),
                            )
                            .map_err(map_worker_err)?
                        {
                            own_effects_vec = own_eff;
                            eff_opt = Some(eff);
                        }
                        if let Some(eff) = eff_opt {
                            cold_topo_cycle = Some((topo, in_cycle));
                            d_inline_effects = ti.elapsed();
                            Ok(Some(eff))
                        } else {
                            capture_cache = false;
                            inc_ctx = None;
                            let eff = opt::compute_module_effects(&irs, &module, Some(&budget))
                                .map_err(map_worker_err)?;
                            d_inline_effects = ti.elapsed();
                            Ok(Some(eff))
                        }
                    } else {
                        capture_cache = false;
                        inc_ctx = None;
                        if pre_summary_complete {
                            let mut counts = vec![0u32; n_defs];
                            let mut edges: Vec<Vec<usize>> = Vec::with_capacity(n_defs);
                            for cs in &pre_inline_callees {
                                edges.push(cs.iter().map(|&(c, _)| c as usize).collect());
                                for &(c, cnt) in cs {
                                    counts[c as usize] = counts[c as usize].saturating_add(cnt);
                                }
                            }
                            let (in_cycle, topo) = opt::call_graph_sccs_from_edges(&edges);
                            let mut opt_irs: Vec<Option<IrFunc>> =
                                std::mem::take(&mut irs).into_iter().map(Some).collect();
                            opt::inline_small_helpers_subset_parallel_with_pre(
                                &mut opt_irs,
                                &in_cycle,
                                &topo,
                                &counts,
                                &module,
                                0,
                                opt::InlineMode::ColdNoCache,
                                n_workers,
                                Some(&pre_inline_callees),
                                Some(&pre_inst_counts),
                                Some(&pre_was_inlinable),
                                Some(&budget),
                            )
                            .map_err(map_worker_err)?;
                            irs = opt_irs.into_iter().map(|o| o.unwrap()).collect();
                            let n_total = n_imports + n_defs;
                            let own_eff = parallel_try_map(n_defs, n_workers, |bi| {
                                opt::try_compute_fn_own_effects(&irs[bi], &module, n_total)
                            });
                            let eff = if let Some(ref own_eff) = own_eff
                                && let Some(e) = opt::try_compute_module_effects_from_lookup(
                                    own_eff.len(),
                                    |i| &own_eff[i],
                                    Some(&topo),
                                    &module,
                                    Some(&budget),
                                )
                                .map_err(map_worker_err)?
                            {
                                e
                            } else {
                                opt::compute_module_effects(&irs, &module, Some(&budget))
                                    .map_err(map_worker_err)?
                            };
                            d_inline_effects = ti.elapsed();
                            Ok(Some(eff))
                        } else {
                            opt::inline_small_helpers(&mut irs, &module, Some(&budget))
                                .map_err(map_worker_err)?;
                            let eff = opt::compute_module_effects(&irs, &module, Some(&budget))
                                .map_err(map_worker_err)?;
                            d_inline_effects = ti.elapsed();
                            Ok(Some(eff))
                        }
                    }
                } else {
                    Ok(None)
                }
            })();

            let deferred = h_hashes.and_then(|h| h.join().ok()).flatten();
            (eff_res, deferred)
        });
        let effects = effects?;
        if let Some((_, ref mut bhs_slot, _)) = inc_ctx
            && bhs_slot.is_none()
        {
            if let Some(dh) = deferred_hashes_res {
                *bhs_slot = Some(std::sync::Arc::new(dh));
            } else if let Some(spans) = body_spans_opt.as_deref()
                && spans.len() == n_defs
                && let Some(dh) = hash_raw_spans_parallel(wasm, spans, n_workers)
            {
                *bhs_slot = Some(std::sync::Arc::new(dh));
            } else {
                inc_ctx = None;
            }
        }
        check_dl()?;

        // Phase 5c (parallel, LPT-ordered + chunked batching for small fns): per-function Stage 2 IR opt,
        // optional `verify_ir`, register allocation, and optional `verify_alloc`.
        // Sort ascending by SSA value count so `.pop()` pops the largest functions FIRST (LPT).
        let mut queue_vec: Vec<(usize, IrFunc)> = irs.into_iter().enumerate().collect();
        queue_vec.sort_unstable_by(|(a_bi, a_ir), (b_bi, b_ir)| {
            a_ir.val_tys
                .len()
                .cmp(&b_ir.val_tys.len())
                .then_with(|| b_bi.cmp(a_bi))
        });
        let work_queue: Mutex<Vec<(usize, IrFunc)>> = Mutex::new(queue_vec);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        type Phase5cItem = (usize, IrFunc, AllocResult, Option<opt::FnElideInput>);
        let worker_allocs: Mutex<Vec<Vec<Phase5cItem>>> = Mutex::new(Vec::with_capacity(n_workers));
        let want_elide_in = opts.opt;
        let run_5c = || {
            let mut local_out: Vec<Phase5cItem> = Vec::new();
            let mut local_batch: Vec<(usize, IrFunc)> = Vec::with_capacity(16);
            let mut scratch = opt::Stage2Scratch::default();
            let mut ra_scratch = RegallocScratch::default();
            let mut loc_ssa = 0u64;
            let mut loc_ra = 0u64;
            'outer: loop {
                if failed.load(Ordering::Acquire) {
                    break;
                }
                {
                    let mut q = work_queue.lock().unwrap_or_else(|g| g.into_inner());
                    let Some((bi, ir)) = q.pop() else {
                        break;
                    };
                    let small = ir.val_tys.len() <= 96;
                    local_batch.push((bi, ir));
                    if small {
                        // Drain a chunk of tiny functions per lock to eliminate Mutex contention.
                        let take = q.len().min(15);
                        let split = q.len() - take;
                        local_batch.extend(q.drain(split..));
                    }
                }
                for (bi, mut ir) in local_batch.drain(..) {
                    if failed.load(Ordering::Acquire) {
                        break 'outer;
                    }
                    let step_res: Result<(AllocResult, Option<opt::FnElideInput>), String> =
                        (|| {
                            if let Some(ref eff) = effects {
                                let ts = Instant::now();
                                opt::optimize_func_stage2_with_scratch(
                                    &mut ir,
                                    &module,
                                    eff,
                                    Some(&budget),
                                    &mut scratch,
                                )?;
                                loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                            }
                            if opts.verify {
                                verify_ir(&ir, &module)?;
                            }
                            let tr = Instant::now();
                            let alloc = allocate_with_config_and_scratch(
                                &ir,
                                alloc_cfg,
                                Some(&budget),
                                &mut ra_scratch,
                            )?;
                            loc_ra = loc_ra.saturating_add(tr.elapsed().as_nanos() as u64);
                            if opts.verify {
                                verify_alloc_with_config(&ir, &alloc, alloc_cfg)?;
                            }
                            let elide_in = if want_elide_in {
                                opt::try_compute_fn_elide_input(&ir, &alloc, n_imports, n_defs)
                            } else {
                                None
                            };
                            Ok((alloc, elide_in))
                        })();
                    match step_res {
                        Ok((alloc, elide_in)) => local_out.push((bi, ir, alloc, elide_in)),
                        Err(e) => {
                            failed.store(true, Ordering::Release);
                            let mut guard = first_err.lock().unwrap_or_else(|g| g.into_inner());
                            if guard.as_ref().is_none_or(|(prev_bi, _)| bi < *prev_bi) {
                                *guard = Some((bi, e));
                            }
                            break 'outer;
                        }
                    }
                }
            }
            ns_ssa_opt.fetch_add(loc_ssa, Ordering::Relaxed);
            ns_regalloc.fetch_add(loc_ra, Ordering::Relaxed);
            worker_allocs
                .lock()
                .unwrap_or_else(|g| g.into_inner())
                .push(local_out);
        };
        std::thread::scope(|s| {
            for _ in 1..n_workers {
                let _ = std::thread::Builder::new().spawn_scoped(s, run_5c);
            }
            run_5c();
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        let mut ir_slots: Vec<Option<IrFunc>> = (0..n_defs).map(|_| None).collect();
        let mut alloc_slots: Vec<Option<AllocResult>> = (0..n_defs).map(|_| None).collect();
        let mut elide_slots: Vec<Option<opt::FnElideInput>> = if want_elide_in {
            (0..n_defs).map(|_| None).collect()
        } else {
            Vec::new()
        };
        let mut all_elide_ok = want_elide_in;
        for batch in worker_allocs
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
        {
            for (bi, ir, alloc, elide_in) in batch {
                ir_slots[bi] = Some(ir);
                alloc_slots[bi] = Some(alloc);
                if want_elide_in {
                    if let Some(ei) = elide_in {
                        elide_slots[bi] = Some(ei);
                    } else {
                        all_elide_ok = false;
                    }
                }
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
        let mut elide_inputs_vec: Vec<opt::FnElideInput> = Vec::new();
        let fn_can_trap: Option<Vec<bool>> = if opts.opt {
            let te = Instant::now();
            let mut cached_elide: Option<Vec<bool>> = None;
            if all_elide_ok {
                let elide_in: Vec<opt::FnElideInput> =
                    elide_slots.into_iter().map(|o| o.unwrap()).collect();
                if let Some(out) = opt::try_run_stack_and_trap_elision_from_summaries(
                    &elide_in,
                    &module,
                    Some(&budget),
                )
                .map_err(map_worker_err)?
                {
                    elide_inputs_vec = elide_in;
                    for (bi, ir) in irs.iter_mut().enumerate() {
                        ir.needs_stack_check = out.needs_stack_check[bi];
                        ir.elided_callee_stack_bytes = out.elided_callee_stack_bytes[bi];
                    }
                    cached_elide = Some(out.fn_can_trap);
                }
            }
            let res = if let Some(v) = cached_elide {
                v
            } else {
                inc_ctx = None;
                opt::run_stack_and_trap_elision(&mut irs, &allocs, &module, Some(&budget))
                    .map_err(map_worker_err)?
            };
            d_elide = te.elapsed();
            Some(res)
        } else {
            None
        };
        check_dl()?;

        // Phase 5e (parallel, LPT-ordered): x86-64 machine code lowering fused with L1-hot
        // incremental metadata hashing when capturing `IncCacheData`.
        let mut order_5e: Vec<usize> = (0..n_defs).collect();
        order_5e.sort_unstable_by(|&a, &b| {
            irs[b]
                .val_tys
                .len()
                .cmp(&irs[a].val_tys.len())
                .then_with(|| a.cmp(&b))
        });
        let next_pos = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        type MetaSnap = (Vec<(u32, u32, u32)>, [u8; 32], [u8; 32], u32);
        type EmitWorkerBatch = Vec<(usize, Asm, Option<MetaSnap>)>;
        let worker_results: Mutex<Vec<EmitWorkerBatch>> = Mutex::new(Vec::with_capacity(n_workers));
        let fn_can_trap_ref = fn_can_trap.as_deref();
        let want_meta_snap = inc_ctx.is_some()
            && inline_track.is_some()
            && effects.is_some()
            && fn_can_trap.is_some();
        let eff_ref = effects.as_ref();
        let own_ref = &own_effects_vec;
        let elide_ref = &elide_inputs_vec;
        let run_5e = || {
            let mut local_out: Vec<(usize, Asm, Option<MetaSnap>)> = Vec::new();
            let mut loc_em = 0u64;
            loop {
                if failed.load(Ordering::Acquire) {
                    break;
                }
                let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                if pos >= n_defs {
                    break;
                }
                let bi = order_5e[pos];
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
                    Ok(asm) => {
                        let snap = if want_meta_snap
                            && let (Some(eff), Some(can_trap)) = (eff_ref, fn_can_trap_ref)
                        {
                            asm.try_export_jt_entries().map(|jt_entries| {
                                let callee_effects_hash = inc_cache::hash_callee_effects(
                                    &own_ref[bi].direct_callees,
                                    eff,
                                );
                                let callee_trap_hash = inc_cache::hash_callee_traps(
                                    &elide_ref[bi].direct_def_callees,
                                    n_imports,
                                    can_trap,
                                );
                                let pre_relax_code_len =
                                    asm.pre_relax_code_len.max(asm.code.len() as u32);
                                (
                                    jt_entries,
                                    callee_effects_hash,
                                    callee_trap_hash,
                                    pre_relax_code_len,
                                )
                            })
                        } else {
                            None
                        };
                        local_out.push((bi, asm, snap));
                    }
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
        };
        std::thread::scope(|s| {
            for _ in 1..n_workers {
                let _ = std::thread::Builder::new().spawn_scoped(s, run_5e);
            }
            run_5e();
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        let mut meta_slots: Vec<Option<MetaSnap>> = if want_meta_snap {
            (0..n_defs).map(|_| None).collect()
        } else {
            Vec::new()
        };
        let mut all_meta_ok = want_meta_snap;
        for batch in worker_results
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
        {
            for (bi, asm, snap) in batch {
                compiled[bi] = Some(asm);
                if want_meta_snap {
                    if let Some(sn) = snap {
                        meta_slots[bi] = Some(sn);
                    } else {
                        all_meta_ok = false;
                    }
                }
            }
        }

        // Build zero-copy `Arc<IncCacheData>` by moving `code`/`relocs`/`local_rodata` straight
        // out of `compiled` (zero 7+ MiB cloning!), so Phase 6 linking and background
        // `save_inc_cache` share the exact same immutable `Arc<IncCacheData>`.
        let mut bg_val_index_inputs: Option<DeferredValIndex> = None;
        if all_meta_ok
            && let (Some((abi_hash, Some(body_hashes), _)), Some(mut track)) =
                (&inc_ctx, inline_track)
            && let Some(mut cached_funcs) = inc_cache::try_vec_with_capacity(n_defs)
        {
            for (bi, ((own_eff, elide_in), snap_opt)) in own_effects_vec
                .into_iter()
                .zip(elide_inputs_vec)
                .zip(meta_slots)
                .enumerate()
            {
                let (jt_entries, callee_effects_hash, callee_trap_hash, pre_relax_code_len) =
                    snap_opt.unwrap();
                let asm = compiled[bi].take().unwrap();
                cached_funcs.push(inc_cache::CachedFn {
                    raw_body_hash: body_hashes[bi],
                    was_inlinable: track.was_inlinable[bi],
                    pre_inline_callees: std::mem::take(&mut pre_inline_callees[bi]),
                    inlined_callees: std::mem::take(&mut track.inlined_callees[bi]),
                    inline_added_insts: track.added_insts[bi],
                    own_effects: own_eff,
                    elide_input: elide_in,
                    callee_effects_hash,
                    needs_stack_check: irs[bi].needs_stack_check,
                    elided_callee_stack_bytes: irs[bi].elided_callee_stack_bytes,
                    callee_trap_hash,
                    pre_relax_code_len,
                    code: inc_cache::CachedBytes::Owned(asm.code),
                    relocs: asm.relocs,
                    local_rodata: inc_cache::CachedBytes::Owned(asm.local_rodata),
                    jt_entries,
                    jt_labels: Vec::new(),
                    jt_patches: Vec::new(),
                });
            }
            if let (Some(sh), Some((topo_bu, in_cyc))) = (shell_hash_opt, cold_topo_cycle.take())
                && admit_summaries.len() == n_defs
            {
                // Assemble `ValIndexData` off the critical path in the background save thread!
                bg_val_index_inputs = Some((
                    sh,
                    *abi_hash,
                    topo_bu,
                    in_cyc,
                    std::mem::take(&mut admit_summaries),
                    std::sync::Arc::clone(body_hashes),
                ));
            }
            new_inc_cache = Some(std::sync::Arc::new(inc_cache::IncCacheData {
                abi_hash: *abi_hash,
                global_words: module.globals.len().div_ceil(64),
                hit_inline_cap: track.hit_module_cap,
                funcs: cached_funcs,
                backing: None,
            }));
        }
        if n_defs >= 256 {
            deferred_cold_drop = Some((irs, allocs));
        }
        // Pass deferred `ValIndexData` build inputs into Phase 6 background save.
        if let Some((sh, abi_h, topo_bu, in_cyc, sums, bhs)) = bg_val_index_inputs {
            deferred_val_index = Some((sh, abi_h, topo_bu, in_cyc, sums, bhs));
        }
        if n_defs >= 256 {
            deferred_extra_drop = Some((pre_inline_callees, effects));
        }
    }
    check_dl()?;

    // 6. Deterministic merge & ELF64 linking (zero-copy directly from `Arc<IncCacheData>` when
    //    present, overlapped with best-effort incremental cache write and background deallocation).
    let t_link = Instant::now();
    let link_cache: Option<std::sync::Arc<inc_cache::IncCacheData>> = new_inc_cache.take();
    let save_val_idx = new_val_index.take();
    let save_deferred_vidx = deferred_val_index.take();
    let bg_extra_drop = deferred_extra_drop.take();
    let save_done: Option<std::sync::Arc<(Mutex<bool>, std::sync::Condvar)>> =
        if let (Some(dir), Some(data_arc)) = (opts.inc_cache_dir.as_deref(), link_cache.as_ref()) {
            let dir_buf = dir.to_path_buf();
            let data_bg = std::sync::Arc::clone(data_arc);
            let done = std::sync::Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let done_bg = std::sync::Arc::clone(&done);
            let bg_validated = (n_defs >= 256).then_some(validated);
            let bg_cold_drop = deferred_cold_drop.take();
            if std::thread::Builder::new()
                .spawn(move || {
                    // Save `.bin` and `.vidx` concurrently so both finish well before ELF publish!
                    let dir_ref = &dir_buf;
                    let data_ref = &*data_bg;
                    std::thread::scope(|ss| {
                        let h_vidx = std::thread::Builder::new()
                            .spawn_scoped(ss, move || {
                                if let Some(ref vidx) = save_val_idx {
                                    inc_cache::save_val_index(dir_ref, vidx);
                                } else if let Some((sh, abi_h, topo_bu, in_cyc, sums, bhs)) =
                                    save_deferred_vidx
                                {
                                    let bodies: Vec<inc_cache::ValBodyEntry> = sums
                                        .into_iter()
                                        .zip(bhs.iter())
                                        .map(|(a, &bh)| val_body_entry_from_summary(a, bh))
                                        .collect();
                                    let vidx = inc_cache::ValIndexData {
                                        shell_hash: sh,
                                        abi_hash: abi_h,
                                        topo_bottom_up: topo_bu,
                                        in_cycle: in_cyc,
                                        bodies,
                                    };
                                    inc_cache::save_val_index(dir_ref, &vidx);
                                }
                            })
                            .ok();
                        inc_cache::save_inc_cache(dir_ref, data_ref);
                        if let Some(h) = h_vidx {
                            let _ = h.join();
                        }
                    });
                    let (lk, cv) = &*done_bg;
                    *lk.lock().unwrap_or_else(|g| g.into_inner()) = true;
                    cv.notify_all();
                    drop(bg_extra_drop);
                    drop(bg_cold_drop);
                    drop(bg_validated);
                })
                .is_ok()
            {
                Some(done)
            } else {
                None
            }
        } else {
            if n_defs >= 256 {
                let bg_validated = validated;
                let bg_cold_drop = deferred_cold_drop.take();
                let _ = std::thread::Builder::new().spawn(move || {
                    drop(bg_extra_drop);
                    drop(bg_cold_drop);
                    drop(bg_validated);
                });
            }
            None
        };

    let link_res: Result<(Vec<u8>, usize, usize), X64CompileError> = (|| {
        let mut def_asms: Vec<Asm> = if link_cache.is_some() {
            Vec::new()
        } else {
            let mut out = Vec::with_capacity(n_defs);
            for (bi, item) in compiled.into_iter().enumerate() {
                let asm = item.ok_or_else(|| {
                    X64CompileError::Failed(format!("x64: missing compiled body {bi}"))
                })?;
                out.push(asm);
            }
            out
        };

        // 6a. Precompute exact merged `.rodata` offsets for exports, imports, and every function's `local_rodata` (16-byte aligned).
        let mut planned_rodata_len = rodata.len();
        let mut reserve_local_ro_len = |len: usize| -> Result<u64, X64CompileError> {
            if len == 0 {
                return Ok(0);
            }
            let aligned =
                align_up_checked(planned_rodata_len, 16).map_err(X64CompileError::Failed)?;
            let next = aligned.checked_add(len).ok_or_else(|| {
                X64CompileError::Failed("x64: merged .rodata size overflow".into())
            })?;
            if next > MAX_TOTAL_RODATA_BYTES {
                return Err(X64CompileError::Failed(format!(
                    "x64: complete merged .rodata ({next} B) exceeds cap {MAX_TOTAL_RODATA_BYTES} B"
                )));
            }
            planned_rodata_len = next;
            Ok(aligned as u64)
        };
        let mut exp_local_rodata_offs: Vec<u64> = Vec::with_capacity(export_asms.len());
        for asm in &export_asms {
            exp_local_rodata_offs.push(reserve_local_ro_len(asm.local_rodata.len())?);
        }
        let mut helper_local_rodata_offs: Vec<u64> = Vec::with_capacity(helper_asms.len());
        for asm in &helper_asms {
            helper_local_rodata_offs.push(reserve_local_ro_len(asm.local_rodata.len())?);
        }
        let mut imp_local_rodata_offs: Vec<u64> = Vec::with_capacity(import_asms.len());
        for asm in &import_asms {
            imp_local_rodata_offs.push(reserve_local_ro_len(asm.local_rodata.len())?);
        }
        let mut fn_local_rodata_offs: Vec<u64> = Vec::with_capacity(n_defs);
        if let Some(ref cache_arc) = link_cache {
            for (bi, cf) in cache_arc.funcs.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                fn_local_rodata_offs.push(reserve_local_ro_len(cf.local_rodata.len())?);
            }
        } else {
            for (bi, asm) in def_asms.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                fn_local_rodata_offs.push(reserve_local_ro_len(asm.local_rodata.len())?);
            }
        }

        // 6b. Precompute exact merged `.text` capacity and merge (16-byte aligned per item)
        //     in deterministic order: 1. exports / `_start`, 2. shared helpers, 3. import trampolines,
        //     4. defined functions.
        let mut planned_text_len = 0usize;
        let mut plan_code_len = |len: usize| -> Result<(), X64CompileError> {
            let aligned =
                align_up_checked(planned_text_len, 16).map_err(X64CompileError::Failed)?;
            let next = aligned
                .checked_add(len)
                .ok_or_else(|| X64CompileError::Failed("x64: merged .text size overflow".into()))?;
            if next > MAX_TOTAL_CODE_BYTES {
                return Err(X64CompileError::Failed(format!(
                    "x64: complete merged .text ({next} B) exceeds cap {MAX_TOTAL_CODE_BYTES} B"
                )));
            }
            planned_text_len = next;
            Ok(())
        };
        for asm in export_asms.iter().chain(&helper_asms).chain(&import_asms) {
            plan_code_len(asm.code.len())?;
        }
        if let Some(ref cache_arc) = link_cache {
            for cf in &cache_arc.funcs {
                plan_code_len(cf.code.len())?;
            }
        } else {
            for asm in &def_asms {
                plan_code_len(asm.code.len())?;
            }
        }
        let mut text: Vec<u8> = Vec::with_capacity(planned_text_len);
        let push_text = |text: &mut Vec<u8>, bytes: &[u8]| -> Result<u64, X64CompileError> {
            let aligned = align_up_checked(text.len(), 16).map_err(X64CompileError::Failed)?;
            // Pad inter-function gaps with INT3 (0xcc) for clean trap behavior if misjumped.
            text.resize(aligned, 0xcc);
            text.extend_from_slice(bytes);
            Ok(aligned as u64)
        };

        let mut export_defs: Vec<ExportDef<'_>> = Vec::with_capacity(export_asms.len());
        let mut export_text_offs: Vec<u64> = Vec::with_capacity(export_asms.len());
        for (i, asm) in export_asms.iter().enumerate() {
            let off = push_text(&mut text, &asm.code)?;
            export_text_offs.push(off);
            if opts.target == X64Target::BlotCdylib {
                export_defs.push(ExportDef {
                    name: EXPORT_NAMES[i],
                    text_off: off,
                    size: asm.code.len() as u64,
                });
            }
        }

        let mut helper_text_offs: Vec<u64> = Vec::with_capacity(helper_asms.len());
        for asm in &helper_asms {
            helper_text_offs.push(push_text(&mut text, &asm.code)?);
        }

        let mut func_text_offs: Vec<u64> = Vec::with_capacity(n_imports + n_defs);
        for asm in &import_asms {
            func_text_offs.push(push_text(&mut text, &asm.code)?);
        }
        if let Some(ref cache_arc) = link_cache {
            let backing = cache_arc.backing.as_deref().map(Vec::as_slice);
            for (bi, cf) in cache_arc.funcs.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                func_text_offs.push(push_text(&mut text, cf.code.as_slice(backing))?);
            }
        } else {
            for (bi, asm) in def_asms.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                func_text_offs.push(push_text(&mut text, &asm.code)?);
            }
        }

        // 6c. Plan ELF64 layout (shared object or static executable).
        enum PlannedElf {
            So(ElfLayout),
            Exec(ElfExecLayout),
        }
        let (planned_elf, ro_vaddr, tx_vaddr) = match opts.target {
            X64Target::BlotCdylib => {
                let lay = ElfLayout::plan(&EXPORT_NAMES, planned_rodata_len)
                    .map_err(X64CompileError::Failed)?;
                let (rv, tv) = (lay.rodata_vaddr as u64, lay.text_vaddr as u64);
                (PlannedElf::So(lay), rv, tv)
            }
            X64Target::StandaloneExec => {
                let lay =
                    ElfExecLayout::plan(planned_rodata_len).map_err(X64CompileError::Failed)?;
                let (rv, tv) = (lay.rodata_vaddr as u64, lay.text_vaddr as u64);
                (PlannedElf::Exec(lay), rv, tv)
            }
        };

        // 6d. Append `local_rodata` for exports, imports, and defined functions (patching jump tables).
        rodata.reserve_exact(planned_rodata_len.saturating_sub(rodata.len()));
        for (i, asm) in export_asms.iter_mut().enumerate() {
            if !asm.local_rodata.is_empty() {
                let target_off = exp_local_rodata_offs[i] as usize;
                rodata.resize(target_off, 0);
                rodata.extend_from_slice(&asm.local_rodata);
            }
        }
        for (i, asm) in helper_asms.iter().enumerate() {
            if !asm.local_rodata.is_empty() {
                let target_off = helper_local_rodata_offs[i] as usize;
                rodata.resize(target_off, 0);
                rodata.extend_from_slice(&asm.local_rodata);
            }
        }
        for (k, asm) in import_asms.iter_mut().enumerate() {
            if !asm.local_rodata.is_empty() {
                let target_off = imp_local_rodata_offs[k] as usize;
                rodata.resize(target_off, 0);
                rodata.extend_from_slice(&asm.local_rodata);
            }
        }
        if let Some(ref cache_arc) = link_cache {
            let backing = cache_arc.backing.as_deref().map(Vec::as_slice);
            for (bi, cf) in cache_arc.funcs.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                if !cf.local_rodata.is_empty() {
                    let target_off = fn_local_rodata_offs[bi] as usize;
                    debug_assert!(target_off >= rodata.len());
                    rodata.resize(target_off, 0);
                    rodata.extend_from_slice(cf.local_rodata.as_slice(backing));
                    if !cf.jt_entries.is_empty() {
                        let fn_vaddr = tx_vaddr
                            .checked_add(func_text_offs[n_imports + bi])
                            .ok_or_else(|| {
                                X64CompileError::Failed("x64: fn_vaddr overflow".into())
                            })?;
                        let lro_vaddr =
                            ro_vaddr
                                .checked_add(fn_local_rodata_offs[bi])
                                .ok_or_else(|| {
                                    X64CompileError::Failed(
                                        "x64: local_rodata vaddr overflow".into(),
                                    )
                                })?;
                        Asm::patch_cached_jump_tables_in_slice(
                            &mut rodata[target_off..],
                            &cf.jt_entries,
                            fn_vaddr,
                            lro_vaddr,
                        )
                        .map_err(X64CompileError::Failed)?;
                    }
                }
            }
        } else {
            for (bi, asm) in def_asms.iter_mut().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                if !asm.local_rodata.is_empty() {
                    let fn_vaddr = tx_vaddr
                        .checked_add(func_text_offs[n_imports + bi])
                        .ok_or_else(|| X64CompileError::Failed("x64: fn_vaddr overflow".into()))?;
                    let lro_vaddr =
                        ro_vaddr
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
                    .ok_or_else(|| {
                        X64CompileError::Failed("x64: ci entry offset overflow".into())
                    })?;
                let target_off = *func_text_offs.get(fi as usize).ok_or_else(|| {
                    X64CompileError::Failed(format!("x64: ci target func {fi} out of range"))
                })?;
                let target_vaddr = tx_vaddr.checked_add(target_off).ok_or_else(|| {
                    X64CompileError::Failed("x64: ci target vaddr overflow".into())
                })?;
                let disp = i32::try_from(i128::from(target_vaddr) - i128::from(table_base_vaddr))
                    .map_err(|_| {
                    X64CompileError::Failed("x64: ci target rel32 overflow".into())
                })?;
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
                .ok_or_else(|| {
                    X64CompileError::Failed("x64: export table vaddr overflow".into())
                })?;
            let target_off = *func_text_offs.get(p.func_idx).ok_or_else(|| {
                X64CompileError::Failed(format!(
                    "x64: export target func {} out of range",
                    p.func_idx
                ))
            })?;
            let target_vaddr = tx_vaddr.checked_add(target_off).ok_or_else(|| {
                X64CompileError::Failed("x64: export target vaddr overflow".into())
            })?;
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
            let lro = (!asm.local_rodata.is_empty()).then_some(exp_local_rodata_offs[i]);
            patch_relocs(&mut text, export_text_offs[i], &asm.relocs, lro, &link_ctx)
                .map_err(X64CompileError::Failed)?;
        }
        for (i, asm) in helper_asms.iter().enumerate() {
            let lro = (!asm.local_rodata.is_empty()).then_some(helper_local_rodata_offs[i]);
            patch_relocs(&mut text, helper_text_offs[i], &asm.relocs, lro, &link_ctx)
                .map_err(X64CompileError::Failed)?;
        }
        for (k, asm) in import_asms.iter().enumerate() {
            let lro = (!asm.local_rodata.is_empty()).then_some(imp_local_rodata_offs[k]);
            patch_relocs(&mut text, func_text_offs[k], &asm.relocs, lro, &link_ctx)
                .map_err(X64CompileError::Failed)?;
        }
        if let Some(ref cache_arc) = link_cache {
            for (bi, cf) in cache_arc.funcs.iter().enumerate() {
                if (bi & 1023) == 1023 {
                    check_dl()?;
                }
                let lro = (!cf.local_rodata.is_empty()).then_some(fn_local_rodata_offs[bi]);
                patch_relocs(
                    &mut text,
                    func_text_offs[n_imports + bi],
                    &cf.relocs,
                    lro,
                    &link_ctx,
                )
                .map_err(X64CompileError::Failed)?;
            }
        } else {
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
        }
        check_dl()?;

        // 6h. Emit ELF64 shared library or static executable image.
        let text_len = text.len();
        let rodata_len = rodata.len();
        let img = match planned_elf {
            PlannedElf::So(lay) => lay.emit(&rodata, &text, &export_defs),
            PlannedElf::Exec(lay) => lay.emit(&rodata, &text, export_text_offs[0]),
        }
        .map_err(X64CompileError::Failed)?;
        Ok((img, text_len, rodata_len))
    })();

    let (img, text_len, rodata_len) = link_res?;
    let d_link = t_link.elapsed();

    if std::env::var_os("WTIME_AOT_TIMINGS").is_some_and(|v| v == "1") {
        let total_ms = t_total.elapsed().as_secs_f64() * 1000.0;
        let ssa_ms = (ns_ssa_opt.load(Ordering::Relaxed) as f64) / 1e6;
        let ra_ms = (ns_regalloc.load(Ordering::Relaxed) as f64) / 1e6;
        let em_ms = (ns_emit.load(Ordering::Relaxed) as f64) / 1e6;
        eprintln!(
            "wtime-aot x64 timings: validate={:.2}ms admit={:.2}ms ssa_opt={:.2}ms(cpu) inline_eff={:.2}ms regalloc={:.2}ms(cpu) elide={:.2}ms emit={:.2}ms(cpu) link={:.2}ms total={:.2}ms (fns={} reused={} workers={} text={}B rodata={}B)",
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
            inc_reused_count,
            n_workers,
            text_len,
            rodata_len,
        );
    }

    Ok((img, save_done))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn try_compile_defs_incremental(
    validated: &mut ValidatedBacking,
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    opts: &X64CompileOptions,
    alloc_cfg: AllocConfig,
    n_workers: usize,
    abi_hash: [u8; 32],
    body_hashes: &[[u8; 32]],
    mut prev_cache: inc_cache::IncCacheData,
    prev_topo_cycle: Option<(&[usize], &[bool])>,
    budget: &CompileBudget,
    ns_ssa_opt: &AtomicU64,
    ns_regalloc: &AtomicU64,
    ns_emit: &AtomicU64,
    d_inline_effects: &mut std::time::Duration,
    d_elide: &mut std::time::Duration,
) -> Result<
    Option<(
        Option<Vec<Asm>>,
        Option<std::sync::Arc<inc_cache::IncCacheData>>,
        usize,
        Option<(Vec<usize>, Vec<bool>)>,
    )>,
    X64CompileError,
> {
    let n_imports = module.imports.len();
    let n_defs = validated.decoded().code.len();
    let n_total = n_imports + n_defs;
    let map_worker_err = |err: String| -> X64CompileError {
        if budget.check_deadline().is_ok()
            && (err.contains("exceeds cap") || err.contains("exceeds frame cap"))
        {
            X64CompileError::AdmissionRejected(err)
        } else {
            X64CompileError::Failed(err)
        }
    };

    let Some(mut changed) = inc_cache::try_vec_with_capacity(n_defs) else {
        return Ok(None);
    };
    changed.resize(n_defs, false);
    let mut changed_indices = Vec::new();
    for bi in 0..n_defs {
        if prev_cache.funcs[bi].raw_body_hash != body_hashes[bi] {
            changed[bi] = true;
            if changed_indices.len() == changed_indices.capacity()
                && changed_indices.try_reserve(1).is_err()
            {
                return Ok(None);
            }
            changed_indices.push(bi);
        }
    }
    if changed_indices.len() > n_defs / 2 {
        return Ok(None);
    }

    // Helper: ensure any skipped bodies in `indices` are decoded & validated on demand, then
    // lower them into `raw_irs` in parallel with preallocated fallible output slots.
    let lower_subset = |validated: &mut ValidatedBacking,
                        indices: &[usize],
                        raw_irs: &mut [Option<IrFunc>]|
     -> Result<bool, X64CompileError> {
        if indices.is_empty() {
            return Ok(true);
        }
        for &bi in indices {
            validated
                .ensure_body_validated(bi)
                .map_err(|e| match e.kind {
                    ErrorKind::DisabledFeature | ErrorKind::ResourceLimit => {
                        X64CompileError::AdmissionRejected(format!("x64: {e}"))
                    }
                    _ => X64CompileError::Failed(format!("invalid wasm: {e}")),
                })?;
        }
        let decoded = validated.decoded();
        let Some(mut slots_vec) =
            inc_cache::try_vec_with_capacity::<Option<(usize, IrFunc)>>(indices.len())
        else {
            return Ok(false);
        };
        for _ in 0..indices.len() {
            slots_vec.push(None);
        }
        let out_slots = Mutex::new(slots_vec);
        let next_pos = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        let workers = n_workers.min(indices.len());
        let run_worker = || {
            let mut opt_scratch = OptIrScratch::default();
            let mut loc_ssa = 0u64;
            loop {
                if failed.load(Ordering::Acquire) {
                    break;
                }
                let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                if pos >= indices.len() {
                    break;
                }
                let bi = indices[pos];
                let func_index = n_imports + bi;
                let ts = Instant::now();
                let res = lower_wasm_to_ir_with_budget_and_scratch(
                    decoded,
                    module,
                    func_index,
                    opts.opt,
                    Some(budget),
                    &mut opt_scratch,
                )
                .and_then(|mut ir| {
                    opt::prune_early_block_params(&mut ir, Some(budget))?;
                    Ok(ir)
                });
                loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                match res {
                    Ok(ir) => {
                        out_slots.lock().unwrap_or_else(|g| g.into_inner())[pos] = Some((bi, ir));
                    }
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
        };
        std::thread::scope(|s| {
            for _ in 1..workers {
                let _ = std::thread::Builder::new().spawn_scoped(s, run_worker);
            }
            run_worker();
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        for (bi, ir) in out_slots
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
            .into_iter()
            .flatten()
        {
            raw_irs[bi] = Some(ir);
        }
        Ok(true)
    };

    let Some(mut raw_irs) = inc_cache::try_vec_with_capacity::<Option<IrFunc>>(n_defs) else {
        return Ok(None);
    };
    for _ in 0..n_defs {
        raw_irs.push(None);
    }
    if !lower_subset(validated, &changed_indices, &mut raw_irs)? {
        return Ok(None);
    }

    // Recompute exact module pre-inline call graph & SCCs using cached edges for unchanged
    // functions, skipping both `cur_edges` allocation and Tarjan SCC when `!any_callees_changed`
    // and cached `(topo_bottom_up, in_cycle)` is available!
    let ti = Instant::now();
    let (Some(mut pre_callees_changed), Some(mut new_inlinable), Some(mut cur_call_counts)) = (
        inc_cache::try_vec_with_capacity::<Option<Vec<(u32, u32)>>>(n_defs),
        inc_cache::try_vec_with_capacity::<bool>(n_defs),
        inc_cache::try_vec_with_capacity::<u32>(n_defs),
    ) else {
        return Ok(None);
    };
    new_inlinable.resize(n_defs, false);
    cur_call_counts.resize(n_defs, 0u32);

    let mut any_callees_changed = false;
    for bi in 0..n_defs {
        let prev_cs = &prev_cache.funcs[bi].pre_inline_callees;
        let cur_cs: &[(u32, u32)] = if changed[bi] {
            let ir = raw_irs[bi].as_ref().unwrap();
            new_inlinable[bi] = opt::is_inlinable_ir(ir);
            let Some(cs) = opt::try_collect_pre_inline_callees(ir, n_imports, n_defs) else {
                return Ok(None);
            };
            if cs != *prev_cs {
                any_callees_changed = true;
            }
            pre_callees_changed.push(Some(cs));
            pre_callees_changed[bi].as_deref().unwrap()
        } else {
            new_inlinable[bi] = prev_cache.funcs[bi].was_inlinable;
            pre_callees_changed.push(None);
            prev_cs.as_slice()
        };
        for &(c, cnt) in cur_cs {
            cur_call_counts[c as usize] = cur_call_counts[c as usize].saturating_add(cnt);
        }
    }

    let mut rev_callers_opt: Option<Vec<Vec<usize>>> = None;
    fn ensure_rev_callers<'a>(
        slot: &'a mut Option<Vec<Vec<usize>>>,
        prev_funcs: &[inc_cache::CachedFn],
        changed_callees: &[Option<Vec<(u32, u32)>>],
    ) -> Option<&'a [Vec<usize>]> {
        if slot.is_none() {
            let n_defs = prev_funcs.len();
            let mut rev = inc_cache::try_vec_with_capacity::<Vec<usize>>(n_defs)?;
            for _ in 0..n_defs {
                rev.push(Vec::new());
            }
            for bi in 0..n_defs {
                let row: &[(u32, u32)] = changed_callees[bi]
                    .as_deref()
                    .unwrap_or(&prev_funcs[bi].pre_inline_callees);
                for &(c, _) in row {
                    let r = &mut rev[c as usize];
                    if r.len() == r.capacity() && r.try_reserve(1).is_err() {
                        return None;
                    }
                    r.push(bi);
                }
            }
            *slot = Some(rev);
        }
        slot.as_deref()
    }

    let (in_cycle, topo_bottom_up) =
        if !any_callees_changed && let Some((prev_topo, prev_cyc)) = prev_topo_cycle {
            let Some(t) = inc_cache::try_copy_slice(prev_topo) else {
                return Ok(None);
            };
            let Some(c) = inc_cache::try_copy_slice(prev_cyc) else {
                return Ok(None);
            };
            (c, t)
        } else {
            let Some(mut cur_edges) = inc_cache::try_vec_with_capacity::<Vec<usize>>(n_defs) else {
                return Ok(None);
            };
            for (bi, changed_row) in pre_callees_changed.iter().enumerate() {
                let row: &[(u32, u32)] = changed_row
                    .as_deref()
                    .unwrap_or(&prev_cache.funcs[bi].pre_inline_callees);
                let Some(mut cur_row) = inc_cache::try_vec_with_capacity(row.len()) else {
                    return Ok(None);
                };
                for &(c, _) in row {
                    cur_row.push(c as usize);
                }
                cur_edges.push(cur_row);
            }
            let Some(pair) = opt::try_call_graph_sccs_from_edges(&cur_edges) else {
                return Ok(None);
            };
            pair
        };

    // Seed initial dirty set from changed bodies, cycle transitions, single-site transitions, and inlined-callee deps.
    let (Some(mut dirty), Some(mut cycle_or_body_changed)) = (
        inc_cache::try_copy_slice(&changed),
        inc_cache::try_vec_with_capacity(n_defs),
    ) else {
        return Ok(None);
    };
    cycle_or_body_changed.resize(n_defs, false);
    if any_callees_changed {
        let (Some(mut prev_edges), Some(mut prev_call_counts)) = (
            inc_cache::try_vec_with_capacity::<Vec<usize>>(n_defs),
            inc_cache::try_vec_with_capacity::<u32>(n_defs),
        ) else {
            return Ok(None);
        };
        prev_call_counts.resize(n_defs, 0u32);
        for bi in 0..n_defs {
            let prev_cs = &prev_cache.funcs[bi].pre_inline_callees;
            let Some(mut prev_row) = inc_cache::try_vec_with_capacity(prev_cs.len()) else {
                return Ok(None);
            };
            for &(c, cnt) in prev_cs {
                prev_row.push(c as usize);
                prev_call_counts[c as usize] = prev_call_counts[c as usize].saturating_add(cnt);
            }
            prev_edges.push(prev_row);
        }
        let Some((prev_in_cycle, _)) = opt::try_call_graph_sccs_from_edges(&prev_edges) else {
            return Ok(None);
        };
        for bi in 0..n_defs {
            let single_flipped = (prev_call_counts[bi] == 1) != (cur_call_counts[bi] == 1);
            if changed[bi] || in_cycle[bi] != prev_in_cycle[bi] || single_flipped {
                cycle_or_body_changed[bi] = true;
            }
        }
    } else {
        // Pre-inline call graph is 100% identical to cached run: zero cycle or call-count transitions.
        for bi in 0..n_defs {
            if changed[bi] {
                cycle_or_body_changed[bi] = true;
            }
        }
    }
    let Some(mut queue) = inc_cache::try_vec_with_capacity(n_defs) else {
        return Ok(None);
    };
    for (bi, is_dirty) in dirty.iter_mut().enumerate() {
        if !*is_dirty {
            let c_fn = &prev_cache.funcs[bi];
            let inline_dep_dirty = c_fn
                .inlined_callees
                .iter()
                .any(|&c| cycle_or_body_changed[c as usize]);
            let callee_inlinable_changed = c_fn.pre_inline_callees.iter().any(|&(c, _)| {
                let cu = c as usize;
                cycle_or_body_changed[cu]
                    && (prev_cache.funcs[cu].was_inlinable || new_inlinable[cu])
            });
            if inline_dep_dirty || callee_inlinable_changed {
                *is_dirty = true;
            }
        }
        if *is_dirty {
            queue.push(bi);
        }
    }
    // Propagate dirty upward through callers of any dirty potentially-inlinable callee.
    if queue
        .iter()
        .any(|&c| prev_cache.funcs[c].was_inlinable || new_inlinable[c])
    {
        let Some(rev_callers) = ensure_rev_callers(
            &mut rev_callers_opt,
            &prev_cache.funcs,
            &pre_callees_changed,
        ) else {
            return Ok(None);
        };
        while let Some(c) = queue.pop() {
            if prev_cache.funcs[c].was_inlinable || new_inlinable[c] {
                for &caller in &rev_callers[c] {
                    if !dirty[caller] {
                        dirty[caller] = true;
                        queue.push(caller);
                    }
                }
            }
        }
    }

    let mut dirty_count = dirty.iter().filter(|&&d| d).count();
    if dirty_count > n_defs / 2 {
        return Ok(None);
    }

    // Sparse overlays for dirty functions only (ZERO cloning of 5,000+ clean `own_effects`/`elide_input`!).
    let (
        Some(mut opt_irs),
        Some(mut opt_allocs),
        Some(mut inline_track_dirty),
        Some(mut own_effects_dirty),
        Some(mut elide_inputs_dirty),
        Some(mut compiled_dirty),
    ) = (
        inc_cache::try_vec_with_capacity::<Option<IrFunc>>(n_defs),
        inc_cache::try_vec_with_capacity::<Option<AllocResult>>(n_defs),
        inc_cache::try_vec_with_capacity::<Option<(Vec<u32>, u32, bool)>>(n_defs),
        inc_cache::try_vec_with_capacity::<Option<opt::FnOwnEffects>>(n_defs),
        inc_cache::try_vec_with_capacity::<Option<opt::FnElideInput>>(n_defs),
        inc_cache::try_vec_with_capacity::<bool>(n_defs),
    )
    else {
        return Ok(None);
    };
    for _ in 0..n_defs {
        opt_irs.push(None);
        opt_allocs.push(None);
        inline_track_dirty.push(None);
        own_effects_dirty.push(None);
        elide_inputs_dirty.push(None);
        compiled_dirty.push(false);
    }

    // Fixed-point loop: compile dirty functions, check if transitive effects or stack/trap elision
    // changed for any clean caller, and promote those callers if so (almost always 1 iteration!).
    let (effects, elide_out) = loop {
        budget.check_deadline().map_err(X64CompileError::Failed)?;
        if dirty_count > n_defs / 2 {
            return Ok(None);
        }

        // Collect newly dirty functions (not yet inlined/optimized in this loop).
        // Every warm-only preparation buffer uses fallible allocation so low memory returns
        // `Ok(None)` cleanly and retries on the cold path.
        let (Some(mut need_lower), Some(mut cone_q)) = (
            inc_cache::try_vec_with_capacity(n_defs),
            inc_cache::try_vec_with_capacity(n_defs),
        ) else {
            return Ok(None);
        };
        need_lower.resize(n_defs, false);
        for bi in 0..n_defs {
            if dirty[bi] && !compiled_dirty[bi] {
                need_lower[bi] = true;
                cone_q.push(bi);
            }
        }
        // Pull transitive inlinable callees needed as read-only inputs for newly dirty callers.
        while let Some(u) = cone_q.pop() {
            let row: &[(u32, u32)] = pre_callees_changed[u]
                .as_deref()
                .unwrap_or(&prev_cache.funcs[u].pre_inline_callees);
            for &(c_u32, _) in row {
                let c = c_u32 as usize;
                if !need_lower[c] && (prev_cache.funcs[c].was_inlinable || new_inlinable[c]) {
                    need_lower[c] = true;
                    cone_q.push(c);
                }
            }
        }

        let Some(mut to_lower) = inc_cache::try_vec_with_capacity(n_defs) else {
            return Ok(None);
        };
        for bi in 0..n_defs {
            if need_lower[bi] && raw_irs[bi].is_none() {
                to_lower.push(bi);
            }
        }
        if !lower_subset(validated, &to_lower, &mut raw_irs)? {
            return Ok(None);
        }

        let Some(mut work_irs) = inc_cache::try_vec_with_capacity::<Option<IrFunc>>(n_defs) else {
            return Ok(None);
        };
        for bi in 0..n_defs {
            if need_lower[bi] {
                let Some(cloned) = raw_irs[bi].as_ref().and_then(IrFunc::try_clone) else {
                    return Ok(None);
                };
                work_irs.push(Some(cloned));
            } else {
                work_irs.push(None);
            }
        }

        // Sum `inline_added_insts` of all functions NOT in `work_irs` so module cap check is exact.
        let mut outside_added = 0usize;
        for bi in 0..n_defs {
            if !need_lower[bi] {
                let add = if let Some((_, a, _)) = inline_track_dirty[bi] {
                    a as usize
                } else {
                    prev_cache.funcs[bi].inline_added_insts as usize
                };
                outside_added = outside_added.saturating_add(add);
            }
        }
        let Some(mut track) = opt::inline_small_helpers_subset_parallel(
            &mut work_irs,
            &in_cycle,
            &topo_bottom_up,
            &cur_call_counts,
            module,
            outside_added,
            opt::InlineMode::WarmSpeculative,
            n_workers,
            Some(budget),
        )
        .map_err(map_worker_err)?
        else {
            return Ok(None);
        };
        if track.hit_module_cap || !track.tracking_complete {
            return Ok(None);
        }

        let (Some(mut newly_dirty_list), Some(mut newly_inlinable)) = (
            inc_cache::try_vec_with_capacity(n_defs),
            inc_cache::try_vec_with_capacity(n_defs),
        ) else {
            return Ok(None);
        };
        for bi in 0..n_defs {
            if dirty[bi] && !compiled_dirty[bi] {
                let ir = work_irs[bi].as_ref().unwrap();
                let Some(own_eff) = opt::try_compute_fn_own_effects(ir, module, n_total) else {
                    return Ok(None);
                };
                own_effects_dirty[bi] = Some(own_eff);
                inline_track_dirty[bi] = Some((
                    std::mem::take(&mut track.inlined_callees[bi]),
                    track.added_insts[bi],
                    track.was_inlinable[bi],
                ));
                if track.was_inlinable[bi] && !new_inlinable[bi] {
                    new_inlinable[bi] = true;
                    newly_inlinable.push(bi);
                }
                newly_dirty_list.push(bi);
            }
        }
        let mut promoted = false;
        if !newly_inlinable.is_empty() {
            let Some(rev_callers) = ensure_rev_callers(
                &mut rev_callers_opt,
                &prev_cache.funcs,
                &pre_callees_changed,
            ) else {
                return Ok(None);
            };
            let (Some(mut seen_promo), Some(mut promo_q)) = (
                inc_cache::try_vec_with_capacity(n_defs),
                inc_cache::try_vec_with_capacity(n_defs),
            ) else {
                return Ok(None);
            };
            seen_promo.resize(n_defs, false);
            for bi in newly_inlinable {
                seen_promo[bi] = true;
                promo_q.push(bi);
            }
            while let Some(u) = promo_q.pop() {
                for &caller in &rev_callers[u] {
                    if !dirty[caller] {
                        dirty[caller] = true;
                        dirty_count += 1;
                        promoted = true;
                    } else if compiled_dirty[caller] {
                        compiled_dirty[caller] = false;
                        promoted = true;
                    }
                    if !seen_promo[caller]
                        && (prev_cache.funcs[caller].was_inlinable || new_inlinable[caller])
                    {
                        seen_promo[caller] = true;
                        promo_q.push(caller);
                    }
                }
            }
        }

        // Recompute exact transitive effects over ALL functions in topological order without cloning clean summaries.
        let Some(effects) = opt::try_compute_module_effects_from_lookup(
            n_defs,
            |bi| {
                own_effects_dirty[bi]
                    .as_ref()
                    .unwrap_or(&prev_cache.funcs[bi].own_effects)
            },
            Some(&topo_bottom_up),
            module,
            Some(budget),
        )
        .map_err(map_worker_err)?
        else {
            return Ok(None);
        };
        let own_effects_changed = dirty.iter().enumerate().any(|(bi, &d)| {
            d && own_effects_dirty[bi]
                .as_ref()
                .is_some_and(|e| *e != prev_cache.funcs[bi].own_effects)
        });
        if own_effects_changed {
            for (bi, is_dirty) in dirty.iter_mut().enumerate() {
                if !*is_dirty {
                    let h = inc_cache::hash_callee_effects(
                        &prev_cache.funcs[bi].own_effects.direct_callees,
                        &effects,
                    );
                    if h != prev_cache.funcs[bi].callee_effects_hash {
                        *is_dirty = true;
                        dirty_count += 1;
                        promoted = true;
                    }
                }
            }
        }
        if promoted {
            // Re-run loop to include promoted callers BEFORE running Stage 2 opt!
            continue;
        }

        // Run Stage 2 opt + regalloc ONLY on `newly_dirty_list` (parallel when >1),
        // reusing worker-local `Stage2Scratch`.
        if !newly_dirty_list.is_empty() {
            let (Some(mut work_q_vec), Some(mut out_slots_vec)) = (
                inc_cache::try_vec_with_capacity(newly_dirty_list.len()),
                inc_cache::try_vec_with_capacity::<Option<(usize, IrFunc, AllocResult)>>(
                    newly_dirty_list.len(),
                ),
            ) else {
                return Ok(None);
            };
            for &bi in &newly_dirty_list {
                work_q_vec.push((bi, work_irs[bi].take().unwrap()));
                out_slots_vec.push(None);
            }
            work_q_vec.sort_unstable_by_key(|(_, ir)| ir.val_tys.len());
            let work_q = Mutex::new(work_q_vec);
            let out_slots = Mutex::new(out_slots_vec);
            let next_out = AtomicUsize::new(0);
            let failed = AtomicBool::new(false);
            let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
            let workers = n_workers.min(newly_dirty_list.len());
            let run_worker = || {
                let mut scratch = opt::Stage2Scratch::default();
                let mut ra_scratch = RegallocScratch::default();
                let mut loc_ssa = 0u64;
                let mut loc_ra = 0u64;
                loop {
                    if failed.load(Ordering::Acquire) {
                        break;
                    }
                    let item = work_q.lock().unwrap_or_else(|g| g.into_inner()).pop();
                    let Some((bi, mut ir)) = item else {
                        break;
                    };
                    let step_res: Result<AllocResult, String> = (|| {
                        let ts = Instant::now();
                        opt::optimize_func_stage2_with_scratch(
                            &mut ir,
                            module,
                            &effects,
                            Some(budget),
                            &mut scratch,
                        )?;
                        loc_ssa = loc_ssa.saturating_add(ts.elapsed().as_nanos() as u64);
                        if opts.verify {
                            verify_ir(&ir, module)?;
                        }
                        let tr = Instant::now();
                        let alloc = allocate_with_config_and_scratch(
                            &ir,
                            alloc_cfg,
                            Some(budget),
                            &mut ra_scratch,
                        )?;
                        loc_ra = loc_ra.saturating_add(tr.elapsed().as_nanos() as u64);
                        if opts.verify {
                            verify_alloc_with_config(&ir, &alloc, alloc_cfg)?;
                        }
                        Ok(alloc)
                    })();
                    match step_res {
                        Ok(alloc) => {
                            let idx = next_out.fetch_add(1, Ordering::Relaxed);
                            out_slots.lock().unwrap_or_else(|g| g.into_inner())[idx] =
                                Some((bi, ir, alloc));
                        }
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
            };
            std::thread::scope(|s| {
                for _ in 1..workers {
                    let _ = std::thread::Builder::new().spawn_scoped(s, run_worker);
                }
                run_worker();
            });
            if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
                return Err(map_worker_err(err));
            }
            for (bi, ir, alloc) in out_slots
                .into_inner()
                .unwrap_or_else(|g| g.into_inner())
                .into_iter()
                .flatten()
            {
                let Some(elide_in) =
                    opt::try_compute_fn_elide_input(&ir, &alloc, n_imports, n_defs)
                else {
                    return Ok(None);
                };
                elide_inputs_dirty[bi] = Some(elide_in);
                opt_irs[bi] = Some(ir);
                opt_allocs[bi] = Some(alloc);
                compiled_dirty[bi] = true;
            }
        }

        // Recompute exact Step 8a/8b stack & trap elision over ALL functions without cloning clean summaries.
        let te = Instant::now();
        let Some(elide_out) = opt::try_run_stack_and_trap_elision_from_lookup(
            n_defs,
            |bi| {
                elide_inputs_dirty[bi]
                    .as_ref()
                    .unwrap_or(&prev_cache.funcs[bi].elide_input)
            },
            module,
            Some(budget),
        )
        .map_err(map_worker_err)?
        else {
            return Ok(None);
        };
        *d_elide += te.elapsed();

        let elide_inputs_changed = dirty.iter().enumerate().any(|(bi, &d)| {
            d && elide_inputs_dirty[bi]
                .as_ref()
                .is_some_and(|e| *e != prev_cache.funcs[bi].elide_input)
        });
        let mut promoted_elide = false;
        if elide_inputs_changed {
            for (bi, is_dirty) in dirty.iter_mut().enumerate() {
                if !*is_dirty {
                    let c_fn = &prev_cache.funcs[bi];
                    let trap_h = inc_cache::hash_callee_traps(
                        &c_fn.elide_input.direct_def_callees,
                        n_imports,
                        &elide_out.fn_can_trap,
                    );
                    if c_fn.needs_stack_check != elide_out.needs_stack_check[bi]
                        || c_fn.elided_callee_stack_bytes != elide_out.elided_callee_stack_bytes[bi]
                        || c_fn.callee_trap_hash != trap_h
                    {
                        *is_dirty = true;
                        dirty_count += 1;
                        promoted_elide = true;
                    }
                }
            }
        }
        if promoted_elide {
            continue;
        }
        break (effects, elide_out);
    };
    *d_inline_effects += ti.elapsed();

    // Reserve `final_asms` BEFORE emitting so allocation failure skips emission cleanly.
    let (Some(mut dirty_indices), Some(mut dirty_asms), Some(mut final_asms)) = (
        inc_cache::try_vec_with_capacity(n_defs),
        inc_cache::try_vec_with_capacity::<Option<Asm>>(n_defs),
        inc_cache::try_vec_with_capacity(n_defs),
    ) else {
        return Ok(None);
    };
    for (bi, &is_dirty) in dirty.iter().enumerate() {
        if is_dirty {
            dirty_indices.push(bi);
        }
        dirty_asms.push(None);
    }
    for &bi in &dirty_indices {
        let ir = opt_irs[bi].as_mut().unwrap();
        ir.needs_stack_check = elide_out.needs_stack_check[bi];
        ir.elided_callee_stack_bytes = elide_out.elided_callee_stack_bytes[bi];
    }

    // Emit machine code ONLY for `dirty_indices` with preallocated fallible output slots
    // and `spawn_scoped` fallback.
    if !dirty_indices.is_empty() {
        let Some(mut out_slots_vec) =
            inc_cache::try_vec_with_capacity::<Option<(usize, Asm)>>(dirty_indices.len())
        else {
            return Ok(None);
        };
        for _ in 0..dirty_indices.len() {
            out_slots_vec.push(None);
        }
        let out_slots = Mutex::new(out_slots_vec);
        let next_pos = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
        let fn_can_trap_ref = Some(elide_out.fn_can_trap.as_slice());
        let workers = n_workers.min(dirty_indices.len());
        let run_worker = || {
            let mut loc_em = 0u64;
            loop {
                if failed.load(Ordering::Acquire) {
                    break;
                }
                let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                if pos >= dirty_indices.len() {
                    break;
                }
                let bi = dirty_indices[pos];
                let te = Instant::now();
                let res = lower_ir_to_x64_with_budget(
                    opt_irs[bi].as_ref().unwrap(),
                    opt_allocs[bi].as_ref().unwrap(),
                    layout,
                    ro,
                    opts.isa,
                    fn_can_trap_ref,
                    Some(budget),
                );
                loc_em = loc_em.saturating_add(te.elapsed().as_nanos() as u64);
                match res {
                    Ok(asm) => {
                        out_slots.lock().unwrap_or_else(|g| g.into_inner())[pos] = Some((bi, asm));
                    }
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
        };
        std::thread::scope(|s| {
            for _ in 1..workers {
                let _ = std::thread::Builder::new().spawn_scoped(s, run_worker);
            }
            run_worker();
        });
        if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
            return Err(map_worker_err(err));
        }
        for (bi, asm) in out_slots
            .into_inner()
            .unwrap_or_else(|g| g.into_inner())
            .into_iter()
            .flatten()
        {
            dirty_asms[bi] = Some(asm);
        }
    }

    // Assemble zero-copy `Arc<IncCacheData>` directly (moving both clean `CachedFn`s and dirty
    // `Asm`s with ZERO cloning of 7+ MiB of unchanged machine code!).
    // 1. Charge exact pre-relaxation code/rodata bytes for unchanged functions in deterministic order.
    let mut reused = 0usize;
    for (bi, prev_cached) in prev_cache.funcs.iter().enumerate() {
        if !dirty[bi] {
            budget
                .charge_code(prev_cached.pre_relax_code_len as usize)
                .map_err(map_worker_err)?;
            budget
                .charge_rodata(prev_cached.local_rodata.len())
                .map_err(map_worker_err)?;
            reused += 1;
        }
    }
    let prev_backing = prev_cache.backing.take();
    let prev_backing_slice = prev_backing.as_deref().map(Vec::as_slice);
    let Some(mut updated_funcs) = inc_cache::try_vec_with_capacity::<inc_cache::CachedFn>(n_defs)
    else {
        for (bi, prev_cached) in prev_cache.funcs.drain(..).enumerate() {
            if dirty[bi] {
                final_asms.push(dirty_asms[bi].take().unwrap());
            } else {
                final_asms.push(prev_cached.into_asm(prev_backing_slice));
            }
        }
        return Ok(Some((Some(final_asms), None, reused, None)));
    };
    let mut save_ok = true;
    for (bi, prev_cached) in prev_cache.funcs.drain(..).enumerate() {
        if dirty[bi] {
            let asm = dirty_asms[bi].take().unwrap();
            if save_ok {
                let (inlined_callees, inline_added_insts, was_inlinable) =
                    inline_track_dirty[bi].take().unwrap();
                let own_eff = own_effects_dirty[bi]
                    .take()
                    .unwrap_or(prev_cached.own_effects);
                let elide_in = elide_inputs_dirty[bi]
                    .take()
                    .unwrap_or(prev_cached.elide_input);
                let pre_inline = pre_callees_changed[bi]
                    .take()
                    .unwrap_or(prev_cached.pre_inline_callees);
                let Some(jt_entries) = asm.try_export_jt_entries() else {
                    save_ok = false;
                    // Recover already-moved clean/dirty entries into `final_asms`!
                    for prev_u in updated_funcs.drain(..) {
                        final_asms.push(prev_u.into_asm(prev_backing_slice));
                    }
                    final_asms.push(asm);
                    continue;
                };
                let callee_effects_hash =
                    inc_cache::hash_callee_effects(&own_eff.direct_callees, &effects);
                let callee_trap_hash = inc_cache::hash_callee_traps(
                    &elide_in.direct_def_callees,
                    n_imports,
                    &elide_out.fn_can_trap,
                );
                let pre_relax_code_len = asm.pre_relax_code_len.max(asm.code.len() as u32);
                updated_funcs.push(inc_cache::CachedFn {
                    raw_body_hash: body_hashes[bi],
                    was_inlinable,
                    pre_inline_callees: pre_inline,
                    inlined_callees,
                    inline_added_insts,
                    own_effects: own_eff,
                    elide_input: elide_in,
                    callee_effects_hash,
                    needs_stack_check: elide_out.needs_stack_check[bi],
                    elided_callee_stack_bytes: elide_out.elided_callee_stack_bytes[bi],
                    callee_trap_hash,
                    pre_relax_code_len,
                    code: inc_cache::CachedBytes::Owned(asm.code),
                    relocs: asm.relocs,
                    local_rodata: inc_cache::CachedBytes::Owned(asm.local_rodata),
                    jt_entries,
                    jt_labels: Vec::new(),
                    jt_patches: Vec::new(),
                });
            } else {
                final_asms.push(asm);
            }
        } else if save_ok {
            updated_funcs.push(prev_cached);
        } else {
            final_asms.push(prev_cached.into_asm(prev_backing_slice));
        }
    }

    if save_ok {
        let updated = std::sync::Arc::new(inc_cache::IncCacheData {
            abi_hash,
            global_words: module.globals.len().div_ceil(64),
            hit_inline_cap: false,
            funcs: updated_funcs,
            backing: prev_backing,
        });
        Ok(Some((
            None,
            Some(updated),
            reused,
            Some((topo_bottom_up, in_cycle)),
        )))
    } else {
        Ok(Some((Some(final_asms), None, reused, None)))
    }
}
