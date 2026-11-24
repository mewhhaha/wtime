//! Self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm -> Linux AArch64 (`EM_AARCH64`)
//! AOT compiler (`wtime-aot --backend aarch64`).
//!
//! Shares the target-independent decoding, admission, SSA IR lowering, Stage 2 optimizer
//! (inlining, LICM, GVN/BCE, loop versioning, loop rotation, boundary-clamp peeling,
//! byte-sum vectorization, and bit-exact `f64x2` SLP), and SSA verifier with `--backend x64`,
//! then performs AArch64 register allocation, ARMv8-A/NEON lowering, and ELF64 `ET_DYN` / `ET_EXEC`
//! emission with zero external toolchain dependencies.

pub mod asm;
pub mod elf;
pub mod lower;
pub mod regalloc;
pub mod runtime;
#[cfg(test)]
mod tests_aarch64;

use std::time::Instant;

use asm::{A64Asm, A64RelocTarget, patch_a64_relocs};
use elf::{A64ElfExecLayout, A64ElfSoLayout};
use lower::lower_ir_to_aarch64;
use regalloc::allocate;
use runtime::{emit_a64_exports, emit_a64_helpers, emit_a64_standalone_start};

use crate::x64::elf::ExportDef;
use crate::x64::ir::lower_wasm_to_ir_with_budget;
use crate::x64::module::{
    CompileBudget, MAX_TOTAL_CODE_BYTES, MAX_TOTAL_RODATA_BYTES, X64Target,
    admit_x64_module_for_target, x64_validate_features, x64_validate_limits,
};
use crate::x64::opt::{
    Stage2Scratch, compute_module_effects, inline_small_helpers, optimize_func_stage2_with_scratch,
};
use crate::x64::runtime::{EXPORT_NAMES, RuntimeTables, TRAP_MESSAGES};
use crate::x64::verify::verify_ir;
use crate::x64::{CiTableLayout, CtxLayout, MAX_WASM_BYTES_X64, NUM_HELPERS, X64CompileError};

/// Compilation options for `--backend aarch64`.
#[derive(Clone, Debug)]
pub struct A64CompileOptions {
    pub target: X64Target,
    pub optimize: bool,
    pub deadline: Option<Instant>,
}

impl Default for A64CompileOptions {
    fn default() -> Self {
        Self {
            target: X64Target::BlotCdylib,
            optimize: true,
            deadline: None,
        }
    }
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
    })
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

    let validated =
        wtime_validate::validate(wasm, &x64_validate_features(), &x64_validate_limits()).map_err(
            |e| X64CompileError::AdmissionRejected(format!("aarch64: validation failed: {e}")),
        )?;
    let decoded = validated.decoded();
    let module = admit_x64_module_for_target(decoded, opts.target, Some(&budget))
        .map_err(X64CompileError::AdmissionRejected)?;
    if !module.imports.is_empty() {
        return Err(X64CompileError::AdmissionRejected(
            "aarch64: host/WASI imports are not yet supported by --backend aarch64".into(),
        ));
    }

    // 1. Build shared `.rodata` tables (trap messages, data segments, call_indirect slots).
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

    let mut data_segs = Vec::with_capacity(module.active_data_segs.len());
    for &(mem_off, data_idx) in &module.active_data_segs {
        let span = decoded.data[data_idx].bytes;
        let bytes = &wasm[span.start..span.end];
        let roff = append_rodata(&mut rodata, 1, bytes).map_err(X64CompileError::Failed)?;
        data_segs.push((mem_off, roff, bytes.len() as u32));
    }

    let mut ci_tables = Vec::with_capacity(module.ci_sigs.len());
    let mut ci_patch_info: Vec<(u32, Vec<Option<u32>>)> = Vec::with_capacity(module.ci_sigs.len());
    for sig in &module.ci_sigs {
        let (roff, slice) = if sig.span == 0 {
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
        });
        ci_patch_info.push((roff, slice));
    }

    let tables = RuntimeTables {
        trap_table_off,
        imp_table_off: 0,
        imp_count: 0,
        call_export_table_off: 0,
        call_export_count: 0,
        get_global_table_off: 0,
        get_global_count: 0,
        set_global_table_off: 0,
        set_global_count: 0,
        data_segs,
        v128_global_inits: Vec::new(),
    };
    let layout =
        CtxLayout::compute(&module, ci_tables).map_err(X64CompileError::AdmissionRejected)?;

    // 2. Lower all defined functions -> shared SSA IR -> Stage 2 opt -> verify_ir -> AArch64 regalloc & lowering.
    let n_defs = decoded.code.len();
    let mut irs = Vec::with_capacity(n_defs);
    for di in 0..n_defs {
        budget.check_deadline().map_err(X64CompileError::Failed)?;
        let ir = lower_wasm_to_ir_with_budget(
            decoded,
            &module,
            module.imports.len() + di,
            opts.optimize,
            Some(&budget),
        )
        .map_err(X64CompileError::AdmissionRejected)?;
        irs.push(ir);
    }
    if opts.optimize {
        inline_small_helpers(&mut irs, &module, Some(&budget)).map_err(X64CompileError::Failed)?;
        let effects = compute_module_effects(&irs, &module, Some(&budget))
            .map_err(X64CompileError::Failed)?;
        let mut scratch = Stage2Scratch::default();
        for ir in &mut irs {
            optimize_func_stage2_with_scratch(ir, &module, &effects, Some(&budget), &mut scratch)
                .map_err(X64CompileError::Failed)?;
        }
    }

    let mut fn_asms: Vec<A64Asm> = Vec::with_capacity(n_defs);
    for ir in &irs {
        verify_ir(ir, &module).map_err(X64CompileError::Failed)?;
        let alloc = allocate(ir, Some(&budget)).map_err(X64CompileError::AdmissionRejected)?;
        let fa =
            lower_ir_to_aarch64(ir, &alloc, &layout).map_err(X64CompileError::AdmissionRejected)?;
        fn_asms.push(fa);
    }

    // 3. Emit helpers and either Blot ABI 8 exports (`ET_DYN`) or standalone `_start` (`ET_EXEC`).
    let helper_asms =
        emit_a64_helpers(&module, &layout, &tables).map_err(X64CompileError::Failed)?;
    let (export_asms, standalone_asm) = match opts.target {
        X64Target::BlotCdylib => (
            emit_a64_exports(&module, &layout, &tables).map_err(X64CompileError::Failed)?,
            None,
        ),
        X64Target::StandaloneExec => (
            Vec::new(),
            Some(emit_a64_standalone_start(&module, &layout).map_err(X64CompileError::Failed)?),
        ),
    };

    // 4. Append all per-function / helper / export local `.rodata` slices into `.rodata`.
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

    let mut func_offs: Vec<usize> = Vec::with_capacity(n_defs);
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
            let fn_va = text_vaddr + func_offs[di] as u64;
            let lro_va = rodata_vaddr + u64::from(fn_lro[di]);
            a.patch_jump_tables(fn_va, lro_va)
                .map_err(X64CompileError::Failed)?;
            let start = fn_lro[di] as usize;
            rodata[start..start + a.local_rodata.len()].copy_from_slice(&a.local_rodata);
        }
    }
    for (roff, slots) in &ci_patch_info {
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
    for (di, a) in fn_asms.iter().enumerate() {
        let lro = fn_lro[di];
        patch_a64_relocs(&mut text, text_vaddr, func_offs[di], &a.relocs, |t| {
            resolve_reloc(t, lro)
        })
        .map_err(X64CompileError::Failed)?;
    }

    // 9. Emit final ELF64 image.
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
    })
}
