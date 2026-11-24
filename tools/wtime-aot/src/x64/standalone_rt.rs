//! Self-contained Linux x86-64 static executable (`ET_EXEC`) runtime & direct-syscall
//! WASI / Goose host import trampolines for `--backend x64` standalone binaries.

use wtime_types::ValType;

use super::asm::{Asm, Cond, Gp, Mem, RelocTarget};
use super::lower::TargetIsa;
use super::module::X64Module;
use super::runtime::{RuntimeTables, RuntimeTestHooks};
use super::{
    CtxLayout, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES, GUEST_STACK_TOTAL_BYTES,
    GUEST_STACK_USABLE_BYTES, HELPER_INITIALIZE, OFF_EXEC_ARGC, OFF_EXEC_ARGV,
    OFF_EXEC_CLOCK_ORIGIN, OFF_EXEC_EXIT_CODE, OFF_EXEC_MEM_LIMIT, OFF_EXEC_TIMING_PATH,
    OFF_GUEST_STACK_BASE, OFF_GUEST_STACK_TOP, OFF_HANDLE_ALLOC_BYTES, OFF_MEM_BASE, OFF_MEM_LEN,
    OFF_MEM_MAX_PAGES, OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR, RodataConsts, TRAP_EXIT, TRAP_HOST,
    TRAP_MESSAGES, TRAP_UNSUPPORTED_ISA,
};

const SYS_READ: u32 = 0;
const SYS_WRITE: u32 = 1;
const SYS_CLOSE: u32 = 3;
const SYS_MMAP: u32 = 9;
const SYS_MPROTECT: u32 = 10;
const SYS_CLOCK_GETTIME: u32 = 228;
const SYS_EXIT_GROUP: u32 = 231;
const SYS_OPENAT: u32 = 257;

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
    if isa.has_abm_bmi1 || isa.has_bmi2 {
        a.cmp_ri(false, Gp::Rbp, 7);
        a.jcc(Cond::B, lbl_fail_isa);
    }
    a.mov_ri32(Gp::Rax, 1);
    a.cpuid();
    let baseline_mask: i32 = (1 << 9) | (1 << 19) | (1 << 20) | (1 << 23);
    a.and_ri(false, Gp::Rcx, baseline_mask);
    a.cmp_ri(false, Gp::Rcx, baseline_mask);
    a.jcc(Cond::Ne, lbl_fail_isa);
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

        a.test_rr(true, Gp::Rbp, Gp::Rbp);
        a.jcc(Cond::E, lbl_mem_done);
        a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_MEM_MAX_PAGES), Gp::Rbp);

        a.mov_rr(true, Gp::Rbx, Gp::Rbp);
        a.shl_imm(true, Gp::Rbx, 16);
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
