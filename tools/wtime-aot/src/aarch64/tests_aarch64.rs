//! Verification tests for `--backend aarch64`:
//! 1. Disassembles emitted AArch64 ELF64 `ET_DYN` (`.so`) and static `ET_EXEC` binaries with
//!    `llvm-objdump --triple=aarch64 -d` to assert 100% valid ARMv8-A encodings (zero `<unknown>`).
//! 2. Executes emitted AArch64 machine code directly inside a self-contained pure-Rust
//!    (`#![forbid(unsafe_code)]`) AArch64 user-mode functional emulator and differentially verifies
//!    exact return values, linear memory bytes, and traps against `wtime-runtime` across:
//!    - Integer arithmetic, shifts, rotates, clz/ctz/popcnt, div/rem + div0/overflow traps
//!    - Floating-point (`f32`/`f64`, NaN, -0.0, min/max, rounding, all 16 trunc ops)
//!    - Linear memory (Wasm32 & `memory64`, OOB traps, bulk `copy`/`fill`, `size`/`grow`)
//!    - `br_table`, recursion, multi-value calls, `call_indirect` + traps
//!    - Real auto-vectorized kernels (`graph_bfs.wasm`, `parser_hash.wasm`, `image_filter.wasm`,
//!      plus an `f64x2` SLP + `psadbw` kernel)!

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};

use super::{A64CompileOptions, A64CompiledImage, compile_wasm_to_aarch64_for_test};
use crate::x64::module::{X64Target, x64_validate_features, x64_validate_limits};
use crate::x64::{HELPER_INITIALIZE, OFF_MEM_BASE, OFF_MEM_MAX_PAGES, OFF_STACK_FLOOR};

// ============================================================================
// Pure-Rust `#![forbid(unsafe_code)]` AArch64 User-Mode Functional Emulator
// ============================================================================

struct A64Vm {
    /// X0..X30; X31 is SP (`sp`).
    x: [u64; 31],
    sp: u64,
    /// V0..V31 (128-bit little-endian bytes).
    v: [[u8; 16]; 32],
    pc: u64,
    /// NZCV flags.
    n: bool,
    z: bool,
    c: bool,
    v_flag: bool,
    /// Virtual address space segments `(base, reserve_len, backing_bytes)`.
    /// Any address in `[base + backing_bytes.len(), base + reserve_len)` is `PROT_NONE` (guard fault).
    segs: Vec<(u64, u64, Vec<u8>)>,
    /// Next anonymous `mmap` address.
    next_mmap: u64,
    /// Emulated exit status from `SYS_EXIT_GROUP`.
    exited: Option<i32>,
    /// Captured stdout bytes from `SYS_WRITE(fd=1)`.
    stdout: Vec<u8>,
    /// Captured stderr bytes from `SYS_WRITE(fd=2)`.
    stderr: Vec<u8>,
    /// Emulated stdin bytes for `SYS_READ(fd=0)`.
    stdin: Vec<u8>,
    stdin_pos: usize,
    /// Installed Linux arm64 `SIGSEGV` (`SA_SIGINFO`) handler address.
    sigsegv_handler: Option<u64>,
    /// Pending guard-page fault address to dispatch to `sigsegv_handler`.
    pending_fault_addr: Option<u64>,
}

const RET_SENTINEL: u64 = 0xdead_beef_0000_0000;

impl A64Vm {
    fn new(img: &A64CompiledImage) -> Self {
        let mut vm = Self {
            x: [0; 31],
            sp: 0,
            v: [[0; 16]; 32],
            pc: 0,
            n: false,
            z: false,
            c: false,
            v_flag: false,
            segs: Vec::new(),
            next_mmap: 0x1000_0000,
            exited: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdin: Vec::new(),
            stdin_pos: 0,
            sigsegv_handler: None,
            pending_fault_addr: None,
        };
        vm.segs.push((
            img.rodata_vaddr,
            img.rodata.len() as u64,
            img.rodata.clone(),
        ));
        vm.segs
            .push((img.text_vaddr, img.text.len() as u64, img.text.clone()));
        // Host stack for calling exports (and `_start` argc/argv = 0).
        let host_stack_base = vm.alloc_seg(256 * 1024);
        vm.sp = host_stack_base + 256 * 1024 - 64;
        vm
    }

    fn alloc_seg(&mut self, len: usize) -> u64 {
        let al = (len as u64 + 0xffff) & !0xffff;
        let base = self.next_mmap;
        self.next_mmap += al.max(0x10000);
        let actual = len.max(1);
        self.segs.push((base, actual as u64, vec![0u8; actual]));
        base
    }

    fn alloc_guard_seg(&mut self, reserve_len: u64) -> u64 {
        let al = (reserve_len + 0xffff) & !0xffff;
        let base = self.next_mmap;
        self.next_mmap += al.max(0x10000);
        self.segs.push((base, reserve_len, Vec::new()));
        base
    }

    fn mprotect_rw(&mut self, addr: u64, len: u64) {
        if len == 0 {
            return;
        }
        for (base, reserve_len, data) in &mut self.segs {
            if addr >= *base && addr.saturating_add(len) <= base.saturating_add(*reserve_len) {
                let need = (addr - *base + len) as usize;
                if data.len() < need {
                    data.resize(need, 0);
                }
                return;
            }
        }
    }

    fn is_guard_fault(&self, addr: u64, len: usize) -> bool {
        let end = addr.saturating_add(len as u64);
        self.segs.iter().any(|(base, reserve_len, data)| {
            addr >= *base
                && end <= base.saturating_add(*reserve_len)
                && end > base.saturating_add(data.len() as u64)
        })
    }

    fn read_bytes(&self, addr: u64, len: usize) -> Result<&[u8], String> {
        for (base, _, data) in &self.segs {
            if addr >= *base
                && let Some(off) = (addr - *base).try_into().ok()
                && let Some(end) = usize::checked_add(off, len)
                && end <= data.len()
            {
                return Ok(&data[off..end]);
            }
        }
        Err(format!(
            "A64Vm: unmapped read at {addr:#x} len={len} pc={:#x}",
            self.pc
        ))
    }

    fn write_bytes(&mut self, addr: u64, bytes: &[u8]) -> Result<(), String> {
        for (base, _, data) in &mut self.segs {
            if addr >= *base
                && let Some(off) = (addr - *base).try_into().ok()
                && let Some(end) = usize::checked_add(off, bytes.len())
                && end <= data.len()
            {
                data[off..end].copy_from_slice(bytes);
                return Ok(());
            }
        }
        Err(format!(
            "A64Vm: unmapped write at {addr:#x} len={} pc={:#x}",
            bytes.len(),
            self.pc
        ))
    }

    fn read_bytes_checked(&mut self, addr: u64, len: usize) -> Result<Option<Vec<u8>>, String> {
        if let Ok(slice) = self.read_bytes(addr, len) {
            return Ok(Some(slice.to_vec()));
        }
        if self.sigsegv_handler.is_some() && self.is_guard_fault(addr, len) {
            self.pending_fault_addr = Some(addr);
            return Ok(None);
        }
        Err(format!(
            "A64Vm: unmapped read at {addr:#x} len={len} pc={:#x}",
            self.pc
        ))
    }

    fn write_bytes_checked(&mut self, addr: u64, bytes: &[u8]) -> Result<bool, String> {
        if self.write_bytes(addr, bytes).is_ok() {
            return Ok(true);
        }
        if self.sigsegv_handler.is_some() && self.is_guard_fault(addr, bytes.len()) {
            self.pending_fault_addr = Some(addr);
            return Ok(false);
        }
        Err(format!(
            "A64Vm: unmapped write at {addr:#x} len={} pc={:#x}",
            bytes.len(),
            self.pc
        ))
    }

    fn dispatch_sigsegv(&mut self, fault_addr: u64) -> Result<(), String> {
        let handler = self
            .sigsegv_handler
            .ok_or_else(|| format!("A64Vm: unhandled SIGSEGV at {fault_addr:#x}"))?;
        let frame = self.alloc_seg(1024);
        let siginfo_ptr = frame;
        let ucontext_ptr = frame + 128;
        // siginfo_t.si_addr at +16
        self.write_bytes(siginfo_ptr + 16, &fault_addr.to_le_bytes())?;
        // ucontext_t.uc_mcontext.fault_address at +176, regs[0..31] at +184..+432, sp at +432, pc at +440
        self.write_bytes(ucontext_ptr + 176, &fault_addr.to_le_bytes())?;
        let regs_snap = self.x;
        for (i, &rv) in regs_snap.iter().enumerate() {
            self.write_bytes(ucontext_ptr + 184 + (i as u64) * 8, &rv.to_le_bytes())?;
        }
        let sp_snap = self.sp;
        let pc_snap = self.pc;
        self.write_bytes(ucontext_ptr + 432, &sp_snap.to_le_bytes())?;
        self.write_bytes(ucontext_ptr + 440, &pc_snap.to_le_bytes())?;
        self.x[0] = 11; // SIGSEGV
        self.x[1] = siginfo_ptr;
        self.x[2] = ucontext_ptr;
        self.x[30] = RET_SENTINEL;
        self.pc = handler;
        Ok(())
    }

    fn read_u32(&self, addr: u64) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.read_bytes(addr, 4)?.try_into().unwrap(),
        ))
    }
    fn read_u64(&self, addr: u64) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.read_bytes(addr, 8)?.try_into().unwrap(),
        ))
    }

    #[inline]
    fn get_x(&self, r: u8, sp_mode: bool) -> u64 {
        if r == 31 {
            if sp_mode { self.sp } else { 0 }
        } else {
            self.x[r as usize]
        }
    }
    #[inline]
    fn set_x(&mut self, r: u8, val: u64, sp_mode: bool) {
        if r == 31 {
            if sp_mode {
                self.sp = val;
            }
        } else {
            self.x[r as usize] = val;
        }
    }
    #[inline]
    fn set_w(&mut self, r: u8, val: u32, sp_mode: bool) {
        self.set_x(r, u64::from(val), sp_mode);
    }

    fn cond_holds(&self, cond: u8) -> bool {
        let base = match cond & 0xe {
            0x0 => self.z,
            0x2 => self.c,
            0x4 => self.n,
            0x6 => self.v_flag,
            0x8 => self.c && !self.z,
            0xa => self.n == self.v_flag,
            0xc => !self.z && (self.n == self.v_flag),
            0xe => true,
            _ => unreachable!(),
        };
        if (cond & 1) != 0 && (cond & 0xe) != 0xe {
            !base
        } else {
            base
        }
    }

    fn add_with_carry(
        &mut self,
        is64: bool,
        a: u64,
        b: u64,
        carry_in: bool,
        set_flags: bool,
    ) -> u64 {
        if is64 {
            let sum128 = u128::from(a) + u128::from(b) + u128::from(carry_in);
            let res = sum128 as u64;
            if set_flags {
                self.n = (res as i64) < 0;
                self.z = res == 0;
                self.c = (sum128 >> 64) != 0;
                let s = (a as i64 as i128) + (b as i64 as i128) + i128::from(carry_in);
                self.v_flag = s != i128::from(res as i64);
            }
            res
        } else {
            let a32 = a as u32;
            let b32 = b as u32;
            let sum64 = u64::from(a32) + u64::from(b32) + u64::from(carry_in);
            let res = sum64 as u32;
            if set_flags {
                self.n = (res as i32) < 0;
                self.z = res == 0;
                self.c = (sum64 >> 32) != 0;
                let s = i64::from(a32 as i32) + i64::from(b32 as i32) + i64::from(carry_in);
                self.v_flag = s != i64::from(res as i32);
            }
            u64::from(res)
        }
    }

    fn run_until_ret(&mut self, entry: u64, max_steps: usize) -> Result<usize, String> {
        self.pc = entry;
        self.x[30] = RET_SENTINEL;
        for step in 0..max_steps {
            if self.pc == RET_SENTINEL || self.exited.is_some() {
                return Ok(step);
            }
            self.step()?;
        }
        Err(format!(
            "A64Vm: step limit ({max_steps}) exceeded at pc={:#x}",
            self.pc
        ))
    }

    fn step(&mut self) -> Result<(), String> {
        let pc = self.pc;
        let insn = self.read_u32(pc)?;
        let mut next_pc = pc + 4;

        let rd = (insn & 31) as u8;
        let rn = ((insn >> 5) & 31) as u8;
        let rm = ((insn >> 16) & 31) as u8;
        let sf = (insn >> 31) != 0;

        if insn == 0xd503_201f {
            // NOP
        } else if (insn & 0xffe0_001f) == 0xd400_0001 {
            // SVC #imm16 (Linux AArch64 syscall)
            let nr = self.x[8];
            match nr {
                222 => {
                    // mmap(addr, len, prot, flags, fd, off)
                    let len = self.x[1];
                    let prot = self.x[2];
                    let ptr = if prot == 0 && len >= 0x1000_0000 {
                        self.alloc_guard_seg(len)
                    } else {
                        self.alloc_seg(len as usize)
                    };
                    self.x[0] = ptr;
                }
                226 => {
                    // mprotect(addr, len, prot)
                    let addr = self.x[0];
                    let len = self.x[1];
                    let prot = self.x[2];
                    if prot != 0 {
                        self.mprotect_rw(addr, len);
                    }
                    self.x[0] = 0;
                }
                215 => {
                    // munmap -> ok (0)
                    self.x[0] = 0;
                }
                63 => {
                    // read(fd, buf, len)
                    let fd = self.x[0];
                    let buf = self.x[1];
                    let len = self.x[2] as usize;
                    if fd == 0 {
                        let avail = self.stdin.len().saturating_sub(self.stdin_pos);
                        let take = avail.min(len);
                        if take > 0 {
                            let bytes = self.stdin[self.stdin_pos..self.stdin_pos + take].to_vec();
                            self.write_bytes(buf, &bytes)?;
                            self.stdin_pos += take;
                        }
                        self.x[0] = take as u64;
                    } else {
                        self.x[0] = (-9i64) as u64; // -EBADF
                    }
                }
                64 => {
                    // write(fd, buf, len)
                    let fd = self.x[0];
                    let buf = self.x[1];
                    let len = self.x[2] as usize;
                    let data = self.read_bytes(buf, len)?.to_vec();
                    if fd == 1 {
                        self.stdout.extend_from_slice(&data);
                    } else {
                        self.stderr.extend_from_slice(&data);
                    }
                    self.x[0] = len as u64;
                }
                113 => {
                    // clock_gettime(clk_id, ts_ptr)
                    let ts_ptr = self.x[1];
                    self.write_bytes(ts_ptr, &1000u64.to_le_bytes())?;
                    self.write_bytes(ts_ptr + 8, &123_456_789u64.to_le_bytes())?;
                    self.x[0] = 0;
                }
                134 => {
                    // rt_sigaction(sig, act, oldact, sigsetsize)
                    let sig = self.x[0];
                    let act = self.x[1];
                    if sig == 11 && act != 0 {
                        let h = self.read_u64(act)?;
                        self.sigsegv_handler = (h != 0).then_some(h);
                    }
                    self.x[0] = 0;
                }
                94 => {
                    // exit_group(code)
                    self.exited = Some(self.x[0] as i32);
                }
                _ => return Err(format!("A64Vm: unsupported syscall {nr}")),
            }
        } else if (insn & 0x7c00_0000) == 0x1400_0000 {
            // B / BL imm26
            let is_bl = (insn >> 31) != 0;
            let imm26 = (((insn & 0x03ff_ffff) as i32) << 6) >> 6;
            if is_bl {
                self.x[30] = next_pc;
            }
            next_pc = pc.wrapping_add((i64::from(imm26) << 2) as u64);
        } else if (insn & 0xff00_0010) == 0x5400_0000 {
            // B.cond imm19
            let cond = (insn & 15) as u8;
            let imm19 = ((((insn >> 5) & 0x7_ffff) as i32) << 13) >> 13;
            if self.cond_holds(cond) {
                next_pc = pc.wrapping_add((i64::from(imm19) << 2) as u64);
            }
        } else if (insn & 0x7e00_0000) == 0x3400_0000 {
            // CBZ / CBNZ
            let is_nz = ((insn >> 24) & 1) != 0;
            let imm19 = ((((insn >> 5) & 0x7_ffff) as i32) << 13) >> 13;
            let v = self.get_x(rd, false);
            let val = if sf { v } else { u64::from(v as u32) };
            if (val != 0) == is_nz {
                next_pc = pc.wrapping_add((i64::from(imm19) << 2) as u64);
            }
        } else if (insn & 0xff9f_fc1f) == 0xd61f_0000 {
            // BR / BLR / RET
            let op = (insn >> 21) & 3;
            let target = self.get_x(rn, false);
            if op == 1 {
                self.x[30] = next_pc;
            }
            next_pc = target;
        } else if (insn & 0x1f00_0000) == 0x1000_0000 {
            // ADR / ADRP
            let is_adrp = (insn >> 31) != 0;
            let immlo = (insn >> 29) & 3;
            let immhi = (insn >> 5) & 0x7_ffff;
            let imm21 = ((((immhi << 2) | immlo) as i32) << 11) >> 11;
            let res = if is_adrp {
                (pc & !0xfff).wrapping_add((i64::from(imm21) << 12) as u64)
            } else {
                pc.wrapping_add(i64::from(imm21) as u64)
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x1f80_0000) == 0x1280_0000 {
            // MOVN / MOVZ / MOVK
            let opc = (insn >> 29) & 3;
            let hw = ((insn >> 21) & 3) * 16;
            let imm16 = u64::from((insn >> 5) & 0xffff);
            let mask = if sf { u64::MAX } else { u64::from(u32::MAX) };
            let res = match opc {
                0 => (!(imm16 << hw)) & mask,
                2 => (imm16 << hw) & mask,
                3 => {
                    let old = self.get_x(rd, false);
                    ((old & !(0xffffu64 << hw)) | (imm16 << hw)) & mask
                }
                _ => return Err(format!("A64Vm: bad mov wide {insn:#010x}")),
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x1f00_0000) == 0x1100_0000 {
            // ADD / ADDS / SUB / SUBS (immediate)
            let is_sub = ((insn >> 30) & 1) != 0;
            let s_flag = ((insn >> 29) & 1) != 0;
            let sh = ((insn >> 22) & 1) * 12;
            let imm = u64::from((insn >> 10) & 0xfff) << sh;
            let a = self.get_x(rn, true);
            let b = if is_sub { !imm } else { imm };
            let res = self.add_with_carry(sf, a, b, is_sub, s_flag);
            self.set_x(rd, res, !s_flag);
        } else if (insn & 0x1f20_0000) == 0x0b00_0000 {
            // ADD / ADDS / SUB / SUBS (shifted reg, shift=0)
            let is_sub = ((insn >> 30) & 1) != 0;
            let s_flag = ((insn >> 29) & 1) != 0;
            let a = self.get_x(rn, false);
            let raw_b = self.get_x(rm, false);
            let b = if is_sub { !raw_b } else { raw_b };
            let res = self.add_with_carry(sf, a, b, is_sub, s_flag);
            self.set_x(rd, res, false);
        } else if (insn & 0x1fe0_0000) == 0x0b20_0000 {
            // ADD / SUB extended register (`SP` delta add and `ADD Xd, Xn, Wm, UXTW`)
            let is_sub = ((insn >> 30) & 1) != 0;
            let s_flag = ((insn >> 29) & 1) != 0;
            let opt = (insn >> 13) & 7;
            let shift = (insn >> 10) & 7;
            let a = self.get_x(rn, true);
            let raw_b = self.get_x(rm, false);
            let ext_b = match opt {
                0b010 => u64::from(raw_b as u32), // UXTW
                0b011 => raw_b,                   // UXTX
                0b110 => (raw_b as i32) as i64 as u64,
                0b111 => raw_b,
                _ => return Err(format!("A64Vm: unsupported ext add opt {opt}")),
            } << shift;
            let b = if is_sub { !ext_b } else { ext_b };
            let res = self.add_with_carry(sf, a, b, is_sub, s_flag);
            self.set_x(rd, res, !s_flag);
        } else if (insn & 0x1f00_0000) == 0x0a00_0000 {
            // Logical shifted reg (AND, BIC, ORR, ORN, EOR, EON, ANDS)
            let opc = (insn >> 29) & 3;
            let inv = ((insn >> 21) & 1) != 0;
            let a = self.get_x(rn, false);
            let mut b = self.get_x(rm, false);
            if inv {
                b = !b;
            }
            let mask = if sf { u64::MAX } else { u64::from(u32::MAX) };
            let res = match opc {
                0 | 3 => (a & b) & mask,
                1 => (a | b) & mask,
                2 => (a ^ b) & mask,
                _ => unreachable!(),
            };
            if opc == 3 {
                self.n = if sf {
                    (res as i64) < 0
                } else {
                    (res as i32) < 0
                };
                self.z = res == 0;
                self.c = false;
                self.v_flag = false;
            }
            self.set_x(rd, res, false);
        } else if (insn & 0xffe0_fc00) == 0x9bc0_7c00 {
            // UMULH Xd, Xn, Xm
            let a = u128::from(self.get_x(rn, false));
            let b = u128::from(self.get_x(rm, false));
            let res = ((a * b) >> 64) as u64;
            self.set_x(rd, res, false);
        } else if (insn & 0x7fe0_fc00) == 0x1a00_0000 {
            // ADC Rd, Rn, Rm
            let a = self.get_x(rn, false);
            let b = self.get_x(rm, false);
            let res = self.add_with_carry(sf, a, b, self.c, false);
            self.set_x(rd, res, false);
        } else if (insn & 0x7fe0_0000) == 0x1b00_0000 {
            // MADD / MSUB
            let o0 = ((insn >> 15) & 1) != 0;
            let ra = ((insn >> 10) & 31) as u8;
            let a = self.get_x(rn, false);
            let b = self.get_x(rm, false);
            let c = self.get_x(ra, false);
            let res = if sf {
                let p = a.wrapping_mul(b);
                if o0 {
                    c.wrapping_sub(p)
                } else {
                    c.wrapping_add(p)
                }
            } else {
                let p = (a as u32).wrapping_mul(b as u32);
                let r32 = if o0 {
                    (c as u32).wrapping_sub(p)
                } else {
                    (c as u32).wrapping_add(p)
                };
                u64::from(r32)
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x7fe0_0c00) == 0x1ac0_0800 || (insn & 0x7fe0_0000) == 0x1ac0_0000 {
            // Data-processing 2-source: UDIV, SDIV, LSLV, LSRV, ASRV, RORV
            let op = (insn >> 10) & 63;
            let a = self.get_x(rn, false);
            let b = self.get_x(rm, false);
            let res = if sf {
                match op {
                    2 => a.checked_div(b).unwrap_or(0),
                    3 => {
                        let (sa, sb) = (a as i64, b as i64);
                        if sb == 0 {
                            0
                        } else if sa == i64::MIN && sb == -1 {
                            i64::MIN as u64
                        } else {
                            (sa / sb) as u64
                        }
                    }
                    8 => a.wrapping_shl(b as u32),
                    9 => a.wrapping_shr(b as u32),
                    10 => ((a as i64).wrapping_shr(b as u32)) as u64,
                    11 => a.rotate_right(b as u32),
                    _ => return Err(format!("A64Vm: unsupported dp2 op {op} insn={insn:#010x}")),
                }
            } else {
                let (a32, b32) = (a as u32, b as u32);
                let r32 = match op {
                    2 => a32.checked_div(b32).unwrap_or(0),
                    3 => {
                        let (sa, sb) = (a32 as i32, b32 as i32);
                        if sb == 0 {
                            0
                        } else if sa == i32::MIN && sb == -1 {
                            i32::MIN as u32
                        } else {
                            (sa / sb) as u32
                        }
                    }
                    8 => a32.wrapping_shl(b32),
                    9 => a32.wrapping_shr(b32),
                    10 => ((a32 as i32).wrapping_shr(b32)) as u32,
                    11 => a32.rotate_right(b32),
                    _ => return Err(format!("A64Vm: unsupported dp2 op {op}")),
                };
                u64::from(r32)
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x7fff_0000) == 0x5ac0_0000 {
            // Data-processing 1-source: RBIT, CLZ
            let op = (insn >> 10) & 63;
            let a = self.get_x(rn, false);
            let res = if sf {
                match op {
                    0 => a.reverse_bits(),
                    4 => u64::from(a.leading_zeros()),
                    _ => return Err(format!("A64Vm: unsupported dp1 op {op}")),
                }
            } else {
                let a32 = a as u32;
                let r32 = match op {
                    0 => a32.reverse_bits(),
                    4 => a32.leading_zeros(),
                    _ => return Err(format!("A64Vm: unsupported dp1 op {op}")),
                };
                u64::from(r32)
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x1f80_0000) == 0x1300_0000 {
            // SBFM / UBFM
            let opc = (insn >> 29) & 3;
            let immr = (insn >> 16) & 63;
            let imms = (insn >> 10) & 63;
            let w: u32 = if sf { 64 } else { 32 };
            let src = if sf {
                self.get_x(rn, false)
            } else {
                u64::from(self.get_x(rn, false) as u32)
            };
            let res = if imms >= immr {
                let width = imms - immr + 1;
                let shifted = src >> immr;
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                let bits = shifted & mask;
                if opc == 0 {
                    // SBFM sign-extend bit `width - 1`
                    let sign = (bits >> (width - 1)) & 1;
                    let ext = if sign != 0 && width < 64 {
                        bits | !mask
                    } else {
                        bits
                    };
                    if sf { ext } else { u64::from(ext as u32) }
                } else {
                    if sf { bits } else { u64::from(bits as u32) }
                }
            } else {
                // LSL-style UBFM/SBFM (`imms < immr`)
                let lsh = w - immr;
                let width = imms + 1;
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                let bits = (src & mask) << lsh;
                if sf { bits } else { u64::from(bits as u32) }
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x7fa0_0000) == 0x1380_0000 {
            // EXTR (ROR #imms)
            let imms = (insn >> 10) & 63;
            let a = self.get_x(rn, false);
            let res = if sf {
                a.rotate_right(imms)
            } else {
                u64::from((a as u32).rotate_right(imms))
            };
            self.set_x(rd, res, false);
        } else if (insn & 0x7fe0_0800) == 0x1a80_0000 {
            // CSEL / CSINC
            let o2 = ((insn >> 10) & 1) != 0;
            let cond = ((insn >> 12) & 15) as u8;
            let a = self.get_x(rn, false);
            let b = self.get_x(rm, false);
            let raw = if self.cond_holds(cond) {
                a
            } else if o2 {
                b.wrapping_add(1)
            } else {
                b
            };
            let res = if sf { raw } else { u64::from(raw as u32) };
            self.set_x(rd, res, false);
        } else if (insn & 0x3a00_0000) == 0x2800_0000 {
            // STP / LDP (GP 64-bit & SIMD 128-bit)
            let is_q = ((insn >> 26) & 1) != 0;
            let mode = (insn >> 23) & 7;
            let is_load = ((insn >> 22) & 1) != 0;
            let imm7 = ((((insn >> 15) & 0x7f) as i32) << 25) >> 25;
            let rt2 = ((insn >> 10) & 31) as u8;
            let scale = if is_q { 16i64 } else { 8i64 };
            let delta = i64::from(imm7) * scale;
            let base = self.get_x(rn, true);
            let addr = match mode {
                0b010 => base.wrapping_add(delta as u64),
                0b011 => {
                    let na = base.wrapping_add(delta as u64);
                    self.set_x(rn, na, true);
                    na
                }
                0b001 => {
                    let na = base.wrapping_add(delta as u64);
                    self.set_x(rn, na, true);
                    base
                }
                _ => return Err(format!("A64Vm: unsupported ldp/stp mode {mode}")),
            };
            if is_q {
                if is_load {
                    let b1: [u8; 16] = self.read_bytes(addr, 16)?.try_into().unwrap();
                    let b2: [u8; 16] = self.read_bytes(addr + 16, 16)?.try_into().unwrap();
                    self.v[rd as usize] = b1;
                    self.v[rt2 as usize] = b2;
                } else {
                    let b1 = self.v[rd as usize];
                    let b2 = self.v[rt2 as usize];
                    self.write_bytes(addr, &b1)?;
                    self.write_bytes(addr + 16, &b2)?;
                }
            } else if is_load {
                let v1 = self.read_u64(addr)?;
                let v2 = self.read_u64(addr + 8)?;
                self.set_x(rd, v1, false);
                self.set_x(rt2, v2, false);
            } else {
                let v1 = self.get_x(rd, false);
                let v2 = self.get_x(rt2, false);
                self.write_bytes(addr, &v1.to_le_bytes())?;
                self.write_bytes(addr + 8, &v2.to_le_bytes())?;
            }
        } else if (insn & 0x3b00_0000) == 0x3900_0000 || (insn & 0x3b20_0c00) == 0x3820_0800 {
            // Load/Store (unsigned immediate or register offset)
            let size = (insn >> 30) & 3;
            let is_fp = ((insn >> 26) & 1) != 0;
            let opc = (insn >> 22) & 3;
            let is_uimm = ((insn >> 24) & 1) != 0;
            let base = self.get_x(rn, true);
            let addr = if is_uimm {
                let shift = if is_fp && size == 0 && opc >= 2 {
                    4
                } else {
                    size
                };
                let imm12 = u64::from((insn >> 10) & 0xfff);
                base.wrapping_add(imm12 << shift)
            } else {
                let opt = (insn >> 13) & 7;
                let s_bit = ((insn >> 12) & 1) != 0;
                let idx = self.get_x(rm, false);
                let base_off = if opt == 0b010 {
                    u64::from(idx as u32)
                } else {
                    idx
                };
                let shift = if !s_bit {
                    0
                } else if is_fp && size == 0 && opc >= 2 {
                    4
                } else {
                    size
                };
                base.wrapping_add(base_off << shift)
            };
            if is_fp {
                match (size, opc) {
                    (2, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 4)? {
                            self.v[rd as usize] = [0; 16];
                            self.v[rd as usize][0..4].copy_from_slice(&b);
                        }
                    }
                    (2, 0) => {
                        let b = self.v[rd as usize][0..4].to_vec();
                        self.write_bytes_checked(addr, &b)?;
                    }
                    (3, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 8)? {
                            self.v[rd as usize] = [0; 16];
                            self.v[rd as usize][0..8].copy_from_slice(&b);
                        }
                    }
                    (3, 0) => {
                        let b = self.v[rd as usize][0..8].to_vec();
                        self.write_bytes_checked(addr, &b)?;
                    }
                    (0, 3) => {
                        if let Some(b) = self.read_bytes_checked(addr, 16)? {
                            self.v[rd as usize].copy_from_slice(&b);
                        }
                    }
                    (0, 2) => {
                        let b = self.v[rd as usize];
                        self.write_bytes_checked(addr, &b)?;
                    }
                    _ => return Err(format!("A64Vm: unsupported FP ldst {insn:#010x}")),
                }
            } else {
                match (size, opc) {
                    (0, 0) => {
                        let b = self.get_x(rd, false) as u8;
                        self.write_bytes_checked(addr, &[b])?;
                    }
                    (0, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 1)? {
                            self.set_w(rd, u32::from(b[0]), false);
                        }
                    }
                    (0, 2) => {
                        if let Some(b) = self.read_bytes_checked(addr, 1)? {
                            self.set_x(rd, i64::from(b[0] as i8) as u64, false);
                        }
                    }
                    (0, 3) => {
                        if let Some(b) = self.read_bytes_checked(addr, 1)? {
                            self.set_w(rd, i32::from(b[0] as i8) as u32, false);
                        }
                    }
                    (1, 0) => {
                        let h = (self.get_x(rd, false) as u16).to_le_bytes();
                        self.write_bytes_checked(addr, &h)?;
                    }
                    (1, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 2)? {
                            let h = u16::from_le_bytes(b.try_into().unwrap());
                            self.set_w(rd, u32::from(h), false);
                        }
                    }
                    (1, 2) => {
                        if let Some(b) = self.read_bytes_checked(addr, 2)? {
                            let h = i16::from_le_bytes(b.try_into().unwrap());
                            self.set_x(rd, i64::from(h) as u64, false);
                        }
                    }
                    (1, 3) => {
                        if let Some(b) = self.read_bytes_checked(addr, 2)? {
                            let h = i16::from_le_bytes(b.try_into().unwrap());
                            self.set_w(rd, i32::from(h) as u32, false);
                        }
                    }
                    (2, 0) => {
                        let w = (self.get_x(rd, false) as u32).to_le_bytes();
                        self.write_bytes_checked(addr, &w)?;
                    }
                    (2, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 4)? {
                            let w = u32::from_le_bytes(b.try_into().unwrap());
                            self.set_w(rd, w, false);
                        }
                    }
                    (2, 2) => {
                        if let Some(b) = self.read_bytes_checked(addr, 4)? {
                            let w = u32::from_le_bytes(b.try_into().unwrap()) as i32;
                            self.set_x(rd, i64::from(w) as u64, false);
                        }
                    }
                    (3, 0) => {
                        let q = self.get_x(rd, false).to_le_bytes();
                        self.write_bytes_checked(addr, &q)?;
                    }
                    (3, 1) => {
                        if let Some(b) = self.read_bytes_checked(addr, 8)? {
                            let q = u64::from_le_bytes(b.try_into().unwrap());
                            self.set_x(rd, q, false);
                        }
                    }
                    _ => return Err(format!("A64Vm: unsupported GP ldst {insn:#010x}")),
                }
            }
            if let Some(fault_addr) = self.pending_fault_addr.take() {
                self.dispatch_sigsegv(fault_addr)?;
                return Ok(());
            }
        } else if (insn & 0x5f20_0000) == 0x1e20_0000 {
            // Scalar FP data processing, compare, select, int<->fp convert/move
            let ftype = (insn >> 22) & 3;
            let is64 = ftype == 1;
            let get_f32 = |v: &[[u8; 16]; 32], r: u8| {
                f32::from_le_bytes(v[r as usize][0..4].try_into().unwrap())
            };
            let get_f64 = |v: &[[u8; 16]; 32], r: u8| {
                f64::from_le_bytes(v[r as usize][0..8].try_into().unwrap())
            };
            let set_f32 = |v: &mut [[u8; 16]; 32], r: u8, val: f32| {
                v[r as usize] = [0; 16];
                v[r as usize][0..4].copy_from_slice(&val.to_le_bytes());
            };
            let set_f64 = |v: &mut [[u8; 16]; 32], r: u8, val: f64| {
                v[r as usize] = [0; 16];
                v[r as usize][0..8].copy_from_slice(&val.to_le_bytes());
            };

            if (insn & 0x7f20_0c00) == 0x1e20_0800 {
                // Binary FP: FMUL/FDIV/FADD/FSUB/FMAX/FMIN
                let op = (insn >> 12) & 15;
                if is64 {
                    let (a, b) = (get_f64(&self.v, rn), get_f64(&self.v, rm));
                    let r = match op {
                        0 => a * b,
                        1 => a / b,
                        2 => a + b,
                        3 => a - b,
                        4 => wasm_f64_max(a, b),
                        5 => wasm_f64_min(a, b),
                        _ => return Err(format!("A64Vm: bad fp bin {op}")),
                    };
                    set_f64(&mut self.v, rd, r);
                } else {
                    let (a, b) = (get_f32(&self.v, rn), get_f32(&self.v, rm));
                    let r = match op {
                        0 => a * b,
                        1 => a / b,
                        2 => a + b,
                        3 => a - b,
                        4 => wasm_f32_max(a, b),
                        5 => wasm_f32_min(a, b),
                        _ => return Err(format!("A64Vm: bad fp bin {op}")),
                    };
                    set_f32(&mut self.v, rd, r);
                }
            } else if (insn & 0x7f20_fc00) == 0x1e20_4000 {
                // Unary FP: FMOV, FABS, FNEG, FSQRT, FCVT, FRINT*
                let op = (insn >> 15) & 63;
                match (ftype, op) {
                    (0, 5) => {
                        let a = get_f32(&self.v, rn);
                        set_f64(&mut self.v, rd, f64::from(a));
                    }
                    (1, 4) => {
                        let a = get_f64(&self.v, rn);
                        set_f32(&mut self.v, rd, a as f32);
                    }
                    (0, _) => {
                        let a = get_f32(&self.v, rn);
                        let r = match op {
                            0 => a,
                            1 => a.abs(),
                            2 => -a,
                            3 => a.sqrt(),
                            8 => a.round_ties_even(),
                            9 => a.ceil(),
                            10 => a.floor(),
                            11 => a.trunc(),
                            _ => return Err(format!("A64Vm: bad fp un {op}")),
                        };
                        set_f32(&mut self.v, rd, r);
                    }
                    (1, _) => {
                        let a = get_f64(&self.v, rn);
                        let r = match op {
                            0 => a,
                            1 => a.abs(),
                            2 => -a,
                            3 => a.sqrt(),
                            8 => a.round_ties_even(),
                            9 => a.ceil(),
                            10 => a.floor(),
                            11 => a.trunc(),
                            _ => return Err(format!("A64Vm: bad fp un {op}")),
                        };
                        set_f64(&mut self.v, rd, r);
                    }
                    _ => return Err(format!("A64Vm: bad ftype {ftype}")),
                }
            } else if (insn & 0x7f20_fc1f) == 0x1e20_2000 {
                // FCMP
                let (ord, eq, lt) = if is64 {
                    let (a, b) = (get_f64(&self.v, rn), get_f64(&self.v, rm));
                    if a.is_nan() || b.is_nan() {
                        (false, false, false)
                    } else {
                        (true, a == b, a < b)
                    }
                } else {
                    let (a, b) = (get_f32(&self.v, rn), get_f32(&self.v, rm));
                    if a.is_nan() || b.is_nan() {
                        (false, false, false)
                    } else {
                        (true, a == b, a < b)
                    }
                };
                if !ord {
                    self.n = false;
                    self.z = false;
                    self.c = true;
                    self.v_flag = true;
                } else if eq {
                    self.n = false;
                    self.z = true;
                    self.c = true;
                    self.v_flag = false;
                } else if lt {
                    self.n = true;
                    self.z = false;
                    self.c = false;
                    self.v_flag = false;
                } else {
                    self.n = false;
                    self.z = false;
                    self.c = true;
                    self.v_flag = false;
                }
            } else if (insn & 0x7f20_0c00) == 0x1e20_0c00 {
                // FCSEL
                let cond = ((insn >> 12) & 15) as u8;
                let src = if self.cond_holds(cond) { rn } else { rm };
                if is64 {
                    let val = get_f64(&self.v, src);
                    set_f64(&mut self.v, rd, val);
                } else {
                    let val = get_f32(&self.v, src);
                    set_f32(&mut self.v, rd, val);
                }
            } else if (insn & 0x7f20_fc00) == 0x1e20_0000 {
                let rmode = (insn >> 19) & 3;
                let opcode = (insn >> 16) & 7;
                match (rmode, opcode) {
                    (0, 2) => {
                        // SCVTF
                        let v = self.get_x(rn, false);
                        if is64 {
                            let f = if sf {
                                (v as i64) as f64
                            } else {
                                f64::from(v as i32)
                            };
                            set_f64(&mut self.v, rd, f);
                        } else {
                            let f = if sf {
                                (v as i64) as f32
                            } else {
                                (v as i32) as f32
                            };
                            set_f32(&mut self.v, rd, f);
                        }
                    }
                    (0, 3) => {
                        // UCVTF
                        let v = self.get_x(rn, false);
                        if is64 {
                            let f = if sf { v as f64 } else { f64::from(v as u32) };
                            set_f64(&mut self.v, rd, f);
                        } else {
                            let f = if sf { v as f32 } else { (v as u32) as f32 };
                            set_f32(&mut self.v, rd, f);
                        }
                    }
                    (3, 0) => {
                        // FCVTZS (saturating!)
                        let res = match (sf, is64) {
                            (false, false) => u64::from((get_f32(&self.v, rn) as i32) as u32),
                            (false, true) => u64::from((get_f64(&self.v, rn) as i32) as u32),
                            (true, false) => (get_f32(&self.v, rn) as i64) as u64,
                            (true, true) => (get_f64(&self.v, rn) as i64) as u64,
                        };
                        self.set_x(rd, res, false);
                    }
                    (3, 1) => {
                        // FCVTZU (saturating!)
                        let res = match (sf, is64) {
                            (false, false) => u64::from(get_f32(&self.v, rn) as u32),
                            (false, true) => u64::from(get_f64(&self.v, rn) as u32),
                            (true, false) => get_f32(&self.v, rn) as u64,
                            (true, true) => get_f64(&self.v, rn) as u64,
                        };
                        self.set_x(rd, res, false);
                    }
                    (0, 6) => {
                        // FMOV GP <- FP
                        if is64 {
                            let b =
                                u64::from_le_bytes(self.v[rn as usize][0..8].try_into().unwrap());
                            self.set_x(rd, b, false);
                        } else {
                            let b =
                                u32::from_le_bytes(self.v[rn as usize][0..4].try_into().unwrap());
                            self.set_w(rd, b, false);
                        }
                    }
                    (0, 7) => {
                        // FMOV FP <- GP
                        let v = self.get_x(rn, false);
                        self.v[rd as usize] = [0; 16];
                        if is64 {
                            self.v[rd as usize][0..8].copy_from_slice(&v.to_le_bytes());
                        } else {
                            self.v[rd as usize][0..4].copy_from_slice(&(v as u32).to_le_bytes());
                        }
                    }
                    _ => return Err(format!("A64Vm: bad fp-int op {insn:#010x}")),
                }
            } else {
                return Err(format!("A64Vm: unsupported FP insn {insn:#010x}"));
            }
        } else {
            // AdvSIMD (NEON) instructions
            self.step_neon(insn)?;
        }

        self.pc = next_pc;
        Ok(())
    }

    fn step_neon(&mut self, insn: u32) -> Result<(), String> {
        let rd = (insn & 31) as usize;
        let rn = ((insn >> 5) & 31) as usize;
        let rm = ((insn >> 16) & 31) as usize;
        if (insn & 0xffff_ffe0) == 0x4f00_0400 {
            // MOVI Vd.16B, #0
            self.v[rd] = [0; 16];
        } else if (insn & 0xbfff_fc00) == 0x0e20_5800 {
            // CNT Vd.8B / 16B
            let q = ((insn >> 30) & 1) != 0;
            let len = if q { 16 } else { 8 };
            let src = self.v[rn];
            let mut out = [0u8; 16];
            for i in 0..len {
                out[i] = src[i].count_ones() as u8;
            }
            self.v[rd] = out;
        } else if (insn & 0xffff_fc00) == 0x0e31_b800 {
            // ADDV Bd, Vn.8B
            let sum: u8 = self.v[rn][0..8].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            self.v[rd] = [0; 16];
            self.v[rd][0] = sum;
        } else if (insn & 0xffe0_fc00) == 0x4e00_0000 {
            // TBL Vd.16B, {Vn.16B}, Vm.16B
            let tbl = self.v[rn];
            let idx = self.v[rm];
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = if idx[i] < 16 { tbl[idx[i] as usize] } else { 0 };
            }
            self.v[rd] = out;
        } else if (insn & 0xdf3f_fc00) == 0x4e20_2800 {
            // SADDLP (U=0) / UADDLP (U=1) 128-bit
            let unsigned = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let src = self.v[rn];
            let mut out = [0u8; 16];
            match (unsigned, size) {
                (true, 0) => {
                    for i in 0..8 {
                        let s = u16::from(src[2 * i]) + u16::from(src[2 * i + 1]);
                        out[2 * i..2 * i + 2].copy_from_slice(&s.to_le_bytes());
                    }
                }
                (false, 0) => {
                    for i in 0..8 {
                        let s = i16::from(src[2 * i] as i8) + i16::from(src[2 * i + 1] as i8);
                        out[2 * i..2 * i + 2].copy_from_slice(&s.to_le_bytes());
                    }
                }
                (true, 1) => {
                    for i in 0..4 {
                        let a = u16::from_le_bytes(src[4 * i..4 * i + 2].try_into().unwrap());
                        let b = u16::from_le_bytes(src[4 * i + 2..4 * i + 4].try_into().unwrap());
                        out[4 * i..4 * i + 4]
                            .copy_from_slice(&(u32::from(a) + u32::from(b)).to_le_bytes());
                    }
                }
                (false, 1) => {
                    for i in 0..4 {
                        let a = i16::from_le_bytes(src[4 * i..4 * i + 2].try_into().unwrap());
                        let b = i16::from_le_bytes(src[4 * i + 2..4 * i + 4].try_into().unwrap());
                        out[4 * i..4 * i + 4]
                            .copy_from_slice(&(i32::from(a) + i32::from(b)).to_le_bytes());
                    }
                }
                (true, 2) => {
                    for i in 0..2 {
                        let a = u32::from_le_bytes(src[8 * i..8 * i + 4].try_into().unwrap());
                        let b = u32::from_le_bytes(src[8 * i + 4..8 * i + 8].try_into().unwrap());
                        out[8 * i..8 * i + 8]
                            .copy_from_slice(&(u64::from(a) + u64::from(b)).to_le_bytes());
                    }
                }
                (false, 2) => {
                    for i in 0..2 {
                        let a = i32::from_le_bytes(src[8 * i..8 * i + 4].try_into().unwrap());
                        let b = i32::from_le_bytes(src[8 * i + 4..8 * i + 8].try_into().unwrap());
                        out[8 * i..8 * i + 8]
                            .copy_from_slice(&(i64::from(a) + i64::from(b)).to_le_bytes());
                    }
                }
                _ => return Err(format!("A64Vm: bad addlp size {size}")),
            }
            self.v[rd] = out;
        } else if (insn & 0xff3f_fc00) == 0x6e31_a800 {
            // UMINV Vd, Vn (128-bit)
            let size = (insn >> 22) & 3;
            let src = self.v[rn];
            let mut out = [0u8; 16];
            match size {
                0 => out[0] = *src.iter().min().unwrap(),
                1 => {
                    let m = (0..8)
                        .map(|i| u16::from_le_bytes(src[2 * i..2 * i + 2].try_into().unwrap()))
                        .min()
                        .unwrap();
                    out[0..2].copy_from_slice(&m.to_le_bytes());
                }
                2 => {
                    let m = (0..4)
                        .map(|i| u32::from_le_bytes(src[4 * i..4 * i + 4].try_into().unwrap()))
                        .min()
                        .unwrap();
                    out[0..4].copy_from_slice(&m.to_le_bytes());
                }
                _ => return Err(format!("A64Vm: bad uminv size {size}")),
            }
            self.v[rd] = out;
        } else if (insn & 0xdf3f_fc00) == 0x4e20_b800 {
            // ABS (U=0) / NEG (U=1) 128-bit integer
            let is_neg = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let src = self.v[rn];
            let mut out = [0u8; 16];
            match size {
                0 => {
                    for i in 0..16 {
                        let x = src[i] as i8;
                        out[i] = (if is_neg {
                            x.wrapping_neg()
                        } else {
                            x.wrapping_abs()
                        }) as u8;
                    }
                }
                1 => {
                    for i in 0..8 {
                        let x = i16::from_le_bytes(src[2 * i..2 * i + 2].try_into().unwrap());
                        let r = if is_neg {
                            x.wrapping_neg()
                        } else {
                            x.wrapping_abs()
                        };
                        out[2 * i..2 * i + 2].copy_from_slice(&r.to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..4 {
                        let x = i32::from_le_bytes(src[4 * i..4 * i + 4].try_into().unwrap());
                        let r = if is_neg {
                            x.wrapping_neg()
                        } else {
                            x.wrapping_abs()
                        };
                        out[4 * i..4 * i + 4].copy_from_slice(&r.to_le_bytes());
                    }
                }
                3 => {
                    for i in 0..2 {
                        let x = i64::from_le_bytes(src[8 * i..8 * i + 8].try_into().unwrap());
                        let r = if is_neg {
                            x.wrapping_neg()
                        } else {
                            x.wrapping_abs()
                        };
                        out[8 * i..8 * i + 8].copy_from_slice(&r.to_le_bytes());
                    }
                }
                _ => unreachable!(),
            }
            self.v[rd] = out;
        } else if (insn & 0xff20_fc00) == 0x4e20_bc00 {
            // ADDP 128-bit pairwise add
            let size = (insn >> 22) & 3;
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut out = [0u8; 16];
            match size {
                0 => {
                    for i in 0..8 {
                        out[i] = a[2 * i].wrapping_add(a[2 * i + 1]);
                        out[8 + i] = b[2 * i].wrapping_add(b[2 * i + 1]);
                    }
                }
                1 => {
                    for i in 0..4 {
                        let x0 = u16::from_le_bytes(a[4 * i..4 * i + 2].try_into().unwrap());
                        let x1 = u16::from_le_bytes(a[4 * i + 2..4 * i + 4].try_into().unwrap());
                        let y0 = u16::from_le_bytes(b[4 * i..4 * i + 2].try_into().unwrap());
                        let y1 = u16::from_le_bytes(b[4 * i + 2..4 * i + 4].try_into().unwrap());
                        out[2 * i..2 * i + 2].copy_from_slice(&x0.wrapping_add(x1).to_le_bytes());
                        out[8 + 2 * i..8 + 2 * i + 2]
                            .copy_from_slice(&y0.wrapping_add(y1).to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..2 {
                        let x0 = u32::from_le_bytes(a[8 * i..8 * i + 4].try_into().unwrap());
                        let x1 = u32::from_le_bytes(a[8 * i + 4..8 * i + 8].try_into().unwrap());
                        let y0 = u32::from_le_bytes(b[8 * i..8 * i + 4].try_into().unwrap());
                        let y1 = u32::from_le_bytes(b[8 * i + 4..8 * i + 8].try_into().unwrap());
                        out[4 * i..4 * i + 4].copy_from_slice(&x0.wrapping_add(x1).to_le_bytes());
                        out[8 + 4 * i..8 + 4 * i + 4]
                            .copy_from_slice(&y0.wrapping_add(y1).to_le_bytes());
                    }
                }
                3 => {
                    let x0 = u64::from_le_bytes(a[0..8].try_into().unwrap());
                    let x1 = u64::from_le_bytes(a[8..16].try_into().unwrap());
                    let y0 = u64::from_le_bytes(b[0..8].try_into().unwrap());
                    let y1 = u64::from_le_bytes(b[8..16].try_into().unwrap());
                    out[0..8].copy_from_slice(&x0.wrapping_add(x1).to_le_bytes());
                    out[8..16].copy_from_slice(&y0.wrapping_add(y1).to_le_bytes());
                }
                _ => unreachable!(),
            }
            self.v[rd] = out;
        } else if (insn & 0xdf20_fc00) == 0x4e20_0c00
            || (insn & 0xdf20_fc00) == 0x4e20_2c00
            || (insn & 0xdf20_fc00) == 0x4e20_3400
            || (insn & 0xdf20_fc00) == 0x4e20_3c00
            || (insn & 0xdf20_fc00) == 0x4e20_4400
            || (insn & 0xdf20_fc00) == 0x4e20_6400
            || (insn & 0xdf20_fc00) == 0x4e20_6c00
            || (insn & 0xff20_fc00) == 0x6e20_8c00
            || (insn & 0xff20_fc00) == 0x6e20_1400
            || (insn & 0xff20_fc00) == 0x6e20_b400
        {
            // Three-same 128-bit integer ops: SQADD/UQADD/SQSUB/UQSUB/CMEQ/CMGT/CMGE/CMHI/CMHS/
            // SSHL/USHL/SMAX/UMAX/SMIN/UMIN/URHADD/SQRDMULH
            let u = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let opc = (insn >> 11) & 31;
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut out = [0u8; 16];
            let lane_bytes = 1usize << size;
            let n_lanes = 16 / lane_bytes;
            for i in 0..n_lanes {
                let s = i * lane_bytes;
                match size {
                    0 => {
                        let (au, bu) = (a[s], b[s]);
                        let (ai, bi) = (au as i8, bu as i8);
                        out[s] = match (u, opc) {
                            (false, 0b00001) => ai.saturating_add(bi) as u8,
                            (true, 0b00001) => au.saturating_add(bu),
                            (false, 0b00101) => ai.saturating_sub(bi) as u8,
                            (true, 0b00101) => au.saturating_sub(bu),
                            (true, 0b10001) => {
                                if au == bu {
                                    0xff
                                } else {
                                    0
                                }
                            }
                            (false, 0b00110) => {
                                if ai > bi {
                                    0xff
                                } else {
                                    0
                                }
                            }
                            (false, 0b00111) => {
                                if ai >= bi {
                                    0xff
                                } else {
                                    0
                                }
                            }
                            (true, 0b00110) => {
                                if au > bu {
                                    0xff
                                } else {
                                    0
                                }
                            }
                            (true, 0b00111) => {
                                if au >= bu {
                                    0xff
                                } else {
                                    0
                                }
                            }
                            (false, 0b01100) => ai.max(bi) as u8,
                            (true, 0b01100) => au.max(bu),
                            (false, 0b01101) => ai.min(bi) as u8,
                            (true, 0b01101) => au.min(bu),
                            (true, 0b00010) => ((u16::from(au) + u16::from(bu) + 1) >> 1) as u8,
                            (false, 0b01000) => {
                                let sh = bi;
                                if sh >= 0 {
                                    if sh >= 8 { 0 } else { (ai << sh) as u8 }
                                } else {
                                    let r = (-sh).min(7);
                                    (ai >> r) as u8
                                }
                            }
                            (true, 0b01000) => {
                                let sh = bi;
                                if sh >= 0 {
                                    if sh >= 8 { 0 } else { au << sh }
                                } else if -sh >= 8 {
                                    0
                                } else {
                                    au >> (-sh)
                                }
                            }
                            _ => return Err(format!("A64Vm: bad neon3s8 u={u} opc={opc:#x}")),
                        };
                    }
                    1 => {
                        let au = u16::from_le_bytes(a[s..s + 2].try_into().unwrap());
                        let bu = u16::from_le_bytes(b[s..s + 2].try_into().unwrap());
                        let (ai, bi) = (au as i16, bu as i16);
                        let r: u16 = match (u, opc) {
                            (false, 0b00001) => ai.saturating_add(bi) as u16,
                            (true, 0b00001) => au.saturating_add(bu),
                            (false, 0b00101) => ai.saturating_sub(bi) as u16,
                            (true, 0b00101) => au.saturating_sub(bu),
                            (true, 0b10001) => {
                                if au == bu {
                                    u16::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00110) => {
                                if ai > bi {
                                    u16::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00111) => {
                                if ai >= bi {
                                    u16::MAX
                                } else {
                                    0
                                }
                            }
                            (true, 0b00110) => {
                                if au > bu {
                                    u16::MAX
                                } else {
                                    0
                                }
                            }
                            (true, 0b00111) => {
                                if au >= bu {
                                    u16::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b01100) => ai.max(bi) as u16,
                            (true, 0b01100) => au.max(bu),
                            (false, 0b01101) => ai.min(bi) as u16,
                            (true, 0b01101) => au.min(bu),
                            (true, 0b00010) => ((u32::from(au) + u32::from(bu) + 1) >> 1) as u16,
                            (true, 0b10110) => {
                                // SQRDMULH
                                let prod = i32::from(ai) * i32::from(bi);
                                let rounded = (prod + 0x4000) >> 15;
                                rounded.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
                                    as u16
                            }
                            (false, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 16 { 0 } else { (ai << sh) as u16 }
                                } else {
                                    let r = (-sh).min(15);
                                    (ai >> r) as u16
                                }
                            }
                            (true, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 16 { 0 } else { au << sh }
                                } else if -sh >= 16 {
                                    0
                                } else {
                                    au >> (-sh)
                                }
                            }
                            _ => return Err(format!("A64Vm: bad neon3s16 u={u} opc={opc:#x}")),
                        };
                        out[s..s + 2].copy_from_slice(&r.to_le_bytes());
                    }
                    2 => {
                        let au = u32::from_le_bytes(a[s..s + 4].try_into().unwrap());
                        let bu = u32::from_le_bytes(b[s..s + 4].try_into().unwrap());
                        let (ai, bi) = (au as i32, bu as i32);
                        let r: u32 = match (u, opc) {
                            (true, 0b10001) => {
                                if au == bu {
                                    u32::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00110) => {
                                if ai > bi {
                                    u32::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00111) => {
                                if ai >= bi {
                                    u32::MAX
                                } else {
                                    0
                                }
                            }
                            (true, 0b00110) => {
                                if au > bu {
                                    u32::MAX
                                } else {
                                    0
                                }
                            }
                            (true, 0b00111) => {
                                if au >= bu {
                                    u32::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b01100) => ai.max(bi) as u32,
                            (true, 0b01100) => au.max(bu),
                            (false, 0b01101) => ai.min(bi) as u32,
                            (true, 0b01101) => au.min(bu),
                            (false, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 32 { 0 } else { (ai << sh) as u32 }
                                } else {
                                    let r = (-sh).min(31);
                                    (ai >> r) as u32
                                }
                            }
                            (true, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 32 { 0 } else { au << sh }
                                } else if -sh >= 32 {
                                    0
                                } else {
                                    au >> (-sh)
                                }
                            }
                            _ => return Err(format!("A64Vm: bad neon3s32 u={u} opc={opc:#x}")),
                        };
                        out[s..s + 4].copy_from_slice(&r.to_le_bytes());
                    }
                    3 => {
                        let au = u64::from_le_bytes(a[s..s + 8].try_into().unwrap());
                        let bu = u64::from_le_bytes(b[s..s + 8].try_into().unwrap());
                        let (ai, bi) = (au as i64, bu as i64);
                        let r: u64 = match (u, opc) {
                            (true, 0b10001) => {
                                if au == bu {
                                    u64::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00110) => {
                                if ai > bi {
                                    u64::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b00111) => {
                                if ai >= bi {
                                    u64::MAX
                                } else {
                                    0
                                }
                            }
                            (false, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 64 { 0 } else { (ai << sh) as u64 }
                                } else {
                                    let r = (-sh).min(63);
                                    (ai >> r) as u64
                                }
                            }
                            (true, 0b01000) => {
                                let sh = (bu & 0xff) as i8;
                                if sh >= 0 {
                                    if sh >= 64 { 0 } else { au << sh }
                                } else if -sh >= 64 {
                                    0
                                } else {
                                    au >> (-sh)
                                }
                            }
                            _ => return Err(format!("A64Vm: bad neon3s64 u={u} opc={opc:#x}")),
                        };
                        out[s..s + 8].copy_from_slice(&r.to_le_bytes());
                    }
                    _ => unreachable!(),
                }
            }
            self.v[rd] = out;
        } else if (insn & 0xff80_fc00) == 0x4f00_0400 && ((insn >> 19) & 15) != 0 {
            // SSHR Vd.<T>, Vn.<T>, #shift (Q=1)
            let enc = (insn >> 16) & 0x7f;
            let immh = enc >> 3;
            let (size, base_w) = if (immh & 8) != 0 {
                (3, 128)
            } else if (immh & 4) != 0 {
                (2, 64)
            } else if (immh & 2) != 0 {
                (1, 32)
            } else {
                (0, 16)
            };
            let shift = base_w - enc;
            let src = self.v[rn];
            let mut out = [0u8; 16];
            match size {
                0 => {
                    for i in 0..16 {
                        out[i] = ((src[i] as i8) >> shift.min(7)) as u8;
                    }
                }
                1 => {
                    for i in 0..8 {
                        let x = i16::from_le_bytes(src[2 * i..2 * i + 2].try_into().unwrap());
                        out[2 * i..2 * i + 2].copy_from_slice(&(x >> shift.min(15)).to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..4 {
                        let x = i32::from_le_bytes(src[4 * i..4 * i + 4].try_into().unwrap());
                        out[4 * i..4 * i + 4].copy_from_slice(&(x >> shift.min(31)).to_le_bytes());
                    }
                }
                3 => {
                    for i in 0..2 {
                        let x = i64::from_le_bytes(src[8 * i..8 * i + 8].try_into().unwrap());
                        out[8 * i..8 * i + 8].copy_from_slice(&(x >> shift.min(63)).to_le_bytes());
                    }
                }
                _ => unreachable!(),
            }
            self.v[rd] = out;
        } else if (insn & 0x9f80_fc00) == 0x0f00_a400 {
            // SXTL / SXTL2 / UXTL / UXTL2
            let high = ((insn >> 30) & 1) != 0;
            let unsigned = ((insn >> 29) & 1) != 0;
            let immh = (insn >> 19) & 15;
            let src = self.v[rn];
            let base = if high { 8 } else { 0 };
            let mut out = [0u8; 16];
            match immh {
                1 => {
                    for i in 0..8 {
                        let b = src[base + i];
                        let w: u16 = if unsigned {
                            u16::from(b)
                        } else {
                            i16::from(b as i8) as u16
                        };
                        out[2 * i..2 * i + 2].copy_from_slice(&w.to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..4 {
                        let h = u16::from_le_bytes(
                            src[base + 2 * i..base + 2 * i + 2].try_into().unwrap(),
                        );
                        let w: u32 = if unsigned {
                            u32::from(h)
                        } else {
                            i32::from(h as i16) as u32
                        };
                        out[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
                    }
                }
                4 => {
                    for i in 0..2 {
                        let s = u32::from_le_bytes(
                            src[base + 4 * i..base + 4 * i + 4].try_into().unwrap(),
                        );
                        let d: u64 = if unsigned {
                            u64::from(s)
                        } else {
                            i64::from(s as i32) as u64
                        };
                        out[8 * i..8 * i + 8].copy_from_slice(&d.to_le_bytes());
                    }
                }
                _ => return Err(format!("A64Vm: bad xtl immh {immh}")),
            }
            self.v[rd] = out;
        } else if (insn & 0xbf3f_fc00) == 0x0e21_4800 || (insn & 0xbf3f_fc00) == 0x2e21_2800 {
            // SQXTN / SQXTN2 / SQXTUN / SQXTUN2
            let high = ((insn >> 30) & 1) != 0;
            let unsigned = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let src = self.v[rn];
            let mut out = if high { self.v[rd] } else { [0u8; 16] };
            let base = if high { 8 } else { 0 };
            match size {
                0 => {
                    for i in 0..8 {
                        let x = i16::from_le_bytes(src[2 * i..2 * i + 2].try_into().unwrap());
                        out[base + i] = if unsigned {
                            x.clamp(0, 255) as u8
                        } else {
                            x.clamp(-128, 127) as i8 as u8
                        };
                    }
                }
                1 => {
                    for i in 0..4 {
                        let x = i32::from_le_bytes(src[4 * i..4 * i + 4].try_into().unwrap());
                        let h: u16 = if unsigned {
                            x.clamp(0, 65535) as u16
                        } else {
                            x.clamp(-32768, 32767) as i16 as u16
                        };
                        out[base + 2 * i..base + 2 * i + 2].copy_from_slice(&h.to_le_bytes());
                    }
                }
                _ => return Err(format!("A64Vm: bad sqxt*n size {size}")),
            }
            self.v[rd] = out;
        } else if (insn & 0x9f20_fc00) == 0x0e20_c000 {
            // SMULL / SMULL2 / UMULL / UMULL2
            let high = ((insn >> 30) & 1) != 0;
            let unsigned = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let (a, b) = (self.v[rn], self.v[rm]);
            let base = if high { 8 } else { 0 };
            let mut out = [0u8; 16];
            match size {
                0 => {
                    for i in 0..8 {
                        let r: u16 = if unsigned {
                            u16::from(a[base + i]) * u16::from(b[base + i])
                        } else {
                            (i16::from(a[base + i] as i8) * i16::from(b[base + i] as i8)) as u16
                        };
                        out[2 * i..2 * i + 2].copy_from_slice(&r.to_le_bytes());
                    }
                }
                1 => {
                    for i in 0..4 {
                        let x = u16::from_le_bytes(
                            a[base + 2 * i..base + 2 * i + 2].try_into().unwrap(),
                        );
                        let y = u16::from_le_bytes(
                            b[base + 2 * i..base + 2 * i + 2].try_into().unwrap(),
                        );
                        let r: u32 = if unsigned {
                            u32::from(x) * u32::from(y)
                        } else {
                            (i32::from(x as i16) * i32::from(y as i16)) as u32
                        };
                        out[4 * i..4 * i + 4].copy_from_slice(&r.to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..2 {
                        let x = u32::from_le_bytes(
                            a[base + 4 * i..base + 4 * i + 4].try_into().unwrap(),
                        );
                        let y = u32::from_le_bytes(
                            b[base + 4 * i..base + 4 * i + 4].try_into().unwrap(),
                        );
                        let r: u64 = if unsigned {
                            u64::from(x) * u64::from(y)
                        } else {
                            (i64::from(x as i32) * i64::from(y as i32)) as u64
                        };
                        out[8 * i..8 * i + 8].copy_from_slice(&r.to_le_bytes());
                    }
                }
                _ => return Err(format!("A64Vm: bad mull size {size}")),
            }
            self.v[rd] = out;
        } else if matches!(
            insn & 0xffbf_fc00,
            0x4e21_8800
                | 0x4e21_9800
                | 0x4ea1_8800
                | 0x4ea1_9800
                | 0x4e21_d800
                | 0x6e21_d800
                | 0x4ea1_b800
                | 0x6ea1_b800
        ) {
            // Vector FP round & int<->fp convert (FRINTN/M/P/Z, SCVTF, UCVTF, FCVTZS, FCVTZU)
            let is64 = ((insn >> 22) & 1) != 0;
            let tag = insn & 0xffbf_fc00;
            let a = self.v[rn];
            let mut out = [0u8; 16];
            if is64 {
                for i in 0..2 {
                    let s = 8 * i;
                    let raw = u64::from_le_bytes(a[s..s + 8].try_into().unwrap());
                    let f = f64::from_bits(raw);
                    let bits: u64 = match tag {
                        0x4e21_8800 => f.round_ties_even().to_bits(),
                        0x4e21_9800 => f.floor().to_bits(),
                        0x4ea1_8800 => f.ceil().to_bits(),
                        0x4ea1_9800 => f.trunc().to_bits(),
                        0x4e21_d800 => ((raw as i64) as f64).to_bits(),
                        0x6e21_d800 => (raw as f64).to_bits(),
                        0x4ea1_b800 => (f as i64) as u64,
                        0x6ea1_b800 => f as u64,
                        _ => unreachable!(),
                    };
                    out[s..s + 8].copy_from_slice(&bits.to_le_bytes());
                }
            } else {
                for i in 0..4 {
                    let s = 4 * i;
                    let raw = u32::from_le_bytes(a[s..s + 4].try_into().unwrap());
                    let f = f32::from_bits(raw);
                    let bits: u32 = match tag {
                        0x4e21_8800 => f.round_ties_even().to_bits(),
                        0x4e21_9800 => f.floor().to_bits(),
                        0x4ea1_8800 => f.ceil().to_bits(),
                        0x4ea1_9800 => f.trunc().to_bits(),
                        0x4e21_d800 => ((raw as i32) as f32).to_bits(),
                        0x6e21_d800 => (raw as f32).to_bits(),
                        0x4ea1_b800 => (f as i32) as u32,
                        0x6ea1_b800 => f as u32,
                        _ => unreachable!(),
                    };
                    out[s..s + 4].copy_from_slice(&bits.to_le_bytes());
                }
            }
            self.v[rd] = out;
        } else if (insn & 0xffff_fc00) == 0x0e61_6800 {
            // FCVTN Vd.2S, Vn.2D (clears upper 64 bits)
            let a = self.v[rn];
            let mut out = [0u8; 16];
            for i in 0..2 {
                let d = f64::from_le_bytes(a[8 * i..8 * i + 8].try_into().unwrap());
                out[4 * i..4 * i + 4].copy_from_slice(&(d as f32).to_le_bytes());
            }
            self.v[rd] = out;
        } else if (insn & 0xffff_fc00) == 0x0e61_7800 {
            // FCVTL Vd.2D, Vn.2S
            let a = self.v[rn];
            let mut out = [0u8; 16];
            for i in 0..2 {
                let s = f32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap());
                out[8 * i..8 * i + 8].copy_from_slice(&f64::from(s).to_le_bytes());
            }
            self.v[rd] = out;
        } else if (insn & 0xdf20_fc00) == 0x4e20_1c00 {
            // Bitwise 16B: AND, BIC, ORR, EOR, BSL
            let u = ((insn >> 29) & 1) != 0;
            let sz = (insn >> 22) & 3;
            let (a, b, d0) = (self.v[rn], self.v[rm], self.v[rd]);
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = match (u, sz) {
                    (false, 0) => a[i] & b[i],
                    (false, 1) => a[i] & !b[i],
                    (false, 2) => a[i] | b[i],
                    (true, 0) => a[i] ^ b[i],
                    (true, 1) => (a[i] & d0[i]) | (b[i] & !d0[i]),
                    _ => return Err(format!("A64Vm: bad neon bitop {insn:#010x}")),
                };
            }
            self.v[rd] = out;
        } else if (insn & 0xffff_fc00) == 0x6e20_5800 {
            // NOT Vd.16B, Vn.16B
            let src = self.v[rn];
            for (dst, &s) in self.v[rd].iter_mut().zip(&src) {
                *dst = !s;
            }
        } else if (insn & 0xdf20_fc00) == 0x4e20_8400 {
            // ADD / SUB 128-bit vector
            let is_sub = ((insn >> 29) & 1) != 0;
            let size = (insn >> 22) & 3;
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut out = [0u8; 16];
            match size {
                0 => {
                    for i in 0..16 {
                        out[i] = if is_sub {
                            a[i].wrapping_sub(b[i])
                        } else {
                            a[i].wrapping_add(b[i])
                        };
                    }
                }
                1 => {
                    for i in 0..8 {
                        let x = u16::from_le_bytes(a[2 * i..2 * i + 2].try_into().unwrap());
                        let y = u16::from_le_bytes(b[2 * i..2 * i + 2].try_into().unwrap());
                        let r = if is_sub {
                            x.wrapping_sub(y)
                        } else {
                            x.wrapping_add(y)
                        };
                        out[2 * i..2 * i + 2].copy_from_slice(&r.to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..4 {
                        let x = u32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap());
                        let y = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
                        let r = if is_sub {
                            x.wrapping_sub(y)
                        } else {
                            x.wrapping_add(y)
                        };
                        out[4 * i..4 * i + 4].copy_from_slice(&r.to_le_bytes());
                    }
                }
                3 => {
                    for i in 0..2 {
                        let x = u64::from_le_bytes(a[8 * i..8 * i + 8].try_into().unwrap());
                        let y = u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
                        let r = if is_sub {
                            x.wrapping_sub(y)
                        } else {
                            x.wrapping_add(y)
                        };
                        out[8 * i..8 * i + 8].copy_from_slice(&r.to_le_bytes());
                    }
                }
                _ => unreachable!(),
            }
            self.v[rd] = out;
        } else if (insn & 0xff20_fc00) == 0x4e20_9c00 {
            // MUL 128-bit vector
            let size = (insn >> 22) & 3;
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut out = [0u8; 16];
            match size {
                1 => {
                    for i in 0..8 {
                        let x = u16::from_le_bytes(a[2 * i..2 * i + 2].try_into().unwrap());
                        let y = u16::from_le_bytes(b[2 * i..2 * i + 2].try_into().unwrap());
                        out[2 * i..2 * i + 2].copy_from_slice(&x.wrapping_mul(y).to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..4 {
                        let x = u32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap());
                        let y = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
                        out[4 * i..4 * i + 4].copy_from_slice(&x.wrapping_mul(y).to_le_bytes());
                    }
                }
                _ => return Err(format!("A64Vm: bad neon mul size {size}")),
            }
            self.v[rd] = out;
        } else if matches!(
            insn & 0xffa0_fc00,
            0x4e20_d400
                | 0x4ea0_d400
                | 0x6e20_dc00
                | 0x6e20_fc00
                | 0x4e20_f400
                | 0x4ea0_f400
                | 0x6ea0_e400
                | 0x6e20_e400
                | 0x4e20_e400
        ) {
            // FP 128-bit binary (fadd/fsub/fmul/fdiv/fmax/fmin/fcmgt/fcmge/fcmeq)
            let is64 = ((insn >> 22) & 1) != 0;
            let op_id = insn & 0xffa0_fc00;
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut out = [0u8; 16];
            if is64 {
                for i in 0..2 {
                    let x = f64::from_le_bytes(a[8 * i..8 * i + 8].try_into().unwrap());
                    let y = f64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
                    let bits: u64 = match op_id {
                        0x4e20_d400 => (x + y).to_bits(),
                        0x4ea0_d400 => (x - y).to_bits(),
                        0x6e20_dc00 => (x * y).to_bits(),
                        0x6e20_fc00 => (x / y).to_bits(),
                        0x4e20_f400 => wasm_f64_max(x, y).to_bits(),
                        0x4ea0_f400 => wasm_f64_min(x, y).to_bits(),
                        0x6ea0_e400 => {
                            if x > y {
                                u64::MAX
                            } else {
                                0
                            }
                        }
                        0x6e20_e400 => {
                            if x >= y {
                                u64::MAX
                            } else {
                                0
                            }
                        }
                        0x4e20_e400 => {
                            if x == y {
                                u64::MAX
                            } else {
                                0
                            }
                        }
                        _ => return Err(format!("A64Vm: bad neon fp64 op {op_id:#x}")),
                    };
                    out[8 * i..8 * i + 8].copy_from_slice(&bits.to_le_bytes());
                }
            } else {
                for i in 0..4 {
                    let x = f32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap());
                    let y = f32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
                    let bits: u32 = match op_id {
                        0x4e20_d400 => (x + y).to_bits(),
                        0x4ea0_d400 => (x - y).to_bits(),
                        0x6e20_dc00 => (x * y).to_bits(),
                        0x6e20_fc00 => (x / y).to_bits(),
                        0x4e20_f400 => wasm_f32_max(x, y).to_bits(),
                        0x4ea0_f400 => wasm_f32_min(x, y).to_bits(),
                        0x6ea0_e400 => {
                            if x > y {
                                u32::MAX
                            } else {
                                0
                            }
                        }
                        0x6e20_e400 => {
                            if x >= y {
                                u32::MAX
                            } else {
                                0
                            }
                        }
                        0x4e20_e400 => {
                            if x == y {
                                u32::MAX
                            } else {
                                0
                            }
                        }
                        _ => return Err(format!("A64Vm: bad neon fp32 op {op_id:#x}")),
                    };
                    out[4 * i..4 * i + 4].copy_from_slice(&bits.to_le_bytes());
                }
            }
            self.v[rd] = out;
        } else if (insn & 0xdfbe_fc00) == 0x4ea0_f800 {
            // FABS / FNEG / FSQRT 128-bit
            let is64 = ((insn >> 22) & 1) != 0;
            let u = ((insn >> 29) & 1) != 0;
            let sqrt = ((insn >> 16) & 1) != 0;
            let a = self.v[rn];
            let mut out = [0u8; 16];
            if is64 {
                for i in 0..2 {
                    let x = f64::from_le_bytes(a[8 * i..8 * i + 8].try_into().unwrap());
                    let r = if sqrt {
                        x.sqrt()
                    } else if u {
                        -x
                    } else {
                        x.abs()
                    };
                    out[8 * i..8 * i + 8].copy_from_slice(&r.to_le_bytes());
                }
            } else {
                for i in 0..4 {
                    let x = f32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap());
                    let r = if sqrt {
                        x.sqrt()
                    } else if u {
                        -x
                    } else {
                        x.abs()
                    };
                    out[4 * i..4 * i + 4].copy_from_slice(&r.to_le_bytes());
                }
            }
            self.v[rd] = out;
        } else if (insn & 0xffe0_fc00) == 0x4e00_0c00 {
            // DUP Vd.<T>, Rn
            let imm5 = (insn >> 16) & 31;
            let tz = imm5.trailing_zeros();
            let sz = 1usize << tz;
            let val = self.get_x(rn as u8, false).to_le_bytes();
            for i in 0..(16 / sz) {
                self.v[rd][i * sz..(i + 1) * sz].copy_from_slice(&val[0..sz]);
            }
        } else if (insn & 0xffe0_fc00) == 0x4e00_0400 {
            // DUP Vd.<T>, Vn.<t>[lane]
            let imm5 = (insn >> 16) & 31;
            let tz = imm5.trailing_zeros();
            let sz = 1usize << tz;
            let lane = (imm5 >> (tz + 1)) as usize;
            let elt = self.v[rn][lane * sz..(lane + 1) * sz].to_vec();
            for i in 0..(16 / sz) {
                self.v[rd][i * sz..(i + 1) * sz].copy_from_slice(&elt);
            }
        } else if (insn & 0xbfe0_ec00) == 0x0e00_2c00 {
            // UMOV / SMOV
            let is_signed = ((insn >> 12) & 1) == 0;
            let is64 = ((insn >> 30) & 1) != 0;
            let imm5 = (insn >> 16) & 31;
            let tz = imm5.trailing_zeros();
            let sz = 1usize << tz;
            let lane = (imm5 >> (tz + 1)) as usize;
            let slice = &self.v[rn][lane * sz..(lane + 1) * sz];
            let res: u64 = match (sz, is_signed, is64) {
                (1, false, _) => u64::from(slice[0]),
                (1, true, false) => u64::from((slice[0] as i8) as i32 as u32),
                (1, true, true) => (slice[0] as i8) as i64 as u64,
                (2, false, _) => u64::from(u16::from_le_bytes(slice.try_into().unwrap())),
                (2, true, false) => {
                    u64::from((i16::from_le_bytes(slice.try_into().unwrap())) as i32 as u32)
                }
                (2, true, true) => (i16::from_le_bytes(slice.try_into().unwrap())) as i64 as u64,
                (4, false, _) => u64::from(u32::from_le_bytes(slice.try_into().unwrap())),
                (4, true, _) => (i32::from_le_bytes(slice.try_into().unwrap())) as i64 as u64,
                (8, _, _) => u64::from_le_bytes(slice.try_into().unwrap()),
                _ => return Err(format!("A64Vm: bad umov/smov sz={sz}")),
            };
            self.set_x(rd as u8, res, false);
        } else if (insn & 0xffe0_fc00) == 0x4e00_1c00 {
            // INS Vd.<t>[lane], Rn
            let imm5 = (insn >> 16) & 31;
            let tz = imm5.trailing_zeros();
            let sz = 1usize << tz;
            let lane = (imm5 >> (tz + 1)) as usize;
            let val = self.get_x(rn as u8, false).to_le_bytes();
            self.v[rd][lane * sz..(lane + 1) * sz].copy_from_slice(&val[0..sz]);
        } else if (insn & 0xffe0_8400) == 0x6e00_0400 {
            // INS Vd.<t>[dst_lane], Vn.<t>[src_lane]
            let imm5 = (insn >> 16) & 31;
            let imm4 = (insn >> 11) & 15;
            let tz = imm5.trailing_zeros();
            let sz = 1usize << tz;
            let dl = (imm5 >> (tz + 1)) as usize;
            let sl = (imm4 >> tz) as usize;
            let elt = self.v[rn][sl * sz..(sl + 1) * sz].to_vec();
            self.v[rd][dl * sz..(dl + 1) * sz].copy_from_slice(&elt);
        } else {
            return Err(format!(
                "A64Vm: unimplemented AArch64 insn {insn:#010x} at pc={:#x}",
                self.pc
            ));
        }
        Ok(())
    }
}

fn wasm_f32_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a < b {
        a
    } else {
        b
    }
}
fn wasm_f32_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() || b.is_sign_positive() {
            0.0
        } else {
            -0.0
        }
    } else if a > b {
        a
    } else {
        b
    }
}
fn wasm_f64_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a < b {
        a
    } else {
        b
    }
}
fn wasm_f64_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() || b.is_sign_positive() {
            0.0
        } else {
            -0.0
        }
    } else if a > b {
        a
    } else {
        b
    }
}

fn verify_disasm_aarch64(label: &str, elf_bytes: &[u8]) {
    let dir = std::env::temp_dir().join(format!("wtime-a64-test-{}-{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("out.elf");
    std::fs::write(&path, elf_bytes).unwrap();
    let out = Command::new("llvm-objdump")
        .args(["--triple=aarch64", "-d", "-h", path.to_str().unwrap()])
        .output()
        .expect("llvm-objdump must be installed to verify AArch64 encodings");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "llvm-objdump failed on {label}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("elf64-littleaarch64"),
        "expected elf64-littleaarch64 header for {label}, got:\n{}",
        &stdout[..stdout.len().min(400)]
    );
    for line in stdout.lines() {
        assert!(
            !line.contains("<unknown>") && !line.contains(".word"),
            "invalid AArch64 encoding in {label}: {line}"
        );
    }
}

/// Runs `(name, arg)` through both `wtime-runtime` and the compiled AArch64 ELF64 binary
/// (via `blot_aot_new` + `blot_aot_call_export` inside `A64Vm`) and asserts bit-exact parity!
fn assert_aarch64_oracle(label: &str, wasm: &[u8], calls: &[(&str, Value)], mem_check_len: usize) {
    let img = compile_wasm_to_aarch64_for_test(
        wasm,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 0,
            deadline: None,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("aarch64 compile failed for {label}: {e:?}"));
    verify_disasm_aarch64(label, &img.elf);

    // 1. Reference `wtime-runtime` instance.
    let cfg = Config {
        features: x64_validate_features(),
        limits: x64_validate_limits(),
    };
    let engine = Engine::new(cfg).unwrap();
    let compiled = engine.compile(wasm).unwrap();
    let mut store = Store::new(&engine);
    let linker = Linker::new(&engine);
    let inst = linker.instantiate(&mut store, &compiled).unwrap();

    // 2. Emulated AArch64 instance calling real `blot_aot_new` & `blot_aot_call_export`!
    let mut vm = A64Vm::new(&img);
    let exp_map: BTreeMap<String, u64> = img.export_vaddrs.iter().cloned().collect();
    let fn_new = exp_map["blot_aot_new"];
    let fn_call = exp_map["blot_aot_call_export"];
    let fn_ver = exp_map["blot_aot_abi_version"];

    vm.run_until_ret(fn_ver, 100).unwrap();
    assert_eq!(vm.x[0], 8, "{label}: blot_aot_abi_version must return 8");

    let scratch_page = vm.alloc_seg(4096);
    let trap_ptr = scratch_page;
    let res_ptr = scratch_page + 16;
    let name_buf = scratch_page + 64;

    vm.x[0] = 0;
    vm.x[1] = 0;
    vm.x[2] = 0;
    vm.x[3] = trap_ptr;
    vm.run_until_ret(fn_new, 10_000_000).unwrap();
    let ctx_ptr = vm.x[0];
    assert_ne!(ctx_ptr, 0, "{label}: blot_aot_new returned NULL");

    for &(fname, ref arg) in calls {
        let External::Func(fhandle) = inst.export(&store, fname).unwrap() else {
            panic!("export {fname} is not a func");
        };
        let ref_res = store.invoke(fhandle, std::slice::from_ref(arg));
        let arg_bits: u64 = match *arg {
            Value::I32(v) => u64::from(v as u32),
            Value::I64(v) => v as u64,
            Value::F32(b) => u64::from(b),
            Value::F64(b) => b,
            _ => panic!("unsupported oracle arg {arg:?}"),
        };
        vm.write_bytes(name_buf, fname.as_bytes()).unwrap();
        vm.write_bytes(trap_ptr, &[0u8]).unwrap();
        vm.write_bytes(res_ptr, &0u64.to_le_bytes()).unwrap();
        vm.x[0] = ctx_ptr;
        vm.x[1] = name_buf;
        vm.x[2] = fname.len() as u64;
        vm.x[3] = arg_bits;
        vm.x[4] = res_ptr;
        vm.x[5] = trap_ptr;
        vm.run_until_ret(fn_call, 50_000_000).unwrap();
        let rc = vm.x[0] as u32;
        let a64_trap = vm.read_bytes(trap_ptr, 1).unwrap()[0];
        let a64_res = vm.read_u64(res_ptr).unwrap();
        match ref_res {
            Ok(RunOutcome::Returned(vals)) => {
                assert_eq!(
                    rc, 0,
                    "{label}/{fname}({arg:?}): expected ok, got rc={rc} trap={a64_trap}"
                );
                let expected_bits: u64 = match vals.first() {
                    None => 0,
                    Some(Value::I32(v)) => u64::from(*v as u32),
                    Some(Value::I64(v)) => *v as u64,
                    Some(Value::F32(b)) => u64::from(*b),
                    Some(Value::F64(b)) => *b,
                    other => panic!("unexpected ret {other:?}"),
                };
                assert_eq!(
                    a64_res, expected_bits,
                    "{label}/{fname}({arg:?}): AArch64={a64_res:#x} != wtime-runtime={expected_bits:#x}"
                );
            }
            Err(_) => {
                assert_eq!(
                    rc, 2,
                    "{label}/{fname}({arg:?}): expected trap rc=2, got rc={rc}"
                );
                assert_ne!(
                    a64_trap, 0,
                    "{label}/{fname}({arg:?}): trap code must be non-zero"
                );
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    if mem_check_len > 0 {
        let External::Memory(mhandle) = inst
            .export(&store, "blot:memory")
            .or_else(|_| inst.export(&store, "memory"))
            .unwrap()
        else {
            panic!("missing exported memory");
        };
        let ref_mem = store.read_memory(mhandle, 0, mem_check_len).unwrap();
        let a64_mem_base = vm.read_u64(ctx_ptr + OFF_MEM_BASE as u64).unwrap();
        let a64_slice = vm.read_bytes(a64_mem_base, mem_check_len).unwrap();
        assert_eq!(
            a64_slice,
            ref_mem.as_slice(),
            "{label}: memory content mismatch"
        );
    }
    let _ = (HELPER_INITIALIZE, OFF_MEM_MAX_PAGES, OFF_STACK_FLOOR);
}

#[test]
fn aarch64_oracle_real_kernels_and_autovec_parity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    // 1. graph_bfs.wasm
    let bfs = std::fs::read(root.join("graph_bfs.wasm")).unwrap();
    assert_aarch64_oracle(
        "graph_bfs",
        &bfs,
        &[("prepare", Value::I32(128)), ("run", Value::I32(128))],
        1024,
    );
    // 2. parser_hash.wasm
    let ph = std::fs::read(root.join("parser_hash.wasm")).unwrap();
    assert_aarch64_oracle(
        "parser_hash",
        &ph,
        &[("prepare", Value::I32(256)), ("run", Value::I32(256))],
        512,
    );
    // 3. image_filter.wasm (exercises boundary-clamp peeling + NEON uaddlp byte-sum vectorizer!)
    let im = std::fs::read(root.join("image_filter.wasm")).unwrap();
    assert_aarch64_oracle(
        "image_filter",
        &im,
        &[
            ("prepare", Value::I32(256)),
            ("run", Value::I32(256)),
            ("run", Value::I32(17)),
            ("run", Value::I32(3)),
            ("run", Value::I32(1)),
        ],
        4352,
    );
}

#[test]
fn aarch64_oracle_arith_fp_memory64_slp_traps_and_standalone_exec() {
    fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
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
    fn sec(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![id];
        leb_u32(&mut out, payload.len() as u32);
        out.extend_from_slice(payload);
        out
    }
    fn body(locals: &[(u32, u8)], code: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        leb_u32(&mut p, locals.len() as u32);
        for &(c, t) in locals {
            leb_u32(&mut p, c);
            p.push(t);
        }
        p.extend_from_slice(code);
        p.push(0x0b);
        let mut out = Vec::new();
        leb_u32(&mut out, p.len() as u32);
        out.extend_from_slice(&p);
        out
    }
    fn wasm_mod(secs: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"\x00asm\x01\x00\x00\x00".to_vec();
        for s in secs {
            out.extend_from_slice(s);
        }
        out
    }

    // Module 1: Comprehensive scalar, FP, div/rem traps, br_table, memory.copy/fill/grow, & f64x2 SLP.
    let mut types = vec![2];
    types.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // (i32) -> i32
    types.extend_from_slice(&[0x60, 1, 0x7e, 1, 0x7e]); // (i64) -> i64
    let funcs = vec![5, 0, 0, 1, 0, 0];
    let mem = vec![1, 0x01, 1, 3]; // min=1 max=3 pages
    let mut exps = vec![6];
    for (name, kind, idx) in [
        ("blot:memory", 2u8, 0u32),
        ("bits", 0, 0),
        ("div_s", 0, 1),
        ("fp_slp", 0, 2),
        ("brtab", 0, 3),
        ("memops", 0, 4),
    ] {
        leb_u32(&mut exps, name.len() as u32);
        exps.extend_from_slice(name.as_bytes());
        exps.push(kind);
        leb_u32(&mut exps, idx);
    }

    // f0(x): clz(x) + ctz(x) + popcnt(x) + rotl(x, 7) ^ rotr(x, 11)
    let b0 = vec![
        0x20, 0, 0x67, // clz
        0x20, 0, 0x68, 0x6a, // + ctz
        0x20, 0, 0x69, 0x6a, // + popcnt
        0x20, 0, 0x41, 7, 0x77, 0x6a, // + rotl(x, 7)
        0x20, 0, 0x41, 11, 0x78, 0x73, // ^ rotr(x, 11)
    ];
    // f1(d): i32.div_s(-2147483648, d) -> traps on d=0 and d=-1!
    let b1 = vec![0x41, 0x80, 0x80, 0x80, 0x80, 0x78, 0x20, 0, 0x6d];
    // f2(n: i64) -> i64: writes 8 f64 pairs and runs a counted f64x2 SLP loop exercising
    // fadd, fsub, fmul, fmin, fmax, and accumulator lanes on NEON!
    let mut b2 = Vec::new();
    for k in 0..8u32 {
        let base = k * 32;
        for (slot, val) in [
            (0u32, (k as f64) * 1.5 - 4.0),
            (8, (k as f64) * -0.75 + 2.5),
            (16, 0.5 + (k as f64) * 0.25),
            (24, -1.25 + (k as f64) * 0.125),
        ] {
            b2.push(0x41);
            leb_u32(&mut b2, base + slot);
            b2.push(0x44);
            b2.extend_from_slice(&val.to_le_bytes());
            b2.extend_from_slice(&[0x39, 3, 0]);
        }
    }
    // locals: 1:i32(i), 2:f64(acc0), 3:f64(acc1)
    // loop i = 0..8:
    //   off = i << 5
    //   nx = f64.min(f64.max((x + vx * 0.5) - 0.25, -2.0), 3.0)
    //   ny = f64.min(f64.max((y + vy * 0.5) - 0.25, -2.0), 3.0)
    //   store nx -> off+0, ny -> off+8; acc0 += nx; acc1 += ny
    fn push_f64(out: &mut Vec<u8>, v: f64) {
        out.push(0x44);
        out.extend_from_slice(&v.to_le_bytes());
    }
    b2.extend_from_slice(&[0x03, 0x40]); // loop
    // store nx at off+0
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74]); // off
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 0]); // x
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 16]); // vx
    push_f64(&mut b2, 0.5);
    b2.extend_from_slice(&[0xa2, 0xa0]); // x + vx*0.5
    push_f64(&mut b2, 0.25);
    b2.push(0xa1); // - 0.25
    push_f64(&mut b2, -2.0);
    b2.push(0xa5); // f64.max(..., -2.0)
    push_f64(&mut b2, 3.0);
    b2.push(0xa4); // f64.min(..., 3.0)
    b2.extend_from_slice(&[0x39, 3, 0]);
    // store ny at off+8
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74]); // off
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 8]); // y
    b2.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 24]); // vy
    push_f64(&mut b2, 0.5);
    b2.extend_from_slice(&[0xa2, 0xa0]);
    push_f64(&mut b2, 0.25);
    b2.push(0xa1);
    push_f64(&mut b2, -2.0);
    b2.push(0xa5);
    push_f64(&mut b2, 3.0);
    b2.push(0xa4);
    b2.extend_from_slice(&[0x39, 3, 8]);
    // acc0 += mem[off+0]; acc1 += mem[off+8]
    b2.extend_from_slice(&[0x20, 2, 0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 0, 0xa0, 0x21, 2]);
    b2.extend_from_slice(&[0x20, 3, 0x20, 1, 0x41, 5, 0x74, 0x2b, 3, 8, 0xa0, 0x21, 3]);
    // i += 1; br_if 0 (i < 8)
    b2.extend_from_slice(&[
        0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x41, 8, 0x49, 0x0d, 0, 0x0b,
    ]);
    // return reinterpret((acc0 + acc1 + f64(n)).nearest())
    b2.extend_from_slice(&[0x20, 2, 0x20, 3, 0xa0, 0x20, 0, 0xb9, 0xa0, 0x9c, 0xbd]);
    // f3(i): br_table over 4 arms
    let b3 = vec![
        0x02, 0x7f, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x20, 0, 0x0e, 3, 0, 1, 2,
        2, // br_table [0,1,2] default=2
        0x0b, 0x41, 10, 0x0c, 2, 0x0b, 0x41, 20, 0x0c, 1, 0x0b, 0x41, 30, 0x0b,
    ];
    // f4(delta): memory.fill + memory.copy + memory.grow(delta) + load
    let b4 = vec![
        0x41, 64, 0x41, 0x3c, 0x41, 32, 0xfc, 11, 0, // memory.fill(64, 0x3c, 32)
        0x41, 80, 0x41, 64, 0x41, 32, 0xfc, 10, 0, 0, // memory.copy(80, 64, 32)
        0x20, 0, 0x40, 0, // memory.grow(delta)
        0x41, 96, 0x2d, 0, 0, 0x6a, // + load8_u(96)
    ];

    let mut code = vec![5];
    code.extend_from_slice(&body(&[], &b0));
    code.extend_from_slice(&body(&[], &b1));
    code.extend_from_slice(&body(&[(1, 0x7f), (2, 0x7c)], &b2));
    code.extend_from_slice(&body(&[], &b3));
    code.extend_from_slice(&body(&[], &b4));

    let wasm = wasm_mod(&[
        sec(1, &types),
        sec(3, &funcs),
        sec(5, &mem),
        sec(7, &exps),
        sec(10, &code),
    ]);
    assert_aarch64_oracle(
        "a64_core",
        &wasm,
        &[
            ("bits", Value::I32(0)),
            ("bits", Value::I32(0x1234_5678)),
            ("bits", Value::I32(-1)),
            ("div_s", Value::I32(2)),
            ("div_s", Value::I32(0)),  // TRAP_DIV_ZERO
            ("div_s", Value::I32(-1)), // TRAP_INT_OVERFLOW
            ("fp_slp", Value::I64(7)),
            ("fp_slp", Value::I64(-42)),
            ("brtab", Value::I32(0)),
            ("brtab", Value::I32(1)),
            ("brtab", Value::I32(2)),
            ("brtab", Value::I32(99)),
            ("memops", Value::I32(1)),
            ("memops", Value::I32(5)), // exceeds max -> grow returns -1
        ],
        128,
    );

    // Module 2: Standalone static Linux AArch64 `ET_EXEC` with `memory64` + `_start`!
    let t2 = vec![1, 0x60, 0, 0];
    let f2 = vec![1, 0];
    let m2 = vec![1, 0x05, 1, 2]; // memory64 min=1 max=2
    let mut e2 = vec![1];
    e2.push(6);
    e2.extend_from_slice(b"_start");
    e2.push(0);
    e2.push(0);
    let mut c2 = vec![1];
    // memory64 store + load in `_start`
    c2.extend_from_slice(&body(
        &[],
        &[0x42, 16, 0x42, 42, 0x37, 3, 0, 0x42, 16, 0x29, 3, 0, 0x1a],
    ));
    let wasm64 = wasm_mod(&[
        sec(1, &t2),
        sec(3, &f2),
        sec(5, &m2),
        sec(7, &e2),
        sec(10, &c2),
    ]);
    let exec_img = compile_wasm_to_aarch64_for_test(
        &wasm64,
        &A64CompileOptions {
            target: X64Target::StandaloneExec,
            optimize: true,
            jobs: 0,
            deadline: None,
            ..Default::default()
        },
    )
    .expect("aarch64 standalone ET_EXEC compile failed");
    // Verify ELF header (`ET_EXEC = 2`, `EM_AARCH64 = 183`) and valid disassembly via llvm-objdump!
    assert_eq!(
        u16::from_le_bytes(exec_img.elf[16..18].try_into().unwrap()),
        2
    );
    assert_eq!(
        u16::from_le_bytes(exec_img.elf[18..20].try_into().unwrap()),
        183
    );
    verify_disasm_aarch64("a64_standalone_mem64", &exec_img.elf);
    // Execute `_start` from ELF `e_entry` inside our pure-Rust AArch64 VM and assert clean exit(0)!
    let entry_va = u64::from_le_bytes(exec_img.elf[24..32].try_into().unwrap());
    let mut vm = A64Vm::new(&exec_img);
    vm.run_until_ret(entry_va, 1_000_000).unwrap();
    assert_eq!(
        vm.exited,
        Some(0),
        "standalone AArch64 ET_EXEC must exit(0)"
    );
}

#[test]
fn aarch64_oracle_all_236_simd_opcodes_madd_msub_and_parallel_determinism() {
    use crate::x64::tests_unit::{
        code_sec, enc_body, export_sec, func_sec, leb_i32, leb_u32, section, type_sec, wasm_module,
    };

    fn push_simd_op(out: &mut Vec<u8>, op: u16) {
        assert_eq!(op >> 8, 0xfd);
        out.push(0xfd);
        leb_u32(out, u32::from(op & 0xff));
    }
    fn push_v128_const(out: &mut Vec<u8>, bytes: [u8; 16]) {
        out.push(0xfd);
        leb_u32(out, 12);
        out.extend_from_slice(&bytes);
    }

    let covered = std::cell::RefCell::new(std::collections::BTreeSet::<u16>::new());
    let emit_op = |out: &mut Vec<u8>, op: u16| {
        covered.borrow_mut().insert(op);
        push_simd_op(out, op);
    };

    // Rich edge-case seed identical to x64 `oracle_hill1_all_236_simd_opcodes`.
    let mut seed = [0u8; 256];
    seed[0..16].copy_from_slice(&[
        0x00, 0x01, 0x7f, 0x80, 0x81, 0xff, 0x10, 0x20, 0xfe, 0x02, 0x55, 0xaa, 0x0f, 0xf0, 0x33,
        0xcc,
    ]);
    seed[0..2].copy_from_slice(&(-32768i16).to_le_bytes());
    seed[16..32].copy_from_slice(&[
        0x00, 0x80, 0x7f, 0x80, 0xff, 0x01, 0x00, 0x7f, 0x03, 0x11, 0x10, 0x20, 0xff, 0x00, 0x07,
        0x08,
    ]);
    for (i, v) in [
        (-0.0f32).to_bits(),
        3.75f32.to_bits(),
        0x7fc0_1234u32,
        (-2147483904.0f32).to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[32 + i * 4..36 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in [
        0.0f32.to_bits(),
        (-3.75f32).to_bits(),
        2147483648.0f32.to_bits(),
        4294967296.0f32.to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[48 + i * 4..52 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in [
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        3000000000.0f32.to_bits(),
        (-0.5f32).to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        seed[64 + i * 4..68 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    seed[80..88].copy_from_slice(&(-0.0f64).to_bits().to_le_bytes());
    seed[88..96].copy_from_slice(&3.75f64.to_bits().to_le_bytes());
    seed[96..104].copy_from_slice(&0.0f64.to_bits().to_le_bytes());
    seed[104..112].copy_from_slice(&0x7ff8_0000_0000_0042u64.to_le_bytes());
    seed[112..120].copy_from_slice(&(-3_000_000_000.0f64).to_bits().to_le_bytes());
    seed[120..128].copy_from_slice(&5_000_000_000.0f64.to_bits().to_le_bytes());
    seed[128..136].copy_from_slice(&f64::NEG_INFINITY.to_bits().to_le_bytes());
    seed[136..144].copy_from_slice(&3_000_000_000.5f64.to_bits().to_le_bytes());
    seed[144..152].copy_from_slice(&i64::MIN.to_le_bytes());
    seed[152..160].copy_from_slice(&i64::MAX.to_le_bytes());
    seed[160..168].copy_from_slice(&(-1i64).to_le_bytes());
    seed[168..176].copy_from_slice(&0x0123_4567_89ab_cdefi64.to_le_bytes());

    let mut out_off: i32 = 512;
    let store_v128 =
        |b: &mut Vec<u8>,
         cursor: &mut i32,
         cov: &std::cell::RefCell<std::collections::BTreeSet<u16>>| {
            b.extend_from_slice(&[0x21, 1]);
            b.push(0x41);
            leb_i32(b, *cursor);
            b.extend_from_slice(&[0x20, 1]);
            cov.borrow_mut().insert(0xfd0b);
            push_simd_op(b, 0xfd0b);
            b.extend_from_slice(&[0, 0]);
            *cursor += 16;
        };
    let load_at =
        |b: &mut Vec<u8>, off: i32, cov: &std::cell::RefCell<std::collections::BTreeSet<u16>>| {
            b.push(0x41);
            leb_i32(b, off);
            cov.borrow_mut().insert(0xfd00);
            push_simd_op(b, 0xfd00);
            b.extend_from_slice(&[0, 0]);
        };

    // f0: loads, lane loads/stores, shuffle, swizzle, splats, extract/replace, bitselect, to_i32
    let mut b0 = Vec::new();
    for op in [
        0xfd00, 0xfd01, 0xfd02, 0xfd03, 0xfd04, 0xfd05, 0xfd06, 0xfd07, 0xfd08, 0xfd09, 0xfd0a,
        0xfd5c, 0xfd5d,
    ] {
        b0.extend_from_slice(&[0x41, 0]);
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0]);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    covered.borrow_mut().insert(0xfd0c);
    push_v128_const(
        &mut b0,
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    );
    store_v128(&mut b0, &mut out_off, &covered);
    for lanes in [
        [0, 16, 1, 17, 2, 18, 15, 31, 30, 14, 7, 23, 8, 24, 0, 31],
        [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
        [
            16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    ] {
        load_at(&mut b0, 0, &covered);
        load_at(&mut b0, 16, &covered);
        emit_op(&mut b0, 0xfd0d);
        b0.extend_from_slice(&lanes);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    load_at(&mut b0, 0, &covered);
    load_at(&mut b0, 16, &covered);
    emit_op(&mut b0, 0xfd0e);
    store_v128(&mut b0, &mut out_off, &covered);
    for (op, push_arg) in [
        (0xfd0fu16, vec![0x20, 0]),
        (0xfd10, vec![0x20, 0]),
        (0xfd11, vec![0x20, 0]),
        (0xfd12, vec![0x20, 0, 0xac]),
        (0xfd13, vec![0x20, 0, 0xb2]),
        (0xfd14, vec![0x20, 0, 0xb7]),
    ] {
        b0.extend_from_slice(&push_arg);
        emit_op(&mut b0, op);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    let mix_i32 = |b: &mut Vec<u8>| {
        b.extend_from_slice(&[0x20, 2, 0x41, 31, 0x6c, 0x73, 0x21, 2]);
    };
    for (op, lane, conv) in [
        (0xfd15u16, 3u8, vec![]),
        (0xfd16, 3, vec![]),
        (0xfd18, 1, vec![]),
        (0xfd19, 1, vec![]),
        (0xfd1b, 2, vec![]),
        (0xfd1d, 1, vec![0xa7]),
        (0xfd1f, 1, vec![0xbc]),
        (0xfd21, 1, vec![0xbd, 0xa7]),
    ] {
        load_at(&mut b0, if op >= 0xfd1f { 32 } else { 0 }, &covered);
        emit_op(&mut b0, op);
        b0.push(lane);
        b0.extend_from_slice(&conv);
        mix_i32(&mut b0);
    }
    for (op, lane, val_ops) in [
        (0xfd17u16, 5u8, vec![0x20, 0]),
        (0xfd1a, 3, vec![0x20, 0]),
        (0xfd1c, 2, vec![0x20, 0]),
        (0xfd1e, 1, vec![0x20, 0, 0xac]),
        (0xfd20, 2, vec![0x20, 0, 0xb2]),
        (0xfd22, 1, vec![0x20, 0, 0xb7]),
    ] {
        load_at(&mut b0, 0, &covered);
        b0.extend_from_slice(&val_ops);
        emit_op(&mut b0, op);
        b0.push(lane);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    for (op, lane) in [(0xfd54u16, 7u8), (0xfd55, 5), (0xfd56, 3), (0xfd57, 1)] {
        b0.extend_from_slice(&[0x41, 4]);
        load_at(&mut b0, 0, &covered);
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0, lane]);
        store_v128(&mut b0, &mut out_off, &covered);
    }
    for (op, lane) in [(0xfd58u16, 7u8), (0xfd59, 5), (0xfd5a, 3), (0xfd5b, 1)] {
        b0.push(0x41);
        leb_i32(&mut b0, out_off);
        load_at(&mut b0, 0, &covered);
        emit_op(&mut b0, op);
        b0.extend_from_slice(&[0, 0, lane]);
        out_off += 16;
    }
    for op in [
        0xfd53u16, 0xfd63, 0xfd83, 0xfda3, 0xfdc3, 0xfd64, 0xfd84, 0xfda4, 0xfdc4,
    ] {
        for off in [0, 16, 144] {
            load_at(&mut b0, off, &covered);
            emit_op(&mut b0, op);
            mix_i32(&mut b0);
        }
        push_v128_const(&mut b0, [0xff; 16]);
        emit_op(&mut b0, op);
        mix_i32(&mut b0);
    }
    load_at(&mut b0, 0, &covered);
    load_at(&mut b0, 16, &covered);
    load_at(&mut b0, 144, &covered);
    emit_op(&mut b0, 0xfd52);
    store_v128(&mut b0, &mut out_off, &covered);
    b0.extend_from_slice(&[0x20, 2]);

    // f1: ALL unary V128 ops + ALL 12 shifts
    let mut b1 = Vec::new();
    for &(op, offs) in &[
        (0xfd4du16, &[0i32][..]),
        (0xfd5e, &[80, 96, 112]),
        (0xfd5f, &[32, 48, 64]),
        (0xfd60, &[0, 16]),
        (0xfd61, &[0, 16]),
        (0xfd62, &[0, 16]),
        (0xfd67, &[32, 48, 64]),
        (0xfd68, &[32, 48, 64]),
        (0xfd69, &[32, 48, 64]),
        (0xfd6a, &[32, 48, 64]),
        (0xfd74, &[80, 96, 112, 128]),
        (0xfd75, &[80, 96, 112, 128]),
        (0xfd7a, &[80, 96, 112, 128]),
        (0xfd94, &[80, 96, 112, 128]),
        (0xfd7c, &[0, 16]),
        (0xfd7d, &[0, 16]),
        (0xfd7e, &[0, 16]),
        (0xfd7f, &[0, 16]),
        (0xfd80, &[0, 16]),
        (0xfd81, &[0, 16]),
        (0xfd87, &[0]),
        (0xfd88, &[0]),
        (0xfd89, &[0]),
        (0xfd8a, &[0]),
        (0xfda0, &[0, 144]),
        (0xfda1, &[0, 144]),
        (0xfda7, &[0]),
        (0xfda8, &[0]),
        (0xfda9, &[0]),
        (0xfdaa, &[0]),
        (0xfdc0, &[144, 160]),
        (0xfdc1, &[144, 160]),
        (0xfdc7, &[0, 144]),
        (0xfdc8, &[0, 144]),
        (0xfdc9, &[0, 144]),
        (0xfdca, &[0, 144]),
        (0xfde0, &[32, 48, 64]),
        (0xfde1, &[32, 48, 64]),
        (0xfde3, &[32, 48, 64]),
        (0xfdec, &[80, 96, 112]),
        (0xfded, &[80, 96, 112]),
        (0xfdef, &[80, 96, 112]),
        (0xfdf8, &[32, 48, 64]),
        (0xfdf9, &[32, 48, 64]),
        (0xfdfa, &[0, 144]),
        (0xfdfb, &[0, 144]),
        (0xfdfc, &[80, 96, 112, 128]),
        (0xfdfd, &[80, 96, 112, 128]),
        (0xfdfe, &[0, 144]),
        (0xfdff, &[0, 144]),
    ] {
        for &off in offs {
            load_at(&mut b1, off, &covered);
            emit_op(&mut b1, op);
            store_v128(&mut b1, &mut out_off, &covered);
        }
    }
    for op in [
        0xfd6bu16, 0xfd6c, 0xfd6d, 0xfd8b, 0xfd8c, 0xfd8d, 0xfdab, 0xfdac, 0xfdad, 0xfdcb, 0xfdcc,
        0xfdcd,
    ] {
        for off in [0, 144] {
            load_at(&mut b1, off, &covered);
            b1.extend_from_slice(&[0x20, 0]);
            emit_op(&mut b1, op);
            store_v128(&mut b1, &mut out_off, &covered);
        }
    }
    b1.push(0x41);
    leb_i32(&mut b1, out_off);

    // f2: ALL binary V128 ops
    let mut b2 = Vec::new();
    for &op in &[
        0xfd23u16, 0xfd24, 0xfd25, 0xfd26, 0xfd27, 0xfd28, 0xfd29, 0xfd2a, 0xfd2b, 0xfd2c, 0xfd2d,
        0xfd2e, 0xfd2f, 0xfd30, 0xfd31, 0xfd32, 0xfd33, 0xfd34, 0xfd35, 0xfd36, 0xfd37, 0xfd38,
        0xfd39, 0xfd3a, 0xfd3b, 0xfd3c, 0xfd3d, 0xfd3e, 0xfd3f, 0xfd40, 0xfd4e, 0xfd4f, 0xfd50,
        0xfd51, 0xfd65, 0xfd66, 0xfd85, 0xfd86, 0xfd6e, 0xfd6f, 0xfd70, 0xfd71, 0xfd72, 0xfd73,
        0xfd76, 0xfd77, 0xfd78, 0xfd79, 0xfd7b, 0xfd82, 0xfd8e, 0xfd8f, 0xfd90, 0xfd91, 0xfd92,
        0xfd93, 0xfd95, 0xfd96, 0xfd97, 0xfd98, 0xfd99, 0xfd9b, 0xfd9c, 0xfd9d, 0xfd9e, 0xfd9f,
        0xfdae, 0xfdb1, 0xfdb5, 0xfdb6, 0xfdb7, 0xfdb8, 0xfdb9, 0xfdba, 0xfdbc, 0xfdbd, 0xfdbe,
        0xfdbf, 0xfdce, 0xfdd1, 0xfdd5, 0xfdd6, 0xfdd7, 0xfdd8, 0xfdd9, 0xfdda, 0xfddb, 0xfddc,
        0xfddd, 0xfdde, 0xfddf,
    ] {
        let (a_off, b_off) = if op >= 0xfdce { (144, 160) } else { (0, 16) };
        load_at(&mut b2, a_off, &covered);
        load_at(&mut b2, b_off, &covered);
        emit_op(&mut b2, op);
        store_v128(&mut b2, &mut out_off, &covered);
    }
    for op in [
        0xfd41u16, 0xfd42, 0xfd43, 0xfd44, 0xfd45, 0xfd46, 0xfde4, 0xfde5, 0xfde6, 0xfde7, 0xfde8,
        0xfde9, 0xfdea, 0xfdeb,
    ] {
        for (a_off, b_off) in [(32, 48), (48, 32), (32, 64)] {
            load_at(&mut b2, a_off, &covered);
            load_at(&mut b2, b_off, &covered);
            emit_op(&mut b2, op);
            store_v128(&mut b2, &mut out_off, &covered);
        }
    }
    for op in [
        0xfd47u16, 0xfd48, 0xfd49, 0xfd4a, 0xfd4b, 0xfd4c, 0xfdf0, 0xfdf1, 0xfdf2, 0xfdf3, 0xfdf4,
        0xfdf5, 0xfdf6, 0xfdf7,
    ] {
        for (a_off, b_off) in [(80, 96), (96, 80), (112, 128)] {
            load_at(&mut b2, a_off, &covered);
            load_at(&mut b2, b_off, &covered);
            emit_op(&mut b2, op);
            store_v128(&mut b2, &mut out_off, &covered);
        }
    }
    b2.push(0x41);
    leb_i32(&mut b2, out_off);

    assert_eq!(covered.borrow().len(), 236);
    assert!(out_off < 65_536);

    // f3: exercises integer `madd`/`msub` fusion + wide overlapping `memory.copy` (1..=64 B) & `memory.fill`!
    // madd: (x * y) + z; msub: z - (x * y)
    let mut b3 = vec![
        // x = arg, y = arg + 7, z = arg ^ 0x55
        0x20, 0, 0x41, 7, 0x6a, 0x21, 1, // local 1 = y
        0x20, 0, 0x41, 0x55, 0x73, 0x21, 2, // local 2 = z
        // madd: (x * y) + z
        0x20, 0, 0x20, 1, 0x6c, 0x20, 2, 0x6a, // msub: z - (y * (x ^ 3))
        0x20, 2, 0x20, 1, 0x20, 0, 0x41, 3, 0x73, 0x6c, 0x6b, // combine
        0x73,
    ];
    // Wide overlapping memory.copy (sizes 17, 32, 49, 64, fwd & bwd) + memory.fill(64)
    for &(len, bwd_s, fwd_s) in &[(17i32, 5i32, 3i32), (32, 8, 7), (49, 13, 11), (64, 15, 19)] {
        b3.push(0x41);
        leb_i32(&mut b3, 256 + bwd_s);
        b3.push(0x41);
        leb_i32(&mut b3, 256);
        b3.push(0x41);
        leb_i32(&mut b3, len);
        b3.extend_from_slice(&[0xfc, 10, 0, 0]);
        b3.push(0x41);
        leb_i32(&mut b3, 256 - fwd_s);
        b3.push(0x41);
        leb_i32(&mut b3, 256);
        b3.push(0x41);
        leb_i32(&mut b3, len);
        b3.extend_from_slice(&[0xfc, 10, 0, 0]);
    }
    b3.push(0x41);
    leb_i32(&mut b3, 384);
    b3.extend_from_slice(&[0x20, 0, 0x41]);
    leb_i32(&mut b3, 64);
    b3.extend_from_slice(&[0xfc, 11, 0]);

    let mut data_sec = vec![1, 0x00, 0x41, 0, 0x0b];
    leb_u32(&mut data_sec, seed.len() as u32);
    data_sec.extend_from_slice(&seed);

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0, 0, 0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("simd_misc", 0, 0),
            ("simd_un_shift", 0, 1),
            ("simd_bin", 0, 2),
            ("madd_copy", 0, 3),
        ]),
        code_sec(&[
            enc_body(&[(1, 0x7b), (1, 0x7f)], &b0),
            enc_body(&[(1, 0x7b)], &b1),
            enc_body(&[(1, 0x7b)], &b2),
            enc_body(&[(2, 0x7f)], &b3),
        ]),
        section(11, &data_sec),
    ]);

    assert_aarch64_oracle(
        "a64_simd_236",
        &wasm,
        &[
            ("simd_misc", Value::I32(-42)),
            ("simd_bin", Value::I32(0)),
            ("simd_un_shift", Value::I32(0)),
            ("simd_un_shift", Value::I32(1)),
            ("simd_un_shift", Value::I32(7)),
            ("simd_un_shift", Value::I32(15)),
            ("simd_un_shift", Value::I32(63)),
            ("simd_un_shift", Value::I32(-1)),
            ("madd_copy", Value::I32(12345)),
            ("madd_copy", Value::I32(-77)),
        ],
        out_off as usize,
    );

    // Parallel compilation determinism: `--jobs 1` vs `--jobs 8` MUST emit 100% byte-identical ELF!
    for (name, bytes) in [
        ("simd236", wasm.as_slice()),
        (
            "image_filter",
            &std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/image_filter.wasm"),
            )
            .unwrap(),
        ),
        (
            "graph_bfs",
            &std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/graph_bfs.wasm"),
            )
            .unwrap(),
        ),
    ] {
        let j1 = compile_wasm_to_aarch64_for_test(
            bytes,
            &A64CompileOptions {
                target: X64Target::BlotCdylib,
                optimize: true,
                jobs: 1,
                deadline: None,
                ..Default::default()
            },
        )
        .unwrap();
        let j8 = compile_wasm_to_aarch64_for_test(
            bytes,
            &A64CompileOptions {
                target: X64Target::BlotCdylib,
                optimize: true,
                jobs: 8,
                deadline: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            j1.elf, j8.elf,
            "aarch64 --jobs 1 vs --jobs 8 ELF must be byte-identical for {name}"
        );
    }
}

#[test]
fn aarch64_oracle_step3_blur3_magic_hoist_and_f64x4() {
    use crate::x64::tests_unit::{
        code_sec, enc_body, export_sec, func_sec, leb_i32, leb_u32, section, type_sec, wasm_module,
    };
    // Seed 0..128 with [255, 255, 255, ...] and mixed bytes.
    let mut seed = vec![0u8; 128];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = if i < 6 {
            255
        } else {
            ((i.wrapping_mul(37).wrapping_add(11)) & 0xff) as u8
        };
    }
    let mut f64_seg = Vec::new();
    for v in [1.5f64, -0.0, 7.25, -4.5, 2.25, 0.0, -3.5, 4.0] {
        f64_seg.extend_from_slice(&v.to_le_bytes());
    }
    let mut data_sec = vec![2, 0, 0x41, 0, 0x0b];
    leb_u32(&mut data_sec, seed.len() as u32);
    data_sec.extend_from_slice(&seed);
    data_sec.extend_from_slice(&[0, 0x41]);
    leb_i32(&mut data_sec, 1024);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, f64_seg.len() as u32);
    data_sec.extend_from_slice(&f64_seg);

    // f0(n): 3-tap blur / 3 -> dst 512 (triggers Op::MemByteBlur3Div on AArch64 lowering!).
    let mut b0 = vec![
        0x41, 0, 0x21, 1, 0x02, 0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, 0x20, 1, 0x41,
        1, 0x6b, 0x41, 0, 0x20, 1, 0x1b, 0x21, 2, 0x20, 0, 0x41, 1, 0x6b, 0x20, 1, 0x41, 1, 0x6a,
        0x20, 1, 0x41, 1, 0x6a, 0x20, 0, 0x4f, 0x1b, 0x21, 3, 0x20, 1, 0x20, 2, 0x2d, 0, 0, 0x20,
        1, 0x2d, 0, 0, 0x6a, 0x20, 3, 0x2d, 0, 0, 0x6a, 0x41, 3, 0x6e, 0x3a, 0, 0x80, 0x04, 0x20,
        1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x41, 0, 0x21, 1, 0x41, 0, 0x21, 2, 0x02,
        0x40, 0x03, 0x40, 0x20, 1, 0x20, 0, 0x4f, 0x0d, 1, 0x20, 2, 0x41, 31, 0x6c, 0x20, 1, 0x2d,
        0, 0x80, 0x04, 0x6a, 0x21, 2, 0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 0, 0x0b, 0x0b, 0x20,
        2,
    ];
    let _ = &mut b0;

    // f1(mode): F64x4BinMem (mem rhs + splat rhs) returning low 32 bits of XOR of stored f64 bits.
    let mut b1 = Vec::new();
    b1.push(0x41);
    leb_i32(&mut b1, 1088);
    b1.extend_from_slice(&[0x21, 1, 0x41]);
    leb_i32(&mut b1, 1024);
    b1.extend_from_slice(&[0x21, 11, 0x41]);
    leb_i32(&mut b1, 1056);
    b1.extend_from_slice(&[0x21, 12]);
    for k in 0..4usize {
        b1.extend_from_slice(&[0x20, 11, 0x2b, 3, (k * 8) as u8, 0x21, (2 + k) as u8]);
    }
    for k in 0..4usize {
        b1.extend_from_slice(&[0x20, 12, 0x2b, 3, (k * 8) as u8, 0x21, (6 + k) as u8]);
    }
    for k in 0..4usize {
        b1.extend_from_slice(&[
            0x20,
            1,
            0x20,
            (2 + k) as u8,
            0x20,
            (6 + k) as u8,
            0xa0,
            0x39,
            3,
            (k * 8) as u8,
        ]);
    }
    // Scale 4 lanes in-place by f64::from(mode + 2) (splat rhs!)
    b1.extend_from_slice(&[0x20, 0, 0x41, 2, 0x6a, 0xb7, 0x21, 10]); // local 10 = f64(mode + 2)
    for k in 0..4usize {
        b1.extend_from_slice(&[
            0x20,
            1,
            0x20,
            1,
            0x2b,
            3,
            (k * 8) as u8,
            0x20,
            10,
            0xa2,
            0x39,
            3,
            (k * 8) as u8,
        ]);
    }
    b1.extend_from_slice(&[0x20, 1, 0x28, 2, 0]);
    for k in 1..8usize {
        b1.extend_from_slice(&[0x20, 1, 0x28, 2, (k * 4) as u8, 0x73]);
    }

    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[("blot:memory", 2, 0), ("blur3", 0, 0), ("f64x4", 0, 1)]),
        code_sec(&[
            enc_body(&[(3, 0x7f)], &b0),
            enc_body(&[(1, 0x7f), (9, 0x7c), (2, 0x7f)], &b1),
        ]),
        section(11, &data_sec),
    ]);

    assert_aarch64_oracle(
        "a64_step3_blur3_f64x4",
        &wasm,
        &[
            ("blur3", Value::I32(0)),
            ("blur3", Value::I32(3)),
            ("blur3", Value::I32(18)),
            ("blur3", Value::I32(67)),
            ("f64x4", Value::I32(1)),
            ("f64x4", Value::I32(-5)),
        ],
        1120,
    );
}

#[test]
fn aarch64_incremental_cache_1fn_edit_byte_exact() {
    use crate::x64::tests_unit::{
        code_sec, enc_body, export_sec, func_sec, leb_i32, section, type_sec, wasm_module,
    };

    // Build an 8-function module with a small inlinable helper, a br_table jump table,
    // a global effect, and independent sibling functions so a 1-function edit exercises
    // partial reuse, inliner dirty propagation, jump-table caching, and byte-exact linking!
    let make_mod = |edited_const: i32, helper_const: i32| -> Vec<u8> {
        let mut bodies: Vec<Vec<u8>> = Vec::new();
        // f0(x): small helper x + helper_const (inlined into f1)
        let mut b0 = vec![0x20, 0, 0x41];
        leb_i32(&mut b0, helper_const);
        b0.push(0x6a);
        bodies.push(enc_body(&[], &b0));
        // f1(x): calls f0(x) * 3
        bodies.push(enc_body(&[], &[0x20, 0, 0x10, 0, 0x41, 3, 0x6c]));
        // f2(x): br_table switch over 0..3 -> exercises cached jump tables!
        bodies.push(enc_body(
            &[],
            &[
                0x02, 0x7f, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x20, 0, 0x0e, 3, 0, 1, 2, 2, 0x0b,
                0x41, 11, 0x0c, 2, 0x0b, 0x41, 22, 0x0c, 1, 0x0b, 0x41, 33, 0x0b,
            ],
        ));
        // f3..f6: untouched functions
        for k in 0..4i32 {
            let mut bk = vec![0x20, 0, 0x41];
            leb_i32(&mut bk, 100 + k * 7);
            bk.extend_from_slice(&[0x6a, 0x41, 5, 0x6c]);
            bodies.push(enc_body(&[], &bk));
        }
        // f7(x): edited target leaf function
        let mut b7 = vec![0x20, 0, 0x41];
        leb_i32(&mut b7, edited_const);
        b7.push(0x73); // xor
        bodies.push(enc_body(&[], &b7));

        wasm_module(&[
            type_sec(&[(&[0x7f], &[0x7f])]),
            func_sec(&[0; 8]),
            section(5, &[1, 0x01, 1, 2]),
            export_sec(&[
                ("blot:memory", 2, 0),
                ("f1", 0, 1),
                ("f2", 0, 2),
                ("f7", 0, 7),
            ]),
            code_sec(&bodies),
        ])
    };

    let root = crate::build::create_fresh_test_root("wtime-aot-a64-inc");
    let inc_dir = root.0.join("a64-inc");
    let gen_fp = [0x42u8; 32];

    let w1 = make_mod(17, 5);
    let w2 = make_mod(99, 5); // 1-fn leaf edit (f7)
    let w3 = make_mod(99, 13); // 1-fn inlined-callee edit (f0 -> must recompile f0 + caller f1!)

    // 1. Cold build populates `a64-inc` + `.vidx`.
    let cold_w1_nocache = compile_wasm_to_aarch64_for_test(
        &w1,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let cold_w1_cache = compile_wasm_to_aarch64_for_test(
        &w1,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            inc_cache_dir: Some(inc_dir.clone()),
            gen_fingerprint: gen_fp,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(cold_w1_nocache.elf, cold_w1_cache.elf);
    assert_eq!(cold_w1_cache.inc_reused_funcs, 0);

    // 2. Warm 1-fn leaf edit (f7): MUST reuse 7/8 functions AND be 100% byte-identical to cold w2!
    let cold_w2 = compile_wasm_to_aarch64_for_test(
        &w2,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let warm_w2 = compile_wasm_to_aarch64_for_test(
        &w2,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            inc_cache_dir: Some(inc_dir.clone()),
            gen_fingerprint: gen_fp,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        warm_w2.inc_reused_funcs, 7,
        "1-fn leaf edit must reuse 7/8 functions"
    );
    assert_eq!(
        warm_w2.elf, cold_w2.elf,
        "a64-inc 1-fn leaf edit must emit 100% byte-identical ELF to cold compile"
    );

    // 3. Warm 1-fn inlined-callee edit (f0 -> invalidates f0 & caller f1, reuses 6/8)!
    let cold_w3 = compile_wasm_to_aarch64_for_test(
        &w3,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let warm_w3 = compile_wasm_to_aarch64_for_test(
        &w3,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 4,
            inc_cache_dir: Some(inc_dir.clone()),
            gen_fingerprint: gen_fp,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        warm_w3.inc_reused_funcs, 6,
        "inlined helper edit must invalidate f0 + caller f1 and reuse 6/8"
    );
    assert_eq!(
        warm_w3.elf, cold_w3.elf,
        "a64-inc inlined-callee edit must emit 100% byte-identical ELF to cold compile"
    );

    // 4. Corruption fallback: flip a byte in `a64-inc/*.bin`, verify clean cold fallback & exact ELF!
    for entry in std::fs::read_dir(&inc_dir).unwrap().flatten() {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("bin") {
            let mut bytes = std::fs::read(entry.path()).unwrap();
            bytes[20] ^= 0xff;
            std::fs::write(entry.path(), bytes).unwrap();
        }
    }
    let after_corrupt = compile_wasm_to_aarch64_for_test(
        &w3,
        &A64CompileOptions {
            target: X64Target::BlotCdylib,
            optimize: true,
            jobs: 2,
            inc_cache_dir: Some(inc_dir),
            gen_fingerprint: gen_fp,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(after_corrupt.inc_reused_funcs, 0);
    assert_eq!(after_corrupt.elf, cold_w3.elf);
    let _ = std::fs::remove_dir_all(&root.0);
}

#[test]
fn aarch64_standalone_guard_page_bce_and_goose_ryu_wasi_parity() {
    use crate::x64::tests_unit::{
        code_sec, enc_body, export_sec, func_sec, leb_i32, leb_u32, section, type_sec, wasm_module,
    };

    fn enc_import(out: &mut Vec<u8>, m: &str, f: &str, type_idx: u32) {
        leb_u32(out, m.len() as u32);
        out.extend_from_slice(m.as_bytes());
        leb_u32(out, f.len() as u32);
        out.extend_from_slice(f.as_bytes());
        out.push(0x00); // func import
        leb_u32(out, type_idx);
    }

    // 1. Standalone Goose + WASI + env binary testing Ryū f64/f32 formatting, integers, bools,
    //    stdin `bench_input`/`bench_input_f64`, `format_f64`, and WASI `fd_write`!
    // Types:
    //  0: () -> ()
    //  1: (f64) -> ()
    //  2: (f32) -> ()
    //  3: (i32, f64) -> i32
    //  4: (i64) -> ()
    //  5: (i32) -> ()
    //  6: () -> i64
    //  7: () -> f64
    //  8: (i32, i32) -> ()
    //  9: (i32, i32, i32, i32) -> i32
    let types = type_sec(&[
        (&[], &[]),
        (&[0x7c], &[]),
        (&[0x7d], &[]),
        (&[0x7f, 0x7c], &[0x7f]),
        (&[0x7e], &[]),
        (&[0x7f], &[]),
        (&[], &[0x7e]),
        (&[], &[0x7c]),
        (&[0x7f, 0x7f], &[]),
        (&[0x7f, 0x7f, 0x7f, 0x7f], &[0x7f]),
    ]);
    let mut imp = Vec::new();
    leb_u32(&mut imp, 10);
    enc_import(&mut imp, "goose", "print_f64", 1); // 0
    enc_import(&mut imp, "goose", "print_f32", 2); // 1
    enc_import(&mut imp, "goose", "format_f64", 3); // 2
    enc_import(&mut imp, "goose", "print_i64", 4); // 3
    enc_import(&mut imp, "goose", "print_bool", 5); // 4
    enc_import(&mut imp, "goose", "newline", 0); // 5
    enc_import(&mut imp, "goose", "write", 8); // 6
    enc_import(&mut imp, "env", "bench_input", 6); // 7
    enc_import(&mut imp, "env", "bench_input_f64", 7); // 8
    enc_import(&mut imp, "wasi_snapshot_preview1", "fd_write", 9); // 9

    let mut body = Vec::new();
    let f64_cases: &[(f64, &str)] = &[
        (0.0, "0.0"),
        (-0.0, "-0.0"),
        (1.0, "1.0"),
        (-42.5, "-42.5"),
        (0.1, "0.1"),
        (1e-5, "1e-05"),
        (1e20, "1e+20"),
        (std::f64::consts::PI, "3.141592653589793"),
        (f64::NAN, "nan"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
        (f64::from_bits(1), "5e-324"),
    ];
    let mut expected_stdout = String::new();
    for &(v, exp) in f64_cases {
        body.push(0x44);
        body.extend_from_slice(&v.to_le_bytes());
        body.extend_from_slice(&[0x10, 0, 0x10, 5]); // print_f64; newline
        expected_stdout.push_str(exp);
        expected_stdout.push('\n');
    }
    for &(v, exp) in &[(0.1f32, "0.1"), (-0.0f32, "-0.0"), (f32::INFINITY, "inf")] {
        body.push(0x43);
        body.extend_from_slice(&v.to_le_bytes());
        body.extend_from_slice(&[0x10, 1, 0x10, 5]); // print_f32; newline
        expected_stdout.push_str(exp);
        expected_stdout.push('\n');
    }
    // format_f64(1024, 123.456) -> len; goose::write(1024, len); newline
    body.push(0x41);
    leb_i32(&mut body, 1024);
    body.push(0x44);
    body.extend_from_slice(&123.456f64.to_le_bytes());
    body.extend_from_slice(&[0x10, 2, 0x21, 0, 0x41]);
    leb_i32(&mut body, 1024);
    body.extend_from_slice(&[0x20, 0, 0x10, 6, 0x10, 5]);
    expected_stdout.push_str("123.456\n");

    // Read stdin "42 -3.125\n" via bench_input + bench_input_f64 and print them!
    body.extend_from_slice(&[0x10, 7, 0x10, 3, 0x10, 5]); // print_i64(bench_input()); newline
    body.extend_from_slice(&[0x10, 8, 0x10, 0, 0x10, 5]); // print_f64(bench_input_f64()); newline
    body.extend_from_slice(&[0x41, 1, 0x10, 4, 0x10, 5]); // print_bool(1); newline
    expected_stdout.push_str("42\n-3.125\ntrue\n");

    // WASI fd_write(1, iov@0 -> {ptr: 2048 ("WASI\n"), len: 5}, 1, nout@16)
    let mut data_sec = vec![1, 0, 0x41];
    leb_i32(&mut data_sec, 2048);
    data_sec.push(0x0b);
    leb_u32(&mut data_sec, 5);
    data_sec.extend_from_slice(b"WASI\n");
    body.extend_from_slice(&[0x41, 0, 0x41]);
    leb_i32(&mut body, 2048);
    body.extend_from_slice(&[
        0x36, 2, 0, // mem[0] = 2048
        0x41, 4, 0x41, 5, 0x36, 2, 0, // mem[4] = 5
        0x41, 1, 0x41, 0, 0x41, 1, 0x41, 16, 0x10, 9, 0x1a, // fd_write(1, 0, 1, 16); drop
    ]);
    expected_stdout.push_str("WASI\n");

    let wasm_goose = wasm_module(&[
        types,
        section(2, &imp),
        func_sec(&[0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[("memory", 2, 0), ("_start", 0, 10)]),
        code_sec(&[enc_body(&[(1, 0x7f)], &body)]),
        section(11, &data_sec),
    ]);
    let img_goose = compile_wasm_to_aarch64_for_test(
        &wasm_goose,
        &A64CompileOptions {
            target: X64Target::StandaloneExec,
            optimize: true,
            jobs: 1,
            ..Default::default()
        },
    )
    .expect("aarch64 standalone Goose+WASI compile failed");
    verify_disasm_aarch64("a64_standalone_goose_ryu", &img_goose.elf);
    let mut vm = A64Vm::new(&img_goose);
    vm.stdin = b"42 -3.125\n".to_vec();
    vm.run_until_ret(img_goose.entry_vaddr.unwrap(), 2_000_000)
        .unwrap();
    assert_eq!(
        vm.exited,
        Some(0),
        "stderr={:?}",
        String::from_utf8_lossy(&vm.stderr)
    );
    assert_eq!(
        String::from_utf8(vm.stdout).unwrap(),
        expected_stdout,
        "AArch64 Ryū / Goose / WASI stdout mismatch"
    );

    // 2. Standalone memory32 guard-page BCE + hardware SIGSEGV OOB trap test!
    //    `load_at(idx)` executes an unchecked load (elided by guard-page BCE!) at `idx = 65536`,
    //    landing in the `PROT_NONE` guard region -> SIGSEGV handler writes canonical message and exits(1)!
    let wasm_oob = wasm_module(&[
        type_sec(&[(&[], &[])]),
        func_sec(&[0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[("memory", 2, 0), ("_start", 0, 0)]),
        code_sec(&[enc_body(
            &[],
            &[
                0x41, 0x80, 0x80, 0x04, // i32.const 65536 (first byte of PROT_NONE guard!)
                0x28, 2,
                0,    // i32.load (software bounds check elided -> hardware guard fault!)
                0x1a, // drop
            ],
        )]),
    ]);
    let img_oob = compile_wasm_to_aarch64_for_test(
        &wasm_oob,
        &A64CompileOptions {
            target: X64Target::StandaloneExec,
            optimize: true,
            jobs: 1,
            ..Default::default()
        },
    )
    .expect("aarch64 standalone guard-page OOB compile failed");
    verify_disasm_aarch64("a64_standalone_guard_oob", &img_oob.elf);
    let mut vm_oob = A64Vm::new(&img_oob);
    vm_oob
        .run_until_ret(img_oob.entry_vaddr.unwrap(), 1_000_000)
        .unwrap();
    assert_eq!(vm_oob.exited, Some(1), "guard-page OOB must exit(1)");
    assert_eq!(
        String::from_utf8_lossy(&vm_oob.stderr),
        "out of bounds memory access\n"
    );
}

#[test]
fn aarch64_oracle_stage6_narrow_eq_loops() {
    use crate::x64::tests_unit::{
        code_sec, enc_body, export_sec, func_sec, leb_i32, section, type_sec, wasm_module,
    };
    // f0(packed): 2-key filtered loop `for v in init..limit { if v == k1 || v == k2 { acc = acc*131 + v + 3; mem[320+cnt] = v; cnt += 1 } }`
    let mut b0 = vec![
        0x20, 0, 0x41, 15, 0x71, 0x22, 1, 0x21, 5, // init = v
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // limit
        0x20, 0, 0x41, 8, 0x76, 0x41, 15, 0x71, 0x21, 3, // k1
        0x20, 0, 0x41, 12, 0x76, 0x41, 15, 0x71, 0x21, 4, // k2
        0x41, 5, 0x21, 6, // acc = 5
        0x41, 0, 0x21, 7, // cnt = 0
        0x02, 0x40, 0x03, 0x40, 0x20, 5, 0x20, 2, 0x4f, 0x0d, 1, // if v >=u limit break
        0x20, 5, 0x20, 3, 0x46, 0x20, 5, 0x20, 4, 0x46, 0x72, // v == k1 || v == k2
        0x04, 0x40, 0x20, 6, 0x41,
    ];
    leb_i32(&mut b0, 131);
    b0.extend_from_slice(&[0x6c, 0x20, 5, 0x6a, 0x41, 3, 0x6a, 0x21, 6]);
    b0.extend_from_slice(&[0x20, 7, 0x41]);
    leb_i32(&mut b0, 320);
    b0.extend_from_slice(&[0x6a, 0x20, 5, 0x3a, 0, 0]);
    b0.extend_from_slice(&[
        0x20, 7, 0x41, 1, 0x6a, 0x21, 7, 0x0b, 0x20, 5, 0x41, 1, 0x6a, 0x21, 5, 0x0c, 0, 0x0b,
        0x0b, 0x20, 6, 0x20, 7, 0x41, 24, 0x74, 0x73,
    ]);
    // f1(packed): unguarded do-while loop (must NOT be mis-narrowed when init >= limit)
    let b1 = vec![
        0x20, 0, 0x41, 15, 0x71, 0x21, 1, // v = init
        0x20, 0, 0x41, 4, 0x76, 0x41, 15, 0x71, 0x21, 2, // limit
        0x20, 0, 0x41, 8, 0x76, 0x41, 15, 0x71, 0x21, 3, // k1
        0x41, 0, 0x21, 4, // acc = 0
        0x03, 0x40, 0x20, 1, 0x20, 3, 0x46, 0x04, 0x40, 0x20, 4, 0x20, 1, 0x6a, 0x41, 0xe4, 0x00,
        0x6a, 0x21, 4, 0x0b, 0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x20, 2, 0x49, 0x0d, 0, 0x0b, 0x20,
        4,
    ];
    let wasm = wasm_module(&[
        type_sec(&[(&[0x7f], &[0x7f])]),
        func_sec(&[0, 0]),
        section(5, &[1, 0x01, 1, 1]),
        export_sec(&[
            ("blot:memory", 2, 0),
            ("narrow_2k", 0, 0),
            ("dowhile_1k", 0, 1),
        ]),
        code_sec(&[enc_body(&[(7, 0x7f)], &b0), enc_body(&[(4, 0x7f)], &b1)]),
    ]);

    let mut calls = Vec::new();
    for (init, limit, k1, k2) in [
        (0, 0, 0, 0),
        (0, 1, 0, 0),
        (0, 10, 2, 7),
        (0, 10, 7, 2),
        (0, 10, 5, 5),
        (3, 10, 1, 8),
        (3, 10, 8, 12),
        (3, 10, 1, 12),
        (8, 3, 8, 9),
    ] {
        calls.push((
            "narrow_2k",
            Value::I32(init | (limit << 4) | (k1 << 8) | (k2 << 12)),
        ));
    }
    for (init, limit, k1) in [(5, 0, 5), (5, 5, 5), (2, 7, 4)] {
        calls.push(("dowhile_1k", Value::I32(init | (limit << 4) | (k1 << 8))));
    }
    assert_aarch64_oracle("a64_stage6_narrow_eq_loops", &wasm, &calls, 384);

    // Verify `fixtures/graph_bfs.wasm` parity on AArch64 too!
    let bfs_wasm =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/graph_bfs.wasm"))
            .expect("read fixtures/graph_bfs.wasm");
    assert_aarch64_oracle(
        "a64_stage6_graph_bfs",
        &bfs_wasm,
        &[
            ("run", Value::I32(0)),
            ("run", Value::I32(1)),
            ("run", Value::I32(16)),
            ("run", Value::I32(64)),
            ("run", Value::I32(1024)),
        ],
        4224,
    );
}
