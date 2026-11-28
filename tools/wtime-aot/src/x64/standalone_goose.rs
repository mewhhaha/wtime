//! Additional Goose host imports for the standalone Linux executable.

use super::{
    Asm, Cond, FS_OPEN_DIR_PARENT, FS_OPEN_FILE_PARENT, FS_OPEN_LIST, Gp, HELPER_GOOSE_FMT_FLOAT,
    HELPER_STANDALONE_FS, HELPER_STANDALONE_RESERVE_COPY, Mem, O_CLOEXEC, O_CREAT, O_EXCL,
    O_NOFOLLOW_X64, O_WRONLY, OFF_EXEC_OUTPUT_LIMIT, OFF_EXEC_POLICY_PTR, OFF_EXEC_WRITTEN,
    POLICY_OFF_DIR_COUNT, RelocTarget, SYS_CLOSE, SYS_GETCWD, SYS_MMAP, SYS_MUNMAP, SYS_READ,
    TRAP_HOST, Xmm, emit_check_range, emit_write_all,
};

const SYS_NANOSLEEP: u32 = 35;
const SYS_FSYNC: u32 = 74;
const SYS_GETDENTS64: u32 = 217;
const SYS_MKDIRAT: u32 = 258;
const SYS_UNLINKAT: u32 = 263;
const SYS_RENAMEAT: u32 = 264;
const SYS_GETRANDOM: u32 = 318;
const AT_REMOVEDIR: u32 = 0x200;

fn syscall(a: &mut Asm, number: u32) {
    a.mov_ri32(Gp::Rax, number);
    a.syscall();
}

fn complete(a: &mut Asm) {
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
}

fn trap(a: &mut Asm) {
    a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
    a.ret();
}

fn stack(offset: i32) -> Mem {
    Mem::base_disp(Gp::Rsp, offset)
}

pub(super) fn unpack_path(a: &mut Asm, packed: bool, zext: impl Fn(&mut Asm, Gp, usize)) {
    if packed {
        a.mov_rr(true, Gp::Rsi, Gp::Rdi);
        a.shr_imm(true, Gp::Rsi, 32);
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
    } else {
        zext(a, Gp::Rdi, 0);
        zext(a, Gp::Rsi, 1);
    }
}

/// Within the FS helper, split the grant-relative path. It restores the
/// slash after openat2 and returns the leaf separately from the pinned fd.
pub(super) fn emit_split_parent(a: &mut Asm, dot_off: u32, split_at: Mem, basename: Mem) {
    let scan = a.new_label();
    let next = a.new_label();
    let split = a.new_label();
    let leaf_only = a.new_label();
    let done = a.new_label();
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(scan);
    a.movzx_rm8(false, Gp::R8, Mem::base(Gp::Rax));
    a.test_rr(false, Gp::R8, Gp::R8);
    a.jcc(Cond::E, split);
    a.cmp_ri(false, Gp::R8, i32::from(b'/'));
    a.jcc(Cond::Ne, next);
    a.mov_rr(true, Gp::Rcx, Gp::Rax);
    a.bind(next);
    a.add_ri(true, Gp::Rax, 1);
    a.jmp(scan);
    a.bind(split);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, leaf_only);
    a.mov_mi8(Mem::base(Gp::Rcx), 0);
    a.mov_mr(true, split_at, Gp::Rcx);
    a.add_ri(true, Gp::Rcx, 1);
    a.mov_mr(true, basename, Gp::Rcx);
    a.jmp(done);
    a.bind(leaf_only);
    a.mov_mr(true, basename, Gp::Rsi);
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(dot_off));
    a.bind(done);
}

pub(super) fn emit_getcwd(a: &mut Asm, fixed_mem: bool) {
    let fail = a.new_label();
    let oob = a.new_label();
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, fail);
    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::R15, OFF_EXEC_POLICY_PTR));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, fail);
    a.mov_rm(true, Gp::Rdx, Mem::base_disp(Gp::Rdx, POLICY_OFF_DIR_COUNT));
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, fail);
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.mov_rr(true, Gp::R8, Gp::Rdi);
    a.add_rr(true, Gp::Rdi, Gp::R14);
    syscall(a, SYS_GETCWD);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Le, fail);
    a.mov_rr(true, Gp::Rax, Gp::R8);
    complete(a);
    a.bind(fail);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    complete(a);
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_make_dir(a: &mut Asm, fixed_mem: bool) {
    let done = a.new_label();
    let retry_stat = a.new_label();
    let oob = a.new_label();
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.sub_ri(true, Gp::Rsp, 24);
    a.mov_mr(true, stack(0), Gp::Rdi);
    a.mov_mr(true, stack(8), Gp::Rsi);
    a.mov_ri32(Gp::Rdx, 1);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, done);
    a.mov_rm(true, Gp::Rdi, stack(0));
    a.mov_rm(true, Gp::Rsi, stack(8));
    a.mov_ri32(Gp::Rdx, FS_OPEN_DIR_PARENT);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, done);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, retry_stat);
    a.mov_mr(true, stack(16), Gp::Rax);
    a.mov_rr(true, Gp::Rdi, Gp::Rax);
    a.mov_rr(true, Gp::Rsi, Gp::Rdx);
    a.mov_ri32(Gp::Rdx, 0o777);
    syscall(a, SYS_MKDIRAT);
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.mov_rm(true, Gp::Rdi, stack(16));
    syscall(a, SYS_CLOSE);
    a.mov_ri32(Gp::Rax, 1);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::E, done);
    // If another caller created it, confirm it is a directory inside the grant.
    a.bind(retry_stat);
    a.mov_rm(true, Gp::Rdi, stack(0));
    a.mov_rm(true, Gp::Rsi, stack(8));
    a.mov_ri32(Gp::Rdx, 1);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.bind(done);
    a.add_ri(true, Gp::Rsp, 24);
    a.ret();
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_atomic_write(a: &mut Asm, fixed_mem: bool) {
    let oob = a.new_label();
    let deny = a.new_label();
    let close_parent = a.new_label();
    let remove_temp = a.new_label();
    let leave = a.new_label();
    let random = a.new_label();
    let sync = a.new_label();
    let close_file = a.new_label();
    let hex = a.new_label();
    let prefix = a.add_local_rodata(1, b".goose-tmp-");
    let digits = a.add_local_rodata(1, b"0123456789abcdef");

    // Validate the same guest memory and account for the same output as write_file.
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    emit_check_range(a, Gp::Rdx, Gp::Rcx, fixed_mem, oob);
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_EXEC_WRITTEN));
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.jcc(Cond::B, oob);
    a.cmp_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::R15, OFF_EXEC_OUTPUT_LIMIT),
    );
    a.jcc(Cond::A, oob);
    a.mov_mr(true, Mem::base_disp(Gp::R15, OFF_EXEC_WRITTEN), Gp::Rax);

    // Stack slots: content offset/len, basename, random bytes, temporary basename.
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, 72);
    a.mov_mr(true, stack(0), Gp::Rdx);
    a.mov_mr(true, stack(8), Gp::Rcx);
    a.mov_ri32(Gp::Rdx, FS_OPEN_FILE_PARENT);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, deny);
    a.mov_rr(true, Gp::Rbx, Gp::Rax);
    a.mov_mr(true, stack(16), Gp::Rdx);

    // Make a unique sibling and only ever unlink a temporary file we created.
    a.bind(random);
    a.lea(true, Gp::Rdi, stack(24));
    a.mov_ri32(Gp::Rsi, 8);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    syscall(a, SYS_GETRANDOM);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, random);
    a.cmp_ri(true, Gp::Rax, 8);
    a.jcc(Cond::Ne, close_parent);
    a.lea(true, Gp::Rdi, stack(32));
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(prefix));
    a.mov_ri32(Gp::Rcx, 11);
    a.rep_movsb();
    a.lea_rip(Gp::Rsi, RelocTarget::LocalRodata(digits));
    a.mov_rm(true, Gp::Rax, stack(24));
    a.mov_ri32(Gp::Rcx, 16);
    a.bind(hex);
    a.mov_rr(false, Gp::Rdx, Gp::Rax);
    a.and_ri(false, Gp::Rdx, 15);
    a.movzx_rm8(false, Gp::Rdx, Mem::indexed(Gp::Rsi, Gp::Rdx, 1, 0));
    a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rdx);
    a.add_ri(true, Gp::Rdi, 1);
    a.shr_imm(true, Gp::Rax, 4);
    a.sub_ri(false, Gp::Rcx, 1);
    a.jcc(Cond::Ne, hex);
    a.mov_mi8(Mem::base(Gp::Rdi), 0);

    // The destination must never be the temporary name, even when it is absent.
    let same_name = a.new_label();
    let open_temp = a.new_label();
    a.mov_rm(true, Gp::Rsi, stack(16));
    a.lea(true, Gp::Rdi, stack(32));
    a.bind(same_name);
    a.movzx_rm8(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.movzx_rm8(false, Gp::Rdx, Mem::base(Gp::Rdi));
    a.cmp_rr(false, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::Ne, open_temp);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, random);
    a.add_ri(true, Gp::Rdi, 1);
    a.add_ri(true, Gp::Rsi, 1);
    a.jmp(same_name);
    a.bind(open_temp);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    a.lea(true, Gp::Rsi, stack(32));
    a.mov_ri32(
        Gp::Rdx,
        O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW_X64 | O_CLOEXEC,
    );
    a.mov_ri32(Gp::R10, 0o600);
    syscall(a, super::SYS_OPENAT);
    a.cmp_ri(true, Gp::Rax, -17); // EEXIST
    a.jcc(Cond::E, random);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, close_parent);
    a.mov_rr(true, Gp::Rbp, Gp::Rax);
    a.mov_rm(true, Gp::Rax, stack(0));
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R14, Gp::Rax, 1, 0));
    a.mov_rm(true, Gp::Rdx, stack(8));
    emit_write_all(a, Gp::Rbp);
    a.mov_rr(true, Gp::R9, Gp::Rdx);
    a.test_rr(true, Gp::R9, Gp::R9);
    a.jcc(Cond::Ne, close_file);

    a.bind(sync);
    a.mov_rr(false, Gp::Rdi, Gp::Rbp);
    syscall(a, SYS_FSYNC);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, sync);
    a.mov_rr(true, Gp::R9, Gp::Rax);
    a.bind(close_file);
    a.mov_rr(false, Gp::Rdi, Gp::Rbp);
    syscall(a, SYS_CLOSE);
    a.or_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::Ne, remove_temp);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    a.lea(true, Gp::Rsi, stack(32));
    a.mov_rr(false, Gp::Rdx, Gp::Rbx);
    a.mov_rm(true, Gp::R10, stack(16));
    syscall(a, SYS_RENAMEAT);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ne, remove_temp);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    syscall(a, SYS_CLOSE);
    a.mov_ri32(Gp::Rax, 1);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(leave);

    a.bind(remove_temp);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    a.lea(true, Gp::Rsi, stack(32));
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    syscall(a, SYS_UNLINKAT);
    a.bind(close_parent);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    syscall(a, SYS_CLOSE);
    a.bind(deny);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, 72);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_frexp(a: &mut Asm, fixed_mem: bool) {
    let oob = a.new_label();
    let normal = a.new_label();
    let write_exp = a.new_label();
    a.mov_ri32(Gp::Rsi, 4);
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.movq_rx(Gp::Rax, Xmm::Xmm0);
    a.mov_rr(true, Gp::Rcx, Gp::Rax);
    a.shl_imm(true, Gp::Rcx, 1);
    a.jcc(Cond::E, write_exp); // signed zero
    a.shr_imm(true, Gp::Rcx, 53);
    a.cmp_ri(false, Gp::Rcx, 0x7ff);
    a.jcc(Cond::E, write_exp); // infinity or NaN
    a.test_rr(false, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::Ne, normal);
    // Normalize subnormals by an exact multiplication by 2^54.
    a.mov_ri64(Gp::Rax, 1077 << 52);
    a.movq_xr(Xmm::Xmm1, Gp::Rax);
    a.mulsd(Xmm::Xmm0, Xmm::Xmm1);
    a.movq_rx(Gp::Rax, Xmm::Xmm0);
    a.mov_rr(true, Gp::Rcx, Gp::Rax);
    a.shl_imm(true, Gp::Rcx, 1);
    a.shr_imm(true, Gp::Rcx, 53);
    a.sub_ri(false, Gp::Rdx, 54);
    a.bind(normal);
    a.add_rr(false, Gp::Rdx, Gp::Rcx);
    a.sub_ri(false, Gp::Rdx, 1022);
    a.mov_ri64(Gp::Rcx, 0x800f_ffff_ffff_ffff);
    a.and_rr(true, Gp::Rax, Gp::Rcx);
    a.mov_ri64(Gp::Rcx, 1022 << 52);
    a.or_rr(true, Gp::Rax, Gp::Rcx);
    a.movq_xr(Xmm::Xmm0, Gp::Rax);
    a.bind(write_exp);
    a.mov_mr(false, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0), Gp::Rdx);
    complete(a);
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_ldexp(a: &mut Asm) {
    let end = a.new_label();
    let positive = a.new_label();
    let negative = a.new_label();
    let upper_bound = a.new_label();
    let lower_bound = a.new_label();
    let final_scale = a.new_label();
    a.movq_rx(Gp::Rax, Xmm::Xmm0);
    a.shl_imm(true, Gp::Rax, 1);
    a.jcc(Cond::E, end); // signed zero
    a.shr_imm(true, Gp::Rax, 53);
    a.cmp_ri(false, Gp::Rax, 0x7ff);
    a.jcc(Cond::E, end);
    a.cmp_ri(false, Gp::Rdi, 4096);
    a.jcc(Cond::Le, upper_bound);
    a.mov_ri32(Gp::Rdi, 4096);
    a.bind(upper_bound);
    a.cmp_ri(false, Gp::Rdi, -4096);
    a.jcc(Cond::Ge, lower_bound);
    a.mov_ri32(Gp::Rdi, (-4096i32) as u32);
    a.bind(lower_bound);
    a.bind(positive);
    a.cmp_ri(false, Gp::Rdi, 1023);
    a.jcc(Cond::Le, negative);
    a.mov_ri64(Gp::Rax, 2046 << 52);
    a.movq_xr(Xmm::Xmm1, Gp::Rax);
    a.mulsd(Xmm::Xmm0, Xmm::Xmm1);
    a.sub_ri(false, Gp::Rdi, 1023);
    a.jmp(positive);
    a.bind(negative);
    a.cmp_ri(false, Gp::Rdi, -1022);
    a.jcc(Cond::Ge, final_scale);
    // Keep intermediate values normal whenever the final subnormal result can round up.
    a.mov_ri64(Gp::Rax, 54 << 52);
    a.movq_xr(Xmm::Xmm1, Gp::Rax);
    a.mulsd(Xmm::Xmm0, Xmm::Xmm1);
    a.add_ri(false, Gp::Rdi, 969);
    a.jmp(negative);
    a.bind(final_scale);
    a.add_ri(false, Gp::Rdi, 1023);
    a.shl_imm(true, Gp::Rdi, 52);
    a.movq_xr(Xmm::Xmm1, Gp::Rdi);
    a.mulsd(Xmm::Xmm0, Xmm::Xmm1);
    a.bind(end);
    complete(a);
}

pub(super) fn emit_list_dir(a: &mut Asm, fixed_mem: bool) {
    const CONTROL: i32 = 0;
    const FD: i32 = 8;
    const MAP_LEN: i32 = 16;
    const LIMIT: i32 = 24;
    const SIZE: i32 = 32;
    const COUNT: i32 = 40;
    const DENTS: i32 = 56;
    const DENTS_LEN: i32 = 8192;
    const FRAME: i32 = DENTS + DENTS_LEN;

    let oob = a.new_label();
    let no_fd = a.new_label();
    let close = a.new_label();
    let free = a.new_label();
    let free_and_close = a.new_label();
    let read = a.new_label();
    let read_done = a.new_label();
    let record = a.new_label();
    let name = a.new_label();
    let char_loop = a.new_label();
    let copy = a.new_label();
    let render = a.new_label();
    let render_char = a.new_label();
    let newline = a.new_label();
    let append = a.new_label();
    let leave = a.new_label();

    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, FRAME);
    a.mov_mr(true, stack(CONTROL), Gp::Rdx);
    a.mov_ri32(Gp::Rdx, FS_OPEN_LIST);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, no_fd);
    a.mov_mr(true, stack(FD), Gp::Rax);

    // Reserve address space for packed NUL-terminated names, the output, and a
    // heap of 8-byte name pointers. Each stored name uses at least two bytes.
    // Only pages actually used are populated, and the byte limit is checked
    // before copying any entry. The mapping is released after the guest copy.
    a.mov_rm(
        true,
        Gp::Rsi,
        Mem::base_disp(Gp::R15, OFF_EXEC_OUTPUT_LIMIT),
    );
    a.mov_mr(true, stack(LIMIT), Gp::Rsi);
    a.add_ri(true, Gp::Rsi, 1);
    a.jcc(Cond::B, close);
    a.imul_rri(true, Gp::Rsi, Gp::Rsi, 6);
    a.jcc(Cond::O, close);
    a.mov_mr(true, stack(MAP_LEN), Gp::Rsi);
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.mov_ri32(Gp::Rdx, super::PROT_RW);
    a.mov_ri32(Gp::R10, super::MAP_PRIVATE_ANON_NORESERVE);
    a.mov_ri64(Gp::R8, u64::MAX);
    a.xor_rr(false, Gp::R9, Gp::R9);
    syscall(a, SYS_MMAP);
    a.cmp_ri(true, Gp::Rax, -4095);
    a.jcc(Cond::Ae, close);
    a.mov_rr(true, Gp::Rbp, Gp::Rax);
    a.mov_rm(true, Gp::Rax, stack(LIMIT));
    a.lea(true, Gp::Rbx, Mem::indexed(Gp::Rbp, Gp::Rax, 2, 0));
    a.mov_mi32(true, stack(SIZE), 0);
    a.mov_mi32(true, stack(COUNT), 0);

    a.bind(read);
    a.mov_rm(false, Gp::Rdi, stack(FD));
    a.lea(true, Gp::Rsi, stack(DENTS));
    a.mov_ri32(Gp::Rdx, DENTS_LEN as u32);
    syscall(a, SYS_GETDENTS64);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, read);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, free_and_close);
    a.jcc(Cond::E, read_done);
    a.lea(true, Gp::R8, stack(DENTS));
    a.lea(true, Gp::R9, Mem::indexed(Gp::R8, Gp::Rax, 1, 0));

    a.bind(record);
    a.cmp_rr(true, Gp::R8, Gp::R9);
    a.jcc(Cond::Ae, read);
    // linux_dirent64: d_reclen at byte 16, d_name at byte 19.
    a.lea(true, Gp::Rsi, Mem::base_disp(Gp::R8, 19));
    a.movzx_rm16(false, Gp::Rax, Mem::base_disp(Gp::R8, 16));
    a.add_rr(true, Gp::R8, Gp::Rax);
    a.movzx_rm8(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.cmp_ri(false, Gp::Rax, i32::from(b'.'));
    a.jcc(Cond::Ne, name);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rsi, 1));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, record); // "."
    a.cmp_ri(false, Gp::Rax, i32::from(b'.'));
    a.jcc(Cond::Ne, name);
    a.movzx_rm8(false, Gp::Rax, Mem::base_disp(Gp::Rsi, 2));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, record); // ".."
    a.bind(name);
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(char_loop);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_ri(false, Gp::Rax, i32::from(b'\n'));
    a.jcc(Cond::E, record); // The Goose list format cannot represent this name.
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, copy);
    a.jmp(char_loop);

    a.bind(copy);
    a.mov_rm(true, Gp::Rax, stack(SIZE));
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rbp, Gp::Rax, 1, 0));
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.jcc(Cond::B, free_and_close);
    a.cmp_rm(true, Gp::Rax, stack(LIMIT));
    a.jcc(Cond::A, free_and_close);
    a.mov_mr(true, stack(SIZE), Gp::Rax);
    a.mov_rm(true, Gp::Rax, stack(COUNT));
    a.mov_mr(true, Mem::indexed(Gp::Rbx, Gp::Rax, 8, 0), Gp::Rdi);
    a.add_ri(true, Gp::Rax, 1);
    a.mov_mr(true, stack(COUNT), Gp::Rax);
    a.rep_movsb();
    a.jmp(record);

    a.bind(read_done);
    a.mov_rm(false, Gp::Rdi, stack(FD));
    syscall(a, SYS_CLOSE);
    emit_sort_names(a, COUNT);

    a.mov_rm(true, Gp::Rdx, stack(LIMIT));
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rbp, Gp::Rdx, 1, 0));
    a.mov_rr(true, Gp::Rdx, Gp::Rdi); // output start
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(render);
    a.cmp_rm(true, Gp::Rcx, stack(COUNT));
    a.jcc(Cond::Ae, append);
    a.mov_rm(true, Gp::Rsi, Mem::indexed(Gp::Rbx, Gp::Rcx, 8, 0));
    a.bind(render_char);
    a.movzx_rm8(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, newline);
    a.mov_mr8(Mem::base(Gp::Rdi), Gp::Rax);
    a.add_ri(true, Gp::Rsi, 1);
    a.add_ri(true, Gp::Rdi, 1);
    a.jmp(render_char);
    a.bind(newline);
    a.mov_mi8(Mem::base(Gp::Rdi), b'\n');
    a.add_ri(true, Gp::Rdi, 1);
    a.add_ri(true, Gp::Rcx, 1);
    a.jmp(render);
    a.bind(append);
    a.mov_rr(true, Gp::Rsi, Gp::Rdx);
    a.mov_rm(true, Gp::Rdx, stack(SIZE));
    a.mov_rm(true, Gp::Rdi, stack(CONTROL));
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
    a.mov_rr(false, Gp::Rbx, Gp::R11);
    a.mov_ri32(Gp::R8, 1);
    a.jmp(free);

    a.bind(free_and_close);
    a.mov_rm(false, Gp::Rdi, stack(FD));
    syscall(a, SYS_CLOSE);
    a.xor_rr(false, Gp::Rbx, Gp::Rbx);
    a.xor_rr(false, Gp::R8, Gp::R8);
    a.bind(free);
    a.mov_rr(true, Gp::Rdi, Gp::Rbp);
    a.mov_rm(true, Gp::Rsi, stack(MAP_LEN));
    syscall(a, SYS_MUNMAP);
    a.mov_rr(false, Gp::R11, Gp::Rbx);
    a.mov_rr(false, Gp::Rax, Gp::R8);
    a.jmp(leave);
    a.bind(close);
    a.mov_rm(false, Gp::Rdi, stack(FD));
    syscall(a, SYS_CLOSE);
    a.bind(no_fd);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, FRAME);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
    a.bind(oob);
    trap(a);
}

// Sort pointers to NUL-terminated strings in place with heapsort. The
// comparison uses unsigned bytes, matching the other Goose hosts.
fn emit_sort_names(a: &mut Asm, count: i32) {
    let build = a.new_label();
    let extract = a.new_label();
    let extract_loop = a.new_label();
    let done = a.new_label();
    let sift = a.new_label();
    let sift_loop = a.new_label();
    let compare_child = a.new_label();
    let compare_parent = a.new_label();
    let sift_done = a.new_label();
    let compare = a.new_label();
    let compare_loop = a.new_label();
    let compare_done = a.new_label();

    // Keep loop indices in a scratch slot. Calls to sift may clobber all
    // caller-saved registers. Rbx is the pointer array.
    const INDEX: i32 = 48;
    a.mov_rm(true, Gp::Rsi, stack(count));
    a.mov_rr(true, Gp::Rdi, Gp::Rsi);
    a.shr_imm(true, Gp::Rdi, 1);
    a.bind(build);
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, extract);
    a.sub_ri(true, Gp::Rdi, 1);
    a.mov_mr(true, stack(INDEX), Gp::Rdi);
    a.call_label(sift);
    a.mov_rm(true, Gp::Rdi, stack(INDEX));
    a.mov_rm(true, Gp::Rsi, stack(count));
    a.jmp(build);

    a.bind(extract);
    a.mov_rm(true, Gp::Rsi, stack(count));
    a.bind(extract_loop);
    a.cmp_ri(true, Gp::Rsi, 1);
    a.jcc(Cond::Be, done);
    a.sub_ri(true, Gp::Rsi, 1);
    a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rbx));
    a.mov_rm(true, Gp::Rcx, Mem::indexed(Gp::Rbx, Gp::Rsi, 8, 0));
    a.mov_mr(true, Mem::base(Gp::Rbx), Gp::Rcx);
    a.mov_mr(true, Mem::indexed(Gp::Rbx, Gp::Rsi, 8, 0), Gp::Rax);
    a.mov_mr(true, stack(INDEX), Gp::Rsi);
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.call_label(sift);
    a.mov_rm(true, Gp::Rsi, stack(INDEX));
    a.jmp(extract_loop);

    a.bind(sift);
    a.mov_rr(true, Gp::R8, Gp::Rdi); // root
    a.mov_rr(true, Gp::R9, Gp::Rsi); // heap length
    a.bind(sift_loop);
    a.lea(true, Gp::R10, Mem::indexed(Gp::R8, Gp::R8, 1, 1)); // left child
    a.cmp_rr(true, Gp::R10, Gp::R9);
    a.jcc(Cond::Ae, sift_done);
    a.lea(true, Gp::R11, Mem::base_disp(Gp::R10, 1)); // right child
    a.cmp_rr(true, Gp::R11, Gp::R9);
    a.jcc(Cond::B, compare_child);
    a.jmp(compare_parent);
    a.bind(compare_child);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::Rbx, Gp::R11, 8, 0));
    a.mov_rm(true, Gp::Rsi, Mem::indexed(Gp::Rbx, Gp::R10, 8, 0));
    a.call_label(compare);
    a.jcc(Cond::Be, compare_parent);
    a.mov_rr(true, Gp::R10, Gp::R11);
    a.bind(compare_parent);
    a.mov_rm(true, Gp::Rdi, Mem::indexed(Gp::Rbx, Gp::R10, 8, 0));
    a.mov_rm(true, Gp::Rsi, Mem::indexed(Gp::Rbx, Gp::R8, 8, 0));
    a.call_label(compare);
    a.jcc(Cond::Be, sift_done);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::Rbx, Gp::R8, 8, 0));
    a.mov_rm(true, Gp::Rcx, Mem::indexed(Gp::Rbx, Gp::R10, 8, 0));
    a.mov_mr(true, Mem::indexed(Gp::Rbx, Gp::R10, 8, 0), Gp::Rax);
    a.mov_mr(true, Mem::indexed(Gp::Rbx, Gp::R8, 8, 0), Gp::Rcx);
    a.mov_rr(true, Gp::R8, Gp::R10);
    a.jmp(sift_loop);
    a.bind(sift_done);
    a.ret();

    // Return unsigned byte comparison in flags.
    a.bind(compare);
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(compare_loop);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, 0));
    a.movzx_rm8(false, Gp::Rdx, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, 0));
    a.cmp_rr(false, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::Ne, compare_done);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, compare_done);
    a.add_ri(true, Gp::Rcx, 1);
    a.jmp(compare_loop);
    a.bind(compare_done);
    a.ret();
    a.bind(done);
}

pub(super) fn emit_remove_entry(a: &mut Asm, fixed_mem: bool, is_dir: bool) {
    let done = a.new_label();
    let deny = a.new_label();
    let oob = a.new_label();
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.sub_ri(true, Gp::Rsp, 24);
    a.mov_ri32(
        Gp::Rdx,
        if is_dir {
            FS_OPEN_DIR_PARENT
        } else {
            FS_OPEN_FILE_PARENT
        },
    );
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, done);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, deny);
    a.mov_mr(true, stack(0), Gp::Rax);
    a.mov_rr(false, Gp::Rdi, Gp::Rax);
    a.mov_rr(true, Gp::Rsi, Gp::Rdx);
    if is_dir {
        a.mov_ri32(Gp::Rdx, AT_REMOVEDIR);
    } else {
        a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    }
    syscall(a, SYS_UNLINKAT);
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.mov_rm(false, Gp::Rdi, stack(0));
    syscall(a, SYS_CLOSE);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.test_rr(true, Gp::R8, Gp::R8);
    let fail = a.new_label();
    a.jcc(Cond::Ne, fail);
    a.mov_ri32(Gp::Rax, 1);
    a.bind(fail);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(done);
    a.bind(deny);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.bind(done);
    a.add_ri(true, Gp::Rsp, 24);
    a.ret();
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_rename_file(a: &mut Asm, fixed_mem: bool) {
    let oob = a.new_label();
    let deny = a.new_label();
    let close_from = a.new_label();
    let leave = a.new_label();
    let cp_lp = a.new_label();
    let cp_done = a.new_label();
    let trap_from = a.new_label();
    let rename_fail = a.new_label();

    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    emit_check_range(a, Gp::Rdx, Gp::Rcx, fixed_mem, oob);
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, 312);
    a.mov_mr(true, stack(0), Gp::Rdx);
    a.mov_mr(true, stack(8), Gp::Rcx);
    a.mov_ri32(Gp::Rdx, FS_OPEN_FILE_PARENT);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, deny);
    a.mov_rr(true, Gp::Rbx, Gp::Rax);
    a.xor_rr(false, Gp::Rcx, Gp::Rcx);
    a.bind(cp_lp);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rdx, Gp::Rcx, 1, 0));
    a.mov_mr8(Mem::indexed(Gp::Rsp, Gp::Rcx, 1, 16), Gp::Rax);
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, cp_done);
    a.add_ri(true, Gp::Rcx, 1);
    a.cmp_ri(true, Gp::Rcx, 256);
    a.jcc(Cond::Ae, close_from);
    a.jmp(cp_lp);
    a.bind(cp_done);

    a.mov_rm(true, Gp::Rdi, stack(0));
    a.mov_rm(true, Gp::Rsi, stack(8));
    a.mov_ri32(Gp::Rdx, FS_OPEN_FILE_PARENT);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_FS));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, trap_from);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::L, close_from);
    a.mov_rr(true, Gp::Rbp, Gp::Rax);
    a.mov_rr(true, Gp::R10, Gp::Rdx);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    a.lea(true, Gp::Rsi, stack(16));
    a.mov_rr(false, Gp::Rdx, Gp::Rbp);
    syscall(a, SYS_RENAMEAT);
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.mov_rr(false, Gp::Rdi, Gp::Rbp);
    syscall(a, SYS_CLOSE);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    syscall(a, SYS_CLOSE);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.test_rr(true, Gp::R8, Gp::R8);
    a.jcc(Cond::Ne, rename_fail);
    a.mov_ri32(Gp::Rax, 1);
    a.bind(rename_fail);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(leave);

    a.bind(trap_from);
    a.mov_rr(false, Gp::Rbp, Gp::R11);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    syscall(a, SYS_CLOSE);
    a.mov_rr(false, Gp::R11, Gp::Rbp);
    a.jmp(leave);

    a.bind(close_from);
    a.mov_rr(false, Gp::Rdi, Gp::Rbx);
    syscall(a, SYS_CLOSE);
    a.bind(deny);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, 312);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_random_u64(a: &mut Asm) {
    let retry = a.new_label();
    let fail = a.new_label();
    let done = a.new_label();
    a.sub_ri(true, Gp::Rsp, 24);
    a.bind(retry);
    a.mov_rr(true, Gp::Rdi, Gp::Rsp);
    a.mov_ri32(Gp::Rsi, 8);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    syscall(a, SYS_GETRANDOM);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, retry);
    a.cmp_ri(true, Gp::Rax, 8);
    a.jcc(Cond::Ne, fail);
    a.mov_rm(true, Gp::Rax, stack(0));
    a.jmp(done);
    a.bind(fail);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(done);
    a.add_ri(true, Gp::Rsp, 24);
    complete(a);
}

pub(super) fn emit_random_bytes(a: &mut Asm, fixed_mem: bool) {
    let oob = a.new_label();
    let lp = a.new_label();
    let ok = a.new_label();
    let fail = a.new_label();
    let leave = a.new_label();
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, 8);
    a.lea(true, Gp::Rbx, Mem::indexed(Gp::R14, Gp::Rdi, 1, 0));
    a.mov_rr(true, Gp::Rbp, Gp::Rsi);
    a.bind(lp);
    a.test_rr(true, Gp::Rbp, Gp::Rbp);
    a.jcc(Cond::E, ok);
    a.mov_rr(true, Gp::Rdi, Gp::Rbx);
    a.mov_rr(true, Gp::Rsi, Gp::Rbp);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    syscall(a, SYS_GETRANDOM);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, lp);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Le, fail);
    a.add_rr(true, Gp::Rbx, Gp::Rax);
    a.sub_rr(true, Gp::Rbp, Gp::Rax);
    a.jmp(lp);
    a.bind(ok);
    a.mov_ri32(Gp::Rax, 1);
    a.jmp(leave);
    a.bind(fail);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, 8);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    complete(a);
    a.bind(oob);
    trap(a);
}

pub(super) fn emit_sleep_ms(a: &mut Asm) {
    let done_early = a.new_label();
    let retry = a.new_label();
    let done = a.new_label();
    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::Le, done_early);
    a.sub_ri(true, Gp::Rsp, 40);
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.xor_rr(false, Gp::Rdx, Gp::Rdx);
    a.mov_ri32(Gp::Rcx, 1000);
    a.div_r(true, Gp::Rcx);
    a.imul_rri(true, Gp::Rdx, Gp::Rdx, 1_000_000);
    a.mov_mr(true, stack(0), Gp::Rax);
    a.mov_mr(true, stack(8), Gp::Rdx);
    a.bind(retry);
    a.mov_rr(true, Gp::Rdi, Gp::Rsp);
    a.lea(true, Gp::Rsi, stack(16));
    syscall(a, SYS_NANOSLEEP);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::Ne, done);
    a.mov_rm(true, Gp::Rax, stack(16));
    a.mov_rm(true, Gp::Rdx, stack(24));
    a.mov_mr(true, stack(0), Gp::Rax);
    a.mov_mr(true, stack(8), Gp::Rdx);
    a.jmp(retry);
    a.bind(done);
    a.add_ri(true, Gp::Rsp, 40);
    a.bind(done_early);
    complete(a);
}

pub(super) fn emit_read_line(a: &mut Asm) {
    const CONTROL: i32 = 0;
    const TOTAL: i32 = 8;
    const GOT_ANY: i32 = 16;
    const BYTE_TMP: i32 = 24;
    const BUF: i32 = 32;
    const BUF_LEN: i32 = 4096;
    const FRAME: i32 = 4152; // 8 + 16 + 4152 = 4176 (16B aligned)

    let lp = a.new_label();
    let read1 = a.new_label();
    let no_flush = a.new_label();
    let flush_all = a.new_label();
    let eof = a.new_label();
    let no_cr = a.new_label();
    let skip_final_flush = a.new_label();
    let ret_zero = a.new_label();
    let trap_leave = a.new_label();
    let leave = a.new_label();

    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, FRAME);
    a.mov_mr(true, stack(CONTROL), Gp::Rdi);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(true, stack(TOTAL), Gp::Rax);
    a.mov_mr(true, stack(GOT_ANY), Gp::Rax);
    a.xor_rr(false, Gp::Rbx, Gp::Rbx);

    a.bind(lp);
    a.cmp_ri(true, Gp::Rbx, BUF_LEN);
    a.jcc(Cond::B, no_flush);
    a.movzx_rm8(false, Gp::Rax, stack(BUF + BUF_LEN - 1));
    a.cmp_ri(false, Gp::Rax, i32::from(b'\r'));
    a.jcc(Cond::Ne, flush_all);
    a.mov_rm(true, Gp::Rdi, stack(CONTROL));
    a.lea(true, Gp::Rsi, stack(BUF));
    a.mov_ri32(Gp::Rdx, (BUF_LEN - 1) as u32);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.mov_mi8(stack(BUF), b'\r');
    a.mov_ri32(Gp::Rbx, 1);
    a.jmp(no_flush);
    a.bind(flush_all);
    a.mov_rm(true, Gp::Rdi, stack(CONTROL));
    a.lea(true, Gp::Rsi, stack(BUF));
    a.mov_ri32(Gp::Rdx, BUF_LEN as u32);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.xor_rr(false, Gp::Rbx, Gp::Rbx);

    a.bind(no_flush);
    a.bind(read1);
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.lea(true, Gp::Rsi, stack(BYTE_TMP));
    a.mov_ri32(Gp::Rdx, 1);
    syscall(a, SYS_READ);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, read1);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Le, eof);
    a.mov_mi32(true, stack(GOT_ANY), 1);
    a.movzx_rm8(false, Gp::Rax, stack(BYTE_TMP));
    a.cmp_ri(false, Gp::Rax, i32::from(b'\n'));
    a.jcc(Cond::E, eof);
    a.mov_rm(true, Gp::Rcx, stack(TOTAL));
    a.add_ri(true, Gp::Rcx, 1);
    a.jcc(Cond::B, trap_leave);
    a.cmp_rm(
        true,
        Gp::Rcx,
        Mem::base_disp(Gp::R15, OFF_EXEC_OUTPUT_LIMIT),
    );
    a.jcc(Cond::A, trap_leave);
    a.mov_mr(true, stack(TOTAL), Gp::Rcx);
    a.mov_mr8(Mem::indexed(Gp::Rsp, Gp::Rbx, 1, BUF), Gp::Rax);
    a.add_ri(true, Gp::Rbx, 1);
    a.jmp(lp);

    a.bind(eof);
    a.mov_rm(true, Gp::Rax, stack(GOT_ANY));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, ret_zero);
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::E, skip_final_flush);
    a.movzx_rm8(false, Gp::Rax, Mem::indexed(Gp::Rsp, Gp::Rbx, 1, BUF - 1));
    a.cmp_ri(false, Gp::Rax, i32::from(b'\r'));
    a.jcc(Cond::Ne, no_cr);
    a.sub_ri(true, Gp::Rbx, 1);
    a.bind(no_cr);
    a.test_rr(true, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::E, skip_final_flush);
    a.mov_rm(true, Gp::Rdi, stack(CONTROL));
    a.lea(true, Gp::Rsi, stack(BUF));
    a.mov_rr(true, Gp::Rdx, Gp::Rbx);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.bind(skip_final_flush);
    a.mov_ri32(Gp::Rax, 1);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(leave);

    a.bind(ret_zero);
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(leave);

    a.bind(trap_leave);
    a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, FRAME);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
}

pub(super) fn emit_read_stdin(a: &mut Asm, has_max: bool) {
    const CONTROL: i32 = 0;
    const REMAINING: i32 = 8;
    const TOTAL: i32 = 16;
    const BUF: i32 = 32;
    const BUF_LEN: u32 = 4096;
    const FRAME: i32 = 4136; // 8 + 16 + 4136 = 4160 (16B aligned)

    let early_zero = a.new_label();
    let lp = a.new_label();
    let read_lp = a.new_label();
    let done = a.new_label();
    let trap_leave = a.new_label();
    let leave = a.new_label();

    if has_max {
        a.test_rr(true, Gp::Rsi, Gp::Rsi);
        a.jcc(Cond::Le, early_zero);
    }
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.sub_ri(true, Gp::Rsp, FRAME);
    a.mov_mr(true, stack(CONTROL), Gp::Rdi);
    if has_max {
        a.mov_mr(true, stack(REMAINING), Gp::Rsi);
    }
    a.xor_rr(false, Gp::Rax, Gp::Rax);
    a.mov_mr(true, stack(TOTAL), Gp::Rax);

    a.bind(lp);
    a.mov_ri32(Gp::Rdx, BUF_LEN);
    if has_max {
        a.mov_rm(true, Gp::Rcx, stack(REMAINING));
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        a.jcc(Cond::E, done);
        a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
        a.cmovcc(Cond::B, true, Gp::Rdx, Gp::Rcx);
    }
    a.mov_rr(true, Gp::Rbx, Gp::Rdx);
    a.bind(read_lp);
    a.xor_rr(false, Gp::Rdi, Gp::Rdi);
    a.lea(true, Gp::Rsi, stack(BUF));
    a.mov_rr(true, Gp::Rdx, Gp::Rbx);
    syscall(a, SYS_READ);
    a.cmp_ri(true, Gp::Rax, -4);
    a.jcc(Cond::E, read_lp);
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Le, done);
    a.mov_rm(true, Gp::Rcx, stack(TOTAL));
    a.add_rr(true, Gp::Rcx, Gp::Rax);
    a.jcc(Cond::B, trap_leave);
    a.cmp_rm(
        true,
        Gp::Rcx,
        Mem::base_disp(Gp::R15, OFF_EXEC_OUTPUT_LIMIT),
    );
    a.jcc(Cond::A, trap_leave);
    a.mov_mr(true, stack(TOTAL), Gp::Rcx);
    if has_max {
        a.mov_rm(true, Gp::Rcx, stack(REMAINING));
        a.sub_rr(true, Gp::Rcx, Gp::Rax);
        a.mov_mr(true, stack(REMAINING), Gp::Rcx);
    }
    a.mov_rm(true, Gp::Rdi, stack(CONTROL));
    a.lea(true, Gp::Rsi, stack(BUF));
    a.mov_rr(true, Gp::Rdx, Gp::Rax);
    a.call_reloc(RelocTarget::Helper(HELPER_STANDALONE_RESERVE_COPY));
    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, leave);
    a.jmp(lp);

    a.bind(done);
    if has_max {
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        a.mov_rm(true, Gp::Rcx, stack(TOTAL));
        a.test_rr(true, Gp::Rcx, Gp::Rcx);
        let z = a.new_label();
        a.jcc(Cond::E, z);
        a.mov_ri32(Gp::Rax, 1);
        a.bind(z);
    }
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.jmp(leave);

    a.bind(trap_leave);
    a.mov_ri32(Gp::R11, u32::from(TRAP_HOST));
    a.bind(leave);
    a.add_ri(true, Gp::Rsp, FRAME);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    if has_max {
        a.bind(early_zero);
        a.xor_rr(false, Gp::Rax, Gp::Rax);
        complete(a);
    }
}

pub(super) fn emit_f64_digits(a: &mut Asm, fixed_mem: bool) {
    let oob = a.new_label();
    let done = a.new_label();
    emit_check_range(a, Gp::Rdi, Gp::Rsi, fixed_mem, oob);
    a.mov_ri64(Gp::R8, 8);
    emit_check_range(a, Gp::Rdx, Gp::R8, fixed_mem, oob);
    a.sub_ri(true, Gp::Rsp, 72);
    a.mov_mr(true, stack(32), Gp::Rdi);
    a.mov_mr(true, stack(40), Gp::Rsi);
    a.mov_mr(true, stack(48), Gp::Rdx);
    a.mov_rr(true, Gp::Rdi, Gp::Rsp);
    a.mov_ri32(Gp::Rsi, 2);
    a.call_reloc(RelocTarget::Helper(HELPER_GOOSE_FMT_FLOAT));
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, done);
    a.mov_rm(true, Gp::Rcx, stack(40));
    a.cmp_rr(true, Gp::Rax, Gp::Rcx);
    a.cmovcc(Cond::A, true, Gp::Rax, Gp::Rcx);
    a.mov_rr(true, Gp::R8, Gp::Rax);
    a.mov_rr(true, Gp::Rcx, Gp::Rax);
    a.mov_rm(true, Gp::Rdi, stack(32));
    a.add_rr(true, Gp::Rdi, Gp::R14);
    a.mov_rr(true, Gp::Rsi, Gp::Rsp);
    a.rep_movsb();
    a.mov_rm(true, Gp::Rdx, stack(24));
    a.mov_rm(true, Gp::R9, stack(48));
    a.mov_mr(true, Mem::indexed(Gp::R14, Gp::R9, 1, 0), Gp::Rdx);
    a.mov_rr(true, Gp::Rax, Gp::R8);
    a.bind(done);
    a.add_ri(true, Gp::Rsp, 72);
    complete(a);
    a.bind(oob);
    trap(a);
}
