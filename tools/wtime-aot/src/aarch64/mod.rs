//! Self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm -> Linux AArch64 (`EM_AARCH64`)
//! AOT compiler (`wtime-aot --backend aarch64`).
//!
//! Shares the target-independent decoding, incremental validation, admission, SSA IR lowering,
//! Stage 2 optimizer (inlining, LICM, GVN/BCE, loop versioning, loop rotation, boundary-clamp
//! peeling, byte-sum & 3-tap blur vectorization, and bit-exact `f64x2`/`f64x4` SLP), and SSA
//! verifier with `--backend x64`, then performs AArch64 register allocation, ARMv8-A/NEON
//! lowering, per-function incremental caching (`a64-inc`), standalone guard-page BCE +
//! WASI/Goose/Ryū runtime emission, and ELF64 `ET_DYN` / `ET_EXEC` linking with zero external
//! toolchain dependencies.

pub mod asm;
pub mod elf;
pub mod inc_cache;
pub mod lower;
pub mod regalloc;
pub mod runtime;
pub mod simd_lower;
pub mod standalone_rt;
#[cfg(test)]
mod tests_aarch64;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use asm::{A64Asm, A64RelocTarget, patch_a64_relocs};
use elf::{A64ElfExecLayout, A64ElfSoLayout};
use inc_cache::{
    A64IncCacheData, CachedA64Fn, compute_a64_abi_hash, load_a64_inc_cache, save_a64_inc_cache,
    try_build_cached_a64_fn,
};
use lower::lower_ir_to_aarch64;
use regalloc::allocate;
use runtime::{
    emit_a64_blot_import_trampoline, emit_a64_exports, emit_a64_helper_host_call,
    emit_a64_helper_read_global, emit_a64_helpers,
};
use standalone_rt::{
    emit_a64_helper_goose_fmt_float, emit_a64_helper_standalone_fs,
    emit_a64_helper_standalone_reserve_copy, emit_a64_standalone_import_trampoline,
    emit_a64_standalone_start,
};

use crate::x64::elf::ExportDef;
use crate::x64::inc_cache::{
    self as x64_inc, ValBodyEntry, ValIndexData, has_val_index, hash_callee_effects,
    load_val_index, save_val_index, scan_wasm_shell_and_body_spans,
};
use crate::x64::ir::{IrFunc, OptIrScratch, lower_wasm_to_ir_with_budget_and_scratch};
use crate::x64::lower::TargetIsa;
use crate::x64::module::{
    self as x64_mod, BodyAdmitSummary, CompileBudget, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES,
    X64Module, X64Target, admit_x64_module_for_target_with_summaries, x64_validate_features,
    x64_validate_limits,
};
use crate::x64::opt::{
    self, FnOwnEffects, InlineMode, Stage2Scratch, compute_module_effects_from_summaries,
    inline_small_helpers, optimize_func_stage2_with_scratch, try_compute_fn_own_effects,
};
use crate::x64::runtime::{EXPORT_NAMES, RuntimeTables, TRAP_MESSAGES, global_kind_code};
use crate::x64::verify::verify_ir;
use crate::x64::{
    CiTableLayout, CtxLayout, HELPER_GOOSE_FMT_FLOAT, HELPER_HOST_CALL, HELPER_READ_GLOBAL,
    HELPER_STANDALONE_FS, HELPER_STANDALONE_RESERVE_COPY, MAX_WASM_BYTES_X64, NUM_HELPERS,
    X64CompileError,
};

/// Compilation options for `--backend aarch64`.
#[derive(Clone, Debug)]
pub struct A64CompileOptions {
    pub target: X64Target,
    pub optimize: bool,
    /// Worker thread count (`0` = automatic up to available CPU cores).
    pub jobs: usize,
    pub deadline: Option<Instant>,
    /// Emit speculation barriers and keep software bounds checks through lowering.
    pub spectre_mitigations: bool,
    /// Optional per-function incremental cache directory (`<cache>/a64-inc`).
    pub inc_cache_dir: Option<PathBuf>,
    /// Generator build fingerprint bound into incremental cache keys.
    pub gen_fingerprint: [u8; 32],
}

impl Default for A64CompileOptions {
    fn default() -> Self {
        Self {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 0,
            deadline: None,
            spectre_mitigations: false,
            inc_cache_dir: None,
            gen_fingerprint: [0u8; 32],
        }
    }
}

fn resolve_a64_jobs(jobs: usize) -> usize {
    if jobs > 0 {
        return jobs.min(64);
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(16)
}

/// Compiled AArch64 artifact details (used by both ELF emission and the in-process AArch64 test oracle).
#[cfg(test)]
pub struct A64CompiledImage {
    pub elf: Vec<u8>,
    pub rodata_vaddr: u64,
    pub text_vaddr: u64,
    pub rodata: Vec<u8>,
    pub text: Vec<u8>,
    pub export_vaddrs: Vec<(String, u64)>,
    pub entry_vaddr: Option<u64>,
    pub inc_reused_funcs: usize,
}

fn align_up(v: usize, a: usize) -> usize {
    let m = a - 1;
    (v + m) & !m
}

fn append_rodata(rodata: &mut Vec<u8>, align: usize, bytes: &[u8]) -> Result<u32, String> {
    let pad = (align - (rodata.len() & (align - 1))) & (align - 1);
    if rodata.len() + pad + bytes.len() > MAX_TOTAL_RODATA_BYTES {
        return Err("aarch64: .rodata exceeds cap".into());
    }
    rodata.resize(rodata.len() + pad, 0);
    let off = rodata.len() as u32;
    rodata.extend_from_slice(bytes);
    Ok(off)
}

/// Compiles a WebAssembly module directly to a Linux AArch64 ELF64 `.so` (`BlotCdylib`)
/// or standalone static executable (`StandaloneExec`).
pub fn compile_wasm_to_aarch64(
    wasm: &[u8],
    opts: &A64CompileOptions,
) -> Result<Vec<u8>, X64CompileError> {
    compile_wasm_to_aarch64_detailed(wasm, opts).map(|d| d.elf)
}

struct A64BuildOutput {
    elf: Vec<u8>,
    #[cfg(test)]
    rodata_vaddr: u64,
    #[cfg(test)]
    text_vaddr: u64,
    #[cfg(test)]
    rodata: Vec<u8>,
    #[cfg(test)]
    text: Vec<u8>,
    #[cfg(test)]
    export_vaddrs: Vec<(String, u64)>,
    #[cfg(test)]
    entry_vaddr: Option<u64>,
    #[cfg(test)]
    inc_reused_funcs: usize,
}

#[cfg(test)]
pub fn compile_wasm_to_aarch64_for_test(
    wasm: &[u8],
    opts: &A64CompileOptions,
) -> Result<A64CompiledImage, X64CompileError> {
    let d = compile_wasm_to_aarch64_detailed(wasm, opts)?;
    Ok(A64CompiledImage {
        elf: d.elf,
        rodata_vaddr: d.rodata_vaddr,
        text_vaddr: d.text_vaddr,
        rodata: d.rodata,
        text: d.text,
        export_vaddrs: d.export_vaddrs,
        entry_vaddr: d.entry_vaddr,
        inc_reused_funcs: d.inc_reused_funcs,
    })
}

enum A64Validated {
    Full(wtime_validate::ValidatedModule),
    Incremental(wtime_validate::IncrementalModule),
}

impl A64Validated {
    fn decoded(&self) -> &wtime_validate::DecodedModule {
        match self {
            A64Validated::Full(v) => v.decoded(),
            A64Validated::Incremental(v) => v.decoded(),
        }
    }

    fn ensure_body_decoded(&mut self, bi: usize) -> Result<(), X64CompileError> {
        match self {
            A64Validated::Full(_) => Ok(()),
            A64Validated::Incremental(v) => v
                .ensure_body_validated(bi)
                .map_err(|e| X64CompileError::Failed(format!("aarch64: validate body {bi}: {e}"))),
        }
    }
}

fn hash_spans_parallel(wasm: &[u8], spans: &[(usize, usize)], n_workers: usize) -> Vec<[u8; 32]> {
    let n = spans.len();
    let mut out = vec![[0u8; 32]; n];
    if n_workers <= 1 || n < 64 {
        for (slot, &(s, e)) in out.iter_mut().zip(spans) {
            *slot = crate::sha256::digest(&wasm[s..e]);
        }
        return out;
    }
    let workers = n_workers.min(16).min(n);
    let per = n.div_ceil(workers);
    std::thread::scope(|sc| {
        let mut rem = out.as_mut_slice();
        let mut idx = 0usize;
        while !rem.is_empty() {
            let take = per.min(rem.len());
            let (slice, rest) = rem.split_at_mut(take);
            let start = idx;
            rem = rest;
            idx += take;
            let mut job = move || {
                for (j, slot) in slice.iter_mut().enumerate() {
                    let (s, e) = spans[start + j];
                    *slot = crate::sha256::digest(&wasm[s..e]);
                }
            };
            if rem.is_empty() {
                job();
            } else {
                let _ = std::thread::Builder::new().spawn_scoped(sc, job);
            }
        }
    });
    // Guarantee every slot is filled (covers rare spawn_scoped failure cleanly).
    for (i, slot) in out.iter_mut().enumerate() {
        if *slot == [0u8; 32] {
            let (s, e) = spans[i];
            *slot = crate::sha256::digest(&wasm[s..e]);
        }
    }
    out
}

fn build_val_body_entry(raw_body_hash: [u8; 32], admit: BodyAdmitSummary) -> ValBodyEntry {
    ValBodyEntry {
        raw_body_hash,
        total_locals: admit.total_locals,
        insn_count: admit.insn_count,
        imm_elems: admit.imm_elems,
        saw_data_count_op: admit.saw_data_count_op,
        uses_exceptions: admit.uses_exceptions,
        uses_bulk_data: admit.uses_bulk_data,
        uses_bulk_elem: admit.uses_bulk_elem,
        dyn_table_ops: admit.dyn_table_ops,
        sig_arity: admit.sig_arity as u32,
        br_table_entries: admit.br_table_entries as u32,
        g_reads: admit.g_reads,
        g_writes: admit.g_writes,
        ci_pairs: admit.ci_pairs,
    }
}

/// Attempts incremental compilation against `prev_cache`; returns `Ok(None)` to fall back to cold compile.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn try_compile_a64_incremental(
    validated: &mut A64Validated,
    module: &X64Module,
    layout: &CtxLayout,
    opts: &A64CompileOptions,
    n_workers: usize,
    abi_hash: [u8; 32],
    body_hashes: &[[u8; 32]],
    prev_cache: A64IncCacheData,
    budget: &CompileBudget,
) -> Result<Option<(Vec<A64Asm>, A64IncCacheData, usize, Vec<usize>, Vec<bool>)>, X64CompileError> {
    let n_defs = body_hashes.len();
    let n_imports = module.imports.len();
    let n_total = n_imports + n_defs;
    let global_words = module.globals.len().div_ceil(64);
    if prev_cache.hit_inline_cap || prev_cache.funcs.len() != n_defs {
        return Ok(None);
    }

    let mut direct_dirty = vec![false; n_defs];
    let mut n_direct_dirty = 0usize;
    for bi in 0..n_defs {
        if body_hashes[bi] != prev_cache.funcs[bi].raw_body_hash {
            direct_dirty[bi] = true;
            n_direct_dirty += 1;
        }
    }
    if n_direct_dirty > n_defs / 2 && n_defs > 8 {
        return Ok(None);
    }

    // 1. Lower `direct_dirty` bodies to compute their new `pre_inline_callees` and inlinability.
    let mut lowered_irs: Vec<Option<IrFunc>> = (0..n_defs).map(|_| None).collect();
    let mut new_pre_callees: Vec<Option<Vec<(u32, u32)>>> = vec![None; n_defs];
    let mut new_inlinable: Vec<bool> = prev_cache.funcs.iter().map(|f| f.was_inlinable).collect();
    {
        for (bi, &is_dirty) in direct_dirty.iter().enumerate() {
            if is_dirty {
                validated.ensure_body_decoded(bi)?;
            }
        }
        let decoded = validated.decoded();
        let mut opt_scratch = OptIrScratch::default();
        for (bi, &is_dirty) in direct_dirty.iter().enumerate() {
            if !is_dirty {
                continue;
            }
            budget.check_deadline().map_err(X64CompileError::Failed)?;
            let mut ir = lower_wasm_to_ir_with_budget_and_scratch(
                decoded,
                module,
                n_imports + bi,
                opts.optimize,
                Some(budget),
                &mut opt_scratch,
            )
            .map_err(X64CompileError::AdmissionRejected)?;
            opt::prune_early_block_params(&mut ir, Some(budget))
                .map_err(X64CompileError::Failed)?;
            let Some((_ic, inl, callees)) =
                opt::try_collect_pre_inline_summary(&ir, n_imports, n_defs)
            else {
                return Ok(None);
            };
            new_inlinable[bi] = inl;
            new_pre_callees[bi] = Some(callees);
            lowered_irs[bi] = Some(ir);
        }
    }

    // 2. Build updated call graph, call counts, SCCs, and transitive `inline_dirty` set.
    let mut old_counts = vec![0u32; n_defs];
    let mut new_counts = vec![0u32; n_defs];
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(n_defs);
    let mut rev_callers: Vec<Vec<usize>> = vec![Vec::new(); n_defs];
    for (bi, prev_fn) in prev_cache.funcs.iter().enumerate() {
        for &(c, cnt) in &prev_fn.pre_inline_callees {
            old_counts[c as usize] = old_counts[c as usize].saturating_add(cnt);
        }
        let cs: &[(u32, u32)] = new_pre_callees[bi]
            .as_deref()
            .unwrap_or(&prev_fn.pre_inline_callees);
        let mut e = Vec::with_capacity(cs.len());
        for &(c, cnt) in cs {
            let cu = c as usize;
            new_counts[cu] = new_counts[cu].saturating_add(cnt);
            e.push(cu);
            rev_callers[cu].push(bi);
        }
        edges.push(e);
    }
    let (in_cycle, topo) = opt::call_graph_sccs_from_edges(&edges);

    // Seed `inline_dirty` with direct edits, call-count threshold changes, or cycle membership changes.
    let mut inline_dirty = direct_dirty.clone();
    let mut wl: Vec<usize> = Vec::new();
    for bi in 0..n_defs {
        let count_changed = old_counts[bi] != new_counts[bi];
        if direct_dirty[bi] || count_changed {
            for &caller in &rev_callers[bi] {
                if !inline_dirty[caller] {
                    inline_dirty[caller] = true;
                    wl.push(caller);
                }
            }
        }
        if !inline_dirty[bi]
            && prev_cache.funcs[bi].inlined_callees.iter().any(|&c| {
                direct_dirty[c as usize] || old_counts[c as usize] != new_counts[c as usize]
            })
        {
            inline_dirty[bi] = true;
            wl.push(bi);
        }
    }
    while let Some(u) = wl.pop() {
        if new_inlinable[u] || prev_cache.funcs[u].was_inlinable {
            for &caller in &rev_callers[u] {
                if !inline_dirty[caller] {
                    inline_dirty[caller] = true;
                    wl.push(caller);
                }
            }
        }
    }

    // 3. Ensure every `inline_dirty` function AND any callee it might inline is lowered!
    let mut need_lower = inline_dirty.clone();
    let mut l_wl: Vec<usize> = (0..n_defs).filter(|&i| inline_dirty[i]).collect();
    while let Some(u) = l_wl.pop() {
        let cs: &[(u32, u32)] = new_pre_callees[u]
            .as_deref()
            .unwrap_or(&prev_cache.funcs[u].pre_inline_callees);
        for &(c, _) in cs {
            let cu = c as usize;
            if !need_lower[cu] && new_inlinable[cu] && !in_cycle[cu] {
                need_lower[cu] = true;
                l_wl.push(cu);
            }
        }
    }
    for bi in 0..n_defs {
        if need_lower[bi] && lowered_irs[bi].is_none() {
            validated.ensure_body_decoded(bi)?;
        }
    }
    {
        let decoded = validated.decoded();
        let mut opt_scratch = OptIrScratch::default();
        for bi in 0..n_defs {
            if need_lower[bi] && lowered_irs[bi].is_none() {
                budget.check_deadline().map_err(X64CompileError::Failed)?;
                let mut ir = lower_wasm_to_ir_with_budget_and_scratch(
                    decoded,
                    module,
                    n_imports + bi,
                    opts.optimize,
                    Some(budget),
                    &mut opt_scratch,
                )
                .map_err(X64CompileError::AdmissionRejected)?;
                opt::prune_early_block_params(&mut ir, Some(budget))
                    .map_err(X64CompileError::Failed)?;
                lowered_irs[bi] = Some(ir);
            }
        }
    }

    // 4. Run bottom-up parallel inlining over the lowered subset (tracking total added insts against cap).
    let reused_added: usize = (0..n_defs)
        .filter(|&i| !inline_dirty[i])
        .map(|i| prev_cache.funcs[i].inline_added_insts as usize)
        .sum();
    let Some(inl_track) = opt::inline_small_helpers_subset_parallel(
        &mut lowered_irs,
        &in_cycle,
        &topo,
        &new_counts,
        module,
        reused_added,
        InlineMode::WarmSpeculative,
        n_workers,
        Some(budget),
    )
    .map_err(X64CompileError::Failed)?
    else {
        return Ok(None);
    };
    if inl_track.hit_module_cap || !inl_track.tracking_complete {
        return Ok(None);
    }

    // 5. Recompute `FnOwnEffects` for `inline_dirty` functions, merge with cached summaries, and run fixpoint.
    let mut own_effects: Vec<FnOwnEffects> = Vec::with_capacity(n_defs);
    for bi in 0..n_defs {
        if inline_dirty[bi] {
            let ir = lowered_irs[bi].as_ref().unwrap();
            let Some(eff) = try_compute_fn_own_effects(ir, module, n_total) else {
                return Ok(None);
            };
            own_effects.push(eff);
        } else {
            own_effects.push(prev_cache.funcs[bi].own_effects.clone());
        }
    }
    let effects = compute_module_effects_from_summaries(&own_effects, module, Some(budget))
        .map_err(X64CompileError::Failed)?;

    // 6. Determine exact `stage2_dirty` set (inlined-dirty OR callee transitive effects changed).
    let mut new_callee_eff_hashes = Vec::with_capacity(n_defs);
    let mut stage2_dirty = vec![false; n_defs];
    let mut dirty_list = Vec::new();
    for bi in 0..n_defs {
        let h = hash_callee_effects(&own_effects[bi].direct_callees, &effects);
        new_callee_eff_hashes.push(h);
        if inline_dirty[bi] || h != prev_cache.funcs[bi].callee_effects_hash {
            if !inline_dirty[bi] && !prev_cache.funcs[bi].inlined_callees.is_empty() {
                // Fall back to cold compile so cached inlined bodies are re-inlined byte-identically.
                return Ok(None);
            }
            stage2_dirty[bi] = true;
            dirty_list.push(bi);
        }
    }

    // Ensure any newly `stage2_dirty` function (not previously lowered) is lowered + inlined.
    for &bi in &dirty_list {
        if lowered_irs[bi].is_none() {
            validated.ensure_body_decoded(bi)?;
            let decoded = validated.decoded();
            let mut opt_scratch = OptIrScratch::default();
            let mut ir = lower_wasm_to_ir_with_budget_and_scratch(
                decoded,
                module,
                n_imports + bi,
                opts.optimize,
                Some(budget),
                &mut opt_scratch,
            )
            .map_err(X64CompileError::AdmissionRejected)?;
            opt::prune_early_block_params(&mut ir, Some(budget))
                .map_err(X64CompileError::Failed)?;
            lowered_irs[bi] = Some(ir);
        }
    }

    // 7. Compile ONLY `dirty_list` through Stage 2 -> verify_ir -> regalloc -> AArch64 lower!
    let mut dirty_asms: Vec<Option<A64Asm>> = (0..n_defs).map(|_| None).collect();
    if !dirty_list.is_empty() {
        let workers = n_workers.min(dirty_list.len());
        if workers <= 1 {
            let mut scratch = Stage2Scratch::default();
            for &bi in &dirty_list {
                let mut ir = lowered_irs[bi].take().unwrap();
                if !opts.spectre_mitigations {
                    optimize_func_stage2_with_scratch(
                        &mut ir,
                        module,
                        &effects,
                        Some(budget),
                        &mut scratch,
                    )
                    .map_err(X64CompileError::Failed)?;
                }
                verify_ir(&ir, module).map_err(X64CompileError::Failed)?;
                let alloc =
                    allocate(&ir, Some(budget)).map_err(X64CompileError::AdmissionRejected)?;
                let fa = lower_ir_to_aarch64(&ir, &alloc, layout)
                    .map_err(X64CompileError::AdmissionRejected)?;
                dirty_asms[bi] = Some(fa);
            }
        } else {
            let ir_slots: Vec<Mutex<Option<IrFunc>>> =
                lowered_irs.into_iter().map(Mutex::new).collect();
            let asm_slots: Vec<Mutex<Option<A64Asm>>> =
                (0..n_defs).map(|_| Mutex::new(None)).collect();
            let next_pos = AtomicUsize::new(0);
            let failed = AtomicBool::new(false);
            let first_err: Mutex<Option<(usize, X64CompileError)>> = Mutex::new(None);
            let run_w = || {
                let mut scratch = Stage2Scratch::default();
                loop {
                    if failed.load(Ordering::Acquire) {
                        break;
                    }
                    let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                    if pos >= dirty_list.len() {
                        break;
                    }
                    let bi = dirty_list[pos];
                    let mut ir = ir_slots[bi]
                        .lock()
                        .unwrap_or_else(|g| g.into_inner())
                        .take()
                        .unwrap();
                    let res = (|| -> Result<A64Asm, X64CompileError> {
                        if !opts.spectre_mitigations {
                            optimize_func_stage2_with_scratch(
                                &mut ir,
                                module,
                                &effects,
                                Some(budget),
                                &mut scratch,
                            )
                            .map_err(X64CompileError::Failed)?;
                        }
                        verify_ir(&ir, module).map_err(X64CompileError::Failed)?;
                        let alloc = allocate(&ir, Some(budget))
                            .map_err(X64CompileError::AdmissionRejected)?;
                        lower_ir_to_aarch64(&ir, &alloc, layout)
                            .map_err(X64CompileError::AdmissionRejected)
                    })();
                    match res {
                        Ok(fa) => {
                            *asm_slots[bi].lock().unwrap_or_else(|g| g.into_inner()) = Some(fa);
                        }
                        Err(err) => {
                            failed.store(true, Ordering::Release);
                            let mut g = first_err.lock().unwrap_or_else(|g| g.into_inner());
                            if g.as_ref().is_none_or(|(prev, _)| bi < *prev) {
                                *g = Some((bi, err));
                            }
                            break;
                        }
                    }
                }
            };
            std::thread::scope(|sc| {
                for _ in 1..workers {
                    let _ = std::thread::Builder::new().spawn_scoped(sc, run_w);
                }
                run_w();
            });
            if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
                return Err(err);
            }
            for &bi in &dirty_list {
                dirty_asms[bi] = asm_slots[bi]
                    .lock()
                    .unwrap_or_else(|g| g.into_inner())
                    .take();
            }
        }
    }

    // 8. Assemble final `fn_asms` and updated `A64IncCacheData` preserving exact `0..n_defs` order.
    let backing = prev_cache.backing.as_deref().map(Vec::as_slice);
    let mut fn_asms = Vec::with_capacity(n_defs);
    let mut new_funcs: Vec<CachedA64Fn> = Vec::with_capacity(n_defs);
    let mut reused = 0usize;
    for (bi, prev_fn) in prev_cache.funcs.into_iter().enumerate() {
        if stage2_dirty[bi] {
            let fa = dirty_asms[bi].take().unwrap();
            let pre_c = new_pre_callees[bi]
                .take()
                .unwrap_or(prev_fn.pre_inline_callees);
            let inl_c = if inline_dirty[bi] {
                inl_track.inlined_callees[bi].clone()
            } else {
                prev_fn.inlined_callees
            };
            let inl_add = if inline_dirty[bi] {
                inl_track.added_insts[bi]
            } else {
                prev_fn.inline_added_insts
            };
            let cf = try_build_cached_a64_fn(
                body_hashes[bi],
                new_inlinable[bi],
                pre_c,
                inl_c,
                inl_add,
                own_effects[bi].clone(),
                new_callee_eff_hashes[bi],
                &fa,
            );
            if let Some(cf) = cf {
                new_funcs.push(cf);
            } else {
                return Ok(None);
            }
            fn_asms.push(fa);
        } else {
            reused += 1;
            let asm = prev_fn.clone().into_asm(backing);
            new_funcs.push(prev_fn);
            fn_asms.push(asm);
        }
    }

    let updated = A64IncCacheData {
        abi_hash,
        global_words,
        hit_inline_cap: false,
        funcs: new_funcs,
        backing: prev_cache.backing,
    };
    Ok(Some((fn_asms, updated, reused, topo, in_cycle)))
}

fn compile_wasm_to_aarch64_detailed(
    wasm: &[u8],
    opts: &A64CompileOptions,
) -> Result<A64BuildOutput, X64CompileError> {
    if wasm.len() > MAX_WASM_BYTES_X64 {
        return Err(X64CompileError::AdmissionRejected(format!(
            "aarch64: module size ({} B) exceeds cap ({MAX_WASM_BYTES_X64} B)",
            wasm.len()
        )));
    }
    let budget = CompileBudget::new(opts.deadline);
    budget.check_deadline().map_err(X64CompileError::Failed)?;

    let inc_enabled = opts.inc_cache_dir.is_some() && opts.optimize;
    let mut vlimits = x64_validate_limits();
    vlimits.compile_threads = if wasm.len() >= 256 * 1024 {
        resolve_a64_jobs(opts.jobs).min(16)
    } else {
        1
    };

    // Optional `.vidx` incremental validation fast path (shares x64's exact `.vidx` format in `a64-inc`!).
    let mut shell_hash_opt: Option<[u8; 32]> = None;
    let mut pre_body_hashes: Option<Vec<[u8; 32]>> = None;
    let mut pre_vidx: Option<ValIndexData> = None;
    let mut pre_admit_skip: Vec<bool> = Vec::new();

    if inc_enabled
        && let Some(cache_dir) = opts.inc_cache_dir.as_deref()
        && let Some((sh, spans)) = scan_wasm_shell_and_body_spans(
            wasm,
            &opts.gen_fingerprint,
            TargetIsa::default(),
            opts.optimize,
            true,
            opts.target,
        )
    {
        shell_hash_opt = Some(sh);
        let n_hint = spans.len();
        if n_hint > 0 && has_val_index(cache_dir, &sh) {
            let hashes = hash_spans_parallel(wasm, &spans, resolve_a64_jobs(opts.jobs));
            if let Some(vidx) = load_val_index(cache_dir, &sh, n_hint) {
                let changed = hashes
                    .iter()
                    .zip(&vidx.bodies)
                    .filter(|(bh, vb)| *bh != &vb.raw_body_hash)
                    .count();
                if changed <= n_hint / 2 {
                    pre_vidx = Some(vidx);
                }
            }
            pre_body_hashes = Some(hashes);
        }
    }

    let mut validated: A64Validated = if let (Some(vidx), Some(bhs)) =
        (pre_vidx.as_ref(), pre_body_hashes.as_ref())
    {
        let bodies_ref = &vidx.bodies;
        let inc_mod = wtime_validate::validate_incremental(
            wasm,
            &x64_validate_features(),
            &vlimits,
            |idx, _slice| {
                let vb = bodies_ref.get(idx)?;
                if bhs.get(idx)? == &vb.raw_body_hash {
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
        .map_err(|e| X64CompileError::Failed(format!("aarch64: validation failed: {e}")))?;
        pre_admit_skip = (0..bodies_ref.len())
            .map(|idx| !inc_mod.is_body_validated(idx))
            .collect();
        A64Validated::Incremental(inc_mod)
    } else {
        A64Validated::Full(
            wtime_validate::validate(wasm, &x64_validate_features(), &vlimits)
                .map_err(|e| X64CompileError::Failed(format!("aarch64: validation failed: {e}")))?,
        )
    };

    let pre_vidx_admit = pre_vidx
        .as_ref()
        .filter(|v| pre_admit_skip.len() == v.bodies.len())
        .map(|v| (v.bodies.as_slice(), pre_admit_skip.as_slice()));
    let (module, mut admit_summaries) = admit_x64_module_for_target_with_summaries(
        validated.decoded(),
        opts.target,
        Some(&budget),
        pre_vidx_admit,
        inc_enabled,
    )
    .map_err(X64CompileError::AdmissionRejected)?;

    // 1. Build shared `.rodata` tables (trap messages, call_indirect slots BEFORE data segments
    //    so edits to data segments do not shift `ci_tables` offsets).
    let mut rodata = Vec::with_capacity(4096);
    let mut trap_str_offs = [0u32; 22];
    for (i, msg) in TRAP_MESSAGES.iter().enumerate() {
        let mut s = msg.as_bytes().to_vec();
        s.push(0);
        trap_str_offs[i] = append_rodata(&mut rodata, 1, &s).map_err(X64CompileError::Failed)?;
    }
    let trap_table_off =
        append_rodata(&mut rodata, 8, &[0u8; 22 * 8]).map_err(X64CompileError::Failed)?;
    for (i, msg) in TRAP_MESSAGES.iter().enumerate() {
        let rel = (trap_str_offs[i] as i32) - (trap_table_off as i32);
        let pos = (trap_table_off as usize) + i * 8;
        rodata[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
        rodata[pos + 4..pos + 8].copy_from_slice(&(msg.len() as u32).to_le_bytes());
    }

    let mut ci_tables = Vec::with_capacity(module.ci_sigs.len());
    let mut ci_patch_info: Vec<(u32, Vec<Option<u32>>)> = Vec::with_capacity(module.ci_sigs.len());
    for sig in &module.ci_sigs {
        let (roff, slice) = if sig.is_dynamic || sig.is_call_ref || sig.span == 0 {
            (0, Vec::new())
        } else {
            let mut sl = vec![None; sig.span as usize];
            for &(slot, fidx) in &sig.targets {
                sl[(slot - sig.min_slot) as usize] = Some(fidx);
            }
            let roff = append_rodata(&mut rodata, 4, &vec![0u8; (sig.span as usize) * 4])
                .map_err(X64CompileError::Failed)?;
            (roff, sl)
        };
        ci_tables.push(CiTableLayout {
            table_len: sig.table_len,
            min_slot: sig.min_slot,
            span: sig.span,
            rodata_off: roff,
            dyn_table_desc_off: -1,
            canon_sig: sig.canon_sig,
            is_call_ref: sig.is_call_ref,
            table_is64: sig.table_is64,
        });
        ci_patch_info.push((roff, slice));
    }

    let func_sig_ro_off = if module.uses_dyn_ci {
        let mut sig_bytes = Vec::with_capacity(module.func_canon_sig.len() * 4);
        for &cs in &module.func_canon_sig {
            sig_bytes.extend_from_slice(&cs.to_le_bytes());
        }
        append_rodata(&mut rodata, 4, &sig_bytes).map_err(X64CompileError::Failed)?
    } else {
        0
    };
    let func_addr_ro_off = if module.uses_dyn_ci || module.uses_call_ref {
        append_rodata(&mut rodata, 4, &vec![0u8; module.func_canon_sig.len() * 4])
            .map_err(X64CompileError::Failed)?
    } else {
        0
    };

    let empty_off = append_rodata(&mut rodata, 1, &[0]).map_err(X64CompileError::Failed)?;
    let mut data_segs = Vec::with_capacity(module.active_data_segs.len());
    for &(mem_idx, mem_off, data_idx) in &module.active_data_segs {
        let span = validated.decoded().data[data_idx].bytes;
        let bytes = &wasm[span.start..span.end];
        let roff = if bytes.is_empty() {
            empty_off
        } else {
            append_rodata(&mut rodata, 1, bytes).map_err(X64CompileError::Failed)?
        };
        data_segs.push((mem_idx, mem_off, roff, bytes.len() as u32));
    }

    let passive_data_table_off = if module.uses_bulk_data && module.data_seg_count > 0 {
        let mut entries = vec![(empty_off, 0u32); module.data_seg_count];
        for &di in &module.passive_data_segs {
            let span = validated.decoded().data[di].bytes;
            let bytes = &wasm[span.start..span.end];
            let roff = if bytes.is_empty() {
                empty_off
            } else {
                append_rodata(&mut rodata, 1, bytes).map_err(X64CompileError::Failed)?
            };
            entries[di] = (roff, bytes.len() as u32);
        }
        let tbl_off = append_rodata(&mut rodata, 8, &vec![0u8; entries.len() * 8])
            .map_err(X64CompileError::Failed)?;
        for (i, &(roff, len)) in entries.iter().enumerate() {
            let rel = (roff as i64 - tbl_off as i64) as i32;
            let pos = (tbl_off as usize) + i * 8;
            rodata[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
            rodata[pos + 4..pos + 8].copy_from_slice(&len.to_le_bytes());
        }
        tbl_off
    } else {
        0
    };

    let mut active_dyn_elem_inits = Vec::new();
    let mut passive_elem_entries = if module.uses_bulk_elem {
        vec![(empty_off, 0u32); module.elem_segs.len()]
    } else {
        Vec::new()
    };
    for (ei, seg) in module.elem_segs.iter().enumerate() {
        let need_active_dyn = seg
            .active_target
            .is_some_and(|(tbl, _)| module.dyn_tables.contains_key(&tbl));
        let need_passive = module.uses_bulk_elem && seg.is_passive;
        if !need_active_dyn && !need_passive {
            continue;
        }
        let len = seg.entries.len() as u32;
        let roff = if seg.entries.is_empty() {
            empty_off
        } else {
            let mut buf = Vec::with_capacity(seg.entries.len() * 4);
            for &enc in &seg.entries {
                buf.extend_from_slice(&enc.to_le_bytes());
            }
            append_rodata(&mut rodata, 4, &buf).map_err(X64CompileError::Failed)?
        };
        if let Some((tbl, dst_off)) = seg.active_target
            && need_active_dyn
        {
            active_dyn_elem_inits.push((tbl, dst_off, roff, len));
        }
        if need_passive {
            passive_elem_entries[ei] = (roff, len);
        }
    }
    let elem_seg_table_off = if module.uses_bulk_elem && !module.elem_segs.is_empty() {
        let tbl_off = append_rodata(&mut rodata, 8, &vec![0u8; passive_elem_entries.len() * 8])
            .map_err(X64CompileError::Failed)?;
        for (i, &(roff, len)) in passive_elem_entries.iter().enumerate() {
            let rel = (roff as i64 - tbl_off as i64) as i32;
            let pos = (tbl_off as usize) + i * 8;
            rodata[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
            rodata[pos + 4..pos + 8].copy_from_slice(&len.to_le_bytes());
        }
        tbl_off
    } else {
        0
    };

    let mut layout = CtxLayout::compute(&module, ci_tables)
        .map_err(X64CompileError::AdmissionRejected)?
        .with_spectre_mitigations(opts.spectre_mitigations);
    layout.func_sig_ro_off = func_sig_ro_off;
    layout.func_addr_ro_off = func_addr_ro_off;

    let (
        imp_table_off,
        imp_count,
        get_global_table_off,
        get_global_count,
        set_global_table_off,
        set_global_count,
    ) = if opts.target == X64Target::BlotCdylib {
        let push_ro_str = |rodata: &mut Vec<u8>, s: &str| -> Result<(u32, u32), X64CompileError> {
            let len = u32::try_from(s.len()).map_err(|_| {
                X64CompileError::Failed("aarch64: rodata string exceeds u32".into())
            })?;
            if s.is_empty() {
                Ok((empty_off, 0))
            } else {
                let mut bytes = Vec::with_capacity(s.len() + 1);
                bytes.extend_from_slice(s.as_bytes());
                bytes.push(0);
                let off = append_rodata(rodata, 1, &bytes).map_err(X64CompileError::Failed)?;
                Ok((off, len))
            }
        };

        let mut imp_offs = Vec::with_capacity(module.imports.len());
        for (m, n) in &module.imports {
            imp_offs.push((push_ro_str(&mut rodata, m)?, push_ro_str(&mut rodata, n)?));
        }
        let imp_count = u32::try_from(imp_offs.len())
            .map_err(|_| X64CompileError::Failed("aarch64: import count exceeds u32".into()))?;
        let imp_table_off = if imp_offs.is_empty() {
            0
        } else {
            let tbl = append_rodata(&mut rodata, 4, &vec![0u8; imp_offs.len() * 16])
                .map_err(X64CompileError::Failed)?;
            for (i, &((moff, mlen), (noff, nlen))) in imp_offs.iter().enumerate() {
                let mrel = i32::try_from(i64::from(moff) - i64::from(tbl)).map_err(|_| {
                    X64CompileError::Failed("aarch64: import mod rel32 overflow".into())
                })?;
                let nrel = i32::try_from(i64::from(noff) - i64::from(tbl)).map_err(|_| {
                    X64CompileError::Failed("aarch64: import name rel32 overflow".into())
                })?;
                let pos = (tbl as usize) + i * 16;
                rodata[pos..pos + 4].copy_from_slice(&mrel.to_le_bytes());
                rodata[pos + 4..pos + 8].copy_from_slice(&mlen.to_le_bytes());
                rodata[pos + 8..pos + 12].copy_from_slice(&nrel.to_le_bytes());
                rodata[pos + 12..pos + 16].copy_from_slice(&nlen.to_le_bytes());
            }
            tbl
        };

        let mut get_global_items = Vec::new();
        let mut set_global_items = Vec::new();
        for (name, gi, vt, mutable) in &module.exported_globals {
            let Some(kind) = global_kind_code(*vt) else {
                continue;
            };
            let (noff, nlen) = push_ro_str(&mut rodata, name)?;
            match module.global_consts[*gi] {
                Some(c) => {
                    let bits: u64 = match c {
                        x64_mod::X64ConstVal::I32(v) => u64::from(v as u32),
                        x64_mod::X64ConstVal::I64(v) => v as u64,
                        x64_mod::X64ConstVal::F32(b) => u64::from(b),
                        x64_mod::X64ConstVal::F64(b) => b,
                        x64_mod::X64ConstVal::RefNull => 0,
                        x64_mod::X64ConstVal::FuncRef(f) => u64::from(f) + 1,
                        x64_mod::X64ConstVal::V128(_) => continue,
                    };
                    get_global_items.push((noff, nlen, kind, 1u8, 0i32, bits));
                }
                None => {
                    let ctx_off = layout.global_offs[*gi];
                    get_global_items.push((noff, nlen, kind, 0u8, ctx_off, 0u64));
                    if *mutable {
                        let is64 = u8::from(!matches!(
                            vt,
                            wtime_types::ValType::I32 | wtime_types::ValType::F32
                        ));
                        set_global_items.push((noff, nlen, ctx_off, is64));
                    }
                }
            }
        }
        let get_global_count = u32::try_from(get_global_items.len()).map_err(|_| {
            X64CompileError::Failed("aarch64: global export count exceeds u32".into())
        })?;
        let get_global_table_off = if get_global_items.is_empty() {
            0
        } else {
            let tbl = append_rodata(&mut rodata, 8, &vec![0u8; get_global_items.len() * 24])
                .map_err(X64CompileError::Failed)?;
            for (i, &(noff, nlen, kind, is_const, ctx_off, const_bits)) in
                get_global_items.iter().enumerate()
            {
                let nrel = i32::try_from(i64::from(noff) - i64::from(tbl)).map_err(|_| {
                    X64CompileError::Failed("aarch64: get_global name rel32 overflow".into())
                })?;
                let pos = (tbl as usize) + i * 24;
                rodata[pos..pos + 4].copy_from_slice(&nrel.to_le_bytes());
                rodata[pos + 4..pos + 8].copy_from_slice(&nlen.to_le_bytes());
                rodata[pos + 8] = kind;
                rodata[pos + 9] = is_const;
                rodata[pos + 12..pos + 16].copy_from_slice(&ctx_off.to_le_bytes());
                rodata[pos + 16..pos + 24].copy_from_slice(&const_bits.to_le_bytes());
            }
            tbl
        };

        let set_global_count = u32::try_from(set_global_items.len()).map_err(|_| {
            X64CompileError::Failed("aarch64: mutable global export count exceeds u32".into())
        })?;
        let set_global_table_off = if set_global_items.is_empty() {
            0
        } else {
            let tbl = append_rodata(&mut rodata, 4, &vec![0u8; set_global_items.len() * 16])
                .map_err(X64CompileError::Failed)?;
            for (i, &(noff, nlen, ctx_off, is64)) in set_global_items.iter().enumerate() {
                let nrel = i32::try_from(i64::from(noff) - i64::from(tbl)).map_err(|_| {
                    X64CompileError::Failed("aarch64: set_global name rel32 overflow".into())
                })?;
                let pos = (tbl as usize) + i * 16;
                rodata[pos..pos + 4].copy_from_slice(&nrel.to_le_bytes());
                rodata[pos + 4..pos + 8].copy_from_slice(&nlen.to_le_bytes());
                rodata[pos + 8..pos + 12].copy_from_slice(&ctx_off.to_le_bytes());
                rodata[pos + 12] = is64;
            }
            tbl
        };

        (
            imp_table_off,
            imp_count,
            get_global_table_off,
            get_global_count,
            set_global_table_off,
            set_global_count,
        )
    } else {
        (0, 0, 0, 0, 0, 0)
    };

    let tables = RuntimeTables {
        trap_table_off,
        imp_table_off,
        imp_count,
        call_export_table_off: 0,
        call_export_count: 0,
        get_global_table_off,
        get_global_count,
        set_global_table_off,
        set_global_count,
        data_segs,
        v128_global_inits: Vec::new(),
        passive_data_table_off,
        elem_seg_table_off,
        active_dyn_elem_inits,
        func_sig_ro_off,
        func_addr_ro_off,
    };

    // 2. Try incremental compilation against `a64-inc`, falling back cleanly to full parallel cold compile.
    let n_defs = validated.decoded().code.len();
    let n_imports = module.imports.len();
    let n_total = n_imports + n_defs;
    let n_workers = resolve_a64_jobs(opts.jobs).min(n_defs.max(1));
    let global_words = module.globals.len().div_ceil(64);
    let abi_hash = compute_a64_abi_hash(
        &opts.gen_fingerprint,
        opts.optimize,
        opts.spectre_mitigations,
        n_defs,
        &module,
        &layout,
    );

    let mut inc_reused_funcs = 0usize;
    let mut fn_asms_opt: Option<Vec<A64Asm>> = None;
    let mut pending_cache_save: Option<(PathBuf, A64IncCacheData, Option<ValIndexData>)> = None;

    let body_hashes_opt: Option<Vec<[u8; 32]>> = if inc_enabled && n_defs > 0 {
        if let Some(h) = pre_body_hashes.take() {
            Some(h)
        } else {
            let dec = validated.decoded();
            let spans: Vec<(usize, usize)> = dec
                .code
                .iter()
                .map(|b| (b.span.start, b.span.end))
                .collect();
            Some(hash_spans_parallel(wasm, &spans, n_workers))
        }
    } else {
        None
    };

    if inc_enabled
        && n_defs > 0
        && let Some(cache_dir) = opts.inc_cache_dir.as_deref()
        && let Some(ref bhs) = body_hashes_opt
        && let Some(prev_cache) =
            load_a64_inc_cache(cache_dir, &abi_hash, n_defs, n_total, global_words)
        && let Some((asms, updated_cache, reused, topo_bu, in_cyc)) = try_compile_a64_incremental(
            &mut validated,
            &module,
            &layout,
            opts,
            n_workers,
            abi_hash,
            bhs,
            prev_cache,
            &budget,
        )?
    {
        inc_reused_funcs = reused;
        fn_asms_opt = Some(asms);
        let vidx_opt = if let Some(sh) = shell_hash_opt
            && admit_summaries.len() == n_defs
        {
            let mut prev_bodies = pre_vidx.take().map(|v| v.bodies);
            let mut v_bodies = Vec::with_capacity(n_defs);
            for (bi, a) in admit_summaries.drain(..).enumerate() {
                if let Some(ref mut pbs) = prev_bodies
                    && pre_admit_skip.get(bi).copied().unwrap_or(false)
                    && pbs[bi].raw_body_hash == bhs[bi]
                {
                    v_bodies.push(std::mem::replace(
                        &mut pbs[bi],
                        ValBodyEntry {
                            raw_body_hash: [0u8; 32],
                            total_locals: 0,
                            insn_count: 0,
                            imm_elems: 0,
                            saw_data_count_op: false,
                            uses_exceptions: false,
                            uses_bulk_data: false,
                            uses_bulk_elem: false,
                            dyn_table_ops: Vec::new(),
                            sig_arity: 0,
                            br_table_entries: 0,
                            g_reads: Vec::new(),
                            g_writes: Vec::new(),
                            ci_pairs: Vec::new(),
                        },
                    ));
                } else {
                    v_bodies.push(build_val_body_entry(bhs[bi], a));
                }
            }
            Some(ValIndexData {
                shell_hash: sh,
                abi_hash,
                bodies: v_bodies,
                topo_bottom_up: topo_bu,
                in_cycle: in_cyc,
            })
        } else {
            None
        };
        pending_cache_save = Some((cache_dir.to_path_buf(), updated_cache, vidx_opt));
    }

    let mut fn_asms: Vec<A64Asm> = if let Some(asms) = fn_asms_opt {
        asms
    } else {
        // Cold path: ensure every body is decoded (if incremental validation skipped any).
        for bi in 0..n_defs {
            validated.ensure_body_decoded(bi)?;
        }
        let decoded = validated.decoded();

        let mut work_order: Vec<usize> = (0..n_defs).collect();
        if n_workers > 1 && n_defs > 1 {
            work_order.sort_unstable_by(|&a, &b| {
                decoded.code[b]
                    .instructions
                    .len()
                    .cmp(&decoded.code[a].instructions.len())
                    .then_with(|| a.cmp(&b))
            });
        }

        let mut irs: Vec<IrFunc> = if n_workers <= 1 || n_defs <= 1 {
            let mut out = Vec::with_capacity(n_defs);
            let mut opt_scratch = OptIrScratch::default();
            for di in 0..n_defs {
                budget.check_deadline().map_err(X64CompileError::Failed)?;
                let mut ir = lower_wasm_to_ir_with_budget_and_scratch(
                    decoded,
                    &module,
                    n_imports + di,
                    opts.optimize,
                    Some(&budget),
                    &mut opt_scratch,
                )
                .map_err(X64CompileError::AdmissionRejected)?;
                if opts.optimize {
                    opt::prune_early_block_params(&mut ir, Some(&budget))
                        .map_err(X64CompileError::Failed)?;
                }
                out.push(ir);
            }
            out
        } else {
            let ir_slots: Vec<Mutex<Option<IrFunc>>> =
                (0..n_defs).map(|_| Mutex::new(None)).collect();
            let next_pos = AtomicUsize::new(0);
            let failed = AtomicBool::new(false);
            let first_err: Mutex<Option<(usize, X64CompileError)>> = Mutex::new(None);
            let run_lower = || {
                let mut opt_scratch = OptIrScratch::default();
                loop {
                    if failed.load(Ordering::Acquire) {
                        break;
                    }
                    let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                    if pos >= n_defs {
                        break;
                    }
                    let di = work_order[pos];
                    let res = budget
                        .check_deadline()
                        .map_err(X64CompileError::Failed)
                        .and_then(|()| {
                            let mut ir = lower_wasm_to_ir_with_budget_and_scratch(
                                decoded,
                                &module,
                                n_imports + di,
                                opts.optimize,
                                Some(&budget),
                                &mut opt_scratch,
                            )
                            .map_err(X64CompileError::AdmissionRejected)?;
                            if opts.optimize {
                                opt::prune_early_block_params(&mut ir, Some(&budget))
                                    .map_err(X64CompileError::Failed)?;
                            }
                            Ok(ir)
                        });
                    match res {
                        Ok(ir) => {
                            *ir_slots[di].lock().unwrap_or_else(|g| g.into_inner()) = Some(ir);
                        }
                        Err(err) => {
                            failed.store(true, Ordering::Release);
                            let mut g = first_err.lock().unwrap_or_else(|g| g.into_inner());
                            if g.as_ref().is_none_or(|(prev, _)| di < *prev) {
                                *g = Some((di, err));
                            }
                            break;
                        }
                    }
                }
            };
            std::thread::scope(|s| {
                for _ in 1..n_workers {
                    let _ = std::thread::Builder::new().spawn_scoped(s, run_lower);
                }
                run_lower();
            });
            if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
                return Err(err);
            }
            ir_slots
                .into_iter()
                .map(|m| m.into_inner().unwrap_or_else(|g| g.into_inner()).unwrap())
                .collect()
        };

        let mut pre_callees_save: Vec<Vec<(u32, u32)>> = Vec::new();
        let mut pre_inlinable_save: Vec<bool> = Vec::new();
        let mut inl_callees_cap: Vec<Vec<u32>> = Vec::new();
        let mut inl_added_cap: Vec<u32> = Vec::new();
        let mut hit_inline_cap = false;
        let mut tracking_ok = false;
        let mut topo_cycle_save: Option<(Vec<usize>, Vec<bool>)> = None;

        let (effects, own_effs_save) = if opts.optimize && n_defs > 0 {
            let mut pre_inst_counts: Vec<usize> = Vec::with_capacity(n_defs);
            let mut pre_was_inlinable: Vec<bool> = Vec::with_capacity(n_defs);
            let mut pre_callees: Vec<Vec<(u32, u32)>> = Vec::with_capacity(n_defs);
            let mut ok_pre = true;
            for ir in &irs {
                if let Some((ic, inl, cs)) =
                    opt::try_collect_pre_inline_summary(ir, n_imports, n_defs)
                {
                    pre_inst_counts.push(ic);
                    pre_was_inlinable.push(inl);
                    pre_callees.push(cs);
                } else {
                    ok_pre = false;
                    break;
                }
            }
            if ok_pre {
                let mut counts = vec![0u32; n_defs];
                let mut edges: Vec<Vec<usize>> = Vec::with_capacity(n_defs);
                for cs in &pre_callees {
                    edges.push(cs.iter().map(|&(c, _)| c as usize).collect());
                    for &(c, cnt) in cs {
                        counts[c as usize] = counts[c as usize].saturating_add(cnt);
                    }
                }
                let (in_cycle, topo) = opt::call_graph_sccs_from_edges(&edges);
                let mut opt_irs: Vec<Option<IrFunc>> = irs.into_iter().map(Some).collect();
                let mode = if inc_enabled {
                    InlineMode::ColdCaptureCache
                } else {
                    InlineMode::ColdNoCache
                };
                let res = opt::inline_small_helpers_subset_parallel_with_pre(
                    &mut opt_irs,
                    &in_cycle,
                    &topo,
                    &counts,
                    &module,
                    0,
                    mode,
                    n_workers,
                    Some(&pre_callees),
                    Some(&pre_inst_counts),
                    Some(&pre_was_inlinable),
                    Some(&budget),
                )
                .map_err(X64CompileError::Failed)?;
                if let Some(track) = res {
                    hit_inline_cap = track.hit_module_cap;
                    tracking_ok = track.tracking_complete;
                    inl_callees_cap = track.inlined_callees;
                    inl_added_cap = track.added_insts;
                    if inc_enabled && tracking_ok {
                        pre_callees_save = pre_callees;
                        pre_inlinable_save = pre_was_inlinable;
                        topo_cycle_save = Some((topo, in_cycle));
                    }
                }
                irs = opt_irs.into_iter().map(|o| o.unwrap()).collect();
            } else {
                inline_small_helpers(&mut irs, &module, Some(&budget))
                    .map_err(X64CompileError::Failed)?;
                hit_inline_cap = true;
            }
            let mut own_effs = Vec::with_capacity(n_defs);
            for ir in &irs {
                let s = try_compute_fn_own_effects(ir, &module, n_total).ok_or_else(|| {
                    X64CompileError::Failed("aarch64: out of memory computing effects".into())
                })?;
                own_effs.push(s);
            }
            let eff = compute_module_effects_from_summaries(&own_effs, &module, Some(&budget))
                .map_err(X64CompileError::Failed)?;
            (Some(eff), Some(own_effs))
        } else {
            (None, None)
        };

        let compiled_asms: Vec<A64Asm> = if n_workers <= 1 || n_defs <= 1 {
            let mut out = Vec::with_capacity(n_defs);
            let mut scratch = Stage2Scratch::default();
            for mut ir in irs {
                // Stage 2 can prove bounds from branches that have no barrier.
                if !opts.spectre_mitigations
                    && let Some(ref eff) = effects
                {
                    optimize_func_stage2_with_scratch(
                        &mut ir,
                        &module,
                        eff,
                        Some(&budget),
                        &mut scratch,
                    )
                    .map_err(X64CompileError::Failed)?;
                }
                verify_ir(&ir, &module).map_err(X64CompileError::Failed)?;
                let alloc =
                    allocate(&ir, Some(&budget)).map_err(X64CompileError::AdmissionRejected)?;
                let fa = lower_ir_to_aarch64(&ir, &alloc, &layout)
                    .map_err(X64CompileError::AdmissionRejected)?;
                out.push(fa);
            }
            out
        } else {
            let ir_slots: Vec<Mutex<Option<IrFunc>>> =
                irs.into_iter().map(|ir| Mutex::new(Some(ir))).collect();
            let asm_slots: Vec<Mutex<Option<A64Asm>>> =
                (0..n_defs).map(|_| Mutex::new(None)).collect();
            let next_pos = AtomicUsize::new(0);
            let failed = AtomicBool::new(false);
            let first_err: Mutex<Option<(usize, X64CompileError)>> = Mutex::new(None);
            let eff_ref = effects.as_ref();
            let run_compile = || {
                let mut scratch = Stage2Scratch::default();
                loop {
                    if failed.load(Ordering::Acquire) {
                        break;
                    }
                    let pos = next_pos.fetch_add(1, Ordering::Relaxed);
                    if pos >= n_defs {
                        break;
                    }
                    let di = work_order[pos];
                    let mut ir = ir_slots[di]
                        .lock()
                        .unwrap_or_else(|g| g.into_inner())
                        .take()
                        .unwrap();
                    let res = (|| -> Result<A64Asm, X64CompileError> {
                        if !opts.spectre_mitigations
                            && let Some(eff) = eff_ref
                        {
                            optimize_func_stage2_with_scratch(
                                &mut ir,
                                &module,
                                eff,
                                Some(&budget),
                                &mut scratch,
                            )
                            .map_err(X64CompileError::Failed)?;
                        }
                        verify_ir(&ir, &module).map_err(X64CompileError::Failed)?;
                        let alloc = allocate(&ir, Some(&budget))
                            .map_err(X64CompileError::AdmissionRejected)?;
                        lower_ir_to_aarch64(&ir, &alloc, &layout)
                            .map_err(X64CompileError::AdmissionRejected)
                    })();
                    match res {
                        Ok(fa) => {
                            *asm_slots[di].lock().unwrap_or_else(|g| g.into_inner()) = Some(fa);
                        }
                        Err(err) => {
                            failed.store(true, Ordering::Release);
                            let mut g = first_err.lock().unwrap_or_else(|g| g.into_inner());
                            if g.as_ref().is_none_or(|(prev, _)| di < *prev) {
                                *g = Some((di, err));
                            }
                            break;
                        }
                    }
                }
            };
            std::thread::scope(|s| {
                for _ in 1..n_workers {
                    let _ = std::thread::Builder::new().spawn_scoped(s, run_compile);
                }
                run_compile();
            });
            if let Some((_, err)) = first_err.into_inner().unwrap_or_else(|g| g.into_inner()) {
                return Err(err);
            }
            asm_slots
                .into_iter()
                .map(|m| m.into_inner().unwrap_or_else(|g| g.into_inner()).unwrap())
                .collect()
        };

        // Snapshot cold build into `a64-inc` + `.vidx` so the next 1-fn edit hits warm cache!
        if inc_enabled
            && tracking_ok
            && n_defs > 0
            && let Some(cache_dir) = opts.inc_cache_dir.as_deref()
            && let (Some(bhs), Some(eff), Some(own_effs)) =
                (body_hashes_opt.as_ref(), effects.as_ref(), own_effs_save)
            && pre_callees_save.len() == n_defs
            && pre_inlinable_save.len() == n_defs
        {
            let mut cached_funcs = Vec::with_capacity(n_defs);
            let mut all_ok = true;
            for bi in 0..n_defs {
                let ch = hash_callee_effects(&own_effs[bi].direct_callees, eff);
                let inl_c = inl_callees_cap.get(bi).cloned().unwrap_or_default();
                let inl_a = inl_added_cap.get(bi).copied().unwrap_or(0);
                if let Some(cf) = try_build_cached_a64_fn(
                    bhs[bi],
                    pre_inlinable_save[bi],
                    std::mem::take(&mut pre_callees_save[bi]),
                    inl_c,
                    inl_a,
                    own_effs[bi].clone(),
                    ch,
                    &compiled_asms[bi],
                ) {
                    cached_funcs.push(cf);
                } else {
                    all_ok = false;
                    break;
                }
            }
            if all_ok {
                let data = A64IncCacheData {
                    abi_hash,
                    global_words,
                    hit_inline_cap,
                    funcs: cached_funcs,
                    backing: None,
                };
                let vidx_opt = if let (Some(sh), Some((topo_bu, in_cyc))) =
                    (shell_hash_opt, topo_cycle_save)
                    && admit_summaries.len() == n_defs
                {
                    let mut prev_bodies = pre_vidx.take().map(|v| v.bodies);
                    let mut v_bodies = Vec::with_capacity(n_defs);
                    for (bi, a) in admit_summaries.drain(..).enumerate() {
                        if let Some(ref mut pbs) = prev_bodies
                            && pre_admit_skip.get(bi).copied().unwrap_or(false)
                            && pbs[bi].raw_body_hash == bhs[bi]
                        {
                            v_bodies.push(std::mem::replace(
                                &mut pbs[bi],
                                ValBodyEntry {
                                    raw_body_hash: [0u8; 32],
                                    total_locals: 0,
                                    insn_count: 0,
                                    imm_elems: 0,
                                    saw_data_count_op: false,
                                    uses_exceptions: false,
                                    uses_bulk_data: false,
                                    uses_bulk_elem: false,
                                    dyn_table_ops: Vec::new(),
                                    sig_arity: 0,
                                    br_table_entries: 0,
                                    g_reads: Vec::new(),
                                    g_writes: Vec::new(),
                                    ci_pairs: Vec::new(),
                                },
                            ));
                        } else {
                            v_bodies.push(build_val_body_entry(bhs[bi], a));
                        }
                    }
                    Some(ValIndexData {
                        shell_hash: sh,
                        abi_hash,
                        bodies: v_bodies,
                        topo_bottom_up: topo_bu,
                        in_cycle: in_cyc,
                    })
                } else {
                    None
                };
                pending_cache_save = Some((cache_dir.to_path_buf(), data, vidx_opt));
            }
        }

        compiled_asms
    };

    // 3. Emit helpers, standalone import trampolines, and either Blot ABI 8 exports or standalone `_start`.
    let mut helper_asms =
        emit_a64_helpers(&module, &layout, &tables).map_err(X64CompileError::Failed)?;
    let needs_float_fmt = opts.target == X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "goose"
                && matches!(
                    f.as_str(),
                    "print_f64" | "print_f32" | "format_f64" | "format_f32"
                )
        });
    if needs_float_fmt {
        helper_asms.push((
            HELPER_GOOSE_FMT_FLOAT,
            emit_a64_helper_goose_fmt_float().map_err(X64CompileError::Failed)?,
        ));
    }
    let needs_standalone_fs = opts.target == X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "env"
                && matches!(
                    f.as_str(),
                    "gs_os_file_exists"
                        | "gs_os_is_dir"
                        | "gs_os_read_file"
                        | "gs_os_write_file"
                        | "gs_os_append_file"
                )
        });
    if needs_standalone_fs {
        helper_asms.push((
            HELPER_STANDALONE_FS,
            emit_a64_helper_standalone_fs().map_err(X64CompileError::Failed)?,
        ));
    }
    let needs_standalone_reserve = opts.target == X64Target::StandaloneExec
        && module.imports.iter().any(|(m, f)| {
            m == "env" && matches!(f.as_str(), "gs_os_arg" | "gs_os_getenv" | "gs_os_read_file")
        });
    if needs_standalone_reserve {
        helper_asms.push((
            HELPER_STANDALONE_RESERVE_COPY,
            emit_a64_helper_standalone_reserve_copy(&module).map_err(X64CompileError::Failed)?,
        ));
    }

    if opts.target == X64Target::BlotCdylib {
        helper_asms.push((
            HELPER_READ_GLOBAL,
            emit_a64_helper_read_global(&tables).map_err(X64CompileError::Failed)?,
        ));
        if !module.imports.is_empty() {
            helper_asms.push((
                HELPER_HOST_CALL,
                emit_a64_helper_host_call(&module, &layout).map_err(X64CompileError::Failed)?,
            ));
        }
    }

    let mut import_asms: Vec<A64Asm> = Vec::with_capacity(n_imports);
    match opts.target {
        X64Target::StandaloneExec => {
            for k in 0..n_imports {
                import_asms.push(
                    emit_a64_standalone_import_trampoline(&module, k)
                        .map_err(X64CompileError::Failed)?,
                );
            }
        }
        X64Target::BlotCdylib => {
            for k in 0..n_imports {
                import_asms.push(
                    emit_a64_blot_import_trampoline(&module, k)
                        .map_err(X64CompileError::AdmissionRejected)?,
                );
            }
        }
    }

    let (export_asms, standalone_asm) = match opts.target {
        X64Target::BlotCdylib => (
            emit_a64_exports(&module, &layout, &tables).map_err(X64CompileError::Failed)?,
            None,
        ),
        X64Target::StandaloneExec => (
            Vec::new(),
            Some(
                emit_a64_standalone_start(&module, &layout, &tables)
                    .map_err(X64CompileError::Failed)?,
            ),
        ),
    };

    // 4. Append all per-function / import / helper / export local `.rodata` slices into `.rodata`.
    let mut export_lro: Vec<u32> = Vec::with_capacity(export_asms.len());
    for (_, a) in &export_asms {
        let off = if a.local_rodata.is_empty() {
            0
        } else {
            append_rodata(&mut rodata, 16, &a.local_rodata).map_err(X64CompileError::Failed)?
        };
        export_lro.push(off);
    }
    let standalone_lro = if let Some(a) = &standalone_asm {
        if a.local_rodata.is_empty() {
            0
        } else {
            append_rodata(&mut rodata, 16, &a.local_rodata).map_err(X64CompileError::Failed)?
        }
    } else {
        0
    };
    let mut helper_lro: Vec<u32> = Vec::with_capacity(helper_asms.len());
    for (_, a) in &helper_asms {
        let off = if a.local_rodata.is_empty() {
            0
        } else {
            append_rodata(&mut rodata, 16, &a.local_rodata).map_err(X64CompileError::Failed)?
        };
        helper_lro.push(off);
    }
    let mut import_lro: Vec<u32> = Vec::with_capacity(import_asms.len());
    for a in &import_asms {
        let off = if a.local_rodata.is_empty() {
            0
        } else {
            append_rodata(&mut rodata, 16, &a.local_rodata).map_err(X64CompileError::Failed)?
        };
        import_lro.push(off);
    }
    let mut fn_lro: Vec<u32> = Vec::with_capacity(n_defs);
    for a in &fn_asms {
        let off = if a.local_rodata.is_empty() {
            0
        } else {
            append_rodata(&mut rodata, 16, &a.local_rodata).map_err(X64CompileError::Failed)?
        };
        fn_lro.push(off);
    }

    // 5. Plan ELF layout (`rodata_vaddr`, `text_vaddr`).
    enum PlannedElf {
        So(A64ElfSoLayout),
        Exec(A64ElfExecLayout),
    }
    let planned = match opts.target {
        X64Target::BlotCdylib => PlannedElf::So(
            A64ElfSoLayout::plan(&EXPORT_NAMES, rodata.len()).map_err(X64CompileError::Failed)?,
        ),
        X64Target::StandaloneExec => {
            PlannedElf::Exec(A64ElfExecLayout::plan(rodata.len()).map_err(X64CompileError::Failed)?)
        }
    };
    let (rodata_vaddr, text_vaddr) = match &planned {
        PlannedElf::So(p) => (p.rodata_vaddr as u64, p.text_vaddr as u64),
        PlannedElf::Exec(p) => (p.rodata_vaddr as u64, p.text_vaddr as u64),
    };

    // 6. Place all `.text` sections (16-byte aligned, NOP padded).
    let mut text = Vec::new();
    let pad_text = |text: &mut Vec<u8>| {
        let next = align_up(text.len(), 16);
        while text.len() < next {
            text.extend_from_slice(&0xd503_201fu32.to_le_bytes()); // AArch64 NOP
        }
    };

    let mut export_defs: Vec<(String, u64, u64)> = Vec::with_capacity(export_asms.len());
    let mut export_offs: Vec<usize> = Vec::with_capacity(export_asms.len());
    for (name, a) in &export_asms {
        pad_text(&mut text);
        let off = text.len();
        export_offs.push(off);
        export_defs.push(((*name).to_string(), off as u64, a.code.len() as u64));
        text.extend_from_slice(&a.code);
    }

    let standalone_off = if let Some(a) = &standalone_asm {
        pad_text(&mut text);
        let off = text.len();
        text.extend_from_slice(&a.code);
        Some(off)
    } else {
        None
    };

    let mut helper_offs: Vec<Option<usize>> = vec![None; NUM_HELPERS];
    for (hid, a) in &helper_asms {
        pad_text(&mut text);
        let off = text.len();
        helper_offs[*hid as usize] = Some(off);
        text.extend_from_slice(&a.code);
    }

    // `func_offs[0..n_total]`: import trampolines (`0..n_imports`), then defined functions (`n_imports..`).
    let mut func_offs: Vec<usize> = Vec::with_capacity(n_total);
    for a in &import_asms {
        pad_text(&mut text);
        let off = text.len();
        func_offs.push(off);
        text.extend_from_slice(&a.code);
    }
    for a in &fn_asms {
        pad_text(&mut text);
        let off = text.len();
        func_offs.push(off);
        text.extend_from_slice(&a.code);
    }
    if text.len() > MAX_TOTAL_CODE_BYTES {
        return Err(X64CompileError::AdmissionRejected(
            "aarch64: .text exceeds cap".into(),
        ));
    }

    // 7. Patch jump tables and call_indirect dispatch tables in `.rodata`.
    for (di, a) in fn_asms.iter_mut().enumerate() {
        if !a.jump_tables.is_empty() {
            let fn_va = text_vaddr + func_offs[n_imports + di] as u64;
            let lro_va = rodata_vaddr + u64::from(fn_lro[di]);
            a.patch_jump_tables(fn_va, lro_va)
                .map_err(X64CompileError::Failed)?;
            let start = fn_lro[di] as usize;
            rodata[start..start + a.local_rodata.len()].copy_from_slice(&a.local_rodata);
        }
    }
    for (roff, slots) in &ci_patch_info {
        if slots.is_empty() {
            continue;
        }
        let base_va = rodata_vaddr + u64::from(*roff);
        for (i, opt_f) in slots.iter().enumerate() {
            if let Some(fidx) = *opt_f {
                let target_va = text_vaddr + func_offs[fidx as usize] as u64;
                let rel = i32::try_from((target_va as i64) - (base_va as i64)).map_err(|_| {
                    X64CompileError::Failed("aarch64: ci_table offset overflow".into())
                })?;
                let pos = (*roff as usize) + i * 4;
                rodata[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
            }
        }
    }
    if module.uses_dyn_ci || module.uses_call_ref {
        let base_va = rodata_vaddr + u64::from(layout.func_addr_ro_off);
        for (fi, &target_off) in func_offs.iter().enumerate() {
            let target_va = text_vaddr + target_off as u64;
            let rel = i32::try_from((target_va as i64) - (base_va as i64)).map_err(|_| {
                X64CompileError::Failed("aarch64: func_addr offset overflow".into())
            })?;
            let pos = (layout.func_addr_ro_off as usize) + fi * 4;
            rodata[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
        }
    }

    // 8. Patch all link-time AArch64 relocations (`bl`, `b`, `adrp+add`).
    let resolve_reloc = |target: A64RelocTarget, local_ro_off: u32| -> Result<u64, String> {
        match target {
            A64RelocTarget::Func(f) => {
                let off = *func_offs
                    .get(f as usize)
                    .ok_or_else(|| format!("aarch64: bad Func({f}) reloc"))?;
                Ok(text_vaddr + off as u64)
            }
            A64RelocTarget::Helper(h) => {
                let off = helper_offs
                    .get(h as usize)
                    .copied()
                    .flatten()
                    .ok_or_else(|| format!("aarch64: bad Helper({h}) reloc"))?;
                Ok(text_vaddr + off as u64)
            }
            A64RelocTarget::Rodata(off) => Ok(rodata_vaddr + u64::from(off)),
            A64RelocTarget::LocalRodata(off) => {
                Ok(rodata_vaddr + u64::from(local_ro_off) + u64::from(off))
            }
        }
    };

    for (i, (_, a)) in export_asms.iter().enumerate() {
        let lro = export_lro[i];
        patch_a64_relocs(&mut text, text_vaddr, export_offs[i], &a.relocs, |t| {
            resolve_reloc(t, lro)
        })
        .map_err(X64CompileError::Failed)?;
    }
    if let (Some(a), Some(off)) = (&standalone_asm, standalone_off) {
        patch_a64_relocs(&mut text, text_vaddr, off, &a.relocs, |t| {
            resolve_reloc(t, standalone_lro)
        })
        .map_err(X64CompileError::Failed)?;
    }
    for (i, (hid, a)) in helper_asms.iter().enumerate() {
        let lro = helper_lro[i];
        let off = helper_offs[*hid as usize].unwrap();
        patch_a64_relocs(&mut text, text_vaddr, off, &a.relocs, |t| {
            resolve_reloc(t, lro)
        })
        .map_err(X64CompileError::Failed)?;
    }
    for (k, a) in import_asms.iter().enumerate() {
        let lro = import_lro[k];
        patch_a64_relocs(&mut text, text_vaddr, func_offs[k], &a.relocs, |t| {
            resolve_reloc(t, lro)
        })
        .map_err(X64CompileError::Failed)?;
    }
    for (di, a) in fn_asms.iter().enumerate() {
        let lro = fn_lro[di];
        patch_a64_relocs(
            &mut text,
            text_vaddr,
            func_offs[n_imports + di],
            &a.relocs,
            |t| resolve_reloc(t, lro),
        )
        .map_err(X64CompileError::Failed)?;
    }

    // 9. Emit final ELF64 image and persist incremental cache (`a64-inc` + `.vidx`).
    let elf = match planned {
        PlannedElf::So(p) => {
            let defs: Vec<ExportDef<'_>> = export_defs
                .iter()
                .map(|(n, o, s)| ExportDef {
                    name: n.as_str(),
                    text_off: *o,
                    size: *s,
                })
                .collect();
            p.emit(&rodata, &text, &defs)
                .map_err(X64CompileError::Failed)?
        }
        PlannedElf::Exec(p) => p
            .emit(&rodata, &text, standalone_off.unwrap() as u64)
            .map_err(X64CompileError::Failed)?,
    };

    if let Some((dir, cache_data, vidx_opt)) = pending_cache_save {
        save_a64_inc_cache(&dir, &cache_data);
        if let Some(vidx) = vidx_opt {
            save_val_index(&dir, &vidx);
        }
    }

    let _ = (
        x64_inc::try_vec_with_capacity::<u8>,
        x64_mod::MAX_FRAME_BYTES,
        inc_reused_funcs,
    );

    Ok(A64BuildOutput {
        elf,
        #[cfg(test)]
        rodata_vaddr,
        #[cfg(test)]
        text_vaddr,
        #[cfg(test)]
        rodata,
        #[cfg(test)]
        text,
        #[cfg(test)]
        export_vaddrs: export_defs
            .iter()
            .map(|(n, o, _)| (n.clone(), text_vaddr + *o))
            .collect(),
        #[cfg(test)]
        entry_vaddr: standalone_off.map(|o| text_vaddr + o as u64),
        #[cfg(test)]
        inc_reused_funcs,
    })
}
