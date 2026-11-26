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

pub(crate) fn spectre_mitigations_wasm() -> Vec<u8> {
    wasm_module(&[
        type_sec(&[(&[], &[0x7f]), (&[0x7f], &[0x7f])]),
        func_sec(&[0, 1]),
        section(4, &[2, 0x70, 1, 6, 6, 0x6f, 1, 7, 7]),
        section(5, &[1, 1, 1, 1]),
        export_sec(&[("run", 0, 1), ("memory", 2, 0)]),
        section(9, &[1, 0, 0x41, 0, 0x0b, 5, 0, 0, 0, 0, 0]),
        code_sec(&[
            enc_body(&[], &[0x41, 7]),
            enc_body(
                &[],
                &[
                    0x20, 0, 0x41, 42, 0x36, 2, 8, // store 42 at arg + 8
                    0x02, 0x40, 0x02, 0x40, // nested blocks
                    0x20, 0, 0x0e, 2, 0, 1, 1, // br_table inner, outer, default outer
                    0x0b, 0x20, 0, 0x28, 2, 0, 0x0f, // inner: return mem[arg]
                    0x0b, // outer: read memory, test externref for null and call through table 0
                    0x20, 0, 0x28, 2, 0, 0x20, 0, 0x25, 1, 0xd1, 0x6a, 0x20, 0, 0x11, 0, 0, 0x6a,
                ],
            ),
        ]),
    ])
}

#[test]
fn x64_spectre_mitigations_emit_fences_and_separate_incremental_cache_keys() {
    let wasm = spectre_mitigations_wasm();
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-spectre");
    for spectre_mitigations in [false, true] {
        let opts = X64CompileOptions {
            spectre_mitigations,
            verify: true,
            inc_cache_dir: Some(root.0.clone()),
            cache_key_salt: Some([0x42; 32]),
            ..Default::default()
        };
        let image = compile_wasm_to_x64_so(&wasm, &opts).unwrap();
        // The second ELF64 program header describes the executable .text segment.
        let read_u64 = |off| u64::from_le_bytes(image[off..off + 8].try_into().unwrap()) as usize;
        let ph = read_u64(32) + 56;
        let text_off = read_u64(ph + 8);
        let text_len = read_u64(ph + 32);
        let text = &image[text_off..text_off + text_len];
        assert_eq!(
            text.windows(3).any(|bytes| bytes == [0x0f, 0xae, 0xe8]),
            spectre_mitigations
        );
        assert_eq!(compile_wasm_to_x64_so(&wasm, &opts).unwrap(), image);

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if let Some(driver) = super::tests_oracle::get_oracle_driver() {
            use std::io::Write as _;
            use std::process::{Command, Stdio};

            let path = root.0.join(format!("spectre-{spectre_mitigations}.so"));
            std::fs::write(&path, image).unwrap();
            let mut child = Command::new(driver)
                .arg(path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"call run 0\ncall run 1\ncall run 5\ncall run 6\ncall run 10000\n")
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            assert_eq!(
                std::str::from_utf8(&out.stdout).unwrap(),
                "NEW_OK\n\
                 CALL 0 0000000000000000 0 0\n\
                 CALL 0 0000000000000008 0 0\n\
                 CALL 1 deadbeefcafebabe 4 0\n\
                 CALL 1 deadbeefcafebabe 3 0\n\
                 CALL 1 deadbeefcafebabe 2 0\n"
            );
        }
    }
    // Each ABI hash yields a separate base file even with identical Wasm and salt.
    let cache_entries = std::fs::read_dir(root.0.join("x64-inc"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .count();
    assert_eq!(cache_entries, 2);
}

pub(crate) fn spectre_bulk_and_scrub_wasm() -> Vec<u8> {
    let mut run_body = Vec::new();
    let push_i32 = |out: &mut Vec<u8>, v: i32| {
        out.push(0x41);
        leb_i32(out, v);
    };
    let push_i64 = |out: &mut Vec<u8>, v: i64| {
        out.push(0x42);
        leb_i64(out, v);
    };
    // mode == 0: exercise bulk mem/table helpers, extra memory, and mutable global, return 337.
    run_body.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x40]); // if (mode == 0)
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 0x5a);
    push_i32(&mut run_body, 16);
    run_body.extend_from_slice(&[0xfc, 11, 0]); // memory.fill 0
    push_i32(&mut run_body, 8);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 16);
    run_body.extend_from_slice(&[0xfc, 10, 0, 0]); // memory.copy 0 <- 0 (overlap bwd)
    push_i32(&mut run_body, 32);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 16);
    run_body.extend_from_slice(&[0xfc, 10, 0, 0]); // memory.copy 0 <- 0 (fwd)
    push_i32(&mut run_body, 64);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 4);
    run_body.extend_from_slice(&[0xfc, 8, 0, 0]); // memory.init seg=0 mem=0
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 0x7c);
    push_i32(&mut run_body, 16);
    run_body.extend_from_slice(&[0xfc, 11, 1]); // memory.fill 1
    push_i32(&mut run_body, 80);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 8);
    run_body.extend_from_slice(&[0xfc, 10, 0, 1]); // memory.copy 0 <- 1
    run_body.extend_from_slice(&[0xd2, 0, 0x41, 2, 0xfc, 15, 0, 0x1a]); // table.grow 0 +2
    run_body.extend_from_slice(&[0x41, 0, 0x41, 0, 0x41, 2, 0xfc, 12, 0, 0]); // table.init 0 0
    run_body.extend_from_slice(&[0x41, 1, 0x41, 0, 0x41, 2, 0xfc, 14, 0, 0]); // table.copy 0 0
    run_body.extend_from_slice(&[0x41, 4, 0xd2, 1, 0x41, 2, 0xfc, 17, 0]); // table.fill 0
    push_i64(&mut run_body, 0x1122_3344_5566_7788u64 as i64);
    run_body.extend_from_slice(&[0x24, 0]); // global.set 0
    push_i32(&mut run_body, 8);
    run_body.extend_from_slice(&[0x2d, 0, 0]); // i32.load8_u mem0[8] == 0x5a (90)
    push_i32(&mut run_body, 64);
    run_body.extend_from_slice(&[0x2d, 0, 0, 0x6a]); // + mem0[64] == 'A' (65)
    push_i32(&mut run_body, 80);
    run_body.extend_from_slice(&[0x2d, 0, 0, 0x6a]); // + mem0[80] == 0x7c (124)
    run_body.extend_from_slice(&[0x41, 2, 0x11, 0, 0, 0x6a]); // + call_indirect(2) == 29
    run_body.extend_from_slice(&[0x41, 5, 0x11, 0, 0, 0x6a]); // + call_indirect(5) == 29
    run_body.extend_from_slice(&[0x0f, 0x0b]); // return; end

    // mode == 1: valid len == 0 at end of memory 0 -> 99.
    run_body.extend_from_slice(&[0x20, 0, 0x41, 1, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 65536);
    push_i32(&mut run_body, 65536);
    push_i32(&mut run_body, 0);
    run_body.extend_from_slice(&[0xfc, 10, 0, 0]);
    push_i32(&mut run_body, 65536);
    push_i32(&mut run_body, 1);
    push_i32(&mut run_body, 0);
    run_body.extend_from_slice(&[0xfc, 11, 0]);
    push_i32(&mut run_body, 99);
    run_body.extend_from_slice(&[0x0f, 0x0b]);

    // mode == 2: OOB memory.copy -> TRAP_OOB
    run_body.extend_from_slice(&[0x20, 0, 0x41, 2, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 65530);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 10);
    run_body.extend_from_slice(&[0xfc, 10, 0, 0, 0x0b]);

    // mode == 3: OOB memory.fill -> TRAP_OOB
    run_body.extend_from_slice(&[0x20, 0, 0x41, 3, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 65530);
    push_i32(&mut run_body, 1);
    push_i32(&mut run_body, 10);
    run_body.extend_from_slice(&[0xfc, 11, 0, 0x0b]);

    // mode == 4: OOB table.copy -> TRAP_TABLE_OOB
    run_body.extend_from_slice(&[0x20, 0, 0x41, 4, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 5);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 4);
    run_body.extend_from_slice(&[0xfc, 14, 0, 0, 0x0b]);

    // mode == 5: OOB memory.init -> TRAP_OOB
    run_body.extend_from_slice(&[0x20, 0, 0x41, 5, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 2);
    push_i32(&mut run_body, 4);
    run_body.extend_from_slice(&[0xfc, 8, 0, 0, 0x0b]);

    // mode == 6: OOB table.init -> TRAP_TABLE_OOB
    run_body.extend_from_slice(&[0x20, 0, 0x41, 6, 0x46, 0x04, 0x40]);
    push_i32(&mut run_body, 0);
    push_i32(&mut run_body, 1);
    push_i32(&mut run_body, 4);
    run_body.extend_from_slice(&[0xfc, 12, 0, 0, 0x0b]);

    run_body.extend_from_slice(&[0x41, 0]);

    wasm_module(&[
        type_sec(&[(&[], &[0x7f]), (&[0x7f], &[0x7f])]),
        func_sec(&[0, 0, 1]),
        section(4, &[1, 0x70, 1, 4, 8]),
        section(5, &[2, 1, 1, 2, 1, 1, 2]),
        section(6, &[1, 0x7e, 1, 0x42, 0, 0x0b]),
        export_sec(&[("run", 0, 2), ("memory", 2, 0)]),
        section(9, &[1, 1, 0x00, 2, 0, 1]),
        section(12, &[1]),
        code_sec(&[
            enc_body(&[], &[0x41, 13]),
            enc_body(&[], &[0x41, 29]),
            enc_body(&[], &run_body),
        ]),
        section(11, &[1, 1, 4, b'A', b'B', b'C', b'D']),
    ])
}

#[test]
fn x64_spectre_bulk_helpers_and_drop_scrub() {
    let wasm = spectre_bulk_and_scrub_wasm();
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-spectre-bulk");
    let has_lfence = |bytes: &[u8]| bytes.windows(3).any(|w| w == [0x0f, 0xae, 0xe8]);
    let has_rep_stosb = |bytes: &[u8]| bytes.windows(2).any(|w| w == [0xf3, 0xaa]);

    // Check individual bulk helper codegen directly for lfence presence/absence.
    for spectre in [false, true] {
        let mc = super::runtime::emit_helper_mem_copy(false, false, spectre).unwrap();
        let mf = super::runtime::emit_helper_mem_fill(false, false, spectre).unwrap();
        assert_eq!(has_lfence(&mc.code), spectre);
        assert_eq!(has_lfence(&mf.code), spectre);
    }

    // Check full module helpers + blot_aot_drop scrub codegen + native Linux x86_64 execution.
    let validated =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let module = admit_x64_module(validated.decoded()).unwrap();
    for spectre in [false, true] {
        let layout = CtxLayout::compute(&module, Vec::new())
            .unwrap()
            .with_spectre_mitigations(spectre);
        let ro = super::RodataConsts::default();
        let tables = super::runtime::RuntimeTables::default();
        let helpers = super::runtime::emit_all_helpers(
            &module,
            &layout,
            &ro,
            &tables,
            super::runtime::RuntimeTestHooks::default(),
        )
        .unwrap();
        for id in [
            super::HELPER_MEM_COPY,
            super::HELPER_MEM_FILL,
            super::HELPER_EXTRA_MEM_COPY,
            super::HELPER_EXTRA_MEM_FILL,
            super::HELPER_MEM_INIT,
            super::HELPER_TABLE_INIT,
            super::HELPER_TABLE_COPY,
            super::HELPER_TABLE_GROW,
            super::HELPER_TABLE_FILL,
        ] {
            assert_eq!(
                has_lfence(&helpers[id as usize].code),
                spectre,
                "helper {id} lfence mismatch for spectre={spectre}"
            );
        }
        let exports = super::runtime::emit_all_exports(
            &module,
            &layout,
            &ro,
            &tables,
            TargetIsa::default(),
            super::runtime::RuntimeTestHooks::default(),
        )
        .unwrap();
        // EXPORT_NAMES[5] == "blot_aot_drop", EXPORT_NAMES[4] == "blot_aot_new"
        let drop_code = &exports[5].code;
        assert_eq!(has_lfence(drop_code), spectre);
        assert_eq!(has_rep_stosb(drop_code), spectre);
        let new_code = &exports[4].code;
        assert_eq!(has_lfence(new_code), spectre);
        assert_eq!(has_rep_stosb(new_code), spectre);

        let image = compile_wasm_to_x64_so(
            &wasm,
            &X64CompileOptions {
                spectre_mitigations: spectre,
                verify: true,
                ..Default::default()
            },
        )
        .unwrap();

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if let Some(driver) = super::tests_oracle::get_oracle_driver() {
            use std::io::Write as _;
            use std::process::{Command, Stdio};

            let path = root.0.join(format!("spectre-bulk-{spectre}.so"));
            std::fs::write(&path, image).unwrap();
            let mut child = Command::new(driver)
                .arg(path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(
                    b"call run 0\ncall run 1\ncall run 2\ncall run 3\ncall run 4\ncall run 5\ncall run 6\n",
                )
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            assert_eq!(
                std::str::from_utf8(&out.stdout).unwrap(),
                "NEW_OK\n\
                 CALL 0 0000000000000151 0 0\n\
                 CALL 0 0000000000000063 0 0\n\
                 CALL 1 deadbeefcafebabe 2 0\n\
                 CALL 1 deadbeefcafebabe 2 0\n\
                 CALL 1 deadbeefcafebabe 3 0\n\
                 CALL 1 deadbeefcafebabe 2 0\n\
                 CALL 1 deadbeefcafebabe 3 0\n"
            );
        }
    }
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

    // Track 2 additions: test_ri, adc_ri8, shrd_cl, xgetbv, vzeroupper, 256-bit AVX2 VEX, lea_rip_label
    let mut a = Asm::new();
    a.test_ri(false, Gp::Rbp, 8);
    assert_eq!(a.code, [0xf7, 0xc5, 0x08, 0x00, 0x00, 0x00]);
    let mut a = Asm::new();
    a.test_ri(true, Gp::R12, 1);
    assert_eq!(a.code, [0x49, 0xf7, 0xc4, 0x01, 0x00, 0x00, 0x00]);
    let mut a = Asm::new();
    a.adc_ri8(true, Gp::Rdx, 0);
    assert_eq!(a.code, [0x48, 0x83, 0xd2, 0x00]);
    let mut a = Asm::new();
    a.shrd_cl(true, Gp::Rax, Gp::Rdx);
    assert_eq!(a.code, [0x48, 0x0f, 0xad, 0xd0]);
    let mut a = Asm::new();
    a.xgetbv();
    assert_eq!(a.code, [0x0f, 0x01, 0xd0]);
    let mut a = Asm::new();
    a.vzeroupper();
    assert_eq!(a.code, [0xc5, 0xf8, 0x77]);
    let mut a = Asm::new();
    a.vmovdqu_ym(Xmm::Xmm8, Mem::base(Gp::R14));
    assert_eq!(a.code, [0xc4, 0x41, 0x7e, 0x6f, 0x06]);
    let mut a = Asm::new();
    a.vmovdqu_my(Mem::base_disp(Gp::Rdi, 32), Xmm::Xmm0);
    assert_eq!(a.code, [0xc4, 0xe1, 0x7e, 0x7f, 0x47, 0x20]);
    let mut a = Asm::new();
    a.vpbroadcastb_yx(Xmm::Xmm1, Xmm::Xmm0);
    assert_eq!(a.code, [0xc4, 0xe2, 0x7d, 0x78, 0xc8]);
    let mut a = Asm::new();
    a.vpxor_yyy(Xmm::Xmm2, Xmm::Xmm2, Xmm::Xmm2);
    assert_eq!(a.code, [0xc4, 0xe1, 0x6d, 0xef, 0xd2]);
    let mut a = Asm::new();
    a.vpsadbw_yym(Xmm::Xmm3, Xmm::Xmm1, Mem::base(Gp::Rax));
    assert_eq!(a.code, [0xc4, 0xe1, 0x75, 0xf6, 0x18]);
    let mut a = Asm::new();
    a.vpaddq_yyy(Xmm::Xmm2, Xmm::Xmm2, Xmm::Xmm3);
    assert_eq!(a.code, [0xc4, 0xe1, 0x6d, 0xd4, 0xd3]);
    let mut a = Asm::new();
    a.vextracti128_xy(Xmm::Xmm3, Xmm::Xmm2, 1);
    assert_eq!(a.code, [0xc4, 0xe3, 0x7d, 0x39, 0xd3, 0x01]);
    let mut a = Asm::new();
    let lbl = a.new_label();
    a.lea_rip_label(Gp::Rax, lbl);
    a.nop();
    a.bind(lbl);
    a.finish_labels().unwrap();
    assert_eq!(a.code, [0x48, 0x8d, 0x05, 0x01, 0x00, 0x00, 0x00, 0x90]);
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
        ExportDef {
            name: "sym_a",
            text_off: 0,
            size: 16,
        },
        ExportDef {
            name: "sym_c",
            text_off: 16,
            size: 16,
        },
    ];
    let img = layout.emit(&rodata, &text, &defs).unwrap();
    assert_eq!(&img[0..4], b"\x7fELF");
    // Locate DT_HASH table and verify chain links symbol 2 -> symbol 1 -> 0.
    // Find .hash section header (index 2).
    let shoff = u64::from_le_bytes(img[40..48].try_into().unwrap()) as usize;
    let hash_off = u64::from_le_bytes(
        img[shoff + 2 * 64 + 24..shoff + 2 * 64 + 32]
            .try_into()
            .unwrap(),
    ) as usize;
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
    assert!(
        layout2
            .emit(
                &[0u8; 32],
                &[0xc3],
                &[ExportDef {
                    name: "f",
                    text_off: 0,
                    size: 1
                }]
            )
            .is_err()
    );
}

// ---- 3. `CtxLayout` 16 MiB cap unit test (§4.3.2) ----

#[test]
fn ctx_layout_enforces_16_mib_cap() {
    let make_mod = |n_v128_globals: usize, extern_len: u32| X64Module {
        imports: Vec::new(),
        func_type_idx: Vec::new(),
        func_canon_sig: Vec::new(),
        module_types: Vec::new(),
        globals: vec![(ValType::V128, true); n_v128_globals],
        global_inits: vec![X64ConstVal::V128([0; 16]); n_v128_globals],
        global_consts: vec![None; n_v128_globals],
        pinned_global: None,
        exported_globals: Vec::new(),
        exported_funcs: Vec::new(),
        ci_index: Default::default(),
        ci_sigs: Vec::new(),
        uses_dyn_ci: false,
        uses_call_ref: false,
        table_lens: if extern_len > 0 {
            vec![extern_len]
        } else {
            Vec::new()
        },
        table_is64: if extern_len > 0 {
            vec![false]
        } else {
            Vec::new()
        },
        mut_extern_tables: if extern_len > 0 {
            [(0, extern_len)].into_iter().collect()
        } else {
            Default::default()
        },
        dyn_tables: Default::default(),
        memory: None,
        memory64: false,
        extra_memories: Vec::new(),
        tags: Vec::new(),
        uses_exceptions: false,
        fixed32: None,
        active_elem_checks: Vec::new(),
        active_data_segs: Vec::new(),
        data_seg_count: 0,
        passive_data_segs: Vec::new(),
        uses_bulk_data: false,
        elem_segs: Vec::new(),
        uses_bulk_elem: false,
        start: None,
        target: super::module::X64Target::BlotCdylib,
        standalone_start: None,
        standalone_goose_reserve: None,
    };

    // Exact boundary: CTX_HEADER_BYTES header + remaining 16-byte V128 globals = exactly 16 MiB.
    let max_v128 = (MAX_CTX_BYTES - super::CTX_HEADER_BYTES) / 16;
    let ok = CtxLayout::compute(&make_mod(max_v128, 0), Vec::new()).unwrap();
    assert_eq!(ok.total_bytes, MAX_CTX_BYTES);

    // One more global overflows the 16 MiB cap cleanly!
    let err = CtxLayout::compute(&make_mod(max_v128 + 1, 0), Vec::new());
    assert!(err.is_err(), "expected CtxLayout > 16 MiB to fail cleanly");

    // Also test overflow via externref table slots.
    let err_tbl = CtxLayout::compute(&make_mod(max_v128, 1), Vec::new());
    assert!(
        err_tbl.is_err(),
        "expected CtxLayout extern table overflow > 16 MiB to fail cleanly"
    );
}

// ---- 4. `CompileBudget` liveness scheduler contention tests (§4.3.3) ----

#[test]
fn liveness_scheduler_unblocks_waiting_worker_and_times_out_cleanly() {
    // (a) Waiting worker unblocks once competing guards drop.
    let budget = Arc::new(CompileBudget::new(Some(
        Instant::now() + Duration::from_secs(5),
    )));
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
    handle
        .join()
        .unwrap()
        .expect("waiting worker should succeed after capacity released");
    drop(held);

    // (b) Waiting worker times out cleanly when deadline expires while capacity is held.
    let tight = Arc::new(CompileBudget::new(Some(
        Instant::now() + Duration::from_millis(60),
    )));
    let mut held2 = Vec::with_capacity(n_fill);
    for _ in 0..n_fill {
        held2.push(tight.reserve_liveness(MAX_LIVENESS_BYTES).unwrap());
    }
    let err = tight.reserve_liveness(MAX_LIVENESS_BYTES);
    assert!(
        err.is_err(),
        "expected timeout while waiting for liveness capacity"
    );
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
        func_canon_sig: vec![0],
        module_types: vec![FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        }],
        globals: Vec::new(),
        global_inits: Vec::new(),
        global_consts: Vec::new(),
        pinned_global: None,
        exported_globals: Vec::new(),
        exported_funcs: Vec::new(),
        ci_index: Default::default(),
        ci_sigs: Vec::new(),
        uses_dyn_ci: false,
        uses_call_ref: false,
        table_lens: Vec::new(),
        table_is64: Vec::new(),
        mut_extern_tables: Default::default(),
        dyn_tables: Default::default(),
        memory: None,
        memory64: false,
        extra_memories: Vec::new(),
        tags: Vec::new(),
        uses_exceptions: false,
        fixed32: None,
        active_elem_checks: Vec::new(),
        active_data_segs: Vec::new(),
        data_seg_count: 0,
        passive_data_segs: Vec::new(),
        uses_bulk_data: false,
        elem_segs: Vec::new(),
        uses_bulk_elem: false,
        start: None,
        target: super::module::X64Target::BlotCdylib,
        standalone_start: None,
        standalone_goose_reserve: None,
    };

    // Valid simple IR: bb0(v0: I32) -> v1 = v0 + v0 -> Return(v1).
    let good_ir = IrFunc {
        func_index: 0,
        params: vec![Val(0)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32, Ty::I32],
        blocks: vec![Block {
            params: vec![Val(0)],
            insts: vec![Inst {
                res: Some(Val(1)),
                op: Op::I32Bin(IntBinOp::Add, Val(0), Val(0)),
            }],
            term: Term::Return {
                vals: ValRange { start: 0, len: 1 },
            },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
        memory64: false,
        early_pruned: 0,
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
            insts: vec![Inst {
                res: Some(Val(1)),
                op: Op::I64Eqz(Val(0)),
            }],
            term: Term::Return {
                vals: ValRange { start: 0, len: 1 },
            },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
        memory64: false,
        early_pruned: 0,
    };
    assert!(verify_ir(&bad_ty, &empty_mod).is_err());

    // Negative 2: Use-before-def in same block.
    bad_ty.blocks[0].insts = vec![Inst {
        res: Some(Val(1)),
        op: Op::I32Eqz(Val(1)),
    }];
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
                insts: vec![Inst {
                    res: Some(Val(1)),
                    op: Op::ConstI32(7),
                }],
                term: Term::Return {
                    vals: ValRange { start: 0, len: 1 },
                },
                is_loop_header: false,
                preds: 1,
            },
            Block {
                params: vec![],
                insts: vec![],
                term: Term::Return {
                    vals: ValRange { start: 0, len: 1 },
                },
                is_loop_header: false,
                preds: 1,
            },
        ],
        val_pool: vec![Val(1)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
        memory64: false,
        early_pruned: 0,
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
                Inst {
                    res: Some(Val(1)),
                    op: Op::ConstI32(10),
                },
                Inst {
                    res: Some(Val(2)),
                    op: Op::I32Bin(IntBinOp::Add, Val(0), Val(1)),
                },
            ],
            term: Term::Return {
                vals: ValRange { start: 0, len: 1 },
            },
            is_loop_header: false,
            preds: 0,
        }],
        val_pool: vec![Val(2)],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
        memory64: false,
        early_pruned: 0,
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
    assert_eq!(
        parse_x64_target_cpu_syntax("x86-64-v2").unwrap(),
        Some("x64-v2")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("generic").unwrap(),
        Some("x64-v2")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("x86-64-v3").unwrap(),
        Some("x64-v3")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("x86-64-v4").unwrap(),
        Some("x64-v3")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("znver4").unwrap(),
        Some("x64-v3")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("haswell").unwrap(),
        Some("x64-v3")
    );
    assert_eq!(
        parse_x64_target_cpu_syntax("graniterapids").unwrap(),
        Some("x64-v3")
    );
    assert!(parse_x64_target_cpu_syntax("znver").is_err());
    assert!(parse_x64_target_cpu_syntax("znver4a").is_err());
    assert!(parse_x64_target_cpu_syntax("pentium4").is_err());
    assert!(parse_x64_target_cpu_syntax("").is_err());
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    {
        let (tag, isa) = resolve_x64_isa("x86-64-v2").unwrap();
        assert_eq!(tag, "x64-v2");
        assert!(!isa.has_abm_bmi1);
        assert!(!isa.has_bmi2);
        let (tag3, isa3) = resolve_x64_isa("znver4").unwrap();
        assert_eq!(tag3, "x64-v3");
        assert!(isa3.has_abm_bmi1);
        assert!(isa3.has_bmi2);
    }
    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    )))]
    {
        assert!(resolve_x64_isa("x86-64-v2").is_err());
        assert!(resolve_x64_isa("znver4").is_err());
    }
}

// ---- 8. Admission acceptance vs rejection suite (§4.3.3) ----

fn compile_admit_only(wasm: &[u8]) -> Result<Vec<u8>, X64CompileError> {
    compile_wasm_to_x64_so(
        wasm,
        &X64CompileOptions {
            isa: TargetIsa::default(),
            jobs: 1,
            deadline: None,
            opt: true,
            verify: true,
            ..Default::default()
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
    assert!(
        compile_admit_only(&accepted).is_ok(),
        "Stage 1 accepted module must compile cleanly"
    );

    // 2. Rejected: funcref block type (-16 / 0x70) in LIVE code AND in DEAD (unreachable) code.
    // 2-3. Admitted: funcref blocks and `ref.null func` (0xd0 0x70) in live and dead code.
    let dead_funcref_block = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0x02, 0x70, 0x00, 0x0b, 0x1a])]), // unreachable; block funcref; unreachable; end; drop
    ]);
    assert!(compile_admit_only(&dead_funcref_block).is_ok());
    let live_funcref_block = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x02, 0x70, 0xd0, 0x70, 0x0b, 0x1a])]),
    ]);
    assert!(compile_admit_only(&live_funcref_block).is_ok());

    let dead_ref_null_func = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0xd0, 0x70, 0x1a])]),
    ]);
    assert!(compile_admit_only(&dead_ref_null_func).is_ok());

    // 4. Admitted: Wasm 3.0 `function-references` (explicit table initializer `0x40 0x00`,
    //    `call_ref`) and `table64` (`0x04` / `0x05` limits).
    let explicit_tbl_init = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        // table with 0x40 0x00 initializer: (table 1 funcref (ref.null func))
        section(4, &[1, 0x40, 0x00, 0x70, 0x00, 1, 0xd0, 0x70, 0x0b]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    assert!(compile_admit_only(&explicit_tbl_init).is_ok());
    let tbl64_and_call_ref = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        section(4, &[1, 0x40, 0x00, 0x64, 0x00, 0x05, 2, 4, 0xd2, 0, 0x0b]),
        section(9, &[1, 0x03, 0x00, 1, 0]),
        code_sec(&[enc_body(
            &[],
            &[0x20, 0, 0x42, 0, 0x25, 0, 0x14, 0], // (call_ref 0 (local.get 0) (table.get 0 (i64.const 0)))
        )]),
    ]);
    assert!(compile_admit_only(&tbl64_and_call_ref).is_ok());

    // 5. Rejected: Wasm GC non-final `(sub (func))` (`0x50 0x00 0x60 0x00 0x00`).
    let gc_sub_func = wasm_module(&[
        section(1, &[1, 0x50, 0x00, 0x60, 0x00, 0x00]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    assert!(matches!(
        compile_admit_only(&gc_sub_func),
        Err(X64CompileError::AdmissionRejected(_))
    ));

    // 6. Rejected: exported table.
    let exp_tbl = wasm_module(&[section(4, &[1, 0x70, 0x00, 1]), export_sec(&[("t", 1, 0)])]);
    assert!(matches!(
        compile_admit_only(&exp_tbl),
        Err(X64CompileError::AdmissionRejected(_))
    ));

    // 7. Admitted: growable externref table (min != max) backed by dynamic table descriptor.
    let unb_ext = wasm_module(&[section(4, &[1, 0x6f, 0x00, 4])]);
    assert!(compile_admit_only(&unb_ext).is_ok());

    // 8. Admitted: passive data segment.
    let passive_data = wasm_module(&[
        section(5, &[1, 0x00, 1]),
        section(12, &[1]),
        section(11, &[1, 0x01, 2, 0xaa, 0xbb]),
    ]);
    assert!(compile_admit_only(&passive_data).is_ok());

    // 9. Admitted: full 128-bit Wasm SIMD (e.g. i32x4.add = 0xfd 0xae 0x01) and memory64!
    let simd_mod = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0xfd, 0xae, 0x01, 0x1a])]),
    ]);
    assert!(compile_admit_only(&simd_mod).is_ok());

    let mem64 = wasm_module(&[section(5, &[1, 0x04, 1])]);
    assert!(compile_admit_only(&mem64).is_ok());

    // 10. Admitted: relaxed SIMD (0xfd 0x80 0x02 = i8x16.relaxed_swizzle) and tail_call (0x12).
    let relaxed_simd = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x00, 0xfd, 0x80, 0x02, 0x1a])]),
    ]);
    assert!(compile_admit_only(&relaxed_simd).is_ok());

    let tail_call_mod = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        code_sec(&[enc_body(&[], &[0x12, 0])]),
    ]);
    assert!(compile_admit_only(&tail_call_mod).is_ok());

    // 11. Admitted: multi-memory (`multi-memory` proposal); rejected: shared memory (`threads`).
    let multi_mem = wasm_module(&[section(5, &[2, 0x00, 1, 0x00, 1])]);
    assert!(compile_admit_only(&multi_mem).is_ok());
    let shared_mem = wasm_module(&[section(5, &[1, 0x03, 1, 2])]);
    assert!(matches!(
        compile_admit_only(&shared_mem),
        Err(X64CompileError::AdmissionRejected(_))
    ));

    // 12. Rejected: Blot import with unsupported signature (e.g. (i32, i32) -> (i32)).
    let mut bad_imp = vec![1];
    enc_str(&mut bad_imp, "blot:host/1");
    enc_str(&mut bad_imp, "bad");
    bad_imp.push(0x00);
    leb_u32(&mut bad_imp, 0);
    let bad_imp_mod = wasm_module(&[type_sec(&[(&[0x7f, 0x7f], &[0x7f])]), section(2, &bad_imp)]);
    assert!(matches!(
        compile_admit_only(&bad_imp_mod),
        Err(X64CompileError::AdmissionRejected(_))
    ));

    // 13. Rejected: malformed Wasm -> Failed (NOT AdmissionRejected, so CLI exits 1 not 4!).
    assert!(matches!(
        compile_admit_only(b"not-wasm"),
        Err(X64CompileError::Failed(_))
    ));

    // 14. Synthetic `DecodedModule` defense-in-depth admission rejections (§4.3.3):
    //     descriptor types, ImportKind::ExactFunc, MAX_TOTAL_SIG_ARITY expansion,
    //     > MAX_EXPORTS, malformed `start` signature, MAX_AGGREGATE_BR_TABLE_ENTRIES,
    //     and MAX_BLOCKS_PER_FN.
    let base_empty = || {
        wtime_validate::validate(
            &wasm_module(&[]),
            &x64_validate_features(),
            &x64_validate_limits(),
        )
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
            composite: wtime_types::CompositeType::Func(FuncType {
                params: vec![],
                results: vec![],
            }),
        }],
    });
    assert!(
        admit_x64_module(&dm_desc).is_err(),
        "descriptor type must be rejected"
    );

    // (b) `ImportKind::ExactFunc`.
    let mut dm_exact = base_empty();
    dm_exact.types.push(FuncType {
        params: vec![],
        results: vec![],
    });
    dm_exact.imports.push(wtime_validate::Import {
        module: "env".into(),
        name: "exact".into(),
        kind: wtime_validate::ImportKind::ExactFunc(wtime_types::TypeIdx(0)),
    });
    assert!(
        admit_x64_module(&dm_exact).is_err(),
        "ImportKind::ExactFunc must be rejected"
    );

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
            wtime_validate::Instruction {
                opcode: 0x10,
                immediate: wtime_validate::Immediate::Index(0)
            };
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
    dm_exps.types.push(FuncType {
        params: vec![],
        results: vec![],
    });
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
    assert!(
        admit_x64_module(&dm_exps).is_err(),
        "> MAX_EXPORTS must be rejected"
    );

    // (e) Malformed `start` function signature (`(i32)->()` instead of `()->()`).
    let mut dm_start = base_empty();
    dm_start.types.push(FuncType {
        params: vec![ValType::I32],
        results: vec![],
    });
    dm_start.functions.push(wtime_types::TypeIdx(0));
    dm_start.code.push(wtime_validate::FunctionBody {
        locals: vec![],
        instructions: vec![],
        offsets: vec![],
        span: wtime_types::Span { start: 0, end: 0 },
    });
    dm_start.start = Some(wtime_types::FuncIdx(0));
    assert!(
        admit_x64_module(&dm_start).is_err(),
        "malformed start signature must be rejected"
    );

    // (f) Aggregate `br_table` entries > MAX_AGGREGATE_BR_TABLE_ENTRIES.
    let mut dm_brt = base_empty();
    dm_brt.types.push(FuncType {
        params: vec![],
        results: vec![],
    });
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
    dm_blks.types.push(FuncType {
        params: vec![],
        results: vec![],
    });
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
#[ignore = "requires external gdev game.wasm"]
fn verify_ir_and_alloc_pass_on_unopt_and_opt_for_gdev_if_present() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../gdev/.pen/runtime/game.wasm");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let validated =
        wtime_validate::validate(&bytes, &x64_validate_features(), &x64_validate_limits())
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
            isa: TargetIsa {
                has_abm_bmi1: true,
                has_bmi2: true,
                has_avx2: true,
            },
            jobs: 1,
            deadline: None,
            opt: true,
            verify: false,
            ..Default::default()
        },
    )
    .unwrap();
    let so_j8 = compile_wasm_to_x64_so(
        &bytes,
        &X64CompileOptions {
            isa: TargetIsa {
                has_abm_bmi1: true,
                has_bmi2: true,
                has_avx2: true,
            },
            jobs: 8,
            deadline: None,
            opt: true,
            verify: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        so_j1, so_j8,
        "x64 parallel compilation must produce 100% deterministic ELF bytes"
    );
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
        3,
        5,
        6,
        7,
        9,
        10,
        11,
        12,
        13,
        14,
        15,
        17,
        19,
        25,
        31,
        63,
        100,
        127,
        255,
        1000,
        65535,
        0x3fff_ffff,
        0x4000_0001,
        0x7fff_ffff,
        0x8000_0001,
        0xffff_fffe,
        0xffff_ffff,
    ];
    for &d in u32_divs {
        let (m, sh, add) = super::lower::magic_u(32, u64::from(d));
        let mut samples = vec![
            0u32,
            1,
            2,
            3,
            d - 1,
            d,
            d.saturating_add(1),
            u32::MAX - 1,
            u32::MAX,
        ];
        samples.extend([
            7,
            10,
            99,
            1023,
            0x7fff_ffff,
            0x8000_0000,
            0x8000_0001,
            0xdead_beef,
        ]);
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
        3,
        5,
        6,
        7,
        9,
        10,
        11,
        13,
        15,
        100,
        1000,
        0x3fff_ffff,
        0x7fff_ffff,
        0x8000_0001,
        0xffff_ffff,
        (1u64 << 63) + 1,
        u64::MAX - 1,
        u64::MAX,
    ];
    for &d in u64_divs {
        let (m, sh, add) = super::lower::magic_u(64, d);
        let samples = [
            0u64,
            1,
            2,
            d - 1,
            d,
            d.saturating_add(1),
            0x7fff_ffff_ffff_ffff,
            0x8000_0000_0000_0000,
            0x8000_0000_0000_0001,
            0x1234_5678_9abc_def0,
            u64::MAX - 1,
            u64::MAX,
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
        3,
        -3,
        5,
        -5,
        6,
        -6,
        7,
        -7,
        9,
        -9,
        10,
        -10,
        12,
        -12,
        25,
        -25,
        100,
        -100,
        0x3fff_ffff,
        -0x3fff_ffff,
        i32::MAX,
        -i32::MAX,
        i32::MIN + 1,
    ];
    for &d in i32_divs {
        let (m, sh, add_sub) = super::lower::magic_s(32, i64::from(d));
        let samples = [
            0i32,
            1,
            -1,
            2,
            -2,
            d.wrapping_sub(1),
            d,
            d.wrapping_add(1),
            d.wrapping_neg().wrapping_sub(1),
            d.wrapping_neg(),
            d.wrapping_neg().wrapping_add(1),
            12345,
            -12345,
            i32::MIN,
            i32::MIN + 1,
            i32::MAX - 1,
            i32::MAX,
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
        3,
        -3,
        5,
        -5,
        7,
        -7,
        10,
        -10,
        100,
        -100,
        1000,
        -1000,
        0x7fff_ffff,
        -0x7fff_ffff,
        i64::MAX,
        -i64::MAX,
        i64::MIN + 1,
    ];
    for &d in i64_divs {
        let (m, sh, add_sub) = super::lower::magic_s(64, d);
        let samples = [
            0i64,
            1,
            -1,
            2,
            -2,
            d.wrapping_sub(1),
            d,
            d.wrapping_add(1),
            d.wrapping_neg(),
            123456789,
            -123456789,
            i64::MIN,
            i64::MIN + 1,
            i64::MAX - 1,
            i64::MAX,
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
    //   f1: medium_pure(x) = >144 pure adds/xors (internal, NOT inlined due to >144 insts, still elided stack + !fn_can_trap!)
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
    for k in 1..=160i32 {
        // Alternate add & xor so local folding doesn't collapse below the single-caller 144-inst cap.
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
        0x6a, 0x20, 0, 0x41, 5, 0x4a, 0x04, 0x7f, 0x20, 0, 0x41, 1, 0x6a, 0x05, 0x20, 0, 0x41, 1,
        0x6b, 0x0b, 0x6a, 0x20, 0, 0x10, 3, 0x6a, // call rec_fn
        0x20, 0, 0x41, 0, 0x11, 0, 0, 0x6a, // call_indirect tbl_fn
    ];
    let b3 = vec![
        0x20, 0, 0x41, 0, 0x4c, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 3, 0x41,
        1, 0x6a, 0x0b,
    ];
    let b4 = vec![0x20, 0, 0x41, 2, 0x6a];
    // f5(n): local i=0, sum=0; loop { br_if(i >=u n); sum += global.get g1 + memory.size; if (i > 0) { sum += load(n) + (100 / n); } i += 1; br loop; }
    let b5 = vec![
        0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, 0x20, 2, 0x23, 1, 0x3f, 0, 0x6a,
        0x6a, 0x21, 2, // sum += g1 + mem.size
        0x20, 1, 0x41, 0, 0x4a, 0x04, 0x40, 0x20, 2, 0x20, 0, 0x28, 2, 0, 0x41, 100, 0x20, 0, 0x6d,
        0x6a, 0x6a, 0x21, 2, 0x0b, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ];
    // f6(p): load(p) + load(p + 8)
    let b6 = vec![
        0x20, 0, 0x28, 2, 0, 0x20, 0, 0x41, 8, 0x6a, 0x28, 2, 0, 0x6a,
    ];

    let build_mod = |mem_sec: &[u8]| {
        wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0, 0, 0, 0, 0, 0, 0]),
            section(4, &[1, 0x70, 0x00, 1]), // funcref table len=1
            section(5, mem_sec),
            section(6, &glob_p),
            export_sec(&[
                ("entry_caller", 0, 2),
                ("licm_fn", 0, 5),
                ("wrap_bce", 0, 6),
            ]),
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
    let val = wtime_validate::validate(&wasm_1p, &x64_validate_features(), &x64_validate_limits())
        .unwrap();
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
        .filter_map(|i| {
            if let Op::Call { func, .. } = i.op {
                Some(func)
            } else {
                None
            }
        })
        .collect();
    assert!(
        !f2_calls.contains(&0),
        "small_leaf f0 must be inlined into f2"
    );
    assert!(
        f2_calls.contains(&1),
        "medium_pure f1 (>144 insts) must remain a direct call"
    );

    // 2. Diamond if/else in f2 converted to branchless Op::CmpSelect!
    assert!(
        irs[2]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i.op, Op::CmpSelect { .. }))),
        "trivial pure diamond in f2 must lower to Op::CmpSelect"
    );

    // 3. Step 8a + 8b invariants:
    assert!(
        !irs[0].needs_stack_check && !fn_can_trap[0],
        "internal pure leaf f0 elides stack + trap check"
    );
    assert!(
        !irs[1].needs_stack_check && !fn_can_trap[1],
        "internal pure medium f1 elides stack + trap check"
    );
    assert!(
        irs[2].needs_stack_check && fn_can_trap[2],
        "exported f2 MUST retain stack check"
    );
    assert!(
        irs[2].elided_callee_stack_bytes > 0,
        "retaining caller f2 MUST reserve elided callee f1 stack bytes"
    );
    assert!(
        irs[3].needs_stack_check && fn_can_trap[3],
        "recursive f3 MUST retain stack check"
    );
    assert!(
        irs[4].needs_stack_check && fn_can_trap[4],
        "call_indirect target f4 MUST retain stack check"
    );

    // 4. LICM legality in f5: pure GlobalGet(1) & MemSize hoisted into preheader bb0, while trapping Load/Div stay inside loop!
    assert!(
        irs[5].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::GlobalGet(1))),
        "loop-invariant GlobalGet(1) must hoist to preheader bb0"
    );
    assert!(
        irs[5].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::MemSize)),
        "loop-invariant MemSize must hoist to preheader bb0"
    );
    assert!(
        !irs[5].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::Load { .. } | Op::I32Bin(IntBinOp::DivS, _, _))),
        "trapping Load/Div inside loop must NEVER hoist to preheader bb0!"
    );

    // 5. Wrap-safe addressing-mode folding in bounded 1-page memory: f6's second load folds to (p, offset=8)!
    assert!(
        irs[6].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::Load { offset: 8, .. })),
        "bounded 1-page memory must fold load(p + 8) to offset=8"
    );

    // 6. Unbounded 4 GiB memory (`min=1, no max`): load(p + 8) MUST NOT fold (can wrap at 0xffff_fff8)!
    let wasm_4g = build_mod(&[1, 0x00, 1]);
    let val4 = wtime_validate::validate(&wasm_4g, &x64_validate_features(), &x64_validate_limits())
        .unwrap();
    let dec4 = val4.decoded();
    let mod4 = admit_x64_module(dec4).unwrap();
    let mut irs4: Vec<IrFunc> = (0..dec4.code.len())
        .map(|bi| lower_wasm_to_ir(dec4, &mod4, bi).unwrap())
        .collect();
    let eff4 = opt::compute_module_effects(&irs4, &mod4, None).unwrap();
    opt::optimize_func_stage2(&mut irs4[6], &mod4, &eff4, None).unwrap();
    verify_ir(&irs4[6], &mod4).unwrap();
    assert!(
        !irs4[6].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::Load { offset: 8, .. })),
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
    let val =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec = val.decoded();
    let module = admit_x64_module(dec).unwrap();
    let mut irs: Vec<IrFunc> = (0..dec.code.len())
        .map(|bi| lower_wasm_to_ir(dec, &module, bi).unwrap())
        .collect();
    opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    assert!(
        irs[1].br_tables.is_empty(),
        "callee with br_table must not be inlined"
    );
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
    let val2 =
        wtime_validate::validate(&wasm2, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec2 = val2.decoded();
    let mod2 = admit_x64_module(dec2).unwrap();
    let mut irs2: Vec<IrFunc> = (0..dec2.code.len())
        .map(|bi| lower_wasm_to_ir(dec2, &mod2, bi).unwrap())
        .collect();
    // Fill caller's val_pool to MAX_VAL_POOL_PER_FN so cloning callee's return val_range exceeds cap.
    irs2[1]
        .val_pool
        .resize(super::module::MAX_VAL_POOL_PER_FN, super::ir::Val(0));
    opt::inline_small_helpers(&mut irs2, &mod2, None).unwrap();
    verify_ir(&irs2[1], &mod2).unwrap();
    assert!(
        irs2[1].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::Call { func: 0, .. })),
        "rejected inline due to val_pool cap must leave original Op::Call untouched"
    );
}

#[test]
fn stage25_prune_block_params_const_fold_and_short_jump_relaxation() {
    use super::asm::RelocTarget;
    use super::opt;
    use super::regalloc::AllocConfig;

    // 1. Short-jump relaxation (`rel8` 2B vs `rel32` 5-6B) + exact reloc & jump-table offset shifting:
    let mut a = Asm::new();
    let l_short = a.new_label();
    let l_far = a.new_label();
    a.jmp(l_short);
    a.jcc(Cond::Ne, l_short);
    a.jmp(l_far);
    a.bind(l_short);
    // Emit a local rodata reloc AFTER a relaxed short jump to prove reloc.at shifts accurately.
    let loff = a.intern_f32(1.5f32.to_bits());
    let loff_dup = a.intern_f32(1.5f32.to_bits());
    assert_eq!(
        loff, loff_dup,
        "intern_f32 must deduplicate identical bit patterns"
    );
    a.movss_rip(Xmm::Xmm0, RelocTarget::LocalRodata(loff));
    let jt = a.add_jump_table(&[l_short, l_far]);
    a.lea_rip(Gp::Rax, RelocTarget::LocalRodata(jt));
    // Pad >130 bytes so `l_far` stays rel32 while `l_short` relaxes to rel8.
    for _ in 0..40 {
        a.add_ri(true, Gp::Rax, 1);
    }
    a.bind(l_far);
    a.ret();
    let pre_len = a.code.len();
    a.finish_labels().unwrap();
    // Relaxing `jmp l_short` (-3 B) + `jne l_short` (-4 B) must shrink code by exactly 7 bytes.
    assert_eq!(
        a.code.len(),
        pre_len - 7,
        "short-jump relaxation must shrink rel8 branches accurately"
    );
    assert_eq!(a.code[0], 0xeb, "first jmp must relax to short rel8 (0xeb)");
    assert_eq!(
        a.code[2], 0x75,
        "jcc Cond::Ne must relax to short rel8 (0x75)"
    );
    assert_eq!(a.code[4], 0xe9, "far jmp >127B must stay rel32 (0xe9)");
    assert_eq!(a.relocs.len(), 2);
    assert_eq!(
        a.relocs[0].patch_off,
        4 + 5 + 4,
        "movss_rip reloc patch_off must shift cleanly after rel8 relaxation"
    );
    assert_eq!(
        a.relocs[0].next_ip_off,
        4 + 5 + 8,
        "movss_rip reloc next_ip_off must shift cleanly after rel8 relaxation"
    );

    // 2. Dead loop-carried phi cycle pruning + post-inlining const32 folding:
    // f0: inlineable helper(x): `(x + 15) & -16`
    let mut b0 = vec![0x20, 0, 0x41, 15, 0x6a, 0x41];
    leb_i32(&mut b0, -16);
    b0.push(0x71);
    // f1(n): loop `i` from 0..n, modifying 20 loop-local temporaries (l2..l21) that are OVERWRITTEN
    // before being read in every iteration + calling f0(136) (must const-fold to 144!).
    let mut b1 = Vec::new();
    // l1 = i = 0; l2..l21 = dead/loop-local f32/i32 temporaries
    b1.extend_from_slice(&[0x41, 0, 0x21, 1]);
    b1.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]); // block; loop
    b1.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // br_if done (i >=u n)
    for lx in 2u32..22 {
        // lx = i + lx (overwritten every iter, read inside same iter, never loop-carried!)
        b1.push(0x20);
        b1.push(1);
        b1.push(0x41);
        leb_i32(&mut b1, lx as i32);
        b1.push(0x6a);
        b1.push(0x21);
        leb_u32(&mut b1, lx);
    }
    // Use l2 once inside loop, leave l3..l21 unused (dead phi cycles across loop if unpruned!)
    b1.extend_from_slice(&[0x20, 2, 0x1a]); // drop(l2)
    b1.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b]);
    // Return f0(136) + i
    b1.push(0x41);
    leb_i32(&mut b1, 136);
    b1.extend_from_slice(&[0x10, 0, 0x20, 1, 0x6a]);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        section(5, &[1, 0x01, 1, 1]), // fixed 1-page memory -> frees R13!
        export_sec(&[("test_fn", 0, 1)]),
        code_sec(&[enc_body(&[], &b0), enc_body(&[(21, 0x7f)], &b1)]),
    ]);
    let val =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec = val.decoded();
    let module = admit_x64_module(dec).unwrap();
    assert_eq!(module.fixed32, Some(65_536), "min==max==1 must set fixed32");
    assert_eq!(module.pinned_global, None, "no globals -> R12 freed");

    let mut irs: Vec<IrFunc> = (0..dec.code.len())
        .map(|bi| {
            let mut ir = lower_wasm_to_ir(dec, &module, bi).unwrap();
            opt::prune_early_block_params(&mut ir, None).unwrap();
            ir
        })
        .collect();
    opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    let eff = opt::compute_module_effects(&irs, &module, None).unwrap();
    opt::optimize_func_stage2(&mut irs[1], &module, &eff, None).unwrap();
    verify_ir(&irs[1], &module).unwrap();

    // Every single one of the 20 loop-local temporaries must have been pruned from the loop header!
    let max_params = irs[1]
        .blocks
        .iter()
        .skip(1)
        .map(|b| b.params.len())
        .max()
        .unwrap_or(0);
    assert!(
        max_params <= 2,
        "dead loop-carried local cycles must be pruned (got {max_params} params on loop/merge block)"
    );
    // Inlined `f0(136)` -> `(136 + 15) & -16` must constant-fold to `144` (either as `ConstI32(144)`
    // or further folded directly into `I32BinImm(Add, i, 144)`!).
    let has_144 = irs[1].blocks.iter().any(|b| {
        b.insts.iter().any(|i| {
            matches!(
                i.op,
                Op::ConstI32(144) | Op::I32BinImm(super::ir::IntBinOp::Add, _, 144)
            )
        })
    });
    assert!(
        has_144,
        "post-inlining const32 folding must fold f0(136) -> 144 (ConstI32 or I32BinImm)"
    );

    // Dynamic R12 + R13 allocation verifier check:
    let cfg = AllocConfig {
        allow_r12: true,
        allow_r13: true,
        has_bmi2: false,
    };
    let alloc = super::regalloc::allocate_with_config(&irs[1], cfg, None).unwrap();
    super::verify::verify_alloc_with_config(&irs[1], &alloc, cfg).unwrap();
}

#[test]
fn stage3_bmi2_encodings_zero_byte_jmp_and_live_range_holes() {
    use super::asm::RelocTarget;
    use super::ir::{
        Block, BlockId, CmpBranchKind, CmpOp, Inst, IntBinOp, IrFunc, Op, Term, Ty, Val, ValRange,
    };
    use super::regalloc::{AllocConfig, Loc, allocate_with_config};
    use super::verify::verify_alloc_with_config;

    // 1. BMI2 VEX3 encodings verified byte-for-byte against Intel SDM / objdump.
    let mut a = Asm::new();
    a.shlx(false, Gp::Rax, Gp::Rdi, Gp::Rsi);
    assert_eq!(a.code, [0xc4, 0xe2, 0x49, 0xf7, 0xc7]);

    let mut a = Asm::new();
    a.shrx(true, Gp::R8, Gp::R9, Gp::R10);
    assert_eq!(a.code, [0xc4, 0x42, 0xab, 0xf7, 0xc1]);

    let mut a = Asm::new();
    a.sarx(true, Gp::R11, Gp::Rax, Gp::Rcx);
    assert_eq!(a.code, [0xc4, 0x62, 0xf2, 0xf7, 0xd8]);

    let mut a = Asm::new();
    a.rorx(false, Gp::Rdx, Gp::R8, 7);
    assert_eq!(a.code, [0xc4, 0xc3, 0x7b, 0xf0, 0xd0, 0x07]);

    // 2. 0-byte fallthrough `jmp` elimination in `finish_labels()`, shifting labels & relocs accurately.
    let mut a = Asm::new();
    let l_skip = a.new_label();
    let l_next = a.new_label();
    a.jcc(Cond::E, l_skip);
    a.jmp(l_next); // lands directly on l_next -> 5 B -> 0 B!
    a.bind(l_next);
    a.call_reloc(RelocTarget::Func(3));
    a.bind(l_skip);
    a.ret();
    a.finish_labels().unwrap();
    // Expect: 2-byte `je +5` (0x74, 0x05), 5-byte `call rel32`, 1-byte `ret` -> 8 bytes total!
    assert_eq!(a.code.len(), 8);
    assert_eq!(&a.code[0..2], &[0x74, 0x05]);
    assert_eq!(a.code[2], 0xe8);
    assert_eq!(a.code[7], 0xc3);
    assert_eq!(a.relocs.len(), 1);
    assert_eq!(a.relocs[0].patch_off, 3);
    assert_eq!(a.relocs[0].next_ip_off, 7);

    // 3. Exact per-block live-range holes: loop header param `acc` used in latch AND in `exit`
    //    (ordered AFTER latch in RPO!). `acc_next` (computed in latch after `acc`'s last use) must
    //    coalesce into `acc`'s exact register despite `exit` extending `acc`'s hull past latch!
    let mut ir = IrFunc {
        func_index: 0,
        params: vec![Val(0), Val(1)],
        result_tys: vec![Ty::I32],
        val_tys: vec![Ty::I32; 7],
        val_pool: vec![
            Val(1), // 0: init i = param1
            Val(0), // 1: init acc = param0
            Val(5), // 2: next i = i_next
            Val(4), // 3: next acc = acc_next
            Val(3), // 4: return acc
        ],
        v128_consts: Vec::new(),
        br_tables: Vec::new(),
        blocks: vec![
            // bb0: Jump bb1(v1, v0)
            Block {
                params: vec![Val(0), Val(1)],
                insts: vec![],
                term: Term::Jump {
                    target: BlockId(1),
                    args: ValRange { start: 0, len: 2 },
                },
                is_loop_header: false,
                preds: 0,
            },
            // bb1 (loop header): params (v2=i, v3=acc); if i <u param0 goto bb2 else goto bb3
            Block {
                params: vec![Val(2), Val(3)],
                insts: vec![],
                term: Term::CmpBranch {
                    kind: CmpBranchKind::I32(CmpOp::LtU),
                    a: Val(2),
                    b: Val(0),
                    then_bb: BlockId(2),
                    then_args: ValRange::EMPTY,
                    else_bb: BlockId(3),
                    else_args: ValRange::EMPTY,
                },
                is_loop_header: true,
                preds: 2,
            },
            // bb2 (latch): v4 = acc + i; v5 = i + 1; v6 = v4 ^ 7; Jump bb1(v5, v4)
            // Note: v3 dies at inst 0, v4 is live from inst 0 to term; v3 is live again in bb3!
            Block {
                params: vec![],
                insts: vec![
                    Inst {
                        res: Some(Val(4)),
                        op: Op::I32Bin(IntBinOp::Add, Val(3), Val(2)),
                    },
                    Inst {
                        res: Some(Val(5)),
                        op: Op::I32BinImm(IntBinOp::Add, Val(2), 1),
                    },
                    Inst {
                        res: Some(Val(6)),
                        op: Op::I32BinImm(IntBinOp::Xor, Val(4), 7),
                    },
                ],
                term: Term::Jump {
                    target: BlockId(1),
                    args: ValRange { start: 2, len: 2 },
                },
                is_loop_header: false,
                preds: 1,
            },
            // bb3 (exit, ordered AFTER bb2 in RPO): return v3 (acc)
            Block {
                params: vec![],
                insts: vec![],
                term: Term::Return {
                    vals: ValRange { start: 4, len: 1 },
                },
                is_loop_header: false,
                preds: 1,
            },
        ],
        needs_stack_check: false,
        elided_callee_stack_bytes: 0,
        memory64: false,
        early_pruned: 0,
    };
    let alloc = allocate_with_config(&ir, AllocConfig::default(), None).unwrap();
    verify_alloc_with_config(&ir, &alloc, AllocConfig::default()).unwrap();
    assert_eq!(
        alloc.locs[3], alloc.locs[4],
        "live-range holes must let latch update v4 coalesce into loop header param v3"
    );

    // 4. BMI2 `has_bmi2 = true` allows a value to stay in RCX across `Shl`, while `has_bmi2 = false` rejects it!
    ir.blocks[2].insts[0].op = Op::I32Bin(IntBinOp::Shl, Val(3), Val(2));
    let mut forced = allocate_with_config(
        &ir,
        AllocConfig {
            allow_r12: false,
            allow_r13: false,
            has_bmi2: true,
        },
        None,
    )
    .unwrap();
    // Place loop-invariant `v0` (live across `Shl` in bb2!) explicitly in RCX if not already there.
    let v0_orig = forced.locs[0];
    let rcx_owner = forced.locs.iter().position(|&l| l == Loc::Gp(Gp::Rcx));
    if let Some(oi) = rcx_owner {
        forced.locs[oi] = v0_orig;
    }
    forced.locs[0] = Loc::Gp(Gp::Rcx);
    verify_alloc_with_config(
        &ir,
        &forced,
        AllocConfig {
            allow_r12: false,
            allow_r13: false,
            has_bmi2: true,
        },
    )
    .expect("BMI2 mode must allow RCX live across Shl");
    assert!(
        verify_alloc_with_config(
            &ir,
            &forced,
            AllocConfig {
                allow_r12: false,
                allow_r13: false,
                has_bmi2: false,
            },
        )
        .is_err(),
        "non-BMI2 mode must reject RCX live across Shl"
    );
}

#[test]
fn stage3_undo_log_if_nesting_and_inc_cache_parity() {
    // 1. Deeply nested `if`s (40 levels) with 500 locals mutating only 1 local per level:
    //    old full-locals snapshot cloning would exceed caps or blow up; undo-log handles it cleanly.
    let mut deep_body = Vec::new();
    for depth in 0..40u32 {
        deep_body.extend_from_slice(&[0x20, 0, 0x41]);
        leb_i32(&mut deep_body, depth as i32);
        deep_body.extend_from_slice(&[0x4a, 0x04, 0x40]); // if (arg > depth)
        deep_body.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1]); // local 1 += 1
    }
    deep_body.extend(std::iter::repeat_n(0x0b, 40)); // end
    deep_body.extend_from_slice(&[0x20, 1]);
    let deep_wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        export_sec(&[("deep_if", 0, 0)]),
        code_sec(&[enc_body(&[(500, 0x7f)], &deep_body)]),
    ]);
    let deep_so = compile_wasm_to_x64_so(&deep_wasm, &X64CompileOptions::default()).unwrap();
    assert!(!deep_so.is_empty());

    // 2. Per-function incremental cache bit-for-bit parity across leaf edit, inlineable callee edit,
    //    effect cascade, trap/elision cascade, single-caller flip, and corrupted cache recovery.
    let root = crate::build::create_fresh_test_root("wtime-aot-x64-inc-parity");
    let build_variant = |leaf_body: &[u8], caller_body: &[u8], helper_body: &[u8]| -> Vec<u8> {
        wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0, 0, 0, 0]),
            section(5, &[1, 0x01, 1, 2]),
            section(6, &[1, 0x7f, 1, 0x41, 10, 0x0b]), // mut i32 global 0
            export_sec(&[("entry", 0, 2), ("other", 0, 3)]),
            code_sec(&[
                enc_body(&[], leaf_body),                 // f0: internal leaf
                enc_body(&[], helper_body),               // f1: internal helper
                enc_body(&[], caller_body),               // f2: exported entry calling f0 & f1
                enc_body(&[], &[0x20, 0, 0x41, 9, 0x6a]), // f3: untouched leaf
            ]),
        ])
    };
    let make_large_leaf = |k: i32, write_g: bool, add_trap: bool| -> Vec<u8> {
        let mut b = vec![0x20, 0];
        if write_g {
            b.extend_from_slice(&[0x20, 0, 0x24, 0]);
        }
        if add_trap {
            b.extend_from_slice(&[0x41, 100, 0x20, 0, 0x6e, 0x6a]); // div_u(100, x) -> traps!
        }
        for i in 1..=120i32 {
            b.push(0x41);
            leb_i32(&mut b, i + k);
            b.push(if i % 2 == 0 { 0x6a } else { 0x73 });
        }
        b
    };
    let helper_v1 = vec![0x20, 0, 0x41, 3, 0x6c, 0x41, 1, 0x6a];
    let helper_v2 = vec![0x20, 0, 0x41, 5, 0x6c, 0x41, 2, 0x6a];
    let caller_1x = vec![
        0x23, 0, 0x20, 0, 0x10, 0, 0x20, 0, 0x10, 1, 0x6a, 0x6a, 0x23, 0, 0x6a,
    ];
    let caller_2x = vec![
        0x23, 0, 0x20, 0, 0x10, 0, 0x20, 0, 0x10, 1, 0x6a, 0x20, 0, 0x10, 1, 0x6a, 0x6a, 0x23, 0,
        0x6a,
    ];

    let variants = [
        (
            "v0_base",
            build_variant(&make_large_leaf(0, false, false), &caller_1x, &helper_v1),
        ),
        (
            "v1_leaf_edit",
            build_variant(&make_large_leaf(1, false, false), &caller_1x, &helper_v1),
        ),
        (
            "v2_inline_callee_edit",
            build_variant(&make_large_leaf(1, false, false), &caller_1x, &helper_v2),
        ),
        (
            "v3_effect_cascade",
            build_variant(&make_large_leaf(1, true, false), &caller_1x, &helper_v2),
        ),
        (
            "v4_trap_elide_cascade",
            build_variant(&make_large_leaf(1, true, true), &caller_1x, &helper_v2),
        ),
        (
            "v5_single_caller_flip",
            build_variant(&make_large_leaf(1, true, true), &caller_2x, &helper_v2),
        ),
    ];
    let salt = [0x42u8; 32];
    let inc_opts = X64CompileOptions {
        isa: TargetIsa {
            has_abm_bmi1: true,
            has_bmi2: true,
            has_avx2: true,
        },
        jobs: 2,
        opt: true,
        verify: true,
        inc_cache_dir: Some(root.0.clone()),
        cache_key_salt: Some(salt),
        ..Default::default()
    };
    let cold_opts = X64CompileOptions {
        inc_cache_dir: None,
        ..inc_opts.clone()
    };

    for (tag, wasm) in &variants {
        let so_inc = compile_wasm_to_x64_so(wasm, &inc_opts).unwrap();
        let so_cold = compile_wasm_to_x64_so(wasm, &cold_opts).unwrap();
        assert_eq!(
            so_inc, so_cold,
            "incremental cache output diverged from cold build on {tag}"
        );
    }

    // Corrupt the on-disk incremental cache file and verify silent fallback produces identical bytes.
    let inc_dir = root.0.join("x64-inc");
    let mut corrupted = 0usize;
    for entry in std::fs::read_dir(&inc_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "bin") {
            let mut raw = std::fs::read(&p).unwrap();
            let mid = raw.len() / 2;
            raw[mid] ^= 0xff;
            std::fs::write(&p, &raw).unwrap();
            corrupted += 1;
        }
    }
    assert!(corrupted >= 1, "expected at least one x64-inc cache file");
    let so_recovered = compile_wasm_to_x64_so(&variants[5].1, &inc_opts).unwrap();
    let so_cold = compile_wasm_to_x64_so(&variants[5].1, &cold_opts).unwrap();
    assert_eq!(
        so_recovered, so_cold,
        "corrupted incremental cache must recover cleanly to exact cold bytes"
    );
}

#[test]
fn stage3_store_to_load_forwarding_guarded_licm_and_triangle_if() {
    use super::ir::{IntBinOp, LoadKind, Op};
    use super::opt;

    // f0(p, v, w): store i32 [p+0]=v; store i32 [p+4]=w; load i32 [p+0] -> forwarded (0 loads left!)
    let b0 = vec![
        0x20, 0, 0x20, 1, 0x36, 2, 0, // store [p+0] = v
        0x20, 0, 0x20, 2, 0x36, 2, 4, // store [p+4] = w (disjoint!)
        0x20, 0, 0x28, 2, 0, // load [p+0] -> forwarded to v!
    ];
    // f1(p, v, w): store i32 [p+0]=v; store i32 [p+2]=w; load i32 [p+0] -> overlaps! load MUST remain!
    let b1 = vec![
        0x20, 0, 0x20, 1, 0x36, 2, 0, 0x20, 0, 0x20, 2, 0x36, 2, 2, 0x20, 0, 0x28, 2, 0,
    ];
    // f2(d): local i=0, sum=0; loop { br_if(i >=u d); sum += 100u / d; i += 1; br loop; } -> guarded LICM hoists DivU!
    let b2 = vec![
        0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, // br_if(i >=u d) exit
        0x20, 2, 0x41, 0xe4, 0x00, 0x20, 0, 0x6e, 0x6a, 0x21, 2, // sum += 100u / d
        0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20, 2,
    ];
    // f3(x): local y = 0; if x != 0 { y = x - 1 }; return y -> triangle if-conversion to CmpSelect!
    let b3 = vec![
        0x20, 0, 0x41, 0, 0x47, 0x04, 0x40, 0x20, 0, 0x41, 1, 0x6b, 0x21, 1, 0x0b, 0x20, 1,
    ];
    // f4(dst, src): load [src+0] + load [src+12] (proves src+16 checked), then memory.copy(dst, src, 16) -> src_checked == true!
    let b4 = vec![
        0x20, 1, 0x28, 2, 0, 0x1a, 0x20, 1, 0x28, 2, 12, 0x1a, 0x20, 0, 0x20, 1, 0x41, 16, 0xfc,
        0x0a, 0x00, 0x00, 0x41, 0,
    ];

    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f, 0x7f, 0x7f], &[0x7f]),
            (&[0x7f], &[0x7f]),
            (&[0x7f, 0x7f], &[0x7f]),
        ]),
        func_sec(&[0, 0, 1, 1, 2]),
        section(5, &[1, 0x01, 1, 4]),
        code_sec(&[
            enc_body(&[], &b0),
            enc_body(&[], &b1),
            enc_body(&[(2, 0x7f)], &b2),
            enc_body(&[(1, 0x7f)], &b3),
            enc_body(&[], &b4),
        ]),
    ]);
    let val =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let dec = val.decoded();
    let module = admit_x64_module(dec).unwrap();
    let mut irs: Vec<IrFunc> = (0..dec.code.len())
        .map(|bi| lower_wasm_to_ir(dec, &module, bi).unwrap())
        .collect();
    let effects = opt::compute_module_effects(&irs, &module, None).unwrap();
    for ir in &mut irs {
        opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
        verify_ir(ir, &module).unwrap();
    }

    assert!(
        !irs[0]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i.op, Op::Load { .. }))),
        "disjoint store [p+4] must not kill store-to-load forwarding of [p+0]"
    );
    assert!(
        irs[1]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(
                i.op,
                Op::Load {
                    kind: LoadKind::I32,
                    ..
                }
            ))),
        "overlapping store [p+2] MUST kill forwarding of [p+0]"
    );
    // In f2, `DivU` must be hoisted out of the loop body into a non-loop guarded preheader!
    let loop_hdrs: Vec<usize> = irs[2]
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(bi, b)| b.is_loop_header.then_some(bi))
        .collect();
    assert!(!loop_hdrs.is_empty());
    let div_in_non_loop = irs[2].blocks.iter().any(|b| {
        !b.is_loop_header
            && b.preds == 1
            && b.insts
                .iter()
                .any(|i| matches!(i.op, Op::I32Bin(IntBinOp::DivU | IntBinOp::DivUNz, _, _)))
    });
    assert!(
        div_in_non_loop,
        "guarded LICM must hoist divisor-guarded DivU into a non-loop preheader"
    );
    assert!(
        irs[3]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i.op, Op::CmpSelect { .. }))),
        "triangle if without else must convert to Op::CmpSelect"
    );
    assert!(
        irs[4]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(
                i.op,
                Op::MemCopyConst {
                    len: 16,
                    src_checked: true,
                    dst_checked: false,
                    ..
                }
            ))),
        "MemCopyConst BCE must elide dominated src bounds check while retaining unchecked dst"
    );
}

#[test]
fn stage3_council_round1_regressions() {
    use super::asm::Asm;
    use super::build_shared_rodata;
    use super::ir::{IntBinOp, LoadKind, Op};
    use super::lower::lower_ir_to_x64;
    use super::opt;

    // 1. Self-loop (`L: jmp L`) and empty mutual jump cycle (`L1: jmp L2; L2: jmp L1`) must
    //    NEVER be erased by 0-byte fallthrough `jmp` elimination.
    {
        let mut a = Asm::new();
        let l = a.new_label();
        a.bind(l);
        a.jmp(l);
        a.finish_labels().unwrap();
        assert_eq!(a.code, vec![0xeb, 0xfe], "self-loop jmp must remain eb fe");

        let mut b = Asm::new();
        let l1 = b.new_label();
        let l2 = b.new_label();
        b.bind(l1);
        b.jmp(l2);
        b.bind(l2);
        b.jmp(l1);
        b.finish_labels().unwrap();
        assert_eq!(
            b.code,
            vec![0xeb, 0xfe],
            "empty mutual jump cycle must collapse to eb fe, not empty fallthrough"
        );

        // End-to-end Wasm `(if (local.get 0) (then (loop $l (br $l)))) (i32.const 42)`:
        let spin_body = vec![
            0x20, 0, 0x04, 0x40, 0x03, 0x40, 0x0c, 0, 0x0b, 0x0b, 0x41, 42,
        ];
        let spin_wasm = wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0]),
            export_sec(&[("spin", 0, 0)]),
            code_sec(&[enc_body(&[], &spin_body)]),
        ]);
        let val =
            wtime_validate::validate(&spin_wasm, &x64_validate_features(), &x64_validate_limits())
                .unwrap();
        let dec = val.decoded();
        let module = admit_x64_module(dec).unwrap();
        let layout = CtxLayout::compute(&module, Vec::new()).unwrap();
        let budget = CompileBudget::new(None);
        let shared = build_shared_rodata(dec, &module, &layout.global_offs, &budget).unwrap();
        let mut ir = lower_wasm_to_ir(dec, &module, 0).unwrap();
        let effects =
            opt::compute_module_effects(std::slice::from_ref(&ir), &module, None).unwrap();
        opt::optimize_func_stage2(&mut ir, &module, &effects, None).unwrap();
        verify_ir(&ir, &module).unwrap();
        let alloc = allocate(&ir).unwrap();
        verify_alloc(&ir, &alloc).unwrap();
        let asm =
            lower_ir_to_x64(&ir, &alloc, &layout, &shared.consts, TargetIsa::default()).unwrap();
        assert!(
            asm.code.windows(2).any(|w| w == [0xeb, 0xfe]),
            "emitted spin loop must preserve eb fe self-jump: {:02x?}",
            asm.code
        );
    }

    // 2. `i64.store32 -> i32.load` must NOT forward `Ty::I64` as `Ty::I32`, and
    //    guarded-preheader LICM must preserve SSA dominance when `exit_bb` uses `hdr.params`
    //    and never panic on nested loops.
    {
        // f0(addr: i32, v64: i64) -> i32: i64.store32 [addr], v64; i32.load [addr]
        let f0 = vec![0x20, 0, 0x20, 1, 0x3e, 2, 0, 0x20, 0, 0x28, 2, 0];
        // f1(d: i32) -> i32: x = d*7+3; i = 0x1234; while i <u d { x += 1000u / d; i += 1 }; return i*1000 + x
        let mut f1 = vec![0x20, 0, 0x41, 7, 0x6c, 0x41, 3, 0x6a, 0x21, 2];
        f1.push(0x41);
        leb_i32(&mut f1, 0x1234);
        f1.extend_from_slice(&[0x21, 1, 0x02, 0x40, 0x03, 0x40]);
        f1.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]); // br_if(i >=u d) exit
        f1.extend_from_slice(&[0x20, 2, 0x41]);
        leb_i32(&mut f1, 1000);
        f1.extend_from_slice(&[0x20, 0, 0x6e, 0x6a, 0x21, 2]);
        f1.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b]);
        f1.extend_from_slice(&[0x20, 1, 0x41]);
        leb_i32(&mut f1, 1000);
        f1.extend_from_slice(&[0x6c, 0x20, 2, 0x6a]);
        // f2(d1: i32, d2: i32) -> i32: nested unsigned-guarded loops where inner hoists `v1 = 10000u /u d1`
        // and outer computes `v1 /u d2` inside the inner loop (triggers stale `dom` if unchecked!).
        let mut f2 = Vec::new();
        // local 2 = i2, local 3 = i1, local 4 = acc
        f2.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]); // outer block/loop
        f2.extend_from_slice(&[0x20, 2, 0x20, 1, 0x4f, 0x0d, 1]); // if i2 >=u d2 break outer
        f2.extend_from_slice(&[0x41, 0, 0x21, 3]); // i1 = 0
        f2.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]); // inner block/loop
        f2.extend_from_slice(&[0x20, 3, 0x20, 0, 0x4f, 0x0d, 1]); // if i1 >=u d1 break inner
        f2.extend_from_slice(&[0x20, 4, 0x41]);
        leb_i32(&mut f2, 10000);
        f2.extend_from_slice(&[0x20, 0, 0x6e, 0x20, 1, 0x6e, 0x6a, 0x21, 4]); // acc += (10000u / d1) / d2
        f2.extend_from_slice(&[0x20, 3, 0x41, 1, 0x6a, 0x21, 3, 0x0c, 0, 0x0b, 0x0b]); // end inner
        f2.extend_from_slice(&[0x20, 2, 0x41, 1, 0x6a, 0x21, 2, 0x0c, 0, 0x0b, 0x0b]); // end outer
        f2.extend_from_slice(&[0x20, 4]);

        let wasm = wasm_module(&[
            type_sec(&[
                (&[0x7f, 0x7e], &[0x7f]),
                (&[0x7f], &[0x7f]),
                (&[0x7f, 0x7f], &[0x7f]),
            ]),
            func_sec(&[0, 1, 2]),
            section(5, &[1, 0x01, 1, 2]),
            code_sec(&[
                enc_body(&[], &f0),
                enc_body(&[(2, 0x7f)], &f1),
                enc_body(&[(3, 0x7f)], &f2),
            ]),
        ]);
        let val = wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits())
            .unwrap();
        let dec = val.decoded();
        let module = admit_x64_module(dec).unwrap();
        let mut irs: Vec<IrFunc> = (0..dec.code.len())
            .map(|bi| lower_wasm_to_ir(dec, &module, bi).unwrap())
            .collect();
        let effects = opt::compute_module_effects(&irs, &module, None).unwrap();
        for ir in &mut irs {
            opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
            verify_ir(ir, &module).unwrap();
        }
        assert!(
            irs[0]
                .blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(
                    i.op,
                    Op::Load {
                        kind: LoadKind::I32,
                        ..
                    }
                ))),
            "i64.store32 must NOT forward I64 Val to i32.load"
        );
        assert!(
            irs[1].blocks.iter().any(|b| !b.is_loop_header
                && b.preds == 1
                && b.insts.iter().any(|i| {
                    matches!(i.op, Op::I32Bin(IntBinOp::DivU | IntBinOp::DivUNz, _, _))
                })),
            "guarded LICM must still hoist DivU when exit_bb promotes escaping hdr.params"
        );
    }

    // 3. Incremental cache parity on SCC cycle break + two-table `call_indirect` reorder +
    //    hardened handling of symlinked cache directories/files and crafted out-of-range indices.
    {
        let root = crate::build::create_fresh_test_root("wtime-aot-x64-inc-r1-reg");
        let opts_inc = X64CompileOptions {
            isa: TargetIsa {
                has_abm_bmi1: true,
                has_bmi2: true,
                has_avx2: true,
            },
            jobs: 2,
            opt: true,
            verify: true,
            inc_cache_dir: Some(root.0.clone()),
            cache_key_salt: Some([0x77u8; 32]),
            ..Default::default()
        };
        let opts_cold = X64CompileOptions {
            inc_cache_dir: None,
            ..opts_inc.clone()
        };

        // 3a. SCC cycle break: f0 (small) calls f1; f1 calls f0 in v1 (cycle!) and does not in v2 (breaks cycle!);
        //     f2 (exported caller) calls f0. Build v1 then v2 incrementally and assert exact cold parity.
        let build_cycle = |f1_calls_f0: bool| -> Vec<u8> {
            let f0 = vec![0x20, 0, 0x41, 1, 0x6a, 0x10, 1]; // f1(x + 1)
            let f1 = if f1_calls_f0 {
                // if x > 0 { f0(x - 1) } else { 7 }
                vec![
                    0x20, 0, 0x41, 0, 0x4a, 0x04, 0x7f, 0x20, 0, 0x41, 1, 0x6b, 0x10, 0, 0x05,
                    0x41, 7, 0x0b,
                ]
            } else {
                // x * 3 + 7 (no call to f0 -> breaks cycle!)
                vec![0x20, 0, 0x41, 3, 0x6c, 0x41, 7, 0x6a]
            };
            let f2 = vec![0x20, 0, 0x10, 0, 0x41, 5, 0x6a]; // f0(x) + 5
            let f3 = vec![0x20, 0, 0x41, 9, 0x73]; // untouched padding fn
            wasm_module(&[
                type_sec(&[(&[0x7f], &[0x7f])]),
                func_sec(&[0, 0, 0, 0]),
                export_sec(&[("entry", 0, 2), ("other", 0, 3)]),
                code_sec(&[
                    enc_body(&[], &f0),
                    enc_body(&[], &f1),
                    enc_body(&[], &f2),
                    enc_body(&[], &f3),
                ]),
            ])
        };
        let w_cyc = build_cycle(true);
        let w_brk = build_cycle(false);
        assert_eq!(
            compile_wasm_to_x64_so(&w_cyc, &opts_inc).unwrap(),
            compile_wasm_to_x64_so(&w_cyc, &opts_cold).unwrap(),
            "cycle v1 inc vs cold"
        );
        assert_eq!(
            compile_wasm_to_x64_so(&w_brk, &opts_inc).unwrap(),
            compile_wasm_to_x64_so(&w_brk, &opts_cold).unwrap(),
            "cycle-break v2 inc must match cold bit-for-bit"
        );

        // 3b. Two funcref tables with identical slots where encounter order of `(tbl, type)` swaps.
        let build_two_tables = |swap_order: bool| -> Vec<u8> {
            let (t_first, t_second) = if swap_order { (1u8, 0u8) } else { (0u8, 1u8) };
            let f0 = vec![0x20, 0, 0x41, 1, 0x6a];
            let f1 = vec![0x20, 0, 0x20, 0, 0x11, 0, t_first]; // call_indirect type 0 table t_first
            let f2 = vec![0x20, 0, 0x20, 0, 0x11, 0, t_second]; // call_indirect type 0 table t_second
            wasm_module(&[
                type_sec(&[(&[0x7f], &[0x7f])]),
                func_sec(&[0, 0, 0]),
                // Table section: 2 funcref tables of min=1, max=1
                section(4, &[2, 0x70, 0x01, 1, 1, 0x70, 0x01, 1, 1]),
                export_sec(&[("a", 0, 1), ("b", 0, 2)]),
                // Elem section: table 0 slot 0 = f0; table 1 slot 0 = f0
                section(
                    9,
                    &[
                        2, // 2 elem segments
                        0x00, 0x41, 0, 0x0b, 1, 0, // active tbl 0 offset 0 -> [f0]
                        0x02, 1, 0x41, 0, 0x0b, 0x00, 1, 0, // active tbl 1 offset 0 -> [f0]
                    ],
                ),
                code_sec(&[enc_body(&[], &f0), enc_body(&[], &f1), enc_body(&[], &f2)]),
            ])
        };
        let w_t1 = build_two_tables(false);
        let w_t2 = build_two_tables(true);
        assert_eq!(
            compile_wasm_to_x64_so(&w_t1, &opts_inc).unwrap(),
            compile_wasm_to_x64_so(&w_t1, &opts_cold).unwrap()
        );
        assert_eq!(
            compile_wasm_to_x64_so(&w_t2, &opts_inc).unwrap(),
            compile_wasm_to_x64_so(&w_t2, &opts_cold).unwrap(),
            "swapped two-table ci_index must not collide in abi_hash"
        );

        // 3c. Crafted cache file with valid Merkle trailer + out-of-range `eff_callees` and huge
        //     `n_relocs` must be rejected cleanly by `load_inc_cache` without panic or OOM.
        let crafted_root = crate::build::create_fresh_test_root("wtime-aot-x64-inc-crafted");
        let opts_crafted = X64CompileOptions {
            inc_cache_dir: Some(crafted_root.0.clone()),
            ..opts_inc.clone()
        };
        let expected_so = compile_wasm_to_x64_so(&w_brk, &opts_crafted).unwrap();
        let inc_dir = crafted_root.0.join("x64-inc");
        let bin_path = std::fs::read_dir(&inc_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "bin"))
            .expect("inc cache file");
        let orig = std::fs::read(&bin_path).unwrap();
        let sign_payload = |p: &[u8]| -> Vec<u8> {
            let mut v = p.to_vec();
            let mut root_h = crate::sha256::Sha256::new();
            root_h.update(b"WTX64INC-ROOT-1");
            root_h.update(&(p.len() as u64).to_le_bytes());
            root_h.update(&crate::sha256::digest(p));
            v.extend_from_slice(&root_h.finish());
            v
        };
        // Truncated payload with valid Merkle signature must fall back cleanly.
        std::fs::write(&bin_path, sign_payload(&orig[..orig.len() - 40])).unwrap();
        assert_eq!(
            compile_wasm_to_x64_so(&w_brk, &opts_crafted).unwrap(),
            expected_so,
            "truncated signed cache file must fall back cleanly"
        );

        // Crafted signed cache file with out-of-range `inlined_callees` index (9999) and bogus huge
        // `n_relocs` must return `None` from `load_inc_cache` without panicking or OOMing.
        {
            let mut entries = Vec::new();
            let mut offsets = vec![0u32];
            for fi in 0..4 {
                entries.extend_from_slice(&[0u8; 32]); // raw_body_hash
                entries.push(0); // flags
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_pre = 0
                if fi == 0 {
                    entries.extend_from_slice(&1u32.to_le_bytes()); // n_inlined = 1
                    entries.extend_from_slice(&9999u32.to_le_bytes()); // OOB callee!
                } else {
                    entries.extend_from_slice(&0u32.to_le_bytes()); // n_inlined = 0
                }
                entries.extend_from_slice(&[0u8; 160]); // padding >= min_fn_wire
                offsets.push(entries.len() as u32);
            }
            let mut crafted = orig[..53].to_vec();
            for off in offsets {
                crafted.extend_from_slice(&off.to_le_bytes());
            }
            crafted.extend_from_slice(&entries);
            let abi: [u8; 32] = crafted[12..44].try_into().unwrap();
            let zero_hashes = [[0u8; 32]; 4];
            std::fs::write(&bin_path, sign_payload(&crafted)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(
                    &crafted_root.0,
                    &abi,
                    0,
                    4,
                    0,
                    4096,
                    &zero_hashes
                )
                .is_none(),
                "OOB inlined_callees in signed cache file must be rejected"
            );
            // Also verify `save_inc_cache` + `compile_wasm_to_x64_so` recovers cleanly.
            assert_eq!(
                compile_wasm_to_x64_so(&w_brk, &opts_crafted).unwrap(),
                expected_so
            );
        }

        #[cfg(unix)]
        {
            // Replace cache file with a symlink to a decoy file and confirm loader ignores it safely.
            let decoy = crafted_root.0.join("decoy.bin");
            std::fs::write(&decoy, &orig).unwrap();
            std::fs::remove_file(&bin_path).unwrap();
            std::os::unix::fs::symlink(&decoy, &bin_path).unwrap();
            assert_eq!(
                compile_wasm_to_x64_so(&w_brk, &opts_crafted).unwrap(),
                expected_so,
                "symlinked inc cache file must be rejected safely"
            );
        }

        // 3d. Crafted cache file with `RelocTarget::LocalRodata(0)` when `ro_len == 0` must be
        //     rejected by `load_inc_cache` rather than causing a link-time failure.
        {
            let mut entries = Vec::new();
            let mut offsets = vec![0u32];
            for _ in 0..4 {
                entries.extend_from_slice(&[0u8; 32]); // raw_body_hash
                entries.push(0); // flags
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_pre
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_inlined
                entries.extend_from_slice(&0u32.to_le_bytes()); // inline_added_insts
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_eff
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_elide
                entries.extend_from_slice(&0u64.to_le_bytes()); // written_tables
                entries.extend_from_slice(&0i32.to_le_bytes()); // max_helper_bytes
                entries.extend_from_slice(&0i32.to_le_bytes()); // own_stack_bytes
                entries.extend_from_slice(&0i32.to_le_bytes()); // elided_callee_stack_bytes
                entries.extend_from_slice(&[0u8; 32]); // callee_effects_hash
                entries.extend_from_slice(&[0u8; 32]); // callee_trap_hash
                entries.extend_from_slice(&8u32.to_le_bytes()); // pre_relax_code_len = 8
                entries.extend_from_slice(&8u32.to_le_bytes()); // code_len = 8
                entries.extend_from_slice(&[0x90u8; 8]); // code
                entries.extend_from_slice(&1u32.to_le_bytes()); // n_relocs = 1
                entries.extend_from_slice(&0u32.to_le_bytes()); // patch_off = 0
                entries.extend_from_slice(&4u32.to_le_bytes()); // next_ip_off = 4
                entries.push(3u8); // RelocTarget::LocalRodata
                entries.extend_from_slice(&0u32.to_le_bytes()); // off = 0
                entries.extend_from_slice(&0u32.to_le_bytes()); // ro_len = 0 (invalid with LocalRodata(0)!)
                entries.extend_from_slice(&0u32.to_le_bytes()); // n_jt = 0
                offsets.push(entries.len() as u32);
            }
            let mut crafted = orig[..53].to_vec();
            for off in offsets {
                crafted.extend_from_slice(&off.to_le_bytes());
            }
            crafted.extend_from_slice(&entries);
            let abi: [u8; 32] = crafted[12..44].try_into().unwrap();
            let zero_hashes = [[0u8; 32]; 4];
            let _ = std::fs::remove_file(&bin_path);
            std::fs::write(&bin_path, sign_payload(&crafted)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(
                    &crafted_root.0,
                    &abi,
                    0,
                    4,
                    0,
                    4096,
                    &zero_hashes
                )
                .is_none(),
                "LocalRodata(0) with ro_len == 0 must be rejected"
            );
        }

        // 3e. Transitive post-compaction `was_inlinable` flip (`Outer -> Main -> Mid -> Leaf`):
        //     In v1, `Mid` is not inlinable and `Outer` inlines `Main` (leaving `call Mid`).
        //     In v2, `Mid` calls `Leaf` (which traps `unreachable`) before >16 dead `if` blocks,
        //     so `Mid` has >16 raw blocks (`is_inlinable_ir(raw) == false`) yet compacts to <=16
        //     blocks after inlining `Leaf` (`track.was_inlinable[Mid] == true`).
        //     Transitive promotion MUST dirty BOTH `Main` and `Outer` for exact cold parity!
        let build_transitive_flip = |mid_v2: bool| -> Vec<u8> {
            let leaf = vec![0x00]; // unreachable
            let mid = if !mid_v2 {
                // > 64 insts -> not inlinable in v1
                let mut m = vec![0x20, 0];
                for i in 1..=80i32 {
                    m.push(0x41);
                    leb_i32(&mut m, i);
                    m.push(0x6a);
                }
                m
            } else {
                // Calls `Leaf` (f0), followed by 9 `if (x) { x += 1 } else { x += 2 }` blocks
                // (> 16 raw blocks so `is_inlinable_ir(raw) == false`, <= 64 insts, all dead after inlining `Leaf`!).
                let mut m = vec![0x20, 0, 0x10, 0, 0x21, 0];
                for _ in 0..9 {
                    m.extend_from_slice(&[
                        0x20, 0, 0x04, 0x7f, 0x20, 0, 0x41, 1, 0x6a, 0x05, 0x20, 0, 0x41, 2, 0x6a,
                        0x0b, 0x21, 0,
                    ]);
                }
                m.extend_from_slice(&[0x20, 0]);
                m
            };
            let main_fn = vec![0x20, 0, 0x10, 1, 0x41, 1, 0x6a]; // Mid(x) + 1
            let outer_fn = vec![0x20, 0, 0x10, 2, 0x41, 2, 0x6a]; // Main(x) + 2
            let mut bodies = vec![
                enc_body(&[], &leaf),
                enc_body(&[], &mid),
                enc_body(&[], &main_fn),
                enc_body(&[], &outer_fn),
            ];
            let mut func_tys = vec![0u32; 10];
            for k in 0..6i32 {
                let pad = vec![0x20, 0, 0x41, (k + 1) as u8, 0x73];
                bodies.push(enc_body(&[], &pad));
                func_tys[4 + k as usize] = 0;
            }
            wasm_module(&[
                type_sec(&[(&[0x7f], &[0x7f])]),
                func_sec(&func_tys),
                export_sec(&[("outer", 0, 3), ("pad", 0, 4)]),
                code_sec(&bodies),
            ])
        };
        let tr_root = crate::build::create_fresh_test_root("wtime-aot-x64-inc-trans-flip");
        let opts_tr_inc = X64CompileOptions {
            inc_cache_dir: Some(tr_root.0.clone()),
            ..opts_inc.clone()
        };
        let w_tr1 = build_transitive_flip(false);
        let w_tr2 = build_transitive_flip(true);
        assert_eq!(
            compile_wasm_to_x64_so(&w_tr1, &opts_tr_inc).unwrap(),
            compile_wasm_to_x64_so(&w_tr1, &opts_cold).unwrap()
        );
        assert_eq!(
            compile_wasm_to_x64_so(&w_tr2, &opts_tr_inc).unwrap(),
            compile_wasm_to_x64_so(&w_tr2, &opts_cold).unwrap(),
            "transitive post-compaction was_inlinable flip must match cold build bit-for-bit"
        );

        // 3f. Sequential equal-size guarded-LICM loops (with bb0 parameter numerators so guarded
        //     LICM actually fires on loop 1 while loop 2's preheader collides with loop 1's exit!)
        //     must compile deterministically across runs.
        let mut seq_loops = Vec::new();
        // Params: 0 = num1, 1 = num2, 2 = n; Locals: 3 = i, 4 = acc, 5 = j
        // Loop 1: for i=0..n { acc += num1 /u n }
        seq_loops.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
        seq_loops.extend_from_slice(&[0x20, 3, 0x20, 2, 0x4f, 0x0d, 1]);
        seq_loops.extend_from_slice(&[0x20, 4, 0x20, 0, 0x20, 2, 0x6e, 0x6a, 0x21, 4]);
        seq_loops.extend_from_slice(&[0x20, 3, 0x41, 1, 0x6a, 0x21, 3, 0x0c, 0, 0x0b, 0x0b]);
        // Loop 2 (same block count!): for j=0..n { acc += num2 /u n }
        seq_loops.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
        seq_loops.extend_from_slice(&[0x20, 5, 0x20, 2, 0x4f, 0x0d, 1]);
        seq_loops.extend_from_slice(&[0x20, 4, 0x20, 1, 0x20, 2, 0x6e, 0x6a, 0x21, 4]);
        seq_loops.extend_from_slice(&[0x20, 5, 0x41, 1, 0x6a, 0x21, 5, 0x0c, 0, 0x0b, 0x0b]);
        seq_loops.extend_from_slice(&[0x20, 4]);
        let w_seq = wasm_module(&[
            type_sec(&[(&[0x7f, 0x7f, 0x7f], &[0x7f])]),
            func_sec(&[0]),
            export_sec(&[("seq", 0, 0)]),
            code_sec(&[enc_body(&[(3, 0x7f)], &seq_loops)]),
        ]);
        let ref_so = compile_wasm_to_x64_so(&w_seq, &opts_cold).unwrap();
        for _ in 0..8 {
            assert_eq!(
                compile_wasm_to_x64_so(&w_seq, &opts_cold).unwrap(),
                ref_so,
                "equal-size sequential guarded LICM loops must be 100% deterministic"
            );
        }

        // 3g. Hardening invariants:
        //     - `CfgDom::dominates` returns `false` on identical invalid/out-of-range IDs.
        //     - `sweep_stale_tmp_files` never removes non-staging files like `tmp.notes`.
        //     - High-alternation cross-block CFG triggering `MAX_SEGS_PER_VAL` fallback passes `verify_alloc`.
        {
            let val =
                wtime_validate::validate(&w_seq, &x64_validate_features(), &x64_validate_limits())
                    .unwrap();
            let dec = val.decoded();
            let module = admit_x64_module(dec).unwrap();
            let mut ir = lower_wasm_to_ir(dec, &module, 0).unwrap();
            let dom = opt::rebuild_cfg_and_prune(&mut ir, None).unwrap();
            assert!(
                !dom.dominates(9999, 9999),
                "dominates must return false for identical OOB block IDs"
            );

            // Exercise `sweep_stale_tmp_files_with_cutoff` directly on both a non-staging file
            // (`tmp.notes`, must survive) and a staging file (`tmp.<pid>.<seq>`, must be deleted).
            let x64_inc = tr_root.0.join("x64-inc");
            let notes = x64_inc.join("tmp.notes");
            let stale = x64_inc.join("tmp.999999.0");
            std::fs::write(&notes, b"keep").unwrap();
            std::fs::write(&stale, b"stale").unwrap();
            super::inc_cache::sweep_stale_tmp_files_with_cutoff(
                &x64_inc,
                std::time::Duration::ZERO,
            );
            assert!(
                notes.exists(),
                "sweep must not delete non-staging tmp.notes"
            );
            assert!(
                !stale.exists(),
                "sweep must delete stale staging tmp.<pid>.<seq> file"
            );
            let _ = std::fs::remove_file(&notes);

            // Build a synthetic CFG whose RPO is `[0, 1, 2, 3, ..., 2*N]` where `Val(0)` is live
            // only in even blocks (`0, 2, ..., 2*N`) and dead in every odd block (`1, 3, ..., 2*N-1`).
            // Odd block `1` keeps 9 non-rematerializable GP values (`Val(1)` + `t0..t7`) live
            // simultaneously (= all 9 default GP regs):
            // - At `N = 10` (11 disjoint segments <= `MAX_SEGS_PER_VAL = 64`), exact live-range
            //   holes let one of block `1`'s 9 values reuse `Val(0)`'s GP register.
            // - At `N = 80` (81 disjoint segments > `MAX_SEGS_PER_VAL = 64`), `Val(0)` triggers
            //   `cb_seg_overflow` and falls back to its conservative full hull, forcing `Val(0)`'s
            //   location to stay disjoint from all 9 simultaneously live values in block `1`.
            let build_alt_ir = |pairs: u32| -> (super::ir::IrFunc, Vec<super::ir::Val>) {
                use super::ir::{
                    Block, BlockId, Inst, IntBinOp, IrFunc, Op, Term, Ty, Val, ValRange,
                };
                let mut ir = IrFunc {
                    func_index: 0,
                    params: vec![Val(0), Val(1)],
                    result_tys: vec![Ty::I32],
                    val_tys: vec![Ty::I32, Ty::I32],
                    blocks: Vec::with_capacity((2 * pairs + 1) as usize),
                    val_pool: vec![Val(0), Val(1)],
                    v128_consts: Vec::new(),
                    br_tables: Vec::new(),
                    needs_stack_check: false,
                    elided_callee_stack_bytes: 0,
                    memory64: false,
                    early_pruned: 0,
                };
                let empty = ValRange { start: 0, len: 0 };
                let ret_v0 = ValRange { start: 0, len: 1 };
                let ret_v1 = ValRange { start: 1, len: 1 };
                let mut hole_live = vec![Val(1)];
                for i in 0..pairs {
                    ir.blocks.push(Block {
                        params: if i == 0 {
                            vec![Val(0), Val(1)]
                        } else {
                            Vec::new()
                        },
                        insts: Vec::new(),
                        term: Term::BranchIf {
                            cond: Val(1),
                            then_bb: BlockId(2 * i + 2),
                            then_args: empty,
                            else_bb: BlockId(2 * i + 1),
                            else_args: empty,
                        },
                        is_loop_header: false,
                        preds: if i == 0 { 0 } else { 1 },
                    });
                    if i == 0 {
                        let mut insts = Vec::with_capacity(16);
                        let mut prev = Val(1);
                        let mut prev2 = Val(1);
                        let mut temps = Vec::with_capacity(8);
                        for _ in 0..8 {
                            let t = ir.push_val(Ty::I32);
                            insts.push(Inst {
                                res: Some(t),
                                op: Op::I32Bin(IntBinOp::Add, prev2, prev),
                            });
                            temps.push(t);
                            hole_live.push(t);
                            prev2 = prev;
                            prev = t;
                        }
                        let mut acc = Val(1);
                        for &t in &temps {
                            let next = ir.push_val(Ty::I32);
                            insts.push(Inst {
                                res: Some(next),
                                op: Op::I32Bin(IntBinOp::Xor, acc, t),
                            });
                            acc = next;
                        }
                        let ret_acc = ir.push_pool(&[acc]).unwrap();
                        ir.blocks.push(Block {
                            params: Vec::new(),
                            insts,
                            term: Term::Return { vals: ret_acc },
                            is_loop_header: false,
                            preds: 1,
                        });
                    } else {
                        ir.blocks.push(Block {
                            params: Vec::new(),
                            insts: Vec::new(),
                            term: Term::Return { vals: ret_v1 },
                            is_loop_header: false,
                            preds: 1,
                        });
                    }
                }
                ir.blocks.push(Block {
                    params: Vec::new(),
                    insts: Vec::new(),
                    term: Term::Return { vals: ret_v0 },
                    is_loop_header: false,
                    preds: 1,
                });
                (ir, hole_live)
            };

            let (ir_holes, hole_live_small) = build_alt_ir(10);
            let alloc_holes = super::regalloc::allocate(&ir_holes).unwrap();
            super::verify::verify_alloc(&ir_holes, &alloc_holes).unwrap();
            assert!(
                hole_live_small
                    .iter()
                    .any(|&v| alloc_holes.locs[v.0 as usize] == alloc_holes.locs[0]),
                "with 11 <= 64 segments, exact live-range hole in block 1 should allow reusing Val(0)'s register"
            );

            let (ir_overflow, hole_live_big) = build_alt_ir(80);
            let alloc_overflow = super::regalloc::allocate(&ir_overflow).unwrap();
            super::verify::verify_alloc(&ir_overflow, &alloc_overflow).unwrap();
            assert!(
                hole_live_big
                    .iter()
                    .all(|&v| alloc_overflow.locs[v.0 as usize] != alloc_overflow.locs[0]),
                "with 81 > 64 segments, Val(0) must fall back to its conservative full hull across block 1"
            );
        }
    }
}

#[test]
fn stage4_hills_ir_loop_versioning_short_circuit_and_parallel_admit() {
    use super::ir::{Op, Term};

    // 1. `image_filter.wasm` fixture: verify `version_checked_counted_loops` emits checked fast
    //    loops with folded `+4096` offset, and both `verify_ir` and `verify_alloc` pass!
    let img_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/image_filter.wasm");
    let img_wasm = std::fs::read(&img_path).expect("read image_filter.wasm");
    let val = wtime_validate::validate(
        &img_wasm,
        &super::module::x64_validate_features(),
        &super::module::x64_validate_limits(),
    )
    .unwrap();
    let module = super::module::admit_x64_module(val.decoded()).unwrap();
    let mut ir = super::ir::lower_wasm_to_ir(val.decoded(), &module, 1).unwrap();
    let eff = super::opt::compute_module_effects(&[ir.clone()], &module, None).unwrap();
    super::opt::optimize_func_stage2(&mut ir, &module, &eff, None).unwrap();
    super::verify::verify_ir(&ir, &module).unwrap();
    let alloc = super::regalloc::allocate(&ir).unwrap();
    super::verify::verify_alloc(&ir, &alloc).unwrap();
    let has_folded_checked_load = ir.blocks.iter().any(|bb| {
        bb.insts.iter().any(|ins| {
            matches!(
                ins.op,
                Op::Load {
                    offset: 4096,
                    checked: true,
                    ..
                }
            )
        })
    });
    let has_folded_checked_store = ir.blocks.iter().any(|bb| {
        bb.insts.iter().any(|ins| {
            matches!(
                ins.op,
                Op::Store {
                    offset: 4096,
                    checked: true,
                    ..
                }
            )
        })
    });
    assert!(
        has_folded_checked_load && has_folded_checked_store,
        "image_filter must version both counted loops with checked=true and folded +4096 offset"
    );

    // 2. `graph_bfs.wasm` fixture: verify `split_short_circuit_branches` eliminated `Or` before
    //    `BranchIf` in `run` (after inlining `$edge`) and passes IR/alloc verification.
    let bfs_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/graph_bfs.wasm");
    let bfs_wasm = std::fs::read(&bfs_path).expect("read graph_bfs.wasm");
    let val_bfs = wtime_validate::validate(
        &bfs_wasm,
        &super::module::x64_validate_features(),
        &super::module::x64_validate_limits(),
    )
    .unwrap();
    let mod_bfs = super::module::admit_x64_module(val_bfs.decoded()).unwrap();
    let mut irs_bfs: Vec<_> = (0..val_bfs.decoded().code.len())
        .map(|bi| super::ir::lower_wasm_to_ir(val_bfs.decoded(), &mod_bfs, bi).unwrap())
        .collect();
    super::opt::inline_small_helpers(&mut irs_bfs, &mod_bfs, None).unwrap();
    let eff_bfs = super::opt::compute_module_effects(&irs_bfs, &mod_bfs, None).unwrap();
    super::opt::optimize_func_stage2(&mut irs_bfs[2], &mod_bfs, &eff_bfs, None).unwrap();
    super::verify::verify_ir(&irs_bfs[2], &mod_bfs).unwrap();
    let alloc_bfs = super::regalloc::allocate(&irs_bfs[2]).unwrap();
    super::verify::verify_alloc(&irs_bfs[2], &alloc_bfs).unwrap();
    let has_or_inst = irs_bfs[2].blocks.iter().any(|bb| {
        bb.insts
            .iter()
            .any(|ins| matches!(ins.op, Op::I32Bin(super::ir::IntBinOp::Or, _, _)))
    });
    assert!(
        !has_or_inst,
        "graph_bfs `Or(eq1, eq2)` branch must be split into short-circuit CmpBranch"
    );
    let cmp_branches = irs_bfs[2]
        .blocks
        .iter()
        .filter(|bb| matches!(bb.term, Term::CmpBranch { .. }))
        .count();
    assert!(
        cmp_branches >= 4,
        "expected short-circuit CmpBranch fusion in graph_bfs (got {cmp_branches})"
    );

    // 3. Parallel `admit_x64_module_with_budget` (>= 256 functions) + incremental cache parity on
    //    both unchanged-callees/effects edit AND changed-callees/effects edit.
    let build_big = |edit_kind: u8| -> Vec<u8> {
        let n_fns = 300usize;
        let mut func_types = vec![0u8; n_fns];
        func_types[0] = 0;
        let mut bodies = Vec::with_capacity(n_fns);
        for i in 0..n_fns {
            let body = if i == 0 {
                match edit_kind {
                    0 => vec![0x20, 0, 0x41, 1, 0x6a], // x + 1
                    1 => vec![0x20, 0, 0x41, 2, 0x6a], // x + 2 (same callees & effects -> fast path!)
                    _ => vec![0x20, 0, 0x41, 0, 0x20, 0, 0x36, 2, 0, 0x41, 3, 0x6a], // writes mem!
                }
            } else if i == 150 {
                // call_indirect via table 1
                vec![0x20, 0, 0x20, 0, 0x11, 0, 1]
            } else if i == 290 {
                // call_indirect via table 0
                vec![0x20, 0, 0x20, 0, 0x11, 0, 0]
            } else {
                // call f0(x) + i
                let mut b = vec![0x20, 0, 0x10, 0, 0x41];
                leb_i32(&mut b, (i & 31) as i32);
                b.push(0x6a);
                b
            };
            bodies.push(enc_body(&[], &body));
        }
        let mut fsec = Vec::new();
        leb_u32(&mut fsec, n_fns as u32);
        fsec.extend_from_slice(&func_types);
        let mut csec = Vec::new();
        leb_u32(&mut csec, n_fns as u32);
        for b in bodies {
            csec.extend_from_slice(&b);
        }
        wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            section(3, &fsec),
            section(4, &[2, 0x70, 0x01, 1, 1, 0x70, 0x01, 1, 1]),
            section(5, &[1, 0x01, 1, 1]),
            export_sec(&[("f0", 0, 0), ("f150", 0, 150), ("f290", 0, 290)]),
            section(
                9,
                &[
                    2, 0x00, 0x41, 0, 0x0b, 1, 0, 0x02, 1, 0x41, 0, 0x0b, 0x00, 1, 0,
                ],
            ),
            section(10, &csec),
        ])
    };
    let root = crate::build::create_fresh_test_root("wtime-aot-stage4-big");
    let opts_inc = X64CompileOptions {
        jobs: 4,
        opt: true,
        verify: true,
        inc_cache_dir: Some(root.0.clone()),
        cache_key_salt: Some([0x5au8; 32]),
        ..Default::default()
    };
    let opts_cold = X64CompileOptions {
        inc_cache_dir: None,
        ..opts_inc.clone()
    };
    for edit_kind in [0u8, 1, 2] {
        let w = build_big(edit_kind);
        let so_inc = compile_wasm_to_x64_so(&w, &opts_inc).unwrap();
        let so_cold = compile_wasm_to_x64_so(&w, &opts_cold).unwrap();
        assert_eq!(
            so_inc, so_cold,
            "300-fn parallel admit + inc cache parity failed on edit_kind={edit_kind}"
        );
    }
}

#[test]
fn stage5_wave2_rotation_alignment_memcopy_forward_and_inc_v6() {
    use super::asm::{Asm, Cond, Gp, RelocTarget};
    use super::ir::{Op, StoreKind, Term};
    use super::opt;

    // 1. `graph_bfs.wasm` + `image_filter.wasm`: verify loop rotation turns top-tested loops into
    //    bottom-tested loops (`is_loop_header` blocks contain actual loop work) and hot-fallthrough
    //    layout keeps loop latches contiguous while sinking cold side bodies.
    {
        let bfs_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/graph_bfs.wasm");
        let bfs_wasm = std::fs::read(&bfs_path).expect("read graph_bfs.wasm");
        let val = wtime_validate::validate(
            &bfs_wasm,
            &super::module::x64_validate_features(),
            &super::module::x64_validate_limits(),
        )
        .unwrap();
        let module = super::module::admit_x64_module(val.decoded()).unwrap();
        let mut irs: Vec<_> = (0..val.decoded().code.len())
            .map(|bi| super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap())
            .collect();
        opt::inline_small_helpers(&mut irs, &module, None).unwrap();
        let eff = opt::compute_module_effects(&irs, &module, None).unwrap();
        opt::optimize_func_stage2(&mut irs[2], &module, &eff, None).unwrap();
        verify_ir(&irs[2], &module).unwrap();
        let alloc = super::regalloc::allocate(&irs[2]).unwrap();
        verify_alloc(&irs[2], &alloc).unwrap();

        let inner_hdr_oi = alloc
            .block_order
            .iter()
            .position(|&b| {
                let bb = &irs[2].blocks[b as usize];
                bb.is_loop_header
                    && matches!(bb.term, Term::CmpBranch { .. })
                    && bb.insts.is_empty()
            })
            .expect("expected rotated inner loop header in graph_bfs");
        let next_b = alloc.block_order[inner_hdr_oi + 1];
        let next_has_store = irs[2].blocks[next_b as usize]
            .insts
            .iter()
            .any(|ins| matches!(ins.op, Op::Store { .. }));
        assert!(
            !next_has_store,
            "hot-fallthrough layout must sink store-heavy `hit` block out of line after inner loop latch"
        );
    }
    {
        let img_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/image_filter.wasm");
        let img_wasm = std::fs::read(&img_path).expect("read image_filter.wasm");
        let val = wtime_validate::validate(
            &img_wasm,
            &super::module::x64_validate_features(),
            &super::module::x64_validate_limits(),
        )
        .unwrap();
        let module = super::module::admit_x64_module(val.decoded()).unwrap();
        let mut irs: Vec<_> = (0..val.decoded().code.len())
            .map(|bi| super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap())
            .collect();
        opt::inline_small_helpers(&mut irs, &module, None).unwrap();
        let eff = opt::compute_module_effects(&irs, &module, None).unwrap();
        opt::optimize_func_stage2(&mut irs[1], &module, &eff, None).unwrap();
        verify_ir(&irs[1], &module).unwrap();
        let alloc = super::regalloc::allocate(&irs[1]).unwrap();
        verify_alloc(&irs[1], &alloc).unwrap();

        let fast_rotated_hdrs = irs[1]
            .blocks
            .iter()
            .filter(|bb| {
                bb.is_loop_header
                    && bb.insts.iter().any(|ins| {
                        matches!(
                            ins.op,
                            Op::Load { checked: true, .. } | Op::Store { checked: true, .. }
                        )
                    })
            })
            .count();
        assert!(
            fast_rotated_hdrs >= 2,
            "both fast versioned loops in image_filter must rotate (got {fast_rotated_hdrs})"
        );
    }

    // 2. `Asm::mark_align16`: verify exact 16-byte alignment, relocation offset preservation across
    //    NOP padding, and safe skipping when padding would overflow a crossing `rel8` branch.
    {
        let mut a = Asm::new();
        let lbl_loop = a.new_label();
        let lbl_exit = a.new_label();
        a.nop(); // 1 B -> call at 1..6 (`next_ip_off == 6`)
        a.call_reloc(RelocTarget::Func(0));
        let pre_reloc_patch = a.relocs[0].patch_off;
        let pre_reloc_next = a.relocs[0].next_ip_off;
        a.mark_align16(lbl_loop);
        a.bind(lbl_loop);
        a.call_reloc(RelocTarget::Func(1));
        let post_reloc_orig_patch = a.relocs[1].patch_off;
        a.cmp_ri(false, Gp::Rax, 0);
        a.jcc(Cond::E, lbl_exit);
        a.jmp(lbl_loop);
        a.bind(lbl_exit);
        a.ret();
        a.finish_labels().unwrap();

        assert_eq!(
            a.relocs[0].patch_off, pre_reloc_patch,
            "reloc ending right before aligned label must keep patch_off unshifted"
        );
        assert_eq!(
            a.relocs[0].next_ip_off, pre_reloc_next,
            "reloc ending right before aligned label must keep next_ip_off unshifted"
        );
        assert_eq!(
            a.relocs[1].patch_off,
            post_reloc_orig_patch + 10,
            "reloc starting at aligned label must shift forward by exact NOP pad (10 B)"
        );
        assert_eq!(
            (a.relocs[1].patch_off - 1) & 15,
            0,
            "aligned loop header start must sit on an exact 16-byte boundary"
        );
    }
    {
        // `rel8` boundary safety: forward `jcc` crosses a misaligned `align16` label with
        // displacement `+124` (after relaxation); inserting 13 B of NOPs would push it to `+137 > 127`,
        // so `finish_labels` must skip padding that label rather than overflowing `i8`.
        let mut a = Asm::new();
        let lbl_mid = a.new_label();
        let lbl_far = a.new_label();
        a.nop();
        a.jcc(Cond::E, lbl_far);
        a.mark_align16(lbl_mid);
        a.bind(lbl_mid);
        for _ in 0..124 {
            a.nop();
        }
        a.bind(lbl_far);
        a.ret();
        a.finish_labels().unwrap();
        assert_eq!(a.code[1], 0x74, "expected relaxed je rel8");
        assert_eq!(
            a.code[2] as i8, 124,
            "rel8 displacement must stay exact +124"
        );
    }

    // 3. `MemCopyConst` store-to-load forwarding + checked-store offset folding in counted loops.
    {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x02, 0x40, 0x03, 0x40]);
        body.extend_from_slice(&[0x20, 1, 0x20, 0, 0x4f, 0x0d, 1]);
        body.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x21, 2]);
        body.extend_from_slice(&[0x20, 2, 0x2b, 3, 0, 0x21, 3]);
        body.extend_from_slice(&[0x20, 2, 0x2b, 3, 8, 0x21, 4]);
        body.extend_from_slice(&[0x20, 2, 0x2b, 3, 16, 0x21, 5]);
        body.extend_from_slice(&[0x20, 2, 0x2b, 3, 24, 0x21, 6]);
        body.extend_from_slice(&[0x20, 2, 0x20, 3, 0x20, 5, 0xa0, 0x39, 3, 0]);
        body.extend_from_slice(&[0x20, 2, 0x20, 4, 0x20, 6, 0xa0, 0x39, 3, 8]);
        body.extend_from_slice(&[0x20, 2, 0x41]);
        leb_i32(&mut body, 4096);
        body.extend_from_slice(&[0x6a, 0x20, 2, 0x41, 32, 0xfc, 0x0a, 0, 0]);
        body.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b]);
        let wasm = wasm_module(&[
            type_sec(&[(&[0x7f], &[])]),
            func_sec(&[0]),
            section(5, &[1, 0x01, 2, 2]),
            export_sec(&[("f", 0, 0)]),
            code_sec(&[enc_body(&[(2, 0x7f), (4, 0x7c)], &body)]),
        ]);
        let val = wtime_validate::validate(
            &wasm,
            &super::module::x64_validate_features(),
            &super::module::x64_validate_limits(),
        )
        .unwrap();
        let module = super::module::admit_x64_module(val.decoded()).unwrap();
        let mut ir = super::ir::lower_wasm_to_ir(val.decoded(), &module, 0).unwrap();
        let eff = opt::compute_module_effects(std::slice::from_ref(&ir), &module, None).unwrap();
        opt::optimize_func_stage2(&mut ir, &module, &eff, None).unwrap();
        verify_ir(&ir, &module).unwrap();

        let has_memcopy = ir.blocks.iter().any(|b| {
            b.insts
                .iter()
                .any(|ins| matches!(ins.op, Op::MemCopyConst { .. }))
        });
        assert!(
            !has_memcopy,
            "fully register-resident 32B MemCopyConst must be forwarded to direct checked f64 stores"
        );
        let folded_dest_bytes: usize = ir
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .map(|ins| match ins.op {
                Op::Store {
                    kind: StoreKind::F64,
                    offset: 4096 | 4104 | 4112 | 4120,
                    checked: true,
                    ..
                } => 8,
                Op::Store {
                    kind: StoreKind::V128,
                    offset: 4096 | 4112,
                    checked: true,
                    ..
                } => 16,
                _ => 0,
            })
            .sum();
        assert!(
            folded_dest_bytes >= 32,
            "forwarded f64/v128 stores must fold +4096 base offset (got {folded_dest_bytes} bytes)"
        );
        let has_bce_free_loop = ir.blocks.iter().any(|b| {
            b.is_loop_header
                && b.insts.iter().any(|ins| {
                    matches!(
                        ins.op,
                        Op::Store {
                            kind: StoreKind::F64 | StoreKind::V128,
                            offset: 4096,
                            checked: true,
                            ..
                        }
                    )
                })
                && !b
                    .insts
                    .iter()
                    .any(|ins| matches!(ins.op, Op::BoundsCheck { .. }))
        });
        assert!(
            has_bce_free_loop,
            "versioned rotated fast loop must have folded f64/v128 stores with zero BoundsChecks"
        );
    }

    // 4. `inc_cache` v6 offset-table integrity hardening: crafted malformed offset tables and
    //    trailing bytes inside an entry slice must all be rejected cleanly with `None`.
    {
        let root = crate::build::create_fresh_test_root("wtime-aot-inc-v6-hardening");
        let w = wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0, 0]),
            code_sec(&[
                enc_body(&[], &[0x20, 0, 0x41, 1, 0x6a]),
                enc_body(&[], &[0x20, 0, 0x41, 2, 0x6a]),
            ]),
        ]);
        let opts = X64CompileOptions {
            opt: true,
            inc_cache_dir: Some(root.0.clone()),
            cache_key_salt: Some([0x77u8; 32]),
            ..Default::default()
        };
        let expected_so = compile_wasm_to_x64_so(&w, &opts).unwrap();
        let inc_dir = root.0.join("x64-inc");
        let bin_path = std::fs::read_dir(&inc_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "bin"))
            .expect("v6 cache file");
        let orig = std::fs::read(&bin_path).unwrap();
        let orig_payload = &orig[..orig.len() - 32];
        let abi: [u8; 32] = orig_payload[12..44].try_into().unwrap();
        let entries_base = 53 + 3 * 4;
        let off1 = u32::from_le_bytes(orig_payload[53 + 4..53 + 8].try_into().unwrap());
        let off2 = u32::from_le_bytes(orig_payload[53 + 8..53 + 12].try_into().unwrap());
        let bh0: [u8; 32] = orig_payload[entries_base..entries_base + 32]
            .try_into()
            .unwrap();
        let bh1: [u8; 32] = orig_payload
            [entries_base + off1 as usize..entries_base + off1 as usize + 32]
            .try_into()
            .unwrap();
        let bhs = [bh0, bh1];

        let sign = |p: &[u8]| -> Vec<u8> {
            let mut out = p.to_vec();
            let sum = super::inc_cache::parallel_payload_digest(p).unwrap();
            out.extend_from_slice(&sum);
            out
        };

        assert!(
            super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_some(),
            "valid v6 cache file must load"
        );

        // Case A: non-zero `offsets[0]`
        {
            let mut p = orig_payload.to_vec();
            p[53..57].copy_from_slice(&4u32.to_le_bytes());
            std::fs::write(&bin_path, sign(&p)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_none(),
                "non-zero offsets[0] must be rejected"
            );
        }
        // Case B: non-monotonic offsets (`offsets[1] > offsets[2]`)
        {
            let mut p = orig_payload.to_vec();
            p[57..61].copy_from_slice(&(off2 + 4).to_le_bytes());
            std::fs::write(&bin_path, sign(&p)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_none(),
                "non-monotonic offset table must be rejected"
            );
        }
        // Case C: `offsets[n_defs]` != `entries.len()`
        {
            let mut p = orig_payload.to_vec();
            p[61..65].copy_from_slice(&(off2 - 1).to_le_bytes());
            std::fs::write(&bin_path, sign(&p)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_none(),
                "mismatched final offset must be rejected"
            );
        }
        // Case D: truncated offset table
        {
            let p = orig_payload[..57].to_vec();
            std::fs::write(&bin_path, sign(&p)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_none(),
                "truncated offset table must be rejected"
            );
        }
        // Case E: extra trailing byte inside entry 0's slice
        {
            let mut p = orig_payload[..entries_base].to_vec();
            p[57..61].copy_from_slice(&(off1 + 1).to_le_bytes());
            p[61..65].copy_from_slice(&(off2 + 1).to_le_bytes());
            p.extend_from_slice(&orig_payload[entries_base..entries_base + off1 as usize]);
            p.push(0u8);
            p.extend_from_slice(&orig_payload[entries_base + off1 as usize..]);
            std::fs::write(&bin_path, sign(&p)).unwrap();
            assert!(
                super::inc_cache::load_inc_cache(&root.0, &abi, 0, 2, 0, 4096, &bhs).is_none(),
                "trailing byte inside v6 entry slice must be rejected"
            );
        }
        assert_eq!(compile_wasm_to_x64_so(&w, &opts).unwrap(), expected_so);
    }
}

#[test]
fn stage6_graph_bfs_ir_and_narrowing_check() {
    let wasm = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/graph_bfs.wasm"
    ))
    .expect("read graph_bfs.wasm");
    let val = wtime_validate::validate(
        &wasm,
        &super::x64_validate_features(),
        &super::x64_validate_limits(),
    )
    .unwrap();
    let module = super::module::admit_x64_module(val.decoded()).unwrap();
    let mut irs = Vec::new();
    for bi in 0..val.decoded().code.len() {
        let mut ir = super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap();
        super::opt::prune_early_block_params(&mut ir, None).unwrap();
        irs.push(ir);
    }
    super::opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    let effects = super::opt::compute_module_effects(&irs, &module, None).unwrap();
    let mut ir = irs[2].clone();
    super::opt::optimize_func_stage2(&mut ir, &module, &effects, None).unwrap();
    super::verify::verify_ir(&ir, &module).unwrap();
    // Sparse equality-filtered loop narrowing must eliminate the `v == k1 || v == k2` scan loop
    // in `graph_bfs::run` and emit branchless `CmpSelect` min/max preheader ordering!
    let has_cmp_select = ir.blocks.iter().any(|b| {
        b.insts
            .iter()
            .any(|ins| matches!(ins.op, super::ir::Op::CmpSelect { .. }))
    });
    assert!(
        has_cmp_select,
        "graph_bfs::run must narrow the equality-filtered inner scan loop"
    );
}

/// Shared Wasm module exercising ALL 20 relaxed SIMD opcodes (`0xfd 0x100..=0x113`).
pub(crate) fn build_relaxed_simd_test_wasm() -> Vec<u8> {
    // f0(case: i32) -> i32: populates 3 v128 inputs at mem[0..48] depending on `case`,
    // executes all 20 relaxed SIMD ops storing each 16-byte output to mem[64 + k*16],
    // and returns a folded i32 checksum over bytes 64..384.
    let mut b = Vec::new();
    let vconst = |out: &mut Vec<u8>, bytes: [u8; 16]| {
        out.extend_from_slice(&[0xfd, 0x0c]);
        out.extend_from_slice(&bytes);
    };
    let f32x4 = |a: f32, b: f32, c: f32, d: f32| -> [u8; 16] {
        let mut r = [0u8; 16];
        r[0..4].copy_from_slice(&a.to_bits().to_le_bytes());
        r[4..8].copy_from_slice(&b.to_bits().to_le_bytes());
        r[8..12].copy_from_slice(&c.to_bits().to_le_bytes());
        r[12..16].copy_from_slice(&d.to_bits().to_le_bytes());
        r
    };
    let f64x2 = |a: f64, b: f64| -> [u8; 16] {
        let mut r = [0u8; 16];
        r[0..8].copy_from_slice(&a.to_bits().to_le_bytes());
        r[8..16].copy_from_slice(&b.to_bits().to_le_bytes());
        r
    };
    // Locals: 1=a(v128), 2=b(v128), 3=c(v128), 4=i(i32), 5=acc(i32)
    // Select input triple based on `case` (0 = general/subnormal/dot, 1 = NaN/neg-zero/OOB-swizzle/sat-trunc, 2 = -128*i7/-128 extreme & non-canonical laneselect).
    // if case == 0
    b.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x40]);
    vconst(&mut b, f32x4(1.5, -2.25, 0.125, 100.75));
    b.extend_from_slice(&[0x21, 1]);
    vconst(&mut b, f32x4(-3.0, 4.5, 16.0, -0.5));
    b.extend_from_slice(&[0x21, 2]);
    vconst(&mut b, f32x4(0.25, -1.0, 1.0e-6, 42.0));
    b.extend_from_slice(&[0x21, 3]);
    b.push(0x05); // else
    // if case == 1
    b.extend_from_slice(&[0x20, 0, 0x41, 1, 0x46, 0x04, 0x40]);
    vconst(&mut b, f64x2(-0.0, f64::NAN));
    b.extend_from_slice(&[0x21, 1]);
    vconst(&mut b, f64x2(0.0, 1.0e30));
    b.extend_from_slice(&[0x21, 2]);
    vconst(
        &mut b,
        [
            0, 15, 16, 127, 128, 255, 3, 7, 1, 2, 3, 4, 0x5a, 0xff, 0x00, 0x80,
        ],
    );
    b.extend_from_slice(&[0x21, 3]);
    b.push(0x05); // else (case >= 2)
    vconst(
        &mut b,
        [
            0x80, 0x7f, 0x80, 0x80, 0xff, 0x01, 0x7f, 0x80, 0xfe, 0x02, 0x80, 0x7f, 0x40, 0xc0,
            0x00, 0xff,
        ],
    );
    b.extend_from_slice(&[0x21, 1]);
    vconst(
        &mut b,
        [
            0x7f, 0x7f, 0x80, 0x80, 0x01, 0x7f, 0x7f, 0x80, 0x02, 0x7e, 0x7f, 0x7f, 0x10, 0x7f,
            0x7f, 0x01,
        ],
    );
    b.extend_from_slice(&[0x21, 2]);
    vconst(
        &mut b,
        [
            0x5a, 0xff, 0x00, 0x80, 0x00, 0xff, 0xff, 0x00, 0xa5, 0x00, 0xff, 0x7f, 0x80, 0x00,
            0xff, 0x00,
        ],
    );
    b.extend_from_slice(&[0x21, 3]);
    b.extend_from_slice(&[0x0b, 0x0b]);

    let emit_store = |out: &mut Vec<u8>, slot: u32, op_sub: u8, arity: u8| {
        out.push(0x41);
        leb_i32(out, (64 + slot * 16) as i32);
        out.extend_from_slice(&[0x20, 1]);
        if arity >= 2 {
            out.extend_from_slice(&[0x20, 2]);
        }
        if arity >= 3 {
            out.extend_from_slice(&[0x20, 3]);
        }
        // Relaxed SIMD opcodes 0x100..=0x113 encode as `0xfd, 0x80 + (sub), 0x02`.
        out.extend_from_slice(&[0xfd, 0x80 | op_sub, 0x02]);
        // v128.store align=4 offset=0
        out.extend_from_slice(&[0xfd, 0x0b, 0x04, 0x00]);
    };
    let arities: [u8; 20] = [
        2, // 0x100 i8x16.relaxed_swizzle
        1, // 0x101 i32x4.relaxed_trunc_f32x4_s
        1, // 0x102 i32x4.relaxed_trunc_f32x4_u
        1, // 0x103 i32x4.relaxed_trunc_f64x2_s_zero
        1, // 0x104 i32x4.relaxed_trunc_f64x2_u_zero
        3, // 0x105 f32x4.relaxed_madd
        3, // 0x106 f32x4.relaxed_nmadd
        3, // 0x107 f64x2.relaxed_madd
        3, // 0x108 f64x2.relaxed_nmadd
        3, // 0x109 i8x16.relaxed_laneselect
        3, // 0x10a i16x8.relaxed_laneselect
        3, // 0x10b i32x4.relaxed_laneselect
        3, // 0x10c i64x2.relaxed_laneselect
        2, // 0x10d f32x4.relaxed_min
        2, // 0x10e f32x4.relaxed_max
        2, // 0x10f f64x2.relaxed_min
        2, // 0x110 f64x2.relaxed_max
        2, // 0x111 i16x8.relaxed_q15mulr_s
        2, // 0x112 i16x8.relaxed_dot_i8x16_i7x16_s
        3, // 0x113 i32x4.relaxed_dot_i8x16_i7x16_add_s
    ];
    for (k, &ar) in arities.iter().enumerate() {
        emit_store(&mut b, k as u32, k as u8, ar);
    }
    // Fold bytes 64..384 (80 i32 words) into checksum acc.
    b.extend_from_slice(&[0x41, 64, 0x21, 4, 0x41, 0, 0x21, 5]);
    b.extend_from_slice(&[0x03, 0x40]); // loop
    b.extend_from_slice(&[0x20, 5, 0x41]);
    leb_i32(&mut b, 31);
    b.extend_from_slice(&[0x6c, 0x20, 4, 0x28, 2, 0, 0x73, 0x21, 5]);
    b.extend_from_slice(&[0x20, 4, 0x41, 4, 0x6a, 0x22, 4, 0x41]);
    leb_i32(&mut b, 384);
    b.extend_from_slice(&[0x49, 0x0d, 0, 0x0b, 0x20, 5]);

    wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[("blot:memory", 2, 0), ("test_relaxed", 0, 0)]),
        code_sec(&[enc_body(&[(3, 0x7b), (2, 0x7f)], &b)]),
    ])
}

/// Shared Wasm module exercising `return_call` (0x12) and `return_call_indirect` (0x13):
/// O(1)-stack deep mutual recursion, high-arity stack args (same-arity O(1) AND growing-arity
/// fallback), multi-value overflow return buffers, and indirect call traps.
pub(crate) fn build_tail_call_test_wasm() -> Vec<u8> {
    let ten_i64 = [0x7e; 10];
    let hi_params = [
        0x7f, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e, 0x7e,
    ];
    // Types:
    // 0: (i32) -> (i32)
    // 1: (i32, i64 x 10) -> (i64)
    // 2: (i32) -> (i64)
    // 3: (i64) -> (i64 x 10)
    // 4: (i64) -> (i64)
    // 5: (i32) -> (i64 x 10)
    // 6: (i32, i64 x 10) -> (i64 x 10)
    // 7: () -> ()
    let tsec = type_sec(&[
        (&[0x7f], &[0x7f]),
        (&hi_params, &[0x7e]),
        (&[0x7f], &[0x7e]),
        (&[0x7e], &ten_i64),
        (&[0x7e], &[0x7e]),
        (&[0x7f], &ten_i64),
        (&hi_params, &ten_i64),
        (&[], &[]),
    ]);
    // Functions:
    // 0: even (type 0)
    // 1: odd (type 0)
    // 2: even_ind (type 0)
    // 3: odd_ind (type 0)
    // 4: sum_hi_arity (type 1)
    // 5: grow_arity_entry (type 2)
    // 6: leaf_multiret (type 3)
    // 7: tail_multiret (type 3)
    // 8: check_multiret (type 4)
    // 9: leaf_multiret_hi (type 6)
    // 10: grow_multiret_helper (type 5)
    // 11: grow_multiret_caller (type 2)
    // 12: bad_sig (type 7)
    // 13: ind_trap (type 0)
    let fsec = func_sec(&[0, 0, 0, 0, 1, 2, 3, 3, 4, 6, 5, 2, 7, 0]);
    // Table 0: 6 funcref slots; active elem at 0: [2, 3, 4, 6, 12] (slot 5 is null!).
    let tbl_p = vec![1, 0x70, 0x00, 6];
    let elem_p = vec![1, 0x00, 0x41, 0x00, 0x0b, 5, 2, 3, 4, 6, 12];

    // f0: even(n) = if n == 0 { 1 } else { return_call odd(n - 1) }
    let f0 = vec![
        0x20, 0, 0x45, 0x04, 0x7f, 0x41, 1, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x12, 1, 0x0b,
    ];
    // f1: odd(n) = if n == 0 { 0 } else { return_call even(n - 1) }
    let f1 = vec![
        0x20, 0, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x12, 0, 0x0b,
    ];
    // f2: even_ind(n) = if n == 0 { 1 } else { return_call_indirect type 0 table 0 (n - 1, slot 1) }
    let f2 = vec![
        0x20, 0, 0x45, 0x04, 0x7f, 0x41, 1, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x41, 1, 0x13, 0, 0, 0x0b,
    ];
    // f3: odd_ind(n) = if n == 0 { 0 } else { return_call_indirect type 0 table 0 (n - 1, slot 0) }
    let f3 = vec![
        0x20, 0, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x41, 0, 0x13, 0, 0, 0x0b,
    ];
    // f4: sum_hi_arity(n, a0..a9) -> if n == 0 { sum((k+1)*a_k) } else { return_call f4(n-1, a1..a9, a0+1) }
    let mut f4 = vec![0x20, 0, 0x45, 0x04, 0x7e];
    f4.extend_from_slice(&[0x42, 0]);
    for k in 0u8..10 {
        f4.extend_from_slice(&[0x20, 1 + k, 0x42, 2 * k + 1, 0x7e, 0x7c]);
    }
    f4.push(0x05);
    f4.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6b]);
    for k in 1u8..10 {
        f4.extend_from_slice(&[0x20, 1 + k]);
    }
    f4.extend_from_slice(&[0x20, 1, 0x42, 1, 0x7c]);
    f4.extend_from_slice(&[0x12, 4, 0x0b]);

    // f5: grow_arity_entry(n) = return_call sum_hi_arity(n, 1, 2, ..., 10)
    let mut f5 = vec![0x20, 0];
    for k in 1u8..=10 {
        f5.extend_from_slice(&[0x42, k]);
    }
    f5.extend_from_slice(&[0x12, 4]);

    // f6: leaf_multiret(x) -> (x+1, x+2, ..., x+10)
    let mut f6 = Vec::new();
    for k in 1u8..=10 {
        f6.extend_from_slice(&[0x20, 0, 0x42, k, 0x7c]);
    }
    // f7: tail_multiret(x) = return_call leaf_multiret(x + 7)
    let f7 = vec![0x20, 0, 0x42, 7, 0x7c, 0x12, 6];
    // f8: check_multiret(x) = call tail_multiret(x); fold 10 values with weighted sum
    let mut f8 = vec![0x20, 0, 0x10, 7];
    // Stack has [r0, r1, ..., r9]; pop into locals 1..=10 then fold
    for k in (1u8..=10).rev() {
        f8.extend_from_slice(&[0x21, k]);
    }
    f8.extend_from_slice(&[0x42, 0]);
    for k in 1u8..=10 {
        f8.extend_from_slice(&[0x20, k, 0x42, 3 * k, 0x7e, 0x7c]);
    }

    // f9: leaf_multiret_hi(n, a0..a9) -> if n == 0 { (a0..a9) } else { return_call f9(n-1, a1..a9, a0+3) }
    let mut f9 = vec![0x20, 0, 0x45, 0x04, 0x40];
    for k in 0u8..10 {
        f9.extend_from_slice(&[0x20, 1 + k]);
    }
    f9.extend_from_slice(&[0x0f, 0x0b]); // return; end
    f9.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6b]);
    for k in 1u8..10 {
        f9.extend_from_slice(&[0x20, 1 + k]);
    }
    f9.extend_from_slice(&[0x20, 1, 0x42, 3, 0x7c, 0x12, 9]);

    // f10: grow_multiret_helper(n) = return_call f9(n, 10, 20, ..., 100) [growing-arity + overflow retbuf!]
    let mut f10 = vec![0x20, 0];
    for k in 1i32..=10 {
        f10.push(0x42);
        leb_i32(&mut f10, k * 10);
    }
    f10.extend_from_slice(&[0x12, 9]);

    // f11: grow_multiret_caller(n) = call f10(n); fold 10 returns
    let mut f11 = vec![0x20, 0, 0x10, 10];
    for k in (1u8..=10).rev() {
        f11.extend_from_slice(&[0x21, k]);
    }
    f11.extend_from_slice(&[0x42, 0]);
    for k in 1u8..=10 {
        f11.extend_from_slice(&[0x20, k, 0x42, k, 0x7e, 0x7c]);
    }

    // f12: bad_sig () -> ()
    let f12 = Vec::new();
    // f13: ind_trap(slot) = return_call_indirect type 0 table 0 (42, slot)
    let f13 = vec![0x41, 42, 0x20, 0, 0x13, 0, 0];

    wasm_module(&[
        tsec,
        fsec,
        section(4, &tbl_p),
        export_sec(&[
            ("even", 0, 0),
            ("even_ind", 0, 2),
            ("grow_arity", 0, 5),
            ("check_multiret", 0, 8),
            ("grow_multiret", 0, 11),
            ("ind_trap", 0, 13),
        ]),
        section(9, &elem_p),
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[], &f1),
            enc_body(&[], &f2),
            enc_body(&[], &f3),
            enc_body(&[], &f4),
            enc_body(&[], &f5),
            enc_body(&[], &f6),
            enc_body(&[], &f7),
            enc_body(&[(10, 0x7e)], &f8),
            enc_body(&[], &f9),
            enc_body(&[], &f10),
            enc_body(&[(10, 0x7e)], &f11),
            enc_body(&[], &f12),
            enc_body(&[], &f13),
        ]),
    ])
}

/// Shared Wasm module testing `Op::F64AxpyLoop` (add/sub, aliasing, OOB partial-store trap)
/// AND growable `memory64` 1-branch bounds checks.
pub(crate) fn build_axpy_edge_test_wasm() -> Vec<u8> {
    // f0: axpy_add(dst: i32, src: i32, count: i32, scale: f64)
    // f1: axpy_sub(dst: i32, src: i32, count: i32, scale: f64)
    // f2: run_case(case: i32) -> i64
    let make_axpy = |is_sub: bool| -> Vec<u8> {
        let mut b = vec![
            0x41,
            0,
            0x21,
            4, // i = 0
            0x02,
            0x40,
            0x03,
            0x40, // block; loop
            0x20,
            4,
            0x20,
            2,
            0x4f,
            0x0d,
            1, // if i >=u count break
            // addr_d = dst + (i << 3)
            0x20,
            0,
            0x20,
            4,
            0x41,
            3,
            0x74,
            0x6a,
            0x22,
            5,
            // load dst[i]
            0x20,
            5,
            0x2b,
            3,
            0,
            // load src[i] * scale
            0x20,
            1,
            0x20,
            4,
            0x41,
            3,
            0x74,
            0x6a,
            0x2b,
            3,
            0,
            0x20,
            3,
            0xa2,
            // add or sub
            if is_sub { 0xa1 } else { 0xa0 },
            // store dst[i]
            0x39,
            3,
            0,
            // i += 1; br 0
            0x20,
            4,
            0x41,
            1,
            0x6a,
            0x21,
            4,
            0x0c,
            0,
            0x0b,
            0x0b,
        ];
        let _ = &mut b;
        b
    };
    let f0 = make_axpy(false);
    let f1 = make_axpy(true);

    // f2(case: i32) -> i64:
    // Seed mem[0..512] with 64 distinct f64 values, then run selected axpy case and return FNV hash of mem[0..512].
    let mut f2 = vec![
        // for i in 0..64 { mem[i*8] = f64(i * 3 - 50) * 0.25 }
        0x41, 0, 0x21, 1, // i = 0
        0x03, 0x40, 0x20, 1, 0x41, 3, 0x74, // i*8
        0x20, 1, 0x41, 3, 0x6c, 0x41, 50, 0x6b, 0xb7, // f64.convert_i32_s(i*3 - 50)
        0x44,
    ];
    f2.extend_from_slice(&0.25f64.to_bits().to_le_bytes());
    f2.extend_from_slice(&[
        0xa2, 0x39, 3, 0, 0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x41, 64, 0x49, 0x0d, 0, 0x0b,
    ]);
    let emit_call_case =
        |out: &mut Vec<u8>, id: i32, dst: i32, src: i32, cnt: i32, scale: f64, is_sub: bool| {
            out.push(0x20);
            out.push(0);
            out.push(0x41);
            leb_i32(out, id);
            out.extend_from_slice(&[0x46, 0x04, 0x40]);
            out.push(0x41);
            leb_i32(out, dst);
            out.push(0x41);
            leb_i32(out, src);
            out.push(0x41);
            leb_i32(out, cnt);
            out.push(0x44);
            out.extend_from_slice(&scale.to_bits().to_le_bytes());
            out.extend_from_slice(&[0x10, if is_sub { 1 } else { 0 }, 0x0b]);
        };
    // Disjoint counts: 0, 1, 2, 3, 4, 7, 8, 9, 15, 16, 17 (add & sub)
    for (idx, &cnt) in [0, 1, 2, 3, 4, 7, 8, 9, 15, 16, 17].iter().enumerate() {
        emit_call_case(&mut f2, idx as i32, 0, 256, cnt, 1.375, (idx & 1) == 1);
    }
    // Same-base dst == src (vectorizable!)
    emit_call_case(&mut f2, 20, 64, 64, 13, 0.5, false);
    emit_call_case(&mut f2, 21, 64, 64, 13, -0.75, true);
    // Forward overlap: dst < src < dst_end (must fall back to scalar!)
    emit_call_case(&mut f2, 22, 0, 8, 17, 1.25, false);
    // Backward overlap: src < dst < src_end (must fall back to scalar!)
    emit_call_case(&mut f2, 23, 8, 0, 17, 1.25, true);
    // OOB partial-store trap: dst = 65536 - 16, src = 0, count = 4 (writes 2 lanes then traps!)
    emit_call_case(&mut f2, 99, 65520, 0, 4, 2.0, false);

    // Hash mem[0..512] as i64
    f2.extend_from_slice(&[0x41, 0, 0x21, 1, 0x42, 0, 0x21, 2]);
    f2.extend_from_slice(&[
        0x03, 0x40, 0x20, 2, 0x42, 31, 0x7e, 0x20, 1, 0x29, 3, 0, 0x85, 0x21, 2, 0x20, 1, 0x41, 8,
        0x6a, 0x22, 1, 0x41,
    ]);
    leb_i32(&mut f2, 512);
    f2.extend_from_slice(&[0x49, 0x0d, 0, 0x0b, 0x20, 2]);

    wasm_module(&[
        type_sec(&[(&[0x7f, 0x7f, 0x7f, 0x7c], &[]), (&[0x7f], &[0x7e])]),
        func_sec(&[0, 0, 1]),
        section(5, &[1, 0x01, 1, 2]),
        export_sec(&[("blot:memory", 2, 0), ("run_case", 0, 2)]),
        code_sec(&[
            enc_body(&[(2, 0x7f)], &f0),
            enc_body(&[(2, 0x7f)], &f1),
            enc_body(&[(1, 0x7f), (1, 0x7e)], &f2),
        ]),
    ])
}

/// Shared Wasm module testing 1-branch growable `memory64` bounds checks (`min=1, max=4`).
pub(crate) fn build_mem64_1branch_test_wasm() -> Vec<u8> {
    // mem64: flags=0x05 (memory64 + max), min=1, max=4
    let mem_p = vec![1, 0x05, 1, 4];
    // f0(a: i64) -> i64: i64.load offset=0 (end_off=8 <= 65536 -> 1-branch fast check)
    let f0 = vec![0x20, 0, 0x29, 3, 0];
    // f1(a: i64) -> i64: i64.store offset=8 (a^0x1234), then i64.load offset=8 (end_off=16 <= 65536)
    let mut f1 = vec![0x20, 0, 0x20, 0, 0x42];
    leb_i32(&mut f1, 0x1234);
    f1.extend_from_slice(&[0x85, 0x37, 3, 8, 0x20, 0, 0x29, 3, 8]);
    // f2(a: i64) -> i64: memory.grow(1); drop; i64.store offset=8 (a+99); i64.load offset=8
    let mut f2 = vec![0x42, 1, 0x40, 0, 0x1a, 0x20, 0, 0x20, 0, 0x42];
    leb_i32(&mut f2, 99);
    f2.extend_from_slice(&[0x7c, 0x37, 3, 8, 0x20, 0, 0x29, 3, 8]);
    // f3(a: i64) -> i64: i64.load offset=70000 (> mem_min_bytes=65536 -> 2-branch wrap-proof check)
    let mut f3 = vec![0x20, 0, 0x29, 3];
    leb_u32(&mut f3, 70000);

    wasm_module(&[
        type_sec(&[(&[0x7e], &[0x7e])]),
        func_sec(&[0, 0, 0, 0]),
        section(5, &mem_p),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("load_off0", 0, 0),
            ("store_load_off8", 0, 1),
            ("grow_store_load_off8", 0, 2),
            ("load_off_big", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[], &f1),
            enc_body(&[], &f2),
            enc_body(&[], &f3),
        ]),
    ])
}

#[test]
fn stage7_f64_axpy_vectorizes_matmul_and_edge_module() {
    for (name, wasm) in [
        (
            "matmul_f64",
            std::fs::read(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/matmul_f64.wasm"),
            )
            .expect("read fixtures/matmul_f64.wasm"),
        ),
        ("axpy_edge", build_axpy_edge_test_wasm()),
    ] {
        let val = wtime_validate::validate(
            &wasm,
            &super::x64_validate_features(),
            &super::x64_validate_limits(),
        )
        .unwrap();
        let module = super::module::admit_x64_module(val.decoded()).unwrap();
        let mut irs = Vec::new();
        for bi in 0..val.decoded().code.len() {
            let mut ir = super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap();
            super::opt::prune_early_block_params(&mut ir, None).unwrap();
            irs.push(ir);
        }
        let effects = super::opt::compute_module_effects(&irs, &module, None).unwrap();
        let mut any_axpy = false;
        for ir0 in &irs {
            let mut ir = ir0.clone();
            super::opt::optimize_func_stage2(&mut ir, &module, &effects, None).unwrap();
            super::verify::verify_ir(&ir, &module).unwrap();
            if ir.blocks.iter().any(|b| {
                b.insts
                    .iter()
                    .any(|ins| matches!(ins.op, super::ir::Op::F64AxpyLoop { .. }))
            }) {
                any_axpy = true;
            }
        }
        assert!(any_axpy, "{name} must emit Op::F64AxpyLoop");
    }
}

#[test]
fn stage8_add_scaled_const64_pow2_rem_s_and_const_div_inline() {
    // f0(base: i32, idx: i64) -> i32: base + (i32.wrap_i64(idx) << 2) -> Op::AddScaled { is64: false, shift: 2 }
    let f0 = vec![0x20, 0, 0x20, 1, 0xa7, 0x41, 2, 0x74, 0x6a];
    // f1(base: i64, idx: i64) -> i64: base + (idx << 3) -> Op::AddScaled { is64: true, shift: 3 }
    let f1 = vec![0x20, 0, 0x20, 1, 0x42, 3, 0x86, 0x7c];
    // f2(x: i64, d: i64) -> i64: >64-inst helper with `x % d` so only constant-divisor loop specialization inlines it!
    let mut f2 = vec![0x20, 0, 0x20, 1, 0x81]; // x % d
    for _ in 0..70 {
        f2.extend_from_slice(&[0x42, 1, 0x7c]); // + 1 (keeps >64 raw insts before folding)
    }
    // Wait: ir builder folds `+1 +1 ...` if same const; use `local.get 0; xor` alternation so inst_count > 64!
    let mut f2_real = vec![0x20, 0, 0x20, 1, 0x81];
    for k in 0..35u8 {
        f2_real.extend_from_slice(&[0x20, 0, 0x85, 0x42, (k + 1) & 0x3f, 0x7c]);
    }
    let _ = f2;
    // f3(n: i64) -> i64: loop summing f2(i, 64) for i in 0..n
    let mut f3 = vec![
        0x42, 0, 0x21, 1, // acc = 0
        0x42, 0, 0x21, 2, // i = 0
        0x02, 0x40, // block
        0x03, 0x40, // loop
        0x20, 2, 0x20, 0, 0x57, 0x0d, 1, // br_if 1 (i >=s n)
        0x20, 1, 0x20, 2, 0x42, 0xc0, 0x00, 0x10, 2, 0x7c, 0x21, 1, // acc += f2(i, 64)
        0x20, 2, 0x42, 1, 0x7c, 0x21, 2, // i += 1
        0x0c, 0, // br 0
        0x0b, 0x0b, // end loop, end block
        0x20, 1, // return acc
    ];
    let _ = &mut f3;
    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f, 0x7e], &[0x7f]),
            (&[0x7e, 0x7e], &[0x7e]),
            (&[0x7e], &[0x7e]),
        ]),
        func_sec(&[0, 1, 1, 2]),
        export_sec(&[("f0", 0, 0), ("f1", 0, 1), ("f3", 0, 3)]),
        code_sec(&[
            enc_body(&[], &f0),
            enc_body(&[], &f1),
            enc_body(&[], &f2_real),
            enc_body(&[(2, 0x7e)], &f3),
        ]),
    ]);
    let val = wtime_validate::validate(
        &wasm,
        &super::x64_validate_features(),
        &super::x64_validate_limits(),
    )
    .unwrap();
    let module = super::module::admit_x64_module(val.decoded()).unwrap();
    let mut irs: Vec<IrFunc> = (0..val.decoded().code.len())
        .map(|bi| {
            let mut ir = super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap();
            super::opt::prune_early_block_params(&mut ir, None).unwrap();
            ir
        })
        .collect();
    assert!(
        !super::opt::is_inlinable_ir(&irs[0])
            || irs[2].blocks.iter().map(|b| b.insts.len()).sum::<usize>() > 64,
        "f2 must exceed standard 64-inst cap"
    );
    assert!(
        super::opt::is_inlinable_ir(&irs[2]),
        "f2 with param RemS must qualify under const-div specialization tier"
    );
    super::opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    let effects = super::opt::compute_module_effects(&irs, &module, None).unwrap();
    for ir in &mut irs {
        super::opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
        super::verify::verify_ir(ir, &module).unwrap();
        let alloc = super::regalloc::allocate(ir).unwrap();
        super::verify::verify_alloc(ir, &alloc).unwrap();
    }
    assert!(
        irs[0]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(
                i.op,
                Op::AddScaled {
                    is64: false,
                    shift: 2,
                    ..
                }
            ))),
        "f0 must fold wrap+shl+add into 32-bit Op::AddScaled"
    );
    assert!(
        irs[1]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(
                i.op,
                Op::AddScaled {
                    is64: true,
                    shift: 3,
                    ..
                }
            ))),
        "f1 must fold 64-bit shl+add into 64-bit Op::AddScaled"
    );
    let f3_has_call = irs[3]
        .blocks
        .iter()
        .any(|b| b.insts.iter().any(|i| matches!(i.op, Op::Call { .. })));
    let f3_has_imm_rem = irs[3].blocks.iter().any(|b| {
        b.insts
            .iter()
            .any(|i| matches!(i.op, Op::I64BinImm(super::ir::IntBinOp::RemS, _, 64)))
    });
    assert!(
        !f3_has_call,
        "f3 must inline const-div callee f2 inside loop"
    );
    assert!(
        f3_has_imm_rem,
        "f3 must fold inlined i64.rem_s(..., 64) into I64BinImm(RemS, _, 64)"
    );
}

#[test]
fn stage9_euclid_rem_bool_mask_and_loop_unroll() {
    // f0(x: i32) -> i32: r = x % 64; if r < 0 { r + 64 } else { r } -> x & 63
    let f0 = vec![
        0x20, 0, 0x41, 0xc0, 0x00, 0x6f, 0x21, 1, // r = x rem_s 64
        0x20, 1, 0x41, 0xc0, 0x00, 0x6a, // r + 64
        0x20, 1, // r
        0x20, 1, 0x41, 0, 0x48, // r <s 0
        0x1b, // select
    ];
    // f1(x: i64) -> i64: r = x % 32; if r < 0 { r + 32 } else { r } -> x & 31
    let f1 = vec![
        0x20, 0, 0x42, 32, 0x81, 0x21, 1, // r = x rem_s 32
        0x20, 1, 0x42, 32, 0x7c, // r + 32
        0x20, 1, // r
        0x20, 1, 0x42, 0, 0x53, // r <s 0
        0x1b, // select
    ];
    // f2(a: i32, b: i32) -> i32: (a == b) & 0xff -> a == b
    let f2 = vec![0x20, 0, 0x20, 1, 0x46, 0x41, 0xff, 0x01, 0x71];
    // f3(x: i32) -> i32: bottom-tested 3-iter loop `d = -1; do { acc += x * d + d; d += 1; } while d <= 1;`
    let f3 = vec![
        0x41, 0, 0x21, 1, // acc = 0
        0x41, 0x7f, 0x21, 2, // d = -1
        0x03, 0x40, // loop
        0x20, 1, 0x20, 0, 0x20, 2, 0x6c, 0x20, 2, 0x6a, 0x6a, 0x21, 1, // acc += x*d + d
        0x20, 2, 0x41, 1, 0x6a, 0x22, 2, // d += 1
        0x41, 1, 0x4c, // d <=s 1
        0x0d, 0,    // br_if 0
        0x0b, // end loop
        0x20, 1,
    ];
    let wasm = wasm_module(&[
        type_sec(&[
            (&[0x7f], &[0x7f]),
            (&[0x7e], &[0x7e]),
            (&[0x7f, 0x7f], &[0x7f]),
        ]),
        func_sec(&[0, 1, 2, 0]),
        export_sec(&[("f0", 0, 0), ("f1", 0, 1), ("f2", 0, 2), ("f3", 0, 3)]),
        code_sec(&[
            enc_body(&[(1, 0x7f)], &f0),
            enc_body(&[(1, 0x7e)], &f1),
            enc_body(&[], &f2),
            enc_body(&[(2, 0x7f)], &f3),
        ]),
    ]);
    let val = wtime_validate::validate(
        &wasm,
        &super::x64_validate_features(),
        &super::x64_validate_limits(),
    )
    .unwrap();
    let module = super::module::admit_x64_module(val.decoded()).unwrap();
    let mut irs: Vec<IrFunc> = (0..val.decoded().code.len())
        .map(|bi| super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap())
        .collect();
    let effects = super::opt::compute_module_effects(&irs, &module, None).unwrap();
    for ir in &mut irs {
        super::opt::optimize_func_stage2(ir, &module, &effects, None).unwrap();
        super::verify::verify_ir(ir, &module).unwrap();
        let alloc = super::regalloc::allocate(ir).unwrap();
        super::verify::verify_alloc(ir, &alloc).unwrap();
    }
    assert!(
        irs[0].blocks.iter().any(|b| b
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::I32BinImm(super::ir::IntBinOp::And, _, 63))))
            && !irs[0]
                .blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(
                    i.op,
                    Op::I32BinImm(super::ir::IntBinOp::RemS, _, _) | Op::CmpSelect { .. }
                ))),
        "f0 must fold signed Euclidean mod 64 into `x & 63`"
    );
    assert!(
        irs[1].blocks.iter().any(|b| b
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::I64BinImm(super::ir::IntBinOp::And, _, 31))))
            && !irs[1]
                .blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(
                    i.op,
                    Op::I64BinImm(super::ir::IntBinOp::RemS, _, _) | Op::CmpSelect { .. }
                ))),
        "f1 must fold signed Euclidean mod 32 into `x & 31`"
    );
    assert!(
        !irs[2].blocks.iter().any(|b| b
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::I32BinImm(super::ir::IntBinOp::And, _, 255)))),
        "f2 must eliminate redundant `And(_, 0xff)` on 0/1 comparison result"
    );
    let f3_has_backedge = irs[3].blocks.iter().enumerate().any(|(bi, b)| {
        let mut any = false;
        b.term
            .for_each_succ(&irs[3].br_tables, |s: super::ir::BlockId| {
                if s.0 as usize <= bi {
                    any = true;
                }
            });
        any
    });
    assert!(
        !f3_has_backedge,
        "f3 3-iter constant-trip loop must be fully unrolled with zero backedges"
    );
}

#[test]
fn heapsort_sift_down_hoists_root_and_linearizes_right_child() {
    use super::ir::{Op, Term};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/heapsort_i32.wasm");
    let wasm = std::fs::read(&path).expect("read heapsort_i32.wasm");
    let val = wtime_validate::validate(
        &wasm,
        &super::module::x64_validate_features(),
        &super::module::x64_validate_limits(),
    )
    .unwrap();
    let module = super::module::admit_x64_module(val.decoded()).unwrap();
    let mut irs: Vec<_> = (0..val.decoded().code.len())
        .map(|bi| super::ir::lower_wasm_to_ir(val.decoded(), &module, bi).unwrap())
        .collect();
    super::opt::inline_small_helpers(&mut irs, &module, None).unwrap();
    let eff = super::opt::compute_module_effects(&irs, &module, None).unwrap();
    super::opt::optimize_func_stage2(&mut irs[2], &module, &eff, None).unwrap();
    super::verify::verify_ir(&irs[2], &module).unwrap();
    let alloc = super::regalloc::allocate(&irs[2]).unwrap();
    super::verify::verify_alloc(&irs[2], &alloc).unwrap();
    let mut non_linear = Vec::new();
    let mut linear_tri_count = 0usize;
    for (oi, &b) in alloc.block_order.iter().enumerate() {
        let bb = &irs[2].blocks[b as usize];
        if let Term::CmpBranch {
            then_bb, else_bb, ..
        } = bb.term
        {
            for (arm, merge) in [(then_bb.0, else_bb.0), (else_bb.0, then_bb.0)] {
                let ab = &irs[2].blocks[arm as usize];
                if ab
                    .insts
                    .iter()
                    .any(|i| matches!(i.op, Op::LoadScaled { .. }))
                    && matches!(ab.term, Term::Jump { target, .. } if target.0 == merge)
                {
                    if alloc.block_order.get(oi + 1) == Some(&arm) {
                        linear_tri_count += 1;
                    } else {
                        non_linear.push((
                            b,
                            arm,
                            merge,
                            ab.preds,
                            ab.insts.len(),
                            ab.is_loop_header,
                            alloc.block_order.get(oi + 1).copied(),
                        ));
                    }
                }
            }
        }
    }
    assert!(
        linear_tri_count >= 2 && non_linear.is_empty(),
        "both right-child LoadScaled triangle arms must be linearized inline (found {linear_tri_count}, non_linear={non_linear:?})"
    );
}

#[test]
fn stack_elision_keeps_checks_for_call_ref_and_dyn_table_targets() {
    // type 0: (i32) -> i32  (used by f0 via call_ref and f1 via dynamic call_indirect)
    // type 1: (i64) -> i64  (used by f2 via direct call only)
    // type 2: (i32) -> i32  (entry)
    let types = type_sec(&[(&[0x7f], &[0x7f]), (&[0x7e], &[0x7e]), (&[0x7f], &[0x7f])]);
    let mut tbl = vec![1, 0x70, 0x00];
    leb_u32(&mut tbl, 2);
    // elem declarative for func 0 and func 1 so `ref.func 0`/`ref.func 1` validate.
    let elem = section(9, &[1, 0x03, 0x00, 2, 0, 1]);
    let f0 = enc_body(&[], &[0x20, 0, 0x41, 1, 0x6a]); // x + 1
    let f1 = enc_body(&[], &[0x20, 0, 0x41, 2, 0x6a]); // x + 2
    let f2 = enc_body(&[], &[0x20, 0, 0x42, 3, 0x7c]); // x + 3
    // f3 (entry): table.set 0 (0, ref.func 1); call_ref 0 (x, ref.func 0) + call_indirect 0 (x, 0) + i32.wrap(call 2 (i64.extend_u x))
    let f3 = enc_body(
        &[],
        &[
            0x41, 0, 0xd2, 1, 0x26, 0, // table.set 0 (0, ref.func 1)
            0x20, 0, 0xd2, 0, 0x14, 0, // call_ref 0 (x, ref.func 0)
            0x20, 0, 0x41, 0, 0x11, 0, 0,    // call_indirect type=0 tbl=0
            0x6a, // add
            0x20, 0, 0xad, 0x10, 2, 0xa7, // i32.wrap(call 2 (i64.extend_u x))
            0x6a,
        ],
    );
    let wasm = wasm_module(&[
        types,
        func_sec(&[0, 0, 1, 2]),
        section(4, &tbl),
        export_sec(&[("entry", 0, 3)]),
        elem,
        code_sec(&[f0, f1, f2, f3]),
    ]);
    let val =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let module = admit_x64_module(val.decoded()).unwrap();
    let mut irs: Vec<_> = (0..val.decoded().code.len())
        .map(|bi| lower_wasm_to_ir(val.decoded(), &module, bi).unwrap())
        .collect();
    let allocs: Vec<_> = irs.iter().map(|ir| allocate(ir).unwrap()).collect();
    super::opt::run_stack_and_trap_elision(&mut irs, &allocs, &module, None).unwrap();
    assert!(
        irs[0].needs_stack_check,
        "call_ref target f0 must NOT have its stack check elided"
    );
    assert!(
        irs[1].needs_stack_check,
        "dynamic call_indirect target f1 must NOT have its stack check elided"
    );
    assert!(
        !irs[2].needs_stack_check,
        "distinct-signature direct-only leaf f2 may still elide its stack check"
    );
}

#[test]
fn memory64_max_min_pages_compiles_without_shift_overflow() {
    fn encode_mem64(flags: u8, min_pages: u64) -> Vec<u8> {
        let mut out = vec![flags];
        let mut v = min_pages;
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
        out
    }
    let max_m64_pages: u64 = 1u64 << 48;
    // Module A: memory 0 has min_pages == 1 << 48 (MAX_MEM64_PAGES).
    let mut mem_a = vec![1];
    mem_a.extend_from_slice(&encode_mem64(0x04, max_m64_pages));
    let wasm_a = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        section(5, &mem_a),
        export_sec(&[("_start", 0, 0)]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    // Module B: memory 0 is small (1 page), extra memory 1 has min_pages == 1 << 48.
    let mut mem_b = vec![2, 0x00, 1];
    mem_b.extend_from_slice(&encode_mem64(0x04, max_m64_pages));
    let wasm_b = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        section(5, &mem_b),
        export_sec(&[("_start", 0, 0)]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    for (label, wasm) in [("mem0_1shl48", &wasm_a), ("mem1_1shl48", &wasm_b)] {
        for target in [
            super::module::X64Target::BlotCdylib,
            super::module::X64Target::StandaloneExec,
        ] {
            let x64_opts = X64CompileOptions {
                target,
                ..Default::default()
            };
            compile_wasm_to_x64_so(wasm, &x64_opts)
                .unwrap_or_else(|e| panic!("x64 {label} {target:?} compile failed: {e:?}"));
            let a64_opts = crate::aarch64::A64CompileOptions {
                target,
                ..Default::default()
            };
            crate::aarch64::compile_wasm_to_aarch64(wasm, &a64_opts)
                .unwrap_or_else(|e| panic!("aarch64 {label} {target:?} compile failed: {e:?}"));
        }
    }
}

#[test]
fn peel_boundary_clamped_loops_remaps_load_scaled_and_axpy_keeps_unrelated_checks() {
    use super::ir::{CmpBranchKind, CmpOp, FloatBinOp, LoadKind, StoreKind};
    // Build a minimal admitted module with 1 page of memory for IR verification.
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f, 0x7f, 0x7c], &[])]),
        func_sec(&[0]),
        section(5, &[1, 0x00, 1]),
        export_sec(&[("f", 0, 0)]),
        code_sec(&[enc_body(&[], &[])]),
    ]);
    let val =
        wtime_validate::validate(&wasm, &x64_validate_features(), &x64_validate_limits()).unwrap();
    let module = admit_x64_module(val.decoded()).unwrap();

    // 1. Boundary-clamped loop containing `LoadScaled` + `AddScaled` + `StoreScaled` + boundary select.
    let mut ir = IrFunc {
        func_index: 0,
        memory64: false,
        params: vec![Val(0), Val(1), Val(2)],
        result_tys: vec![],
        val_tys: vec![
            Ty::I32, // v0: init
            Ty::I32, // v1: limit
            Ty::F64, // v2: unused param
            Ty::I32, // v3: iv
            Ty::I32, // v4: next_i = iv + 1
            Ty::I32, // v5: clamped = (iv == 0) ? v0 : iv
            Ty::I32, // v6: loaded = LoadScaled(I32, clamped, 4, 0)
            Ty::I32, // v7: scaled = AddScaled(v6, clamped, 2, 16)
        ],
        blocks: vec![
            Block {
                params: vec![Val(0), Val(1), Val(2)],
                insts: vec![],
                term: Term::Jump {
                    target: BlockId(1),
                    args: ValRange { start: 0, len: 1 },
                },
                preds: 0,
                is_loop_header: false,
            },
            Block {
                params: vec![Val(3)],
                insts: vec![
                    Inst {
                        res: Some(Val(4)),
                        op: Op::I32BinImm(IntBinOp::Add, Val(3), 1),
                    },
                    Inst {
                        res: Some(Val(5)),
                        op: Op::CmpSelect {
                            kind: CmpBranchKind::I32Imm(CmpOp::Eq, 0),
                            a: Val(3),
                            b: Val(3),
                            tv: Val(0),
                            fv: Val(3),
                        },
                    },
                    Inst {
                        res: Some(Val(6)),
                        op: Op::LoadScaled {
                            kind: LoadKind::I32,
                            addr: Val(5),
                            scale: 4,
                            offset: 0,
                        },
                    },
                    Inst {
                        res: Some(Val(7)),
                        op: Op::AddScaled {
                            is64: false,
                            base: Val(6),
                            idx: Val(5),
                            shift: 2,
                            disp: 16,
                        },
                    },
                    Inst {
                        res: None,
                        op: Op::StoreScaled {
                            kind: StoreKind::I32,
                            addr: Val(3),
                            val: Val(7),
                            scale: 4,
                            offset: 0,
                        },
                    },
                ],
                term: Term::CmpBranch {
                    kind: CmpBranchKind::I32(CmpOp::LtU),
                    a: Val(4),
                    b: Val(1),
                    then_bb: BlockId(1),
                    then_args: ValRange { start: 1, len: 1 },
                    else_bb: BlockId(2),
                    else_args: ValRange::EMPTY,
                },
                preds: 2,
                is_loop_header: true,
            },
            Block {
                params: vec![],
                insts: vec![],
                term: Term::Return {
                    vals: ValRange::EMPTY,
                },
                preds: 1,
                is_loop_header: false,
            },
        ],
        val_pool: vec![Val(0), Val(4)],
        v128_consts: vec![],
        br_tables: vec![],
        early_pruned: 0,
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    let eff = super::opt::compute_module_effects(std::slice::from_ref(&ir), &module, None).unwrap();
    super::opt::optimize_func_stage2(&mut ir, &module, &eff, None).unwrap();
    verify_ir(&ir, &module)
        .expect("peeled loop with LoadScaled/AddScaled/StoreScaled must stay valid SSA");

    // 2. AXPY loop with an unrelated trapping `BoundsCheck` (addr = limit, end_off = 999999)
    // must NOT be vectorized into `Op::F64AxpyLoop` (which would silently drop the BoundsCheck!).
    let mut axpy = IrFunc {
        func_index: 0,
        memory64: false,
        params: vec![Val(0), Val(1), Val(2)],
        result_tys: vec![],
        val_tys: vec![
            Ty::I32, // v0: init (0)
            Ty::I32, // v1: limit
            Ty::F64, // v2: scale
            Ty::I32, // v3: iv
            Ty::I32, // v4: byte_off = iv << 3
            Ty::F64, // v5: dst_val = load(F64, byte_off, 0)
            Ty::F64, // v6: src_val = load(F64, byte_off, 256)
            Ty::F64, // v7: prod = src_val * scale
            Ty::F64, // v8: out = dst_val + prod
            Ty::I32, // v9: next = iv + 1
        ],
        blocks: vec![
            Block {
                params: vec![Val(0), Val(1), Val(2)],
                insts: vec![],
                term: Term::Jump {
                    target: BlockId(1),
                    args: ValRange { start: 0, len: 1 },
                },
                preds: 0,
                is_loop_header: false,
            },
            Block {
                params: vec![Val(3)],
                insts: vec![
                    Inst {
                        res: None,
                        op: Op::BoundsCheck {
                            addr: Val(1),
                            end_off: 999_999,
                            for_bulk: false,
                        },
                    },
                    Inst {
                        res: Some(Val(4)),
                        op: Op::I32BinImm(IntBinOp::Shl, Val(3), 3),
                    },
                    Inst {
                        res: Some(Val(5)),
                        op: Op::Load {
                            kind: LoadKind::F64,
                            addr: Val(4),
                            offset: 0,
                            checked: false,
                        },
                    },
                    Inst {
                        res: Some(Val(6)),
                        op: Op::Load {
                            kind: LoadKind::F64,
                            addr: Val(4),
                            offset: 256,
                            checked: false,
                        },
                    },
                    Inst {
                        res: Some(Val(7)),
                        op: Op::F64Bin(FloatBinOp::Mul, Val(6), Val(2)),
                    },
                    Inst {
                        res: Some(Val(8)),
                        op: Op::F64Bin(FloatBinOp::Add, Val(5), Val(7)),
                    },
                    Inst {
                        res: None,
                        op: Op::Store {
                            kind: StoreKind::F64,
                            addr: Val(4),
                            val: Val(8),
                            offset: 0,
                            checked: true,
                        },
                    },
                    Inst {
                        res: Some(Val(9)),
                        op: Op::I32BinImm(IntBinOp::Add, Val(3), 1),
                    },
                ],
                term: Term::CmpBranch {
                    kind: CmpBranchKind::I32(CmpOp::LtU),
                    a: Val(9),
                    b: Val(1),
                    then_bb: BlockId(1),
                    then_args: ValRange { start: 1, len: 1 },
                    else_bb: BlockId(2),
                    else_args: ValRange::EMPTY,
                },
                preds: 2,
                is_loop_header: true,
            },
            Block {
                params: vec![],
                insts: vec![],
                term: Term::Return {
                    vals: ValRange::EMPTY,
                },
                preds: 1,
                is_loop_header: false,
            },
        ],
        val_pool: vec![Val(0), Val(9)],
        v128_consts: vec![],
        br_tables: vec![],
        early_pruned: 0,
        needs_stack_check: true,
        elided_callee_stack_bytes: 0,
    };
    let eff2 =
        super::opt::compute_module_effects(std::slice::from_ref(&axpy), &module, None).unwrap();
    super::opt::optimize_func_stage2(&mut axpy, &module, &eff2, None).unwrap();
    verify_ir(&axpy, &module).unwrap();
    assert!(
        !axpy.blocks.iter().any(|b| b
            .insts
            .iter()
            .any(|i| matches!(i.op, Op::F64AxpyLoop { .. }))),
        "AXPY vectorization must NOT drop an unrelated BoundsCheck"
    );
}
