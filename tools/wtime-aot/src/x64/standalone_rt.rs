//! Self-contained Linux x86-64 static executable (`ET_EXEC`) runtime & direct-syscall
//! WASI / Goose host import trampolines for `--backend x64` standalone binaries.

use wtime_types::ValType;

use super::asm::{Asm, Cond, Gp, Mem, RelocTarget, Xmm};
use super::lower::TargetIsa;
use super::module::X64Module;
use super::runtime::{RuntimeTables, RuntimeTestHooks};
use super::{
    CtxLayout, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES, GUEST_STACK_TOTAL_BYTES,
    GUEST_STACK_USABLE_BYTES, HELPER_GOOSE_FMT_FLOAT, HELPER_INITIALIZE, OFF_EXEC_ARGC,
    OFF_EXEC_ARGV, OFF_EXEC_CLOCK_ORIGIN, OFF_EXEC_EXIT_CODE, OFF_EXEC_MEM_LIMIT,
    OFF_EXEC_TIMING_PATH, OFF_GUEST_STACK_BASE, OFF_GUEST_STACK_TOP, OFF_HANDLE_ALLOC_BYTES,
    OFF_MEM_BASE, OFF_MEM_LEN, OFF_MEM_MAX_PAGES, OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR,
    RodataConsts, TRAP_EXIT, TRAP_HOST, TRAP_MESSAGES, TRAP_OOB, TRAP_UNSUPPORTED_ISA,
};

const SYS_READ: u32 = 0;
const SYS_WRITE: u32 = 1;
const SYS_CLOSE: u32 = 3;
const SYS_MMAP: u32 = 9;
const SYS_MPROTECT: u32 = 10;
const SYS_RT_SIGACTION: u32 = 13;
const SYS_RT_SIGRETURN: u32 = 15;
const SYS_CLOCK_GETTIME: u32 = 228;
const SYS_EXIT_GROUP: u32 = 231;
const SYS_OPENAT: u32 = 257;

const SIGSEGV: u32 = 11;
const SA_SIGINFO_RESTORER: u64 = 0x0400_0004; // SA_SIGINFO (0x4) | SA_RESTORER (0x0400_0000)
/// `8 GiB + 64 KiB` (`0x2_0001_0000`): covers `u32::MAX (addr) + u32::MAX (offset) + 16 (v128)`
/// with zero wraparound possibility in standalone `memory32` guard-page BCE.
const GUARD_RESERVE_BYTES: u64 = (1u64 << 33) + 65_536;

const PROT_NONE: u32 = 0;
const PROT_RW: u32 = 3;
const MAP_PRIVATE_ANON: u32 = 0x22;
const MAP_PRIVATE_ANON_NORESERVE: u32 = 0x4022;

const CLOCK_REALTIME: u32 = 0;
const CLOCK_MONOTONIC: u32 = 1;

/// Emits a small inline helper writing unsigned decimal `Rax` to `[Rdi]` (returns length in `Rax`,
/// writes right-to-left into `[buf + 24]` and copies? Even simpler: pushes digits on stack or writes
/// right-to-left into a 32-byte buffer `[Rsp + buf_off .. buf_off + 32]`, returning `Rsi = start_ptr`, `Rdx = len`).
fn emit_fmt_u64_to_buf(a: &mut Asm, val: Gp, buf_end_disp: i32) {
    // Expects `val` in `Rax`. Uses `Rcx = 10`, `Rdx`, `Rsi = cursor`.
    // Returns `Rsi = ptr`, `Rdx = len`.
    if val != Gp::Rax {
        a.mov_rr(true, Gp::Rax, val);
    }
    a.lea(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, buf_end_disp));
    a.mov_ri32(Gp::Rcx, 10);
    let lbl_loop = a.new_label();
    a.bind(lbl_loop);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::Rcx); // Rax = q, Rdx = r
    a.add_ri(false, Gp::Rdx, b'0' as i32);
    a.sub_ri(true, Gp::Rsi, 1);
    a.mov_mr8(Mem::base(Gp::Rsi), Gp::Rdx);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, lbl_loop);
    a.lea(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, buf_end_disp));
    a.sub_rr(true, Gp::Rdx, Gp::Rsi);
}

/// Helper: reads `CLOCK_MONOTONIC` (or `CLOCK_REALTIME`) in nanoseconds into `Rax`.
/// Uses 16 bytes at `[Rsp + ts_off]`. Clobbers `Rax, Rcx, Rdx, Rdi, Rsi, R11`.
fn emit_clock_ns(a: &mut Asm, clock_id: u32, ts_off: i32) {
    a.mov_ri32(Gp::Rdi, clock_id);
    a.lea(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, ts_off));
    a.mov_ri32(Gp::Rax, SYS_CLOCK_GETTIME);
    a.syscall();
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, ts_off));
    a.imul_rri(true, Gp::Rax, Gp::Rax, 1_000_000_000);
    a.add_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, ts_off + 8));
}

/// Helper: writes `[Rsi .. +Rdx]` to `fd` (handles partial writes/EINTR).
/// Clobbers `Rax, Rcx, Rdx, Rdi, Rsi, R8, R11`.
fn emit_write_all(a: &mut Asm, fd: Gp) {
    let lbl_loop = a.new_label();
    let lbl_done = a.new_label();
    if fd != Gp::R8 {
        a.mov_rr(false, Gp::R8, fd);
    }
    a.bind(lbl_loop);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_rr(false, Gp::Rdi, Gp::R8);
    a.mov_ri32(Gp::Rax, SYS_WRITE);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4); // -EINTR
    a.jcc(Cond::E, lbl_loop);
    a.cmp_ri(true, Gp::Rax, 0);
    a.jcc(Cond::Le, lbl_done);
    a.add_rr(true, Gp::Rsi, Gp::Rax);
    a.sub_rr(true, Gp::Rdx, Gp::Rax);
    a.jmp(lbl_loop);
    a.bind(lbl_done);
}

/// Helper: bounds-check `(ptr: u64, len: u64)` against linear memory length (`R10` or `R13`).
/// Jumps to `lbl_oob` if `ptr + len` overflows or exceeds `mem_len`.
fn emit_check_range(a: &mut Asm, ptr: Gp, len: Gp, fixed_mem: bool, lbl_oob: super::asm::Label) {
    let mem_len = if fixed_mem {
        a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_MEM_LEN));
        Gp::R10
    } else {
        Gp::R13
    };
    a.mov_rr(true, Gp::Rax, ptr);
    a.add_rr(true, Gp::Rax, len);
    a.jcc(Cond::B, lbl_oob); // CF=1 => overflow
    a.cmp_rr(true, Gp::Rax, mem_len);
    a.jcc(Cond::A, lbl_oob);
}

/// Helper: compares NUL-terminated string in `Rdi` against constant ASCII bytes in local rodata.
/// Sets `Eax = 1` if equal, `0` otherwise. Clobbers `Rax, Rcx, Rsi, R10`.
fn emit_streq_const(a: &mut Asm, s: &[u8]) {
    let mut nul = Vec::with_capacity(s.len() + 1);
    nul.extend_from_slice(s);
    nul.push(0);
    let off = a.add_local_rodata(1, &nul);
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(off));
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    let lbl_lp = a.new_label();
    let lbl_no = a.new_label();
    let lbl_yes = a.new_label();
    let lbl_done = a.new_label();
    a.bind(lbl_lp);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0));
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.cmp_rr(false, Gp::Rax, Gp::R10);
    a.jcc(Cond::Ne, lbl_no);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_yes);
    a.add_ri(true, Gp::Rcx, 1);
    a.jmp(lbl_lp);
    a.bind(lbl_yes);
    a.mov_ri32(Gp::Rax, 1);
    a.jmp(lbl_done);
    a.bind(lbl_no);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(lbl_done);
}

/// Parses unsigned decimal NUL-terminated string at `Rdi` into `Rax`.
fn emit_parse_u64(a: &mut Asm) {
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    let lbl_lp = a.new_label();
    let lbl_done = a.new_label();
    a.bind(lbl_lp);
    a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rdi));
    a.sub_ri(false, Gp::Rcx, b'0' as i32);
    a.cmp_ri(false, Gp::Rcx, 10);
    a.jcc(Cond::Ae, lbl_done);
    a.imul_rri(true, Gp::Rax, Gp::Rax, 10);
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.add_ri(true, Gp::Rdi, 1);
    a.jmp(lbl_lp);
    a.bind(lbl_done);
}

/// Emits the standalone static Linux x86-64 `_start` entry point (`e_entry`).
pub fn emit_standalone_entry(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
    isa: TargetIsa,
    _hooks: RuntimeTestHooks,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(1024);

    // Linux process entry:
    //   [rsp + 0] = argc (u64)
    //   [rsp + 8] = argv[0], argv[1], ..., NULL
    // Save `R12 = argc`, `R13 = argv` before aligning `Rsp`!
    a.mov_rm(true, Gp::R12, Mem::base(Gp::Rsp));
    a.lea(true, Gp::R13, Mem::base_disp(Gp::Rsp, 8));
    // Align Rsp down to 16 bytes and reserve 160 bytes of local host frame:
    //   [rsp + 0..16]   = timespec scratch
    //   [rsp + 16..24]  = t0_ns
    //   [rsp + 24..32]  = t1_ns
    //   [rsp + 32..40]  = t2_ns
    //   [rsp + 40..48]  = timing_path (ptr or 0)
    //   [rsp + 48..56]  = mem_limit_pages (0 = default)
    //   [rsp + 56..64]  = guest_argc
    //   [rsp + 64..72]  = guest_argv
    //   [rsp + 72..80]  = host_mxcsr
    //   [rsp + 80..112] = decimal formatting scratch (32 B)
    a.and_ri(true, Gp::Rsp, -16);
    a.sub_ri(true, Gp::Rsp, 160);
    a.stmxcsr(Mem::base_disp(Gp::Rsp, 72));
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 40), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::R12);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 64), Gp::R13);

    // ---- Step 0: Parse optional host CLI flags before `--` ----
    // Recognizes `--timing-json PATH`, `--memory-limit-pages N`, `--allow-files DIR`,
    // `--allow-env VAL`, `--time-limit S`, `--output-limit N`, `--stack-limit-mib N`, and `--`.
    let lbl_arg_loop = a.new_label();
    let lbl_arg_done = a.new_label();
    a.mov_ri64(Gp::Rbx, 1); // arg index = 1 (skip argv[0])
    a.bind(lbl_arg_loop);
    a.cmp_rr(true, Gp::Rbx, Gp::R12);
    a.jcc(Cond::Ae, lbl_arg_done);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    // Check "--"
    emit_streq_const(&mut a, b"--");
    let lbl_not_sep = a.new_label();
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_not_sep);
    // Guest args start at argv[rbx+1], guest argc = argc - (rbx + 1), or keep argv[0] slot:
    // Point guest_argv to &argv[rbx] (overwrite argv[rbx] with argv[0]? No, don't mutate argv;
    // simply set guest_argc = argc - (rbx + 1), guest_argv = &argv[rbx + 1]).
    a.add_ri(true, Gp::Rbx, 1);
    a.mov_rr(true, Gp::Rax, Gp::R12);
    a.sub_rr(true, Gp::Rax, Gp::Rbx);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);
    a.lea(true, Gp::Rax, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 64), Gp::Rax);
    a.jmp(lbl_arg_done);

    a.bind(lbl_not_sep);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    emit_streq_const(&mut a, b"--timing-json");
    let lbl_not_tj = a.new_label();
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_not_tj);
    a.add_ri(true, Gp::Rbx, 1);
    a.cmp_rr(true, Gp::Rbx, Gp::R12);
    a.jcc(Cond::Ae, lbl_arg_done);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 40), Gp::Rax);
    a.add_ri(true, Gp::Rbx, 1);
    a.jmp(lbl_arg_loop);

    a.bind(lbl_not_tj);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    emit_streq_const(&mut a, b"--memory-limit-pages");
    let lbl_not_ml = a.new_label();
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_not_ml);
    a.add_ri(true, Gp::Rbx, 1);
    a.cmp_rr(true, Gp::Rbx, Gp::R12);
    a.jcc(Cond::Ae, lbl_arg_done);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
    emit_parse_u64(&mut a);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rax);
    a.add_ri(true, Gp::Rbx, 1);
    a.jmp(lbl_arg_loop);

    a.bind(lbl_not_ml);
    // Skip known 2-token runner flags (`--allow-files`, `--allow-env`, `--time-limit`, `--output-limit`, `--stack-limit-mib`)
    for flag in [
        b"--allow-files".as_slice(),
        b"--allow-env",
        b"--time-limit",
        b"--output-limit",
        b"--stack-limit-mib",
    ] {
        a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::R13, Gp::Rbx, 8, 0));
        emit_streq_const(&mut a, flag);
        let lbl_next = a.new_label();
        a.test_rr(false, Gp::Rax, Gp::Rax);
        a.jcc(Cond::E, lbl_next);
        a.add_ri(true, Gp::Rbx, 2);
        a.jmp(lbl_arg_loop);
        a.bind(lbl_next);
    }
    a.bind(lbl_arg_done);

    // ---- Step 1: CPUID ISA check (x86-64-v2 + optional x64-v3) ----
    let lbl_fail_isa = a.new_label();
    let lbl_fail_trap = a.new_label();
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.cpuid();
    a.mov_rr(false, Gp::Rbp, Gp::Rax);
    a.cmp_ri(false, Gp::Rbp, 1);
    a.jcc(Cond::B, lbl_fail_isa);
    if isa.has_abm_bmi1 || isa.has_bmi2 || isa.has_avx2 {
        a.cmp_ri(false, Gp::Rbp, 7);
        a.jcc(Cond::B, lbl_fail_isa);
    }
    a.mov_ri32(Gp::Rax, 1);
    a.cpuid();
    let mut baseline_mask: i32 = (1 << 9) | (1 << 19) | (1 << 20) | (1 << 23);
    if isa.has_avx2 {
        baseline_mask |= (1 << 27) | (1 << 28); // OSXSAVE + AVX
    }
    a.and_ri(false, Gp::Rcx, baseline_mask);
    a.cmp_ri(false, Gp::Rcx, baseline_mask);
    a.jcc(Cond::Ne, lbl_fail_isa);
    if isa.has_avx2 {
        a.xor_rr(false, Gp::Rcx, Gp::Rcx);
        a.xgetbv();
        a.and_ri(false, Gp::Rax, 0x6);
        a.cmp_ri(false, Gp::Rax, 0x6);
        a.jcc(Cond::Ne, lbl_fail_isa);
    }
    if isa.has_abm_bmi1 {
        a.mov_ri32(Gp::Rax, 0x8000_0000);
        a.cpuid();
        a.cmp_ri(false, Gp::Rax, 0x8000_0001u32 as i32);
        a.jcc(Cond::B, lbl_fail_isa);
        a.mov_ri32(Gp::Rax, 0x8000_0001);
        a.cpuid();
        a.and_ri(false, Gp::Rcx, 1 << 5);
        a.jcc(Cond::E, lbl_fail_isa);
    }
    let mut need_ebx7: i32 = 0;
    if isa.has_abm_bmi1 {
        need_ebx7 |= 1 << 3;
    }
    if isa.has_avx2 {
        need_ebx7 |= 1 << 5; // AVX2
    }
    if isa.has_bmi2 {
        need_ebx7 |= 1 << 8;
    }
    if need_ebx7 != 0 {
        a.mov_ri32(Gp::Rax, 7);
        a.xor_rr(false, Gp::Rcx, Gp::Rcx);
        a.cpuid();
        a.and_ri(false, Gp::Rbx, need_ebx7);
        a.cmp_ri(false, Gp::Rbx, need_ebx7);
        a.jcc(Cond::Ne, lbl_fail_isa);
    }

    // ---- Step 2: Record t0_ns ----
    emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);

    // ---- Step 3: Allocate Ctx + Guest Stack + Linear Memory ----
    let lbl_fail_alloc = a.new_label();
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri64(Gp::Rsi, layout.total_bytes as u64);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc);
    a.mov_rr(true, Gp::R15, Gp::Rax);

    a.mov_ri64(Gp::Rax, layout.total_bytes as u64);
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, OFF_HANDLE_ALLOC_BYTES),
        Gp::Rax,
    );
    // Populate exec fields in Ctx
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 56));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_ARGC), Gp::Rax);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 64));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_ARGV), Gp::Rax);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    a.mov_mr(
        true,
        Mem::base_disp(Gp::R15, OFF_EXEC_CLOCK_ORIGIN),
        Gp::Rax,
    );
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 40));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_TIMING_PATH), Gp::Rax);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 48));
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_MEM_LIMIT), Gp::Rax);

    // Guest stack: 64 KiB guard + 8 MiB RW
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri64(Gp::Rsi, GUEST_STACK_TOTAL_BYTES as u64);
    a.mov_ri32(Gp::Rdx, PROT_NONE);
    a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON_NORESERVE);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    a.mov_ri32(Gp::Rax, SYS_MMAP);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc);
    a.mov_rr(true, Gp::Rbx, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_BASE), Gp::Rbx);

    a.lea(
        true,
        Gp::Rdi,
        Mem::base_disp(Gp::Rbx, GUEST_STACK_GUARD_BYTES as i32),
    );
    a.mov_ri64(Gp::Rsi, GUEST_STACK_USABLE_BYTES as u64);
    a.mov_ri32(Gp::Rdx, PROT_RW);
    a.mov_ri32(Gp::Rax, SYS_MPROTECT);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, lbl_fail_alloc);

    a.lea(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rbx, GUEST_STACK_TOTAL_BYTES as i32),
    );
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP), Gp::Rax);
    a.lea(
        true,
        Gp::Rax,
        Mem::base_disp(
            Gp::Rbx,
            (GUEST_STACK_GUARD_BYTES + GUEST_STACK_FLOOR_HEADROOM) as i32,
        ),
    );
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_STACK_FLOOR), Gp::Rax);

    // Linear memory (Wasm32 or memory64)
    a.xor_rr(false, Gp::R14, Gp::R14);
    a.xor_rr(false, Gp::R13, Gp::R13);
    if let Some((mem_min, declared_max)) = module.memory {
        let lbl_have_lim = a.new_label();
        let lbl_mem_done = a.new_label();
        let default_lim = if module.memory64 {
            mem_min.max(131_072) // 8 GiB default for memory64
        } else {
            65_536
        };
        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 48));
        a.test_rr(true, Gp::Rbx, Gp::Rbx);
        a.jcc(Cond::Ne, lbl_have_lim);
        a.mov_ri64(Gp::Rbx, default_lim);
        a.bind(lbl_have_lim);

        a.mov_ri64(Gp::Rax, mem_min);
        a.cmp_rr(true, Gp::Rax, Gp::Rbx);
        a.jcc(Cond::A, lbl_fail_alloc);

        let hard_cap = if module.memory64 { 1u64 << 28 } else { 65_536 };
        let clamped_decl = declared_max.min(hard_cap);
        a.mov_ri64(Gp::Rbp, clamped_decl);
        a.cmp_rr(true, Gp::Rbx, Gp::Rbp);
        a.cmovcc(Cond::B, true, Gp::Rbp, Gp::Rbx);
        a.cmp_rr(true, Gp::Rax, Gp::Rbp);
        a.jcc(Cond::A, lbl_fail_alloc);

        if layout.use_guard_pages {
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_MAX_PAGES), Gp::Rbp);
            a.mov_ri64(Gp::Rbx, GUARD_RESERVE_BYTES);
        } else {
            a.test_rr(true, Gp::Rbp, Gp::Rbp);
            a.jcc(Cond::E, lbl_mem_done);
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_MAX_PAGES), Gp::Rbp);
            a.mov_rr(true, Gp::Rbx, Gp::Rbp);
            a.shl_imm(true, Gp::Rbx, 16);
        }
        a.xor_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(true, Gp::Rsi, Gp::Rbx);
        a.mov_ri32(Gp::Rdx, PROT_NONE);
        a.mov_ri32(Gp::R10, MAP_PRIVATE_ANON_NORESERVE);
        a.mov_ri64(Gp::R8, u64::MAX);
        a.xor_rr(false, Gp::R9, Gp::R9);
        a.mov_ri32(Gp::Rax, SYS_MMAP);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc);

        a.mov_rr(true, Gp::R14, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_BASE), Gp::R14);
        a.mov_mr(
            true,
            Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
            Gp::Rbx,
        );
        if mem_min > 0 {
            a.mov_ri64(Gp::R13, mem_min << 16);
            a.mov_rr(true, Gp::Rdi, Gp::R14);
            a.mov_rr(true, Gp::Rsi, Gp::R13);
            a.mov_ri32(Gp::Rdx, PROT_RW);
            a.mov_ri32(Gp::Rax, SYS_MPROTECT);
            a.syscall();
            a.cmp_ri(true, Gp::Rax, -4095);
            a.jcc(Cond::Ae, lbl_fail_alloc);
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_LEN), Gp::R13);
        }
        a.bind(lbl_mem_done);
    }

    // ---- Step 3b: Install SIGSEGV guard-page handler (standalone memory32 ONLY) ----
    let lbl_sig_handler = a.new_label();
    let lbl_sig_restorer = a.new_label();
    let lbl_sig_oob = a.new_label();
    if layout.use_guard_pages {
        // Initialize t1_ns = 0 so `lbl_sig_oob` knows whether fault occurred during initialize.
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
        // Build `struct kernel_sigaction` at `[rsp + 80..112]` (32 bytes):
        //   +0: sa_handler, +8: sa_flags, +16: sa_restorer, +24: sa_mask
        a.lea_rip_label(Gp::Rax, lbl_sig_handler);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 80), Gp::Rax);
        a.mov_ri64(Gp::Rax, SA_SIGINFO_RESTORER);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 88), Gp::Rax);
        a.lea_rip_label(Gp::Rax, lbl_sig_restorer);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 96), Gp::Rax);
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 104), Gp::Rax);
        a.mov_ri32(Gp::Rdi, SIGSEGV);
        a.lea(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, 80));
        a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        a.mov_ri32(Gp::R10, 8); // sizeof(kernel_sigset_t)
        a.mov_ri32(Gp::Rax, SYS_RT_SIGACTION);
        a.syscall();
        a.cmp_ri(true, Gp::Rax, -4095);
        a.jcc(Cond::Ae, lbl_fail_alloc);
    }

    // ---- Step 4: Switch to guest stack, run wt_initialize, then `_start` ----
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_mr(true, Mem::base_disp(Gp::Rax, -8), Gp::Rsp);
    a.lea(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -16));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));
    a.call_reloc(RelocTarget::Helper(HELPER_INITIALIZE));

    // Switch back briefly to record t1_ns
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
    a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -8));
    a.mov_rr(false, Gp::Rbx, Gp::R11); // save init trap code in Rbx
    emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rax);

    let lbl_after_start = a.new_label();
    a.test_rr(false, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::Ne, lbl_after_start);

    if let Some((start_fi, returns_i32)) = module.standalone_start {
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
        a.mov_mr(true, Mem::base_disp(Gp::Rax, -8), Gp::Rsp);
        a.lea(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -16));
        a.call_reloc(RelocTarget::Func(start_fi));
        a.mov_rr(false, Gp::Rbx, Gp::R11);
        if returns_i32 {
            let lbl_skip_ret = a.new_label();
            a.test_rr(false, Gp::Rbx, Gp::Rbx);
            a.jcc(Cond::Ne, lbl_skip_ret);
            a.movsxd_rr(Gp::Rax, Gp::Rax);
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_EXIT_CODE), Gp::Rax);
            a.bind(lbl_skip_ret);
        }
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
        a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -8));
        emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rax);
    }

    a.bind(lbl_after_start);
    if layout.use_guard_pages {
        // Restore default SIGSEGV disposition before writing timing JSON or exiting.
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 80), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 88), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 96), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 104), Gp::Rax);
        a.mov_ri32(Gp::Rdi, SIGSEGV);
        a.lea(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, 80));
        a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        a.mov_ri32(Gp::R10, 8);
        a.mov_ri32(Gp::Rax, SYS_RT_SIGACTION);
        a.syscall();
    }
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 72));

    // ---- Step 5: Optional `--timing-json PATH` write ----
    let lbl_no_timing = a.new_label();
    a.mov_rm(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, 40));
    a.test_rr(true, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_no_timing);
    a.mov_ri64(Gp::Rdi, (-100i64) as u64); // AT_FDCWD
    a.mov_ri32(Gp::Rdx, 0x241); // O_WRONLY | O_CREAT | O_TRUNC
    a.mov_ri32(Gp::R10, 0o644);
    a.mov_ri32(Gp::Rax, SYS_OPENAT);
    a.syscall();
    a.cmp_ri(true, Gp::Rax, 0);
    a.jcc(Cond::L, lbl_no_timing);
    a.mov_rr(false, Gp::Rbp, Gp::Rax); // fd in Rbp

    let b1 = b"{\n  \"engine\": \"wtime-aot\",\n  \"compile_instantiate_ns\": ";
    let b2 = b",\n  \"compile_ns\": 0,\n  \"start_function_ns\": ";
    let b3 = b",\n  \"engine_total_ns\": ";
    let b4 = b"\n}\n";
    let p1 = a.add_local_rodata(1, b1);
    let p2 = a.add_local_rodata(1, b2);
    let p3 = a.add_local_rodata(1, b3);
    let p4 = a.add_local_rodata(1, b4);

    // part 1
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p1));
    a.mov_ri32(Gp::Rdx, b1.len() as u32);
    emit_write_all(&mut a, Gp::Rbp);
    // t1 - t0
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 24));
    a.sub_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    emit_fmt_u64_to_buf(&mut a, Gp::Rax, 112);
    emit_write_all(&mut a, Gp::Rbp);
    // part 2
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p2));
    a.mov_ri32(Gp::Rdx, b2.len() as u32);
    emit_write_all(&mut a, Gp::Rbp);
    // t2 - t1
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 32));
    a.sub_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 24));
    emit_fmt_u64_to_buf(&mut a, Gp::Rax, 112);
    emit_write_all(&mut a, Gp::Rbp);
    // part 3
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p3));
    a.mov_ri32(Gp::Rdx, b3.len() as u32);
    emit_write_all(&mut a, Gp::Rbp);
    // t2 - t0
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 32));
    a.sub_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    emit_fmt_u64_to_buf(&mut a, Gp::Rax, 112);
    emit_write_all(&mut a, Gp::Rbp);
    // part 4
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p4));
    a.mov_ri32(Gp::Rdx, b4.len() as u32);
    emit_write_all(&mut a, Gp::Rbp);

    a.mov_rr(false, Gp::Rdi, Gp::Rbp);
    a.mov_ri32(Gp::Rax, SYS_CLOSE);
    a.syscall();
    a.bind(lbl_no_timing);

    // ---- Step 6: Exit cleanly or report trap message to stderr ----
    let lbl_normal_exit = a.new_label();
    a.test_rr(false, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::E, lbl_normal_exit);
    a.cmp_ri(false, Gp::Rbx, i32::from(TRAP_EXIT));
    a.jcc(Cond::E, lbl_normal_exit);
    a.jmp(lbl_fail_trap);

    a.bind(lbl_normal_exit);
    a.mov_rm(false, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_EXEC_EXIT_CODE));
    a.mov_ri32(Gp::Rax, SYS_EXIT_GROUP);
    a.syscall();
    a.ud2();

    a.bind(lbl_fail_isa);
    a.mov_ri32(Gp::Rbx, u32::from(TRAP_UNSUPPORTED_ISA));
    a.jmp(lbl_fail_trap);

    a.bind(lbl_fail_alloc);
    a.mov_ri32(Gp::Rbx, 12); // TRAP_ALLOC_FAILED

    a.bind(lbl_fail_trap);
    // Print canonical TRAP_MESSAGES[Rbx - 1] + "\n" to stderr (fd=2)
    a.movzx_rr8(Gp::Rax, Gp::Rbx);
    a.sub_ri(false, Gp::Rax, 1);
    let lbl_exit1 = a.new_label();
    a.cmp_ri(false, Gp::Rax, TRAP_MESSAGES.len() as i32);
    a.jcc(Cond::Ae, lbl_exit1);
    a.lea_rip(Gp::R9, RelocTarget::Rodata(tables.trap_table_off));
    a.movsxd_rm(Gp::Rsi, Mem::indexed(Gp::R9, Gp::Rax, 8, 0));
    a.add_rr(true, Gp::Rsi, Gp::R9);
    a.mov_rm(false, Gp::Rdx, Mem::indexed(Gp::R9, Gp::Rax, 8, 4));
    a.mov_ri32(Gp::R8, 2);
    emit_write_all(&mut a, Gp::R8);
    let nl_off = a.add_local_rodata(1, b"\n");
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(nl_off));
    a.mov_ri32(Gp::Rdx, 1);
    a.mov_ri32(Gp::R8, 2);
    emit_write_all(&mut a, Gp::R8);
    a.bind(lbl_exit1);
    a.mov_ri32(Gp::Rdi, 1);
    a.mov_ri32(Gp::Rax, SYS_EXIT_GROUP);
    a.syscall();
    a.ud2();

    if layout.use_guard_pages {
        // Landing pad entered in normal user context after `rt_sigreturn` redirects RIP.
        a.bind(lbl_sig_oob);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_GUEST_STACK_TOP));
        a.mov_rm(true, Gp::Rsp, Mem::base_disp(Gp::Rax, -8));
        a.mov_ri32(Gp::Rbx, u32::from(TRAP_OOB));
        emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rax);
        let lbl_have_t1 = a.new_label();
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 24));
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::Ne, lbl_have_t1);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
        a.bind(lbl_have_t1);
        a.jmp(lbl_after_start);

        // Linux x86-64 SA_SIGINFO handler: (rdi=sig, rsi=siginfo_t*, rdx=ucontext_t*)
        // Exact Linux x86-64 ABI offsets: si_addr = +16; gregs[R14]=+88, [R15]=+96, [RIP]=+168.
        let lbl_not_ours = a.new_label();
        a.bind(lbl_sig_handler);
        a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsi, 16)); // si_addr
        a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rdx, 96)); // ucontext R15 (Ctx*)
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::E, lbl_not_ours);
        a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::Rdx, 88)); // ucontext R14 (mem_base)
        a.test_rr(true, Gp::R8, Gp::R8);
        a.jcc(Cond::E, lbl_not_ours);
        a.cmp_rm(true, Gp::R8, Mem::base_disp(Gp::Rcx, OFF_MEM_BASE));
        a.jcc(Cond::Ne, lbl_not_ours);
        a.mov_rr(true, Gp::R9, Gp::Rax);
        a.sub_rr(true, Gp::R9, Gp::R8); // si_addr - mem_base
        a.mov_ri64(Gp::R10, GUARD_RESERVE_BYTES);
        a.cmp_rr(true, Gp::R9, Gp::R10);
        a.jcc(Cond::Ae, lbl_not_ours);
        a.lea_rip_label(Gp::R9, lbl_sig_oob);
        a.mov_mr(true, Mem::base_disp(Gp::Rdx, 168), Gp::R9); // ucontext RIP = lbl_sig_oob
        a.ret();

        a.bind(lbl_not_ours);
        // Not a linear-memory guard fault: restore SIG_DFL and re-fault authentically.
        a.sub_ri(true, Gp::Rsp, 40);
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
        a.mov_ri32(Gp::Rdi, SIGSEGV);
        a.mov_rr(true, Gp::Rsi, Gp::Rsp);
        a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        a.mov_ri32(Gp::R10, 8);
        a.mov_ri32(Gp::Rax, SYS_RT_SIGACTION);
        a.syscall();
        a.add_ri(true, Gp::Rsp, 40);
        a.ret();

        a.bind(lbl_sig_restorer);
        a.mov_ri32(Gp::Rax, SYS_RT_SIGRETURN);
        a.syscall();
        a.ud2();
    }

    a.finish_labels()?;
    Ok(a)
}

/// Emits a direct-syscall import trampoline for standalone executables (`Target::StandaloneExec`).
pub fn emit_standalone_import_trampoline(module: &X64Module, k: usize) -> Result<Asm, String> {
    let (ref mname, ref fname) = module.imports[k];
    let ty = module.func_type(k).ok_or("import type out of range")?;
    let is_mem64 = module.memory64;
    let fixed_mem = module.fixed32.is_some();
    let mut a = Asm::with_capacity(256);

    // Helper: zero-extend param register to 64-bit if its declared Wasm type is I32.
    let zext_param = |a: &mut Asm, reg: Gp, idx: usize| {
        if ty.params.get(idx) == Some(&ValType::I32) {
            a.mov_rr(false, reg, reg);
        }
    };

    match (mname.as_str(), fname.as_str()) {
        ("wasi_snapshot_preview1", "proc_exit") | ("goose", "exit") => {
            if ty.params[0] == ValType::I32 {
                a.movsxd_rr(Gp::Rdi, Gp::Rdi);
            }
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_EXIT_CODE), Gp::Rdi);
            a.mov_ri32(Gp::R11, u32::from(TRAP_EXIT));
            a.ret();
        }
        ("goose", "newline") => {
            let nl = a.add_local_rodata(1, b"\n");
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(nl));
            a.mov_ri32(Gp::Rdx, 1);
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("goose", "print_bool") => {
            let s_true = a.add_local_rodata(1, b"true");
            let s_false = a.add_local_rodata(1, b"false");
            let lbl_false = a.new_label();
            let lbl_go = a.new_label();
            a.test_rr(ty.params[0] == ValType::I64, Gp::Rdi, Gp::Rdi);
            a.jcc(Cond::E, lbl_false);
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(s_true));
            a.mov_ri32(Gp::Rdx, 4);
            a.jmp(lbl_go);
            a.bind(lbl_false);
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(s_false));
            a.mov_ri32(Gp::Rdx, 5);
            a.bind(lbl_go);
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("goose", "print_u64") => {
            zext_param(&mut a, Gp::Rdi, 0);
            a.sub_ri(true, Gp::Rsp, 40);
            emit_fmt_u64_to_buf(&mut a, Gp::Rdi, 32);
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.add_ri(true, Gp::Rsp, 40);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("goose", "print_i64") => {
            if ty.params[0] == ValType::I32 {
                a.movsxd_rr(Gp::Rdi, Gp::Rdi);
            }
            a.sub_ri(true, Gp::Rsp, 40);
            let lbl_pos = a.new_label();
            let lbl_do_write = a.new_label();
            a.mov_rr(true, Gp::R9, Gp::Rdi); // save original sign
            a.test_rr(true, Gp::Rdi, Gp::Rdi);
            a.jcc(Cond::Ge, lbl_pos);
            a.neg(true, Gp::Rdi);
            a.bind(lbl_pos);
            emit_fmt_u64_to_buf(&mut a, Gp::Rdi, 32);
            a.test_rr(true, Gp::R9, Gp::R9);
            a.jcc(Cond::Ge, lbl_do_write);
            a.sub_ri(true, Gp::Rsi, 1);
            a.mov_mi8(Mem::base(Gp::Rsi), b'-');
            a.add_ri(true, Gp::Rdx, 1);
            a.bind(lbl_do_write);
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.add_ri(true, Gp::Rsp, 40);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("goose", "print_f64") | ("goose", "print_f32") => {
            let is_f64 = fname == "print_f64";
            if is_f64 && ty.params[0] == ValType::F32 {
                a.cvtss2sd(Xmm::Xmm0, Xmm::Xmm0);
            } else if !is_f64 && ty.params[0] == ValType::F64 {
                a.cvtsd2ss(Xmm::Xmm0, Xmm::Xmm0);
            }
            a.sub_ri(true, Gp::Rsp, 40);
            a.mov_rr(true, Gp::Rdi, Gp::Rsp);
            if is_f64 {
                a.mov_ri32(Gp::Rsi, 1);
            } else {
                a.xor_rr(false, Gp::Rsi, Gp::Rsi);
            }
            a.call_reloc(RelocTarget::Helper(HELPER_GOOSE_FMT_FLOAT));
            a.mov_rr(true, Gp::Rsi, Gp::Rsp);
            a.mov_rr(true, Gp::Rdx, Gp::Rax);
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.add_ri(true, Gp::Rsp, 40);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("goose", "format_f64") | ("goose", "format_f32") => {
            let is_f64 = fname == "format_f64";
            zext_param(&mut a, Gp::Rdi, 0);
            if is_f64 && ty.params[1] == ValType::F32 {
                a.cvtss2sd(Xmm::Xmm0, Xmm::Xmm0);
            } else if !is_f64 && ty.params[1] == ValType::F64 {
                a.cvtsd2ss(Xmm::Xmm0, Xmm::Xmm0);
            }
            let lbl_oob = a.new_label();
            a.mov_ri64(Gp::Rsi, 32);
            emit_check_range(&mut a, Gp::Rdi, Gp::Rsi, fixed_mem, lbl_oob);
            a.sub_ri(true, Gp::Rsp, 8);
            a.lea(true, Gp::Rdi, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
            if is_f64 {
                a.mov_ri32(Gp::Rsi, 1);
            } else {
                a.xor_rr(false, Gp::Rsi, Gp::Rsi);
            }
            a.call_reloc(RelocTarget::Helper(HELPER_GOOSE_FMT_FLOAT));
            a.add_ri(true, Gp::Rsp, 8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "bench_input") => {
            // Read one whitespace-delimited signed decimal integer from stdin (fd=0), byte-by-byte
            // so successive `bench_input()` calls never over-consume subsequent tokens.
            a.push(Gp::Rbx);
            a.push(Gp::Rbp);
            a.sub_ri(true, Gp::Rsp, 24);
            let lbl_skip_ws = a.new_label();
            let lbl_check_sign = a.new_label();
            let lbl_read_first = a.new_label();
            let lbl_digits = a.new_label();
            let lbl_got_digit = a.new_label();
            let lbl_done = a.new_label();
            let lbl_fail = a.new_label();

            // Helper inline: read 1 byte into `[rsp+0]`, looping on -EINTR; Rax <= 0 => EOF/err.
            let emit_read1 = |a: &mut Asm| {
                let lp = a.new_label();
                a.bind(lp);
                a.xor_rr(false, Gp::Rdi, Gp::Rdi); // fd = 0
                a.mov_rr(true, Gp::Rsi, Gp::Rsp);
                a.mov_ri32(Gp::Rdx, 1);
                a.mov_ri32(Gp::Rax, SYS_READ);
                a.syscall();
                a.cmp_ri(true, Gp::Rax, -4); // -EINTR
                a.jcc(Cond::E, lp);
            };

            a.xor_rr(false, Gp::Rbp, Gp::Rbp); // rbp = neg flag
            a.xor_rr(false, Gp::Rbx, Gp::Rbx); // rbx = acc

            a.bind(lbl_skip_ws);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.cmp_ri(false, Gp::Rcx, b' ' as i32);
            a.jcc(Cond::E, lbl_skip_ws);
            a.sub_ri(false, Gp::Rcx, 9);
            a.cmp_ri(false, Gp::Rcx, 5); // '\t' (9) ..= '\r' (13)
            a.jcc(Cond::B, lbl_skip_ws);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));

            a.bind(lbl_check_sign);
            let lbl_not_minus = a.new_label();
            a.cmp_ri(false, Gp::Rcx, b'-' as i32);
            a.jcc(Cond::Ne, lbl_not_minus);
            a.mov_ri32(Gp::Rbp, 1);
            a.jmp(lbl_read_first);
            a.bind(lbl_not_minus);
            let lbl_have_first = a.new_label();
            a.cmp_ri(false, Gp::Rcx, b'+' as i32);
            a.jcc(Cond::Ne, lbl_have_first);

            a.bind(lbl_read_first);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));

            a.bind(lbl_have_first);
            a.sub_ri(false, Gp::Rcx, b'0' as i32);
            a.cmp_ri(false, Gp::Rcx, 10);
            a.jcc(Cond::Ae, lbl_fail);
            a.jmp(lbl_got_digit);

            a.bind(lbl_digits);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_done);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.sub_ri(false, Gp::Rcx, b'0' as i32);
            a.cmp_ri(false, Gp::Rcx, 10);
            a.jcc(Cond::Ae, lbl_done);

            a.bind(lbl_got_digit);
            a.imul_rri(true, Gp::Rbx, Gp::Rbx, 10);
            a.add_rr(true, Gp::Rbx, Gp::Rcx);
            a.jmp(lbl_digits);

            a.bind(lbl_done);
            a.mov_rr(true, Gp::Rax, Gp::Rbx);
            let lbl_pos = a.new_label();
            a.test_rr(false, Gp::Rbp, Gp::Rbp);
            a.jcc(Cond::E, lbl_pos);
            a.neg(true, Gp::Rax);
            a.bind(lbl_pos);
            if ty.results[0] == ValType::I32 {
                a.mov_rr(false, Gp::Rax, Gp::Rax);
            }
            a.add_ri(true, Gp::Rsp, 24);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();

            a.bind(lbl_fail);
            a.add_ri(true, Gp::Rsp, 24);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "bench_input_f64") => {
            // Read one whitespace-delimited decimal floating-point token from stdin (fd=0) into Xmm0.
            a.push(Gp::Rbx);
            a.push(Gp::Rbp);
            a.sub_ri(true, Gp::Rsp, 40);
            let ten_off = a.add_local_rodata(8, &10.0f64.to_bits().to_le_bytes());
            let neg_mask_off = a.add_local_rodata(16, &{
                let mut m = [0u8; 16];
                m[..8].copy_from_slice(&0x8000_0000_0000_0000u64.to_le_bytes());
                m
            });
            let emit_read1 = |a: &mut Asm| {
                let lp = a.new_label();
                a.bind(lp);
                a.xor_rr(false, Gp::Rdi, Gp::Rdi);
                a.mov_rr(true, Gp::Rsi, Gp::Rsp);
                a.mov_ri32(Gp::Rdx, 1);
                a.mov_ri32(Gp::Rax, SYS_READ);
                a.syscall();
                a.cmp_ri(true, Gp::Rax, -4);
                a.jcc(Cond::E, lp);
            };
            let lbl_ws = a.new_label();
            let lbl_after_sign = a.new_label();
            let lbl_read_after_sign = a.new_label();
            let lbl_int_lp = a.new_label();
            let lbl_frac_lp = a.new_label();
            let lbl_exp_sign = a.new_label();
            let lbl_exp_lp = a.new_label();
            let lbl_scale = a.new_label();
            let lbl_scale_pos = a.new_label();
            let lbl_scale_neg = a.new_label();
            let lbl_finish = a.new_label();
            let lbl_fail = a.new_label();

            // [rsp+8]=neg, [rsp+16]=exp10 (i64), [rsp+24]=any_digit, rbx=mant (u64), rbp=mant_digits
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rax);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rax);
            a.xor_rr(false, Gp::Rbx, Gp::Rbx);
            a.xor_rr(false, Gp::Rbp, Gp::Rbp);

            a.bind(lbl_ws);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.cmp_ri(false, Gp::Rcx, b' ' as i32);
            a.jcc(Cond::E, lbl_ws);
            a.sub_ri(false, Gp::Rcx, 9);
            a.cmp_ri(false, Gp::Rcx, 5);
            a.jcc(Cond::B, lbl_ws);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));

            let lbl_not_m = a.new_label();
            a.cmp_ri(false, Gp::Rcx, b'-' as i32);
            a.jcc(Cond::Ne, lbl_not_m);
            a.mov_mi32(true, Mem::base_disp(Gp::Rsp, 8), 1);
            a.jmp(lbl_read_after_sign);
            a.bind(lbl_not_m);
            a.cmp_ri(false, Gp::Rcx, b'+' as i32);
            a.jcc(Cond::Ne, lbl_after_sign);

            a.bind(lbl_read_after_sign);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));

            // Integer part loop
            a.bind(lbl_after_sign);
            let lbl_check_dot = a.new_label();
            a.bind(lbl_int_lp);
            a.mov_rr(false, Gp::Rdx, Gp::Rcx);
            a.sub_ri(false, Gp::Rdx, b'0' as i32);
            a.cmp_ri(false, Gp::Rdx, 10);
            a.jcc(Cond::Ae, lbl_check_dot);
            a.mov_mi32(true, Mem::base_disp(Gp::Rsp, 24), 1);
            let lbl_int_ovf = a.new_label();
            let lbl_int_next = a.new_label();
            a.cmp_ri(true, Gp::Rbp, 18);
            a.jcc(Cond::Ae, lbl_int_ovf);
            a.imul_rri(true, Gp::Rbx, Gp::Rbx, 10);
            a.add_rr(true, Gp::Rbx, Gp::Rdx);
            a.add_ri(true, Gp::Rbp, 1);
            a.jmp(lbl_int_next);
            a.bind(lbl_int_ovf);
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
            a.add_ri(true, Gp::Rax, 1);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
            a.bind(lbl_int_next);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_scale);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.jmp(lbl_int_lp);

            // Optional '.' fractional part
            a.bind(lbl_check_dot);
            let lbl_check_exp = a.new_label();
            a.cmp_ri(false, Gp::Rcx, b'.' as i32);
            a.jcc(Cond::Ne, lbl_check_exp);
            a.bind(lbl_frac_lp);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_scale);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.mov_rr(false, Gp::Rdx, Gp::Rcx);
            a.sub_ri(false, Gp::Rdx, b'0' as i32);
            a.cmp_ri(false, Gp::Rdx, 10);
            a.jcc(Cond::Ae, lbl_check_exp);
            a.mov_mi32(true, Mem::base_disp(Gp::Rsp, 24), 1);
            a.cmp_ri(true, Gp::Rbp, 18);
            a.jcc(Cond::Ae, lbl_frac_lp);
            a.imul_rri(true, Gp::Rbx, Gp::Rbx, 10);
            a.add_rr(true, Gp::Rbx, Gp::Rdx);
            a.add_ri(true, Gp::Rbp, 1);
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
            a.sub_ri(true, Gp::Rax, 1);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
            a.jmp(lbl_frac_lp);

            // Optional 'e'/'E' exponent
            a.bind(lbl_check_exp);
            a.cmp_ri(false, Gp::Rcx, b'e' as i32);
            a.jcc(Cond::E, lbl_exp_sign);
            a.cmp_ri(false, Gp::Rcx, b'E' as i32);
            a.jcc(Cond::Ne, lbl_scale);
            a.bind(lbl_exp_sign);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.xor_rr(false, Gp::R8, Gp::R8); // exp_neg
            a.xor_rr(false, Gp::R9, Gp::R9); // exp_val
            let lbl_e_not_m = a.new_label();
            let lbl_e_have = a.new_label();
            a.cmp_ri(false, Gp::Rcx, b'-' as i32);
            a.jcc(Cond::Ne, lbl_e_not_m);
            a.mov_ri32(Gp::R8, 1);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.jmp(lbl_e_have);
            a.bind(lbl_e_not_m);
            a.cmp_ri(false, Gp::Rcx, b'+' as i32);
            a.jcc(Cond::Ne, lbl_e_have);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_fail);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.bind(lbl_e_have);
            a.bind(lbl_exp_lp);
            a.mov_rr(false, Gp::Rdx, Gp::Rcx);
            a.sub_ri(false, Gp::Rdx, b'0' as i32);
            a.cmp_ri(false, Gp::Rdx, 10);
            let lbl_exp_done = a.new_label();
            a.jcc(Cond::Ae, lbl_exp_done);
            a.imul_rri(true, Gp::R9, Gp::R9, 10);
            a.add_rr(true, Gp::R9, Gp::Rdx);
            emit_read1(&mut a);
            a.cmp_ri(true, Gp::Rax, 0);
            a.jcc(Cond::Le, lbl_exp_done);
            a.movzx_rm8(false, Gp::Rcx, Mem::base(Gp::Rsp));
            a.jmp(lbl_exp_lp);
            a.bind(lbl_exp_done);
            let lbl_e_pos = a.new_label();
            a.test_rr(false, Gp::R8, Gp::R8);
            a.jcc(Cond::E, lbl_e_pos);
            a.neg(true, Gp::R9);
            a.bind(lbl_e_pos);
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
            a.add_rr(true, Gp::Rax, Gp::R9);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);

            a.bind(lbl_scale);
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 24));
            a.test_rr(true, Gp::Rax, Gp::Rax);
            a.jcc(Cond::E, lbl_fail);
            a.xorps(Xmm::Xmm0, Xmm::Xmm0);
            a.cvtsi2sd(true, Xmm::Xmm0, Gp::Rbx);
            a.movsd_rip(Xmm::Xmm1, RelocTarget::LocalRodata(ten_off));
            a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 16));
            a.test_rr(true, Gp::Rcx, Gp::Rcx);
            a.jcc(Cond::E, lbl_finish);
            a.jcc(Cond::L, lbl_scale_neg);
            a.bind(lbl_scale_pos);
            a.mulsd(Xmm::Xmm0, Xmm::Xmm1);
            a.sub_ri(true, Gp::Rcx, 1);
            a.jcc(Cond::G, lbl_scale_pos);
            a.jmp(lbl_finish);
            a.bind(lbl_scale_neg);
            a.neg(true, Gp::Rcx);
            let lbl_sn_lp = a.new_label();
            a.bind(lbl_sn_lp);
            a.divsd(Xmm::Xmm0, Xmm::Xmm1);
            a.sub_ri(true, Gp::Rcx, 1);
            a.jcc(Cond::G, lbl_sn_lp);

            a.bind(lbl_finish);
            let lbl_ret_ok = a.new_label();
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 8));
            a.test_rr(true, Gp::Rax, Gp::Rax);
            a.jcc(Cond::E, lbl_ret_ok);
            a.xmm_rip_op(
                Some(0x66),
                0x57,
                Xmm::Xmm0,
                RelocTarget::LocalRodata(neg_mask_off),
            );
            a.bind(lbl_ret_ok);
            if ty.results[0] == ValType::F32 {
                a.cvtsd2ss(Xmm::Xmm0, Xmm::Xmm0);
            }
            a.add_ri(true, Gp::Rsp, 40);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();

            a.bind(lbl_fail);
            a.add_ri(true, Gp::Rsp, 40);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
            a.ret();
        }
        ("goose", "write") => {
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rsi, 1);
            let lbl_oob = a.new_label();
            let lbl_ok = a.new_label();
            emit_check_range(&mut a, Gp::Rdi, Gp::Rsi, fixed_mem, lbl_oob);
            a.test_rr(true, Gp::Rsi, Gp::Rsi);
            a.jcc(Cond::E, lbl_ok);
            a.mov_rr(true, Gp::Rdx, Gp::Rsi);
            a.lea(true, Gp::Rsi, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
            a.mov_ri32(Gp::R8, 1);
            emit_write_all(&mut a, Gp::R8);
            a.bind(lbl_ok);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
            a.ret();
        }
        ("goose", "abort") => {
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rsi, 1);
            a.sub_ri(true, Gp::Rsp, 56);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::Rdi);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rsi);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rdx);
            let b1 = b"goose runtime error: ";
            let b2 = b" (line ";
            let b3 = b")\n";
            let p1 = a.add_local_rodata(1, b1);
            let p2 = a.add_local_rodata(1, b2);
            let p3 = a.add_local_rodata(1, b3);
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p1));
            a.mov_ri32(Gp::Rdx, b1.len() as u32);
            a.mov_ri32(Gp::R8, 2);
            emit_write_all(&mut a, Gp::R8);
            let lbl_skip_msg = a.new_label();
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rsp, 0));
            a.mov_rm(true, Gp::Rsi, Mem::base_disp(Gp::Rsp, 8));
            emit_check_range(&mut a, Gp::Rdi, Gp::Rsi, fixed_mem, lbl_skip_msg);
            a.test_rr(true, Gp::Rsi, Gp::Rsi);
            a.jcc(Cond::E, lbl_skip_msg);
            a.mov_rr(true, Gp::Rdx, Gp::Rsi);
            a.lea(true, Gp::Rsi, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
            a.mov_ri32(Gp::R8, 2);
            emit_write_all(&mut a, Gp::R8);
            a.bind(lbl_skip_msg);
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p2));
            a.mov_ri32(Gp::Rdx, b2.len() as u32);
            a.mov_ri32(Gp::R8, 2);
            emit_write_all(&mut a, Gp::R8);
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
            emit_fmt_u64_to_buf(&mut a, Gp::Rax, 56);
            a.mov_ri32(Gp::R8, 2);
            emit_write_all(&mut a, Gp::R8);
            a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(p3));
            a.mov_ri32(Gp::Rdx, b3.len() as u32);
            a.mov_ri32(Gp::R8, 2);
            emit_write_all(&mut a, Gp::R8);
            a.add_ri(true, Gp::Rsp, 56);
            a.mov_ri64(Gp::Rax, 1);
            a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_EXIT_CODE), Gp::Rax);
            a.mov_ri32(Gp::R11, u32::from(TRAP_EXIT));
            a.ret();
        }
        ("env", "gs_os_write_stdout") | ("env", "gs_os_write_stderr") => {
            let fd = if fname == "gs_os_write_stdout" { 1 } else { 2 };
            if ty.params.len() == 1 {
                // Packed u64: ptr = low 32 bits, len = high 32 bits
                a.mov_rr(true, Gp::Rsi, Gp::Rdi);
                a.shr_imm(true, Gp::Rsi, 32);
                a.mov_rr(false, Gp::Rdi, Gp::Rdi);
            } else {
                zext_param(&mut a, Gp::Rdi, 0);
                zext_param(&mut a, Gp::Rsi, 1);
            }
            let lbl_oob = a.new_label();
            let lbl_ok = a.new_label();
            emit_check_range(&mut a, Gp::Rdi, Gp::Rsi, fixed_mem, lbl_oob);
            a.test_rr(true, Gp::Rsi, Gp::Rsi);
            a.jcc(Cond::E, lbl_ok);
            a.mov_rr(true, Gp::Rdx, Gp::Rsi);
            a.lea(true, Gp::Rsi, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
            a.mov_ri32(Gp::R8, fd);
            emit_write_all(&mut a, Gp::R8);
            a.bind(lbl_ok);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_clock_ns") | ("env", "bench_clock") => {
            a.sub_ri(true, Gp::Rsp, 24);
            emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
            a.sub_rm(
                true,
                Gp::Rax,
                Mem::base_disp(Gp::R15, OFF_EXEC_CLOCK_ORIGIN),
            );
            a.add_ri(true, Gp::Rsp, 24);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "gs_os_time_ns") => {
            a.sub_ri(true, Gp::Rsp, 24);
            emit_clock_ns(&mut a, CLOCK_REALTIME, 0);
            a.add_ri(true, Gp::Rsp, 24);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "gs_os_arg_count") => {
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_EXEC_ARGC));
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "gs_os_resource_dir") => {
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "system") => {
            a.mov_ri64(Gp::Rax, u64::MAX);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "bench_barrier") => {
            if matches!(ty.params[0], ValType::I32 | ValType::I64) {
                a.mov_rr(ty.params[0] == ValType::I64, Gp::Rax, Gp::Rdi);
            }
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("env", "gs_os_flush_stdout" | "bench_sink" | "emscripten_notify_memory_growth") => {
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_close") => {
            let lbl_bad = a.new_label();
            a.cmp_ri(false, Gp::Rdi, 2);
            a.jcc(Cond::A, lbl_bad);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_bad);
            a.mov_ri32(Gp::Rax, 8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_seek") => {
            let lbl_bad = a.new_label();
            a.cmp_ri(false, Gp::Rdi, 2);
            a.jcc(Cond::A, lbl_bad);
            a.mov_ri32(Gp::Rax, 70);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_bad);
            a.mov_ri32(Gp::Rax, 8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "environ_sizes_get") => {
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rsi, 1);
            let lbl_oob = a.new_label();
            let w: u64 = if is_mem64 { 8 } else { 4 };
            a.mov_ri64(Gp::Rdx, w);
            emit_check_range(&mut a, Gp::Rdi, Gp::Rdx, fixed_mem, lbl_oob);
            emit_check_range(&mut a, Gp::Rsi, Gp::Rdx, fixed_mem, lbl_oob);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0), Gp::Rax);
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rsi, 1, 0), Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::Rax, 21); // EFAULT
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "environ_get") => {
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "args_sizes_get") => {
            // Compute total byte size (including NUL terminators) of guest argv.
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rsi, 1);
            let lbl_oob = a.new_label();
            let w: u64 = if is_mem64 { 8 } else { 4 };
            a.mov_ri64(Gp::Rdx, w);
            emit_check_range(&mut a, Gp::Rdi, Gp::Rdx, fixed_mem, lbl_oob);
            emit_check_range(&mut a, Gp::Rsi, Gp::Rdx, fixed_mem, lbl_oob);
            a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::R15, OFF_EXEC_ARGC));
            a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::R15, OFF_EXEC_ARGV));
            a.xor_rr(false, Gp::R9, Gp::R9); // total bytes
            a.xor_rr(false, Gp::Rdx, Gp::Rdx); // idx
            let lbl_outer = a.new_label();
            let lbl_inner = a.new_label();
            let lbl_done = a.new_label();
            a.bind(lbl_outer);
            a.cmp_rr(true, Gp::Rdx, Gp::Rcx);
            a.jcc(Cond::Ae, lbl_done);
            a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R8, Gp::Rdx, 8, 0));
            a.bind(lbl_inner);
            a.movzx_rm8(false, Gp::R10, Mem::base(Gp::Rax));
            a.add_ri(true, Gp::Rax, 1);
            a.add_ri(true, Gp::R9, 1);
            a.test_rr(false, Gp::R10, Gp::R10);
            a.jcc(Cond::Ne, lbl_inner);
            a.add_ri(true, Gp::Rdx, 1);
            a.jmp(lbl_outer);
            a.bind(lbl_done);
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0), Gp::Rcx);
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rsi, 1, 0), Gp::R9);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::Rax, 21);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "args_get") => {
            zext_param(&mut a, Gp::Rdi, 0); // argv_ptr (guest offset)
            zext_param(&mut a, Gp::Rsi, 1); // buf_ptr  (guest offset)
            a.push(Gp::Rbx);
            a.push(Gp::Rbp);
            let lbl_oob = a.new_label();
            let lbl_outer = a.new_label();
            let lbl_inner = a.new_label();
            let lbl_done = a.new_label();
            let w: i32 = if is_mem64 { 8 } else { 4 };
            a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::R15, OFF_EXEC_ARGC));
            a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::R15, OFF_EXEC_ARGV));
            // Bounds-check argv table: argc * w
            a.mov_rr(true, Gp::Rdx, Gp::Rbx);
            a.shl_imm(true, Gp::Rdx, if is_mem64 { 3 } else { 2 });
            emit_check_range(&mut a, Gp::Rdi, Gp::Rdx, fixed_mem, lbl_oob);
            a.xor_rr(false, Gp::Rcx, Gp::Rcx); // i = 0
            a.bind(lbl_outer);
            a.cmp_rr(true, Gp::Rcx, Gp::Rbx);
            a.jcc(Cond::Ae, lbl_done);
            // Write current guest buf_ptr to argv[i]
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0), Gp::Rsi);
            a.add_ri(true, Gp::Rdi, w);
            a.mov_rm(true, Gp::R8, Mem::indexed(Gp::Rbp, Gp::Rcx, 8, 0)); // host char*
            a.bind(lbl_inner);
            a.mov_ri64(Gp::Rdx, 1);
            emit_check_range(&mut a, Gp::Rsi, Gp::Rdx, fixed_mem, lbl_oob);
            a.movzx_rm8(false, Gp::R9, Mem::base(Gp::R8));
            a.mov_mr8(Mem::indexed(Gp::R14, Gp::Rsi, 1, 0), Gp::R9);
            a.add_ri(true, Gp::R8, 1);
            a.add_ri(true, Gp::Rsi, 1);
            a.test_rr(false, Gp::R9, Gp::R9);
            a.jcc(Cond::Ne, lbl_inner);
            a.add_ri(true, Gp::Rcx, 1);
            a.jmp(lbl_outer);
            a.bind(lbl_done);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.mov_ri32(Gp::Rax, 21);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "clock_time_get") => {
            // (clock_id: Rdi, precision: Rsi, out_ptr: Rdx) -> i32
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rdx, 2);
            let lbl_oob = a.new_label();
            let lbl_einval = a.new_label();
            let lbl_mono = a.new_label();
            let lbl_store = a.new_label();
            a.mov_ri64(Gp::R8, 8);
            emit_check_range(&mut a, Gp::Rdx, Gp::R8, fixed_mem, lbl_oob);
            a.cmp_ri(false, Gp::Rdi, 1);
            a.jcc(Cond::A, lbl_einval);
            a.sub_ri(true, Gp::Rsp, 40);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rdx);
            a.cmp_ri(false, Gp::Rdi, 1);
            a.jcc(Cond::E, lbl_mono);
            emit_clock_ns(&mut a, CLOCK_REALTIME, 0);
            a.jmp(lbl_store);
            a.bind(lbl_mono);
            emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
            a.sub_rm(
                true,
                Gp::Rax,
                Mem::base_disp(Gp::R15, OFF_EXEC_CLOCK_ORIGIN),
            );
            a.bind(lbl_store);
            a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 16));
            a.add_ri(true, Gp::Rsp, 40);
            a.mov_mr(true, Mem::indexed(Gp::R14, Gp::Rdx, 1, 0), Gp::Rax);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_einval);
            a.mov_ri32(Gp::Rax, 28);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
            a.bind(lbl_oob);
            a.mov_ri32(Gp::Rax, 21);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_write") | ("wasi_snapshot_preview1", "fd_read") => {
            let is_write = fname == "fd_write";
            // (fd: Rdi, iovs: Rsi, iovs_len: Rdx, n_out: Rcx) -> i32
            zext_param(&mut a, Gp::Rdi, 0);
            zext_param(&mut a, Gp::Rsi, 1);
            zext_param(&mut a, Gp::Rdx, 2);
            zext_param(&mut a, Gp::Rcx, 3);
            a.push(Gp::Rbx);
            a.push(Gp::Rbp);
            a.sub_ri(true, Gp::Rsp, 40);
            // [rsp+0]=fd, [rsp+8]=iovs_off, [rsp+16]=iovs_len, [rsp+24]=n_out_off, [rsp+32]=total
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::Rdi);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rsi);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rdx);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rcx);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rax);

            let lbl_ebadf = a.new_label();
            let lbl_efault = a.new_label();
            let lbl_loop = a.new_label();
            let lbl_next = a.new_label();
            let lbl_done = a.new_label();

            // Check fd validity: 1..=2 for write, 0 for read.
            if is_write {
                a.cmp_ri(false, Gp::Rdi, 1);
                a.jcc(Cond::B, lbl_ebadf);
                a.cmp_ri(false, Gp::Rdi, 2);
                a.jcc(Cond::A, lbl_ebadf);
            } else {
                a.test_rr(false, Gp::Rdi, Gp::Rdi);
                a.jcc(Cond::Ne, lbl_ebadf);
            }

            let ptr_w: u64 = if is_mem64 { 8 } else { 4 };
            let iov_sz: u64 = ptr_w * 2;
            // Bounds-check n_out slot first.
            a.mov_ri64(Gp::R8, ptr_w);
            emit_check_range(&mut a, Gp::Rcx, Gp::R8, fixed_mem, lbl_efault);

            // Loop over iovs: Rbx = i, Rbp = iovs_len
            a.xor_rr(false, Gp::Rbx, Gp::Rbx);
            a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::Rsp, 16));
            a.bind(lbl_loop);
            a.cmp_rr(true, Gp::Rbx, Gp::Rbp);
            a.jcc(Cond::Ae, lbl_done);
            // iov_off = iovs_off + i * iov_sz
            a.mov_rr(true, Gp::R8, Gp::Rbx);
            a.shl_imm(true, Gp::R8, if is_mem64 { 4 } else { 3 });
            a.add_rm(true, Gp::R8, Mem::base_disp(Gp::Rsp, 8));
            a.jcc(Cond::B, lbl_efault);
            a.mov_ri64(Gp::R9, iov_sz);
            emit_check_range(&mut a, Gp::R8, Gp::R9, fixed_mem, lbl_efault);

            // Load buf_off (Rdi) and buf_len (Rsi)
            a.mov_rm(is_mem64, Gp::Rdi, Mem::indexed(Gp::R14, Gp::R8, 1, 0));
            a.mov_rm(
                is_mem64,
                Gp::Rsi,
                Mem::indexed(Gp::R14, Gp::R8, 1, ptr_w as i32),
            );
            emit_check_range(&mut a, Gp::Rdi, Gp::Rsi, fixed_mem, lbl_efault);
            a.test_rr(true, Gp::Rsi, Gp::Rsi);
            a.jcc(Cond::E, lbl_next);

            a.mov_rr(true, Gp::Rdx, Gp::Rsi);
            a.lea(true, Gp::Rsi, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
            if is_write {
                // Loop on partial writes / -EINTR so multi-iov writes never skip unwritten bytes.
                let lbl_wloop = a.new_label();
                a.bind(lbl_wloop);
                a.test_rr(true, Gp::Rdx, Gp::Rdx);
                a.jcc(Cond::E, lbl_next);
                a.mov_rm(false, Gp::Rdi, Mem::base_disp(Gp::Rsp, 0));
                a.mov_ri32(Gp::Rax, SYS_WRITE);
                a.syscall();
                a.cmp_ri(true, Gp::Rax, -4); // -EINTR
                a.jcc(Cond::E, lbl_wloop);
                a.cmp_ri(true, Gp::Rax, 0);
                a.jcc(Cond::Le, lbl_done);
                a.add_rr(true, Gp::Rsi, Gp::Rax);
                a.sub_rr(true, Gp::Rdx, Gp::Rax);
                a.add_mr_via_reg(Gp::Rsp, 32, Gp::Rax);
                a.jmp(lbl_wloop);
            } else {
                let lbl_rloop = a.new_label();
                a.bind(lbl_rloop);
                a.mov_rm(false, Gp::Rdi, Mem::base_disp(Gp::Rsp, 0));
                a.mov_ri32(Gp::Rax, SYS_READ);
                a.syscall();
                a.cmp_ri(true, Gp::Rax, -4); // -EINTR
                a.jcc(Cond::E, lbl_rloop);
                a.cmp_ri(true, Gp::Rax, 0);
                a.jcc(Cond::L, lbl_done);
                a.add_mr_via_reg(Gp::Rsp, 32, Gp::Rax);
                a.jmp(lbl_done);
            }
            a.bind(lbl_next);
            a.add_ri(true, Gp::Rbx, 1);
            a.jmp(lbl_loop);

            a.bind(lbl_done);
            a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 24));
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 32));
            a.mov_mr(is_mem64, Mem::indexed(Gp::R14, Gp::Rcx, 1, 0), Gp::Rax);
            a.add_ri(true, Gp::Rsp, 40);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.xor_rr(false, Gp::Rax, Gp::Rax);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();

            a.bind(lbl_ebadf);
            a.add_ri(true, Gp::Rsp, 40);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.mov_ri32(Gp::Rax, 8);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();

            a.bind(lbl_efault);
            a.add_ri(true, Gp::Rsp, 40);
            a.pop(Gp::Rbp);
            a.pop(Gp::Rbx);
            a.mov_ri32(Gp::Rax, 21);
            a.xor_rr(false, Gp::R11, Gp::R11);
            a.ret();
        }
        other => {
            return Err(format!(
                "x64: unsupported standalone executable import {}::{}",
                other.0, other.1
            ));
        }
    }

    a.finish_labels()?;
    Ok(a)
}

trait AsmExt {
    fn add_mr_via_reg(&mut self, base: Gp, disp: i32, src: Gp);
}

impl AsmExt for Asm {
    fn add_mr_via_reg(&mut self, base: Gp, disp: i32, src: Gp) {
        self.add_rm(true, src, Mem::base_disp(base, disp));
        self.mov_mr(true, Mem::base_disp(base, disp), src);
    }
}

/// 1024-bit unsigned integer used strictly at AOT compile time to build Ryū's exact
/// `POW5_SPLIT` and `POW5_INV_SPLIT` 128-bit lookup tables in zero-dependency safe Rust.
#[derive(Clone, Copy)]
struct U1024([u64; 16]);

impl U1024 {
    const ZERO: Self = Self([0; 16]);
    const ONE: Self = Self([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    fn mul_small(&mut self, k: u64) {
        let mut carry = 0u128;
        for limb in &mut self.0 {
            let v = u128::from(*limb) * u128::from(k) + carry;
            *limb = v as u64;
            carry = v >> 64;
        }
    }

    fn bit_len(&self) -> u32 {
        for i in (0..16).rev() {
            if self.0[i] != 0 {
                return (i as u32) * 64 + (64 - self.0[i].leading_zeros());
            }
        }
        0
    }

    fn ge(&self, other: &Self) -> bool {
        for i in (0..16).rev() {
            if self.0[i] != other.0[i] {
                return self.0[i] > other.0[i];
            }
        }
        true
    }

    fn sub_assign(&mut self, other: &Self) {
        let mut borrow = 0u128;
        for i in 0..16 {
            let a = u128::from(self.0[i]);
            let b = u128::from(other.0[i]) + borrow;
            if a >= b {
                self.0[i] = (a - b) as u64;
                borrow = 0;
            } else {
                self.0[i] = ((1u128 << 64) + a - b) as u64;
                borrow = 1;
            }
        }
    }

    fn shl1_with_bit(&mut self, bit: u64) {
        let mut carry = bit & 1;
        for limb in &mut self.0 {
            let next = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
    }

    fn low128_shr(&self, shift: u32) -> (u64, u64) {
        let word = (shift / 64) as usize;
        let bit = shift % 64;
        let get = |idx: usize| -> u64 { self.0.get(idx).copied().unwrap_or(0) };
        if bit == 0 {
            (get(word), get(word + 1))
        } else {
            let lo = (get(word) >> bit) | (get(word + 1) << (64 - bit));
            let hi = (get(word + 1) >> bit) | (get(word + 2) << (64 - bit));
            (lo, hi)
        }
    }

    fn low128_shl(&self, shift: u32) -> (u64, u64) {
        let v = u128::from(self.0[0]) | (u128::from(self.0[1]) << 64);
        let shifted = v << shift;
        (shifted as u64, (shifted >> 64) as u64)
    }
}

pub(crate) fn build_ryu_pow5_tables() -> (Vec<u8>, Vec<u8>) {
    const DOUBLE_POW5_BITCOUNT: u32 = 125;
    const INV_COUNT: usize = 292;
    const POW_COUNT: usize = 326;
    let mut inv_bytes = Vec::with_capacity(INV_COUNT * 16);
    let mut pow_bytes = Vec::with_capacity(POW_COUNT * 16);
    let mut p5 = U1024::ONE;
    for q in 0..POW_COUNT {
        let bits = if q == 0 { 1 } else { p5.bit_len() };
        // POW5_SPLIT[q] = floor(5^q / 2^(bits - 125))
        let (plo, phi) = if bits >= DOUBLE_POW5_BITCOUNT {
            p5.low128_shr(bits - DOUBLE_POW5_BITCOUNT)
        } else {
            p5.low128_shl(DOUBLE_POW5_BITCOUNT - bits)
        };
        pow_bytes.extend_from_slice(&plo.to_le_bytes());
        pow_bytes.extend_from_slice(&phi.to_le_bytes());

        // POW5_INV_SPLIT[q] = floor(2^(bits - 1 + 125) / 5^q) + 1
        if q < INV_COUNT {
            let shift = bits - 1 + DOUBLE_POW5_BITCOUNT;
            let mut rem = U1024::ZERO;
            let mut quot = 0u128;
            for b in (0..=shift).rev() {
                rem.shl1_with_bit(u64::from(b == shift));
                if rem.ge(&p5) {
                    rem.sub_assign(&p5);
                    if b < 128 {
                        quot |= 1u128 << b;
                    }
                }
            }
            let val = quot + 1;
            inv_bytes.extend_from_slice(&(val as u64).to_le_bytes());
            inv_bytes.extend_from_slice(&((val >> 64) as u64).to_le_bytes());
        }
        p5.mul_small(5);
    }
    (inv_bytes, pow_bytes)
}

/// Emits `HELPER_GOOSE_FMT_FLOAT`: exact Ryū shortest round-trip `f64`/`f32` formatter with
/// Goose's layout (`nan`, `inf`, `-inf`, `0.0`, `-0.0`, fixed vs scientific).
///
/// ABI:
/// - Input: `Rdi = out_ptr (char[32])`, `Xmm0 = value`, `Esi = is_f64 (1 = f64, 0 = f32)`
/// - Output: `Rax = byte_len (1..=32)` written to `[Rdi..Rdi + Rax]`
/// - Preserves `Rbx, Rbp, R12..R15, Xmm8..Xmm13`.
pub fn emit_helper_goose_fmt_float() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(1536);
    let (inv_bytes, pow_bytes) = build_ryu_pow5_tables();
    let inv_tbl_off = a.add_local_rodata(16, &inv_bytes);
    let pow_tbl_off = a.add_local_rodata(16, &pow_bytes);

    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.sub_ri(true, Gp::Rsp, 120);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 0), Gp::Rdi); // [rsp+0] = out_ptr

    // Extract sign -> [rsp+8], biased exp -> R8, mantissa -> R9, exp_max -> R10, bias_adj -> R11, imp_bit -> Rcx
    let lbl_f32 = a.new_label();
    let lbl_unpacked = a.new_label();
    a.test_rr(false, Gp::Rsi, Gp::Rsi);
    a.jcc(Cond::E, lbl_f32);
    // f64 unpack
    a.movq_rx(Gp::Rax, Xmm::Xmm0);
    a.mov_rr(true, Gp::Rdx, Gp::Rax);
    a.shr_imm(true, Gp::Rdx, 63);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rdx);
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.shr_imm(true, Gp::R8, 52);
    a.and_ri(false, Gp::R8, 0x7ff);
    a.mov_ri64(Gp::R9, (1u64 << 52) - 1);
    a.and_rr(true, Gp::R9, Gp::Rax);
    a.mov_ri32(Gp::R10, 0x7ff);
    a.mov_ri32(Gp::R11, 1077); // 1023 + 52 + 2
    a.mov_ri64(Gp::Rcx, 1u64 << 52);
    a.jmp(lbl_unpacked);

    // f32 unpack
    a.bind(lbl_f32);
    a.movd_rx(Gp::Rax, Xmm::Xmm0);
    a.mov_rr(false, Gp::Rdx, Gp::Rax);
    a.shr_imm(false, Gp::Rdx, 31);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 8), Gp::Rdx);
    a.mov_rr(false, Gp::R8, Gp::Rax);
    a.shr_imm(false, Gp::R8, 23);
    a.and_ri(false, Gp::R8, 0xff);
    a.mov_rr(false, Gp::R9, Gp::Rax);
    a.and_ri(false, Gp::R9, 0x7f_ffff);
    a.mov_ri32(Gp::R10, 0xff);
    a.mov_ri32(Gp::R11, 152); // 127 + 23 + 2
    a.mov_ri64(Gp::Rcx, 1u64 << 23);

    a.bind(lbl_unpacked);
    let lbl_ret = a.new_label();
    let lbl_not_special_max = a.new_label();
    let lbl_not_zero = a.new_label();

    // Check exp == exp_max (NaN or ±inf)
    a.cmp_rr(true, Gp::R8, Gp::R10);
    a.jcc(Cond::Ne, lbl_not_special_max);
    let lbl_inf = a.new_label();
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_inf);
    // "nan"
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'n');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'a');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 2), b'n');
    a.mov_ri32(Gp::Rax, 3);
    a.jmp(lbl_ret);
    a.bind(lbl_inf);
    let lbl_pos_inf = a.new_label();
    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_pos_inf);
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'-');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'i');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 2), b'n');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 3), b'f');
    a.mov_ri32(Gp::Rax, 4);
    a.jmp(lbl_ret);
    a.bind(lbl_pos_inf);
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'i');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'n');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 2), b'f');
    a.mov_ri32(Gp::Rax, 3);
    a.jmp(lbl_ret);

    a.bind(lbl_not_special_max);
    // Check ±0.0 (`exp == 0 && mant == 0`)
    a.mov_rr(true, Gp::Rax, Gp::R8);
    a.or_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::Ne, lbl_not_zero);
    let lbl_pos_zero = a.new_label();
    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_pos_zero);
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'-');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'0');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 2), b'.');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 3), b'0');
    a.mov_ri32(Gp::Rax, 4);
    a.jmp(lbl_ret);
    a.bind(lbl_pos_zero);
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'0');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'.');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 2), b'0');
    a.mov_ri32(Gp::Rax, 3);
    a.jmp(lbl_ret);

    // Finite non-zero: compute m2 (Rax), e2 (Rbx), flags (Rbp: bit0=accept_bounds, bit3=mm_shift)
    a.bind(lbl_not_zero);
    let lbl_subnorm = a.new_label();
    let lbl_got_m2 = a.new_label();
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, lbl_subnorm);
    // Normal: m2 = mant | imp_bit; e2 = exp - bias_adj; mm_shift = (mant != 0 || exp <= 1)
    a.mov_rr(true, Gp::Rax, Gp::R9);
    a.or_rr(true, Gp::Rax, Gp::Rcx);
    a.mov_rr(true, Gp::Rbx, Gp::R8);
    a.sub_rr(true, Gp::Rbx, Gp::R11);
    a.xor_rr(false, Gp::Rbp, Gp::Rbp);
    let lbl_mm1 = a.new_label();
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::Ne, lbl_mm1);
    a.cmp_ri(true, Gp::R8, 1);
    a.jcc(Cond::G, lbl_got_m2);
    a.bind(lbl_mm1);
    a.or_ri(false, Gp::Rbp, 8); // bit 3 = mm_shift = 1
    a.jmp(lbl_got_m2);

    a.bind(lbl_subnorm);
    a.mov_rr(true, Gp::Rax, Gp::R9); // m2 = mant
    a.mov_ri64(Gp::Rbx, 1);
    a.sub_rr(true, Gp::Rbx, Gp::R11); // e2 = 1 - bias_adj
    a.mov_ri32(Gp::Rbp, 8); // subnormal (exp == 0 <= 1) always has mm_shift = 1

    a.bind(lbl_got_m2);
    // accept_bounds = ((m2 & 1) == 0) -> bit 0 of Rbp
    let lbl_odd = a.new_label();
    a.test_ri(false, Gp::Rax, 1);
    a.jcc(Cond::Ne, lbl_odd);
    a.or_ri(false, Gp::Rbp, 1);
    a.bind(lbl_odd);
    // mv = 4*m2, mp = mv + 2, mm = mv - 1 - mm_shift
    a.shl_imm(true, Gp::Rax, 2); // Rax = mv
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 16), Gp::Rax);
    a.lea(true, Gp::Rdx, Mem::base_disp(Gp::Rax, 2));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 24), Gp::Rdx);
    a.sub_ri(true, Gp::Rax, 1);
    let lbl_no_mm_shift = a.new_label();
    a.test_ri(false, Gp::Rbp, 8);
    a.jcc(Cond::E, lbl_no_mm_shift);
    a.sub_ri(true, Gp::Rax, 1);
    a.bind(lbl_no_mm_shift);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 32), Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rbp);

    // Inline helper: `mulShift64` on `[rsp+16]`, `[rsp+24]`, `[rsp+32]` using R8=mul_lo, R9=mul_hi, Cl=shift-64.
    let emit_mul_shift_all3 = |a: &mut Asm| {
        for slot in [16i32, 24, 32] {
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, slot));
            a.mov_rr(true, Gp::R10, Gp::Rax);
            a.mul_r(true, Gp::R8);
            a.mov_rr(true, Gp::R11, Gp::Rdx);
            a.mov_rr(true, Gp::Rax, Gp::R10);
            a.mul_r(true, Gp::R9);
            a.add_rr(true, Gp::Rax, Gp::R11);
            a.adc_ri8(true, Gp::Rdx, 0);
            a.shrd_cl(true, Gp::Rax, Gp::Rdx);
            a.mov_mr(true, Mem::base_disp(Gp::Rsp, slot), Gp::Rax);
        }
    };

    // Inline helper: check if `val` in Rax is divisible by `5^q` (q in R12); sets Eax = 1 iff divisible.
    let emit_div_pow5 = |a: &mut Asm| {
        let lp = a.new_label();
        let no = a.new_label();
        let yes = a.new_label();
        let end = a.new_label();
        a.mov_rr(true, Gp::Rcx, Gp::R12);
        a.mov_ri32(Gp::R11, 5);
        a.bind(lp);
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::E, yes);
        a.xor_rr(false, Gp::Rdx, Gp::Rdx);
        a.div_r(true, Gp::R11);
        a.test_rr(true, Gp::Rdx, Gp::Rdx);
        a.jcc(Cond::Ne, no);
        a.sub_ri(true, Gp::Rcx, 1);
        a.jmp(lp);
        a.bind(yes);
        a.mov_ri32(Gp::Rax, 1);
        a.jmp(end);
        a.bind(no);
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.bind(end);
    };

    let lbl_e2_neg = a.new_label();
    let lbl_after_mul = a.new_label();
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::L, lbl_e2_neg);

    // ---- Case e2 >= 0 ----
    // q = (e2 * 78913) >> 18; if e2 > 3 && q > 0 { q -= 1 }
    a.imul_rri(true, Gp::R12, Gp::Rbx, 78913);
    a.shr_imm(true, Gp::R12, 18);
    let lbl_no_dec_q1 = a.new_label();
    a.cmp_ri(true, Gp::Rbx, 3);
    a.jcc(Cond::Le, lbl_no_dec_q1);
    a.test_rr(true, Gp::R12, Gp::R12);
    a.jcc(Cond::E, lbl_no_dec_q1);
    a.sub_ri(true, Gp::R12, 1);
    a.bind(lbl_no_dec_q1);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 40), Gp::R12); // e10 = q

    // pow5bits(q) -> Rax
    let lbl_p5b1_z = a.new_label();
    let lbl_p5b1_done = a.new_label();
    a.test_rr(true, Gp::R12, Gp::R12);
    a.jcc(Cond::E, lbl_p5b1_z);
    a.imul_rri(true, Gp::Rax, Gp::R12, 1_217_359);
    a.shr_imm(true, Gp::Rax, 19);
    a.add_ri(true, Gp::Rax, 1);
    a.jmp(lbl_p5b1_done);
    a.bind(lbl_p5b1_z);
    a.mov_ri32(Gp::Rax, 1);
    a.bind(lbl_p5b1_done);
    // shift - 64 = -e2 + q + 124 + p5b - 64 = q + p5b + 60 - e2
    a.add_rr(true, Gp::Rax, Gp::R12);
    a.add_ri(true, Gp::Rax, 60);
    a.sub_rr(true, Gp::Rax, Gp::Rbx);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rax);

    // Trailing-zero check when q <= 21 (BEFORE overwriting mv/mp/mm!)
    let lbl_pos_skip_tz = a.new_label();
    let lbl_pos_q_nz = a.new_label();
    a.cmp_ri(true, Gp::R12, 21);
    a.jcc(Cond::G, lbl_pos_skip_tz);
    a.test_rr(true, Gp::R12, Gp::R12);
    a.jcc(Cond::Ne, lbl_pos_q_nz);
    // q == 0: 5^0 = 1 divides mv, mm, and mp unconditionally!
    a.or_ri(false, Gp::Rbp, 2); // vr_tz = true
    let lbl_pos_q0_no_ab = a.new_label();
    a.test_ri(false, Gp::Rbp, 1);
    a.jcc(Cond::E, lbl_pos_q0_no_ab);
    a.or_ri(false, Gp::Rbp, 4); // vm_tz = true
    a.jmp(lbl_pos_skip_tz);
    a.bind(lbl_pos_q0_no_ab);
    a.or_ri(false, Gp::Rbp, 16); // dec_vp = true
    a.jmp(lbl_pos_skip_tz);

    a.bind(lbl_pos_q_nz);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16)); // mv
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.mov_ri32(Gp::R11, 5);
    a.div_r(true, Gp::R11);
    let lbl_mv_not5 = a.new_label();
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_mv_not5);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16));
    emit_div_pow5(&mut a);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_pos_skip_tz);
    a.or_ri(false, Gp::Rbp, 2); // vr_tz
    a.jmp(lbl_pos_skip_tz);

    a.bind(lbl_mv_not5);
    let lbl_pos_no_ab = a.new_label();
    a.test_ri(false, Gp::Rbp, 1); // accept_bounds?
    a.jcc(Cond::E, lbl_pos_no_ab);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 32)); // mm
    emit_div_pow5(&mut a);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_pos_skip_tz);
    a.or_ri(false, Gp::Rbp, 4); // vm_tz
    a.jmp(lbl_pos_skip_tz);

    a.bind(lbl_pos_no_ab);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 24)); // mp
    emit_div_pow5(&mut a);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_pos_skip_tz);
    a.or_ri(false, Gp::Rbp, 16); // dec_vp

    a.bind(lbl_pos_skip_tz);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rbp);
    // Load POW5_INV_SPLIT[q] into (R8, R9)
    a.lea_rip(Gp::R13, RelocTarget::LocalRodata(inv_tbl_off));
    a.shl_imm(true, Gp::R12, 4);
    a.mov_rm(true, Gp::R8, Mem::indexed(Gp::R13, Gp::R12, 1, 0));
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R13, Gp::R12, 1, 8));
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 56));
    emit_mul_shift_all3(&mut a);
    a.jmp(lbl_after_mul);

    // ---- Case e2 < 0 ----
    a.bind(lbl_e2_neg);
    a.neg(true, Gp::Rbx); // Rbx = -e2 > 0
    a.imul_rri(true, Gp::R12, Gp::Rbx, 732_923);
    a.shr_imm(true, Gp::R12, 20);
    let lbl_no_dec_q2 = a.new_label();
    a.cmp_ri(true, Gp::Rbx, 1);
    a.jcc(Cond::Le, lbl_no_dec_q2);
    a.test_rr(true, Gp::R12, Gp::R12);
    a.jcc(Cond::E, lbl_no_dec_q2);
    a.sub_ri(true, Gp::R12, 1);
    a.bind(lbl_no_dec_q2);
    // e10 = q + e2 = q - Rbx
    a.mov_rr(true, Gp::Rax, Gp::R12);
    a.sub_rr(true, Gp::Rax, Gp::Rbx);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 40), Gp::Rax);
    // idx = -e2 - q = Rbx - R12 -> R13
    a.mov_rr(true, Gp::R13, Gp::Rbx);
    a.sub_rr(true, Gp::R13, Gp::R12);

    // pow5bits(idx) -> Rax
    let lbl_p5b2_z = a.new_label();
    let lbl_p5b2_done = a.new_label();
    a.test_rr(true, Gp::R13, Gp::R13);
    a.jcc(Cond::E, lbl_p5b2_z);
    a.imul_rri(true, Gp::Rax, Gp::R13, 1_217_359);
    a.shr_imm(true, Gp::Rax, 19);
    a.add_ri(true, Gp::Rax, 1);
    a.jmp(lbl_p5b2_done);
    a.bind(lbl_p5b2_z);
    a.mov_ri32(Gp::Rax, 1);
    a.bind(lbl_p5b2_done);
    // shift - 64 = q - (p5b - 125) - 64 = q + 61 - p5b
    a.mov_rr(true, Gp::Rdx, Gp::R12);
    a.add_ri(true, Gp::Rdx, 61);
    a.sub_rr(true, Gp::Rdx, Gp::Rax);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 56), Gp::Rdx);

    // Trailing-zero check on mv (BEFORE mul_shift)
    let lbl_neg_q_gt1 = a.new_label();
    let lbl_neg_tz_done = a.new_label();
    a.cmp_ri(true, Gp::R12, 1);
    a.jcc(Cond::G, lbl_neg_q_gt1);
    a.or_ri(false, Gp::Rbp, 2); // vr_tz = true
    let lbl_neg_no_ab = a.new_label();
    a.test_ri(false, Gp::Rbp, 1);
    a.jcc(Cond::E, lbl_neg_no_ab);
    // q == 0: 2^0 = 1 divides mm unconditionally; q == 1: 2 | mm iff mm_shift == 1.
    let lbl_neg_set_vm_tz = a.new_label();
    a.test_rr(true, Gp::R12, Gp::R12);
    a.jcc(Cond::E, lbl_neg_set_vm_tz);
    a.test_ri(false, Gp::Rbp, 8); // mm_shift == 1?
    a.jcc(Cond::E, lbl_neg_tz_done);
    a.bind(lbl_neg_set_vm_tz);
    a.or_ri(false, Gp::Rbp, 4); // vm_tz = true
    a.jmp(lbl_neg_tz_done);
    a.bind(lbl_neg_no_ab);
    a.or_ri(false, Gp::Rbp, 16); // dec_vp
    a.jmp(lbl_neg_tz_done);

    a.bind(lbl_neg_q_gt1);
    a.cmp_ri(true, Gp::R12, 63);
    a.jcc(Cond::Ge, lbl_neg_tz_done);
    // All `q` shifted-out decimal digits are 0 iff `2^q | mv` (since `-e2 >= q`).
    a.mov_rr(true, Gp::Rcx, Gp::R12);
    a.mov_ri64(Gp::Rax, 1);
    a.shl_cl(true, Gp::Rax);
    a.sub_ri(true, Gp::Rax, 1);
    a.and_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 16)); // mv & ((1 << q) - 1)
    a.jcc(Cond::Ne, lbl_neg_tz_done);
    a.or_ri(false, Gp::Rbp, 2); // vr_tz = true

    a.bind(lbl_neg_tz_done);
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, 48), Gp::Rbp);
    // Load POW5_SPLIT[idx] into (R8, R9)
    a.lea_rip(Gp::R10, RelocTarget::LocalRodata(pow_tbl_off));
    a.shl_imm(true, Gp::R13, 4);
    a.mov_rm(true, Gp::R8, Mem::indexed(Gp::R10, Gp::R13, 1, 0));
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::R13, 1, 8));
    a.mov_rm(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, 56));
    emit_mul_shift_all3(&mut a);

    // ---- Common post-mul digit reduction ----
    a.bind(lbl_after_mul);
    a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::Rsp, 48));
    a.mov_rm(true, Gp::R12, Mem::base_disp(Gp::Rsp, 16)); // R12 = vr
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::Rsp, 24)); // R13 = vp
    a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 32)); // Rbx = vm
    let lbl_no_dec_vp = a.new_label();
    a.test_ri(false, Gp::Rbp, 16);
    a.jcc(Cond::E, lbl_no_dec_vp);
    a.sub_ri(true, Gp::R13, 1);
    a.bind(lbl_no_dec_vp);

    a.xor_rr(false, Gp::R8, Gp::R8); // R8 = removed = 0
    a.xor_rr(false, Gp::R9, Gp::R9); // R9 = last_removed = 0
    a.mov_ri32(Gp::R10, 10);

    let lbl_fast_red = a.new_label();
    let lbl_got_output = a.new_label();
    a.test_ri(false, Gp::Rbp, 6); // vr_tz || vm_tz?
    a.jcc(Cond::E, lbl_fast_red);

    // Slow/exact trailing-zero reduction loop
    let lbl_tz_lp = a.new_label();
    let lbl_tz_lp_done = a.new_label();
    a.bind(lbl_tz_lp);
    a.mov_rr(true, Gp::Rax, Gp::R13);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::Rdi, Gp::Rax); // Rdi = vp / 10
    a.mov_rr(true, Gp::Rax, Gp::Rbx);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::Rsi, Gp::Rax); // Rsi = vm / 10
    a.mov_rr(true, Gp::R11, Gp::Rdx); // R11 = vm % 10
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::Be, lbl_tz_lp_done);
    let lbl_tz_k1 = a.new_label();
    a.test_rr(true, Gp::R11, Gp::R11);
    a.jcc(Cond::E, lbl_tz_k1);
    a.and_ri(false, Gp::Rbp, !4); // clear vm_tz
    a.bind(lbl_tz_k1);
    let lbl_tz_k2 = a.new_label();
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_tz_k2);
    a.and_ri(false, Gp::Rbp, !2); // clear vr_tz
    a.bind(lbl_tz_k2);
    a.mov_rr(true, Gp::Rax, Gp::R12);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::R12, Gp::Rax);
    a.mov_rr(true, Gp::R9, Gp::Rdx);
    a.mov_rr(true, Gp::R13, Gp::Rdi);
    a.mov_rr(true, Gp::Rbx, Gp::Rsi);
    a.add_ri(true, Gp::R8, 1);
    a.jmp(lbl_tz_lp);

    a.bind(lbl_tz_lp_done);
    let lbl_vm_tz_done = a.new_label();
    a.test_ri(false, Gp::Rbp, 4); // vm_tz?
    a.jcc(Cond::E, lbl_vm_tz_done);
    let lbl_vm_lp = a.new_label();
    a.bind(lbl_vm_lp);
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::E, lbl_vm_tz_done);
    a.mov_rr(true, Gp::Rax, Gp::Rbx);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::Ne, lbl_vm_tz_done);
    a.mov_rr(true, Gp::Rbx, Gp::Rax);
    let lbl_vm_k = a.new_label();
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::E, lbl_vm_k);
    a.and_ri(false, Gp::Rbp, !2);
    a.bind(lbl_vm_k);
    a.mov_rr(true, Gp::Rax, Gp::R12);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::R12, Gp::Rax);
    a.mov_rr(true, Gp::R9, Gp::Rdx);
    a.add_ri(true, Gp::R8, 1);
    a.jmp(lbl_vm_lp);

    a.bind(lbl_vm_tz_done);
    // if vr_tz && last_removed == 5 && (vr & 1) == 0 { last_removed = 4 }
    let lbl_no_tie_even = a.new_label();
    a.test_ri(false, Gp::Rbp, 2);
    a.jcc(Cond::E, lbl_no_tie_even);
    a.cmp_ri(true, Gp::R9, 5);
    a.jcc(Cond::Ne, lbl_no_tie_even);
    a.test_ri(false, Gp::R12, 1);
    a.jcc(Cond::Ne, lbl_no_tie_even);
    a.mov_ri32(Gp::R9, 4);
    a.bind(lbl_no_tie_even);

    // round_up = (vr == vm && (!accept_bounds || !vm_tz)) || last_removed >= 5
    a.mov_rr(true, Gp::Rax, Gp::R12);
    let lbl_tz_inc = a.new_label();
    a.cmp_ri(true, Gp::R9, 5);
    a.jcc(Cond::Ae, lbl_tz_inc);
    a.cmp_rr(true, Gp::R12, Gp::Rbx);
    a.jcc(Cond::Ne, lbl_got_output);
    // vr == vm: round up unless BOTH accept_bounds (1) AND vm_tz (4) are set (mask == 5).
    a.mov_rr(false, Gp::Rcx, Gp::Rbp);
    a.and_ri(false, Gp::Rcx, 5);
    a.cmp_ri(false, Gp::Rcx, 5);
    a.jcc(Cond::E, lbl_got_output);
    a.bind(lbl_tz_inc);
    a.add_ri(true, Gp::Rax, 1);
    a.jmp(lbl_got_output);

    // Fast reduction loop (!vr_tz && !vm_tz)
    a.bind(lbl_fast_red);
    let lbl_f_lp = a.new_label();
    let lbl_f_done = a.new_label();
    a.bind(lbl_f_lp);
    a.mov_rr(true, Gp::Rax, Gp::R13);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::Rdi, Gp::Rax);
    a.mov_rr(true, Gp::Rax, Gp::Rbx);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::Rsi, Gp::Rax);
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::Be, lbl_f_done);
    a.mov_rr(true, Gp::Rax, Gp::R12);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.div_r(true, Gp::R10);
    a.mov_rr(true, Gp::R12, Gp::Rax);
    a.mov_rr(true, Gp::R9, Gp::Rdx);
    a.mov_rr(true, Gp::R13, Gp::Rdi);
    a.mov_rr(true, Gp::Rbx, Gp::Rsi);
    a.add_ri(true, Gp::R8, 1);
    a.jmp(lbl_f_lp);
    a.bind(lbl_f_done);
    a.mov_rr(true, Gp::Rax, Gp::R12);
    let lbl_f_inc = a.new_label();
    a.cmp_ri(true, Gp::R9, 5);
    a.jcc(Cond::Ae, lbl_f_inc);
    a.cmp_rr(true, Gp::R12, Gp::Rbx);
    a.jcc(Cond::Ne, lbl_got_output);
    a.bind(lbl_f_inc);
    a.add_ri(true, Gp::Rax, 1);

    // ---- Format `output` (Rax) + Goose layout ----
    a.bind(lbl_got_output);
    // exp10 = [rsp+40] + R8 (removed)
    a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::Rsp, 40));
    a.add_rr(true, Gp::Rbx, Gp::R8); // Rbx = exp10
    // Format Rax to decimal right-to-left ending at [rsp+96] -> Rsi = start, Rdx = orig_len
    emit_fmt_u64_to_buf(&mut a, Gp::Rax, 96);
    // first_exp = exp10 + orig_len - 1 -> Rbx
    a.add_rr(true, Gp::Rbx, Gp::Rdx);
    a.sub_ri(true, Gp::Rbx, 1);
    // Trim trailing '0's from digits[Rsi..Rsi+Rdx] (count -> Rcx)
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    let lbl_trim = a.new_label();
    let lbl_trim_done = a.new_label();
    a.bind(lbl_trim);
    a.cmp_ri(true, Gp::Rcx, 1);
    a.jcc(Cond::Le, lbl_trim_done);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, -1));
    a.cmp_ri(false, Gp::Rax, b'0' as i32);
    a.jcc(Cond::Ne, lbl_trim_done);
    a.sub_ri(true, Gp::Rcx, 1);
    a.jmp(lbl_trim);
    a.bind(lbl_trim_done);

    // Prepare Goose layout writer: Rdi = out_cur, R12 = orig_out_ptr, Rsi = digits, Rcx = count, Rbx = first_exp
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rsp, 0));
    a.mov_rr(true, Gp::R12, Gp::Rdi);
    let lbl_no_minus = a.new_label();
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rsp, 8));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_no_minus);
    a.mov_mi8(Mem::base(Gp::Rdi), b'-');
    a.add_ri(true, Gp::Rdi, 1);
    a.bind(lbl_no_minus);

    // Inline copy `cnt` bytes from `[src]` to `[Rdi]`, advancing both `src` and `Rdi`.
    let emit_copy_n = |a: &mut Asm, src: Gp, cnt: Gp| {
        let lp = a.new_label();
        let dn = a.new_label();
        a.bind(lp);
        a.test_rr(true, cnt, cnt);
        a.jcc(Cond::Le, dn);
        a.movzx_rm8(false, Gp::Rax, Mem::base(src));
        a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rax);
        a.add_ri(true, src, 1);
        a.add_ri(true, Gp::Rdi, 1);
        a.sub_ri(true, cnt, 1);
        a.jmp(lp);
        a.bind(dn);
    };
    let emit_zeros_n = |a: &mut Asm, cnt: Gp| {
        let lp = a.new_label();
        let dn = a.new_label();
        a.bind(lp);
        a.test_rr(true, cnt, cnt);
        a.jcc(Cond::Le, dn);
        a.mov_mi8(Mem::base(Gp::Rdi), b'0');
        a.add_ri(true, Gp::Rdi, 1);
        a.sub_ri(true, cnt, 1);
        a.jmp(lp);
        a.bind(dn);
    };

    // max_exp = max(count, 15) -> R8
    a.mov_ri64(Gp::R8, 15);
    a.cmp_rr(true, Gp::Rcx, Gp::R8);
    a.cmovcc(Cond::G, true, Gp::R8, Gp::Rcx);

    let lbl_sci = a.new_label();
    let lbl_not_sci = a.new_label();
    let lbl_layout_done = a.new_label();
    a.cmp_ri(true, Gp::Rbx, -4);
    a.jcc(Cond::L, lbl_sci);
    a.cmp_rr(true, Gp::Rbx, Gp::R8);
    a.jcc(Cond::L, lbl_not_sci);

    // Scientific: d[.ddd]e[+-]dd[d]
    a.bind(lbl_sci);
    a.movzx_rm8(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rax);
    a.add_ri(true, Gp::Rsi, 1);
    a.add_ri(true, Gp::Rdi, 1);
    a.sub_ri(true, Gp::Rcx, 1);
    let lbl_sci_e = a.new_label();
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::Le, lbl_sci_e);
    a.mov_mi8(Mem::base(Gp::Rdi), b'.');
    a.add_ri(true, Gp::Rdi, 1);
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rcx);
    a.bind(lbl_sci_e);
    a.mov_mi8(Mem::base(Gp::Rdi), b'e');
    a.add_ri(true, Gp::Rdi, 1);
    let lbl_epos = a.new_label();
    let lbl_edig = a.new_label();
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::Ge, lbl_epos);
    a.mov_mi8(Mem::base(Gp::Rdi), b'-');
    a.add_ri(true, Gp::Rdi, 1);
    a.neg(true, Gp::Rbx);
    a.jmp(lbl_edig);
    a.bind(lbl_epos);
    a.mov_mi8(Mem::base(Gp::Rdi), b'+');
    a.add_ri(true, Gp::Rdi, 1);
    a.bind(lbl_edig);
    let lbl_e_ge10 = a.new_label();
    a.cmp_ri(true, Gp::Rbx, 10);
    a.jcc(Cond::Ae, lbl_e_ge10);
    a.mov_mi8(Mem::base(Gp::Rdi), b'0');
    a.add_ri(true, Gp::Rbx, b'0' as i32);
    a.mov_mr8(Mem::base_disp(Gp::Rdi, 1), Gp::Rbx);
    a.add_ri(true, Gp::Rdi, 2);
    a.jmp(lbl_layout_done);
    a.bind(lbl_e_ge10);
    emit_fmt_u64_to_buf(&mut a, Gp::Rbx, 112); // Rsi = ptr, Rdx = len
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rdx);
    a.jmp(lbl_layout_done);

    // Fixed layouts
    a.bind(lbl_not_sci);
    let lbl_ge0 = a.new_label();
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::Ge, lbl_ge0);
    // -4 <= first_exp < 0: "0." + (-first_exp - 1)*'0' + digits
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'0');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'.');
    a.add_ri(true, Gp::Rdi, 2);
    a.neg(true, Gp::Rbx);
    a.sub_ri(true, Gp::Rbx, 1);
    emit_zeros_n(&mut a, Gp::Rbx);
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rcx);
    a.jmp(lbl_layout_done);

    a.bind(lbl_ge0);
    // whole = first_exp + 1 -> Rbx
    a.add_ri(true, Gp::Rbx, 1);
    let lbl_split = a.new_label();
    a.cmp_rr(true, Gp::Rcx, Gp::Rbx);
    a.jcc(Cond::G, lbl_split);
    // count <= whole: digits + (whole - count)*'0' + ".0"
    a.sub_rr(true, Gp::Rbx, Gp::Rcx); // trailing zeros
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rcx);
    emit_zeros_n(&mut a, Gp::Rbx);
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 0), b'.');
    a.mov_mi8(Mem::base_disp(Gp::Rdi, 1), b'0');
    a.add_ri(true, Gp::Rdi, 2);
    a.jmp(lbl_layout_done);

    // 0 < whole < count: digits[..whole] + '.' + digits[whole..]
    a.bind(lbl_split);
    a.sub_rr(true, Gp::Rcx, Gp::Rbx); // frac_len = count - whole
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rbx);
    a.mov_mi8(Mem::base(Gp::Rdi), b'.');
    a.add_ri(true, Gp::Rdi, 1);
    emit_copy_n(&mut a, Gp::Rsi, Gp::Rcx);

    a.bind(lbl_layout_done);
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.sub_rr(true, Gp::Rax, Gp::R12);

    a.bind(lbl_ret);
    a.add_ri(true, Gp::Rsp, 120);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    a.finish_labels()?;
    Ok(a)
}
