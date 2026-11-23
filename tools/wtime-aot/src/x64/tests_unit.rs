//! Stage 1 unit tests: byte-level `asm.rs` encodings, `elf.rs` hash collisions,
//! `CtxLayout` 16 MiB cap, `CompileBudget` liveness contention, `verify_ir`/`verify_alloc`
//! soundness, `emit_parallel_moves`, ISA resolution, and Stage 1 admission acceptance/rejection.

use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use wtime_types::{FuncType, ValType};

use super::asm::{Asm, Cond, Gp, Mem, Xmm};
use super::elf::{ElfLayout, ExportDef};
use super::ir::{
    Block, BlockId, Inst, IntBinOp, IrFunc, Op, Term, Ty, Val, ValRange, lower_wasm_to_ir,
    lower_wasm_to_ir_unopt,
};
use super::lower::{MoveLoc, TargetIsa, emit_parallel_moves};
use super::module::{
    CompileBudget, MAX_CTX_BYTES, MAX_LIVENESS_BYTES, X64ConstVal, X64Module, admit_x64_module,
    x64_validate_features, x64_validate_limits,
};
use super::regalloc::{Loc, allocate};
use super::{
    CtxLayout, X64CompileError, X64CompileOptions, compile_wasm_to_x64_so,
    parse_x64_target_cpu_syntax, resolve_x64_isa, verify_alloc, verify_ir,
};

// ---- Minimal Wasm binary encoder helpers (shared by tests_oracle & tests_abi_faults) ----

pub fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

pub fn leb_i32(out: &mut Vec<u8>, mut v: i32) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0);
        out.push(if done { b } else { b | 0x80 });
        if done {
            break;
        }
    }
}

pub fn leb_i64(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0);
        out.push(if done { b } else { b | 0x80 });
        if done {
            break;
        }
    }
}

pub fn section(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    leb_u32(&mut out, payload.len() as u32);
    out.extend_from_slice(payload);
    out
}

pub fn enc_str(out: &mut Vec<u8>, s: &str) {
    leb_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

pub fn enc_body(locals: &[(u32, u8)], code: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    leb_u32(&mut b, locals.len() as u32);
    for &(cnt, ty) in locals {
        leb_u32(&mut b, cnt);
        b.push(ty);
    }
    b.extend_from_slice(code);
    b.push(0x0b);
    let mut out = Vec::new();
    leb_u32(&mut out, b.len() as u32);
    out.extend_from_slice(&b);
    out
}

pub fn wasm_module(sections: &[Vec<u8>]) -> Vec<u8> {
    let mut w = b"\0asm\x01\0\0\0".to_vec();
    for s in sections {
        w.extend_from_slice(s);
    }
    w
}

pub fn type_sec(sigs: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut p = Vec::new();
    leb_u32(&mut p, sigs.len() as u32);
    for &(params, results) in sigs {
        p.push(0x60);
        leb_u32(&mut p, params.len() as u32);
        p.extend_from_slice(params);
        leb_u32(&mut p, results.len() as u32);
        p.extend_from_slice(results);
    }
    section(1, &p)
}

pub fn func_sec(type_idxs: &[u32]) -> Vec<u8> {
    let mut p = Vec::new();
    leb_u32(&mut p, type_idxs.len() as u32);
    for &t in type_idxs {
        leb_u32(&mut p, t);
    }
    section(3, &p)
}

pub fn export_sec(exports: &[(&str, u8, u32)]) -> Vec<u8> {
    let mut p = Vec::new();
    leb_u32(&mut p, exports.len() as u32);
    for &(name, kind, idx) in exports {
        enc_str(&mut p, name);
        p.push(kind);
        leb_u32(&mut p, idx);
    }
    section(7, &p)
}

pub fn code_sec(bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut p = Vec::new();
    leb_u32(&mut p, bodies.len() as u32);
    for b in bodies {
        p.extend_from_slice(b);
    }
    section(10, &p)
}

// ---- 1. Byte-level `asm.rs` unit tests (§4.3.1) ----

#[test]
#[should_panic(expected = "RSP")]
fn asm_rejects_rsp_as_sib_index() {
    let _ = Mem::indexed(Gp::R14, Gp::Rsp, 1, 0);
}

#[test]
#[should_panic(expected = "power of two")]
fn asm_rejects_zero_align_local_rodata() {
    let mut a = Asm::new();
    let _ = a.add_local_rodata(0, &[1, 2, 3, 4]);
}

#[test]
#[should_panic(expected = "power of two")]
fn asm_rejects_non_pow2_align_local_rodata() {
    let mut a = Asm::new();
    let _ = a.add_local_rodata(3, &[1, 2, 3]);
}

#[test]
fn asm_byte_level_encodings() {
    // R12 as valid SIB index: `mov rax, [r14 + r12*4]` -> 4b 8b 04 a6
    let mut a = Asm::new();
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R14, Gp::R12, 4, 0));
    assert_eq!(a.code, [0x4b, 0x8b, 0x04, 0xa6]);

    // Rbp / R13 zero-disp MUST emit mod=0x40 + disp8=0:
    // `mov rax, [rbp]` -> 48 8b 45 00; `mov rax, [r13]` -> 49 8b 45 00
    let mut a = Asm::new();
    a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rbp));
    assert_eq!(a.code, [0x48, 0x8b, 0x45, 0x00]);
    let mut a = Asm::new();
    a.mov_rm(true, Gp::Rax, Mem::base(Gp::R13));
    assert_eq!(a.code, [0x49, 0x8b, 0x45, 0x00]);

    // Forced REX (0x40) on Spl/Bpl/Sil/Dil for 8-bit register ops:
    let mut a = Asm::new();
    a.movzx_rr8(Gp::Rax, Gp::Rdi); // movzx eax, dil -> 40 0f b6 c7
    assert_eq!(a.code, [0x40, 0x0f, 0xb6, 0xc7]);
    let mut a = Asm::new();
    a.movzx_rr8(Gp::Rax, Gp::Rsi); // movzx eax, sil -> 40 0f b6 c6
    assert_eq!(a.code, [0x40, 0x0f, 0xb6, 0xc6]);
    let mut a = Asm::new();
    a.mov_mr8(Mem::base(Gp::Rax), Gp::Rbp); // mov [rax], bpl -> 40 88 28
    assert_eq!(a.code, [0x40, 0x88, 0x28]);
    let mut a = Asm::new();
    a.setcc(Cond::E, Gp::Rdi); // sete dil -> 40 0f 94 c7
    assert_eq!(a.code, [0x40, 0x0f, 0x94, 0xc7]);

    // SSE4.1 pextrq / pinsrq / roundss / roundsd:
    let mut a = Asm::new();
    a.pextrq(Gp::Rax, Xmm::Xmm0, 1); // 66 48 0f 3a 16 c0 01
    assert_eq!(a.code, [0x66, 0x48, 0x0f, 0x3a, 0x16, 0xc0, 0x01]);
    let mut a = Asm::new();
    a.pinsrq(Xmm::Xmm1, Gp::R8, 1); // 66 49 0f 3a 22 c8 01
    assert_eq!(a.code, [0x66, 0x49, 0x0f, 0x3a, 0x22, 0xc8, 0x01]);
    let mut a = Asm::new();
    a.roundss(Xmm::Xmm0, Xmm::Xmm1, 0x09); // 66 0f 3a 0a c1 09
    assert_eq!(a.code, [0x66, 0x0f, 0x3a, 0x0a, 0xc1, 0x09]);
    let mut a = Asm::new();
    a.roundsd(Xmm::Xmm8, Xmm::Xmm9, 0x0b); // 66 45 0f 3a 0b c1 0b
    assert_eq!(a.code, [0x66, 0x45, 0x0f, 0x3a, 0x0b, 0xc1, 0x0b]);

    // Portable bsr / bsf:
    let mut a = Asm::new();
    a.bsr(false, Gp::Rax, Gp::Rcx); // 0f bd c1
    assert_eq!(a.code, [0x0f, 0xbd, 0xc1]);
    let mut a = Asm::new();
    a.bsf(true, Gp::R8, Gp::R9); // 4d 0f bc c1
    assert_eq!(a.code, [0x4d, 0x0f, 0xbc, 0xc1]);

    // CPUID, RDSSPQ (`F3 48 0F 1E C8` / `F3 49 0F 1E C8`), STMXCSR / LDMXCSR:
    let mut a = Asm::new();
    a.cpuid();
    assert_eq!(a.code, [0x0f, 0xa2]);
    let mut a = Asm::new();
    a.rdsspq(Gp::Rax);
    assert_eq!(a.code, [0xf3, 0x48, 0x0f, 0x1e, 0xc8]);
    let mut a = Asm::new();
    a.rdsspq(Gp::R8);
    assert_eq!(a.code, [0xf3, 0x49, 0x0f, 0x1e, 0xc8]);
    let mut a = Asm::new();
    a.stmxcsr(Mem::base(Gp::Rsp)); // 0f ae 1c 24
    assert_eq!(a.code, [0x0f, 0xae, 0x1c, 0x24]);
    let mut a = Asm::new();
    a.ldmxcsr(Mem::base(Gp::Rsp)); // 0f ae 14 24
    assert_eq!(a.code, [0x0f, 0xae, 0x14, 0x24]);
}

// ---- 2. `elf.rs` SysV hash collision & validation unit tests (§4.3.2) ----

#[test]
fn elf_handles_sysv_hash_collision_and_validates_invariants() {
    // Construct two distinct names that collide modulo nbucket == 2 (or exact same elf_hash).
    // Even with nbucket = 2, two names with identical (hash % 2) share a chain in `.hash`!
    let names = ["sym_a", "sym_c"]; // 'a' = 97 (odd), 'c' = 99 (odd) -> both hash % 2 == 1!
    let layout = ElfLayout::plan(&names, 16).unwrap();
    let rodata = vec![0u8; 16];
    let text = vec![0xc3u8; 32];
    let defs = [
        ExportDef { name: "sym_a", text_off: 0, size: 16 },
        ExportDef { name: "sym_c", text_off: 16, size: 16 },
    ];
    let img = layout.emit(&rodata, &text, &defs).unwrap();
    assert_eq!(&img[0..4], b"\x7fELF");
    // Locate DT_HASH table and verify chain links symbol 2 -> symbol 1 -> 0.
    // Find .hash section header (index 2).
    let shoff = u64::from_le_bytes(img[40..48].try_into().unwrap()) as usize;
    let hash_off = u64::from_le_bytes(img[shoff + 2 * 64 + 24..shoff + 2 * 64 + 32].try_into().unwrap()) as usize;
    let nbucket = u32::from_le_bytes(img[hash_off..hash_off + 4].try_into().unwrap());
    let nchain = u32::from_le_bytes(img[hash_off + 4..hash_off + 8].try_into().unwrap());
    assert_eq!(nbucket, 2);
    assert_eq!(nchain, 3);
    let b0 = u32::from_le_bytes(img[hash_off + 8..hash_off + 12].try_into().unwrap());
    let b1 = u32::from_le_bytes(img[hash_off + 12..hash_off + 16].try_into().unwrap());
    let c0 = u32::from_le_bytes(img[hash_off + 16..hash_off + 20].try_into().unwrap());
    let c1 = u32::from_le_bytes(img[hash_off + 20..hash_off + 24].try_into().unwrap());
    let c2 = u32::from_le_bytes(img[hash_off + 24..hash_off + 28].try_into().unwrap());
    // Both symbols landed in bucket 1: head=2, chain[2]=1, chain[1]=0.
    assert_eq!(b0, 0);
    assert_eq!(b1, 2);
    assert_eq!((c0, c1, c2), (0, 0, 1));

    // Duplicate & empty names rejected.
    assert!(ElfLayout::plan(&["dup", "dup"], 0).is_err());
    assert!(ElfLayout::plan(&[""], 0).is_err());
    // Mismatched rodata length after plan rejected.
    let layout2 = ElfLayout::plan(&["f"], 16).unwrap();
    assert!(layout2.emit(&[0u8; 32], &[0xc3], &[ExportDef { name: "f", text_off: 0, size: 1 }]).is_err());
}

// ---- 3. `CtxLayout` 16 MiB cap unit test (§4.3.2) ----

#[test]
fn ctx_layout_enforces_16_mib_cap() {
    let make_mod = |n_v128_globals: usize, extern_len: u32| X64Module {
        imports: Vec::new(),
        func_type_idx: Vec::new(),
        module_types: Vec::new(),
        globals: vec![(ValType::V128, true); n_v128_globals],
        global_inits: vec![X64ConstVal::V128([0; 16]); n_v128_globals],
        global_consts: vec![None; n_v128_globals],
        pinned_global: None,
        exported_globals: Vec::new(),
        exported_funcs: Vec::new(),
        ci_index: Default::default(),
        ci_sigs: Vec::new(),
        mut_extern_tables: if extern_len > 0 { [(0, extern_len)].into_iter().collect() } else { Default::default() },
        memory: None,
        fixed32: None,
        active_elem_checks: Vec::new(),
        active_data_segs: Vec::new(),
        start: None,
    };

    // Exact boundary: 128 B header + (16 MiB - 128 B) of 16-byte V128 globals = exactly 16 MiB.
    let max_v128 = (MAX_CTX_BYTES - 128) / 16;
    let ok = CtxLayout::compute(&make_mod(max_v128, 0), Vec::new()).unwrap();
    assert_eq!(ok.total_bytes, MAX_CTX_BYTES);

    // One more global overflows the 16 MiB cap cleanly!
    let err = CtxLayout::compute(&make_mod(max_v128 + 1, 0), Vec::new());
    assert!(err.is_err(), "expected CtxLayout > 16 MiB to fail cleanly");

    // Also test overflow via externref table slots.
    let err_tbl = CtxLayout::compute(&make_mod(max_v128, 1), Vec::new());
    assert!(err_tbl.is_err(), "expected CtxLayout extern table overflow > 16 MiB to fail cleanly");
}

// ---- 4. `CompileBudget` liveness scheduler contention tests (§4.3.3) ----

#[test]
fn liveness_scheduler_unblocks_waiting_worker_and_times_out_cleanly() {
    // (a) Waiting worker unblocks once competing guards drop.
    let budget = Arc::new(CompileBudget::new(Some(Instant::now() + Duration::from_secs(5))));
    let n_fill = super::module::MAX_CONCURRENT_LIVENESS_BYTES / MAX_LIVENESS_BYTES;
    let mut held = Vec::with_capacity(n_fill);
    for _ in 0..n_fill {
        held.push(budget.reserve_liveness(MAX_LIVENESS_BYTES).unwrap());
    }
    let barrier = Arc::new(Barrier::new(2));
    let b2 = Arc::clone(&budget);
    let bar2 = Arc::clone(&barrier);
    let handle = std::thread::spawn(move || {
        bar2.wait();
        let guard = b2.reserve_liveness(MAX_LIVENESS_BYTES)?;
        drop(guard);
        Ok::<(), String>(())
    });
    barrier.wait();
    std::thread::sleep(Duration::from_millis(25));
    // Release one slot -> waiting worker MUST wake up and succeed.
    held.pop();
    handle.join().unwrap().expect("waiting worker should succeed after capacity released");
    drop(held);

    // (b) Waiting worker times out cleanly when deadline expires while capacity is held.
    let tight = Arc::new(CompileBudget::new(Some(Instant::now() + Duration::from_millis(60))));
    let mut held2 = Vec::with_capacity(n_fill);
    for _ in 0..n_fill {
        held2.push(tight.reserve_liveness(MAX_LIVENESS_BYTES).unwrap());
    }
    let err = tight.reserve_liveness(MAX_LIVENESS_BYTES);
    assert!(err.is_err(), "expected timeout while waiting for liveness capacity");
    assert!(err.err().unwrap().contains("time limit"));
    drop(held2);
}

// ---- 5. `emit_parallel_moves` invariants (§4.3.4) ----

#[test]
#[should_panic(expected = "distinct destinations")]
fn parallel_moves_rejects_duplicate_dst_even_with_self_move() {
    let mut a = Asm::new();
    emit_parallel_moves(
        &mut a,
        &[
            (MoveLoc::Gp(Gp::Rax), MoveLoc::Gp(Gp::Rax), Ty::I64),
            (MoveLoc::Gp(Gp::Rax), MoveLoc::Gp(Gp::Rcx), Ty::I64),
        ],
    );
}

#[test]
fn parallel_moves_emits_cycles_and_stack_swaps_cleanly() {
    let mut a = Asm::new();
    emit_parallel_moves(
        &mut a,
        &[
            (MoveLoc::Gp(Gp::Rax), MoveLoc::Gp(Gp::Rcx), Ty::I64),
            (MoveLoc::Gp(Gp::Rcx), MoveLoc::Gp(Gp::Rdx), Ty::I64),
            (MoveLoc::Gp(Gp::Rdx), MoveLoc::Gp(Gp::Rax), Ty::I64),
            (MoveLoc::Xmm(Xmm::Xmm0), MoveLoc::Xmm(Xmm::Xmm1), Ty::F64),
            (MoveLoc::Xmm(Xmm::Xmm1), MoveLoc::Xmm(Xmm::Xmm2), Ty::F64),
            (MoveLoc::Xmm(Xmm::Xmm2), MoveLoc::Xmm(Xmm::Xmm0), Ty::F64),
            (MoveLoc::Stack(0), MoveLoc::Stack(8), Ty::I64),
            (MoveLoc::Stack(8), MoveLoc::Stack(0), Ty::I64),
            (MoveLoc::Gp(Gp::R8), MoveLoc::Gp(Gp::R9), Ty::I32),
            (MoveLoc::Gp(Gp::Rsi), MoveLoc::Gp(Gp::R9), Ty::I32),
        ],
    );
    assert!(!a.code.is_empty());
}

// ---- 6. `verify_ir` and `verify_alloc` negative & real-module tests (§4.3.1) ----

#[test]
fn verifiers_catch_invalid_ir_and_alloc() {
    let empty_mod = X64Module {
        imports: Vec::new(),
        func_type_idx: vec![0],
        module_types: vec![FuncType { params: vec![ValType::I32], results: vec![ValType::I32] }],
        globals: Vec::new(),
        global_inits: Vec::new(),
        global_consts: Vec::new(),
        pinned_global: None,
        exported_globals: Vec::new(),
        exported_funcs: Vec::new(),
        ci_index: Default::default(),
        ci_sigs: Vec::new(),
        mut_extern_tables: Default::default(),
        memory: None,
        fixed32: None,
        active_elem_checks: Vec::new(),
        active_data_segs: Vec::new(),
        start: None,
    };

    // Valid simple IR: bb0(v0: I32) -> v1 = v0 + v0 -> Return(v1).
    let good_ir = IrFunc {
        func_index: 0,
        params: vec![Val(0)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32, Ty::I32],
        blocks: vec![Block {
            params: vec![Val(0)],
            insts: vec![Inst { res: Some(Val(1)), op: Op::I32Bin(IntBinOp::Add, Val(0), Val(0)) }],
            term: Term::Return { vals: ValRange { start: 0, len: 1 } },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    verify_ir(&good_ir, &empty_mod).unwrap();
    let good_alloc = allocate(&good_ir).unwrap();
    verify_alloc(&good_ir, &good_alloc).unwrap();

    // Negative 1: Type mismatch (I64 op on I32 value).
    let mut bad_ty = IrFunc {
        func_index: 0,
        params: vec![Val(0)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32, Ty::I32],
        blocks: vec![Block {
            params: vec![Val(0)],
            insts: vec![Inst { res: Some(Val(1)), op: Op::I64Eqz(Val(0)) }],
            term: Term::Return { vals: ValRange { start: 0, len: 1 } },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    assert!(verify_ir(&bad_ty, &empty_mod).is_err());

    // Negative 2: Use-before-def in same block.
    bad_ty.blocks[0].insts = vec![
        Inst { res: Some(Val(1)), op: Op::I32Eqz(Val(1)) },
    ];
    assert!(verify_ir(&bad_ty, &empty_mod).is_err());

    // Negative 3: Non-dominating cross-block use (bb0 branches to bb1/bb2; bb2 uses val defined in bb1).
    let bad_dom = IrFunc {
        func_index: 0,
        params: vec![Val(0)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32, Ty::I32],
        blocks: vec![
            Block {
                params: vec![Val(0)],
                insts: vec![],
                term: Term::BranchIf {
                    cond: Val(0),
                    then_bb: BlockId(1),
                    then_args: ValRange::EMPTY,
                    else_bb: BlockId(2),
                    else_args: ValRange::EMPTY,
                },
                is_loop_header: false,
                preds: 0,
            },
            Block {
                params: vec![],
                insts: vec![Inst { res: Some(Val(1)), op: Op::ConstI32(7) }],
                term: Term::Return { vals: ValRange { start: 0, len: 1 } },
                is_loop_header: false,
                preds: 1,
            },
            Block {
                params: vec![],
                insts: vec![],
                term: Term::Return { vals: ValRange { start: 0, len: 1 } },
                is_loop_header: false,
                preds: 1,
            },
        ],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    assert!(verify_ir(&bad_dom, &empty_mod).is_err());

    // Negative 4: verify_alloc catches two simultaneously live values sharing one GP register,
    // and catches non-allocatable scratch register assignment.
    let two_live_ir = IrFunc {
        func_index: 0,
        params: vec![Val(0)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32, Ty::I32, Ty::I32],
        blocks: vec![Block {
            params: vec![Val(0)],
            insts: vec![
                Inst { res: Some(Val(1)), op: Op::ConstI32(10) },
                Inst { res: Some(Val(2)), op: Op::I32Bin(IntBinOp::Add, Val(0), Val(1)) },
            ],
            term: Term::Return { vals: ValRange { start: 0, len: 1 } },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(2)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    verify_ir(&two_live_ir, &empty_mod).unwrap();
    let mut bad_alloc = allocate(&two_live_ir).unwrap();
    verify_alloc(&two_live_ir, &bad_alloc).unwrap();
    bad_alloc.locs[0] = Loc::Gp(Gp::Rax);
    bad_alloc.locs[1] = Loc::Gp(Gp::Rax);
    assert!(verify_alloc(&two_live_ir, &bad_alloc).is_err());
    bad_alloc.locs[1] = Loc::Gp(Gp::R11); // non-allocatable scratch!
    assert!(verify_alloc(&two_live_ir, &bad_alloc).is_err());
}

// ---- 7. ISA profile syntax & resolution tests (§2.1 / §4.2) ----

#[test]
fn isa_profile_syntax_matches_pen_contract() {
    assert_eq!(parse_x64_target_cpu_syntax("native").unwrap(), None);
    assert_eq!(parse_x64_target_cpu_syntax("x86-64-v2").unwrap(), Some("x64-v2"));
    assert_eq!(parse_x64_target_cpu_syntax("generic").unwrap(), Some("x64-v2"));
    assert_eq!(parse_x64_target_cpu_syntax("x86-64-v3").unwrap(), Some("x64-v3"));
    assert_eq!(parse_x64_target_cpu_syntax("x86-64-v4").unwrap(), Some("x64-v3"));
    assert_eq!(parse_x64_target_cpu_syntax("znver4").unwrap(), Some("x64-v3"));
    assert_eq!(parse_x64_target_cpu_syntax("haswell").unwrap(), Some("x64-v3"));
    assert_eq!(parse_x64_target_cpu_syntax("graniterapids").unwrap(), Some("x64-v3"));
    assert!(parse_x64_target_cpu_syntax("znver").is_err());
    assert!(parse_x64_target_cpu_syntax("znver4a").is_err());
    assert!(parse_x64_target_cpu_syntax("pentium4").is_err());
    assert!(parse_x64_target_cpu_syntax("").is_err());
    let (tag, isa) = resolve_x64_isa("x86-64-v2").unwrap();
    assert_eq!(tag, "x64-v2");
    assert!(!isa.has_abm_bmi1);
    let (tag3, isa3) = resolve_x64_isa("znver4").unwrap();
    assert_eq!(tag3, "x64-v3");
    assert!(isa3.has_abm_bmi1);
}

// ---- 8. Admission acceptance vs rejection suite (§4.3.3) ----

fn compile_admit_only(wasm: &[u8]) -> Result<Vec<u8>, X64CompileError> {
    compile_wasm_to_x64_so(
        wasm,
        &X64CompileOptions {
            isa: TargetIsa { has_abm_bmi1: false },
            jobs: 1,
            deadline: None,
            opt: true,
            verify: true,
            hooks: Default::default(),
        },
    )
}

#[test]
fn admission_accepts_stage1_features_and_rejects_disallowed_ones() {
    // 1. Accepted: single-value blocks (-1..-5 | -17), default-null funcref & externref tables,
    //    `blot:memory` export, equivalent plain final Func types in call_indirect, imported start.
    let mut imp_p = vec![1]; // 1 import: env::init () -> ()
    enc_str(&mut imp_p, "env");
    enc_str(&mut imp_p, "init");
    imp_p.push(0x00); // func
    leb_u32(&mut imp_p, 0);
    // Table 0: funcref 2; Table 1: externref min=2 max=2
    let tbl_p = vec![2, 0x70, 0x00, 2, 0x6f, 0x01, 2, 2];
    let mem_p = vec![1, 0x01, 1, 2];
    // Elem: active table 0 offset 0 -> func 1
    let elem_p = vec![1, 0x00, 0x41, 0x00, 0x0b, 1, 1];
    // Body of func 1 (type 1: (i32)->(i32)): exercises blocks -1,-2,-3,-4,-5,-17 and call_indirect with equivalent type 2!
    let mut b1 = Vec::new();
    // block (result i32) i32.const 1 end drop
    b1.extend_from_slice(&[0x02, 0x7f, 0x41, 1, 0x0b, 0x1a]);
    // block (result i64) i64.const 1 end drop
    b1.extend_from_slice(&[0x02, 0x7e, 0x42, 1, 0x0b, 0x1a]);
    // block (result f32) f32.const 0 end drop
    b1.extend_from_slice(&[0x02, 0x7d, 0x43, 0, 0, 0, 0, 0x0b, 0x1a]);
    // block (result f64) f64.const 0 end drop
    b1.extend_from_slice(&[0x02, 0x7c, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0x0b, 0x1a]);
    // block (result v128) v128.const 0 end drop
    b1.extend_from_slice(&[0x02, 0x7b, 0xfd, 0x0c]);
    b1.extend_from_slice(&[0u8; 16]);
    b1.extend_from_slice(&[0x0b, 0x1a]);
    // block (result externref) ref.null extern end drop
    b1.extend_from_slice(&[0x02, 0x6f, 0xd0, 0x6f, 0x0b, 0x1a]);
    // call_indirect type 2 (structurally identical to type 1!)
    b1.extend_from_slice(&[0x20, 0, 0x41, 0, 0x11, 2, 0]);
    let accepted = wasm_module(&[
        type_sec(&[(&[], &[]), (&[0x7f], &[0x7f]), (&[0x7f], &[0x7f])]),
        section(2, &imp_p),
        func_sec(&[1]),
        section(4, &tbl_p),
        section(5, &mem_p),
        export_sec(&[("blot:memory", 2, 0), ("run", 0, 1)]),
        section(8, &[0]), // start = imported func 0!
        section(9, &elem_p),
        code_sec(&[enc_body(&[], &b1)]),
    ]);
    assert!(compile_admit_only(&accepted).is_ok(), "Stage 1 accepted module must compile cleanly");

    // 2. Rejected: funcref block type (-16 / 0x70) in LIVE code AND in DEAD (unreachable) code.
    let dead_funcref_block = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0x02, 0x70, 0x00, 0x0b, 0x1a])]), // unreachable; block funcref; unreachable; end; drop
    ]);
    assert!(matches!(compile_admit_only(&dead_funcref_block), Err(X64CompileError::AdmissionRejected(_))));
    let live_funcref_block = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x02, 0x70, 0xd0, 0x70, 0x0b, 0x1a])]),
    ]);
    assert!(matches!(compile_admit_only(&live_funcref_block), Err(X64CompileError::AdmissionRejected(_))));

    // 3. Rejected: ref.null func (0xd0 0x70) in LIVE and DEAD code.
    let dead_ref_null_func = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0xd0, 0x70, 0x1a])]),
    ]);
    assert!(matches!(compile_admit_only(&dead_ref_null_func), Err(X64CompileError::AdmissionRejected(_))));

    // 4. Rejected: explicit table initializer (`0x40 0x00`) rejected at BOTH front-door validate AND admit_x64_module!
    let explicit_tbl_init = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        // table with 0x40 0x00 initializer: (table 1 funcref (ref.null func))
        section(4, &[1, 0x40, 0x00, 0x70, 0x00, 1, 0xd0, 0x70, 0x0b]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    assert!(matches!(compile_admit_only(&explicit_tbl_init), Err(X64CompileError::AdmissionRejected(_))));
    // Also verify `admit_x64_module` rejects it directly even if decoded with function_references=true!
    let dec_fr = wtime_validate::validate(
        &explicit_tbl_init,
        &wtime_types::FeatureSet::stable_core(),
        &x64_validate_limits(),
    )
    .unwrap();
    assert!(admit_x64_module(dec_fr.decoded()).is_err());

    // 5. Rejected: Wasm GC non-final `(sub (func))` (`0x50 0x00 0x60 0x00 0x00`).
    let gc_sub_func = wasm_module(&[
        section(1, &[1, 0x50, 0x00, 0x60, 0x00, 0x00]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    assert!(matches!(compile_admit_only(&gc_sub_func), Err(X64CompileError::AdmissionRejected(_))));

    // 6. Rejected: exported table.
    let exp_tbl = wasm_module(&[
        section(4, &[1, 0x70, 0x00, 1]),
        export_sec(&[("t", 1, 0)]),
    ]);
    assert!(matches!(compile_admit_only(&exp_tbl), Err(X64CompileError::AdmissionRejected(_))));

    // 7. Rejected: unbounded externref table (min != max).
    let unb_ext = wasm_module(&[
        section(4, &[1, 0x6f, 0x00, 4]),
    ]);
    assert!(matches!(compile_admit_only(&unb_ext), Err(X64CompileError::AdmissionRejected(_))));

    // 8. Rejected: passive data segment.
    let passive_data = wasm_module(&[
        section(5, &[1, 0x00, 1]),
        section(12, &[1]),
        section(11, &[1, 0x01, 2, 0xaa, 0xbb]),
    ]);
    assert!(matches!(compile_admit_only(&passive_data), Err(X64CompileError::AdmissionRejected(_))));

    // 9. Rejected: unsupported SIMD opcode (e.g. i32x4.add = 0xfd 0xae 0x01) even in unreachable code!
    let dead_simd = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0xfd, 0xae, 0x01, 0x1a])]),
    ]);
    assert!(matches!(compile_admit_only(&dead_simd), Err(X64CompileError::AdmissionRejected(_))));

    // 10. Rejected: memory64 (DisabledFeature -> AdmissionRejected exit code 4!).
    let mem64 = wasm_module(&[
        section(5, &[1, 0x04, 1]),
    ]);
    assert!(matches!(compile_admit_only(&mem64), Err(X64CompileError::AdmissionRejected(_))));

    // 11. Rejected: multi-memory (DisabledFeature -> AdmissionRejected exit code 4!).
    let multi_mem = wasm_module(&[
        section(5, &[2, 0x00, 1, 0x00, 1]),
    ]);
    assert!(matches!(compile_admit_only(&multi_mem), Err(X64CompileError::AdmissionRejected(_))));

    // 12. Rejected: Blot import with unsupported signature (e.g. (i32, i32) -> (i32)).
    let mut bad_imp = vec![1];
    enc_str(&mut bad_imp, "blot:host/1");
    enc_str(&mut bad_imp, "bad");
    bad_imp.push(0x00);
    leb_u32(&mut bad_imp, 0);
    let bad_imp_mod = wasm_module(&[
        type_sec(&[(&[0x7f, 0x7f], &[0x7f])]),
        section(2, &bad_imp),
    ]);
    assert!(matches!(compile_admit_only(&bad_imp_mod), Err(X64CompileError::AdmissionRejected(_))));

    // 13. Rejected: malformed Wasm -> Failed (NOT AdmissionRejected, so CLI exits 1 not 4!).
    assert!(matches!(compile_admit_only(b"not-wasm"), Err(X64CompileError::Failed(_))));

    // 14. Synthetic `DecodedModule` defense-in-depth admission rejections (§4.3.3):
    //     descriptor types, ImportKind::ExactFunc, MAX_TOTAL_SIG_ARITY expansion,
    //     > MAX_EXPORTS, malformed `start` signature, MAX_AGGREGATE_BR_TABLE_ENTRIES,
    //     and MAX_BLOCKS_PER_FN.
    let base_empty = || {
        wtime_validate::validate(&wasm_module(&[]), &x64_validate_features(), &x64_validate_limits())
            .unwrap()
            .decoded()
            .clone()
    };

    // (a) Descriptor type (`st.descriptor.is_some()` / `st.describes.is_some()`).
    let mut dm_desc = base_empty();
    dm_desc.type_groups.push(wtime_types::TypeGroup {
        explicit: false,
        types: vec![wtime_types::SubType {
            final_: true,
            supertype: None,
            descriptor: Some(wtime_types::TypeIdx(0)),
            describes: None,
            composite: wtime_types::CompositeType::Func(FuncType { params: vec![], results: vec![] }),
        }],
    });
    assert!(admit_x64_module(&dm_desc).is_err(), "descriptor type must be rejected");

    // (b) `ImportKind::ExactFunc`.
    let mut dm_exact = base_empty();
    dm_exact.types.push(FuncType { params: vec![], results: vec![] });
    dm_exact.imports.push(wtime_validate::Import {
        module: "env".into(),
        name: "exact".into(),
        kind: wtime_validate::ImportKind::ExactFunc(wtime_types::TypeIdx(0)),
    });
    assert!(admit_x64_module(&dm_exact).is_err(), "ImportKind::ExactFunc must be rejected");

    // (c) `MAX_TOTAL_SIG_ARITY` via wide-result signature expansion across call sites.
    let mut dm_arity = base_empty();
    dm_arity.types.push(FuncType {
        params: vec![],
        results: vec![ValType::I32; 1000],
    });
    dm_arity.functions.push(wtime_types::TypeIdx(0));
    dm_arity.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: vec![
            wtime_validate::Instruction { opcode: 0x10, immediate: wtime_validate::Immediate::Index(0) };
            1005
        ],
        offsets: vec![0; 1005],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    assert!(
        admit_x64_module(&dm_arity).is_err(),
        "MAX_TOTAL_SIG_ARITY expansion must be rejected"
    );

    // (d) Export count > MAX_EXPORTS.
    let mut dm_exps = base_empty();
    dm_exps.types.push(FuncType { params: vec![], results: vec![] });
    dm_exps.functions.push(wtime_types::TypeIdx(0));
    dm_exps.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: vec![],
        offsets: vec![],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    dm_exps.exports = (0..=super::module::MAX_EXPORTS)
        .map(|i| wtime_validate::Export {
            name: format!("e{i}"),
            kind: wtime_validate::ExternalKind::Func,
            index: 0,
        })
        .collect();
    assert!(admit_x64_module(&dm_exps).is_err(), "> MAX_EXPORTS must be rejected");

    // (e) Malformed `start` function signature (`(i32)->()` instead of `()->()`).
    let mut dm_start = base_empty();
    dm_start.types.push(FuncType { params: vec![ValType::I32], results: vec![] });
    dm_start.functions.push(wtime_types::TypeIdx(0));
    dm_start.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: vec![],
        offsets: vec![],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    dm_start.start = Some(wtime_types::FuncIdx(0));
    assert!(admit_x64_module(&dm_start).is_err(), "malformed start signature must be rejected");

    // (f) Aggregate `br_table` entries > MAX_AGGREGATE_BR_TABLE_ENTRIES.
    let mut dm_brt = base_empty();
    dm_brt.types.push(FuncType { params: vec![], results: vec![] });
    dm_brt.functions.push(wtime_types::TypeIdx(0));
    let big_targets: Box<[u32]> = vec![0u32; 1_000_000].into_boxed_slice();
    dm_brt.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: vec![
            wtime_validate::Instruction {
                opcode: 0x0e,
                immediate: wtime_validate::Immediate::BrTable(big_targets, 0),
            };
            5
        ],
        offsets: vec![0; 5],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    assert!(
        admit_x64_module(&dm_brt).is_err(),
        "MAX_AGGREGATE_BR_TABLE_ENTRIES overflow must be rejected"
    );

    // (g) `MAX_BLOCKS_PER_FN` cap enforced by IR lowering.
    let mut dm_blks = base_empty();
    dm_blks.types.push(FuncType { params: vec![], results: vec![] });
    dm_blks.functions.push(wtime_types::TypeIdx(0));
    let n_blocks = super::module::MAX_BLOCKS_PER_FN + 2;
    let mut insns = Vec::with_capacity(n_blocks * 2 + 1);
    for _ in 0..n_blocks {
        insns.push(wtime_validate::Instruction {
            opcode: 0x02,
            immediate: wtime_validate::Immediate::Block(-64),
        });
        insns.push(wtime_validate::Instruction {
            opcode: 0x0b,
            immediate: wtime_validate::Immediate::None,
        });
    }
    insns.push(wtime_validate::Instruction {
        opcode: 0x0b,
        immediate: wtime_validate::Immediate::None,
    });
    dm_blks.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: insns,
        offsets: vec![],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    let mod_blks = admit_x64_module(&dm_blks).unwrap();
    assert!(
        lower_wasm_to_ir(&dm_blks, &mod_blks, 0).is_err(),
        "MAX_BLOCKS_PER_FN overflow must be rejected"
    );
}

#[test]
fn verify_ir_and_alloc_pass_on_unopt_and_opt_for_gdev_if_present() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../gdev/.pen/runtime/game.wasm");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let validated = wtime_validate::validate(&bytes, &x64_validate_features(), &x64_validate_limits())
        .expect("gdev game.wasm should validate under Stage 1 features");
    let decoded = validated.decoded();
    let module = admit_x64_module(decoded).expect("gdev game.wasm should pass Stage 1 admission");
    // Verify a deterministic sample of >600 real gdev functions in BOTH raw unopt AND full Stage 2 opt
    // (including inlining, effect analysis, LICM, GVN/BCE, diamond if-conversion, and Step 8a/8b elision)!
    let mut stage2_irs: Vec<IrFunc> = (0..decoded.code.len())
        .map(|bi| lower_wasm_to_ir(decoded, &module, module.imports.len() + bi).unwrap())
        .collect();
    super::opt::inline_small_helpers(&mut stage2_irs, &module, None).unwrap();
    let effects = super::opt::compute_module_effects(&stage2_irs, &module, None).unwrap();
    for bi in (0..decoded.code.len()).step_by(8) {
        let fi = module.imports.len() + bi;
        let ir_u = lower_wasm_to_ir_unopt(decoded, &module, fi).unwrap();
        verify_ir(&ir_u, &module).unwrap();
        let alloc_u = allocate(&ir_u).unwrap();
        verify_alloc(&ir_u, &alloc_u).unwrap();

        super::opt::optimize_func_stage2(&mut stage2_irs[bi], &module, &effects, None).unwrap();
        verify_ir(&stage2_irs[bi], &module).unwrap();
        let alloc_o = allocate(&stage2_irs[bi]).unwrap();
        verify_alloc(&stage2_irs[bi], &alloc_o).unwrap();
    }
    // Full parallel build determinism check: `--jobs 1` vs `--jobs 8` MUST emit byte-identical `.so`!
    let so_j1 = compile_wasm_to_x64_so(
        &bytes,
        &X64CompileOptions {
            isa: TargetIsa { has_abm_bmi1: true },
            jobs: 1,
            deadline: None,
            opt: true,
            verify: false,
            hooks: Default::default(),
        },
    )
    .unwrap();
    let so_j8 = compile_wasm_to_x64_so(
        &bytes,
        &X64CompileOptions {
            isa: TargetIsa { has_abm_bmi1: true },
            jobs: 8,
            deadline: None,
            opt: true,
            verify: false,
            hooks: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(so_j1, so_j8, "x64 parallel compilation must produce 100% deterministic ELF bytes");
}

// ---- Stage 2 Unit Tests: new asm encodings, Granlund–Montgomery magic div, opt.rs passes ----

#[test]
fn stage2_asm_encodings_and_magic_div_exhaustive() {
    // Verify Stage 2 memory-operand ALU & widening mul encodings byte-for-byte.
    let mut a = Asm::new();
    a.add_rm(false, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    assert_eq!(a.code, [0x03, 0x44, 0x24, 0x10]);

    let mut a = Asm::new();
    a.sub_rm(true, Gp::R8, Mem::base_disp(Gp::Rsp, 8));
    assert_eq!(a.code, [0x4c, 0x2b, 0x44, 0x24, 0x08]);

    let mut a = Asm::new();
    a.and_rm(false, Gp::Rcx, Mem::base(Gp::Rax));
    assert_eq!(a.code, [0x23, 0x08]);

    let mut a = Asm::new();
    a.or_rm(false, Gp::Rdx, Mem::base(Gp::Rax));
    assert_eq!(a.code, [0x0b, 0x10]);

    let mut a = Asm::new();
    a.xor_rm(true, Gp::Rsi, Mem::base(Gp::Rax));
    assert_eq!(a.code, [0x48, 0x33, 0x30]);

    let mut a = Asm::new();
    a.imul_rm(true, Gp::R9, Mem::base_disp(Gp::R14, 32));
    assert_eq!(a.code, [0x4d, 0x0f, 0xaf, 0x4e, 0x20]);

    let mut a = Asm::new();
    a.mul_r(true, Gp::R10);
    assert_eq!(a.code, [0x49, 0xf7, 0xe2]);

    let mut a = Asm::new();
    a.imul1_r(true, Gp::R11);
    assert_eq!(a.code, [0x49, 0xf7, 0xeb]);

    // Exhaustive verification of 32-bit and 64-bit unsigned & signed Granlund–Montgomery magic constants.
    let u32_divs: &[u32] = &[
        3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 17, 19, 25, 31, 63, 100, 127, 255, 1000, 65535,
        0x3fff_ffff, 0x4000_0001, 0x7fff_ffff, 0x8000_0001, 0xffff_fffe, 0xffff_ffff,
    ];
    for &d in u32_divs {
        let (m, sh, add) = super::lower::magic_u(32, u64::from(d));
        let mut samples = vec![0u32, 1, 2, 3, d - 1, d, d.saturating_add(1), u32::MAX - 1, u32::MAX];
        samples.extend([7, 10, 99, 1023, 0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0xdead_beef]);
        for n in samples {
            let t = ((u64::from(n) * m) >> 32) as u32;
            let q = if add {
                ((n - t) >> 1).wrapping_add(t) >> (sh - 1)
            } else {
                t >> sh
            };
            assert_eq!(q, n / d, "magic_u32 mismatch for {n} / {d}");
        }
    }

    let u64_divs: &[u64] = &[
        3, 5, 6, 7, 9, 10, 11, 13, 15, 100, 1000, 0x3fff_ffff, 0x7fff_ffff, 0x8000_0001,
        0xffff_ffff, (1u64 << 63) + 1, u64::MAX - 1, u64::MAX,
    ];
    for &d in u64_divs {
        let (m, sh, add) = super::lower::magic_u(64, d);
        let samples = [
            0u64, 1, 2, d - 1, d, d.saturating_add(1), 0x7fff_ffff_ffff_ffff,
            0x8000_0000_0000_0000, 0x8000_0000_0000_0001, 0x1234_5678_9abc_def0, u64::MAX - 1, u64::MAX,
        ];
        for n in samples {
            let t = ((u128::from(n) * u128::from(m)) >> 64) as u64;
            let q = if add {
                ((n - t) >> 1).wrapping_add(t) >> (sh - 1)
            } else {
                t >> sh
            };
            assert_eq!(q, n / d, "magic_u64 mismatch for {n} / {d}");
        }
    }

    let i32_divs: &[i32] = &[
        3, -3, 5, -5, 6, -6, 7, -7, 9, -9, 10, -10, 12, -12, 25, -25, 100, -100,
        0x3fff_ffff, -0x3fff_ffff, i32::MAX, -i32::MAX, i32::MIN + 1,
    ];
    for &d in i32_divs {
        let (m, sh, add_sub) = super::lower::magic_s(32, i64::from(d));
        let samples = [
            0i32, 1, -1, 2, -2, d.wrapping_sub(1), d, d.wrapping_add(1),
            d.wrapping_neg().wrapping_sub(1), d.wrapping_neg(), d.wrapping_neg().wrapping_add(1),
            12345, -12345, i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX,
        ];
        for n in samples {
            let mut q = ((i64::from(n) * m) >> 32) as i32;
            if add_sub > 0 {
                q = q.wrapping_add(n);
            } else if add_sub < 0 {
                q = q.wrapping_sub(n);
            }
            if sh > 0 {
                q >>= sh;
            }
            q = q.wrapping_add(((q as u32) >> 31) as i32);
            assert_eq!(q, n / d, "magic_s32 mismatch for {n} / {d}");
        }
    }

    let i64_divs: &[i64] = &[
        3, -3, 5, -5, 7, -7, 10, -10, 100, -100, 1000, -1000,
        0x7fff_ffff, -0x7fff_ffff, i64::MAX, -i64::MAX, i64::MIN + 1,
    ];
    for &d in i64_divs {
        let (m, sh, add_sub) = super::lower::magic_s(64, d);
        let samples = [
            0i64, 1, -1, 2, -2, d.wrapping_sub(1), d, d.wrapping_add(1),
            d.wrapping_neg(), 123456789, -123456789, i64::MIN, i64::MIN + 1, i64::MAX - 1, i64::MAX,
        ];
        for n in samples {
            let mut q = ((i128::from(n) * i128::from(m)) >> 64) as i64;
            if add_sub > 0 {
                q = q.wrapping_add(n);
            } else if add_sub < 0 {
                q = q.wrapping_sub(n);
            }
            if sh > 0 {
                q >>= sh;
            }
            q = q.wrapping_add(((q as u64) >> 63) as i64);
            assert_eq!(q, n / d, "magic_s64 mismatch for {n} / {d}");
        }
    }
}

#[test]
fn stage2_opt_inlining_licm_wrap_bce_diamond_and_stack_trap_elision() {
    use super::opt;

    // Module 1 (bounded 1-page memory):
    //   g0 = mut i32 (10, pinned), g1 = mut i32 (20, read-only inside licm_fn)
    //   f0: small_leaf(x) = x * 3 + 7 (internal, pure, inlined into f2, elided stack + !fn_can_trap)
    //   f1: medium_pure(x) = >50 pure adds (internal, NOT inlined due to >48 insts, still elided stack + !fn_can_trap!)
    //   f2: entry_caller(x) = calls f0(x) + f1(x) + diamond if(x > 5){x+1}else{x-1} (exported -> retains stack check + elided_callee_stack_bytes > 0!)
    //   f3: rec_fn(x) = if x <= 0 { 0 } else { rec_fn(x-1) + 1 } (recursive -> MUST retain stack check!)
    //   f4: tbl_fn(x) = x + 2 (in call_indirect table -> MUST retain stack check!)
    //   f5: licm_fn(n) = loop reading g1 (pure -> hoisted!) and conditionally loading mem[n] + dividing by n (trapping -> NEVER hoisted!)
    //   f6: wrap_bce(p) = load(p) + load(p + 8) -> in 1-page memory, p + 8 MUST fold to (p, offset=8)!
    let mut glob_p = vec![2];
    glob_p.extend_from_slice(&[0x7f, 1, 0x41, 10, 0x0b]); // g0
    glob_p.extend_from_slice(&[0x7f, 1, 0x41, 20, 0x0b]); // g1

    let b0 = vec![0x20, 0, 0x41, 3, 0x6c, 0x41, 7, 0x6a]; // small_leaf
    let mut b1 = vec![0x20, 0];
    for k in 1..=55i32 {
        // Alternate add & xor so local folding doesn't collapse below 48 insts.
        b1.push(0x41);
        leb_i32(&mut b1, k);
        b1.push(if k % 2 == 0 { 0x6a } else { 0x73 });
    }
    // f2: call f0(x) + call f1(x) + if(x >s 5){x+1}else{x-1} + write g0 so g0 is pinned, write g1 in f2 only
    let b2 = vec![
        0x23, 0, 0x20, 0, 0x6a, 0x24, 0, // g0 += x (makes g0 hottest -> pinned!)
        0x20, 0, 0x24, 1, // g1 = x (makes g1 mutable/unfolded!)
        0x20, 0, 0x10, 0, // call f0
        0x20, 0, 0x10, 1, // call f1
        0x6a,
        0x20, 0, 0x41, 5, 0x4a, 0x04, 0x7f, 0x20, 0, 0x41, 1, 0x6a, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x0b,
        0x6a,
        0x20, 0, 0x10, 3, 0x6a, // call rec_fn
        0x20, 0, 0x41, 0, 0x11, 0, 0, 0x6a, // call_indirect tbl_fn
    ];
    let b3 = vec![
        0x20, 0, 0x41, 0, 0x4c, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 3, 0x41, 1, 0x6a, 0x0b,
    ];
    let b4 = vec![0x20, 0, 0x41, 2, 0x6a];
    // f5(n): local i=0, sum=0; loop { br_if(i >=u n); sum += global.get g1 + memory.size; if (i > 0) { sum += load(n) + (100 / n); } i += 1; br loop; }
    let b5 = vec![
        0x02, 0x40, 0x03, 0x40,
        0x20, 1, 0x20, 0, 0x4f, 0x0d, 1,
        0x20, 2, 0x23, 1, 0x3f, 0, 0x6a, 0x6a, 0x21, 2, // sum += g1 + mem.size
        0x20, 1, 0x41, 0, 0x4a, 0x04, 0x40,
        0x20, 2, 0x20, 0, 0x28, 2, 0, 0x41, 100, 0x20, 0, 0x6d, 0x6a, 0x6a, 0x21, 2,
        0x0b,
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1,
        0x0c, 0,
        0x0b, 0x0b,
        0x20, 2,
    ];
    // f6(p): load(p) + load(p + 8)
    let b6 = vec![0x20, 0, 0x28, 2, 0, 0x20, 0, 0x41, 8, 0x6a, 0x28, 2, 0, 0x6a];

    let build_mod = |mem_sec: &[u8]| {
        wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0, 0, 0, 0, 0, 0, 0]),
            section(4, &[1, 0x70, 0x00, 1]), // funcref table len=1
            section(5, mem_sec),
            section(6, &glob_p),
            export_sec(&[("entry_caller", 0, 2), ("licm_fn", 0, 5), ("wrap_bce", 0, 6)]),
            section(9, &[1, 0x00, 0x41, 0, 0x0b, 1, 4]), // table[0] = f4
            code_sec(&[
                enc_body(&[], &b0),
                enc_body(&[], &b1),
                enc_body(&[], &b2),
                enc_body(&[], &b3),
                enc_body(&[], &b4),
                enc_body(&[(2, 0x7f)], &b5),
                enc_body(&[], &b6),
            ]),
        ])
    };

    // Bounded growable 4-page memory (`min=1, max=4` -> 256 KiB < 4 GiB, non-fixed so MemSize is NOT const-folded):
    let wasm_1p = build_mod(&[1, 0x01, 1, 4]);
    let val = wtime_validate::validate(&wasm_1p, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec = val.decoded();
    let module = admit_x64_module(dec).unwrap();
    let mut irs: Vec<IrFunc> = (0..dec.code.len())
        .map(|bi| lower_wasm_to_ir(dec, &module, bi).unwrap())
        .collect();
    opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    let effects = opt::compute_module_effects(&irs, &module, None).unwrap();
    for ir in &mut irs {
        opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
        verify_ir(ir, &module).unwrap();
    }
    let allocs: Vec<_> = irs.iter().map(|ir| allocate(ir).unwrap()).collect();
    for (ir, al) in irs.iter().zip(&allocs) {
        verify_alloc(ir, al).unwrap();
    }
    let fn_can_trap = opt::run_stack_and_trap_elision(&mut irs, &allocs, &module, None).unwrap();

    // 1. Small leaf f0 was inlined into f2 (no Op::Call { func: 0 } left in f2!), while f1 was NOT inlined.
    let f2_calls: Vec<u32> = irs[2]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| if let Op::Call { func, .. } = i.op { Some(func) } else { None })
        .collect();
    assert!(!f2_calls.contains(&0), "small_leaf f0 must be inlined into f2");
    assert!(f2_calls.contains(&1), "medium_pure f1 (>48 insts) must remain a direct call");

    // 2. Diamond if/else in f2 converted to branchless Op::CmpSelect!
    assert!(
        irs[2].blocks.iter().any(|b| b.insts.iter().any(|i| matches!(i.op, Op::CmpSelect { .. }))),
        "trivial pure diamond in f2 must lower to Op::CmpSelect"
    );

    // 3. Step 8a + 8b invariants:
    assert!(!irs[0].needs_stack_check && !fn_can_trap[0], "internal pure leaf f0 elides stack + trap check");
    assert!(!irs[1].needs_stack_check && !fn_can_trap[1], "internal pure medium f1 elides stack + trap check");
    assert!(irs[2].needs_stack_check && fn_can_trap[2], "exported f2 MUST retain stack check");
    assert!(irs[2].elided_callee_stack_bytes > 0, "retaining caller f2 MUST reserve elided callee f1 stack bytes");
    assert!(irs[3].needs_stack_check && fn_can_trap[3], "recursive f3 MUST retain stack check");
    assert!(irs[4].needs_stack_check && fn_can_trap[4], "call_indirect target f4 MUST retain stack check");

    // 4. LICM legality in f5: pure GlobalGet(1) & MemSize hoisted into preheader bb0, while trapping Load/Div stay inside loop!
    assert!(
        irs[5].blocks[0].insts.iter().any(|i| matches!(i.op, Op::GlobalGet(1))),
        "loop-invariant GlobalGet(1) must hoist to preheader bb0"
    );
    assert!(
        irs[5].blocks[0].insts.iter().any(|i| matches!(i.op, Op::MemSize)),
        "loop-invariant MemSize must hoist to preheader bb0"
    );
    assert!(
        !irs[5].blocks[0].insts.iter().any(|i| matches!(i.op, Op::Load { .. } | Op::I32Bin(IntBinOp::DivS, _, _))),
        "trapping Load/Div inside loop must NEVER hoist to preheader bb0!"
    );

    // 5. Wrap-safe addressing-mode folding in bounded 1-page memory: f6's second load folds to (p, offset=8)!
    assert!(
        irs[6].blocks[0].insts.iter().any(|i| matches!(i.op, Op::Load { offset: 8, .. })),
        "bounded 1-page memory must fold load(p + 8) to offset=8"
    );

    // 6. Unbounded 4 GiB memory (`min=1, no max`): load(p + 8) MUST NOT fold (can wrap at 0xffff_fff8)!
    let wasm_4g = build_mod(&[1, 0x00, 1]);
    let val4 = wtime_validate::validate(&wasm_4g, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec4 = val4.decoded();
    let mod4 = admit_x64_module(dec4).unwrap();
    let mut irs4: Vec<IrFunc> = (0..dec4.code.len())
        .map(|bi| lower_wasm_to_ir(dec4, &mod4, bi).unwrap())
        .collect();
    let eff4 = opt::compute_module_effects(&irs4, &mod4, None).unwrap();
    opt::optimize_func_stage2(&mut irs4[6], &mod4, &eff4, None).unwrap();
    verify_ir(&irs4[6], &mod4).unwrap();
    assert!(
        !irs4[6].blocks[0].insts.iter().any(|i| matches!(i.op, Op::Load { offset: 8, .. })),
        "unbounded 4 GiB memory MUST NOT wrap-fold load(p + 8)!"
    );
}

#[test]
fn stage2_audit_br_table_not_inlined_and_val_pool_cap_atomic() {
    use super::opt;
    // Callee f0 has 0 instructions + a br_table with 64 targets; caller f1 calls f0 twice.
    // `inline_small_helpers` MUST skip f0 (no br_table duplication DoS) and leave f1 valid!
    let mut b0 = vec![0x02, 0x40, 0x20, 0, 0x0e, 64];
    b0.extend(std::iter::repeat_n(0u8, 64));
    b0.push(0); // default
    b0.push(0x0b); // end block
    let b1 = vec![0x20, 0, 0x10, 0, 0x20, 0, 0x10, 0];
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[])]),
        func_sec(&[0, 0]),
        export_sec(&[("caller", 0, 1)]),
        code_sec(&[enc_body(&[], &b0), enc_body(&[], &b1)]),
    ]);
    let val = wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec = val.decoded();
    let module = admit_x64_module(dec).unwrap();
    let mut irs: Vec<IrFunc> = (0..dec.code.len())
        .map(|bi| lower_wasm_to_ir(dec, &module, bi).unwrap())
        .collect();
    opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    assert!(irs[1].br_tables.is_empty(), "callee with br_table must not be inlined");
    verify_ir(&irs[1], &module).unwrap();

    // Atomic val_pool rejection: pad caller.val_pool near MAX_VAL_POOL_PER_FN and verify
    // `inline_small_helpers` leaves caller completely intact (no Term::Unreachable corruption!).
    let b_leaf = vec![0x20, 0, 0x41, 1, 0x6a];
    let b_call = vec![0x20, 0, 0x10, 0];
    let wasm2 = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        export_sec(&[("call_leaf", 0, 1)]),
        code_sec(&[enc_body(&[], &b_leaf), enc_body(&[], &b_call)]),
    ]);
    let val2 = wtime_validate::validate(&wasm2, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec2 = val2.decoded();
    let mod2 = admit_x64_module(dec2).unwrap();
    let mut irs2: Vec<IrFunc> = (0..dec2.code.len())
        .map(|bi| lower_wasm_to_ir(dec2, &mod2, bi).unwrap())
        .collect();
    // Fill caller's val_pool to MAX_VAL_POOL_PER_FN so cloning callee's return val_range exceeds cap.
    irs2[1].val_pool.resize(super::module::MAX_VAL_POOL_PER_FN, super::ir::Val(0));
    opt::inline_small_helpers(&mut irs2, &mod2, None).unwrap();
    verify_ir(&irs2[1], &mod2).unwrap();
    assert!(
        irs2[1].blocks[0].insts.iter().any(|i| matches!(i.op, Op::Call { func: 0, .. })),
        "rejected inline due to val_pool cap must leave original Op::Call untouched"
    );
}
