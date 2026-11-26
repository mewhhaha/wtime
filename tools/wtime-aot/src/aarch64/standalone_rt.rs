//! Standalone Linux AArch64 (`ET_EXEC`) runtime:
//! - Guard-page `8 GiB + 64 KiB` `PROT_NONE` reservation + `rt_sigaction(SIGSEGV)` handler
//!   (`X64Target::StandaloneExec`, `memory32` ONLY — zero signal handlers in `--blot-cdylib` `.so`!)
//! - Full WASI / Goose / `env` standalone direct-syscall import trampolines matching `x64/standalone_rt.rs`
//! - Pure-AArch64 Ryū 128-bit shortest round-trip `f64`/`f32` formatter (`HELPER_GOOSE_FMT_FLOAT`).

use wtime_types::ValType;

use crate::x64::ir::Ty;
use crate::x64::module::X64Module;
use crate::x64::runtime::RuntimeTables;
use crate::x64::standalone_rt::{
    DEFAULT_OUTPUT_LIMIT, POLICY_MAX_DIRS, POLICY_MAX_ENV, POLICY_OFF_ALLOW_ALL_ENV,
    POLICY_OFF_CWD_BUF, POLICY_OFF_CWD_LEN, POLICY_OFF_DIR_COUNT, POLICY_OFF_DIR_FDS,
    POLICY_OFF_DIR_PATH_LENS, POLICY_OFF_DIR_PATHS, POLICY_OFF_ENV_COUNT, POLICY_OFF_ENV_PTRS,
    POLICY_OFF_OPEN_HOW, POLICY_OFF_SCRATCH_PATH, POLICY_OFF_STAT_BUF, POLICY_SLAB_BYTES,
    build_ryu_pow5_tables,
};
use crate::x64::{
    CtxLayout, EXN_SLAB_BYTES, GUEST_STACK_FLOOR_HEADROOM, GUEST_STACK_GUARD_BYTES,
    GUEST_STACK_TOTAL_BYTES, GUEST_STACK_USABLE_BYTES, HELPER_GOOSE_FMT_FLOAT, HELPER_INITIALIZE,
    HELPER_STANDALONE_FS, HELPER_STANDALONE_RESERVE_COPY, INTERNAL_EXN_SIGNAL, OFF_EXEC_ARGC,
    OFF_EXEC_ARGV, OFF_EXEC_CLOCK_ORIGIN, OFF_EXEC_CLOCK_QUANTUM, OFF_EXEC_ENVP,
    OFF_EXEC_EXIT_CODE, OFF_EXEC_MEM_LIMIT, OFF_EXEC_OUTPUT_LIMIT, OFF_EXEC_POLICY_PTR,
    OFF_EXEC_PROG_NAME, OFF_EXEC_RESERVING, OFF_EXEC_TIMING_PATH, OFF_EXEC_WRITTEN, OFF_MEM_BASE,
    OFF_MEM_LEN, OFF_MEM_MAX_PAGES, OFF_MEM_RESERVE_BYTES, OFF_STACK_FLOOR, TRAP_EXIT, TRAP_HOST,
};

use super::asm::{A64Asm, A64RelocTarget, Cond, Label, Reg, VReg};
use super::lower::{CTX_REG, MEM_BASE_REG, MEM_LEN_REG, SGP1, SGP3, emit_load_off, emit_store_off};

const SYS_GETCWD: u64 = 17;
const SYS_OPENAT: u64 = 56;
const SYS_CLOSE: u64 = 57;
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_READLINKAT: u64 = 78;
const SYS_FSTAT: u64 = 80;
const SYS_EXIT_GROUP: u64 = 94;
const SYS_CLOCK_GETTIME: u64 = 113;
const SYS_RT_SIGACTION: u64 = 134;
const SYS_MUNMAP: u64 = 215;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;
const SYS_OPENAT2: u64 = 437;

const AT_FDCWD: u64 = (-100i64) as u64;
const O_RDONLY: u32 = 0;
const O_WRONLY: u32 = 1;
const O_CREAT: u32 = 0x40;
const O_TRUNC: u32 = 0x200;
const O_APPEND: u32 = 0x400;
const O_DIRECTORY_A64: u32 = 0x4000;
const O_NOFOLLOW_A64: u32 = 0x8000;
const O_CLOEXEC: u32 = 0x80000;
const O_PATH: u32 = 0x200000;
const RESOLVE_BENEATH: u64 = 0x08;
const A64_STAT_OFF_MODE: i32 = 16;
const A64_STAT_OFF_SIZE: i32 = 48;

const CLOCK_REALTIME: u32 = 0;
const CLOCK_MONOTONIC: u32 = 1;
const SIGSEGV: u32 = 11;
const SA_SIGINFO: u64 = 4;

/// 8 GiB + 64 KiB guard-page reservation covering every possible `u32(addr) + u32(offset)` access.
pub const A64_GUARD_RESERVE_BYTES: u64 = (1u64 << 33) + 65_536;

fn emit_syscall(asm: &mut A64Asm, num: u64) {
    asm.mov_imm64(Reg::X8, num);
    asm.svc(0);
}

/// Emits `clock_gettime(clk_id, [SP + ts_off])` and returns nanoseconds (`sec * 1e9 + nsec`) in `X0`.
fn emit_clock_ns(a: &mut A64Asm, clk_id: u32, ts_off: u16) {
    a.mov_imm32(Reg::X0, clk_id);
    if ts_off == 0 {
        a.mov_r(true, Reg::X1, Reg::SP);
    } else {
        a.add_imm(true, Reg::X1, Reg::SP, ts_off);
    }
    emit_syscall(a, SYS_CLOCK_GETTIME);
    a.ldr_uimm(true, Reg::X0, Reg::SP, u32::from(ts_off));
    a.ldr_uimm(true, Reg::X1, Reg::SP, u32::from(ts_off) + 8);
    a.mov_imm64(Reg::X2, 1_000_000_000);
    a.mul(true, Reg::X0, Reg::X0, Reg::X2);
    a.add_r(true, Reg::X0, Reg::X0, Reg::X1);
}

/// Loads the clock quantum into `X9` and sets Z if clocks are denied.
fn emit_check_clock_policy(a: &mut A64Asm) {
    a.ldr_uimm(true, Reg::X9, CTX_REG, OFF_EXEC_CLOCK_QUANTUM as u32);
    a.mov_imm64(Reg::X10, u64::MAX);
    a.cmp_r(true, Reg::X9, Reg::X10);
}

/// Rounds the timestamp in `X0` down to a multiple of the quantum in `X9`.
fn emit_coarsen_clock_ns(a: &mut A64Asm) {
    let lbl_done = a.new_label();
    a.cbz(true, Reg::X9, lbl_done);
    a.udiv(true, Reg::X10, Reg::X0, Reg::X9);
    a.mul(true, Reg::X0, Reg::X10, Reg::X9);
    a.bind(lbl_done);
}

/// Loops `SYS_WRITE` on partial writes and `-EINTR` (`-4`).
/// Inputs: `fd_reg`, `ptr_reg` (advanced), `len_reg` (decremented). Clobbers `X0, X1, X2, X8, X11`.
fn emit_write_all(a: &mut A64Asm, fd_reg: Reg, ptr_reg: Reg, len_reg: Reg) {
    let lp = a.new_label();
    let done = a.new_label();
    a.bind(lp);
    a.cbz(true, len_reg, done);
    a.mov_r(true, Reg::X0, fd_reg);
    a.mov_r(true, Reg::X1, ptr_reg);
    a.mov_r(true, Reg::X2, len_reg);
    emit_syscall(a, SYS_WRITE);
    a.mov_imm64(Reg::X11, (-4i64) as u64);
    a.cmp_r(true, Reg::X0, Reg::X11);
    a.b_cond(Cond::Eq, lp);
    a.cmp_imm(true, Reg::X0, 0);
    a.b_cond(Cond::Le, done);
    a.add_r(true, ptr_reg, ptr_reg, Reg::X0);
    a.sub_r(true, len_reg, len_reg, Reg::X0);
    a.b(lp);
    a.bind(done);
}

/// Checks `ptr + len <= MEM_LEN_REG` without unsigned wrap; jumps to `oob` on failure.
/// Clobbers `X11`.
fn emit_check_range(a: &mut A64Asm, ptr_reg: Reg, len_reg: Reg, oob: Label) {
    a.adds_r(true, Reg::X11, ptr_reg, len_reg);
    a.b_cond(Cond::Hs, oob);
    a.cmp_r(true, Reg::X11, MEM_LEN_REG);
    a.b_cond(Cond::Hi, oob);
}

/// Formats unsigned 64-bit `val_reg` in decimal right-to-left ending at `[SP + buf_end_off]`.
/// Returns `X4 = start_ptr`, `X5 = len`. Clobbers `val_reg, X4, X5, X6, X7, X9, X10`.
fn emit_fmt_u64_to_buf(a: &mut A64Asm, val_reg: Reg, buf_end_off: u16) {
    a.add_imm(true, Reg::X7, Reg::SP, buf_end_off);
    a.mov_r(true, Reg::X4, Reg::X7);
    a.mov_imm32(Reg::X6, 10);
    let lp = a.new_label();
    a.bind(lp);
    a.udiv(true, Reg::X9, val_reg, Reg::X6);
    a.msub(true, Reg::X10, Reg::X9, Reg::X6, val_reg);
    a.add_imm(false, Reg::X10, Reg::X10, u16::from(b'0'));
    a.sub_imm(true, Reg::X7, Reg::X7, 1);
    a.strb_uimm(Reg::X10, Reg::X7, 0);
    a.mov_r(true, val_reg, Reg::X9);
    a.cbnz(true, val_reg, lp);
    a.sub_r(true, Reg::X5, Reg::X4, Reg::X7);
    a.mov_r(true, Reg::X4, Reg::X7);
}

/// Compares NUL-terminated string in `X0` against constant ASCII bytes in local rodata.
/// Sets `W0 = 1` if equal, `0` otherwise. Clobbers `X0, X1, X2, X3, X4`.
fn emit_streq_const(a: &mut A64Asm, s: &[u8]) {
    let mut nul = Vec::with_capacity(s.len() + 1);
    nul.extend_from_slice(s);
    nul.push(0);
    let off = a.add_local_rodata(1, &nul);
    a.adr_reloc(Reg::X1, A64RelocTarget::LocalRodata(off));
    a.movz(true, Reg::X2, 0, 0);
    let lp = a.new_label();
    let no = a.new_label();
    let yes = a.new_label();
    let done = a.new_label();
    a.bind(lp);
    a.ldrb_reg(Reg::X3, Reg::X0, Reg::X2, false);
    a.ldrb_reg(Reg::X4, Reg::X1, Reg::X2, false);
    a.cmp_r(false, Reg::X3, Reg::X4);
    a.b_cond(Cond::Ne, no);
    a.cbz(false, Reg::X3, yes);
    a.add_imm(true, Reg::X2, Reg::X2, 1);
    a.b(lp);
    a.bind(yes);
    a.mov_imm32(Reg::X0, 1);
    a.b(done);
    a.bind(no);
    a.movz(false, Reg::X0, 0, 0);
    a.bind(done);
}

/// Parses a non-empty unsigned decimal NUL-terminated string at `X0` into `X0`.
/// Branches to `fail_usage` on empty input, non-digit characters, or `u64` overflow.
/// Clobbers `X0, X1, X2, X3, X4`.
fn emit_parse_u64(a: &mut A64Asm, fail_usage: Label) {
    a.mov_r(true, Reg::X1, Reg::X0);
    a.movz(true, Reg::X0, 0, 0);
    a.ldrb_uimm(Reg::X2, Reg::X1, 0);
    a.sub_imm(false, Reg::X2, Reg::X2, u16::from(b'0'));
    a.cmp_imm(false, Reg::X2, 10);
    a.b_cond(Cond::Hs, fail_usage);
    a.mov_imm32(Reg::X3, 10);
    let lp = a.new_label();
    let done = a.new_label();
    a.bind(lp);
    a.ldrb_uimm(Reg::X2, Reg::X1, 0);
    a.cbz(false, Reg::X2, done);
    a.sub_imm(false, Reg::X2, Reg::X2, u16::from(b'0'));
    a.cmp_imm(false, Reg::X2, 10);
    a.b_cond(Cond::Hs, fail_usage);
    a.umulh(Reg::X4, Reg::X0, Reg::X3);
    a.cbnz(true, Reg::X4, fail_usage);
    a.mul(true, Reg::X0, Reg::X0, Reg::X3);
    a.uxtw(Reg::X2, Reg::X2);
    a.adds_r(true, Reg::X0, Reg::X0, Reg::X2);
    a.b_cond(Cond::Hs, fail_usage);
    a.add_imm(true, Reg::X1, Reg::X1, 1);
    a.b(lp);
    a.bind(done);
}

/// Emits standalone Linux AArch64 `_start` entry point (`ET_EXEC`) with optional 8 GiB + 64 KiB
/// guard-page BCE (`layout.use_guard_pages`), host flag parsing (`--allow-files`, `--allow-env`,
/// `--output-limit`, `--memory-limit-pages`, `--timing-json`, `--no-clock`,
/// `--clock-coarsen-ns`, `--`), monotonic clock origin,
/// canonical trap message output, and `TRAP_EXIT` exit-code propagation.
pub fn emit_a64_standalone_start(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let trap_exit = a.new_label();
    let fail_alloc = a.new_label();
    let fail_usage = a.new_label();
    let after_start = a.new_label();

    // Capture Linux AArch64 `_start` stack: `[SP] = argc`, `SP + 8 = argv`.
    a.ldr_uimm(true, Reg::X20, Reg::SP, 0); // argc
    a.add_imm(true, Reg::X21, Reg::SP, 8); // argv
    // envp = &argv[argc + 1] -> X22
    a.add_imm(true, Reg::X4, Reg::X20, 1);
    a.lsl_imm(true, Reg::X4, Reg::X4, 3);
    a.add_r(true, Reg::X22, Reg::X21, Reg::X4);

    // Allocate zeroed Ctx via mmap. The default clock quantum is zero (full resolution).
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm64(Reg::X1, layout.total_bytes as u64);
    a.mov_imm32(Reg::X2, 3); // PROT_READ | PROT_WRITE
    a.mov_imm32(Reg::X3, 0x22); // MAP_PRIVATE | MAP_ANONYMOUS
    a.mov_imm64(Reg::X4, u64::MAX);
    a.movz(true, Reg::X5, 0, 0);
    emit_syscall(&mut a, SYS_MMAP);
    a.mov_imm64(Reg::X4, (-4095i64) as u64);
    a.cmp_r(true, Reg::X0, Reg::X4);
    a.b_cond(Cond::Hs, fail_alloc);
    a.mov_r(true, CTX_REG, Reg::X0);

    // Allocate 8 MiB guest stack + 64 KiB PROT_NONE guard page (switch SP early so flag parsing has scratch).
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm64(Reg::X1, GUEST_STACK_TOTAL_BYTES as u64);
    a.movz(true, Reg::X2, 0, 0);
    a.mov_imm32(Reg::X3, 0x4022);
    a.mov_imm64(Reg::X4, u64::MAX);
    a.movz(true, Reg::X5, 0, 0);
    emit_syscall(&mut a, SYS_MMAP);
    a.mov_imm64(Reg::X4, (-4095i64) as u64);
    a.cmp_r(true, Reg::X0, Reg::X4);
    a.b_cond(Cond::Hs, fail_alloc);
    a.mov_r(true, Reg::X19, Reg::X0);
    a.mov_imm64(Reg::X1, GUEST_STACK_GUARD_BYTES as u64);
    a.add_r(true, Reg::X0, Reg::X19, Reg::X1);
    a.mov_imm64(Reg::X1, GUEST_STACK_USABLE_BYTES as u64);
    a.mov_imm32(Reg::X2, 3);
    emit_syscall(&mut a, SYS_MPROTECT);
    a.mov_imm64(Reg::X4, (-4095i64) as u64);
    a.cmp_r(true, Reg::X0, Reg::X4);
    a.b_cond(Cond::Hs, fail_alloc);
    a.mov_imm64(
        Reg::X4,
        (GUEST_STACK_GUARD_BYTES + GUEST_STACK_FLOOR_HEADROOM) as u64,
    );
    a.add_r(true, Reg::X4, Reg::X19, Reg::X4);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_STACK_FLOOR,
        SGP3,
    );
    a.mov_imm64(Reg::X4, GUEST_STACK_TOTAL_BYTES as u64);
    a.add_r(true, Reg::X4, Reg::X19, Reg::X4);
    a.mov_r(true, Reg::SP, Reg::X4);
    a.sub_imm(true, Reg::SP, Reg::SP, 160);

    // Initialize default Ctx exec metadata.
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X22),
        None,
        CTX_REG,
        OFF_EXEC_ENVP,
        SGP3,
    );
    a.mov_imm64(Reg::X4, DEFAULT_OUTPUT_LIMIT);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_EXEC_OUTPUT_LIMIT,
        SGP3,
    );
    let prog_fb = a.add_local_rodata(1, b"program\0");
    let lbl_have_p0 = a.new_label();
    let lbl_p0_done = a.new_label();
    a.cbnz(true, Reg::X20, lbl_have_p0);
    a.adr_reloc(Reg::X4, A64RelocTarget::LocalRodata(prog_fb));
    a.b(lbl_p0_done);
    a.bind(lbl_have_p0);
    a.ldr_uimm(true, Reg::X4, Reg::X21, 0);
    a.bind(lbl_p0_done);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_EXEC_PROG_NAME,
        SGP3,
    );

    // Parse optional host CLI flags before `--` (X23 = idx = 1, X24 = policy_ptr = 0).
    let emit_ensure_policy = |a: &mut A64Asm| {
        let lbl_have = a.new_label();
        let lbl_cwd_lp = a.new_label();
        let lbl_cwd_done = a.new_label();
        a.cbnz(true, Reg::X24, lbl_have);
        a.movz(true, Reg::X0, 0, 0);
        a.mov_imm64(Reg::X1, POLICY_SLAB_BYTES);
        a.mov_imm32(Reg::X2, 3);
        a.mov_imm32(Reg::X3, 0x22);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc);
        a.mov_r(true, Reg::X24, Reg::X0);
        emit_store_off(
            a,
            Ty::I64,
            Some(Reg::X24),
            None,
            CTX_REG,
            OFF_EXEC_POLICY_PTR,
            SGP3,
        );
        a.add_imm(true, Reg::X0, Reg::X24, POLICY_OFF_CWD_BUF as u16);
        a.mov_imm32(Reg::X1, 4096);
        emit_syscall(a, SYS_GETCWD);
        a.cmp_imm(true, Reg::X0, 1);
        a.b_cond(Cond::Lt, lbl_have);
        a.add_imm(true, Reg::X1, Reg::X24, POLICY_OFF_CWD_BUF as u16);
        a.movz(true, Reg::X2, 0, 0);
        a.bind(lbl_cwd_lp);
        a.ldrb_reg(Reg::X3, Reg::X1, Reg::X2, false);
        a.cbz(false, Reg::X3, lbl_cwd_done);
        a.add_imm(true, Reg::X2, Reg::X2, 1);
        a.cmp_imm(true, Reg::X2, 4095);
        a.b_cond(Cond::Lo, lbl_cwd_lp);
        a.bind(lbl_cwd_done);
        a.str_uimm(true, Reg::X2, Reg::X24, POLICY_OFF_CWD_LEN as u32);
        a.bind(lbl_have);
    };

    let lbl_arg_lp = a.new_label();
    let lbl_arg_rest = a.new_label();
    let lbl_arg_done = a.new_label();
    a.mov_imm32(Reg::X23, 1);
    a.movz(true, Reg::X24, 0, 0);
    a.bind(lbl_arg_lp);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, lbl_arg_rest);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--");
    let lbl_not_sep = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_sep);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.sub_r(true, Reg::X4, Reg::X20, Reg::X23);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_EXEC_ARGC,
        SGP3,
    );
    a.lsl_imm(true, Reg::X5, Reg::X23, 3);
    a.add_r(true, Reg::X5, Reg::X21, Reg::X5);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X5),
        None,
        CTX_REG,
        OFF_EXEC_ARGV,
        SGP3,
    );
    a.b(lbl_arg_done);

    a.bind(lbl_not_sep);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--timing-json");
    let lbl_not_tj = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_tj);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    a.ldr_reg_scaled(true, Reg::X4, Reg::X21, Reg::X23, false);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_EXEC_TIMING_PATH,
        SGP3,
    );
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_tj);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--no-clock");
    let lbl_not_no_clock = a.new_label();
    let lbl_set_clock = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_no_clock);
    a.mov_imm64(Reg::X0, u64::MAX);
    a.b(lbl_set_clock);

    a.bind(lbl_not_no_clock);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--clock-coarsen-ns");
    let lbl_not_coarsen = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_coarsen);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_parse_u64(&mut a, fail_usage);
    a.cbz(true, Reg::X0, fail_usage);
    a.mov_imm64(Reg::X4, u64::MAX);
    a.cmp_r(true, Reg::X0, Reg::X4);
    a.b_cond(Cond::Eq, fail_usage);
    a.bind(lbl_set_clock);
    a.str_uimm(true, Reg::X0, CTX_REG, OFF_EXEC_CLOCK_QUANTUM as u32);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_coarsen);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--memory-limit-pages");
    let lbl_not_ml = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_ml);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_parse_u64(&mut a, fail_usage);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X0),
        None,
        CTX_REG,
        OFF_EXEC_MEM_LIMIT,
        SGP3,
    );
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_ml);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--output-limit");
    let lbl_not_ol = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_ol);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_parse_u64(&mut a, fail_usage);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X0),
        None,
        CTX_REG,
        OFF_EXEC_OUTPUT_LIMIT,
        SGP3,
    );
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_ol);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--allow-files");
    let lbl_not_af = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_af);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    emit_ensure_policy(&mut a);
    a.ldr_uimm(true, Reg::X12, Reg::X24, POLICY_OFF_DIR_COUNT as u32);
    a.cmp_imm(true, Reg::X12, POLICY_MAX_DIRS as u16);
    a.b_cond(Cond::Hs, fail_usage);
    a.mov_imm64(Reg::X0, AT_FDCWD);
    a.ldr_reg_scaled(true, Reg::X1, Reg::X21, Reg::X23, false);
    a.mov_imm32(Reg::X2, O_RDONLY | O_DIRECTORY_A64 | O_CLOEXEC);
    a.movz(true, Reg::X3, 0, 0);
    emit_syscall(&mut a, SYS_OPENAT);
    a.cmp_imm(true, Reg::X0, 0);
    a.b_cond(Cond::Lt, fail_usage);
    a.mov_r(true, Reg::X19, Reg::X0); // dirfd
    // Build "/proc/self/fd/<fd>\0" at [SP + 64..]
    let pfd_pref = a.add_local_rodata(1, b"/proc/self/fd/");
    a.adr_reloc(Reg::X1, A64RelocTarget::LocalRodata(pfd_pref));
    for i in 0..14u32 {
        a.ldrb_uimm(Reg::X2, Reg::X1, i);
        a.strb_uimm(Reg::X2, Reg::SP, 64 + i);
    }
    a.mov_r(true, Reg::X0, Reg::X19);
    emit_fmt_u64_to_buf(&mut a, Reg::X0, 144); // X4 = digits, X5 = len
    a.add_imm(true, Reg::X1, Reg::SP, 78);
    a.movz(true, Reg::X2, 0, 0);
    let lbl_cp_fd = a.new_label();
    a.bind(lbl_cp_fd);
    a.ldrb_reg(Reg::X3, Reg::X4, Reg::X2, false);
    a.strb_reg(Reg::X3, Reg::X1, Reg::X2, false);
    a.add_imm(true, Reg::X2, Reg::X2, 1);
    a.cmp_r(true, Reg::X2, Reg::X5);
    a.b_cond(Cond::Lo, lbl_cp_fd);
    a.strb_reg(Reg::XZR, Reg::X1, Reg::X2, false);
    // dst_buf = X24 + POLICY_OFF_DIR_PATHS + (dir_count << 10)
    a.ldr_uimm(true, Reg::X12, Reg::X24, POLICY_OFF_DIR_COUNT as u32);
    a.mov_imm64(Reg::X13, POLICY_OFF_DIR_PATHS as u64);
    a.add_r(true, Reg::X13, Reg::X24, Reg::X13);
    a.add_r_lsl(true, Reg::X13, Reg::X13, Reg::X12, 10);
    a.mov_imm64(Reg::X0, AT_FDCWD);
    a.add_imm(true, Reg::X1, Reg::SP, 64);
    a.mov_r(true, Reg::X2, Reg::X13);
    a.mov_imm32(Reg::X3, 1023);
    emit_syscall(&mut a, SYS_READLINKAT);
    a.cmp_imm(true, Reg::X0, 1);
    a.b_cond(Cond::Lt, fail_usage);
    // Reject truncated paths (readlinkat returning bufsiz == 1023 means path >= 1023 bytes).
    a.cmp_imm(true, Reg::X0, 1023);
    a.b_cond(Cond::Ge, fail_usage);
    let lbl_notrim = a.new_label();
    a.cmp_imm(true, Reg::X0, 1);
    a.b_cond(Cond::Le, lbl_notrim);
    a.sub_imm(true, Reg::X2, Reg::X0, 1);
    a.ldrb_reg(Reg::X3, Reg::X13, Reg::X2, false);
    a.cmp_imm(false, Reg::X3, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_notrim);
    a.mov_r(true, Reg::X0, Reg::X2);
    a.bind(lbl_notrim);
    a.strb_reg(Reg::XZR, Reg::X13, Reg::X0, false);
    a.mov_imm32(Reg::X1, POLICY_OFF_DIR_FDS as u32);
    a.add_r(true, Reg::X1, Reg::X24, Reg::X1);
    a.str_reg_scaled(true, Reg::X19, Reg::X1, Reg::X12, false);
    a.mov_imm32(Reg::X1, POLICY_OFF_DIR_PATH_LENS as u32);
    a.add_r(true, Reg::X1, Reg::X24, Reg::X1);
    a.str_reg_scaled(true, Reg::X0, Reg::X1, Reg::X12, false);
    a.add_imm(true, Reg::X12, Reg::X12, 1);
    a.str_uimm(true, Reg::X12, Reg::X24, POLICY_OFF_DIR_COUNT as u32);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_af);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"--allow-env");
    let lbl_not_ae = a.new_label();
    a.cbz(false, Reg::X0, lbl_not_ae);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.b_cond(Cond::Hs, fail_usage);
    emit_ensure_policy(&mut a);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
    emit_streq_const(&mut a, b"*");
    let lbl_env_one = a.new_label();
    let lbl_env_done = a.new_label();
    a.cbz(false, Reg::X0, lbl_env_one);
    a.mov_imm32(Reg::X4, 1);
    a.str_uimm(true, Reg::X4, Reg::X24, POLICY_OFF_ALLOW_ALL_ENV as u32);
    a.b(lbl_env_done);
    a.bind(lbl_env_one);
    a.ldr_uimm(true, Reg::X4, Reg::X24, POLICY_OFF_ENV_COUNT as u32);
    a.cmp_imm(true, Reg::X4, POLICY_MAX_ENV as u16);
    a.b_cond(Cond::Hs, lbl_env_done);
    a.ldr_reg_scaled(true, Reg::X5, Reg::X21, Reg::X23, false);
    a.add_imm(true, Reg::X6, Reg::X24, POLICY_OFF_ENV_PTRS as u16);
    a.str_reg_scaled(true, Reg::X5, Reg::X6, Reg::X4, false);
    a.add_imm(true, Reg::X4, Reg::X4, 1);
    a.str_uimm(true, Reg::X4, Reg::X24, POLICY_OFF_ENV_COUNT as u32);
    a.bind(lbl_env_done);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.b(lbl_arg_lp);

    a.bind(lbl_not_ae);
    for flag in [b"--time-limit".as_slice(), b"--stack-limit-mib"] {
        a.ldr_reg_scaled(true, Reg::X0, Reg::X21, Reg::X23, false);
        emit_streq_const(&mut a, flag);
        let lbl_next = a.new_label();
        a.cbz(false, Reg::X0, lbl_next);
        a.add_imm(true, Reg::X23, Reg::X23, 2);
        a.b(lbl_arg_lp);
        a.bind(lbl_next);
    }

    a.bind(lbl_arg_rest);
    a.cmp_r(true, Reg::X23, Reg::X20);
    a.csel(true, Reg::X23, Reg::X20, Reg::X23, Cond::Hi);
    a.sub_r(true, Reg::X4, Reg::X20, Reg::X23);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X4),
        None,
        CTX_REG,
        OFF_EXEC_ARGC,
        SGP3,
    );
    a.lsl_imm(true, Reg::X5, Reg::X23, 3);
    a.add_r(true, Reg::X5, Reg::X21, Reg::X5);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X5),
        None,
        CTX_REG,
        OFF_EXEC_ARGV,
        SGP3,
    );
    a.bind(lbl_arg_done);

    // Record the monotonic clock origin if guest clocks or timing JSON need it.
    let lbl_record_origin = a.new_label();
    let lbl_skip_origin = a.new_label();
    emit_check_clock_policy(&mut a);
    a.b_cond(Cond::Ne, lbl_record_origin);
    a.ldr_uimm(true, Reg::X0, CTX_REG, OFF_EXEC_TIMING_PATH as u32);
    a.cbz(true, Reg::X0, lbl_skip_origin);
    a.bind(lbl_record_origin);
    emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X0),
        None,
        CTX_REG,
        OFF_EXEC_CLOCK_ORIGIN,
        SGP3,
    );
    a.bind(lbl_skip_origin);

    // Reserve & commit linear memory (8 GiB + 64 KiB PROT_NONE guard region for standalone memory32,
    // or clamped memory64 limit).
    a.movz(true, MEM_BASE_REG, 0, 0);
    a.movz(true, MEM_LEN_REG, 0, 0);
    if let Some((min_pages, declared_max)) = module.memory {
        let lbl_have_lim = a.new_label();
        let lbl_mem_done = a.new_label();
        let default_lim = if module.memory64 {
            min_pages.max(1 << 22) // 256 GiB virtual cap (MAP_NORESERVE; 0 physical RAM until touched)
        } else {
            65_536
        };
        emit_load_off(
            &mut a,
            Ty::I64,
            Some(Reg::X19),
            None,
            CTX_REG,
            OFF_EXEC_MEM_LIMIT,
            SGP3,
        );
        a.cbnz(true, Reg::X19, lbl_have_lim);
        a.mov_imm64(Reg::X19, default_lim);
        a.bind(lbl_have_lim);
        a.mov_imm64(Reg::X4, min_pages);
        a.cmp_r(true, Reg::X4, Reg::X19);
        a.b_cond(Cond::Hi, fail_alloc);
        let hard_cap = if module.memory64 { 1u64 << 28 } else { 65_536 };
        a.mov_imm64(Reg::X20, declared_max.min(hard_cap));
        a.cmp_r(true, Reg::X19, Reg::X20);
        a.csel(true, Reg::X20, Reg::X19, Reg::X20, Cond::Lo);
        a.cmp_r(true, Reg::X4, Reg::X20);
        a.b_cond(Cond::Hi, fail_alloc);
        if layout.use_guard_pages {
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X20),
                None,
                CTX_REG,
                OFF_MEM_MAX_PAGES,
                SGP3,
            );
            a.mov_imm64(Reg::X19, A64_GUARD_RESERVE_BYTES);
        } else {
            a.cbz(true, Reg::X20, lbl_mem_done);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X20),
                None,
                CTX_REG,
                OFF_MEM_MAX_PAGES,
                SGP3,
            );
            a.lsl_imm(true, Reg::X19, Reg::X20, 16);
        }
        a.movz(true, Reg::X0, 0, 0);
        a.mov_r(true, Reg::X1, Reg::X19);
        a.movz(true, Reg::X2, 0, 0);
        a.mov_imm32(Reg::X3, 0x4022);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc);
        a.mov_r(true, MEM_BASE_REG, Reg::X0);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(MEM_BASE_REG),
            None,
            CTX_REG,
            OFF_MEM_BASE,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X19),
            None,
            CTX_REG,
            OFF_MEM_RESERVE_BYTES,
            SGP3,
        );
        if min_pages > 0 && min_pages <= hard_cap {
            a.mov_imm64(MEM_LEN_REG, min_pages << 16);
            a.mov_r(true, Reg::X0, MEM_BASE_REG);
            a.mov_r(true, Reg::X1, MEM_LEN_REG);
            a.mov_imm32(Reg::X2, 3);
            emit_syscall(&mut a, SYS_MPROTECT);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(MEM_LEN_REG),
                None,
                CTX_REG,
                OFF_MEM_LEN,
                SGP3,
            );
        }
        a.bind(lbl_mem_done);
    }

    // Reserve & commit additional linear memories `1..N` (`multi-memory`).
    for (mi, em) in module.extra_memories.iter().enumerate() {
        let desc_off = layout.extra_mem_offs[mi];
        let hard_cap = if em.memory64 { 1u64 << 28 } else { 65_536 };
        if em.min_pages > hard_cap {
            a.b(fail_alloc);
            continue;
        }
        let default_cap = if em.memory64 {
            em.min_pages.max(131_072).min(hard_cap)
        } else {
            65_536
        };
        let eff_max = em.max_pages.min(default_cap);
        if eff_max == 0 {
            continue;
        }
        a.mov_imm64(Reg::X4, eff_max);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            desc_off + 16,
            SGP3,
        );
        let reserve_bytes = eff_max << 16;
        a.mov_imm64(Reg::X1, reserve_bytes);
        a.mov_r(true, Reg::X19, Reg::X1);
        a.movz(true, Reg::X0, 0, 0);
        a.movz(true, Reg::X2, 0, 0);
        a.mov_imm32(Reg::X3, 0x4022);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc);
        a.mov_r(true, Reg::X20, Reg::X0);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X20),
            None,
            CTX_REG,
            desc_off,
            SGP3,
        );
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X19),
            None,
            CTX_REG,
            desc_off + 24,
            SGP3,
        );
        if em.min_pages > 0 {
            let init_len = em.min_pages << 16;
            a.mov_r(true, Reg::X0, Reg::X20);
            a.mov_imm64(Reg::X1, init_len);
            a.mov_imm32(Reg::X2, 3);
            emit_syscall(&mut a, SYS_MPROTECT);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc);
            a.mov_imm64(Reg::X4, init_len);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                desc_off + 8,
                SGP3,
            );
        }
    }

    // Exception arena slab (demand-paged RW, EH modules only).
    if let Some(exn_off) = layout.exn_state_off {
        a.movz(true, Reg::X0, 0, 0);
        a.mov_imm64(Reg::X1, EXN_SLAB_BYTES as u64);
        a.mov_imm32(Reg::X2, 3);
        a.mov_imm32(Reg::X3, 0x22);
        a.mov_imm64(Reg::X4, u64::MAX);
        a.movz(true, Reg::X5, 0, 0);
        emit_syscall(&mut a, SYS_MMAP);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X0),
            None,
            CTX_REG,
            exn_off + 8,
            SGP3,
        );
        a.mov_imm64(Reg::X4, 16);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            exn_off + 16,
            SGP3,
        );
        a.mov_imm64(Reg::X4, EXN_SLAB_BYTES as u64);
        emit_store_off(
            &mut a,
            Ty::I64,
            Some(Reg::X4),
            None,
            CTX_REG,
            exn_off + 24,
            SGP3,
        );
    }

    // Dynamic tables (demand-paged RW mmap, allocated ONLY when `dyn_tables` non-empty).
    for &(desc_off, min_len, max_len) in layout.dyn_tables.values() {
        let reserve_bytes = if max_len > 0 {
            ((u64::from(max_len) * 8) + 4095) & !4095
        } else {
            0
        };
        if reserve_bytes > 0 {
            a.movz(true, Reg::X0, 0, 0);
            a.mov_imm64(Reg::X1, reserve_bytes);
            a.mov_imm32(Reg::X2, 3);
            a.mov_imm32(Reg::X3, 0x22);
            a.mov_imm64(Reg::X4, u64::MAX);
            a.movz(true, Reg::X5, 0, 0);
            emit_syscall(&mut a, SYS_MMAP);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, fail_alloc);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                desc_off,
                SGP3,
            );
            a.mov_imm32(Reg::X4, min_len);
            emit_store_off(
                &mut a,
                Ty::I32,
                Some(Reg::X4),
                None,
                CTX_REG,
                desc_off + 8,
                SGP3,
            );
            a.mov_imm32(Reg::X4, max_len);
            emit_store_off(
                &mut a,
                Ty::I32,
                Some(Reg::X4),
                None,
                CTX_REG,
                desc_off + 12,
                SGP3,
            );
            a.mov_imm64(Reg::X4, reserve_bytes);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                desc_off + 16,
                SGP3,
            );
        }
    }

    // Install SIGSEGV guard-page handler (standalone memory32 ONLY!).
    let lbl_sig_handler = a.new_label();
    if layout.use_guard_pages {
        // struct kernel_sigaction at [SP + 0..32]:
        //   +0: sa_handler, +8: sa_flags (SA_SIGINFO=4), +16: sa_restorer (0), +24: sa_mask (0)
        a.adr(Reg::X4, lbl_sig_handler);
        a.str_uimm(true, Reg::X4, Reg::SP, 0);
        a.mov_imm64(Reg::X4, SA_SIGINFO);
        a.str_uimm(true, Reg::X4, Reg::SP, 8);
        a.str_uimm(true, Reg::XZR, Reg::SP, 16);
        a.str_uimm(true, Reg::XZR, Reg::SP, 24);
        a.mov_imm32(Reg::X0, SIGSEGV);
        a.mov_r(true, Reg::X1, Reg::SP);
        a.movz(true, Reg::X2, 0, 0);
        a.mov_imm32(Reg::X3, 8); // sizeof(kernel_sigset_t)
        emit_syscall(&mut a, SYS_RT_SIGACTION);
        a.mov_imm64(Reg::X4, (-4095i64) as u64);
        a.cmp_r(true, Reg::X0, Reg::X4);
        a.b_cond(Cond::Hs, fail_alloc);
    }

    // Run HELPER_INITIALIZE and optional standalone `_start` / `main`.
    a.bl_reloc(A64RelocTarget::Helper(HELPER_INITIALIZE));
    a.cbnz(false, SGP1, after_start);
    if let Some((entry_f, returns_i32)) = module.standalone_start {
        a.bl_reloc(A64RelocTarget::Func(entry_f));
        if returns_i32 {
            let skip_ret = a.new_label();
            a.cbnz(false, SGP1, skip_ret);
            a.sxtw(Reg::X0, Reg::X0);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                OFF_EXEC_EXIT_CODE,
                SGP3,
            );
            a.bind(skip_ret);
        }
    }

    a.bind(after_start);
    let normal_exit = a.new_label();
    a.cbz(false, SGP1, normal_exit);
    a.cmp_imm(false, SGP1, u16::from(TRAP_EXIT));
    a.b_cond(Cond::Ne, trap_exit);

    a.bind(normal_exit);
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X0),
        None,
        CTX_REG,
        OFF_EXEC_EXIT_CODE,
        SGP3,
    );
    emit_syscall(&mut a, SYS_EXIT_GROUP);

    // Trap exit: print canonical TRAP_MESSAGES[W16 - 1] + "\n" to stderr (fd=2), exit(1).
    a.bind(trap_exit);
    let exit_one = a.new_label();
    a.uxtb(Reg::X9, SGP1);
    if layout.exn_state_off.is_some() {
        let not_exn = a.new_label();
        a.cmp_imm(false, Reg::X9, u16::from(INTERNAL_EXN_SIGNAL));
        a.b_cond(Cond::Ne, not_exn);
        let exn_msg = b"Exception: uncaught wasm exception\n";
        let exn_off = a.add_local_rodata(1, exn_msg);
        a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(exn_off));
        a.mov_imm32(Reg::X13, exn_msg.len() as u32);
        a.mov_imm32(Reg::X14, 2);
        emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
        a.b(exit_one);
        a.bind(not_exn);
    }
    a.sub_imm(false, Reg::X9, Reg::X9, 1);
    a.cmp_imm(
        false,
        Reg::X9,
        crate::x64::runtime::TRAP_MESSAGES.len() as u16,
    );
    a.b_cond(Cond::Hs, exit_one);
    a.adr_reloc(Reg::X10, A64RelocTarget::Rodata(tables.trap_table_off));
    a.lsl_imm(true, Reg::X9, Reg::X9, 3);
    a.add_r(true, Reg::X9, Reg::X10, Reg::X9);
    a.ldrsw_uimm(Reg::X12, Reg::X9, 0);
    a.add_r(true, Reg::X12, Reg::X10, Reg::X12); // msg ptr
    a.ldr_uimm(false, Reg::X13, Reg::X9, 4); // msg len
    a.mov_imm32(Reg::X14, 2); // stderr
    emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
    let nl_off = a.add_local_rodata(1, b"\n");
    a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(nl_off));
    a.mov_imm32(Reg::X13, 1);
    a.mov_imm32(Reg::X14, 2);
    emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
    a.bind(exit_one);
    a.mov_imm32(Reg::X0, 1);
    emit_syscall(&mut a, SYS_EXIT_GROUP);

    a.bind(fail_usage);
    let umsg = b"invalid standalone runner option or --allow-files directory\n";
    let uoff = a.add_local_rodata(1, umsg);
    a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(uoff));
    a.mov_imm32(Reg::X13, umsg.len() as u32);
    a.mov_imm32(Reg::X14, 2);
    emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
    a.mov_imm32(Reg::X0, 2);
    emit_syscall(&mut a, SYS_EXIT_GROUP);

    a.bind(fail_alloc);
    let oom_off = a.add_local_rodata(1, b"allocation failed\n");
    a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(oom_off));
    a.mov_imm32(Reg::X13, 18);
    a.mov_imm32(Reg::X14, 2);
    emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
    a.mov_imm32(Reg::X0, 1);
    emit_syscall(&mut a, SYS_EXIT_GROUP);

    if layout.use_guard_pages {
        // Linux AArch64 SA_SIGINFO handler: (X0=sig, X1=siginfo_t*, X2=ucontext_t*)
        // Exact Linux arm64 ABI offsets:
        //   siginfo_t.si_addr = +16
        //   ucontext_t.uc_mcontext.regs[27] (MEM_BASE_REG) = 184 + 27*8 = 400
        //   ucontext_t.uc_mcontext.regs[28] (CTX_REG)      = 184 + 28*8 = 408
        let not_ours = a.new_label();
        a.bind(lbl_sig_handler);
        a.ldr_uimm(true, Reg::X9, Reg::X1, 16); // si_addr
        a.ldr_uimm(true, Reg::X10, Reg::X2, 408); // ucontext X28 (Ctx*)
        a.cbz(true, Reg::X10, not_ours);
        a.ldr_uimm(true, Reg::X11, Reg::X2, 400); // ucontext X27 (mem_base)
        a.cbz(true, Reg::X11, not_ours);
        a.ldr_uimm(true, Reg::X12, Reg::X10, OFF_MEM_BASE as u32);
        a.cmp_r(true, Reg::X11, Reg::X12);
        a.b_cond(Cond::Ne, not_ours);
        a.sub_r(true, Reg::X9, Reg::X9, Reg::X11); // fault_off = si_addr - mem_base
        a.mov_imm64(Reg::X12, A64_GUARD_RESERVE_BYTES);
        a.cmp_r(true, Reg::X9, Reg::X12);
        a.b_cond(Cond::Hs, not_ours);
        // Confirmed linear-memory guard-page OOB: async-signal-safe write + exit_group(1).
        let oob_msg = b"out of bounds memory access\n";
        let oob_off = a.add_local_rodata(1, oob_msg);
        a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(oob_off));
        a.mov_imm32(Reg::X13, oob_msg.len() as u32);
        a.mov_imm32(Reg::X14, 2);
        emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
        a.mov_imm32(Reg::X0, 1);
        emit_syscall(&mut a, SYS_EXIT_GROUP);

        // Not our linear-memory guard fault: restore SIG_DFL and return to re-fault authentically.
        a.bind(not_ours);
        a.sub_imm(true, Reg::SP, Reg::SP, 32);
        a.str_uimm(true, Reg::XZR, Reg::SP, 0);
        a.str_uimm(true, Reg::XZR, Reg::SP, 8);
        a.str_uimm(true, Reg::XZR, Reg::SP, 16);
        a.str_uimm(true, Reg::XZR, Reg::SP, 24);
        a.mov_imm32(Reg::X0, SIGSEGV);
        a.mov_r(true, Reg::X1, Reg::SP);
        a.movz(true, Reg::X2, 0, 0);
        a.mov_imm32(Reg::X3, 8);
        emit_syscall(&mut a, SYS_RT_SIGACTION);
        a.add_imm(true, Reg::SP, Reg::SP, 32);
        a.ret();
    }

    a.finish()?;
    Ok(a)
}

/// Emits `HELPER_GOOSE_FMT_FLOAT` on AArch64: exact Ryū 128-bit shortest round-trip `f64`/`f32`
/// formatter with Goose's exact layout (`nan`, `inf`, `-inf`, `0.0`, `-0.0`, fixed vs scientific).
///
/// ABI:
/// - Input: `X0 = out_ptr (char[32])`, `V0 = value`, `W1 = is_f64 (1 = f64, 0 = f32)`
/// - Output: `X0 = byte_len (1..=32)` written to `[out_ptr..out_ptr + X0]`
/// - Preserves `X19..X28, FP, LR, V8..V15`.
pub fn emit_a64_helper_goose_fmt_float() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let (inv_bytes, pow_bytes) = build_ryu_pow5_tables();
    let inv_tbl_off = a.add_local_rodata(16, &inv_bytes);
    let pow_tbl_off = a.add_local_rodata(16, &pow_bytes);

    // 192-byte frame: [SP+0..128] locals/buffers, [SP+128..192] saved X19..X24, FP, LR.
    a.sub_imm(true, Reg::SP, Reg::SP, 192);
    a.stp_x(Reg::X19, Reg::X20, Reg::SP, 128);
    a.stp_x(Reg::X21, Reg::X22, Reg::SP, 144);
    a.stp_x(Reg::X23, Reg::X24, Reg::SP, 160);
    a.stp_x(Reg::FP, Reg::LR, Reg::SP, 176);
    a.str_uimm(true, Reg::X0, Reg::SP, 0); // [SP+0] = out_ptr

    // Unpack: sign -> [SP+8], exp -> X9, mant -> X10, exp_max -> X11, bias_adj -> X12, imp_bit -> X13
    let lbl_f32 = a.new_label();
    let lbl_unpacked = a.new_label();
    a.cbz(false, Reg::X1, lbl_f32);
    // f64 unpack
    a.fmov_gp_from_fp(true, Reg::X2, VReg::V0);
    a.lsr_imm(true, Reg::X3, Reg::X2, 63);
    a.str_uimm(true, Reg::X3, Reg::SP, 8);
    a.lsr_imm(true, Reg::X9, Reg::X2, 52);
    a.mov_imm32(Reg::X11, 0x7ff);
    a.and_r(true, Reg::X9, Reg::X9, Reg::X11);
    a.mov_imm64(Reg::X13, (1u64 << 52) - 1);
    a.and_r(true, Reg::X10, Reg::X2, Reg::X13);
    a.mov_imm32(Reg::X12, 1077); // 1023 + 52 + 2
    a.mov_imm64(Reg::X13, 1u64 << 52);
    a.b(lbl_unpacked);

    // f32 unpack
    a.bind(lbl_f32);
    a.fmov_gp_from_fp(false, Reg::X2, VReg::V0);
    a.lsr_imm(false, Reg::X3, Reg::X2, 31);
    a.str_uimm(true, Reg::X3, Reg::SP, 8);
    a.lsr_imm(false, Reg::X9, Reg::X2, 23);
    a.mov_imm32(Reg::X11, 0xff);
    a.and_r(false, Reg::X9, Reg::X9, Reg::X11);
    a.mov_imm32(Reg::X13, 0x7f_ffff);
    a.and_r(false, Reg::X10, Reg::X2, Reg::X13);
    a.mov_imm32(Reg::X12, 152); // 127 + 23 + 2
    a.mov_imm64(Reg::X13, 1u64 << 23);

    a.bind(lbl_unpacked);
    let lbl_ret = a.new_label();
    let lbl_not_max = a.new_label();
    let lbl_not_zero = a.new_label();

    // Check exp == exp_max (NaN or ±inf)
    a.cmp_r(true, Reg::X9, Reg::X11);
    a.b_cond(Cond::Ne, lbl_not_max);
    let lbl_inf = a.new_label();
    a.cbz(true, Reg::X10, lbl_inf);
    // "nan"
    a.mov_imm32(Reg::X3, u32::from(b'n'));
    a.strb_uimm(Reg::X3, Reg::X0, 0);
    a.mov_imm32(Reg::X3, u32::from(b'a'));
    a.strb_uimm(Reg::X3, Reg::X0, 1);
    a.mov_imm32(Reg::X3, u32::from(b'n'));
    a.strb_uimm(Reg::X3, Reg::X0, 2);
    a.mov_imm32(Reg::X0, 3);
    a.b(lbl_ret);

    a.bind(lbl_inf);
    let lbl_pos_inf = a.new_label();
    a.ldr_uimm(true, Reg::X3, Reg::SP, 8);
    a.cbz(true, Reg::X3, lbl_pos_inf);
    for (i, &ch) in b"-inf".iter().enumerate() {
        a.mov_imm32(Reg::X3, u32::from(ch));
        a.strb_uimm(Reg::X3, Reg::X0, i as u32);
    }
    a.mov_imm32(Reg::X0, 4);
    a.b(lbl_ret);
    a.bind(lbl_pos_inf);
    for (i, &ch) in b"inf".iter().enumerate() {
        a.mov_imm32(Reg::X3, u32::from(ch));
        a.strb_uimm(Reg::X3, Reg::X0, i as u32);
    }
    a.mov_imm32(Reg::X0, 3);
    a.b(lbl_ret);

    // Check ±0.0
    a.bind(lbl_not_max);
    a.orr_r(true, Reg::X2, Reg::X9, Reg::X10);
    a.cbnz(true, Reg::X2, lbl_not_zero);
    let lbl_pos_zero = a.new_label();
    a.ldr_uimm(true, Reg::X3, Reg::SP, 8);
    a.cbz(true, Reg::X3, lbl_pos_zero);
    for (i, &ch) in b"-0.0".iter().enumerate() {
        a.mov_imm32(Reg::X3, u32::from(ch));
        a.strb_uimm(Reg::X3, Reg::X0, i as u32);
    }
    a.mov_imm32(Reg::X0, 4);
    a.b(lbl_ret);
    a.bind(lbl_pos_zero);
    for (i, &ch) in b"0.0".iter().enumerate() {
        a.mov_imm32(Reg::X3, u32::from(ch));
        a.strb_uimm(Reg::X3, Reg::X0, i as u32);
    }
    a.mov_imm32(Reg::X0, 3);
    a.b(lbl_ret);

    // Finite non-zero: X2 = m2, X19 = e2 (signed i64), X20 = flags
    a.bind(lbl_not_zero);
    let lbl_subnorm = a.new_label();
    let lbl_got_m2 = a.new_label();
    a.cbz(true, Reg::X9, lbl_subnorm);
    // Normal: m2 = mant | imp_bit, e2 = exp - bias_adj, mm_shift = (mant != 0 || exp <= 1)
    a.orr_r(true, Reg::X2, Reg::X10, Reg::X13);
    a.sub_r(true, Reg::X19, Reg::X9, Reg::X12);
    a.movz(true, Reg::X20, 0, 0);
    let lbl_mm1 = a.new_label();
    a.cbnz(true, Reg::X10, lbl_mm1);
    a.cmp_imm(true, Reg::X9, 1);
    a.b_cond(Cond::Gt, lbl_got_m2);
    a.bind(lbl_mm1);
    a.mov_imm32(Reg::X20, 8); // bit 3 = mm_shift = 1
    a.b(lbl_got_m2);

    a.bind(lbl_subnorm);
    a.mov_r(true, Reg::X2, Reg::X10);
    a.mov_imm32(Reg::X3, 1);
    a.sub_r(true, Reg::X19, Reg::X3, Reg::X12);
    a.mov_imm32(Reg::X20, 8);

    a.bind(lbl_got_m2);
    // accept_bounds = ((m2 & 1) == 0) -> bit 0 of X20
    a.mov_imm32(Reg::X3, 1);
    a.and_r(true, Reg::X4, Reg::X2, Reg::X3);
    let lbl_odd = a.new_label();
    a.cbnz(true, Reg::X4, lbl_odd);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3);
    a.bind(lbl_odd);

    // mv = 4*m2, mp = mv + 2, mm = mv - 1 - mm_shift
    a.lsl_imm(true, Reg::X2, Reg::X2, 2);
    a.str_uimm(true, Reg::X2, Reg::SP, 16);
    a.add_imm(true, Reg::X3, Reg::X2, 2);
    a.str_uimm(true, Reg::X3, Reg::SP, 24);
    a.sub_imm(true, Reg::X2, Reg::X2, 1);
    a.mov_imm32(Reg::X3, 8);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    let lbl_no_mm = a.new_label();
    a.cbz(true, Reg::X4, lbl_no_mm);
    a.sub_imm(true, Reg::X2, Reg::X2, 1);
    a.bind(lbl_no_mm);
    a.str_uimm(true, Reg::X2, Reg::SP, 32);
    a.str_uimm(true, Reg::X20, Reg::SP, 48);

    // Inline 128-bit mulShift64 over [SP+16], [SP+24], [SP+32]:
    // Inputs: X21 = mul_lo, X22 = mul_hi, X23 = shift - 64.
    let emit_mul_shift_all3 = |a: &mut A64Asm| {
        a.mov_imm32(Reg::X24, 64);
        a.sub_r(true, Reg::X24, Reg::X24, Reg::X23); // X24 = 64 - (shift - 64)
        for slot in [16u32, 24, 32] {
            a.ldr_uimm(true, Reg::X0, Reg::SP, slot);
            a.umulh(Reg::X1, Reg::X0, Reg::X21);
            a.mul(true, Reg::X2, Reg::X0, Reg::X22);
            a.umulh(Reg::X3, Reg::X0, Reg::X22);
            a.adds_r(true, Reg::X2, Reg::X2, Reg::X1);
            a.adc(true, Reg::X3, Reg::X3, Reg::XZR);
            a.lsrv(true, Reg::X2, Reg::X2, Reg::X23);
            a.lslv(true, Reg::X3, Reg::X3, Reg::X24);
            a.cmp_imm(true, Reg::X23, 0);
            a.csel(true, Reg::X3, Reg::XZR, Reg::X3, Cond::Eq);
            a.orr_r(true, Reg::X0, Reg::X2, Reg::X3);
            a.str_uimm(true, Reg::X0, Reg::SP, slot);
        }
    };

    // Inline divisibility check: `5^q | X0` (q in X21); returns X0 = 1 iff divisible.
    let emit_div_pow5 = |a: &mut A64Asm| {
        let lp = a.new_label();
        let no = a.new_label();
        let yes = a.new_label();
        let end = a.new_label();
        a.mov_r(true, Reg::X1, Reg::X21);
        a.mov_imm32(Reg::X2, 5);
        a.bind(lp);
        a.cbz(true, Reg::X1, yes);
        a.udiv(true, Reg::X3, Reg::X0, Reg::X2);
        a.msub(true, Reg::X4, Reg::X3, Reg::X2, Reg::X0);
        a.cbnz(true, Reg::X4, no);
        a.mov_r(true, Reg::X0, Reg::X3);
        a.sub_imm(true, Reg::X1, Reg::X1, 1);
        a.b(lp);
        a.bind(yes);
        a.mov_imm32(Reg::X0, 1);
        a.b(end);
        a.bind(no);
        a.movz(true, Reg::X0, 0, 0);
        a.bind(end);
    };

    let lbl_e2_neg = a.new_label();
    let lbl_after_mul = a.new_label();
    a.cmp_imm(true, Reg::X19, 0);
    a.b_cond(Cond::Lt, lbl_e2_neg);

    // ---- Case e2 >= 0 ----
    // q = (e2 * 78913) >> 18; if e2 > 3 && q > 0 { q -= 1 }
    a.mov_imm64(Reg::X3, 78_913);
    a.mul(true, Reg::X21, Reg::X19, Reg::X3);
    a.lsr_imm(true, Reg::X21, Reg::X21, 18);
    let lbl_no_dec_q1 = a.new_label();
    a.cmp_imm(true, Reg::X19, 3);
    a.b_cond(Cond::Le, lbl_no_dec_q1);
    a.cbz(true, Reg::X21, lbl_no_dec_q1);
    a.sub_imm(true, Reg::X21, Reg::X21, 1);
    a.bind(lbl_no_dec_q1);
    a.str_uimm(true, Reg::X21, Reg::SP, 40); // e10 = q

    // pow5bits(q) -> X3
    let lbl_p5b1_z = a.new_label();
    let lbl_p5b1_done = a.new_label();
    a.cbz(true, Reg::X21, lbl_p5b1_z);
    a.mov_imm64(Reg::X4, 1_217_359);
    a.mul(true, Reg::X3, Reg::X21, Reg::X4);
    a.lsr_imm(true, Reg::X3, Reg::X3, 19);
    a.add_imm(true, Reg::X3, Reg::X3, 1);
    a.b(lbl_p5b1_done);
    a.bind(lbl_p5b1_z);
    a.mov_imm32(Reg::X3, 1);
    a.bind(lbl_p5b1_done);
    // shift - 64 = q + p5b + 60 - e2
    a.add_r(true, Reg::X3, Reg::X3, Reg::X21);
    a.add_imm(true, Reg::X3, Reg::X3, 60);
    a.sub_r(true, Reg::X3, Reg::X3, Reg::X19);
    a.str_uimm(true, Reg::X3, Reg::SP, 56);

    // Trailing-zero check when q <= 21
    let lbl_pos_skip_tz = a.new_label();
    let lbl_pos_q_nz = a.new_label();
    a.cmp_imm(true, Reg::X21, 21);
    a.b_cond(Cond::Gt, lbl_pos_skip_tz);
    a.cbnz(true, Reg::X21, lbl_pos_q_nz);
    // q == 0
    a.mov_imm32(Reg::X3, 2);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vr_tz
    a.mov_imm32(Reg::X3, 1);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    let lbl_pos_q0_no_ab = a.new_label();
    a.cbz(true, Reg::X4, lbl_pos_q0_no_ab);
    a.mov_imm32(Reg::X3, 4);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vm_tz
    a.b(lbl_pos_skip_tz);
    a.bind(lbl_pos_q0_no_ab);
    a.mov_imm32(Reg::X3, 16);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // dec_vp
    a.b(lbl_pos_skip_tz);

    a.bind(lbl_pos_q_nz);
    a.ldr_uimm(true, Reg::X0, Reg::SP, 16); // mv
    a.mov_imm32(Reg::X2, 5);
    a.udiv(true, Reg::X3, Reg::X0, Reg::X2);
    a.msub(true, Reg::X4, Reg::X3, Reg::X2, Reg::X0);
    let lbl_mv_not5 = a.new_label();
    a.cbnz(true, Reg::X4, lbl_mv_not5);
    emit_div_pow5(&mut a);
    a.cbz(false, Reg::X0, lbl_pos_skip_tz);
    a.mov_imm32(Reg::X3, 2);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vr_tz
    a.b(lbl_pos_skip_tz);

    a.bind(lbl_mv_not5);
    let lbl_pos_no_ab = a.new_label();
    a.mov_imm32(Reg::X3, 1);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_pos_no_ab);
    a.ldr_uimm(true, Reg::X0, Reg::SP, 32); // mm
    emit_div_pow5(&mut a);
    a.cbz(false, Reg::X0, lbl_pos_skip_tz);
    a.mov_imm32(Reg::X3, 4);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vm_tz
    a.b(lbl_pos_skip_tz);

    a.bind(lbl_pos_no_ab);
    a.ldr_uimm(true, Reg::X0, Reg::SP, 24); // mp
    emit_div_pow5(&mut a);
    a.cbz(false, Reg::X0, lbl_pos_skip_tz);
    a.mov_imm32(Reg::X3, 16);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // dec_vp

    a.bind(lbl_pos_skip_tz);
    a.str_uimm(true, Reg::X20, Reg::SP, 48);
    // Load POW5_INV_SPLIT[q] into (X21, X22)
    a.adr_reloc(Reg::X5, A64RelocTarget::LocalRodata(inv_tbl_off));
    a.lsl_imm(true, Reg::X6, Reg::X21, 4);
    a.add_r(true, Reg::X5, Reg::X5, Reg::X6);
    a.ldr_uimm(true, Reg::X21, Reg::X5, 0);
    a.ldr_uimm(true, Reg::X22, Reg::X5, 8);
    a.ldr_uimm(true, Reg::X23, Reg::SP, 56);
    emit_mul_shift_all3(&mut a);
    a.b(lbl_after_mul);

    // ---- Case e2 < 0 ----
    a.bind(lbl_e2_neg);
    a.neg_r(true, Reg::X19, Reg::X19); // X19 = -e2 > 0
    a.mov_imm64(Reg::X3, 732_923);
    a.mul(true, Reg::X21, Reg::X19, Reg::X3);
    a.lsr_imm(true, Reg::X21, Reg::X21, 20);
    let lbl_no_dec_q2 = a.new_label();
    a.cmp_imm(true, Reg::X19, 1);
    a.b_cond(Cond::Le, lbl_no_dec_q2);
    a.cbz(true, Reg::X21, lbl_no_dec_q2);
    a.sub_imm(true, Reg::X21, Reg::X21, 1);
    a.bind(lbl_no_dec_q2);
    // e10 = q - (-e2)
    a.sub_r(true, Reg::X3, Reg::X21, Reg::X19);
    a.str_uimm(true, Reg::X3, Reg::SP, 40);
    // idx = (-e2) - q -> X22
    a.sub_r(true, Reg::X22, Reg::X19, Reg::X21);

    // pow5bits(idx) -> X3
    let lbl_p5b2_z = a.new_label();
    let lbl_p5b2_done = a.new_label();
    a.cbz(true, Reg::X22, lbl_p5b2_z);
    a.mov_imm64(Reg::X4, 1_217_359);
    a.mul(true, Reg::X3, Reg::X22, Reg::X4);
    a.lsr_imm(true, Reg::X3, Reg::X3, 19);
    a.add_imm(true, Reg::X3, Reg::X3, 1);
    a.b(lbl_p5b2_done);
    a.bind(lbl_p5b2_z);
    a.mov_imm32(Reg::X3, 1);
    a.bind(lbl_p5b2_done);
    // shift - 64 = q + 61 - p5b
    a.add_imm(true, Reg::X4, Reg::X21, 61);
    a.sub_r(true, Reg::X4, Reg::X4, Reg::X3);
    a.str_uimm(true, Reg::X4, Reg::SP, 56);

    // Trailing-zero check on mv
    let lbl_neg_q_gt1 = a.new_label();
    let lbl_neg_tz_done = a.new_label();
    a.cmp_imm(true, Reg::X21, 1);
    a.b_cond(Cond::Gt, lbl_neg_q_gt1);
    a.mov_imm32(Reg::X3, 2);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vr_tz
    let lbl_neg_no_ab = a.new_label();
    a.mov_imm32(Reg::X3, 1);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_neg_no_ab);
    let lbl_neg_set_vm = a.new_label();
    a.cbz(true, Reg::X21, lbl_neg_set_vm);
    a.mov_imm32(Reg::X3, 8);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_neg_tz_done);
    a.bind(lbl_neg_set_vm);
    a.mov_imm32(Reg::X3, 4);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vm_tz
    a.b(lbl_neg_tz_done);
    a.bind(lbl_neg_no_ab);
    a.mov_imm32(Reg::X3, 16);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // dec_vp
    a.b(lbl_neg_tz_done);

    a.bind(lbl_neg_q_gt1);
    a.cmp_imm(true, Reg::X21, 63);
    a.b_cond(Cond::Ge, lbl_neg_tz_done);
    a.mov_imm32(Reg::X3, 1);
    a.lslv(true, Reg::X3, Reg::X3, Reg::X21);
    a.sub_imm(true, Reg::X3, Reg::X3, 1);
    a.ldr_uimm(true, Reg::X4, Reg::SP, 16);
    a.and_r(true, Reg::X3, Reg::X3, Reg::X4);
    a.cbnz(true, Reg::X3, lbl_neg_tz_done);
    a.mov_imm32(Reg::X3, 2);
    a.orr_r(true, Reg::X20, Reg::X20, Reg::X3); // vr_tz

    a.bind(lbl_neg_tz_done);
    a.str_uimm(true, Reg::X20, Reg::SP, 48);
    // Load POW5_SPLIT[idx] into (X21, X22)
    a.adr_reloc(Reg::X5, A64RelocTarget::LocalRodata(pow_tbl_off));
    a.lsl_imm(true, Reg::X6, Reg::X22, 4);
    a.add_r(true, Reg::X5, Reg::X5, Reg::X6);
    a.ldr_uimm(true, Reg::X21, Reg::X5, 0);
    a.ldr_uimm(true, Reg::X22, Reg::X5, 8);
    a.ldr_uimm(true, Reg::X23, Reg::SP, 56);
    emit_mul_shift_all3(&mut a);

    // ---- Common post-mul digit reduction ----
    // X20 = flags, X21 = vr, X22 = vp, X23 = vm, X19 = removed, X24 = last_removed, X15 = 10
    a.bind(lbl_after_mul);
    a.ldr_uimm(true, Reg::X20, Reg::SP, 48);
    a.ldr_uimm(true, Reg::X21, Reg::SP, 16); // vr
    a.ldr_uimm(true, Reg::X22, Reg::SP, 24); // vp
    a.ldr_uimm(true, Reg::X23, Reg::SP, 32); // vm
    let lbl_no_dec_vp = a.new_label();
    a.mov_imm32(Reg::X3, 16);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_no_dec_vp);
    a.sub_imm(true, Reg::X22, Reg::X22, 1);
    a.bind(lbl_no_dec_vp);

    a.movz(true, Reg::X19, 0, 0); // removed = 0
    a.movz(true, Reg::X24, 0, 0); // last_removed = 0
    a.mov_imm32(Reg::X15, 10);

    let lbl_fast_red = a.new_label();
    let lbl_got_output = a.new_label();
    a.mov_imm32(Reg::X3, 6);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_fast_red);

    // Slow/exact trailing-zero reduction loop
    let lbl_tz_lp = a.new_label();
    let lbl_tz_lp_done = a.new_label();
    a.bind(lbl_tz_lp);
    a.udiv(true, Reg::X0, Reg::X22, Reg::X15); // X0 = vp / 10
    a.udiv(true, Reg::X1, Reg::X23, Reg::X15); // X1 = vm / 10
    a.msub(true, Reg::X2, Reg::X1, Reg::X15, Reg::X23); // X2 = vm % 10
    a.cmp_r(true, Reg::X0, Reg::X1);
    a.b_cond(Cond::Ls, lbl_tz_lp_done);
    let lbl_tz_k1 = a.new_label();
    a.cbz(true, Reg::X2, lbl_tz_k1);
    a.mov_imm64(Reg::X3, !4u64);
    a.and_r(true, Reg::X20, Reg::X20, Reg::X3);
    a.bind(lbl_tz_k1);
    let lbl_tz_k2 = a.new_label();
    a.cbz(true, Reg::X24, lbl_tz_k2);
    a.mov_imm64(Reg::X3, !2u64);
    a.and_r(true, Reg::X20, Reg::X20, Reg::X3);
    a.bind(lbl_tz_k2);
    a.udiv(true, Reg::X3, Reg::X21, Reg::X15);
    a.msub(true, Reg::X24, Reg::X3, Reg::X15, Reg::X21);
    a.mov_r(true, Reg::X21, Reg::X3);
    a.mov_r(true, Reg::X22, Reg::X0);
    a.mov_r(true, Reg::X23, Reg::X1);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.b(lbl_tz_lp);

    a.bind(lbl_tz_lp_done);
    let lbl_vm_tz_done = a.new_label();
    a.mov_imm32(Reg::X3, 4);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_vm_tz_done);
    let lbl_vm_lp = a.new_label();
    a.bind(lbl_vm_lp);
    a.cbz(true, Reg::X23, lbl_vm_tz_done);
    a.udiv(true, Reg::X1, Reg::X23, Reg::X15);
    a.msub(true, Reg::X2, Reg::X1, Reg::X15, Reg::X23);
    a.cbnz(true, Reg::X2, lbl_vm_tz_done);
    a.mov_r(true, Reg::X23, Reg::X1);
    let lbl_vm_k = a.new_label();
    a.cbz(true, Reg::X24, lbl_vm_k);
    a.mov_imm64(Reg::X3, !2u64);
    a.and_r(true, Reg::X20, Reg::X20, Reg::X3);
    a.bind(lbl_vm_k);
    a.udiv(true, Reg::X3, Reg::X21, Reg::X15);
    a.msub(true, Reg::X24, Reg::X3, Reg::X15, Reg::X21);
    a.mov_r(true, Reg::X21, Reg::X3);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.b(lbl_vm_lp);

    a.bind(lbl_vm_tz_done);
    let lbl_no_tie = a.new_label();
    a.mov_imm32(Reg::X3, 2);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cbz(true, Reg::X4, lbl_no_tie);
    a.cmp_imm(true, Reg::X24, 5);
    a.b_cond(Cond::Ne, lbl_no_tie);
    a.mov_imm32(Reg::X3, 1);
    a.and_r(true, Reg::X4, Reg::X21, Reg::X3);
    a.cbnz(true, Reg::X4, lbl_no_tie);
    a.mov_imm32(Reg::X24, 4);
    a.bind(lbl_no_tie);

    a.mov_r(true, Reg::X0, Reg::X21);
    let lbl_tz_inc = a.new_label();
    a.cmp_imm(true, Reg::X24, 5);
    a.b_cond(Cond::Hs, lbl_tz_inc);
    a.cmp_r(true, Reg::X21, Reg::X23);
    a.b_cond(Cond::Ne, lbl_got_output);
    a.mov_imm32(Reg::X3, 5);
    a.and_r(true, Reg::X4, Reg::X20, Reg::X3);
    a.cmp_imm(true, Reg::X4, 5);
    a.b_cond(Cond::Eq, lbl_got_output);
    a.bind(lbl_tz_inc);
    a.add_imm(true, Reg::X0, Reg::X0, 1);
    a.b(lbl_got_output);

    // Fast reduction loop (!vr_tz && !vm_tz)
    a.bind(lbl_fast_red);
    let lbl_f_lp = a.new_label();
    let lbl_f_done = a.new_label();
    a.bind(lbl_f_lp);
    a.udiv(true, Reg::X0, Reg::X22, Reg::X15);
    a.udiv(true, Reg::X1, Reg::X23, Reg::X15);
    a.cmp_r(true, Reg::X0, Reg::X1);
    a.b_cond(Cond::Ls, lbl_f_done);
    a.udiv(true, Reg::X3, Reg::X21, Reg::X15);
    a.msub(true, Reg::X24, Reg::X3, Reg::X15, Reg::X21);
    a.mov_r(true, Reg::X21, Reg::X3);
    a.mov_r(true, Reg::X22, Reg::X0);
    a.mov_r(true, Reg::X23, Reg::X1);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.b(lbl_f_lp);
    a.bind(lbl_f_done);
    a.mov_r(true, Reg::X0, Reg::X21);
    let lbl_f_inc = a.new_label();
    a.cmp_imm(true, Reg::X24, 5);
    a.b_cond(Cond::Hs, lbl_f_inc);
    a.cmp_r(true, Reg::X21, Reg::X23);
    a.b_cond(Cond::Ne, lbl_got_output);
    a.bind(lbl_f_inc);
    a.add_imm(true, Reg::X0, Reg::X0, 1);

    // ---- Format `output` (X0) + Goose layout ----
    a.bind(lbl_got_output);
    a.ldr_uimm(true, Reg::X20, Reg::SP, 40); // e10
    a.add_r(true, Reg::X20, Reg::X20, Reg::X19); // exp10 = e10 + removed
    emit_fmt_u64_to_buf(&mut a, Reg::X0, 96); // X4 = digits ptr, X5 = orig_len
    // first_exp = exp10 + orig_len - 1 -> X20
    a.add_r(true, Reg::X20, Reg::X20, Reg::X5);
    a.sub_imm(true, Reg::X20, Reg::X20, 1);
    // Trim trailing '0's (count -> X21, digits ptr -> X22)
    a.mov_r(true, Reg::X21, Reg::X5);
    a.mov_r(true, Reg::X22, Reg::X4);
    let lbl_trim = a.new_label();
    let lbl_trim_done = a.new_label();
    a.bind(lbl_trim);
    a.cmp_imm(true, Reg::X21, 1);
    a.b_cond(Cond::Le, lbl_trim_done);
    a.sub_imm(true, Reg::X3, Reg::X21, 1);
    a.ldrb_reg(Reg::X2, Reg::X22, Reg::X3, false);
    a.cmp_imm(false, Reg::X2, u16::from(b'0'));
    a.b_cond(Cond::Ne, lbl_trim_done);
    a.mov_r(true, Reg::X21, Reg::X3);
    a.b(lbl_trim);
    a.bind(lbl_trim_done);

    // Goose writer: X19 = out_cur, X23 = orig_out_ptr, X22 = digits, X21 = count, X20 = first_exp
    a.ldr_uimm(true, Reg::X19, Reg::SP, 0);
    a.mov_r(true, Reg::X23, Reg::X19);
    let lbl_no_minus = a.new_label();
    a.ldr_uimm(true, Reg::X3, Reg::SP, 8);
    a.cbz(true, Reg::X3, lbl_no_minus);
    a.mov_imm32(Reg::X3, u32::from(b'-'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.bind(lbl_no_minus);

    let emit_copy_n = |a: &mut A64Asm, src: Reg, cnt: Reg| {
        let lp = a.new_label();
        let dn = a.new_label();
        a.bind(lp);
        a.cmp_imm(true, cnt, 0);
        a.b_cond(Cond::Le, dn);
        a.ldrb_uimm(Reg::X3, src, 0);
        a.strb_uimm(Reg::X3, Reg::X19, 0);
        a.add_imm(true, src, src, 1);
        a.add_imm(true, Reg::X19, Reg::X19, 1);
        a.sub_imm(true, cnt, cnt, 1);
        a.b(lp);
        a.bind(dn);
    };
    let emit_zeros_n = |a: &mut A64Asm, cnt: Reg| {
        let lp = a.new_label();
        let dn = a.new_label();
        a.mov_imm32(Reg::X3, u32::from(b'0'));
        a.bind(lp);
        a.cmp_imm(true, cnt, 0);
        a.b_cond(Cond::Le, dn);
        a.strb_uimm(Reg::X3, Reg::X19, 0);
        a.add_imm(true, Reg::X19, Reg::X19, 1);
        a.sub_imm(true, cnt, cnt, 1);
        a.b(lp);
        a.bind(dn);
    };

    // max_exp = max(count, 15) -> X24
    a.mov_imm32(Reg::X24, 15);
    a.cmp_r(true, Reg::X21, Reg::X24);
    a.csel(true, Reg::X24, Reg::X21, Reg::X24, Cond::Gt);

    let lbl_sci = a.new_label();
    let lbl_not_sci = a.new_label();
    let lbl_layout_done = a.new_label();
    a.mov_imm64(Reg::X3, (-4i64) as u64);
    a.cmp_r(true, Reg::X20, Reg::X3);
    a.b_cond(Cond::Lt, lbl_sci);
    a.cmp_r(true, Reg::X20, Reg::X24);
    a.b_cond(Cond::Lt, lbl_not_sci);

    // Scientific: d[.ddd]e[+-]dd[d]
    a.bind(lbl_sci);
    a.ldrb_uimm(Reg::X3, Reg::X22, 0);
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X22, Reg::X22, 1);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.sub_imm(true, Reg::X21, Reg::X21, 1);
    let lbl_sci_e = a.new_label();
    a.cmp_imm(true, Reg::X21, 0);
    a.b_cond(Cond::Le, lbl_sci_e);
    a.mov_imm32(Reg::X3, u32::from(b'.'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    emit_copy_n(&mut a, Reg::X22, Reg::X21);
    a.bind(lbl_sci_e);
    a.mov_imm32(Reg::X3, u32::from(b'e'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    let lbl_epos = a.new_label();
    let lbl_edig = a.new_label();
    a.cmp_imm(true, Reg::X20, 0);
    a.b_cond(Cond::Ge, lbl_epos);
    a.mov_imm32(Reg::X3, u32::from(b'-'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.neg_r(true, Reg::X20, Reg::X20);
    a.b(lbl_edig);
    a.bind(lbl_epos);
    a.mov_imm32(Reg::X3, u32::from(b'+'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.bind(lbl_edig);
    let lbl_e_ge10 = a.new_label();
    a.cmp_imm(true, Reg::X20, 10);
    a.b_cond(Cond::Hs, lbl_e_ge10);
    a.mov_imm32(Reg::X3, u32::from(b'0'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(false, Reg::X20, Reg::X20, u16::from(b'0'));
    a.strb_uimm(Reg::X20, Reg::X19, 1);
    a.add_imm(true, Reg::X19, Reg::X19, 2);
    a.b(lbl_layout_done);
    a.bind(lbl_e_ge10);
    emit_fmt_u64_to_buf(&mut a, Reg::X20, 112); // X4 = ptr, X5 = len
    emit_copy_n(&mut a, Reg::X4, Reg::X5);
    a.b(lbl_layout_done);

    // Fixed layouts
    a.bind(lbl_not_sci);
    let lbl_ge0 = a.new_label();
    a.cmp_imm(true, Reg::X20, 0);
    a.b_cond(Cond::Ge, lbl_ge0);
    a.mov_imm32(Reg::X3, u32::from(b'0'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.mov_imm32(Reg::X3, u32::from(b'.'));
    a.strb_uimm(Reg::X3, Reg::X19, 1);
    a.add_imm(true, Reg::X19, Reg::X19, 2);
    a.neg_r(true, Reg::X20, Reg::X20);
    a.sub_imm(true, Reg::X20, Reg::X20, 1);
    emit_zeros_n(&mut a, Reg::X20);
    emit_copy_n(&mut a, Reg::X22, Reg::X21);
    a.b(lbl_layout_done);

    a.bind(lbl_ge0);
    a.add_imm(true, Reg::X20, Reg::X20, 1); // whole = first_exp + 1
    let lbl_split = a.new_label();
    a.cmp_r(true, Reg::X21, Reg::X20);
    a.b_cond(Cond::Gt, lbl_split);
    a.sub_r(true, Reg::X20, Reg::X20, Reg::X21);
    emit_copy_n(&mut a, Reg::X22, Reg::X21);
    emit_zeros_n(&mut a, Reg::X20);
    a.mov_imm32(Reg::X3, u32::from(b'.'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.mov_imm32(Reg::X3, u32::from(b'0'));
    a.strb_uimm(Reg::X3, Reg::X19, 1);
    a.add_imm(true, Reg::X19, Reg::X19, 2);
    a.b(lbl_layout_done);

    a.bind(lbl_split);
    a.sub_r(true, Reg::X21, Reg::X21, Reg::X20);
    emit_copy_n(&mut a, Reg::X22, Reg::X20);
    a.mov_imm32(Reg::X3, u32::from(b'.'));
    a.strb_uimm(Reg::X3, Reg::X19, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    emit_copy_n(&mut a, Reg::X22, Reg::X21);

    a.bind(lbl_layout_done);
    a.sub_r(true, Reg::X0, Reg::X19, Reg::X23);

    a.bind(lbl_ret);
    a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 128);
    a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 144);
    a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 160);
    a.ldp_x(Reg::FP, Reg::LR, Reg::SP, 176);
    a.add_imm(true, Reg::SP, Reg::SP, 192);
    a.ret();

    a.finish()?;
    Ok(a)
}

/// Emits a direct-syscall AArch64 import trampoline for `Target::StandaloneExec`, matching
/// `x64::standalone_rt::emit_standalone_import_trampoline` across WASI, Goose, and `env` imports.
pub fn emit_a64_standalone_import_trampoline(
    module: &X64Module,
    k: usize,
) -> Result<A64Asm, String> {
    let (ref mname, ref fname) = module.imports[k];
    let ty = module
        .func_type(k)
        .ok_or_else(|| format!("aarch64: missing import type {k}"))?;
    let is_mem64 = module.memory64;
    let mut a = A64Asm::new();

    let zext_param = |a: &mut A64Asm, reg: Reg, idx: usize| {
        if ty.params.get(idx) == Some(&ValType::I32) {
            a.uxtw(reg, reg);
        }
    };

    match (mname.as_str(), fname.as_str()) {
        ("wasi_snapshot_preview1", "proc_exit") | ("goose", "exit") => {
            if ty.params.first() == Some(&ValType::I32) {
                a.sxtw(Reg::X0, Reg::X0);
            }
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                OFF_EXEC_EXIT_CODE,
                SGP3,
            );
            a.mov_imm32(SGP1, u32::from(TRAP_EXIT));
            a.ret();
        }
        ("goose", "newline") => {
            let nl = a.add_local_rodata(1, b"\n");
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(nl));
            a.mov_imm32(Reg::X13, 1);
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("goose", "print_bool") => {
            let s_true = a.add_local_rodata(1, b"true");
            let s_false = a.add_local_rodata(1, b"false");
            let lbl_false = a.new_label();
            let lbl_go = a.new_label();
            a.cbz(ty.params.first() == Some(&ValType::I64), Reg::X0, lbl_false);
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(s_true));
            a.mov_imm32(Reg::X13, 4);
            a.b(lbl_go);
            a.bind(lbl_false);
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(s_false));
            a.mov_imm32(Reg::X13, 5);
            a.bind(lbl_go);
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("goose", "print_u64") => {
            zext_param(&mut a, Reg::X0, 0);
            a.sub_imm(true, Reg::SP, Reg::SP, 48);
            emit_fmt_u64_to_buf(&mut a, Reg::X0, 32); // X4 = ptr, X5 = len
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X4, Reg::X5);
            a.add_imm(true, Reg::SP, Reg::SP, 48);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("goose", "print_i64") => {
            if ty.params.first() == Some(&ValType::I32) {
                a.sxtw(Reg::X0, Reg::X0);
            }
            a.sub_imm(true, Reg::SP, Reg::SP, 48);
            let lbl_pos = a.new_label();
            let lbl_wr = a.new_label();
            a.mov_r(true, Reg::X12, Reg::X0); // save sign
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Ge, lbl_pos);
            a.neg_r(true, Reg::X0, Reg::X0);
            a.bind(lbl_pos);
            emit_fmt_u64_to_buf(&mut a, Reg::X0, 32); // X4 = ptr, X5 = len
            a.cmp_imm(true, Reg::X12, 0);
            a.b_cond(Cond::Ge, lbl_wr);
            a.sub_imm(true, Reg::X4, Reg::X4, 1);
            a.mov_imm32(Reg::X3, u32::from(b'-'));
            a.strb_uimm(Reg::X3, Reg::X4, 0);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.bind(lbl_wr);
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X4, Reg::X5);
            a.add_imm(true, Reg::SP, Reg::SP, 48);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("goose", "print_f64") | ("goose", "print_f32") => {
            let is_f64 = fname == "print_f64";
            if is_f64 && ty.params.first() == Some(&ValType::F32) {
                a.fcvt_d_s(VReg::V0, VReg::V0);
            } else if !is_f64 && ty.params.first() == Some(&ValType::F64) {
                a.fcvt_s_d(VReg::V0, VReg::V0);
            }
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -64);
            a.add_imm(true, Reg::X0, Reg::SP, 16);
            a.mov_imm32(Reg::X1, u32::from(is_f64));
            a.bl_reloc(A64RelocTarget::Helper(HELPER_GOOSE_FMT_FLOAT));
            a.add_imm(true, Reg::X12, Reg::SP, 16);
            a.mov_r(true, Reg::X13, Reg::X0);
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 64);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("goose", "format_f64") | ("goose", "format_f32") => {
            let is_f64 = fname == "format_f64";
            zext_param(&mut a, Reg::X0, 0);
            if is_f64 && ty.params.get(1) == Some(&ValType::F32) {
                a.fcvt_d_s(VReg::V0, VReg::V0);
            } else if !is_f64 && ty.params.get(1) == Some(&ValType::F64) {
                a.fcvt_s_d(VReg::V0, VReg::V0);
            }
            let lbl_oob = a.new_label();
            a.mov_imm32(Reg::X1, 32);
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
            a.add_r(true, Reg::X0, MEM_BASE_REG, Reg::X0);
            a.mov_imm32(Reg::X1, u32::from(is_f64));
            a.bl_reloc(A64RelocTarget::Helper(HELPER_GOOSE_FMT_FLOAT));
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "bench_input") => {
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -32);
            let lbl_skip_ws = a.new_label();
            let lbl_read_first = a.new_label();
            let lbl_have_first = a.new_label();
            let lbl_digits = a.new_label();
            let lbl_got_digit = a.new_label();
            let lbl_done = a.new_label();
            let lbl_fail = a.new_label();

            let emit_read1 = |a: &mut A64Asm| {
                let lp = a.new_label();
                a.bind(lp);
                a.movz(true, Reg::X0, 0, 0); // fd = 0
                a.add_imm(true, Reg::X1, Reg::SP, 16);
                a.mov_imm32(Reg::X2, 1);
                emit_syscall(a, SYS_READ);
                a.mov_imm64(Reg::X3, (-4i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X3);
                a.b_cond(Cond::Eq, lp);
            };

            a.movz(true, Reg::X12, 0, 0); // neg
            a.movz(true, Reg::X13, 0, 0); // acc
            a.mov_imm32(Reg::X14, 10);

            a.bind(lbl_skip_ws);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.cmp_imm(false, Reg::X9, u16::from(b' '));
            a.b_cond(Cond::Eq, lbl_skip_ws);
            a.sub_imm(false, Reg::X10, Reg::X9, 9);
            a.cmp_imm(false, Reg::X10, 5);
            a.b_cond(Cond::Lo, lbl_skip_ws);

            let lbl_not_minus = a.new_label();
            a.cmp_imm(false, Reg::X9, u16::from(b'-'));
            a.b_cond(Cond::Ne, lbl_not_minus);
            a.mov_imm32(Reg::X12, 1);
            a.b(lbl_read_first);
            a.bind(lbl_not_minus);
            a.cmp_imm(false, Reg::X9, u16::from(b'+'));
            a.b_cond(Cond::Ne, lbl_have_first);

            a.bind(lbl_read_first);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);

            a.bind(lbl_have_first);
            a.sub_imm(false, Reg::X9, Reg::X9, u16::from(b'0'));
            a.cmp_imm(false, Reg::X9, 10);
            a.b_cond(Cond::Hs, lbl_fail);
            a.b(lbl_got_digit);

            a.bind(lbl_digits);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_done);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.sub_imm(false, Reg::X9, Reg::X9, u16::from(b'0'));
            a.cmp_imm(false, Reg::X9, 10);
            a.b_cond(Cond::Hs, lbl_done);

            a.bind(lbl_got_digit);
            a.mul(true, Reg::X13, Reg::X13, Reg::X14);
            a.uxtw(Reg::X9, Reg::X9);
            a.add_r(true, Reg::X13, Reg::X13, Reg::X9);
            a.b(lbl_digits);

            a.bind(lbl_done);
            a.mov_r(true, Reg::X0, Reg::X13);
            let lbl_pos = a.new_label();
            a.cbz(false, Reg::X12, lbl_pos);
            a.neg_r(true, Reg::X0, Reg::X0);
            a.bind(lbl_pos);
            if ty.results.first() == Some(&ValType::I32) {
                a.uxtw(Reg::X0, Reg::X0);
            }
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 32);
            a.movz(false, SGP1, 0, 0);
            a.ret();

            a.bind(lbl_fail);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 32);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "bench_input_f64") => {
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -64);
            let ten_off = a.add_local_rodata(8, &10.0f64.to_bits().to_le_bytes());
            let emit_read1 = |a: &mut A64Asm| {
                let lp = a.new_label();
                a.bind(lp);
                a.movz(true, Reg::X0, 0, 0);
                a.add_imm(true, Reg::X1, Reg::SP, 16);
                a.mov_imm32(Reg::X2, 1);
                emit_syscall(a, SYS_READ);
                a.mov_imm64(Reg::X3, (-4i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X3);
                a.b_cond(Cond::Eq, lp);
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

            // [SP+24]=neg, [SP+32]=exp10 (i64), [SP+40]=any_digit, X12=mant, X13=mant_digits, X14=10
            a.str_uimm(true, Reg::XZR, Reg::SP, 24);
            a.str_uimm(true, Reg::XZR, Reg::SP, 32);
            a.str_uimm(true, Reg::XZR, Reg::SP, 40);
            a.movz(true, Reg::X12, 0, 0);
            a.movz(true, Reg::X13, 0, 0);
            a.mov_imm32(Reg::X14, 10);

            a.bind(lbl_ws);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.cmp_imm(false, Reg::X9, u16::from(b' '));
            a.b_cond(Cond::Eq, lbl_ws);
            a.sub_imm(false, Reg::X10, Reg::X9, 9);
            a.cmp_imm(false, Reg::X10, 5);
            a.b_cond(Cond::Lo, lbl_ws);

            let lbl_not_m = a.new_label();
            a.cmp_imm(false, Reg::X9, u16::from(b'-'));
            a.b_cond(Cond::Ne, lbl_not_m);
            a.mov_imm32(Reg::X3, 1);
            a.str_uimm(true, Reg::X3, Reg::SP, 24);
            a.b(lbl_read_after_sign);
            a.bind(lbl_not_m);
            a.cmp_imm(false, Reg::X9, u16::from(b'+'));
            a.b_cond(Cond::Ne, lbl_after_sign);

            a.bind(lbl_read_after_sign);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);

            a.bind(lbl_after_sign);
            let lbl_check_dot = a.new_label();
            a.bind(lbl_int_lp);
            a.sub_imm(false, Reg::X10, Reg::X9, u16::from(b'0'));
            a.cmp_imm(false, Reg::X10, 10);
            a.b_cond(Cond::Hs, lbl_check_dot);
            a.mov_imm32(Reg::X3, 1);
            a.str_uimm(true, Reg::X3, Reg::SP, 40);
            let lbl_int_ovf = a.new_label();
            let lbl_int_next = a.new_label();
            a.cmp_imm(true, Reg::X13, 18);
            a.b_cond(Cond::Hs, lbl_int_ovf);
            a.mul(true, Reg::X12, Reg::X12, Reg::X14);
            a.uxtw(Reg::X10, Reg::X10);
            a.add_r(true, Reg::X12, Reg::X12, Reg::X10);
            a.add_imm(true, Reg::X13, Reg::X13, 1);
            a.b(lbl_int_next);
            a.bind(lbl_int_ovf);
            a.ldr_uimm(true, Reg::X3, Reg::SP, 32);
            a.add_imm(true, Reg::X3, Reg::X3, 1);
            a.str_uimm(true, Reg::X3, Reg::SP, 32);
            a.bind(lbl_int_next);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_scale);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.b(lbl_int_lp);

            a.bind(lbl_check_dot);
            let lbl_check_exp = a.new_label();
            a.cmp_imm(false, Reg::X9, u16::from(b'.'));
            a.b_cond(Cond::Ne, lbl_check_exp);
            a.bind(lbl_frac_lp);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_scale);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.sub_imm(false, Reg::X10, Reg::X9, u16::from(b'0'));
            a.cmp_imm(false, Reg::X10, 10);
            a.b_cond(Cond::Hs, lbl_check_exp);
            a.mov_imm32(Reg::X3, 1);
            a.str_uimm(true, Reg::X3, Reg::SP, 40);
            a.cmp_imm(true, Reg::X13, 18);
            a.b_cond(Cond::Hs, lbl_frac_lp);
            a.mul(true, Reg::X12, Reg::X12, Reg::X14);
            a.uxtw(Reg::X10, Reg::X10);
            a.add_r(true, Reg::X12, Reg::X12, Reg::X10);
            a.add_imm(true, Reg::X13, Reg::X13, 1);
            a.ldr_uimm(true, Reg::X3, Reg::SP, 32);
            a.sub_imm(true, Reg::X3, Reg::X3, 1);
            a.str_uimm(true, Reg::X3, Reg::SP, 32);
            a.b(lbl_frac_lp);

            a.bind(lbl_check_exp);
            a.cmp_imm(false, Reg::X9, u16::from(b'e'));
            a.b_cond(Cond::Eq, lbl_exp_sign);
            a.cmp_imm(false, Reg::X9, u16::from(b'E'));
            a.b_cond(Cond::Ne, lbl_scale);
            a.bind(lbl_exp_sign);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.movz(true, Reg::X6, 0, 0); // exp_neg
            a.movz(true, Reg::X7, 0, 0); // exp_val
            let lbl_e_not_m = a.new_label();
            let lbl_e_have = a.new_label();
            a.cmp_imm(false, Reg::X9, u16::from(b'-'));
            a.b_cond(Cond::Ne, lbl_e_not_m);
            a.mov_imm32(Reg::X6, 1);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.b(lbl_e_have);
            a.bind(lbl_e_not_m);
            a.cmp_imm(false, Reg::X9, u16::from(b'+'));
            a.b_cond(Cond::Ne, lbl_e_have);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_fail);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.bind(lbl_e_have);
            a.bind(lbl_exp_lp);
            a.sub_imm(false, Reg::X10, Reg::X9, u16::from(b'0'));
            a.cmp_imm(false, Reg::X10, 10);
            let lbl_exp_done = a.new_label();
            a.b_cond(Cond::Hs, lbl_exp_done);
            a.mul(true, Reg::X7, Reg::X7, Reg::X14);
            a.uxtw(Reg::X10, Reg::X10);
            a.add_r(true, Reg::X7, Reg::X7, Reg::X10);
            emit_read1(&mut a);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Le, lbl_exp_done);
            a.ldrb_uimm(Reg::X9, Reg::SP, 16);
            a.b(lbl_exp_lp);
            a.bind(lbl_exp_done);
            let lbl_e_pos = a.new_label();
            a.cbz(false, Reg::X6, lbl_e_pos);
            a.neg_r(true, Reg::X7, Reg::X7);
            a.bind(lbl_e_pos);
            a.ldr_uimm(true, Reg::X3, Reg::SP, 32);
            a.add_r(true, Reg::X3, Reg::X3, Reg::X7);
            a.str_uimm(true, Reg::X3, Reg::SP, 32);

            a.bind(lbl_scale);
            a.ldr_uimm(true, Reg::X3, Reg::SP, 40);
            a.cbz(true, Reg::X3, lbl_fail);
            a.ucvtf(true, true, VReg::V0, Reg::X12);
            a.adr_reloc(Reg::X3, A64RelocTarget::LocalRodata(ten_off));
            a.ldr_d_uimm(VReg::V1, Reg::X3, 0);
            a.ldr_uimm(true, Reg::X4, Reg::SP, 32);
            a.cbz(true, Reg::X4, lbl_finish);
            a.cmp_imm(true, Reg::X4, 0);
            a.b_cond(Cond::Lt, lbl_scale_neg);
            a.bind(lbl_scale_pos);
            a.fmul(true, VReg::V0, VReg::V0, VReg::V1);
            a.sub_imm(true, Reg::X4, Reg::X4, 1);
            a.cbnz(true, Reg::X4, lbl_scale_pos);
            a.b(lbl_finish);
            a.bind(lbl_scale_neg);
            a.neg_r(true, Reg::X4, Reg::X4);
            let lbl_sn_lp = a.new_label();
            a.bind(lbl_sn_lp);
            a.fdiv(true, VReg::V0, VReg::V0, VReg::V1);
            a.sub_imm(true, Reg::X4, Reg::X4, 1);
            a.cbnz(true, Reg::X4, lbl_sn_lp);

            a.bind(lbl_finish);
            let lbl_ret_ok = a.new_label();
            a.ldr_uimm(true, Reg::X3, Reg::SP, 24);
            a.cbz(true, Reg::X3, lbl_ret_ok);
            a.fneg(true, VReg::V0, VReg::V0);
            a.bind(lbl_ret_ok);
            if ty.results.first() == Some(&ValType::F32) {
                a.fcvt_s_d(VReg::V0, VReg::V0);
            }
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 64);
            a.movz(false, SGP1, 0, 0);
            a.ret();

            a.bind(lbl_fail);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 64);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("goose", "write") => {
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            let lbl_oob = a.new_label();
            let lbl_ok = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.cbz(true, Reg::X1, lbl_ok);
            a.add_r(true, Reg::X12, MEM_BASE_REG, Reg::X0);
            a.mov_r(true, Reg::X13, Reg::X1);
            a.mov_imm32(Reg::X14, 1);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.bind(lbl_ok);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("goose", "abort") => {
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            a.sub_imm(true, Reg::SP, Reg::SP, 64);
            a.str_uimm(true, Reg::X0, Reg::SP, 0);
            a.str_uimm(true, Reg::X1, Reg::SP, 8);
            a.str_uimm(true, Reg::X2, Reg::SP, 16);
            let b1 = b"goose runtime error: ";
            let b2 = b" (line ";
            let b3 = b")\n";
            let p1 = a.add_local_rodata(1, b1);
            let p2 = a.add_local_rodata(1, b2);
            let p3 = a.add_local_rodata(1, b3);
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(p1));
            a.mov_imm32(Reg::X13, b1.len() as u32);
            a.mov_imm32(Reg::X14, 2);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            let lbl_skip_msg = a.new_label();
            a.ldr_uimm(true, Reg::X0, Reg::SP, 0);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 8);
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_skip_msg);
            a.cbz(true, Reg::X1, lbl_skip_msg);
            a.add_r(true, Reg::X12, MEM_BASE_REG, Reg::X0);
            a.mov_r(true, Reg::X13, Reg::X1);
            a.mov_imm32(Reg::X14, 2);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.bind(lbl_skip_msg);
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(p2));
            a.mov_imm32(Reg::X13, b2.len() as u32);
            a.mov_imm32(Reg::X14, 2);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 16);
            emit_fmt_u64_to_buf(&mut a, Reg::X0, 64);
            a.mov_imm32(Reg::X14, 2);
            emit_write_all(&mut a, Reg::X14, Reg::X4, Reg::X5);
            a.adr_reloc(Reg::X12, A64RelocTarget::LocalRodata(p3));
            a.mov_imm32(Reg::X13, b3.len() as u32);
            a.mov_imm32(Reg::X14, 2);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.add_imm(true, Reg::SP, Reg::SP, 64);
            a.mov_imm32(Reg::X3, 1);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_EXIT_CODE,
                SGP3,
            );
            a.mov_imm32(SGP1, u32::from(TRAP_EXIT));
            a.ret();
        }
        ("env", "gs_os_write_stdout") | ("env", "gs_os_write_stderr") => {
            let fd = if fname == "gs_os_write_stdout" { 1 } else { 2 };
            if ty.params.len() == 1 {
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
            }
            let lbl_oob = a.new_label();
            let lbl_ok = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.cbz(true, Reg::X1, lbl_ok);
            a.add_r(true, Reg::X12, MEM_BASE_REG, Reg::X0);
            a.mov_r(true, Reg::X13, Reg::X1);
            a.mov_imm32(Reg::X14, fd);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.bind(lbl_ok);
            a.movz(true, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_clock_ns" | "bench_clock" | "gs_os_time_ns") => {
            let lbl_allowed = a.new_label();
            emit_check_clock_policy(&mut a);
            a.b_cond(Cond::Ne, lbl_allowed);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_allowed);
            a.sub_imm(true, Reg::SP, Reg::SP, 32);
            if fname == "gs_os_time_ns" {
                emit_clock_ns(&mut a, CLOCK_REALTIME, 0);
            } else {
                emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
                emit_load_off(
                    &mut a,
                    Ty::I64,
                    Some(Reg::X3),
                    None,
                    CTX_REG,
                    OFF_EXEC_CLOCK_ORIGIN,
                    SGP3,
                );
                a.sub_r(true, Reg::X0, Reg::X0, Reg::X3);
            }
            emit_coarsen_clock_ns(&mut a);
            a.add_imm(true, Reg::SP, Reg::SP, 32);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "gs_os_arg_count") => {
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X0),
                None,
                CTX_REG,
                OFF_EXEC_ARGC,
                SGP3,
            );
            a.add_imm(true, Reg::X0, Reg::X0, 1);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "gs_os_arg") => {
            if ty.params.first() == Some(&ValType::I32) {
                a.sxtw(Reg::X0, Reg::X0);
            }
            zext_param(&mut a, Reg::X1, 1);
            let lbl_noop = a.new_label();
            let lbl_prog = a.new_label();
            let lbl_got = a.new_label();
            let lbl_sl = a.new_label();
            let lbl_sl_done = a.new_label();
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_noop);
            a.b_cond(Cond::Eq, lbl_prog);
            a.sub_imm(true, Reg::X0, Reg::X0, 1);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X2),
                None,
                CTX_REG,
                OFF_EXEC_ARGC,
                SGP3,
            );
            a.cmp_r(true, Reg::X0, Reg::X2);
            a.b_cond(Cond::Hs, lbl_noop);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_ARGV,
                SGP3,
            );
            a.ldr_reg_scaled(true, Reg::X4, Reg::X3, Reg::X0, false);
            a.b(lbl_got);
            a.bind(lbl_prog);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                OFF_EXEC_PROG_NAME,
                SGP3,
            );
            a.bind(lbl_got);
            a.movz(true, Reg::X2, 0, 0);
            a.bind(lbl_sl);
            a.ldrb_reg(Reg::X3, Reg::X4, Reg::X2, false);
            a.cbz(false, Reg::X3, lbl_sl_done);
            a.add_imm(true, Reg::X2, Reg::X2, 1);
            a.b(lbl_sl);
            a.bind(lbl_sl_done);
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
            a.mov_r(true, Reg::X0, Reg::X1); // control
            a.mov_r(true, Reg::X1, Reg::X4); // host_ptr
            // X2 = len
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
            a.bind(lbl_noop);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "gs_os_getenv") => {
            if ty.params.len() == 2 {
                zext_param(&mut a, Reg::X1, 1);
                a.mov_r(true, Reg::X2, Reg::X1);
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
                zext_param(&mut a, Reg::X2, 2);
            }
            let lbl_oob = a.new_label();
            let lbl_ret0 = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -48);
            a.add_r(true, Reg::X9, MEM_BASE_REG, Reg::X0);
            a.str_uimm(true, Reg::X9, Reg::SP, 16); // name_host
            a.str_uimm(true, Reg::X1, Reg::SP, 24); // name_len
            a.str_uimm(true, Reg::X2, Reg::SP, 32); // control
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X10),
                None,
                CTX_REG,
                OFF_EXEC_POLICY_PTR,
                SGP3,
            );
            a.cbz(true, Reg::X10, lbl_ret0);
            let lbl_allowed = a.new_label();
            a.ldr_uimm(true, Reg::X3, Reg::X10, POLICY_OFF_ALLOW_ALL_ENV as u32);
            a.cbnz(true, Reg::X3, lbl_allowed);
            a.ldr_uimm(true, Reg::X11, Reg::X10, POLICY_OFF_ENV_COUNT as u32);
            a.add_imm(true, Reg::X12, Reg::X10, POLICY_OFF_ENV_PTRS as u16);
            a.movz(true, Reg::X13, 0, 0);
            let lbl_chk = a.new_label();
            let lbl_cmp = a.new_label();
            let lbl_next = a.new_label();
            let lbl_end = a.new_label();
            a.bind(lbl_chk);
            a.cmp_r(true, Reg::X13, Reg::X11);
            a.b_cond(Cond::Hs, lbl_ret0);
            a.ldr_reg_scaled(true, Reg::X3, Reg::X12, Reg::X13, false);
            a.movz(true, Reg::X4, 0, 0);
            a.bind(lbl_cmp);
            a.cmp_r(true, Reg::X4, Reg::X1);
            a.b_cond(Cond::Hs, lbl_end);
            a.ldrb_reg(Reg::X5, Reg::X3, Reg::X4, false);
            a.ldrb_reg(Reg::X6, Reg::X9, Reg::X4, false);
            a.cmp_r(false, Reg::X5, Reg::X6);
            a.b_cond(Cond::Ne, lbl_next);
            a.add_imm(true, Reg::X4, Reg::X4, 1);
            a.b(lbl_cmp);
            a.bind(lbl_end);
            a.ldrb_reg(Reg::X5, Reg::X3, Reg::X4, false);
            a.cbz(false, Reg::X5, lbl_allowed);
            a.bind(lbl_next);
            a.add_imm(true, Reg::X13, Reg::X13, 1);
            a.b(lbl_chk);

            a.bind(lbl_allowed);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X10),
                None,
                CTX_REG,
                OFF_EXEC_ENVP,
                SGP3,
            );
            a.cbz(true, Reg::X10, lbl_ret0);
            a.ldr_uimm(true, Reg::X9, Reg::SP, 16);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 24);
            let lbl_ep = a.new_label();
            let lbl_ecmp = a.new_label();
            let lbl_ehit = a.new_label();
            let lbl_enext = a.new_label();
            a.bind(lbl_ep);
            a.ldr_uimm(true, Reg::X3, Reg::X10, 0);
            a.cbz(true, Reg::X3, lbl_ret0);
            a.movz(true, Reg::X4, 0, 0);
            a.bind(lbl_ecmp);
            a.cmp_r(true, Reg::X4, Reg::X1);
            a.b_cond(Cond::Hs, lbl_ehit);
            a.ldrb_reg(Reg::X5, Reg::X3, Reg::X4, false);
            a.ldrb_reg(Reg::X6, Reg::X9, Reg::X4, false);
            a.cmp_r(false, Reg::X5, Reg::X6);
            a.b_cond(Cond::Ne, lbl_enext);
            a.add_imm(true, Reg::X4, Reg::X4, 1);
            a.b(lbl_ecmp);
            a.bind(lbl_ehit);
            a.ldrb_reg(Reg::X5, Reg::X3, Reg::X4, false);
            a.cmp_imm(false, Reg::X5, u16::from(b'='));
            a.b_cond(Cond::Ne, lbl_enext);
            // Found: val_ptr = X3 + X1 + 1
            a.add_r(true, Reg::X1, Reg::X3, Reg::X1);
            a.add_imm(true, Reg::X1, Reg::X1, 1);
            a.movz(true, Reg::X2, 0, 0);
            let lbl_vl = a.new_label();
            let lbl_vld = a.new_label();
            a.bind(lbl_vl);
            a.ldrb_reg(Reg::X5, Reg::X1, Reg::X2, false);
            a.cbz(false, Reg::X5, lbl_vld);
            a.add_imm(true, Reg::X2, Reg::X2, 1);
            a.b(lbl_vl);
            a.bind(lbl_vld);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 32); // control
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
            a.mov_imm32(Reg::X0, 1);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 48);
            a.ret();

            a.bind(lbl_enext);
            a.add_imm(true, Reg::X10, Reg::X10, 8);
            a.b(lbl_ep);

            a.bind(lbl_ret0);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 48);
            a.movz(true, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_file_exists") | ("env", "gs_os_is_dir") => {
            let mode = if fname == "gs_os_file_exists" {
                0u32
            } else {
                1u32
            };
            if ty.params.len() == 1 {
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
            }
            let lbl_oob = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -16);
            a.mov_imm32(Reg::X2, mode);
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_FS));
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 16);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_read_file") => {
            if ty.params.len() == 2 {
                zext_param(&mut a, Reg::X1, 1);
                a.mov_r(true, Reg::X2, Reg::X1);
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
                zext_param(&mut a, Reg::X2, 2);
            }
            let lbl_oob = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            // Frame: 224B: [SP+0]=FP/LR, [SP+16]=control, [SP+24]=fd, [SP+32]=buf, [SP+40]=cap, [SP+48]=total, [SP+64..192]=stat
            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -224);
            let lbl_ret0 = a.new_label();
            let lbl_close_ret0 = a.new_label();
            let lbl_unmap_ret0 = a.new_label();
            let lbl_trap_ret = a.new_label();
            a.str_uimm(true, Reg::X2, Reg::SP, 16);
            a.mov_imm32(Reg::X2, 2); // open_read
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_FS));
            a.cbnz(false, SGP1, lbl_trap_ret);
            // Re-entrancy guard check
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_RESERVING,
                SGP3,
            );
            let lbl_not_res = a.new_label();
            a.cbz(true, Reg::X3, lbl_not_res);
            let lbl_skip_cl = a.new_label();
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_skip_cl);
            emit_syscall(&mut a, SYS_CLOSE);
            a.bind(lbl_skip_cl);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.b(lbl_trap_ret);

            a.bind(lbl_not_res);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_ret0);
            a.str_uimm(true, Reg::X0, Reg::SP, 24); // fd
            a.add_imm(true, Reg::X1, Reg::SP, 64);
            emit_syscall(&mut a, SYS_FSTAT);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_close_ret0);
            a.ldr_uimm(false, Reg::X2, Reg::SP, (64 + A64_STAT_OFF_MODE) as u32);
            a.mov_imm32(Reg::X3, 0xf000);
            a.and_r(false, Reg::X2, Reg::X2, Reg::X3);
            a.mov_imm32(Reg::X3, 0x4000);
            a.cmp_r(false, Reg::X2, Reg::X3);
            a.b_cond(Cond::Eq, lbl_close_ret0);
            a.ldr_uimm(true, Reg::X4, Reg::SP, (64 + A64_STAT_OFF_SIZE) as u32);
            a.cmp_imm(true, Reg::X4, 0);
            a.b_cond(Cond::Lt, lbl_close_ret0);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X5),
                None,
                CTX_REG,
                OFF_EXEC_OUTPUT_LIMIT,
                SGP3,
            );
            a.cmp_r(true, Reg::X4, Reg::X5);
            a.b_cond(Cond::Hi, lbl_close_ret0);
            let lbl_nonempty = a.new_label();
            a.cbnz(true, Reg::X4, lbl_nonempty);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 24);
            emit_syscall(&mut a, SYS_CLOSE);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 16);
            a.add_imm(true, Reg::X1, Reg::SP, 64);
            a.movz(true, Reg::X2, 0, 0);
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
            a.mov_imm32(Reg::X0, 1);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 224);
            a.ret();

            a.bind(lbl_nonempty);
            a.add_imm_lsl12(true, Reg::X4, Reg::X4, 1); // +4096
            a.mov_imm64(Reg::X5, !4095u64);
            a.and_r(true, Reg::X4, Reg::X4, Reg::X5);
            a.str_uimm(true, Reg::X4, Reg::SP, 40); // cap
            a.movz(true, Reg::X0, 0, 0);
            a.mov_r(true, Reg::X1, Reg::X4);
            a.mov_imm32(Reg::X2, 3);
            a.mov_imm32(Reg::X3, 0x22);
            a.mov_imm64(Reg::X4, u64::MAX);
            a.movz(true, Reg::X5, 0, 0);
            emit_syscall(&mut a, SYS_MMAP);
            a.mov_imm64(Reg::X4, (-4095i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X4);
            a.b_cond(Cond::Hs, lbl_close_ret0);
            a.str_uimm(true, Reg::X0, Reg::SP, 32); // buf
            a.str_uimm(true, Reg::XZR, Reg::SP, 48); // total

            let lbl_rlp = a.new_label();
            let lbl_reof = a.new_label();
            a.bind(lbl_rlp);
            a.ldr_uimm(true, Reg::X9, Reg::SP, 48);
            a.ldr_uimm(true, Reg::X2, Reg::SP, 40);
            a.subs_r(true, Reg::X2, Reg::X2, Reg::X9);
            a.b_cond(Cond::Ls, lbl_unmap_ret0);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 32);
            a.add_r(true, Reg::X1, Reg::X1, Reg::X9);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 24);
            emit_syscall(&mut a, SYS_READ);
            a.mov_imm64(Reg::X3, (-4i64) as u64);
            a.cmp_r(true, Reg::X0, Reg::X3);
            a.b_cond(Cond::Eq, lbl_rlp);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_unmap_ret0);
            a.cbz(true, Reg::X0, lbl_reof);
            a.ldr_uimm(true, Reg::X9, Reg::SP, 48);
            a.add_r(true, Reg::X9, Reg::X9, Reg::X0);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X5),
                None,
                CTX_REG,
                OFF_EXEC_OUTPUT_LIMIT,
                SGP3,
            );
            a.cmp_r(true, Reg::X9, Reg::X5);
            a.b_cond(Cond::Hi, lbl_unmap_ret0);
            a.str_uimm(true, Reg::X9, Reg::SP, 48);
            a.b(lbl_rlp);

            a.bind(lbl_reof);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 24);
            emit_syscall(&mut a, SYS_CLOSE);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 16);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 32);
            a.ldr_uimm(true, Reg::X2, Reg::SP, 48);
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
            a.mov_r(false, Reg::X12, SGP1); // save trap code
            a.ldr_uimm(true, Reg::X0, Reg::SP, 32);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 40);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.mov_r(false, SGP1, Reg::X12);
            a.mov_imm32(Reg::X0, 1);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 224);
            a.ret();

            a.bind(lbl_unmap_ret0);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 32);
            a.ldr_uimm(true, Reg::X1, Reg::SP, 40);
            emit_syscall(&mut a, SYS_MUNMAP);
            a.bind(lbl_close_ret0);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 24);
            emit_syscall(&mut a, SYS_CLOSE);
            a.bind(lbl_ret0);
            a.movz(true, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.bind(lbl_trap_ret);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 224);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_write_file") | ("env", "gs_os_append_file") => {
            let mode = if fname == "gs_os_write_file" {
                3u32
            } else {
                4u32
            };
            if ty.params.len() == 2 {
                a.lsr_imm(true, Reg::X3, Reg::X1, 32);
                a.uxtw(Reg::X2, Reg::X1);
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
                zext_param(&mut a, Reg::X2, 2);
                zext_param(&mut a, Reg::X3, 3);
            }
            let lbl_oob = a.new_label();
            let lbl_ret0 = a.new_label();
            let lbl_trap_ret = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            emit_check_range(&mut a, Reg::X2, Reg::X3, lbl_oob);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                OFF_EXEC_WRITTEN,
                SGP3,
            );
            a.adds_r(true, Reg::X4, Reg::X4, Reg::X3);
            a.b_cond(Cond::Hs, lbl_oob);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X5),
                None,
                CTX_REG,
                OFF_EXEC_OUTPUT_LIMIT,
                SGP3,
            );
            a.cmp_r(true, Reg::X4, Reg::X5);
            a.b_cond(Cond::Hi, lbl_oob);
            emit_store_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                OFF_EXEC_WRITTEN,
                SGP3,
            );

            a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -48);
            a.str_uimm(true, Reg::X2, Reg::SP, 16);
            a.str_uimm(true, Reg::X3, Reg::SP, 24);
            a.mov_imm32(Reg::X2, mode);
            a.bl_reloc(A64RelocTarget::Helper(HELPER_STANDALONE_FS));
            a.cbnz(false, SGP1, lbl_trap_ret);
            a.cmp_imm(true, Reg::X0, 0);
            a.b_cond(Cond::Lt, lbl_ret0);
            a.str_uimm(true, Reg::X0, Reg::SP, 32); // fd
            a.mov_r(true, Reg::X14, Reg::X0);
            a.ldr_uimm(true, Reg::X12, Reg::SP, 16);
            a.add_r(true, Reg::X12, MEM_BASE_REG, Reg::X12);
            a.ldr_uimm(true, Reg::X13, Reg::SP, 24);
            emit_write_all(&mut a, Reg::X14, Reg::X12, Reg::X13);
            a.ldr_uimm(true, Reg::X0, Reg::SP, 32);
            emit_syscall(&mut a, SYS_CLOSE);
            a.cmp_imm(true, Reg::X13, 0);
            a.mov_imm32(Reg::X1, 1);
            a.csel(true, Reg::X0, Reg::X1, Reg::XZR, Cond::Eq);
            a.movz(false, SGP1, 0, 0);
            a.b(lbl_trap_ret);

            a.bind(lbl_ret0);
            a.movz(true, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.bind(lbl_trap_ret);
            a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 48);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_find_any") => {
            if ty.params.len() == 2 {
                zext_param(&mut a, Reg::X1, 1);
                a.mov_r(true, Reg::X2, Reg::X1);
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
                zext_param(&mut a, Reg::X2, 2);
            }
            let lbl_oob = a.new_label();
            let lbl_lp = a.new_label();
            let lbl_nf = a.new_label();
            let lbl_found = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.mov_imm32(Reg::X3, 256);
            emit_check_range(&mut a, Reg::X2, Reg::X3, lbl_oob);
            a.add_r(true, Reg::X4, MEM_BASE_REG, Reg::X0);
            a.add_r(true, Reg::X5, MEM_BASE_REG, Reg::X2);
            a.movz(true, Reg::X0, 0, 0);
            a.bind(lbl_lp);
            a.cmp_r(true, Reg::X0, Reg::X1);
            a.b_cond(Cond::Hs, lbl_nf);
            a.ldrb_reg(Reg::X6, Reg::X4, Reg::X0, false);
            a.ldrb_reg(Reg::X7, Reg::X5, Reg::X6, false);
            a.cbnz(false, Reg::X7, lbl_found);
            a.add_imm(true, Reg::X0, Reg::X0, 1);
            a.b(lbl_lp);
            a.bind(lbl_nf);
            a.mov_imm64(Reg::X0, u64::MAX);
            a.bind(lbl_found);
            if ty.results.first() == Some(&ValType::I32) {
                a.uxtw(Reg::X0, Reg::X0);
            }
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_find_pair") => {
            if ty.params.len() == 4 {
                zext_param(&mut a, Reg::X3, 3);
                a.mov_r(true, Reg::X4, Reg::X3);
                if ty.params.get(2) == Some(&ValType::I32) {
                    a.sxtw(Reg::X2, Reg::X2);
                }
                a.mov_r(true, Reg::X3, Reg::X2);
                zext_param(&mut a, Reg::X1, 1);
                a.mov_r(true, Reg::X2, Reg::X1);
                a.lsr_imm(true, Reg::X1, Reg::X0, 32);
                a.uxtw(Reg::X0, Reg::X0);
            } else {
                zext_param(&mut a, Reg::X0, 0);
                zext_param(&mut a, Reg::X1, 1);
                zext_param(&mut a, Reg::X2, 2);
                if ty.params.get(3) == Some(&ValType::I32) {
                    a.sxtw(Reg::X3, Reg::X3);
                }
                zext_param(&mut a, Reg::X4, 4);
            }
            let lbl_oob = a.new_label();
            let lbl_nf = a.new_label();
            let lbl_lp = a.new_label();
            let lbl_step = a.new_label();
            let lbl_found = a.new_label();
            emit_check_range(&mut a, Reg::X0, Reg::X1, lbl_oob);
            a.mov_imm32(Reg::X5, 256);
            emit_check_range(&mut a, Reg::X2, Reg::X5, lbl_oob);
            emit_check_range(&mut a, Reg::X4, Reg::X5, lbl_oob);
            a.cmp_imm(true, Reg::X3, 0);
            a.b_cond(Cond::Lt, lbl_oob);
            a.cmp_r(true, Reg::X3, Reg::X1);
            a.b_cond(Cond::Hs, lbl_nf);
            a.sub_r(true, Reg::X1, Reg::X1, Reg::X3); // limit > 0
            a.add_r(true, Reg::X5, MEM_BASE_REG, Reg::X0); // text1
            a.add_r(true, Reg::X6, Reg::X5, Reg::X3); // text2 = text1 + dist
            a.add_r(true, Reg::X7, MEM_BASE_REG, Reg::X2); // first_tbl
            a.add_r(true, Reg::X9, MEM_BASE_REG, Reg::X4); // second_tbl
            a.movz(true, Reg::X0, 0, 0);
            a.bind(lbl_lp);
            a.ldrb_reg(Reg::X10, Reg::X5, Reg::X0, false);
            a.ldrb_reg(Reg::X11, Reg::X7, Reg::X10, false);
            a.cbz(false, Reg::X11, lbl_step);
            a.ldrb_reg(Reg::X10, Reg::X6, Reg::X0, false);
            a.ldrb_reg(Reg::X11, Reg::X9, Reg::X10, false);
            a.cbnz(false, Reg::X11, lbl_found);
            a.bind(lbl_step);
            a.add_imm(true, Reg::X0, Reg::X0, 1);
            a.cmp_r(true, Reg::X0, Reg::X1);
            a.b_cond(Cond::Lo, lbl_lp);
            a.bind(lbl_nf);
            a.mov_imm64(Reg::X0, u64::MAX);
            a.bind(lbl_found);
            if ty.results.first() == Some(&ValType::I32) {
                a.uxtw(Reg::X0, Reg::X0);
            }
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(SGP1, u32::from(TRAP_HOST));
            a.ret();
        }
        ("env", "gs_os_resource_dir") => {
            a.movz(true, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "system") => {
            a.mov_imm64(Reg::X0, u64::MAX);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "bench_barrier") => {
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("env", "gs_os_flush_stdout" | "bench_sink" | "emscripten_notify_memory_growth") => {
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_close") => {
            let lbl_bad = a.new_label();
            a.cmp_imm(false, Reg::X0, 2);
            a.b_cond(Cond::Hi, lbl_bad);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_bad);
            a.mov_imm32(Reg::X0, 8);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_seek") => {
            let lbl_bad = a.new_label();
            a.cmp_imm(false, Reg::X0, 2);
            a.b_cond(Cond::Hi, lbl_bad);
            a.mov_imm32(Reg::X0, 70);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_bad);
            a.mov_imm32(Reg::X0, 8);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "environ_sizes_get") => {
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            let lbl_oob = a.new_label();
            let w: u32 = if is_mem64 { 8 } else { 4 };
            a.mov_imm32(Reg::X2, w);
            emit_check_range(&mut a, Reg::X0, Reg::X2, lbl_oob);
            emit_check_range(&mut a, Reg::X1, Reg::X2, lbl_oob);
            a.add_r(true, Reg::X3, MEM_BASE_REG, Reg::X0);
            a.str_uimm(is_mem64, Reg::XZR, Reg::X3, 0);
            a.add_r(true, Reg::X3, MEM_BASE_REG, Reg::X1);
            a.str_uimm(is_mem64, Reg::XZR, Reg::X3, 0);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(Reg::X0, 21);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "environ_get") => {
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "args_sizes_get") => {
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            let lbl_oob = a.new_label();
            let w: u32 = if is_mem64 { 8 } else { 4 };
            a.mov_imm32(Reg::X2, w);
            emit_check_range(&mut a, Reg::X0, Reg::X2, lbl_oob);
            emit_check_range(&mut a, Reg::X1, Reg::X2, lbl_oob);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_ARGC,
                SGP3,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                OFF_EXEC_ARGV,
                SGP3,
            );
            a.movz(true, Reg::X5, 0, 0); // total_bytes
            a.movz(true, Reg::X6, 0, 0); // idx
            let lbl_outer = a.new_label();
            let lbl_inner = a.new_label();
            let lbl_done = a.new_label();
            a.bind(lbl_outer);
            a.cmp_r(true, Reg::X6, Reg::X3);
            a.b_cond(Cond::Hs, lbl_done);
            a.lsl_imm(true, Reg::X7, Reg::X6, 3);
            a.add_r(true, Reg::X7, Reg::X4, Reg::X7);
            a.ldr_uimm(true, Reg::X7, Reg::X7, 0);
            a.bind(lbl_inner);
            a.ldrb_uimm(Reg::X9, Reg::X7, 0);
            a.add_imm(true, Reg::X7, Reg::X7, 1);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.cbnz(false, Reg::X9, lbl_inner);
            a.add_imm(true, Reg::X6, Reg::X6, 1);
            a.b(lbl_outer);
            a.bind(lbl_done);
            a.add_r(true, Reg::X7, MEM_BASE_REG, Reg::X0);
            a.str_uimm(is_mem64, Reg::X3, Reg::X7, 0);
            a.add_r(true, Reg::X7, MEM_BASE_REG, Reg::X1);
            a.str_uimm(is_mem64, Reg::X5, Reg::X7, 0);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(Reg::X0, 21);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "args_get") => {
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            let lbl_oob = a.new_label();
            let lbl_outer = a.new_label();
            let lbl_inner = a.new_label();
            let lbl_done = a.new_label();
            let w: u16 = if is_mem64 { 8 } else { 4 };
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_ARGC,
                SGP3,
            );
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X4),
                None,
                CTX_REG,
                OFF_EXEC_ARGV,
                SGP3,
            );
            a.lsl_imm(true, Reg::X2, Reg::X3, if is_mem64 { 3 } else { 2 });
            emit_check_range(&mut a, Reg::X0, Reg::X2, lbl_oob);
            a.movz(true, Reg::X5, 0, 0); // i = 0
            a.bind(lbl_outer);
            a.cmp_r(true, Reg::X5, Reg::X3);
            a.b_cond(Cond::Hs, lbl_done);
            a.add_r(true, Reg::X6, MEM_BASE_REG, Reg::X0);
            a.str_uimm(is_mem64, Reg::X1, Reg::X6, 0);
            a.add_imm(true, Reg::X0, Reg::X0, w);
            a.lsl_imm(true, Reg::X6, Reg::X5, 3);
            a.add_r(true, Reg::X6, Reg::X4, Reg::X6);
            a.ldr_uimm(true, Reg::X6, Reg::X6, 0); // host char*
            a.bind(lbl_inner);
            a.mov_imm32(Reg::X2, 1);
            emit_check_range(&mut a, Reg::X1, Reg::X2, lbl_oob);
            a.ldrb_uimm(Reg::X7, Reg::X6, 0);
            a.strb_reg(Reg::X7, MEM_BASE_REG, Reg::X1, false);
            a.add_imm(true, Reg::X6, Reg::X6, 1);
            a.add_imm(true, Reg::X1, Reg::X1, 1);
            a.cbnz(false, Reg::X7, lbl_inner);
            a.add_imm(true, Reg::X5, Reg::X5, 1);
            a.b(lbl_outer);
            a.bind(lbl_done);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(Reg::X0, 21);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "clock_time_get") => {
            let lbl_denied = a.new_label();
            emit_check_clock_policy(&mut a);
            a.b_cond(Cond::Eq, lbl_denied);
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X2, 2);
            let lbl_oob = a.new_label();
            let lbl_einval = a.new_label();
            let lbl_mono = a.new_label();
            let lbl_store = a.new_label();
            a.mov_imm32(Reg::X3, 8);
            emit_check_range(&mut a, Reg::X2, Reg::X3, lbl_oob);
            a.cmp_imm(false, Reg::X0, 1);
            a.b_cond(Cond::Hi, lbl_einval);
            a.sub_imm(true, Reg::SP, Reg::SP, 32);
            a.str_uimm(true, Reg::X2, Reg::SP, 16);
            a.cmp_imm(false, Reg::X0, 1);
            a.b_cond(Cond::Eq, lbl_mono);
            emit_clock_ns(&mut a, CLOCK_REALTIME, 0);
            a.b(lbl_store);
            a.bind(lbl_mono);
            emit_clock_ns(&mut a, CLOCK_MONOTONIC, 0);
            emit_load_off(
                &mut a,
                Ty::I64,
                Some(Reg::X3),
                None,
                CTX_REG,
                OFF_EXEC_CLOCK_ORIGIN,
                SGP3,
            );
            a.sub_r(true, Reg::X0, Reg::X0, Reg::X3);
            a.bind(lbl_store);
            emit_coarsen_clock_ns(&mut a);
            a.ldr_uimm(true, Reg::X2, Reg::SP, 16);
            a.add_imm(true, Reg::SP, Reg::SP, 32);
            a.add_r(true, Reg::X3, MEM_BASE_REG, Reg::X2);
            a.str_uimm(true, Reg::X0, Reg::X3, 0);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_einval);
            a.mov_imm32(Reg::X0, 28);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_oob);
            a.mov_imm32(Reg::X0, 21);
            a.movz(false, SGP1, 0, 0);
            a.ret();
            a.bind(lbl_denied);
            a.mov_imm32(Reg::X0, 2); // EACCES
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        ("wasi_snapshot_preview1", "fd_write") | ("wasi_snapshot_preview1", "fd_read") => {
            let is_write = fname == "fd_write";
            zext_param(&mut a, Reg::X0, 0);
            zext_param(&mut a, Reg::X1, 1);
            zext_param(&mut a, Reg::X2, 2);
            zext_param(&mut a, Reg::X3, 3);
            a.sub_imm(true, Reg::SP, Reg::SP, 48);
            a.str_uimm(true, Reg::X0, Reg::SP, 0); // fd
            a.str_uimm(true, Reg::X1, Reg::SP, 8); // iovs_off
            a.str_uimm(true, Reg::X2, Reg::SP, 16); // iovs_len
            a.str_uimm(true, Reg::X3, Reg::SP, 24); // n_out_off
            a.str_uimm(true, Reg::XZR, Reg::SP, 32); // total

            let lbl_ebadf = a.new_label();
            let lbl_efault = a.new_label();
            let lbl_loop = a.new_label();
            let lbl_next = a.new_label();
            let lbl_done = a.new_label();

            if is_write {
                a.cmp_imm(false, Reg::X0, 1);
                a.b_cond(Cond::Lo, lbl_ebadf);
                a.cmp_imm(false, Reg::X0, 2);
                a.b_cond(Cond::Hi, lbl_ebadf);
            } else {
                a.cbnz(false, Reg::X0, lbl_ebadf);
            }

            let ptr_w: u32 = if is_mem64 { 8 } else { 4 };
            let iov_sz: u32 = ptr_w * 2;
            a.mov_imm32(Reg::X4, ptr_w);
            emit_check_range(&mut a, Reg::X3, Reg::X4, lbl_efault);

            a.movz(true, Reg::X12, 0, 0); // i = 0
            a.ldr_uimm(true, Reg::X13, Reg::SP, 16); // iovs_len
            a.bind(lbl_loop);
            a.cmp_r(true, Reg::X12, Reg::X13);
            a.b_cond(Cond::Hs, lbl_done);
            a.lsl_imm(true, Reg::X4, Reg::X12, if is_mem64 { 4 } else { 3 });
            a.ldr_uimm(true, Reg::X5, Reg::SP, 8);
            a.adds_r(true, Reg::X4, Reg::X4, Reg::X5);
            a.b_cond(Cond::Hs, lbl_efault);
            a.mov_imm32(Reg::X5, iov_sz);
            emit_check_range(&mut a, Reg::X4, Reg::X5, lbl_efault);

            a.add_r(true, Reg::X5, MEM_BASE_REG, Reg::X4);
            a.ldr_uimm(is_mem64, Reg::X6, Reg::X5, 0); // buf_off
            a.ldr_uimm(is_mem64, Reg::X7, Reg::X5, ptr_w); // buf_len
            emit_check_range(&mut a, Reg::X6, Reg::X7, lbl_efault);
            a.cbz(true, Reg::X7, lbl_next);

            a.add_r(true, Reg::X9, MEM_BASE_REG, Reg::X6); // buf_ptr
            a.mov_r(true, Reg::X10, Reg::X7); // rem_len
            if is_write {
                let lbl_wlp = a.new_label();
                a.bind(lbl_wlp);
                a.cbz(true, Reg::X10, lbl_next);
                a.ldr_uimm(true, Reg::X0, Reg::SP, 0);
                a.mov_r(true, Reg::X1, Reg::X9);
                a.mov_r(true, Reg::X2, Reg::X10);
                emit_syscall(&mut a, SYS_WRITE);
                a.mov_imm64(Reg::X3, (-4i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X3);
                a.b_cond(Cond::Eq, lbl_wlp);
                a.cmp_imm(true, Reg::X0, 0);
                a.b_cond(Cond::Le, lbl_done);
                a.ldr_uimm(true, Reg::X3, Reg::SP, 32);
                a.add_r(true, Reg::X3, Reg::X3, Reg::X0);
                a.str_uimm(true, Reg::X3, Reg::SP, 32);
                a.add_r(true, Reg::X9, Reg::X9, Reg::X0);
                a.sub_r(true, Reg::X10, Reg::X10, Reg::X0);
                a.b(lbl_wlp);
            } else {
                let lbl_rlp = a.new_label();
                a.bind(lbl_rlp);
                a.ldr_uimm(true, Reg::X0, Reg::SP, 0);
                a.mov_r(true, Reg::X1, Reg::X9);
                a.mov_r(true, Reg::X2, Reg::X10);
                emit_syscall(&mut a, SYS_READ);
                a.mov_imm64(Reg::X3, (-4i64) as u64);
                a.cmp_r(true, Reg::X0, Reg::X3);
                a.b_cond(Cond::Eq, lbl_rlp);
                a.cmp_imm(true, Reg::X0, 0);
                a.b_cond(Cond::Le, lbl_done);
                a.ldr_uimm(true, Reg::X3, Reg::SP, 32);
                a.add_r(true, Reg::X3, Reg::X3, Reg::X0);
                a.str_uimm(true, Reg::X3, Reg::SP, 32);
                a.b(lbl_done);
            }

            a.bind(lbl_next);
            a.add_imm(true, Reg::X12, Reg::X12, 1);
            a.b(lbl_loop);

            a.bind(lbl_done);
            a.ldr_uimm(true, Reg::X3, Reg::SP, 24);
            a.ldr_uimm(true, Reg::X4, Reg::SP, 32);
            a.add_r(true, Reg::X5, MEM_BASE_REG, Reg::X3);
            a.str_uimm(is_mem64, Reg::X4, Reg::X5, 0);
            a.add_imm(true, Reg::SP, Reg::SP, 48);
            a.movz(false, Reg::X0, 0, 0);
            a.movz(false, SGP1, 0, 0);
            a.ret();

            a.bind(lbl_ebadf);
            a.add_imm(true, Reg::SP, Reg::SP, 48);
            a.mov_imm32(Reg::X0, 8);
            a.movz(false, SGP1, 0, 0);
            a.ret();

            a.bind(lbl_efault);
            a.add_imm(true, Reg::SP, Reg::SP, 48);
            a.mov_imm32(Reg::X0, 21);
            a.movz(false, SGP1, 0, 0);
            a.ret();
        }
        _ => {
            return Err(format!(
                "aarch64: unsupported standalone import `{mname}::{fname}`"
            ));
        }
    }

    a.finish()?;
    Ok(a)
}

/// Emits `HELPER_STANDALONE_RESERVE_COPY` on AArch64.
/// ABI: `X0 = control`, `X1 = host_src_ptr`, `X2 = byte_len` -> `SGP1 (W16) = 0 | trap`.
pub fn emit_a64_helper_standalone_reserve_copy(module: &X64Module) -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let Some((reserve_fi, is_i64)) = module.standalone_goose_reserve else {
        a.mov_imm32(SGP1, u32::from(TRAP_HOST));
        a.ret();
        return Ok(a);
    };
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -32);
    a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    let lbl_trap_host = a.new_label();
    let lbl_ret = a.new_label();
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X3),
        None,
        CTX_REG,
        OFF_EXEC_RESERVING,
        SGP3,
    );
    a.cbnz(true, Reg::X3, lbl_trap_host);
    a.mov_imm32(Reg::X3, 1);
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::X3),
        None,
        CTX_REG,
        OFF_EXEC_RESERVING,
        SGP3,
    );
    a.mov_r(true, Reg::X19, Reg::X1); // host_src
    a.mov_r(true, Reg::X20, Reg::X2); // byte_len
    a.mov_r(true, Reg::X1, Reg::X2);
    if !is_i64 {
        a.uxtw(Reg::X0, Reg::X0);
        a.uxtw(Reg::X1, Reg::X1);
    }
    a.bl_reloc(A64RelocTarget::Func(reserve_fi));
    emit_store_off(
        &mut a,
        Ty::I64,
        Some(Reg::XZR),
        None,
        CTX_REG,
        OFF_EXEC_RESERVING,
        SGP3,
    );
    a.cbnz(false, SGP1, lbl_ret);
    if !is_i64 {
        a.uxtw(Reg::X0, Reg::X0);
    }
    emit_check_range(&mut a, Reg::X0, Reg::X20, lbl_trap_host);
    a.add_r(true, Reg::X4, MEM_BASE_REG, Reg::X0);
    let lbl_ql = a.new_label();
    let lbl_bl = a.new_label();
    let lbl_done = a.new_label();
    a.bind(lbl_ql);
    a.cmp_imm(true, Reg::X20, 8);
    a.b_cond(Cond::Lo, lbl_bl);
    a.ldr_uimm(true, Reg::X5, Reg::X19, 0);
    a.str_uimm(true, Reg::X5, Reg::X4, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 8);
    a.add_imm(true, Reg::X4, Reg::X4, 8);
    a.sub_imm(true, Reg::X20, Reg::X20, 8);
    a.b(lbl_ql);
    a.bind(lbl_bl);
    a.cbz(true, Reg::X20, lbl_done);
    a.ldrb_uimm(Reg::X5, Reg::X19, 0);
    a.strb_uimm(Reg::X5, Reg::X4, 0);
    a.add_imm(true, Reg::X19, Reg::X19, 1);
    a.add_imm(true, Reg::X4, Reg::X4, 1);
    a.sub_imm(true, Reg::X20, Reg::X20, 1);
    a.b(lbl_bl);
    a.bind(lbl_done);
    a.movz(false, SGP1, 0, 0);
    a.b(lbl_ret);
    a.bind(lbl_trap_host);
    a.mov_imm32(SGP1, u32::from(TRAP_HOST));
    a.bind(lbl_ret);
    a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 32);
    a.ret();
    a.finish()?;
    Ok(a)
}

/// Emits `HELPER_STANDALONE_FS` on AArch64 matching `x64::standalone_rt::emit_helper_standalone_fs`.
/// ABI: `X0 = path_off` (bounds-checked), `X1 = path_len`, `X2 = mode (0..=4)`
/// -> `X0 = 0/1 or fd/-1`, `SGP1 (W16) = 0 | TRAP_HOST`.
pub fn emit_a64_helper_standalone_fs() -> Result<A64Asm, String> {
    let mut a = A64Asm::new();
    let dot_off = a.add_local_rodata(1, b".\0");
    // Frame 96B: [SP+0]=FP/LR, [SP+16]=X19/X20, [SP+32]=X21/X22, [SP+48]=X23/X24, [SP+64]=mode, [SP+72]=norm_len
    a.stp_x_pre(Reg::FP, Reg::LR, Reg::SP, -96);
    a.stp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.stp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.stp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.str_uimm(true, Reg::X2, Reg::SP, 64); // mode
    a.add_r(true, Reg::X19, MEM_BASE_REG, Reg::X0); // path_host
    a.mov_r(true, Reg::X20, Reg::X1); // path_len

    let lbl_deny = a.new_label();
    let lbl_trap_nul = a.new_label();
    let lbl_ret_ok = a.new_label();

    // 1. Embedded NUL scan.
    a.movz(true, Reg::X3, 0, 0);
    let lbl_nul_lp = a.new_label();
    let lbl_nul_ok = a.new_label();
    a.bind(lbl_nul_lp);
    a.cmp_r(true, Reg::X3, Reg::X20);
    a.b_cond(Cond::Hs, lbl_nul_ok);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.cbz(false, Reg::X4, lbl_trap_nul);
    a.add_imm(true, Reg::X3, Reg::X3, 1);
    a.b(lbl_nul_lp);
    a.bind(lbl_nul_ok);

    // 2. Policy & non-empty check.
    emit_load_off(
        &mut a,
        Ty::I64,
        Some(Reg::X21),
        None,
        CTX_REG,
        OFF_EXEC_POLICY_PTR,
        SGP3,
    );
    a.cbz(true, Reg::X21, lbl_deny);
    a.ldr_uimm(true, Reg::X3, Reg::X21, POLICY_OFF_DIR_COUNT as u32);
    a.cbz(true, Reg::X3, lbl_deny);
    a.cbz(true, Reg::X20, lbl_deny);

    // 3. Trailing '/' or '/.' check.
    let lbl_trail_ok = a.new_label();
    let lbl_is_trail = a.new_label();
    a.sub_imm(true, Reg::X3, Reg::X20, 1);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Eq, lbl_is_trail);
    a.cmp_imm(false, Reg::X4, u16::from(b'.'));
    a.b_cond(Cond::Ne, lbl_trail_ok);
    a.cmp_imm(true, Reg::X20, 2);
    a.b_cond(Cond::Lo, lbl_trail_ok);
    a.sub_imm(true, Reg::X3, Reg::X20, 2);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_trail_ok);
    a.bind(lbl_is_trail);
    a.ldr_uimm(true, Reg::X4, Reg::SP, 64);
    a.cbz(true, Reg::X4, lbl_deny);
    a.cmp_imm(true, Reg::X4, 3);
    a.b_cond(Cond::Hs, lbl_deny);
    a.bind(lbl_trail_ok);

    // 4. Lexical normalization into `X22 = policy + POLICY_OFF_SCRATCH_PATH`; X23 = out_len; X24 = src_idx.
    a.mov_imm64(Reg::X22, POLICY_OFF_SCRATCH_PATH as u64);
    a.add_r(true, Reg::X22, Reg::X21, Reg::X22);
    a.movz(true, Reg::X23, 0, 0);
    a.movz(true, Reg::X24, 0, 0);
    let lbl_abs = a.new_label();
    a.ldrb_uimm(Reg::X4, Reg::X19, 0);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Eq, lbl_abs);
    a.ldr_uimm(true, Reg::X23, Reg::X21, POLICY_OFF_CWD_LEN as u32);
    a.cbz(true, Reg::X23, lbl_deny);
    a.cmp_imm(true, Reg::X23, 4095);
    a.b_cond(Cond::Hs, lbl_deny);
    a.add_imm(true, Reg::X3, Reg::X21, POLICY_OFF_CWD_BUF as u16);
    a.movz(true, Reg::X4, 0, 0);
    let lbl_cp_cwd = a.new_label();
    a.bind(lbl_cp_cwd);
    a.ldrb_reg(Reg::X5, Reg::X3, Reg::X4, false);
    a.strb_reg(Reg::X5, Reg::X22, Reg::X4, false);
    a.add_imm(true, Reg::X4, Reg::X4, 1);
    a.cmp_r(true, Reg::X4, Reg::X23);
    a.b_cond(Cond::Lo, lbl_cp_cwd);
    let lbl_cwd_nt = a.new_label();
    a.cmp_imm(true, Reg::X23, 1);
    a.b_cond(Cond::Le, lbl_cwd_nt);
    a.sub_imm(true, Reg::X4, Reg::X23, 1);
    a.ldrb_reg(Reg::X5, Reg::X22, Reg::X4, false);
    a.cmp_imm(false, Reg::X5, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_cwd_nt);
    a.mov_r(true, Reg::X23, Reg::X4);
    a.bind(lbl_cwd_nt);

    a.bind(lbl_abs);
    let lbl_clp = a.new_label();
    let lbl_ssl = a.new_label();
    let lbl_sc = a.new_label();
    let lbl_sclp = a.new_label();
    let lbl_scdn = a.new_label();
    let lbl_ndone = a.new_label();
    a.bind(lbl_clp);
    a.bind(lbl_ssl);
    a.cmp_r(true, Reg::X24, Reg::X20);
    a.b_cond(Cond::Hs, lbl_ndone);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X24, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_sc);
    a.add_imm(true, Reg::X24, Reg::X24, 1);
    a.b(lbl_ssl);

    a.bind(lbl_sc);
    a.mov_r(true, Reg::X3, Reg::X24); // comp_start
    a.bind(lbl_sclp);
    a.cmp_r(true, Reg::X24, Reg::X20);
    a.b_cond(Cond::Hs, lbl_scdn);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X24, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Eq, lbl_scdn);
    a.add_imm(true, Reg::X24, Reg::X24, 1);
    a.b(lbl_sclp);

    a.bind(lbl_scdn);
    a.sub_r(true, Reg::X5, Reg::X24, Reg::X3); // clen
    let lbl_nd1 = a.new_label();
    a.cmp_imm(true, Reg::X5, 1);
    a.b_cond(Cond::Ne, lbl_nd1);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'.'));
    a.b_cond(Cond::Eq, lbl_clp);
    a.bind(lbl_nd1);
    let lbl_nd2 = a.new_label();
    a.cmp_imm(true, Reg::X5, 2);
    a.b_cond(Cond::Ne, lbl_nd2);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'.'));
    a.b_cond(Cond::Ne, lbl_nd2);
    a.add_imm(true, Reg::X6, Reg::X3, 1);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X6, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'.'));
    a.b_cond(Cond::Eq, lbl_deny);
    a.bind(lbl_nd2);

    let lbl_put_sep = a.new_label();
    let lbl_no_sep = a.new_label();
    a.cbz(true, Reg::X23, lbl_put_sep);
    a.sub_imm(true, Reg::X6, Reg::X23, 1);
    a.ldrb_reg(Reg::X4, Reg::X22, Reg::X6, false);
    a.cmp_imm(false, Reg::X4, u16::from(b'/'));
    a.b_cond(Cond::Eq, lbl_no_sep);
    a.bind(lbl_put_sep);
    a.cmp_imm(true, Reg::X23, 4094);
    a.b_cond(Cond::Hs, lbl_deny);
    a.mov_imm32(Reg::X4, u32::from(b'/'));
    a.strb_reg(Reg::X4, Reg::X22, Reg::X23, false);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.bind(lbl_no_sep);

    a.add_r(true, Reg::X6, Reg::X23, Reg::X5);
    a.cmp_imm(true, Reg::X6, 4095);
    a.b_cond(Cond::Hs, lbl_deny);
    let lbl_cpc = a.new_label();
    a.bind(lbl_cpc);
    a.ldrb_reg(Reg::X4, Reg::X19, Reg::X3, false);
    a.strb_reg(Reg::X4, Reg::X22, Reg::X23, false);
    a.add_imm(true, Reg::X3, Reg::X3, 1);
    a.add_imm(true, Reg::X23, Reg::X23, 1);
    a.sub_imm(true, Reg::X5, Reg::X5, 1);
    a.cbnz(true, Reg::X5, lbl_cpc);
    a.b(lbl_clp);

    a.bind(lbl_ndone);
    let lbl_nre = a.new_label();
    a.cbnz(true, Reg::X23, lbl_nre);
    a.mov_imm32(Reg::X4, u32::from(b'/'));
    a.strb_uimm(Reg::X4, Reg::X22, 0);
    a.mov_imm32(Reg::X23, 1);
    a.bind(lbl_nre);
    a.strb_reg(Reg::XZR, Reg::X22, Reg::X23, false);
    a.str_uimm(true, Reg::X23, Reg::SP, 72); // norm_len

    // 5. Loop di (X24) = 0..dir_count.
    a.movz(true, Reg::X24, 0, 0);
    let lbl_dlp = a.new_label();
    let lbl_dnext = a.new_label();
    let lbl_dmatch = a.new_label();
    a.bind(lbl_dlp);
    a.ldr_uimm(true, Reg::X3, Reg::X21, POLICY_OFF_DIR_COUNT as u32);
    a.cmp_r(true, Reg::X24, Reg::X3);
    a.b_cond(Cond::Hs, lbl_deny);
    a.mov_imm32(Reg::X3, POLICY_OFF_DIR_PATH_LENS as u32);
    a.add_r(true, Reg::X3, Reg::X21, Reg::X3);
    a.ldr_reg_scaled(true, Reg::X4, Reg::X3, Reg::X24, false); // dlen
    a.mov_imm64(Reg::X5, POLICY_OFF_DIR_PATHS as u64);
    a.add_r(true, Reg::X5, Reg::X21, Reg::X5);
    a.add_r_lsl(true, Reg::X5, Reg::X5, Reg::X24, 10); // dpath
    a.ldr_uimm(true, Reg::X23, Reg::SP, 72); // norm_len

    let lbl_nrd = a.new_label();
    a.cmp_imm(true, Reg::X4, 1);
    a.b_cond(Cond::Ne, lbl_nrd);
    a.ldrb_uimm(Reg::X6, Reg::X5, 0);
    a.cmp_imm(false, Reg::X6, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_nrd);
    let lbl_rsub = a.new_label();
    a.cmp_imm(true, Reg::X23, 1);
    a.b_cond(Cond::Hi, lbl_rsub);
    a.adr_reloc(Reg::X1, A64RelocTarget::LocalRodata(dot_off));
    a.b(lbl_dmatch);
    a.bind(lbl_rsub);
    a.add_imm(true, Reg::X1, Reg::X22, 1);
    a.b(lbl_dmatch);

    a.bind(lbl_nrd);
    a.cmp_r(true, Reg::X23, Reg::X4);
    a.b_cond(Cond::Lo, lbl_dnext);
    a.movz(true, Reg::X6, 0, 0);
    let lbl_plp = a.new_label();
    let lbl_peq = a.new_label();
    a.bind(lbl_plp);
    a.cmp_r(true, Reg::X6, Reg::X4);
    a.b_cond(Cond::Hs, lbl_peq);
    a.ldrb_reg(Reg::X7, Reg::X22, Reg::X6, false);
    a.ldrb_reg(Reg::X9, Reg::X5, Reg::X6, false);
    a.cmp_r(false, Reg::X7, Reg::X9);
    a.b_cond(Cond::Ne, lbl_dnext);
    a.add_imm(true, Reg::X6, Reg::X6, 1);
    a.b(lbl_plp);
    a.bind(lbl_peq);
    let lbl_psub = a.new_label();
    a.cmp_r(true, Reg::X23, Reg::X4);
    a.b_cond(Cond::Hi, lbl_psub);
    a.adr_reloc(Reg::X1, A64RelocTarget::LocalRodata(dot_off));
    a.b(lbl_dmatch);
    a.bind(lbl_psub);
    a.ldrb_reg(Reg::X7, Reg::X22, Reg::X4, false);
    a.cmp_imm(false, Reg::X7, u16::from(b'/'));
    a.b_cond(Cond::Ne, lbl_dnext);
    a.add_r(true, Reg::X1, Reg::X22, Reg::X4);
    a.add_imm(true, Reg::X1, Reg::X1, 1);

    a.bind(lbl_dmatch);
    // Build open_how at X2 + 0 = X21 + POLICY_OFF_OPEN_HOW
    a.mov_imm64(Reg::X2, POLICY_OFF_OPEN_HOW as u64);
    a.add_r(true, Reg::X2, Reg::X21, Reg::X2);
    a.ldr_uimm(true, Reg::X3, Reg::SP, 64);
    let lbl_m2 = a.new_label();
    let lbl_m3 = a.new_label();
    let lbl_m4 = a.new_label();
    let lbl_hr = a.new_label();
    a.cmp_imm(true, Reg::X3, 2);
    a.b_cond(Cond::Eq, lbl_m2);
    a.cmp_imm(true, Reg::X3, 3);
    a.b_cond(Cond::Eq, lbl_m3);
    a.cmp_imm(true, Reg::X3, 4);
    a.b_cond(Cond::Eq, lbl_m4);
    a.mov_imm64(Reg::X4, u64::from(O_PATH | O_CLOEXEC));
    a.movz(true, Reg::X5, 0, 0);
    a.b(lbl_hr);
    a.bind(lbl_m2);
    a.mov_imm64(Reg::X4, u64::from(O_RDONLY | O_CLOEXEC));
    a.movz(true, Reg::X5, 0, 0);
    a.b(lbl_hr);
    a.bind(lbl_m3);
    a.mov_imm64(
        Reg::X4,
        u64::from(O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW_A64 | O_CLOEXEC),
    );
    a.mov_imm32(Reg::X5, 0o644);
    a.b(lbl_hr);
    a.bind(lbl_m4);
    a.mov_imm64(
        Reg::X4,
        u64::from(O_WRONLY | O_CREAT | O_APPEND | O_NOFOLLOW_A64 | O_CLOEXEC),
    );
    a.mov_imm32(Reg::X5, 0o644);

    a.bind(lbl_hr);
    a.str_uimm(true, Reg::X4, Reg::X2, 0);
    a.str_uimm(true, Reg::X5, Reg::X2, 8);
    a.mov_imm64(Reg::X6, RESOLVE_BENEATH);
    a.str_uimm(true, Reg::X6, Reg::X2, 16);
    a.mov_imm32(Reg::X3, POLICY_OFF_DIR_FDS as u32);
    a.add_r(true, Reg::X3, Reg::X21, Reg::X3);
    a.ldr_reg_scaled(true, Reg::X0, Reg::X3, Reg::X24, false);
    // X1 = rel_path, X2 = &open_how
    a.mov_imm32(Reg::X3, 24);
    emit_syscall(&mut a, SYS_OPENAT2);
    a.cmp_imm(true, Reg::X0, 0);
    a.b_cond(Cond::Lt, lbl_dnext);

    a.ldr_uimm(true, Reg::X3, Reg::SP, 64);
    a.cmp_imm(true, Reg::X3, 2);
    a.b_cond(Cond::Hs, lbl_ret_ok);

    // mode 0/1: fstat(fd, &stat_buf), close(fd), test S_IFREG / S_IFDIR
    a.mov_r(true, Reg::X19, Reg::X0); // fd
    a.mov_imm64(Reg::X1, POLICY_OFF_STAT_BUF as u64);
    a.add_r(true, Reg::X1, Reg::X21, Reg::X1);
    emit_syscall(&mut a, SYS_FSTAT);
    a.mov_r(true, Reg::X20, Reg::X0);
    a.mov_r(true, Reg::X0, Reg::X19);
    emit_syscall(&mut a, SYS_CLOSE);
    a.cmp_imm(true, Reg::X20, 0);
    a.b_cond(Cond::Lt, lbl_deny);
    a.mov_imm64(Reg::X1, (POLICY_OFF_STAT_BUF + A64_STAT_OFF_MODE) as u64);
    a.add_r(true, Reg::X1, Reg::X21, Reg::X1);
    a.ldr_uimm(false, Reg::X0, Reg::X1, 0);
    a.mov_imm32(Reg::X2, 0xf000);
    a.and_r(false, Reg::X0, Reg::X0, Reg::X2);
    a.ldr_uimm(true, Reg::X3, Reg::SP, 64);
    let lbl_wdir = a.new_label();
    let lbl_syes = a.new_label();
    a.cbnz(true, Reg::X3, lbl_wdir);
    a.mov_imm32(Reg::X2, 0x8000);
    a.cmp_r(false, Reg::X0, Reg::X2);
    a.b_cond(Cond::Eq, lbl_syes);
    a.b(lbl_deny);
    a.bind(lbl_wdir);
    a.mov_imm32(Reg::X2, 0x4000);
    a.cmp_r(false, Reg::X0, Reg::X2);
    a.b_cond(Cond::Ne, lbl_deny);
    a.bind(lbl_syes);
    a.mov_imm32(Reg::X0, 1);
    a.b(lbl_ret_ok);

    a.bind(lbl_dnext);
    a.add_imm(true, Reg::X24, Reg::X24, 1);
    a.b(lbl_dlp);

    a.bind(lbl_trap_nul);
    a.ldr_uimm(true, Reg::X3, Reg::SP, 64);
    a.cmp_imm(true, Reg::X3, 2);
    a.b_cond(Cond::Lo, lbl_deny);
    a.mov_imm32(SGP1, u32::from(TRAP_HOST));
    let lbl_ep = a.new_label();
    a.b(lbl_ep);

    a.bind(lbl_deny);
    a.ldr_uimm(true, Reg::X3, Reg::SP, 64);
    a.movz(true, Reg::X0, 0, 0);
    a.mov_imm64(Reg::X4, u64::MAX);
    a.cmp_imm(true, Reg::X3, 2);
    a.csel(true, Reg::X0, Reg::X4, Reg::X0, Cond::Hs);

    a.bind(lbl_ret_ok);
    a.movz(false, SGP1, 0, 0);
    a.bind(lbl_ep);
    a.ldp_x(Reg::X19, Reg::X20, Reg::SP, 16);
    a.ldp_x(Reg::X21, Reg::X22, Reg::SP, 32);
    a.ldp_x(Reg::X23, Reg::X24, Reg::SP, 48);
    a.ldp_x_post(Reg::FP, Reg::LR, Reg::SP, 96);
    a.ret();

    a.finish()?;
    Ok(a)
}
